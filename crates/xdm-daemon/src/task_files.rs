//! Paged task listing, completed-file moves and orphan partial cleanup.
use crate::{Result, Service, query_summaries};
use rusqlite::types::Value;
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use xdm_ipc::{
    BulkAction, BulkFailure, BulkReport, BulkSelection, Command, PartialEntry, Reply, TaskFilter,
};

const MAX_PARTIALS: usize = 10_000;
const MAX_META_BYTES: u64 = 4 * 1024 * 1024;

const MAX_PAGE: u64 = 2000;

fn sibling(output: &Path, suffix: &str) -> PathBuf {
    PathBuf::from(format!("{}{suffix}", output.display()))
}

/// SQL `CASE` that substitutes live values (taken from running transfers) for the
/// persisted last-known column; ids and numbers are formatted from integers only.
fn live_case(live: &[(i64, String)], fallback: &str) -> String {
    if live.is_empty() {
        return fallback.to_owned();
    }
    let arms: String = live
        .iter()
        .map(|(id, value)| format!(" WHEN {id} THEN {value}"))
        .collect();
    format!("CASE id{arms} ELSE {fallback} END")
}

fn sort_clause(sort: Option<&str>, inner: &crate::Inner) -> Option<String> {
    let raw = sort.unwrap_or("-id");
    let (descending, name) = match raw.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, raw.strip_prefix('+').unwrap_or(raw)),
    };
    let live = |value: &dyn Fn(xdm_http::Progress) -> String| -> Vec<(i64, String)> {
        inner
            .active
            .iter()
            .filter_map(|(id, active)| {
                let progress = match (&active.control, &active.ftp) {
                    (Some(control), _) => Some(control.progress()),
                    (None, Some(ftp)) => Some(ftp.progress()),
                    _ => None,
                }?;
                Some((*id, value(progress)))
            })
            .collect()
    };
    let column = match name {
        "id" => "id".to_owned(),
        "url" => "url COLLATE NOCASE".to_owned(),
        "output" => "output COLLATE NOCASE".to_owned(),
        "status" => "status".to_owned(),
        "category" => "category COLLATE NOCASE".to_owned(),
        "priority" => "priority".to_owned(),
        "position" => "position".to_owned(),
        "created" => "created_at".to_owned(),
        "updated" => "updated_at".to_owned(),
        // Total bytes: the live length while running, else the last known one.
        "size" => live_case(
            &live(&|p| p.total.unwrap_or(0).to_string()),
            "COALESCE(size_total,0)",
        ),
        // downloaded/total as a fraction, so unknown totals sort first.
        "progress" => live_case(
            &live(&|p| match p.total {
                Some(total) if total > 0 => format!("{:.6}", p.downloaded as f64 / total as f64),
                _ => "0.0".to_owned(),
            }),
            "CASE WHEN COALESCE(size_total,0)>0 THEN CAST(COALESCE(size_done,0) AS REAL)/size_total ELSE 0.0 END",
        ),
        // Live transfer rate; idle tasks count as 0.
        // A bare `0` would read as an ORDER BY column index, so it is cast.
        "rate" => live_case(&live(&|p| p.bytes_per_second.to_string()), "CAST(0 AS INTEGER)"),
        "last_try" => "COALESCE(last_try_at,0)".to_owned(),
        "queue" => "COALESCE((SELECT name FROM queues WHERE id=COALESCE(NULLIF(tasks.queue_id,0),tasks.history_queue_id)),'') COLLATE NOCASE".to_owned(),
        _ => return None,
    };
    // The id tiebreak keeps pages stable while other columns tie.
    Some(format!(
        "{column} {}, id DESC",
        if descending { "DESC" } else { "ASC" }
    ))
}

fn like_escape(text: &str) -> String {
    let mut out = String::from("%");
    for ch in text.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('%');
    out
}

/// Opens a source file for reading without following a final symlink and
/// requires a regular file.
fn open_regular(path: &Path) -> io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    if !file.metadata()?.file_type().is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a regular file"));
    }
    Ok(file)
}

