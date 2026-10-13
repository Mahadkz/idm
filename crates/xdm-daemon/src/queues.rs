use std::collections::HashSet;
use std::io;
use std::sync::atomic::Ordering;

use rusqlite::{params, Connection, OptionalExtension};
use xdm_ipc::{
    CompletionAction, CompletionPending, MoveTarget, QueueEvent, QueueSummary, Reply,
};

use crate::schedule::{self, Kind, Schedule};
use crate::{set_setting, unix_now, Inner, Result, Service};

pub(crate) const DEFAULT_COUNTDOWN_SECS: u64 = 60;

/// A countdown in flight for one queue's finished cycle; the countdown is
/// in-memory, so a daemon restart suppresses rather than surprises.
#[derive(Clone, Debug)]
pub(crate) struct PendingCompletion {
    pub(crate) event_id: i64,
    pub(crate) action: String,
    pub(crate) deadline: i64,
}

pub(crate) const COMPLETION_ACTIONS: [&str; 4] = ["none", "notify", "sleep", "shutdown"];

pub(crate) fn interrupt_queue_workers(inner: &mut Inner, queue_id: Option<i64>) {
    let active = inner
        .active
        .iter()
        .filter(|(_, active)| queue_id.is_none_or(|id| active.queue_id == id))
        .map(|(id, active)| (*id, active.stop.clone(), active.stop_requested.clone()))
        .collect::<Vec<_>>();
    for (id, stop, requested) in active {
        inner.queue_interrupted.insert(id);
        stop.store(true, Ordering::Relaxed);
        requested.store(true, Ordering::Relaxed);
    }
    // Any stop sweep — manual, global or schedule cutoff — suppresses the
    // destructive countdown of the affected queues.
    let pending: Vec<i64> = inner
        .pending
        .keys()
        .filter(|id| queue_id.is_none_or(|target| **id == target))
        .copied()
        .collect();
    for id in pending {
        cancel_pending(inner, id, "suppressed");
    }
}

/// Removes one queue's pending countdown and finalizes its event while the
/// status is still "pending", so a late call never overwrites the outcome.
pub(crate) fn cancel_pending(inner: &mut Inner, queue_id: i64, status: &str) {
    if let Some(pending) = inner.pending.remove(&queue_id) {
        let _ = inner.db.execute(
            "UPDATE queue_events SET action_status=?1 WHERE id=?2 AND action_status='pending'",
            params![status, pending.event_id],
        );
    }
}

pub(crate) fn valid_queue_name(name: &str) -> bool {
    !name.trim().is_empty()
        && name == name.trim()
        && name.len() <= 64
        && !name.chars().any(char::is_control)
}