/// Moves `from` to a destination that must not exist. A hard link publishes
/// atomically on one volume; otherwise the data is copied into a destination
/// reserved with `create_new`, so an existing file is never replaced. If the
/// source cannot be removed afterwards the new copy is rolled back.
fn move_no_overwrite(from: &Path, to: &Path) -> io::Result<()> {
    if std::fs::symlink_metadata(to).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "destination already exists",
        ));
    }
    open_regular(from)?;
    match std::fs::hard_link(from, to) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Err(error),
        Err(_) => {
            let mut source = open_regular(from)?;
            let mut target = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(to)?;
            let copied = io::copy(&mut source, &mut target).and_then(|_| target.sync_all());
            if let Err(error) = copied {
                drop(target);
                let _ = std::fs::remove_file(to);
                return Err(error);
            }
        }
    }
    if let Err(error) = std::fs::remove_file(from) {
        let _ = std::fs::remove_file(to);
        return Err(error);
    }
    Ok(())
}

fn regular_size(path: &Path) -> Option<u64> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    meta.file_type().is_file().then(|| meta.len())
}

/// True when `path` is an XDM checkpoint: bounded JSON with a version, a
/// URL and a segment list (the xdm-http `Checkpoint` shape).
fn is_xdm_checkpoint(path: &Path) -> bool {
    use std::io::Read;
    let Ok(file) = open_regular(path) else {
        return false;
    };
    let mut bytes = Vec::new();
    if file.take(MAX_META_BYTES + 1).read_to_end(&mut bytes).is_err()
        || bytes.len() as u64 > MAX_META_BYTES
    {
        return false;
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return false;
    };
    value.get("version").is_some_and(|v| v.is_u64())
        && value.get("url").is_some_and(|v| v.is_string())
        && value.get("segments").is_some_and(|v| v.is_array())
}

/// Tasks that currently own or work on `output`, other than `except`.
fn output_busy(inner: &crate::Inner, output: &str, except: i64) -> Result<bool> {
    let owned: bool = inner.db.query_row(
        "SELECT EXISTS(SELECT 1 FROM tasks WHERE output=?1 AND id<>?2)",
        rusqlite::params![output, except],
        |row| row.get(0),
    )?;
    if owned {
        return Ok(true);
    }
    for id in inner.active.keys().filter(|id| **id != except) {
        let active_output: Option<String> = rusqlite::OptionalExtension::optional(
            inner.db.query_row("SELECT output FROM tasks WHERE id=?1", [id], |row| row.get(0)),
        )?;
        if active_output.as_deref() == Some(output) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn normalized_destination(raw: &str) -> Option<PathBuf> {
    let path = Path::new(raw);
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return None;
    }
    // components() drops `.` segments and doubled or trailing separators.
    let normalized: PathBuf = path.components().collect();
    let name = normalized.file_name()?.to_str()?;
    xdm_categories::valid_filename(name).then_some(normalized)
}

/// `WHERE` tail and bound values for a task filter.
fn filter_clause(filter: TaskFilter) -> (String, Vec<Value>) {
    let mut clauses = Vec::new();
    let mut values: Vec<Value> = Vec::new();
    if let Some(status) = filter.status {
        // "finished"/"unfinished" group every status for the GUI category tree.
        match status.as_str() {
            "finished" => clauses.push("status='complete'"),
            "unfinished" => clauses.push("status<>'complete'"),
            _ => {
                clauses.push("status=?");
                values.push(Value::Text(status));
            }
        }
    }
    if let Some(category) = filter.category {
        clauses.push("category=? COLLATE NOCASE");
        values.push(Value::Text(category));
    }
    if let Some(queue) = filter.queue_id {
        clauses.push("queue_id=?");
        values.push(Value::Integer(queue));
    }
    if let Some(search) = filter.search.filter(|text| !text.is_empty()) {
        clauses.push("(url LIKE ? ESCAPE '\\' OR output LIKE ? ESCAPE '\\')");
        values.push(Value::Text(like_escape(&search)));
        values.push(Value::Text(like_escape(&search)));
    }
    let wher = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };
    (wher, values)
}

impl Service {
    /// `TaskBulk`: resolve the selection once, then run the single-task
    /// command per task so every rule and failure message is unchanged.
    pub(crate) fn task_bulk(
        &self,
        action: BulkAction,
        selection: BulkSelection,
        expected_total: Option<u64>,
    ) -> Result<Reply> {
        let ids: Vec<i64> = match selection {
            BulkSelection::Ids(ids) => {
                let mut seen = HashSet::new();
                ids.into_iter().filter(|id| seen.insert(*id)).collect()
            }
            BulkSelection::Filter(filter) => {
                let (wher, values) = filter_clause(filter);
                let inner = self
                    .inner
                    .lock()
                    .map_err(|_| io::Error::other("task lock poisoned"))?;
                let mut stmt = inner
                    .db
                    .prepare(&format!("SELECT id FROM tasks {wher} ORDER BY id"))?;
                let rows = stmt.query_map(rusqlite::params_from_iter(values.iter()), |row| {
                    row.get::<_, i64>(0)
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            }
        };
        if let Some(expected) = expected_total {
            if expected != ids.len() as u64 {
                return Ok(Reply::error(format!(
                    "selection changed: {} tasks match now but {expected} were expected; nothing was changed",
                    ids.len()
                )));
            }
        }
        if let BulkAction::Priority { level } = &action {
            if !(0..=2).contains(level) {
                return Ok(Reply::error(
                    "priority must be 0 (low), 1 (normal), or 2 (immediate)",
                ));
            }
        }
        let mut report = BulkReport {
            matched: ids.len() as u64,
            ..BulkReport::default()
        };
        // Pause, resume and priority are single UPDATEs: one transaction (one
        // commit) for the whole selection, with the single command's SQL and messages.
        if matches!(action, BulkAction::Pause | BulkAction::Resume | BulkAction::Priority { .. }) {
            let mut inner = self
                .inner
                .lock()
                .map_err(|_| io::Error::other("task lock poisoned"))?;
            if matches!(action, BulkAction::Pause) {
                for id in &ids {
                    inner.queue_interrupted.remove(id);
                    if let Some(active) = inner.active.get(id) {
                        active.stop.store(true, std::sync::atomic::Ordering::Relaxed);
                        active.stop_requested.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            }
            let (sql, refusal) = match &action {
                BulkAction::Pause => ("UPDATE tasks SET status=CASE WHEN status='running' THEN 'pausing' ELSE 'paused' END, updated_at=CURRENT_TIMESTAMP WHERE id=?1 AND status IN ('queued','running','quota_wait')", "task is not active or queued"),
                BulkAction::Resume => ("UPDATE tasks SET status='queued',error=NULL,retry_count=0,retry_at=NULL,updated_at=CURRENT_TIMESTAMP WHERE id=?1 AND status IN ('paused','failed')", "task is not paused or failed"),
                _ => ("UPDATE tasks SET priority=?2,updated_at=CURRENT_TIMESTAMP WHERE id=?1 AND status IN ('queued','paused','failed','quota_wait')", "task must be queued, paused, or failed to change priority"),
            };
            let level = if let BulkAction::Priority { level } = &action { *level } else { 0 };
            let transaction = inner.db.transaction()?;
            {
                let mut statement = transaction.prepare(sql)?;
                for id in &ids {
                    let updated = if matches!(action, BulkAction::Priority { .. }) {
                        statement.execute(rusqlite::params![id, level])?
                    } else {
                        statement.execute(rusqlite::params![id])?
                    };
                    if updated == 0 {
                        report.failed += 1;
                        if report.failures.len() < 100 {
                            report.failures.push(BulkFailure { task_id: *id, error: refusal.into() });
                        }
                    } else {
                        report.done += 1;
                    }
                }
            }
            transaction.commit()?;
            drop(inner);
            self.wake.notify_one();
            let mut reply = Reply::list(Vec::new());
            reply.total = report.matched;
            reply.bulk = Some(report);
            return Ok(reply);
        }
        // Queue moves are chunked like the single command's 500-task limit; a
        // refused chunk is retried task by task so failures stay per task.
        if let BulkAction::AssignQueue { queue_id } = &action {
            for chunk in ids.chunks(500) {
                let reply = self.command_inner(Command::AssignQueue {
                    queue_id: *queue_id,
                    task_ids: chunk.to_vec(),
                })?;
                if reply.ok {
                    report.done += chunk.len() as u64;
                    continue;
                }
                for id in chunk {
                    let single = self.command_inner(Command::AssignQueue {
                        queue_id: *queue_id,
                        task_ids: vec![*id],
                    })?;
                    if single.ok {
                        report.done += 1;
                    } else {
                        report.failed += 1;
                        if report.failures.len() < 100 {
                            report.failures.push(BulkFailure {
                                task_id: *id,
                                error: single.error.unwrap_or_default(),
                            });
                        }
                    }
                }
            }
            let mut reply = Reply::list(Vec::new());
            reply.total = report.matched;
            reply.bulk = Some(report);
            return Ok(reply);
        }
        if matches!(action, BulkAction::Remove | BulkAction::RemoveWithPartials) {
            self.bulk_remove(&ids, matches!(action, BulkAction::RemoveWithPartials), &mut report)?;
            let mut reply = Reply::list(Vec::new());
            reply.total = report.matched;
            reply.bulk = Some(report);
            return Ok(reply);
        }
        for id in ids {
            let command = match &action {
                BulkAction::Pause => Command::Pause { task_id: id },
                BulkAction::Resume => Command::Resume { task_id: id },
                BulkAction::Remove => Command::Remove { task_id: id },
                BulkAction::RemoveWithPartials => Command::TaskRemoveEx {
                    task_id: id,
                    delete_partial: true,
                },
                BulkAction::Priority { level } => Command::SetPriority {
                    task_id: id,
                    priority: *level,
                },
                BulkAction::AssignQueue { queue_id } => Command::AssignQueue {
                    queue_id: *queue_id,
                    task_ids: vec![id],
                },
                BulkAction::Restart => Command::TaskRestart { task_id: id },
            };
            let reply = self.command_inner(command)?;
            if reply.ok {
                report.done += 1;
            } else {
                report.failed += 1;
                if report.failures.len() < 100 {
                    report.failures.push(BulkFailure {
                        task_id: id,
                        error: reply.error.unwrap_or_default(),
                    });
                }
            }
        }
        let mut reply = Reply::list(Vec::new());
        reply.total = report.matched;
        reply.bulk = Some(report);
        Ok(reply)
    }

    pub(crate) fn list_page(
        &self,
        offset: u64,
        limit: u64,
        sort: Option<String>,
        filter: Option<TaskFilter>,
    ) -> Result<Reply> {
        if limit == 0 || limit > MAX_PAGE {
            return Ok(Reply::error("limit must be 1..=2000"));
        }
        let (wher, mut values) = filter_clause(filter.unwrap_or_default());
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        let Some(order) = sort_clause(sort.as_deref(), &inner) else {
            return Ok(Reply::error("unknown sort column"));
        };
        let total: i64 = inner.db.query_row(
            &format!("SELECT COUNT(*) FROM tasks {wher}"),
            rusqlite::params_from_iter(values.iter()),
            |row| row.get(0),
        )?;
        values.push(Value::Integer(limit as i64));
        values.push(Value::Integer(offset.min(i64::MAX as u64) as i64));
        let tasks = query_summaries(
            &inner,
            &format!("{wher} ORDER BY {order} LIMIT ? OFFSET ?"),
            rusqlite::params_from_iter(values.iter()),
        )?;
        let mut reply = Reply::list(tasks);
        reply.total = total as u64;
        reply.daemon_version = Some(env!("CARGO_PKG_VERSION").to_owned());
        Ok(reply)
    }

    pub(crate) fn move_task_file(&self, task_id: i64, new_path: String) -> Result<Reply> {
        let Some(to) = normalized_destination(&new_path) else {
            return Ok(Reply::error(
                "new_path must be an absolute path without .. and end in a valid file name",
            ));
        };
        let new_path = to.to_string_lossy().into_owned();
        let from = {
            let mut inner = self
                .inner
                .lock()
                .map_err(|_| io::Error::other("task lock poisoned"))?;
            if inner.active.contains_key(&task_id) {
                return Ok(Reply::error("task is running"));
            }
            if inner.moving.contains(&task_id) {
                return Ok(Reply::error("task file is already being moved"));
            }
            let row: Option<(String, String)> = rusqlite::OptionalExtension::optional(
                inner.db.query_row(
                    "SELECT output,status FROM tasks WHERE id=?1",
                    [task_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                ),
            )?;
            let Some((output, status)) = row else {
                return Ok(Reply::error("task not found"));
            };
            if status != "complete" {
                return Ok(Reply::error("only completed tasks can be moved"));
            }
            let taken: bool = inner.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM tasks WHERE output=?1)",
                [&new_path],
                |row| row.get(0),
            )?;
            if taken {
                return Ok(Reply::error("another task already owns that path"));
            }
            if output == new_path {
                return Ok(Reply::error("destination is the current path"));
            }
            inner.moving.insert(task_id);
            PathBuf::from(output)
        };
        let reply = self.move_unlocked(task_id, &from, &to, &new_path);
        if let Ok(mut inner) = self.inner.lock() {
            inner.moving.remove(&task_id);
        }
        reply
    }

    fn move_unlocked(&self, task_id: i64, from: &Path, to: &Path, new_path: &str) -> Result<Reply> {
        if !to.parent().is_some_and(Path::is_dir) {
            return Ok(Reply::error("destination directory does not exist"));
        }
        // Disk work runs without the daemon lock (rule 6); `moving` fences
        // the task meanwhile.
        if let Err(error) = move_no_overwrite(from, to) {
            return Ok(Reply::error(format!("move failed: {error}")));
        }
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        let updated = inner.db.execute(
            "UPDATE tasks SET output=?1, updated_at=CURRENT_TIMESTAMP WHERE id=?2 AND status='complete' AND output=?3",
            rusqlite::params![new_path, task_id, from.to_string_lossy()],
        );
        if matches!(updated, Ok(1)) {
            return Ok(Reply::accepted(task_id));
        }
        Ok(match move_no_overwrite(to, from) {
            Ok(()) => Reply::error("task record could not be updated; file restored to its old path"),
            Err(error) => Reply::error(format!(
                "task record could not be updated and restoring failed ({error}); file left at {new_path}; task record still points at {}",
                from.display()
            )),
        })
    }

    /// Removes a task. Unlinks happen while the lock is held, after the
    /// owner and activity re-check, so no other task can claim the files in
    /// between.
    pub(crate) fn remove_task(&self, task_id: i64, delete_partial: bool) -> Result<Reply> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        remove_locked(&mut inner, task_id, delete_partial)
    }

    /// `TaskBulk` removal: one lock hold and one commit for the selection,
    /// each task under `remove_locked`'s rules inside its own savepoint.
    fn bulk_remove(&self, ids: &[i64], delete_partial: bool, report: &mut BulkReport) -> Result<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        inner.db.execute_batch("BEGIN IMMEDIATE")?;
        for id in ids {
            let reply = match remove_locked(&mut inner, *id, delete_partial) {
                Ok(reply) => reply,
                Err(error) => {
                    let _ = inner.db.execute_batch("ROLLBACK");
                    return Err(error);
                }
            };
            if reply.ok {
                report.done += 1;
            } else {
                report.failed += 1;
                if report.failures.len() < 100 {
                    report.failures.push(BulkFailure {
                        task_id: *id,
                        error: reply.error.unwrap_or_default(),
                    });
                }
            }
        }
        inner.db.execute_batch("COMMIT")?;
        Ok(())
    }

    /// Task outputs as raw strings, taken under the lock; callers resolve
    /// them to canonical paths after releasing it.
    fn task_output_strings(&self) -> Result<Vec<String>> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        let mut statement = inner.db.prepare("SELECT output FROM tasks")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Partial files of tasks that assemble in a temporary directory, and the
    /// current temp setting, taken under the lock.
    fn temp_partial_state(&self) -> Result<(HashSet<PathBuf>, Option<PathBuf>)> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        let mut statement = inner
            .db
            .prepare("SELECT output,temp_dir FROM tasks WHERE temp_dir IS NOT NULL AND temp_dir<>''")?;
        let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
        let mut owned = HashSet::new();
        for (output, temp) in rows.flatten() {
            owned.extend(crate::owned_partial_paths(Path::new(&output), Some(&temp)));
        }
        Ok((owned, inner.temp_dir.clone().map(PathBuf::from)))
    }