pub(crate) fn queue_summaries(inner: &Inner) -> Result<Vec<QueueSummary>> {
    let mut statement = inner.db.prepare("SELECT q.id,q.name,q.running,q.max_concurrent,COALESCE(SUM(t.status='queued'),0),COALESCE(SUM(t.status='paused'),0),COALESCE(SUM(t.status='failed'),0),(SELECT COUNT(*) FROM tasks c WHERE c.history_queue_id=q.id AND c.status='complete'),q.solo,q.retry_max,q.retry_delay,q.sched_kind,q.sched_start,q.sched_stop,q.sched_days,q.completion_action,q.run_state,q.run_exec FROM queues q LEFT JOIN tasks t ON t.queue_id=q.id GROUP BY q.id ORDER BY q.id")?;
    Ok(statement
        .query_map([], |row| {
            let id = row.get::<_, i64>(0)?;
            Ok(QueueSummary {
                id,
                name: row.get(1)?,
                running: row.get(2)?,
                limit: row.get::<_, i64>(3)? as usize,
                active: inner
                    .active
                    .values()
                    .filter(|active| active.queue_id == id)
                    .count(),
                queued: row.get::<_, i64>(4)? as usize,
                paused: row.get::<_, i64>(5)? as usize,
                failed: row.get::<_, i64>(6)? as usize,
                complete: row.get::<_, i64>(7)? as usize,
                started_alone: row.get(8)?,
                retry_max: row.get(9)?,
                retry_delay: row.get::<_, i64>(10)? as u64,
                schedule_kind: row.get(11)?,
                schedule_start: row.get(12)?,
                schedule_stop: row.get(13)?,
                schedule_days: row.get::<_, i64>(14)? as u8,
                completion_action: row.get(15)?,
                run_state: row.get(16)?,
                run_exec: row.get(17)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Gate for one queue: Stop-all clears every independent start, so the global
/// stop always dominates; a queue started on its own ignores the global flag.
pub(crate) fn stop_all_independent(db: &Connection) -> rusqlite::Result<()> {
    db.execute("UPDATE queues SET solo=0,sched_until=NULL", [])?;
    Ok(())
}

pub(crate) fn queue_events(db: &Connection, queue_id: i64) -> rusqlite::Result<Vec<QueueEvent>> {
    db.prepare("SELECT id,queue_id,execution,outcome,action,action_status,at FROM (SELECT * FROM queue_events WHERE queue_id=?1 ORDER BY id DESC LIMIT 100) ORDER BY id")?
        .query_map([queue_id], |row| {
            Ok(QueueEvent {
                id: row.get(0)?,
                queue_id: row.get(1)?,
                execution: row.get(2)?,
                outcome: row.get(3)?,
                action: row.get(4)?,
                action_status: row.get(5)?,
                at: row.get(6)?,
            })
        })?
        .collect()
}

/// Ends a queue's current cycle exactly once: records the event and resolves
/// the completion action only after natural success (never after failure,
/// manual stop or a schedule cutoff). A confirmed destructive power action
/// starts its cancelable countdown instead of running immediately.
pub(crate) fn finish_cycle(inner: &mut Inner, queue_id: i64, outcome: &str, now: i64) -> Result<()> {
    let (exec, action, confirmed, countdown): (i64, String, bool, u64) = inner.db.query_row(
        "SELECT run_exec,completion_action,completion_confirmed,completion_countdown FROM queues WHERE id=?1",
        [queue_id],
        |row| Ok((
            row.get(0)?,
            row.get(1)?,
            row.get::<_, i64>(2)? != 0,
            row.get::<_, i64>(3)?.max(0) as u64,
        )),
    )?;
    let state = match outcome {
        "completed" | "completed_with_failures" => outcome,
        _ => "stopped",
    };
    let armed = outcome == "completed"
        && confirmed
        && CompletionAction::decode(&action).is_some_and(|parsed| parsed.is_destructive());
    let status = if action == "none" {
        "none".to_string()
    } else if outcome == "completed" {
        if armed {
            "pending".to_string()
        } else {
            inner.adapter.run(queue_id, &action, false)
        }
    } else {
        "suppressed".to_string()
    };
    inner.db.execute(
        "UPDATE queues SET run_state=?1 WHERE id=?2",
        params![state, queue_id],
    )?;
    inner.db.execute(
        "INSERT INTO queue_events(queue_id,execution,outcome,action,action_status,at) VALUES (?1,?2,?3,?4,?5,?6)",
        params![queue_id, exec, outcome, action, status, now],
    )?;
    if armed {
        inner.pending.insert(
            queue_id,
            PendingCompletion {
                event_id: inner.db.last_insert_rowid(),
                action,
                deadline: inner.clock.now() + countdown.max(1) as i64,
            },
        );
    }
    inner.db.execute(
        "DELETE FROM queue_events WHERE id <= (SELECT MAX(id) FROM queue_events)-500",
        [],
    )?;
    Ok(())
}

/// Fires countdowns that reached their deadline: the action executes through
/// the adapter (armed) and the event's status becomes its outcome.
pub(crate) fn run_due_completions(inner: &mut Inner) -> Result<()> {
    let now = inner.clock.now();
    let due: Vec<i64> = inner
        .pending
        .iter()
        .filter(|(_, pending)| pending.deadline <= now)
        .map(|(queue_id, _)| *queue_id)
        .collect();
    for queue_id in due {
        let Some(pending) = inner.pending.remove(&queue_id) else {
            continue;
        };
        let status = inner.adapter.run(queue_id, &pending.action, true);
        inner.db.execute(
            "UPDATE queue_events SET action_status=?1 WHERE id=?2 AND action_status='pending'",
            params![status, pending.event_id],
        )?;
    }
    Ok(())
}

/// Starts and stops queues from their own schedules and drives each queue's
/// completion state machine. Runs on every scheduler wake and command.
pub(crate) fn queue_tick(inner: &mut Inner) -> Result<()> {
    let now = inner.clock.now();
    let rows = inner
        .db
        .prepare("SELECT id,sched_kind,sched_start,sched_stop,sched_days,sched_last,sched_until FROM queues WHERE sched_kind<>'none' OR sched_until IS NOT NULL")?
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, Option<i64>>(6)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, kind, start, stop, days, last, until) in rows {
        // Stop first so an overdue stop wins over an overdue start.
        if until.is_some_and(|at| now >= at) {
            inner.db.execute(
                "UPDATE queues SET running=0,solo=0,sched_until=NULL WHERE id=?1",
                [id],
            )?;
            interrupt_queue_workers(inner, Some(id));
            let running: String = inner.db.query_row(
                "SELECT run_state FROM queues WHERE id=?1",
                [id],
                |row| row.get(0),
            )?;
            if running == "running" {
                finish_cycle(inner, id, "schedule_stop", now)?;
            }
        }
        let schedule = Schedule {
            kind: Kind::parse(&kind).unwrap_or(Kind::None),
            start,
            stop,
            days: days as u8,
        };
        if let Some(occurrence) = schedule::current_occurrence(&schedule, now, inner.clock.as_ref())
            && schedule::should_fire(&occurrence, last, now)
        {
            inner.db.execute(
                "UPDATE queues SET running=1,solo=1,sched_last=?1,sched_until=?2 WHERE id=?3",
                params![occurrence.start, occurrence.has_stop.then_some(occurrence.end), id],
            )?;
        }
    }
    let global = inner.queue_running;
    let queues = inner
        .db
        .prepare("SELECT id,running,solo,run_state FROM queues")?
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, bool>(1)?,
                row.get::<_, bool>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, running, solo, state) in queues {
        let eligible = running && (global || solo);
        let pending: i64 = inner.db.query_row(
            "SELECT COUNT(*) FROM tasks WHERE queue_id=?1 AND status IN ('queued','running','pausing')",
            [id],
            |row| row.get(0),
        )?;
        if state == "running" && !eligible {
            finish_cycle(inner, id, "manual_stop", now)?;
        } else if state == "running" && pending == 0 {
            let failed: i64 = inner.db.query_row(
                "SELECT COUNT(*) FROM tasks WHERE queue_id=?1 AND status IN ('failed','restart_required')",
                [id],
                |row| row.get(0),
            )?;
            let outcome = if failed > 0 {
                "completed_with_failures"
            } else {
                "completed"
            };
            finish_cycle(inner, id, outcome, now)?;
        } else if state != "running" && eligible && pending > 0 {
            // New work restarts the cycle; a countdown from the finished
            // cycle must not fire while the queue is in use again.
            cancel_pending(inner, id, "suppressed");
            inner.db.execute(
                "UPDATE queues SET run_state='running',run_exec=run_exec+1 WHERE id=?1",
                [id],
            )?;
        }
    }
    run_due_completions(inner)?;
    Ok(())
}

pub(crate) fn queue_reply(inner: &Inner, queue_id: i64) -> Result<Reply> {
    let mut reply = Reply::list(Vec::new());
    reply.queue_id = Some(queue_id);
    reply.queues = queue_summaries(inner)?;
    Ok(reply)
}

impl Service {
    pub(crate) fn queue_create(&self, name: String, limit: usize) -> Result<Reply> {
        if !valid_queue_name(&name) || !(1..=16).contains(&limit) {
            return Ok(Reply::error(
                "queue name must contain 1–64 bytes and limit must be 1–16",
            ));
        }
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        let count: i64 = inner
            .db
            .query_row("SELECT COUNT(*) FROM queues", [], |row| row.get(0))?;
        if count >= 32 {
            return Ok(Reply::error("at most 32 queues are supported"));
        }
        inner.db.execute(
            "INSERT INTO queues(name,running,max_concurrent) VALUES (?1,0,?2)",
            params![name, limit as i64],
        )?;
        queue_reply(&inner, inner.db.last_insert_rowid())
    }

    pub(crate) fn queue_set_running(&self, queue_id: i64, running: bool) -> Result<Reply> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        if inner.db.execute(
            "UPDATE queues SET running=?1 WHERE id=?2",
            params![running, queue_id],
        )? == 0
        {
            return Ok(Reply::error("queue not found"));
        }
        if !running {
            inner.db.execute(
                "UPDATE queues SET solo=0,sched_until=NULL WHERE id=?1",
                [queue_id],
            )?;
            interrupt_queue_workers(&mut inner, Some(queue_id));
        }
        let reply = queue_reply(&inner, queue_id)?;
        drop(inner);
        self.wake.notify_one();
        Ok(reply)
    }

    pub(crate) fn queue_set_concurrency(&self, queue_id: i64, limit: usize) -> Result<Reply> {
        if !(1..=16).contains(&limit) {
            return Ok(Reply::error("queue limit must be 1–16"));
        }
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        if inner.db.execute(
            "UPDATE queues SET max_concurrent=?1 WHERE id=?2",
            params![limit as i64, queue_id],
        )? == 0
        {
            return Ok(Reply::error("queue not found"));
        }
        let reply = queue_reply(&inner, queue_id)?;
        drop(inner);
        self.wake.notify_one();
        Ok(reply)
    }

    pub(crate) fn queue_rename(&self, queue_id: i64, name: String) -> Result<Reply> {
        if queue_id == 1 {
            return Ok(Reply::error("Main queue cannot be renamed"));
        }
        if !valid_queue_name(&name) {
            return Ok(Reply::error("enter a queue name of 1–64 bytes"));
        }
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        if inner.db.execute(
            "UPDATE queues SET name=?1 WHERE id=?2",
            params![name, queue_id],
        )? == 0
        {
            return Ok(Reply::error("queue not found"));
        }
        queue_reply(&inner, queue_id)
    }

    pub(crate) fn queue_remove(&self, queue_id: i64) -> Result<Reply> {
        if queue_id == 1 {
            return Ok(Reply::error("Main queue cannot be removed"));
        }
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        let count: i64 = inner.db.query_row(
            "SELECT COUNT(*) FROM tasks WHERE queue_id=?1",
            [queue_id],
            |row| row.get(0),
        )?;
        if count > 0 {
            return Ok(Reply::error(
                "move or remove every task before deleting its queue",
            ));
        }
        if inner
            .db
            .execute("DELETE FROM queues WHERE id=?1", [queue_id])?
            == 0
        {
            return Ok(Reply::error("queue not found"));
        }
        inner.pending.remove(&queue_id);
        inner.db.execute(
            "UPDATE tasks SET history_queue_id=NULL WHERE history_queue_id=?1",
            [queue_id],
        )?;
        inner
            .db
            .execute("DELETE FROM queue_events WHERE queue_id=?1", [queue_id])?;
        queue_reply(&inner, queue_id)
    }

    pub(crate) fn assign_queue(&self, queue_id: i64, task_ids: Vec<i64>) -> Result<Reply> {
        if task_ids.is_empty()
            || task_ids.len() > 500
            || task_ids.iter().collect::<HashSet<_>>().len() != task_ids.len()
        {
            return Ok(Reply::error("choose 1–500 distinct task IDs"));
        }
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        let exists: bool = inner.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM queues WHERE id=?1)",
            [queue_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(Reply::error("queue not found"));
        }
        if task_ids.iter().any(|id| inner.active.contains_key(id)) {
            return Ok(Reply::error(
                "pause active tasks before moving them between queues",
            ));
        }
        let transaction = inner.db.transaction()?;
        let retry_max: i64 = transaction.query_row(
            "SELECT retry_max FROM queues WHERE id=?1",
            [queue_id],
            |row| row.get(0),
        )?;
        for id in &task_ids {
            let current = transaction
                .query_row(
                    "SELECT queue_id,status FROM tasks WHERE id=?1",
                    [id],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()?;
            match current {
                None => return Ok(Reply::error("task not found; no tasks moved")),
                // Completed tasks are not live members: only history moves.
                Some((_, status)) if status == "complete" => {
                    transaction.execute(
                        "UPDATE tasks SET history_queue_id=?1 WHERE id=?2",
                        params![queue_id, id],
                    )?;
                }
                Some((current, _)) if current == queue_id => {}
                Some(_) => {
                    transaction.execute(
                        "UPDATE tasks SET queue_id=?1,position=(SELECT COALESCE(MAX(position),0)+1 FROM tasks WHERE queue_id=?1),updated_at=CURRENT_TIMESTAMP WHERE id=?2",
                        params![queue_id, id],
                    )?;
                    // A retry armed under the old queue's policy must not
                    // fire in a queue that would never have armed it; the
                    // failure stands instead. Infinite (-1) always retries.
                    if retry_max >= 0 {
                        transaction.execute(
                            "UPDATE tasks SET status='failed',retry_count=0,retry_at=NULL WHERE id=?1 AND status='queued' AND retry_at IS NOT NULL AND retry_count>?2",
                            params![id, retry_max],
                        )?;
                    }
                }
            }
        }
        transaction.commit()?;
        let mut reply = queue_reply(&inner, queue_id)?;
        reply.task_ids = task_ids;
        drop(inner);
        self.wake.notify_one();
        Ok(reply)
    }

    pub(crate) fn queue_start(&self) -> Result<Reply> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        set_setting(&inner.db, "queue_running", "1")?;
        inner
            .db
            .execute("DELETE FROM settings WHERE key='queue_start_at'", [])?;
        inner.queue_running = true;
        inner.queue_start_at = None;
        drop(inner);
        self.wake.notify_one();
        Ok(Reply::list(Vec::new()))
    }

    pub(crate) fn queue_stop(&self) -> Result<Reply> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        set_setting(&inner.db, "queue_running", "0")?;
        inner.db.execute(
            "DELETE FROM settings WHERE key IN ('queue_start_at','queue_stop_at')",
            [],
        )?;
        inner.queue_running = false;
        inner.queue_start_at = None;
        inner.queue_stop_at = None;
        stop_all_independent(&inner.db)?;
        interrupt_queue_workers(&mut inner, None);
        Ok(Reply::list(Vec::new()))
    }

    pub(crate) fn queue_schedule_start(&self, at_unix: i64) -> Result<Reply> {
        let now = unix_now();
        if at_unix <= now || at_unix > now + 366 * 86400 {
            return Ok(Reply::error(
                "queue start time must be within the next year",
            ));
        }
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        // Arming only records the deadline (IDM behavior): the gate keeps
        // its state and running transfers, including independent queue
        // starts, keep running until the start time opens the gate.
        set_setting(&inner.db, "queue_start_at", &at_unix.to_string())?;
        inner.queue_start_at = Some(at_unix);
        drop(inner);
        self.wake.notify_one();
        Ok(Reply::list(Vec::new()))
    }

    pub(crate) fn queue_schedule_stop(&self, at_unix: i64) -> Result<Reply> {
        let now = unix_now();
        if at_unix <= now || at_unix > now + 366 * 86400 {
            return Ok(Reply::error("queue stop time must be within the next year"));
        }
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        set_setting(&inner.db, "queue_stop_at", &at_unix.to_string())?;
        inner.queue_stop_at = Some(at_unix);
        drop(inner);
        self.wake.notify_one();
        Ok(Reply::list(Vec::new()))
    }

    pub(crate) fn queue_start_one(&self, queue_id: i64) -> Result<Reply> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        if inner.db.execute(
            "UPDATE queues SET running=1,solo=1 WHERE id=?1",
            [queue_id],
        )? == 0
        {
            return Ok(Reply::error("queue not found"));
        }
        queue_reply(&inner, queue_id)
    }

    pub(crate) fn queue_stop_one(&self, queue_id: i64) -> Result<Reply> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        if inner.db.execute(
            "UPDATE queues SET running=0,solo=0,sched_until=NULL WHERE id=?1",
            [queue_id],
        )? == 0
        {
            return Ok(Reply::error("queue not found"));
        }
        interrupt_queue_workers(&mut inner, Some(queue_id));
        queue_reply(&inner, queue_id)
    }

    pub(crate) fn task_move(&self, task_id: i64, target: MoveTarget) -> Result<Reply> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        let queue_id = inner
            .db
            .query_row("SELECT queue_id FROM tasks WHERE id=?1", [task_id], |row| {
                row.get::<_, i64>(0)
            })
            .optional()?;
        let Some(queue_id) = queue_id else {
            return Ok(Reply::error("task not found"));
        };
        if queue_id == 0 {
            return Ok(Reply::error("completed tasks are not in a queue"));
        }
        let mut order = inner
            .db
            .prepare("SELECT id FROM tasks WHERE queue_id=?1 ORDER BY position,id")?
            .query_map([queue_id], |row| row.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let Some(from) = order.iter().position(|id| *id == task_id) else {
            return Ok(Reply::error("task not found"));
        };
        let last = order.len() - 1;
        let to = match target {
            MoveTarget::Up => from.saturating_sub(1),
            MoveTarget::Down => (from + 1).min(last),
            MoveTarget::Top => 0,
            MoveTarget::Bottom => last,
            MoveTarget::Index(index) if index <= last => index,
            MoveTarget::Index(_) => return Ok(Reply::error("index is outside the queue")),
        };
        order.remove(from);
        order.insert(to, task_id);
        let transaction = inner.db.transaction()?;
        for (index, id) in order.iter().enumerate() {
            transaction.execute(
                "UPDATE tasks SET position=?1 WHERE id=?2",
                params![index as i64 + 1, id],
            )?;
        }
        transaction.commit()?;
        drop(inner);
        self.wake.notify_one();
        let mut reply = Reply::accepted(task_id);
        reply.task_ids = order;
        Ok(reply)
    }

    pub(crate) fn queue_set_retry(
        &self,
        queue_id: i64,
        max_retries: i64,
        delay_seconds: u64,
    ) -> Result<Reply> {
        if !(-1..=1000).contains(&max_retries) || delay_seconds > 86400 {
            return Ok(Reply::error(
                "retries must be -1 (infinite) to 1000 and delay at most 86400 seconds",
            ));
        }
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        if inner.db.execute(
            "UPDATE queues SET retry_max=?1,retry_delay=?2 WHERE id=?3",
            params![max_retries, delay_seconds as i64, queue_id],
        )? == 0
        {
            return Ok(Reply::error("queue not found"));
        }
        queue_reply(&inner, queue_id)
    }

    pub(crate) fn queue_set_schedule(
        &self,
        queue_id: i64,
        kind: String,
        start: i64,
        stop: Option<i64>,
        days: u8,
    ) -> Result<Reply> {
        let Some(parsed) = Kind::parse(&kind) else {
            return Ok(Reply::error("schedule kind must be none, once or daily"));
        };
        let schedule = Schedule {
            kind: parsed,
            start,
            stop,
            days,
        };
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        if let Err(error) = schedule::validate(&schedule, inner.clock.now()) {
            return Ok(Reply::error(error));
        }
        // Editing re-arms execution identity: an open window starts once.
        if inner.db.execute(
            "UPDATE queues SET sched_kind=?1,sched_start=?2,sched_stop=?3,sched_days=?4,sched_last=0,sched_until=NULL WHERE id=?5",
            params![kind, start, stop, days as i64, queue_id],
        )? == 0
        {
            return Ok(Reply::error("queue not found"));
        }
        // A running queue adopts the edited cutoff even when it already
        // passed: the start path cannot re-arm a closed window, so without
        // this an earlier stop time would never take effect.
        let running: bool = inner.db.query_row(
            "SELECT running FROM queues WHERE id=?1",
            [queue_id],
            |row| row.get(0),
        )?;
        let now = inner.clock.now();
        if running
            && let Some(occurrence) = schedule::current_occurrence(&schedule, now, inner.clock.as_ref())
            && occurrence.has_stop
            && now >= occurrence.end
        {
            inner.db.execute(
                "UPDATE queues SET sched_until=?1 WHERE id=?2",
                params![occurrence.end, queue_id],
            )?;
        }
        queue_reply(&inner, queue_id)
    }

    pub(crate) fn queue_set_completion(&self, queue_id: i64, action: String) -> Result<Reply> {
        if !COMPLETION_ACTIONS.contains(&action.as_str()) {
            return Ok(Reply::error(
                "completion action must be none, notify, sleep or shutdown",
            ));
        }
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        if inner.db.execute(
            "UPDATE queues SET completion_action=?1 WHERE id=?2",
            params![action, queue_id],
        )? == 0
        {
            return Ok(Reply::error("queue not found"));
        }
        // Editing the action while a countdown runs cancels it.
        cancel_pending(&mut inner, queue_id, "cancelled");
        queue_reply(&inner, queue_id)
    }

    pub(crate) fn queue_set_completion_action(
        &self,
        queue_id: i64,
        action: CompletionAction,
        confirm: bool,
        countdown_seconds: Option<u64>,
    ) -> Result<Reply> {
        let destructive = action.is_destructive();
        if destructive && !confirm {
            return Ok(Reply::error(
                "sleep, hibernate, shutdown and restart require --confirm-destructive (explicit opt-in) and a cancelable countdown",
            ));
        }
        if let CompletionAction::Program { program, args } = &action
            && (program.is_empty()
                || program.len() > 4096
                || program.contains('\0')
                || args.len() > 64
                || args.iter().any(|arg| arg.len() > 4096 || arg.contains('\0')))
        {
            return Ok(Reply::error(
                "program completion needs a path of 1–4096 bytes and at most 64 arguments of up to 4096 bytes",
            ));
        }
        let countdown = countdown_seconds.unwrap_or(DEFAULT_COUNTDOWN_SECS);
        if countdown == 0 || countdown > 86_400 {
            return Ok(Reply::error("countdown must be 1–86400 seconds"));
        }
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        if inner.db.execute(
            "UPDATE queues SET completion_action=?1,completion_confirmed=?2,completion_countdown=?3 WHERE id=?4",
            params![action.encode(), destructive && confirm, countdown as i64, queue_id],
        )? == 0
        {
            return Ok(Reply::error("queue not found"));
        }
        cancel_pending(&mut inner, queue_id, "cancelled");
        queue_reply(&inner, queue_id)
    }

    pub(crate) fn completion_status(&self) -> Result<Reply> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        let now = inner.clock.now();
        let mut reply = Reply::list(Vec::new());
        reply.completion_pending = inner
            .pending
            .iter()
            .map(|(queue_id, pending)| CompletionPending {
                queue_id: *queue_id,
                action: pending.action.clone(),
                deadline_unix: pending.deadline,
                seconds_remaining: pending.deadline.saturating_sub(now) as u64,
            })
            .collect();
        Ok(reply)
    }

    pub(crate) fn completion_cancel(&self, queue_id: i64) -> Result<Reply> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        if inner.pending.contains_key(&queue_id) {
            cancel_pending(&mut inner, queue_id, "cancelled");
            Ok(Reply::accepted(queue_id))
        } else {
            Ok(Reply::error("no pending completion countdown for this queue"))
        }
    }

    pub(crate) fn queue_events_command(&self, queue_id: i64) -> Result<Reply> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        let mut reply = queue_reply(&inner, queue_id)?;
        reply.queue_events = queue_events(&inner.db, queue_id)?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use xdm_ipc::{Command, CompletionAction, MoveTarget};

    use crate::os_actions::RecordOnly;
    use crate::schedule::tests::FakeClock;
    use crate::test_helpers::{finish, order, queue, service, submit, summary, task_state};
    use crate::{Active, Service, unix_now};

    #[test]
    fn manual_order_moves_and_persists_in_queue() {
        let (service, _, _) = service(1000);
        let ids = (0..4)
            .map(|n| submit(&service, &format!("o{n}")))
            .collect::<Vec<_>>();
        let go = |id, target| {
            assert!(service.command(Command::TaskMove { task_id: id, target }).unwrap().ok)
        };
        assert_eq!(order(&service, 1), ids);
        go(ids[3], MoveTarget::Top);
        assert_eq!(order(&service, 1), [ids[3], ids[0], ids[1], ids[2]]);
        go(ids[3], MoveTarget::Down);
        assert_eq!(order(&service, 1), [ids[0], ids[3], ids[1], ids[2]]);
        go(ids[2], MoveTarget::Up);
        assert_eq!(order(&service, 1), [ids[0], ids[3], ids[2], ids[1]]);
        go(ids[0], MoveTarget::Bottom);
        assert_eq!(order(&service, 1), [ids[3], ids[2], ids[1], ids[0]]);
        go(ids[0], MoveTarget::Index(1));
        assert_eq!(order(&service, 1), [ids[3], ids[0], ids[2], ids[1]]);
        let bad = service
            .command(Command::TaskMove { task_id: ids[0], target: MoveTarget::Index(4) })
            .unwrap();
        assert!(!bad.ok);
        // A newly submitted task is appended after every manually ordered one.
        let later = submit(&service, "o-late");
        assert_eq!(order(&service, 1).last(), Some(&later));
        // Moving into another queue appends there.
        let other = queue(&service, "Other");
        service
            .command(Command::AssignQueue { queue_id: other, task_ids: vec![ids[1], ids[3]] })
            .unwrap();
        assert_eq!(order(&service, other), [ids[1], ids[3]]);
        // Priority dominates manual order inside a queue.
        service.command(Command::SetPriority { task_id: ids[3], priority: 2 }).unwrap();
        assert_eq!(order(&service, other), [ids[3], ids[1]]);
    }

    #[test]
    fn completed_tasks_leave_queue_but_keep_history() {
        let (service, _, _) = service(1000);
        let q = queue(&service, "Docs");
        let id = submit(&service, "c0");
        service.command(Command::AssignQueue { queue_id: q, task_ids: vec![id] }).unwrap();
        assert!(!service.command(Command::QueueRemove { queue_id: q }).unwrap().ok);
        finish(&service, id, "complete", None);
        assert_eq!(task_state(&service, id).3, 0);
        let row = summary(&service, q);
        assert_eq!((row.complete, row.queued), (1, 0));
        let listed = service.command(Command::GetTask { task_id: id }).unwrap();
        assert_eq!(listed.tasks[0].history_queue_id, Some(q));
        assert!(!service.command(Command::TaskMove { task_id: id, target: MoveTarget::Top }).unwrap().ok);
        assert!(service.command(Command::QueueRemove { queue_id: q }).unwrap().ok);
    }

    #[test]
    fn queue_schedule_runs_window_once_and_never_after_stop() {
        let (service, clock, _) = service(10_000);
        let q = queue(&service, "Night");
        let id = submit(&service, "s0");
        service.command(Command::AssignQueue { queue_id: q, task_ids: vec![id] }).unwrap();
        let set = |kind: &str, start, stop, days| {
            service.command(Command::QueueSetSchedule { queue_id: q, kind: kind.into(), start, stop, days }).unwrap()
        };
        assert!(!set("daily", 60, None, 0).ok);
        assert!(!set("once", 11_000, Some(10_500), 0).ok);
        assert!(set("once", 11_000, Some(12_000), 0).ok);
        service.command(Command::List).unwrap();
        assert!(!summary(&service, q).started_alone);
        clock.set(11_000);
        service.command(Command::List).unwrap();
        let row = summary(&service, q);
        assert!(row.started_alone && row.running && row.run_state == "running");
        // Manual stop inside the window is not undone by the schedule.
        service.command(Command::QueueStopOne { queue_id: q }).unwrap();
        clock.set(11_500);
        service.command(Command::List).unwrap();
        assert!(!summary(&service, q).running);
        // A new window starts, then its cutoff stops it and records the event.
        assert!(set("once", 12_000, Some(13_000), 0).ok);
        clock.set(12_100);
        service.command(Command::List).unwrap();
        assert!(summary(&service, q).running);
        clock.set(13_000);
        service.command(Command::List).unwrap();
        let row = summary(&service, q);
        assert!(!row.running && !row.started_alone && row.run_state == "stopped");
        // Daemon absent over the whole window: nothing starts afterwards.
        assert!(set("once", 14_000, Some(15_000), 0).ok);
        clock.set(16_000);
        service.command(Command::List).unwrap();
        assert!(!summary(&service, q).started_alone);
        let events = service.command(Command::QueueEvents { queue_id: q }).unwrap().queue_events;
        assert_eq!(events.last().map(|event| event.outcome.as_str()), Some("schedule_stop"));
    }

    #[test]
    fn missed_start_without_stop_is_skipped_after_grace() {
        let (service, clock, _) = service(10_000);
        let q = queue(&service, "Late");
        clock.set(10_000);
        service.command(Command::QueueSetSchedule { queue_id: q, kind: "once".into(), start: 9_990, stop: None, days: 0 }).unwrap();
        service.command(Command::List).unwrap();
        assert!(summary(&service, q).started_alone, "inside grace: catch up once");
        service.command(Command::QueueStopOne { queue_id: q }).unwrap();
        service.command(Command::List).unwrap();
        assert!(!summary(&service, q).started_alone, "executed identity is not repeated");
        service.command(Command::QueueSetSchedule { queue_id: q, kind: "once".into(), start: 1_000, stop: None, days: 0 }).unwrap();
        service.command(Command::List).unwrap();
        assert!(!summary(&service, q).started_alone, "outside grace: skipped");
    }

    #[test]
    fn completion_state_machine_fires_once_and_respects_stop_and_failure() {
        let (service, _, adapter) = service(1000);
        let q = queue(&service, "Done");
        assert!(!service.command(Command::QueueSetCompletion { queue_id: q, action: "format-disk".into() }).unwrap().ok);
        service.command(Command::QueueSetCompletion { queue_id: q, action: "shutdown".into() }).unwrap();
        let run = |service: &Service| service.command(Command::List).unwrap();
        // Empty start: no cycle, no event.
        service.command(Command::QueueStartOne { queue_id: q }).unwrap();
        assert_eq!(summary(&service, q).run_state, "idle");
        let a = submit(&service, "d0");
        let b = submit(&service, "d1");
        service.command(Command::AssignQueue { queue_id: q, task_ids: vec![a, b] }).unwrap();
        run(&service);
        let row = summary(&service, q);
        assert_eq!((row.run_state.as_str(), row.run_exec), ("running", 1));
        finish(&service, a, "complete", None);
        run(&service);
        assert_eq!(adapter.0.load(Ordering::Relaxed), 0);
        finish(&service, b, "complete", None);
        run(&service);
        run(&service);
        assert_eq!(summary(&service, q).run_state, "completed");
        assert_eq!(adapter.0.load(Ordering::Relaxed), 1);
        let events = service.command(Command::QueueEvents { queue_id: q }).unwrap().queue_events;
        assert_eq!(events.len(), 1);
        assert_eq!((events[0].outcome.as_str(), events[0].action_status.as_str(), events[0].execution), ("completed", "fake", 1));
        // New work starts a new cycle; an exhausted failure suppresses the action.
        let c = submit(&service, "d2");
        service.command(Command::AssignQueue { queue_id: q, task_ids: vec![c] }).unwrap();
        run(&service);
        assert_eq!(summary(&service, q).run_exec, 2);
        finish(&service, c, "failed", Some("HTTP 404"));
        run(&service);
        assert_eq!(summary(&service, q).run_state, "completed_with_failures");
        assert_eq!(adapter.0.load(Ordering::Relaxed), 1);
        // Manual stop of a third cycle suppresses it as well.
        service.command(Command::Resume { task_id: c }).unwrap();
        run(&service);
        service.command(Command::QueueStopOne { queue_id: q }).unwrap();
        run(&service);
        assert_eq!(summary(&service, q).run_state, "stopped");
        assert_eq!(adapter.0.load(Ordering::Relaxed), 1);
        let events = service.command(Command::QueueEvents { queue_id: q }).unwrap().queue_events;
        let last = events.last().unwrap();
        assert_eq!((last.outcome.as_str(), last.action_status.as_str()), ("manual_stop", "suppressed"));
    }

    #[test]
    fn destructive_completion_needs_opt_in_and_countdown() {
        let (service, clock, adapter) = service(1000);
        let q = queue(&service, "Power");
        // The typed command rejects destructive actions without opt-in.
        assert!(!service
            .command(Command::QueueSetCompletionAction {
                queue_id: q,
                action: CompletionAction::Shutdown,
                confirm: false,
                countdown_seconds: None,
            })
            .unwrap()
            .ok);
        // The legacy recorded-only form keeps working for unconfirmed clients.
        assert!(service
            .command(Command::QueueSetCompletion { queue_id: q, action: "shutdown".into() })
            .unwrap()
            .ok);
        service
            .command(Command::QueueSetCompletionAction {
                queue_id: q,
                action: CompletionAction::Shutdown,
                confirm: true,
                countdown_seconds: Some(30),
            })
            .unwrap();
        let id = submit(&service, "p0");
        service.command(Command::AssignQueue { queue_id: q, task_ids: vec![id] }).unwrap();
        service.command(Command::QueueStartOne { queue_id: q }).unwrap();
        service.command(Command::List).unwrap();
        finish(&service, id, "complete", None);
        service.command(Command::List).unwrap();
        assert_eq!(summary(&service, q).run_state, "completed");
        assert_eq!(adapter.0.load(Ordering::Relaxed), 0, "nothing runs during the countdown");
        {
            let events = service.command(Command::QueueEvents { queue_id: q }).unwrap().queue_events;
            assert_eq!(events[0].action_status, "pending");
        }
        let status = service.command(Command::CompletionStatus).unwrap();
        assert_eq!(status.completion_pending.len(), 1);
        let pending = &status.completion_pending[0];
        assert_eq!((pending.queue_id, pending.action.as_str()), (q, "shutdown"));
        assert!((1..=30).contains(&pending.seconds_remaining));
        // The countdown elapses: the adapter executes once and the event turns
        // into the adapter's outcome.
        clock.set(1030);
        service.command(Command::List).unwrap();
        assert_eq!(adapter.0.load(Ordering::Relaxed), 1);
        let events = service.command(Command::QueueEvents { queue_id: q }).unwrap().queue_events;
        assert_eq!(events[0].action_status, "fake");
        assert!(service.command(Command::CompletionStatus).unwrap().completion_pending.is_empty());
    }

    #[test]
    fn countdown_is_canceled_or_suppressed_without_execution() {
        let (service, clock, adapter) = service(1000);
        let q = queue(&service, "Cancel");
        service
            .command(Command::QueueSetCompletionAction {
                queue_id: q,
                action: CompletionAction::Shutdown,
                confirm: true,
                countdown_seconds: Some(60),
            })
            .unwrap();
        let arm = |service: &Service, name: &str| {
            let id = submit(service, name);
            service.command(Command::AssignQueue { queue_id: q, task_ids: vec![id] }).unwrap();
            service.command(Command::QueueStartOne { queue_id: q }).unwrap();
            service.command(Command::List).unwrap();
            finish(service, id, "complete", None);
            service.command(Command::List).unwrap();
            assert_eq!(service.command(Command::CompletionStatus).unwrap().completion_pending.len(), 1);
        };
        // Explicit cancel before the countdown elapses.
        arm(&service, "c-first");
        assert!(service.command(Command::CompletionCancel { queue_id: q }).unwrap().ok);
        assert!(!service.command(Command::CompletionCancel { queue_id: q }).unwrap().ok);
        clock.set(2000);
        service.command(Command::List).unwrap();
        assert_eq!(adapter.0.load(Ordering::Relaxed), 0);
        let events = service.command(Command::QueueEvents { queue_id: q }).unwrap().queue_events;
        assert_eq!(events[0].action_status, "cancelled");
        // A manual stop of the queue suppresses the next armed countdown.
        arm(&service, "c-second");
        service.command(Command::QueueStopOne { queue_id: q }).unwrap();
        clock.set(3000);
        service.command(Command::List).unwrap();
        assert_eq!(adapter.0.load(Ordering::Relaxed), 0);
        let events = service.command(Command::QueueEvents { queue_id: q }).unwrap().queue_events;
        assert_eq!(events.last().unwrap().action_status, "suppressed");
        assert!(service.command(Command::CompletionStatus).unwrap().completion_pending.is_empty());
    }

    #[test]
    fn new_cycle_and_final_failure_suppress_pending_countdown() {
        let (service, clock, adapter) = service(1000);
        let q = queue(&service, "Suppressed");
        service
            .command(Command::QueueSetCompletionAction {
                queue_id: q,
                action: CompletionAction::Shutdown,
                confirm: true,
                countdown_seconds: Some(60),
            })
            .unwrap();
        let first = submit(&service, "s-first");
        service.command(Command::AssignQueue { queue_id: q, task_ids: vec![first] }).unwrap();
        service.command(Command::QueueStartOne { queue_id: q }).unwrap();
        service.command(Command::List).unwrap();
        finish(&service, first, "complete", None);
        service.command(Command::List).unwrap();
        assert_eq!(service.command(Command::CompletionStatus).unwrap().completion_pending.len(), 1);
        // Starting a new cycle (and, before it can start, any final failure of
        // its tasks) suppresses the countdown instead of powering off.
        let second = submit(&service, "s-second");
        service.command(Command::AssignQueue { queue_id: q, task_ids: vec![second] }).unwrap();
        finish(&service, second, "failed", Some("HTTP 404"));
        service.command(Command::List).unwrap();
        assert!(service.command(Command::CompletionStatus).unwrap().completion_pending.is_empty());
        clock.set(3000);
        service.command(Command::List).unwrap();
        assert_eq!(adapter.0.load(Ordering::Relaxed), 0);
        let events = service.command(Command::QueueEvents { queue_id: q }).unwrap().queue_events;
        assert_eq!(events[0].action_status, "suppressed");
    }

    #[test]
    fn completion_setting_validates_actions_and_edit_cancels() {
        let (service, _, _) = service(1000);
        let q = queue(&service, "Editing");
        let set = |action, confirm, countdown| {
            service
                .command(Command::QueueSetCompletionAction {
                    queue_id: q,
                    action,
                    confirm,
                    countdown_seconds: countdown,
                })
                .unwrap()
                .ok
        };
        let program = |path: &str, args: &[&str]| CompletionAction::Program {
            program: path.into(),
            args: args.iter().map(|arg| (*arg).into()).collect(),
        };
        assert!(!set(program("", &[]), true, None), "empty program path");
        assert!(!set(program("/long", &[""; 65][..]), true, None), "too many arguments");
        assert!(!set(CompletionAction::Shutdown, true, Some(0)), "zero countdown");
        assert!(!set(CompletionAction::Shutdown, true, Some(86_401)), "countdown cap");
        assert!(set(program("/usr/local/bin/report.sh", &["hello world", "a;b|>$HOME"]), false, Some(5)));
        let row = summary(&service, q);
        assert_eq!(
            row.completion_action,
            program("/usr/local/bin/report.sh", &["hello world", "a;b|>$HOME"]).encode()
        );
        assert!(!set(CompletionAction::Shutdown, false, None), "still needs opt-in");
        assert!(set(CompletionAction::Exit, false, None));
        assert!(!service
            .command(Command::QueueSetCompletion { queue_id: q, action: "format-disk".into() })
            .unwrap()
            .ok);
        assert!(!service
            .command(Command::QueueSetCompletion { queue_id: 99, action: "notify".into() })
            .unwrap()
            .ok);
        // Editing the action while a countdown runs cancels it.
        service
            .command(Command::QueueSetCompletionAction {
                queue_id: q,
                action: CompletionAction::Shutdown,
                confirm: true,
                countdown_seconds: Some(60),
            })
            .unwrap();
        let id = submit(&service, "e-p");
        service.command(Command::AssignQueue { queue_id: q, task_ids: vec![id] }).unwrap();
        service.command(Command::QueueStartOne { queue_id: q }).unwrap();
        service.command(Command::List).unwrap();
        finish(&service, id, "complete", None);
        service.command(Command::List).unwrap();
        assert_eq!(service.command(Command::CompletionStatus).unwrap().completion_pending.len(), 1);
        service
            .command(Command::QueueSetCompletionAction {
                queue_id: q,
                action: CompletionAction::Notify,
                confirm: false,
                countdown_seconds: None,
            })
            .unwrap();
        assert!(service.command(Command::CompletionStatus).unwrap().completion_pending.is_empty());
        let events = service.command(Command::QueueEvents { queue_id: q }).unwrap().queue_events;
        assert_eq!(events[0].action_status, "cancelled");
    }

    #[test]
    fn pending_countdown_is_suppressed_across_restart() {
        let path = std::env::temp_dir().join(format!("xdm-pending-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let first = Service::open_db(&path, Arc::new(FakeClock::new(1000, 0)), Arc::new(RecordOnly)).unwrap();
        first
            .command(Command::QueueSetCompletionAction {
                queue_id: 1,
                action: CompletionAction::Shutdown,
                confirm: true,
                countdown_seconds: Some(60),
            })
            .unwrap();
        let id = submit(&first, "r-pending");
        first.command(Command::QueueStop).unwrap();
        first.command(Command::QueueStart).unwrap();
        first.command(Command::List).unwrap();
        {
            let mut inner = first.inner.lock().unwrap();
            inner.finish_task(id, "complete", None, None, None).unwrap();
        }
        first.command(Command::List).unwrap();
        assert_eq!(first.command(Command::CompletionStatus).unwrap().completion_pending.len(), 1);
        drop(first);
        let second = Service::open_db(&path, Arc::new(FakeClock::new(1000, 0)), Arc::new(RecordOnly)).unwrap();
        assert!(second.command(Command::CompletionStatus).unwrap().completion_pending.is_empty());
        let events = second.command(Command::QueueEvents { queue_id: 1 }).unwrap().queue_events;
        assert_eq!(events[0].action_status, "suppressed");
        drop(second);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn global_stop_dominates_independent_starts() {
        let (service, _, _) = service(1000);
        let q = queue(&service, "Solo");
        service.command(Command::QueueStop).unwrap();
        service.command(Command::QueueStartOne { queue_id: q }).unwrap();
        let row = summary(&service, q);
        assert!(row.running && row.started_alone);
        assert!(!summary(&service, 1).started_alone);
        service.command(Command::QueueStop).unwrap();
        let row = summary(&service, q);
        assert!(!row.started_alone);
    }

    #[test]
    fn scheduling_global_start_only_arms_the_deadline() {
        let (service, _, _) = service(1000);
        let q = queue(&service, "Armed");
        let id = submit(&service, "a0");
        service.command(Command::AssignQueue { queue_id: q, task_ids: vec![id] }).unwrap();
        service.command(Command::QueueStartOne { queue_id: q }).unwrap();
        // One busy worker in the independently started queue.
        let stop = Arc::new(AtomicBool::new(false));
        let stop_requested = Arc::new(AtomicBool::new(false));
        service.inner.lock().unwrap().active.insert(
            id,
            Active {
                queue_id: q,
                stop: stop.clone(),
                stop_requested: stop_requested.clone(),
                control: None,
                ftp: None,
                media: None,
            },
        );
        let at = unix_now() + 60;
        assert!(
            service
                .command(Command::QueueScheduleStart { at_unix: at })
                .unwrap()
                .ok
        );
        // Arming a future start records only the deadline: the gate, the
        // independent start and the busy worker keep running.
        let list = service.command(Command::List).unwrap();
        assert_eq!((list.queue_running, list.queue_start_at), (Some(true), Some(at)));
        let row = summary(&service, q);
        assert!(row.running && row.started_alone);
        assert!(!stop.load(Ordering::Relaxed));
        let inner = service.inner.lock().unwrap();
        assert!(inner.queue_interrupted.is_empty());
    }

    #[test]
    fn schedule_edit_to_earlier_stop_stops_running_queue_at_once() {
        let (service, clock, _) = service(10_000);
        let q = queue(&service, "Edited");
        let id = submit(&service, "e0");
        service.command(Command::AssignQueue { queue_id: q, task_ids: vec![id] }).unwrap();
        // 02:46 local: a 01:40-03:20 window is open and starts the queue.
        service
            .command(Command::QueueSetSchedule { queue_id: q, kind: "daily".into(), start: 100, stop: Some(200), days: 0x7f })
            .unwrap();
        service.command(Command::List).unwrap();
        let row = summary(&service, q);
        assert!(row.running && row.started_alone && row.run_state == "running");
        // Editing the stop to 02:30, already past, must stop the queue on
        // this command's tick: the closed window can never re-arm a cutoff.
        service
            .command(Command::QueueSetSchedule { queue_id: q, kind: "daily".into(), start: 100, stop: Some(150), days: 0x7f })
            .unwrap();
        let row = summary(&service, q);
        assert!(!row.running && !row.started_alone);
        assert_eq!(row.run_state, "stopped");
        let events = service.command(Command::QueueEvents { queue_id: q }).unwrap().queue_events;
        assert_eq!(events.last().map(|event| event.outcome.as_str()), Some("schedule_stop"));
        // Extending the stop instead keeps the queue running to the new end.
        clock.set(10_500);
        service
            .command(Command::QueueSetSchedule { queue_id: q, kind: "daily".into(), start: 100, stop: Some(240), days: 0x7f })
            .unwrap();
        assert!(summary(&service, q).running);
        clock.set(14_400);
        service.command(Command::List).unwrap();
        let row = summary(&service, q);
        assert!(!row.running && row.run_state == "stopped");
    }

    #[test]
    fn moving_pending_retry_into_no_retry_queue_cancels_it() {
        let (service, clock, _) = service(1000);
        let lenient = queue(&service, "Lenient");
        let strict = queue(&service, "Strict");
        service
            .command(Command::QueueSetRetry { queue_id: lenient, max_retries: 3, delay_seconds: 60 })
            .unwrap();
        let id = submit(&service, "r0");
        service.command(Command::AssignQueue { queue_id: lenient, task_ids: vec![id] }).unwrap();
        finish(&service, id, "failed", Some("connection refused"));
        // The transient failure arms one retry under the lenient queue.
        assert_eq!(task_state(&service, id), ("queued".into(), 1, Some(1060), lenient));
        // Moving into a no-retry queue cancels the pending retry: the
        // failure stands instead of firing one more attempt at the armed time.
        service.command(Command::AssignQueue { queue_id: strict, task_ids: vec![id] }).unwrap();
        assert_eq!(task_state(&service, id), ("failed".into(), 0, None, strict));
        clock.set(1200);
        service.command(Command::List).unwrap();
        assert_eq!(task_state(&service, id).0, "failed");
        // A target queue that would still retry keeps the armed retry.
        let other = submit(&service, "r1");
        service.command(Command::AssignQueue { queue_id: lenient, task_ids: vec![other] }).unwrap();
        finish(&service, other, "failed", Some("connection refused"));
        let also = queue(&service, "Also");
        service
            .command(Command::QueueSetRetry { queue_id: also, max_retries: 3, delay_seconds: 5 })
            .unwrap();
        service.command(Command::AssignQueue { queue_id: also, task_ids: vec![other] }).unwrap();
        assert_eq!(task_state(&service, other), ("queued".into(), 1, Some(1260), also));
    }
}