    /// Unowned XDM checkpoints inside the temporary directory.
    fn scan_temp_partials(
        &self,
        skip: &Path,
        found: &mut Vec<PartialEntry>,
        truncated: &mut bool,
    ) -> Result<()> {
        let (owned, temp) = self.temp_partial_state()?;
        let Some(temp) = temp.and_then(|dir| std::fs::canonicalize(dir).ok()) else {
            return Ok(());
        };
        if temp == skip {
            return Ok(());
        }
        for entry in std::fs::read_dir(&temp)?.flatten() {
            let meta_file = entry.path();
            let Some(stem) = meta_file
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix(".part.meta"))
                .filter(|stem| !stem.is_empty())
                .map(str::to_owned)
            else {
                continue;
            };
            let part = temp.join(format!("{stem}.part"));
            if owned.contains(&meta_file) || owned.contains(&part) {
                continue;
            }
            let Some(meta_size) = regular_size(&meta_file) else {
                continue;
            };
            if !is_xdm_checkpoint(&meta_file) {
                continue;
            }
            if found.len() >= MAX_PARTIALS {
                *truncated = true;
                break;
            }
            let part_size = regular_size(&part);
            found.push(PartialEntry {
                output: temp.join(&stem).to_string_lossy().into_owned(),
                part: part_size.map(|_| part.to_string_lossy().into_owned()),
                part_size: part_size.unwrap_or(0),
                meta: Some(meta_file.to_string_lossy().into_owned()),
                meta_size,
            });
        }
        Ok(())
    }

    /// Unowned partial pairs directly in `directory`: a `.part.meta` that
    /// parses as an XDM checkpoint, plus its `.part` if present. A lone
    /// `.part` (a browser's download, say) is never listed. Symlinks and
    /// non-regular files are skipped; at most `MAX_PARTIALS` entries return.
    pub(crate) fn scan_partials(
        &self,
        directory: &str,
    ) -> Result<std::result::Result<(Vec<PartialEntry>, bool), String>> {
        let directory = Path::new(directory);
        if !directory.is_absolute() {
            return Ok(Err("directory must be absolute".into()));
        }
        let Ok(directory) = std::fs::canonicalize(directory) else {
            return Ok(Err("directory does not exist".into()));
        };
        let mut owned = HashSet::new();
        let mut resolved: std::collections::HashMap<PathBuf, bool> = Default::default();
        for output in self.task_output_strings()? {
            let output = PathBuf::from(output);
            let (Some(parent), Some(name)) = (output.parent(), output.file_name()) else {
                continue;
            };
            let same = *resolved.entry(parent.to_path_buf()).or_insert_with(|| {
                parent == directory
                    || std::fs::canonicalize(parent).ok().as_deref() == Some(directory.as_path())
            });
            if same {
                owned.insert(directory.join(name));
            }
        }
        let (temp_owned, _) = self.temp_partial_state()?;
        let mut found = Vec::new();
        let mut truncated = false;
        for entry in std::fs::read_dir(&directory)?.flatten() {
            let meta_file = entry.path();
            let Some(base) = meta_file
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix(".part.meta"))
                .filter(|base| !base.is_empty())
                .map(str::to_owned)
            else {
                continue;
            };
            let output = directory.join(&base);
            if owned.contains(&output)
                || temp_owned.contains(&meta_file)
                || temp_owned.contains(&directory.join(format!("{base}.part")))
            {
                continue;
            }
            let Some(meta_size) = regular_size(&meta_file) else {
                continue;
            };
            if !is_xdm_checkpoint(&meta_file) {
                continue;
            }
            if found.len() >= MAX_PARTIALS {
                truncated = true;
                break;
            }
            let part = sibling(&output, ".part");
            let part_size = regular_size(&part);
            found.push(PartialEntry {
                output: output.to_string_lossy().into_owned(),
                part: part_size.map(|_| part.to_string_lossy().into_owned()),
                part_size: part_size.unwrap_or(0),
                meta: Some(meta_file.to_string_lossy().into_owned()),
                meta_size,
            });
        }
        self.scan_temp_partials(&directory, &mut found, &mut truncated)?;
        found.sort_by(|a, b| a.output.cmp(&b.output));
        Ok(Ok((found, truncated)))
    }

    pub(crate) fn delete_partials(&self, directory: &str, paths: Vec<String>) -> Result<Reply> {
        let found = match self.scan_partials(directory)? {
            Ok((found, _)) => found,
            Err(message) => return Ok(Reply::error(message)),
        };
        let eligible: HashSet<&str> = found
            .iter()
            .flat_map(|entry| [entry.part.as_deref(), entry.meta.as_deref()])
            .flatten()
            .collect();
        if let Some(bad) = paths.iter().find(|path| !eligible.contains(path.as_str())) {
            return Ok(Reply::error(format!(
                "not an unowned partial in the scan: {bad}"
            )));
        }
        // Re-check ownership under the lock right before unlinking: a task
        // submitted since the scan that uses the same file name keeps its files.
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        let mut statement = inner.db.prepare("SELECT output FROM tasks")?;
        let names: HashSet<String> = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .flatten()
            .filter_map(|output| {
                Path::new(&output)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
            })
            .collect();
        let mut temp_owned = HashSet::new();
        {
            let mut statement = inner.db.prepare(
                "SELECT output,temp_dir FROM tasks WHERE temp_dir IS NOT NULL AND temp_dir<>''",
            )?;
            let rows = statement
                .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
            for (output, temp) in rows.flatten() {
                temp_owned.extend(crate::owned_partial_paths(Path::new(&output), Some(&temp)));
            }
        }
        let mut deleted = 0;
        for path in &paths {
            let Some(name) = Path::new(path).file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let base = name
                .strip_suffix(".part.meta")
                .or_else(|| name.strip_suffix(".part"))
                .unwrap_or(name);
            if names.contains(base) || temp_owned.contains(Path::new(path)) {
                continue;
            }
            if regular_size(Path::new(path)).is_some() && std::fs::remove_file(path).is_ok() {
                deleted += 1;
            }
        }
        let mut reply = Reply::list(Vec::new());
        reply.total = deleted;
        Ok(reply)
    }
}


/// Removes one task with the daemon lock held; the caller owns `inner`.
/// Unlinks happen after the owner and activity re-check, so no other task
/// can claim the files in between.
fn remove_locked(inner: &mut crate::Inner, task_id: i64, delete_partial: bool) -> Result<Reply> {
        if inner.active.contains_key(&task_id) {
            return Ok(Reply::error(
                "pause the task and wait for it to stop before removing it",
            ));
        }
        if inner.moving.contains(&task_id) {
            return Ok(Reply::error("task file is being moved"));
        }
        let row: Option<(String, String, Option<String>)> = rusqlite::OptionalExtension::optional(
            inner.db.query_row(
                "SELECT output,status,temp_dir FROM tasks WHERE id=?1",
                [task_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            ),
        )?;
        let shared = match &row {
            Some((output, _, _)) if delete_partial => output_busy(&inner, output, task_id)?,
            _ => false,
        };
        inner.db.execute_batch("SAVEPOINT remove_one")?;
        let removed = inner.db.execute(
            "INSERT INTO removed_tasks(task_id,request_id) SELECT id,request_id FROM tasks WHERE id=?1",
            [task_id],
        );
        let removed = match removed {
            Ok(removed) => removed,
            Err(error) => {
                let _ = inner.db.execute_batch("ROLLBACK TO remove_one; RELEASE remove_one");
                return Err(error.into());
            }
        };
        if removed == 0 {
            inner.db.execute_batch("RELEASE remove_one")?;
            return Ok(Reply::error("task not found"));
        }
        if let Err(error) = inner.db.execute("DELETE FROM tasks WHERE id=?1", [task_id]) {
            let _ = inner.db.execute_batch("ROLLBACK TO remove_one; RELEASE remove_one");
            return Err(error.into());
        }
        inner.db.execute_batch("RELEASE remove_one")?;
        inner.queue_interrupted.remove(&task_id);
    inner.task_logins.remove(&task_id);
        if let (true, false, Some((output, status, temp))) = (delete_partial, shared, row) {
            if status != "complete" {
                for path in crate::owned_partial_paths(Path::new(&output), temp.as_deref()) {
                    if regular_size(&path).is_some() {
                        let _ = std::fs::remove_file(path);
                    }
                }
            }
        }
        Ok(Reply::accepted(task_id))
}

