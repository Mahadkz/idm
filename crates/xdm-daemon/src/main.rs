mod credentials;
mod os_actions;
mod schedule;

use interprocess::local_socket::prelude::*;
use os_actions::CompletionAdapter;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use url::Url;
use xdm_http::{Options, Progress, Proxy, TransferControl, download_with_control};
use schedule::{Clock, Kind, Schedule, SystemClock};
use xdm_ipc::{
    Command, CompletionAction, CompletionPending, GrabProjectRecord, GrabReport, ImportEntry,
    MediaAudioFormat, MediaMode, MoveTarget, Paths, QueueEvent, QueueSummary, Reply, Request,
    TaskSummary, TransferList, read_frame, write_frame,
};

type Error = Box<dyn std::error::Error + Send + Sync>;
type Result<T> = std::result::Result<T, Error>;

struct Inner {
    db: Connection,
    active: HashMap<i64, Active>,
    max_concurrent: usize,
    queue_running: bool,
    queue_start_at: Option<i64>,
    queue_stop_at: Option<i64>,
    queue_interrupted: HashSet<i64>,
    clock: Arc<dyn Clock>,
    adapter: Arc<dyn CompletionAdapter>,
    /// Destructive actions counting down, keyed by queue.
    pending: HashMap<i64, PendingCompletion>,
    /// Global proxy route for HTTP tasks, stored as canonical JSON.
    proxy: String,
    /// Saved site logins for HTTP authentication (N10).
    credentials: credentials::Store,
    /// Live crawls keyed by their site-grab request ID.
    grabs: HashMap<String, GrabHandle>,
    /// Crawls waiting for the worker loop, which owns the service handle.
    pending_grabs: Vec<GrabJob>,
}

/// Default seconds before an opted-in destructive completion action executes.
const DEFAULT_COUNTDOWN_SECS: u64 = 60;

/// A countdown in flight for one queue's finished cycle; the countdown is
/// in-memory, so a daemon restart suppresses rather than surprises.
struct PendingCompletion {
    event_id: i64,
    action: String,
    deadline: i64,
}

/// Executes a queue's recorded completion action; see `os_actions`.
/// The daemon ships a real OS adapter and a JSON-recording fake for tests.

const COMPLETION_ACTIONS: [&str; 4] = ["none", "notify", "sleep", "shutdown"];

const PROXY_SCHEMES: [&str; 5] = ["http://", "https://", "socks4://", "socks5://", "socks5h://"];

/// Methods are strict HTTP tokens here; the engine accepts a superset.
fn valid_method(method: &str) -> bool {
    !method.is_empty() && method.len() <= 16 && method.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// HTTP statuses that retrying cannot fix.
fn is_permanent_error(message: &str) -> bool {
    ["HTTP 401", "HTTP 403", "HTTP 404", "HTTP 410"]
        .iter()
        .any(|status| message.contains(status))
}

/// Web headers a media job replays: bounded count and lengths, no
/// line-breaking that could smuggle further ffmpeg options into the child.
fn valid_media_headers(headers: &[(String, String)]) -> bool {
    headers.len() <= 32
        && headers.iter().all(|(name, value)| {
            !name.is_empty()
                && name.len() <= 128
                && value.len() <= 8192
                && !name.contains(['\r', '\n', '\0'])
                && !value.contains(['\r', '\n', '\0'])
        })
}

/// Fields a `SubmitMediaJob` carries: the manifest-URL and track-index guards
/// of `SubmitMedia`, plus a mode-aware destination extension check.
fn valid_media_request(url: &str, output: &str, mode: &MediaMode) -> bool {
    let manifest = url.split('?').next().unwrap_or("").to_ascii_lowercase();
    let extension = std::path::Path::new(output)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    let bounded = match mode {
        MediaMode::Video {
            video,
            audio,
            subtitles,
        } => {
            *video <= 10000
                && audio.is_none_or(|index| index <= 10000)
                && subtitles.len() <= 16
                && subtitles.iter().all(|index| *index <= 10000)
        }
        MediaMode::Audio { track, .. } => *track <= 10000,
        MediaMode::Subtitles { track } => *track <= 10000,
    };
    (manifest.ends_with(".m3u8") || manifest.ends_with(".mpd"))
        && mode.valid(extension)
        && bounded
}

struct Active {
    queue_id: i64,
    stop: Arc<AtomicBool>,
    control: Option<Arc<TransferControl>>,
    /// Live counters for single-stream FTP transfers.
    ftp: Option<Arc<FtpLive>>,
}

/// Live progress for one FTP transfer, mirroring what the HTTP engine exposes.
#[derive(Debug)]
struct FtpLive {
    downloaded: AtomicU64,
    total: AtomicU64,
    total_known: AtomicBool,
    sample: Mutex<FtpSample>,
}

#[derive(Debug)]
struct FtpSample {
    at: Instant,
    bytes: u64,
    speed: u64,
}

impl Default for FtpLive {
    fn default() -> Self {
        Self {
            downloaded: AtomicU64::new(0),
            total: AtomicU64::new(0),
            total_known: AtomicBool::new(false),
            sample: Mutex::new(FtpSample {
                at: Instant::now(),
                bytes: 0,
                speed: 0,
            }),
        }
    }
}

impl FtpLive {
    /// Called from the FTP worker after every received chunk.
    fn report(&self, downloaded: u64, total: Option<u64>) {
        self.downloaded.store(downloaded, Ordering::Relaxed);
        if let Some(total) = total {
            self.total_known.store(true, Ordering::Relaxed);
            self.total.store(total, Ordering::Relaxed);
        }
    }

    fn progress(&self) -> Progress {
        let downloaded = self.downloaded.load(Ordering::Relaxed);
        let total = self
            .total_known
            .load(Ordering::Relaxed)
            .then(|| self.total.load(Ordering::Relaxed));
        let mut speed = 0;
        if let Ok(mut sample) = self.sample.lock() {
            let elapsed = sample.at.elapsed();
            if elapsed >= Duration::from_millis(250) {
                sample.speed =
                    (downloaded.saturating_sub(sample.bytes) as f64 / elapsed.as_secs_f64()) as u64;
                sample.at = Instant::now();
                sample.bytes = downloaded;
            }
            speed = sample.speed;
        }
        Progress {
            downloaded,
            total,
            bytes_per_second: speed,
            eta_seconds: total
                .filter(|_| speed > 0)
                .map(|total| total.saturating_sub(downloaded).div_ceil(speed)),
        }
    }
}

/// Paused or partially downloaded FTP progress derived from the `.part` file
/// and its `.part.ftp` checkpoint, the FTP analogue of HTTP checkpoints.
fn ftp_checkpoint_progress(output: &Path) -> Option<(u64, Option<u64>)> {
    let encoded = format!("{}.part.ftp", output.display());
    let state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&encoded).ok()?).ok()?;
    let total = state.get("size").and_then(|value| value.as_u64());
    let part = format!("{}.part", output.display());
    Some((std::fs::metadata(&part).ok()?.len(), total))
}

/// A manifest and counters for one background site grab.
struct GrabHandle {
    state: Arc<Mutex<GrabState>>,
}

/// Mutable crawl state shared with the crawl thread.
#[derive(Default)]
struct GrabState {
    state: String,
    files: usize,
    pages: usize,
    errors: usize,
    error: Option<String>,
    task_ids: Vec<i64>,
}

/// A crawl registered for launch by the worker loop, which owns the shared
/// service handle background threads need.
struct GrabJob {
    request_id: String,
    project: xdm_grabber::Project,
    directory: String,
    queue_id: Option<i64>,
    enqueue: bool,
    stop: Arc<AtomicBool>,
    state: Arc<Mutex<GrabState>>,
}

impl GrabState {
    fn report(&self, request_id: &str) -> GrabReport {
        GrabReport {
            request_id: request_id.to_string(),
            state: self.state.clone(),
            files: self.files,
            pages: self.pages,
            errors: self.errors,
            error: self.error.clone(),
            task_ids: self.task_ids.clone(),
        }
    }
}

struct Service {
    /// Shared state handle so short-lived background threads (site grabs) can
    /// be spawned from any `&self` command handler.
    inner: Arc<Mutex<Inner>>,
    wake: Arc<Condvar>,
}

#[derive(Debug)]
struct Work {
    id: i64,
    url: String,
    output: PathBuf,
    kind: WorkKind,
    proxy: Proxy,
    stop: Arc<AtomicBool>,
    /// Live FTP progress holder, mirrored inside the active entry.
    ftp: Option<Arc<FtpLive>>,
}

#[derive(Debug)]
enum WorkKind {
    Http {
        headers: Vec<(String, String)>,
        connections: usize,
        method: String,
        body: Option<Vec<u8>>,
        control: Arc<TransferControl>,
    },
    Ftp,
    Media {
        mode: MediaMode,
        headers: Vec<(String, String)>,
    },
}

/// Stored-mode decode: rows written before `media_mode_json` exist are the
/// legacy `SubmitMedia` video job, and a corrupt or unknown JSON falls back to
/// the legacy interpretation when the columns still name a video track.
fn stored_media_mode(
    media_mode_json: Option<String>,
    media_video: Option<i64>,
    media_audio: Option<i64>,
) -> Option<MediaMode> {
    match media_mode_json.as_deref() {
        Some(text) => serde_json::from_str(text).ok(),
        None => Some(MediaMode::Video {
            video: media_video? as usize,
            audio: media_audio.map(|index| index as usize),
            subtitles: Vec::new(),
        }),
    }
}

/// xdm-media's engine format for a mirrored IPC format; byte values pass
/// through unchanged because the serde shapes are identical.
fn engine_audio_format(format: MediaAudioFormat) -> xdm_media::AudioFormat {
    match format {
        MediaAudioFormat::Original => xdm_media::AudioFormat::Original,
        MediaAudioFormat::Mp3 { bitrate_kbps } => {
            xdm_media::AudioFormat::Mp3 { bitrate_kbps }
        }
        MediaAudioFormat::M4a { bitrate_kbps } => xdm_media::AudioFormat::M4a { bitrate_kbps },
        MediaAudioFormat::Opus { bitrate_kbps } => {
            xdm_media::AudioFormat::Opus { bitrate_kbps }
        }
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn get_setting(db: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    db.query_row("SELECT value FROM settings WHERE key=?1", [key], |row| {
        row.get(0)
    })
    .optional()
}

fn set_setting(db: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    db.execute("INSERT INTO settings(key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key,value])?;
    Ok(())
}

/// User category rules persisted in the daemon database, sorted by name so
/// classification is deterministic.
fn load_rules(db: &Connection) -> Result<Vec<xdm_categories::CategoryRule>> {
    let mut statement =
        db.prepare("SELECT rule_json FROM categories ORDER BY name COLLATE NOCASE")?;
    Ok(statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .filter_map(|json| serde_json::from_str(json).ok())
        .collect())
}

/// Host of an http(s)/(f)tp URL, used by category site rules.
fn url_host(url: &str) -> Option<String> {
    Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_owned))
        .map(|host| {
            host.trim_start_matches('[')
                .trim_end_matches(']')
                .to_ascii_lowercase()
        })
}

/// Origin-level login pattern for a task URL, so a remembered login covers
/// the whole host (with its explicit port when one was given).
fn origin_pattern(url: &str) -> Option<String> {
    let url = Url::parse(url).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    // `host_str` keeps IPv6 brackets, which the pattern parser accepts.
    let host = url.host_str()?;
    let authority = match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    };
    Some(format!("{}://{authority}/", url.scheme()))
}

/// Shares the directory validation used by import and grab destinations.
fn valid_absolute_directory(directory: &str) -> bool {
    std::path::Path::new(directory).is_absolute()
        && !std::path::Path::new(directory)
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
}

fn valid_grab_name(name: &str) -> bool {
    !name.trim().is_empty()
        && name == name.trim()
        && name.len() <= 64
        && !name.chars().any(char::is_control)
}

/// Last percent-decoded URL path segment, or a safe placeholder.
fn grab_filename(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|url| {
            url.path_segments()
                .and_then(|mut segments| segments.next_back().map(str::to_owned))
        })
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            percent_encoding::percent_decode_str(&segment)
                .decode_utf8_lossy()
                .into_owned()
        })
        .filter(|name| xdm_categories::valid_filename(name))
        .unwrap_or_else(|| "download.bin".into())
}

/// Categories as they appear in the Categories dialog: edited built-ins first,
/// then the user's own categories.
fn category_reply(db: &Connection) -> Result<Reply> {
    let rules = load_rules(db)?;
    let mut categories = Vec::new();
    for builtin in xdm_categories::CATEGORIES {
        if let Some(rule) = rules.iter().find(|rule| rule.name == *builtin) {
            categories.push(rule.clone());
        } else {
            categories.push(xdm_categories::CategoryRule {
                name: builtin.to_string(),
                extensions: xdm_categories::builtin_extensions(builtin),
                destination: String::new(),
                sites: Vec::new(),
            });
        }
    }
    categories.extend(
        rules
            .into_iter()
            .filter(|rule| !xdm_categories::CATEGORIES.contains(&rule.name.as_str())),
    );
    let mut reply = Reply::list(Vec::new());
    reply.categories = categories;
    Ok(reply)
}

/// Saved grabber projects as a reply payload.
fn grab_project_reply(inner: &Inner) -> Result<Reply> {
    let records = inner
        .db
        .prepare("SELECT name,project_json FROM grab_projects ORDER BY name")?
        .query_map([], |row| {
            Ok(GrabProjectRecord {
                name: row.get(0)?,
                project: serde_json::from_str(&row.get::<_, String>(1)?)
                    .unwrap_or(serde_json::Value::Null),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut reply = Reply::list(Vec::new());
    reply.grab_projects = records;
    Ok(reply)
}

/// An output slot is free when none of its file or checkpoint artefacts exist.
fn destination_free(output: &Path) -> bool {
    ["", ".part", ".part.meta", ".xdm-part", ".part.ftp"]
        .into_iter()
        .all(|suffix| std::fs::symlink_metadata(format!("{}{suffix}", output.display())).is_err())
}

fn interrupt_queue_workers(inner: &mut Inner, queue_id: Option<i64>) {
    let active = inner
        .active
        .iter()
        .filter(|(_, active)| queue_id.is_none_or(|id| active.queue_id == id))
        .map(|(id, active)| (*id, active.stop.clone()))
        .collect::<Vec<_>>();
    for (id, stop) in active {
        inner.queue_interrupted.insert(id);
        stop.store(true, Ordering::Relaxed);
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
fn cancel_pending(inner: &mut Inner, queue_id: i64, status: &str) {
    if let Some(pending) = inner.pending.remove(&queue_id) {
        let _ = inner.db.execute(
            "UPDATE queue_events SET action_status=?1 WHERE id=?2 AND action_status='pending'",
            params![status, pending.event_id],
        );
    }
}

fn task_summaries(inner: &Inner, task_id: Option<i64>) -> Result<Vec<TaskSummary>> {
    let mut stmt = inner.db.prepare("SELECT id,url,output,status,priority,error,kind,category,rate_limit,connections,created_at,updated_at,media_video,media_audio,queue_id,history_queue_id,retry_count,retry_at,position,auth_realm FROM tasks WHERE id BETWEEN COALESCE(?1,1) AND COALESCE(?1,9223372036854775807) ORDER BY id DESC LIMIT 1000")?;
    let tasks = stmt
        .query_map([task_id], |row| {
            let output: String = row.get(2)?;
            let status: String = row.get(3)?;
            let id: i64 = row.get(0)?;
            let live = inner
                .active
                .get(&id)
                .and_then(|active| match (&active.control, &active.ftp) {
                    (Some(control), _) => Some(control.progress()),
                    (None, Some(ftp)) => Some(ftp.progress()),
                    (None, None) => None,
                });
            let progress: Option<(u64, Option<u64>)> = if live.is_some() {
                None
            } else if status == "complete" {
                std::fs::metadata(&output)
                    .ok()
                    .map(|file| (file.len(), Some(file.len())))
            } else {
                xdm_http::checkpoint_progress(std::path::Path::new(&output))
                    .map(|(done, total)| (done, Some(total)))
                    .or_else(|| ftp_checkpoint_progress(std::path::Path::new(&output)))
            };
            Ok(TaskSummary {
                id,
                url: row.get(1)?,
                output,
                status,
                priority: row.get(4)?,
                error: row.get(5)?,
                downloaded: live
                    .map(|p| p.downloaded)
                    .or_else(|| progress.map(|(done, _)| done)),
                total: live
                    .and_then(|p| p.total)
                    .or_else(|| progress.and_then(|(_, total)| total)),
                kind: row.get(6)?,
                category: row.get(7)?,
                rate_limit: row.get::<_, Option<i64>>(8)?.map(|rate| rate as u64),
                bytes_per_second: live.map(|p| p.bytes_per_second),
                eta_seconds: live.and_then(|p| p.eta_seconds),
                connections: row.get::<_, i64>(9)? as usize,
                created_at: row.get(10)?,
                updated_at: row.get(11)?,
                media_video: row.get::<_, Option<i64>>(12)?.map(|index| index as usize),
                media_audio: row.get::<_, Option<i64>>(13)?.map(|index| index as usize),
                queue_id: row.get(14)?,
                history_queue_id: row.get(15)?,
                retry_count: row.get(16)?,
                retry_at: row.get(17)?,
                position: row.get(18)?,
                auth_realm: row.get(19)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(tasks)
}

fn request_was_removed(db: &Connection, request_id: &str) -> rusqlite::Result<bool> {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM removed_tasks WHERE request_id=?1)",
        [request_id],
        |row| row.get(0),
    )
}

fn next_task_id(db: &Connection) -> rusqlite::Result<i64> {
    db.query_row("SELECT MAX(COALESCE((SELECT MAX(id) FROM tasks),0),COALESCE((SELECT MAX(task_id) FROM removed_tasks),0))+1", [], |row| row.get(0))
}

fn valid_queue_name(name: &str) -> bool {
    !name.trim().is_empty()
        && name == name.trim()
        && name.len() <= 64
        && !name.chars().any(char::is_control)
}

fn queue_summaries(inner: &Inner) -> Result<Vec<QueueSummary>> {
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
fn stop_all_independent(db: &Connection) -> rusqlite::Result<()> {
    db.execute("UPDATE queues SET solo=0,sched_until=NULL", [])?;
    Ok(())
}

fn queue_events(db: &Connection, queue_id: i64) -> rusqlite::Result<Vec<QueueEvent>> {
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
fn finish_cycle(inner: &mut Inner, queue_id: i64, outcome: &str, now: i64) -> Result<()> {
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
fn run_due_completions(inner: &mut Inner) -> Result<()> {
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
fn queue_tick(inner: &mut Inner) -> Result<()> {
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
                "SELECT COUNT(*) FROM tasks WHERE queue_id=?1 AND status='failed'",
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

fn queue_reply(inner: &Inner, queue_id: i64) -> Result<Reply> {
    let mut reply = Reply::list(Vec::new());
    reply.queue_id = Some(queue_id);
    reply.queues = queue_summaries(inner)?;
    Ok(reply)
}

impl Service {
    fn submit_task(
        &self,
        request_id: String,
        url: String,
        output: String,
        headers: Vec<(String, String)>,
        method: String,
        body: Option<Vec<u8>>,
        priority: i64,
        connections: usize,
    ) -> Result<Reply> {
        let method = method.trim().to_ascii_uppercase();
        if request_id.is_empty()
            || request_id.len() > 128
            || output.is_empty()
            || !(url.starts_with("https://")
                || url.starts_with("http://")
                || url.starts_with("ftp://"))
            || !(1..=16).contains(&connections)
            || !(0..=2).contains(&priority)
            || !valid_method(&method)
            || (method == "GET" && body.is_some())
            || body.as_ref().is_some_and(|body| body.len() > xdm_http::MAX_BODY)
        {
            return Ok(Reply::error("invalid submission fields"));
        }
        let headers_json = serde_json::to_string(&headers)?;
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        if request_was_removed(&inner.db, &request_id)? {
            return Ok(Reply::error(
                "this submission was removed; use a new request ID",
            ));
        }
        let existing = inner
            .db
            .query_row(
                "SELECT id,url,output,method,body FROM tasks WHERE request_id=?1",
                [&request_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<Vec<u8>>>(4)?,
                    ))
                },
            )
            .optional()?;
        if let Some((id, old_url, old_output, old_method, old_body)) = existing {
            if old_url != url
                || old_output != output
                || old_method != method
                || old_body != body
            {
                return Ok(Reply::error(
                    "request ID already belongs to a different task",
                ));
            }
            self.wake.notify_one();
            Ok(Reply::accepted(id))
        } else {
            if std::fs::metadata(&output).is_ok() {
                return Ok(Reply::error("destination file already exists"));
            }
            let rules = load_rules(&inner.db)?;
            let name = std::path::Path::new(&output)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            let category =
                xdm_categories::classify_with(&rules, name, url_host(&url).as_deref(), None);
            let id = next_task_id(&inner.db)?;
            let kind = if url.starts_with("ftp://") { "ftp" } else { "http" };
            let inserted = inner.db.execute("INSERT OR IGNORE INTO tasks(id,request_id,url,output,headers_json,priority,connections,status,kind,category,method,body) VALUES (?9,?1,?2,?3,?4,?5,?6,'queued',?8,?7,?10,?11)", params![request_id,url,output,headers_json,priority,connections as i64,category,kind,id,method,body])?;
            if inserted == 0 {
                return Ok(Reply::error("destination is already assigned to a task"));
            }
            let id = inner.db.last_insert_rowid();
            drop(inner);
            self.wake.notify_one();
            Ok(Reply::accepted(id))
        }
    }

    fn open(paths: &Paths) -> Result<Arc<Self>> {
        Self::open_db(
            &paths.database,
            Arc::new(SystemClock),
            os_actions::select(&paths.dir),
        )
    }

    fn open_db(
        database: &std::path::Path,
        clock: Arc<dyn Clock>,
        adapter: Arc<dyn CompletionAdapter>,
    ) -> Result<Arc<Self>> {
        let db = Connection::open(database)?;
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS tasks (
             id INTEGER PRIMARY KEY,
             request_id TEXT NOT NULL UNIQUE,
             url TEXT NOT NULL,
             output TEXT NOT NULL UNIQUE,
             headers_json TEXT NOT NULL,
             priority INTEGER NOT NULL,
             connections INTEGER NOT NULL,
             status TEXT NOT NULL,
             error TEXT,
             kind TEXT NOT NULL DEFAULT 'http',
             media_video INTEGER,
             media_audio INTEGER,
             category TEXT NOT NULL DEFAULT 'Other',
             created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
             updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
          );
          CREATE INDEX IF NOT EXISTS tasks_queue_order ON tasks(status, priority DESC, id ASC);",
        )?;
        let columns = db
            .prepare("PRAGMA table_info(tasks)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (name, definition) in [
            ("kind", "TEXT NOT NULL DEFAULT 'http'"),
            ("media_video", "INTEGER"),
            ("media_audio", "INTEGER"),
            ("category", "TEXT NOT NULL DEFAULT 'Other'"),
            ("rate_limit", "INTEGER"),
            ("redownload_source", "INTEGER"),
            ("queue_id", "INTEGER NOT NULL DEFAULT 1"),
            ("position", "INTEGER NOT NULL DEFAULT 0"),
            ("retry_count", "INTEGER NOT NULL DEFAULT 0"),
            ("retry_at", "INTEGER"),
            ("history_queue_id", "INTEGER"),
            ("method", "TEXT NOT NULL DEFAULT 'GET'"),
            ("body", "BLOB"),
            ("media_mode_json", "TEXT"),
            ("auth_realm", "TEXT"),
        ] {
            if !columns.iter().any(|column| column == name) {
                db.execute_batch(&format!("ALTER TABLE tasks ADD COLUMN {name} {definition}"))?;
            }
        }
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS removed_tasks (task_id INTEGER PRIMARY KEY, request_id TEXT NOT NULL UNIQUE);
             CREATE TABLE IF NOT EXISTS imports (request_id TEXT PRIMARY KEY, payload_json TEXT NOT NULL, task_ids_json TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS grab_projects (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE COLLATE NOCASE, project_json TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS categories (name TEXT PRIMARY KEY COLLATE NOCASE, rule_json TEXT NOT NULL);",
        )?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS queues (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE COLLATE NOCASE, running INTEGER NOT NULL, max_concurrent INTEGER NOT NULL);
            INSERT OR IGNORE INTO queues(id,name,running,max_concurrent) VALUES (1,'Main',1,16);
            CREATE INDEX IF NOT EXISTS tasks_by_queue ON tasks(queue_id,status,priority DESC,id ASC);")?;
        let queue_columns = db
            .prepare("PRAGMA table_info(queues)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (name, definition) in [
            ("solo", "INTEGER NOT NULL DEFAULT 0"),
            ("retry_max", "INTEGER NOT NULL DEFAULT 0"),
            ("retry_delay", "INTEGER NOT NULL DEFAULT 30"),
            ("sched_kind", "TEXT NOT NULL DEFAULT 'none'"),
            ("sched_start", "INTEGER NOT NULL DEFAULT 0"),
            ("sched_stop", "INTEGER"),
            ("sched_days", "INTEGER NOT NULL DEFAULT 0"),
            ("sched_last", "INTEGER NOT NULL DEFAULT 0"),
            ("sched_until", "INTEGER"),
            ("completion_action", "TEXT NOT NULL DEFAULT 'none'"),
            ("run_state", "TEXT NOT NULL DEFAULT 'idle'"),
            ("run_exec", "INTEGER NOT NULL DEFAULT 0"),
            ("completion_confirmed", "INTEGER NOT NULL DEFAULT 0"),
            ("completion_countdown", "INTEGER NOT NULL DEFAULT 60"),
        ] {
            if !queue_columns.iter().any(|column| column == name) {
                db.execute_batch(&format!("ALTER TABLE queues ADD COLUMN {name} {definition}"))?;
            }
        }
        // Manual order: new rows take their own id as position (appended last);
        // completed tasks leave their queue but keep it as history.
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS queue_events (id INTEGER PRIMARY KEY AUTOINCREMENT, queue_id INTEGER NOT NULL, execution INTEGER NOT NULL, outcome TEXT NOT NULL, action TEXT NOT NULL, action_status TEXT NOT NULL, at INTEGER NOT NULL);
             CREATE TRIGGER IF NOT EXISTS tasks_position AFTER INSERT ON tasks WHEN NEW.position=0 BEGIN UPDATE tasks SET position=NEW.id WHERE id=NEW.id; END;
             CREATE INDEX IF NOT EXISTS tasks_by_queue_order ON tasks(queue_id,status,priority DESC,position ASC,id ASC);
             UPDATE tasks SET position=id WHERE position=0;
             UPDATE tasks SET history_queue_id=queue_id,queue_id=0 WHERE status='complete' AND queue_id<>0;",
        )?;
        // A countdown belongs to the daemon instance that armed it: after a
        // restart its destructive action is suppressed rather than executed.
        db.execute(
            "UPDATE queue_events SET action_status='suppressed' WHERE action_status='pending'",
            [],
        )?;
        let max_concurrent = db
            .query_row(
                "SELECT value FROM settings WHERE key='max_concurrent'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| (1..=16).contains(value))
            .unwrap_or(2);
        let queue_running = get_setting(&db, "queue_running")?.as_deref() != Some("0");
        let queue_start_at =
            get_setting(&db, "queue_start_at")?.and_then(|value| value.parse::<i64>().ok());
        let queue_stop_at =
            get_setting(&db, "queue_stop_at")?.and_then(|value| value.parse::<i64>().ok());
        let proxy = get_setting(&db, "proxy")?
            .and_then(|value| serde_json::from_str::<Proxy>(&value).ok())
            .unwrap_or_default();
        // Site logins live beside the task database; in-memory test databases
        // keep the store in memory only.
        let credentials = credentials::Store::load(
            (database.as_os_str() != ":memory:")
                .then(|| database.parent().unwrap_or(Path::new(".")).join("credentials.json")),
        )?;
        db.execute(
            "UPDATE tasks SET status='queued', error=NULL WHERE status IN ('running','pausing')",
            [],
        )?;
        Ok(Arc::new(Self {
            inner: Arc::new(Mutex::new(Inner {
                db,
                active: HashMap::new(),
                max_concurrent,
                queue_running,
                queue_start_at,
                queue_stop_at,
                queue_interrupted: HashSet::new(),
                clock,
                adapter,
                pending: HashMap::new(),
                proxy: serde_json::to_string(&proxy).unwrap_or_default(),
                credentials,
                grabs: HashMap::new(),
                pending_grabs: Vec::new(),
            })),
            wake: Arc::new(Condvar::new()),
        }))
    }

    fn command(&self, command: Command) -> Result<Reply> {
        let tick = |service: &Self| -> Result<()> {
            let mut inner = service
                .inner
                .lock()
                .map_err(|_| io::Error::other("task lock poisoned"))?;
            queue_tick(&mut inner)
        };
        tick(self)?;
        let mut reply = self.command_inner(command)?;
        tick(self)?;
        if reply.ok && !reply.queues.is_empty() {
            let inner = self
                .inner
                .lock()
                .map_err(|_| io::Error::other("task lock poisoned"))?;
            reply.queues = queue_summaries(&inner)?;
        }
        self.wake.notify_one();
        Ok(reply)
    }

    fn command_inner(&self, command: Command) -> Result<Reply> {
        match command {
            Command::QueueCreate { name, limit } => {
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
            Command::QueueSetRunning { queue_id, running } => {
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
            Command::QueueSetConcurrency { queue_id, limit } => {
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
            Command::QueueRename { queue_id, name } => {
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
            Command::QueueRemove { queue_id } => {
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
            Command::AssignQueue { queue_id, task_ids } => {
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
            Command::ImportList {
                request_id,
                directory,
                entries,
                enqueue,
            } => {
                if request_id.is_empty()
                    || request_id.len() > 64
                    || !std::path::Path::new(&directory).is_absolute()
                    || std::path::Path::new(&directory)
                        .components()
                        .any(|part| matches!(part, std::path::Component::ParentDir))
                {
                    return Ok(Reply::error(
                        "import needs a request ID and an absolute destination directory without ..",
                    ));
                }
                if let Err(error) = xdm_transfer_list::validate(&entries) {
                    return Ok(Reply::error(error));
                }
                if std::path::Path::new(&directory).exists()
                    && !std::path::Path::new(&directory).is_dir()
                {
                    return Ok(Reply::error("destination directory is a file"));
                }
                let payload_json = serde_json::to_string(&(&directory, &entries, enqueue))?;
                let mut inner = self
                    .inner
                    .lock()
                    .map_err(|_| io::Error::other("task lock poisoned"))?;
                let previous = inner
                    .db
                    .query_row(
                        "SELECT payload_json,task_ids_json FROM imports WHERE request_id=?1",
                        [&request_id],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                    )
                    .optional()?;
                if let Some((old_payload, ids)) = previous {
                    if old_payload != payload_json {
                        return Ok(Reply::error(
                            "import request ID belongs to a different batch",
                        ));
                    }
                    let ids: Vec<i64> = serde_json::from_str(&ids)?;
                    for id in &ids {
                        let exists: bool = inner.db.query_row(
                            "SELECT EXISTS(SELECT 1 FROM tasks WHERE id=?1)",
                            [id],
                            |row| row.get(0),
                        )?;
                        if !exists {
                            return Ok(Reply::error(
                                "a task from this import was removed; use a new import request ID",
                            ));
                        }
                    }
                    let mut reply = Reply::list(Vec::new());
                    reply.task_ids = ids;
                    return Ok(reply);
                }
                let outputs = entries
                    .iter()
                    .map(|entry| {
                        std::path::Path::new(&directory)
                            .join(&entry.filename)
                            .to_string_lossy()
                            .into_owned()
                    })
                    .collect::<Vec<_>>();
                // The transfer list's own classification is its best guess;
                // when it matches the built-in fallback, user rules may know
                // better via extensions or site patterns.
                let rules = load_rules(&inner.db)?;
                let categories = entries
                    .iter()
                    .map(|entry| {
                        if entry.category == xdm_categories::classify(&entry.filename, None) {
                            xdm_categories::classify_with(
                                &rules,
                                &entry.filename,
                                url_host(&entry.url).as_deref(),
                                None,
                            )
                        } else {
                            entry.category.clone()
                        }
                    })
                    .collect::<Vec<_>>();
                for output in &outputs {
                    if !destination_free(Path::new(output)) {
                        return Ok(Reply::error(format!(
                            "destination already has a file or checkpoint: {output}"
                        )));
                    }
                    let occupied: bool = inner.db.query_row(
                        "SELECT EXISTS(SELECT 1 FROM tasks WHERE output=?1 COLLATE NOCASE)",
                        [output],
                        |row| row.get(0),
                    )?;
                    if occupied {
                        return Ok(Reply::error(format!(
                            "destination is assigned to a task: {output}"
                        )));
                    }
                }
                let first_id = next_task_id(&inner.db)?;
                let transaction = inner.db.transaction()?;
                let mut ids = Vec::with_capacity(entries.len());
                for (index, (entry, output)) in entries.iter().zip(&outputs).enumerate() {
                    let id = first_id
                        .checked_add(index as i64)
                        .ok_or("task ID exhausted")?;
                    transaction.execute(
                        "INSERT INTO tasks(id,request_id,url,output,headers_json,priority,connections,status,kind,media_video,media_audio,category,rate_limit) VALUES (?1,?2,?3,?4,'[]',?5,?6,?7,?8,?9,?10,?11,?12)",
                        params![id,format!("import:{request_id}:{index}"),entry.url,output,entry.priority,entry.connections as i64,if enqueue {"queued"} else {"paused"},entry.kind,entry.video.map(|i| i as i64),entry.audio.map(|i| i as i64),&categories[index],entry.rate_limit.map(|rate| rate as i64)],
                    )?;
                    ids.push(id);
                }
                transaction.execute(
                    "INSERT INTO imports(request_id,payload_json,task_ids_json) VALUES (?1,?2,?3)",
                    params![request_id, payload_json, serde_json::to_string(&ids)?],
                )?;
                transaction.commit()?;
                drop(inner);
                if enqueue {
                    self.wake.notify_one();
                }
                let mut reply = Reply::list(Vec::new());
                reply.task_ids = ids;
                Ok(reply)
            }
            Command::ExportList => {
                let inner = self
                    .inner
                    .lock()
                    .map_err(|_| io::Error::other("task lock poisoned"))?;
                let mut statement = inner.db.prepare("SELECT url,output,kind,priority,connections,rate_limit,media_video,media_audio,media_mode_json,category FROM tasks ORDER BY id LIMIT 501")?;
                let mut entries = statement
                    .query_map([], |row| {
                        let output: String = row.get(1)?;
                        let filename = std::path::Path::new(&output)
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        let stored: String = row.get(9)?;
                        let kind: String = row.get(2)?;
                        let mode_json: Option<String> = row.get(8)?;
                        let video: Option<i64> = row.get(6)?;
                        let audio: Option<i64> = row.get(7)?;
                        // Portable lists carry only the media shape the reader
                        // side validates: a video track muxed into .mp4.
                        // Audio-only, sidecar, subtitle-carrying and MKV jobs
                        // are omitted: as HTTP entries they would re-import as
                        // a download of the manifest itself. Request headers
                        // are never exported (see lib docs).
                        let mode = stored_media_mode(mode_json, video, audio);
                        let portable = kind != "media"
                            || matches!(
                                mode,
                                Some(MediaMode::Video { subtitles, .. }) if subtitles.is_empty()
                            )
                                && std::path::Path::new(&output)
                                    .extension()
                                    .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"))
                                && video.is_some();
                        // Portable lists only carry built-in category names, so
                        // user categories fall back to the shared extension table.
                        let category =
                            if xdm_categories::CATEGORIES.contains(&stored.as_str()) {
                                stored
                            } else {
                                xdm_categories::classify(&filename, None).into()
                            };
                        if !portable {
                            return Ok(None);
                        }
                        Ok(Some(ImportEntry {
                            url: row.get(0)?,
                            filename,
                            kind,
                            priority: row.get(3)?,
                            connections: row.get::<_, i64>(4)? as usize,
                            rate_limit: row.get::<_, Option<i64>>(5)?.map(|rate| rate as u64),
                            video: video.map(|i| i as usize),
                            audio: audio.map(|i| i as usize),
                            category,
                        }))
                    })?
                    .filter_map(|row| row.transpose())
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                for entry in &mut entries {
                    entry.url =
                        xdm_transfer_list::export_url(&entry.url).map_err(io::Error::other)?;
                }
                xdm_transfer_list::export_names(&mut entries);
                if !entries.is_empty() {
                    if let Err(error) = xdm_transfer_list::validate(&entries) {
                        return Ok(Reply::error(error));
                    }
                }
                let mut reply = Reply::list(Vec::new());
                reply.transfer_list = Some(TransferList {
                    version: 1,
                    entries,
                });
                Ok(reply)
            }
            Command::Submit {
                request_id,
                url,
                output,
                headers,
                priority,
                connections,
            } => {
                self.submit_task(
                    request_id,
                    url,
                    output,
                    headers,
                    "GET".into(),
                    None,
                    priority,
                    connections,
                )
            }
            Command::SubmitRequest {
                request_id,
                url,
                output,
                headers,
                method,
                body,
                priority,
                connections,
            } => {
                self.submit_task(
                    request_id,
                    url,
                    output,
                    headers,
                    method,
                    body,
                    priority,
                    connections,
                )
            }
            Command::SetProxy { proxy } => {
                let parsed: Proxy =
                    serde_json::from_value(serde_json::to_value(&proxy)?).map_err(|e| e.to_string())?;
                if let Proxy::Manual { http, https, .. } = &parsed {
                    for url in http.iter().chain(https.iter()) {
                        if !PROXY_SCHEMES.iter().any(|scheme| url.starts_with(scheme)) {
                            return Ok(Reply::error(format!(
                                "proxy URL {url} must start with {}",
                                PROXY_SCHEMES.join(" or ")
                            )));
                        }
                    }
                }
                let stored = serde_json::to_string(&parsed)?;
                let mut inner = self
                    .inner
                    .lock()
                    .map_err(|_| io::Error::other("task lock poisoned"))?;
                set_setting(&inner.db, "proxy", &stored)?;
                inner.proxy = stored.clone();
                let mut reply = Reply::accepted(0);
                reply.proxy = serde_json::to_value(&parsed).ok();
                Ok(reply)
            }
            Command::GetProxy => {
                let inner = self
                    .inner
                    .lock()
                    .map_err(|_| io::Error::other("task lock poisoned"))?;
                let mut reply = Reply::accepted(0);
                reply.proxy = serde_json::from_str(&inner.proxy).ok();
                Ok(reply)
            }
            Command::SubmitMedia {
                request_id,
                url,
                output,
                video,
                audio,
                priority,
            } => {
                let manifest = url.split('?').next().unwrap_or("").to_ascii_lowercase();
                if request_id.is_empty()
                    || request_id.len() > 128
                    || !(url.starts_with("http://") || url.starts_with("https://"))
                    || !(manifest.ends_with(".m3u8") || manifest.ends_with(".mpd"))
                    || output.is_empty()
                    || !std::path::Path::new(&output)
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"))
                    || video > 10000
                    || audio.is_some_and(|index| index > 10000)
                    || !(0..=2).contains(&priority)
                {
                    return Ok(Reply::error("invalid media submission fields"));
                }
                let inner = self
                    .inner
                    .lock()
                    .map_err(|_| io::Error::other("task lock poisoned"))?;
                if request_was_removed(&inner.db, &request_id)? {
                    return Ok(Reply::error(
                        "this submission was removed; use a new request ID",
                    ));
                }
                let existing = inner.db.query_row(
                    "SELECT id,url,output,kind,media_video,media_audio FROM tasks WHERE request_id=?1",
                    [&request_id],
                    |row| Ok((row.get::<_,i64>(0)?, row.get::<_,String>(1)?, row.get::<_,String>(2)?, row.get::<_,String>(3)?, row.get::<_,Option<i64>>(4)?, row.get::<_,Option<i64>>(5)?)),
                ).optional()?;
                if let Some((id, old_url, old_output, kind, old_video, old_audio)) = existing {
                    if old_url != url
                        || old_output != output
                        || kind != "media"
                        || old_video != Some(video as i64)
                        || old_audio != audio.map(|value| value as i64)
                    {
                        return Ok(Reply::error(
                            "request ID already belongs to a different task",
                        ));
                    }
                    return Ok(Reply::accepted(id));
                }
                if std::fs::metadata(&output).is_ok() {
                    return Ok(Reply::error("destination file already exists"));
                }
                let id = next_task_id(&inner.db)?;
                let inserted = inner.db.execute(
                    "INSERT OR IGNORE INTO tasks(id,request_id,url,output,headers_json,priority,connections,status,kind,media_video,media_audio,category) VALUES (?7,?1,?2,?3,'[]',?4,1,'queued','media',?5,?6,'Video')",
                    params![request_id,url,output,priority,video as i64,audio.map(|value| value as i64),id],
                )?;
                if inserted == 0 {
                    return Ok(Reply::error("destination is already assigned to a task"));
                }
                let id = inner.db.last_insert_rowid();
                drop(inner);
                self.wake.notify_one();
                Ok(Reply::accepted(id))
            }
            Command::SubmitMediaJob {
                request_id,
                url,
                output,
                headers,
                mode,
                priority,
            } => {
                if request_id.is_empty()
                    || request_id.len() > 128
                    || !(url.starts_with("http://") || url.starts_with("https://"))
                    || output.is_empty()
                    || !std::path::Path::new(&output).is_absolute()
                    || !(0..=2).contains(&priority)
                    || !valid_media_headers(&headers)
                    || !valid_media_request(&url, &output, &mode)
                {
                    return Ok(Reply::error("invalid media submission fields"));
                }
                let inner = self
                    .inner
                    .lock()
                    .map_err(|_| io::Error::other("task lock poisoned"))?;
                if request_was_removed(&inner.db, &request_id)? {
                    return Ok(Reply::error(
                        "this submission was removed; use a new request ID",
                    ));
                }
                let existing = inner.db.query_row(
                    "SELECT id,url,output,kind,media_video,media_audio,media_mode_json FROM tasks WHERE request_id=?1",
                    [&request_id],
                    |row| Ok((row.get::<_,i64>(0)?, row.get::<_,String>(1)?, row.get::<_,String>(2)?, row.get::<_,String>(3)?, row.get::<_,Option<i64>>(4)?, row.get::<_,Option<i64>>(5)?, row.get::<_,Option<String>>(6)?)),
                ).optional()?;
                if let Some((id, old_url, old_output, kind, old_video, old_audio, old_mode)) = existing
                {
                    let stored = stored_media_mode(old_mode, old_video, old_audio);
                    if old_url != url
                        || old_output != output
                        || kind != "media"
                        || stored.as_ref() != Some(&mode)
                    {
                        return Ok(Reply::error(
                            "request ID already belongs to a different task",
                        ));
                    }
                    return Ok(Reply::accepted(id));
                }
                if std::fs::metadata(&output).is_ok() {
                    return Ok(Reply::error("destination file already exists"));
                }
                let id = next_task_id(&inner.db)?;
                let rules = load_rules(&inner.db)?;
                let category = match &mode {
                    MediaMode::Video { .. } => "Video".to_string(),
                    MediaMode::Audio { .. } => xdm_categories::classify_with(
                        &rules,
                        std::path::Path::new(&output)
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .as_ref(),
                        url_host(&url).as_deref(),
                        None,
                    ),
                    MediaMode::Subtitles { .. } => "Other".to_string(),
                };
                let (media_video, media_audio) = match &mode {
                    MediaMode::Video { video, audio, .. } => {
                        (Some(*video as i64), audio.map(|index| index as i64))
                    }
                    _ => (None, None),
                };
                let inserted = inner.db.execute(
                    "INSERT OR IGNORE INTO tasks(id,request_id,url,output,headers_json,priority,connections,status,kind,media_video,media_audio,media_mode_json,category) VALUES (?10,?1,?2,?3,?4,?5,1,'queued','media',?6,?7,?8,?9)",
                    params![
                        request_id,
                        url,
                        output,
                        serde_json::to_string(&headers)?,
                        priority,
                        media_video,
                        media_audio,
                        serde_json::to_string(&mode)?,
                        category,
                        id
                    ],
                )?;
                if inserted == 0 {
                    return Ok(Reply::error("destination is already assigned to a task"));
                }
                let id = inner.db.last_insert_rowid();
                drop(inner);
                self.wake.notify_one();
                Ok(Reply::accepted(id))
            }
            Command::GetTask { task_id } => {
                let inner = self
                    .inner
                    .lock()
                    .map_err(|_| io::Error::other("task lock poisoned"))?;
                let tasks = task_summaries(&inner, Some(task_id))?;
                if tasks.is_empty() {
                    Ok(Reply::error("task not found"))
                } else {
                    Ok(Reply::list(tasks))
                }
            }
            Command::Remove { task_id } => {
                let mut inner = self
                    .inner
                    .lock()
                    .map_err(|_| io::Error::other("task lock poisoned"))?;
                if inner.active.contains_key(&task_id) {
                    return Ok(Reply::error(
                        "pause the task and wait for it to stop before removing it",
                    ));
                }
                let transaction = inner.db.transaction()?;
                let removed = transaction.execute(
                    "INSERT INTO removed_tasks(task_id,request_id) SELECT id,request_id FROM tasks WHERE id=?1",
                    [task_id],
                )?;
                if removed == 0 {
                    return Ok(Reply::error("task not found"));
                }
                transaction.execute("DELETE FROM tasks WHERE id=?1", [task_id])?;
                transaction.commit()?;
                inner.queue_interrupted.remove(&task_id);
                Ok(Reply::accepted(task_id))
            }
            Command::Redownload {
                task_id,
                request_id,
                output,
            } => {
                if request_id.is_empty()
                    || request_id.len() > 128
                    || !std::path::Path::new(&output).is_absolute()
                    || std::path::Path::new(&output)
                        .components()
                        .any(|part| matches!(part, std::path::Component::ParentDir))
                {
                    return Ok(Reply::error(
                        "redownload requires a request ID and an absolute destination path",
                    ));
                }
                let inner = self
                    .inner
                    .lock()
                    .map_err(|_| io::Error::other("task lock poisoned"))?;
                if request_was_removed(&inner.db, &request_id)? {
                    return Ok(Reply::error(
                        "this submission was removed; use a new request ID",
                    ));
                }
                let existing = inner
                    .db
                    .query_row(
                        "SELECT id,output,redownload_source FROM tasks WHERE request_id=?1",
                        [&request_id],
                        |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, Option<i64>>(2)?,
                            ))
                        },
                    )
                    .optional()?;
                if let Some((id, old_output, source)) = existing {
                    return Ok(if old_output == output && source == Some(task_id) {
                        Reply::accepted(id)
                    } else {
                        Reply::error("request ID already belongs to a different task")
                    });
                }
                if inner.active.contains_key(&task_id) {
                    return Ok(Reply::error("pause the task before redownloading it"));
                }
                let Some((kind, mode_json)) = inner
                    .db
                    .query_row(
                        "SELECT kind,media_mode_json FROM tasks WHERE id=?1",
                        [task_id],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
                    )
                    .optional()?
                else {
                    return Ok(Reply::error("task not found"));
                };
                if kind == "media"
                    && !serde_json::from_str::<MediaMode>(mode_json.as_deref().unwrap_or(""))
                        .ok()
                        .or_else(|| {
                            // Legacy media rows keep the original .mp4 rule.
                            Some(MediaMode::Video { video: 0, audio: None, subtitles: Vec::new() })
                        })
                        .is_none_or(|mode| {
                            std::path::Path::new(&output)
                                .extension()
                                .and_then(|ext| ext.to_str())
                                .is_some_and(|ext| mode.valid(ext))
                        })
                {
                    return Ok(Reply::error(
                        "media redownload destination must fit the task's media mode",
                    ));
                }
                if !destination_free(std::path::Path::new(&output)) {
                    return Ok(Reply::error(
                        "choose a fresh destination without a file or checkpoint",
                    ));
                }
                let id = next_task_id(&inner.db)?;
                let inserted = inner.db.execute(
                    "INSERT OR IGNORE INTO tasks(id,request_id,url,output,headers_json,priority,connections,status,kind,media_video,media_audio,media_mode_json,category,rate_limit,redownload_source,queue_id,method,body) SELECT ?1,?2,url,?3,headers_json,priority,connections,'queued',kind,media_video,media_audio,media_mode_json,category,rate_limit,id,CASE WHEN queue_id=0 THEN COALESCE(history_queue_id,1) ELSE queue_id END,method,body FROM tasks WHERE id=?4",
                    params![id,request_id,output,task_id],
                )?;
                if inserted == 0 {
                    return Ok(Reply::error("destination is already assigned to a task"));
                }
                drop(inner);
                self.wake.notify_one();
                Ok(Reply::accepted(id))
            }
            Command::List => {
                let inner = self
                    .inner
                    .lock()
                    .map_err(|_| io::Error::other("task lock poisoned"))?;
                let tasks = task_summaries(&inner, None)?;
                let mut reply = Reply::list(tasks);
                reply.queue_running = Some(inner.queue_running);
                reply.queue_start_at = inner.queue_start_at;
                reply.queue_stop_at = inner.queue_stop_at;
                reply.queues = queue_summaries(&inner)?;
                Ok(reply)
            }
            Command::Pause { task_id } => {
                let mut inner = self
                    .inner
                    .lock()
                    .map_err(|_| io::Error::other("task lock poisoned"))?;
                inner.queue_interrupted.remove(&task_id);
                if let Some(active) = inner.active.get(&task_id) {
                    active.stop.store(true, Ordering::Relaxed);
                }
                let updated = inner.db.execute("UPDATE tasks SET status=CASE WHEN status='running' THEN 'pausing' ELSE 'paused' END, updated_at=CURRENT_TIMESTAMP WHERE id=?1 AND status IN ('queued','running')", [task_id])?;
                if updated == 0 {
                    Ok(Reply::error("task is not active or queued"))
                } else {
                    Ok(Reply::accepted(task_id))
                }
            }
            Command::Resume { task_id } => {
                let inner = self
                    .inner
                    .lock()
                    .map_err(|_| io::Error::other("task lock poisoned"))?;
                let updated = inner.db.execute("UPDATE tasks SET status='queued',error=NULL,retry_count=0,retry_at=NULL,updated_at=CURRENT_TIMESTAMP WHERE id=?1 AND status IN ('paused','failed')", [task_id])?;
                drop(inner);
                if updated == 0 {
                    Ok(Reply::error("task is not paused or failed"))
                } else {
                    self.wake.notify_one();
                    Ok(Reply::accepted(task_id))
                }
            }
            Command::SetPriority { task_id, priority } => {
                if !(0..=2).contains(&priority) {
                    return Ok(Reply::error(
                        "priority must be 0 (low), 1 (normal), or 2 (immediate)",
                    ));
                }
                let inner = self
                    .inner
                    .lock()
                    .map_err(|_| io::Error::other("task lock poisoned"))?;
                let updated = inner.db.execute(
                    "UPDATE tasks SET priority=?1,updated_at=CURRENT_TIMESTAMP WHERE id=?2 AND status IN ('queued','paused','failed')",
                    params![priority, task_id],
                )?;
                drop(inner);
                if updated == 0 {
                    Ok(Reply::error(
                        "task must be queued, paused, or failed to change priority",
                    ))
                } else {
                    self.wake.notify_one();
                    Ok(Reply::accepted(task_id))
                }
            }
            Command::SetTaskRate {
                task_id,
                bytes_per_second,
            } => {
                if bytes_per_second.is_some_and(|rate| rate == 0 || rate > 1024 * 1024 * 1024) {
                    return Ok(Reply::error(
                        "rate must be 1 byte/s to 1 GiB/s, or unlimited",
                    ));
                }
                let inner = self
                    .inner
                    .lock()
                    .map_err(|_| io::Error::other("task lock poisoned"))?;
                let updated = inner.db.execute(
                    "UPDATE tasks SET rate_limit=?1,updated_at=CURRENT_TIMESTAMP WHERE id=?2 AND kind='http' AND status IN ('queued','running','paused','failed')",
                    params![bytes_per_second.map(|rate| rate as i64), task_id],
                )?;
                if updated == 0 {
                    return Ok(Reply::error(
                        "rate control requires an unfinished HTTP task",
                    ));
                }
                if let Some(control) = inner
                    .active
                    .get(&task_id)
                    .and_then(|active| active.control.as_ref())
                {
                    control.set_rate(bytes_per_second)?;
                }
                Ok(Reply::accepted(task_id))
            }
            Command::SetConcurrency { limit } => {
                if !(1..=16).contains(&limit) {
                    return Ok(Reply::error("concurrency must be between 1 and 16"));
                }
                let mut inner = self
                    .inner
                    .lock()
                    .map_err(|_| io::Error::other("task lock poisoned"))?;
                inner.db.execute("INSERT INTO settings(key,value) VALUES ('max_concurrent',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [limit.to_string()])?;
                inner.max_concurrent = limit;
                drop(inner);
                self.wake.notify_one();
                Ok(Reply::list(Vec::new()))
            }
            Command::QueueStart => {
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
            Command::QueueStop => {
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
            Command::QueueScheduleStart { at_unix } => {
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
                set_setting(&inner.db, "queue_running", "0")?;
                set_setting(&inner.db, "queue_start_at", &at_unix.to_string())?;
                inner.queue_running = false;
                inner.queue_start_at = Some(at_unix);
                stop_all_independent(&inner.db)?;
                interrupt_queue_workers(&mut inner, None);
                drop(inner);
                self.wake.notify_one();
                Ok(Reply::list(Vec::new()))
            }
            Command::QueueScheduleStop { at_unix } => {
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
            Command::QueueStartOne { queue_id } => {
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
            Command::QueueStopOne { queue_id } => {
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
            Command::TaskMove { task_id, target } => {
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
            Command::QueueSetRetry {
                queue_id,
                max_retries,
                delay_seconds,
            } => {
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
            Command::QueueSetSchedule {
                queue_id,
                kind,
                start,
                stop,
                days,
            } => {
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
                queue_reply(&inner, queue_id)
            }
            Command::QueueSetCompletion { queue_id, action } => {
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
            Command::QueueSetCompletionAction {
                queue_id,
                action,
                confirm,
                countdown_seconds,
            } => {
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
            Command::CompletionStatus => {
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
            Command::CompletionCancel { queue_id } => {
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
            Command::QueueEvents { queue_id } => {
                let inner = self
                    .inner
                    .lock()
                    .map_err(|_| io::Error::other("task lock poisoned"))?;
                let mut reply = queue_reply(&inner, queue_id)?;
                reply.queue_events = queue_events(&inner.db, queue_id)?;
                Ok(reply)
            }
            Command::GrabSite {
                request_id,
                project,
                directory,
                queue_id,
                enqueue,
            } => self.grab_start(request_id, project, directory, queue_id, enqueue),
            Command::GrabStatus { request_id } => self.grab_status(&request_id),
            Command::GrabSave { name, project } => self.grab_save(&name, project),
            Command::GrabProjects => self.grab_projects(),
            Command::GrabRemove { name } => self.grab_remove(&name),
            Command::CategoryList => {
                let inner = self
                    .inner
                    .lock()
                    .map_err(|_| io::Error::other("task lock poisoned"))?;
                category_reply(&inner.db)
            }
            Command::CategorySave {
                name,
                extensions,
                destination,
                sites,
            } => self.category_save(&name, extensions, destination, sites),
            Command::CategoryRemove { name } => self.category_remove(&name),
            Command::LoginList => {
                let inner = self
                    .inner
                    .lock()
                    .map_err(|_| io::Error::other("task lock poisoned"))?;
                let mut reply = Reply::list(Vec::new());
                reply.logins = inner.credentials.summaries();
                Ok(reply)
            }
            Command::LoginSave {
                pattern,
                username,
                password,
                enabled,
            } => {
                let mut inner = self
                    .inner
                    .lock()
                    .map_err(|_| io::Error::other("task lock poisoned"))?;
                if let Err(error) = inner.credentials.save(&pattern, &username, &password, enabled) {
                    return Ok(Reply::error(error.to_string()));
                }
                let mut reply = Reply::list(Vec::new());
                reply.logins = inner.credentials.summaries();
                Ok(reply)
            }
            Command::LoginRemove { id } => {
                let mut inner = self
                    .inner
                    .lock()
                    .map_err(|_| io::Error::other("task lock poisoned"))?;
                if !inner.credentials.remove(id)? {
                    return Ok(Reply::error("login not found"));
                }
                let mut reply = Reply::accepted(id);
                reply.logins = inner.credentials.summaries();
                Ok(reply)
            }
            Command::TaskProvideLogin {
                task_id,
                username,
                password,
                remember,
            } => {
                if !credentials::valid_login_fields(&username, &password) {
                    return Ok(Reply::error(
                        "username needs 1–128 printable bytes and the password 0–1024",
                    ));
                }
                let mut inner = self
                    .inner
                    .lock()
                    .map_err(|_| io::Error::other("task lock poisoned"))?;
                let Some((url, status)) = inner
                    .db
                    .query_row(
                        "SELECT url,status FROM tasks WHERE id=?1",
                        [task_id],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                    )
                    .optional()?
                else {
                    return Ok(Reply::error("task not found"));
                };
                if status != "auth_required" {
                    return Ok(Reply::error("task is not waiting for a login"));
                }
                if remember {
                    let Some(pattern) = origin_pattern(&url) else {
                        return Ok(Reply::error("logins apply to http and https tasks only"));
                    };
                    if let Err(error) = inner.credentials.save(&pattern, &username, &password, true)
                    {
                        return Ok(Reply::error(error.to_string()));
                    }
                }
                let updated = inner.db.execute(
                    "UPDATE tasks SET status='queued',error=NULL,auth_realm=NULL,retry_count=0,retry_at=NULL,updated_at=CURRENT_TIMESTAMP WHERE id=?1 AND status='auth_required'",
                    [task_id],
                )?;
                if updated == 0 {
                    return Ok(Reply::error("task is not waiting for a login"));
                }
                let mut reply = Reply::accepted(task_id);
                reply.logins = inner.credentials.summaries();
                drop(inner);
                self.wake.notify_one();
                Ok(reply)
            }
        }
    }

    /// Starts a site grabber crawl on a background thread; entrenched replays
    /// of the same request ID return the recorded crawl state.
    fn grab_start(
        &self,
        request_id: String,
        project: serde_json::Value,
        directory: String,
        queue_id: Option<i64>,
        enqueue: bool,
    ) -> Result<Reply> {
        if request_id.is_empty() || request_id.len() > 64 || !valid_absolute_directory(&directory) {
            return Ok(Reply::error(
                "grab needs a request ID and an absolute destination directory without ..",
            ));
        }
        let project: xdm_grabber::Project = serde_json::from_value(project)
            .map_err(|e| format!("invalid grabber project: {e}"))?;
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        if let Some(handle) = inner.grabs.get(&request_id) {
            let report = handle
                .state
                .lock()
                .map_err(|_| io::Error::other("grab state lock poisoned"))?
                .report(&request_id);
            let mut reply = Reply::list(Vec::new());
            reply.grab = Some(report);
            return Ok(reply);
        }
        if let Some(queue_id) = queue_id {
            let exists: bool = inner.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM queues WHERE id=?1)",
                [queue_id],
                |row| row.get(0),
            )?;
            if !exists {
                return Ok(Reply::error("queue not found"));
            }
        }
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(GrabState {
            state: "running".into(),
            ..GrabState::default()
        }));
        inner.grabs.insert(
            request_id.clone(),
            GrabHandle {
                state: state.clone(),
            },
        );
        drop(inner);
        let pending = request_id.clone();
        self.queue_grab(&pending, project, directory, queue_id, enqueue, stop, state);
        Ok(Reply::grab_running(&request_id))
    }

    /// Registers a crawl for the worker loop to spawn, waking it up.
    fn queue_grab(
        &self,
        request_id: &str,
        project: xdm_grabber::Project,
        directory: String,
        queue_id: Option<i64>,
        enqueue: bool,
        stop: Arc<AtomicBool>,
        state: Arc<Mutex<GrabState>>,
    ) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        inner.pending_grabs.push(GrabJob {
            request_id: request_id.to_string(),
            project,
            directory,
            queue_id,
            enqueue,
            stop,
            state,
        });
        drop(inner);
        self.wake.notify_one();
    }

    /// Crawl body for a background thread: runs the crawl, records counters
    /// and atomically submits the found files as HTTP tasks.
    fn run_grab(
        &self,
        request_id: String,
        project: xdm_grabber::Project,
        directory: String,
        queue_id: Option<i64>,
        enqueue: bool,
        stop: Arc<AtomicBool>,
        state: Arc<Mutex<GrabState>>,
    ) {
        let outcome = xdm_grabber::crawl(&project, stop);
        // Counters first, so a long task submission is visible as done.
        match &outcome {
            Ok(result) => {
                if let Ok(mut state) = state.lock() {
                    state.files = result.files.len();
                    state.pages = result.pages.len();
                    state.errors = result.errors.len();
                }
                match self.add_grab_tasks(&request_id, result, &directory, queue_id, enqueue) {
                    Ok(task_ids) => {
                        if let Ok(mut state) = state.lock() {
                            state.task_ids = task_ids;
                            state.state = "done".into();
                        }
                    }
                    Err(error) => {
                        if let Ok(mut state) = state.lock() {
                            state.state = "failed".into();
                            state.error = Some(error.to_string());
                        }
                    }
                }
            }
            Err(error) => {
                if let Ok(mut state) = state.lock() {
                    state.state = "failed".into();
                    state.error = Some(error.to_string());
                }
            }
        }
        self.wake.notify_one();
    }

    /// Turns a crawl's file list into task rows inside a single transaction so
    /// a grab either adds every file or leaves nothing behind.
    fn add_grab_tasks(
        &self,
        request_id: &str,
        result: &xdm_grabber::CrawlResult,
        directory: &str,
        queue_id: Option<i64>,
        enqueue: bool,
    ) -> Result<Vec<i64>> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        if !inner.grabs.contains_key(request_id) {
            return Ok(Vec::new());
        }
        let rules = load_rules(&inner.db)?;
        let mut entries = Vec::new();
        let mut headers = Vec::new();
        for file in result.files.iter().take(xdm_transfer_list::MAX_ENTRIES) {
            let filename = grab_filename(&file.url);
            headers.push(if file.referer.is_empty() {
                "[]".to_string()
            } else {
                serde_json::to_string(&vec![("Referer".to_string(), file.referer.clone())])?
            });
            let category = xdm_categories::classify_with(
                &rules,
                &filename,
                url_host(&file.url).as_deref(),
                None,
            );
            entries.push(ImportEntry {
                url: file.url.clone(),
                filename,
                kind: "http".into(),
                priority: 1,
                connections: 8,
                rate_limit: None,
                video: None,
                audio: None,
                category,
            });
        }
        // Reuses the transfer-list name helpers so duplicates and unsafe
        // filenames get the same treatment as imports.
        xdm_transfer_list::export_names(&mut entries);
        if entries.is_empty() {
            return Ok(Vec::new());
        }
        let outputs = entries
            .iter()
            .map(|entry| {
                Path::new(directory)
                    .join(&entry.filename)
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        for output in &outputs {
            if !destination_free(Path::new(output)) {
                return Err(format!("grab destination already has a file or checkpoint: {output}").into());
            }
            let occupied: bool = inner.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM tasks WHERE output=?1 COLLATE NOCASE)",
                [output],
                |row| row.get(0),
            )?;
            if occupied {
                return Err(format!("grab destination is assigned to a task: {output}").into());
            }
        }
        let first_id = next_task_id(&inner.db)?;
        let transaction = inner.db.transaction()?;
        let mut task_ids = Vec::with_capacity(entries.len());
        for (index, (entry, output)) in entries.iter().zip(&outputs).enumerate() {
            let id = first_id
                .checked_add(index as i64)
                .ok_or("task ID exhausted")?;
            transaction.execute(
                "INSERT INTO tasks(id,request_id,url,output,headers_json,priority,connections,status,kind,category,queue_id,method) VALUES (?1,?2,?3,?4,?5,1,8,?6,'http',?7,?8,'GET')",
                params![
                    id,
                    format!("grab:{request_id}:{index}"),
                    entry.url,
                    output,
                    &headers[index],
                    if enqueue { "queued" } else { "paused" },
                    entry.category,
                    queue_id.unwrap_or(1) as i64,
                ],
            )?;
            task_ids.push(id);
        }
        transaction.commit()?;
        drop(inner);
        if enqueue {
            self.wake.notify_one();
        }
        Ok(task_ids)
    }

    fn grab_status(&self, request_id: &str) -> Result<Reply> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        let Some(handle) = inner.grabs.get(request_id) else {
            return Ok(Reply::error("grab request ID not found"));
        };
        let mut reply = Reply::list(Vec::new());
        reply.grab = Some(
            handle
                .state
                .lock()
                .map_err(|_| io::Error::other("grab state lock poisoned"))?
                .report(request_id),
        );
        Ok(reply)
    }

    fn grab_save(&self, name: &str, project: serde_json::Value) -> Result<Reply> {
        if !valid_grab_name(name) {
            return Ok(Reply::error("project name must be 1–64 trimmed characters"));
        }
        let parsed: xdm_grabber::Project = serde_json::from_value(project)
            .map_err(|e| format!("invalid grabber project: {e}"))?;
        let canonical = serde_json::to_value(&parsed)?;
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        inner.db.execute(
            "INSERT INTO grab_projects(name,project_json) VALUES (?1,?2) ON CONFLICT(name) DO UPDATE SET project_json=excluded.project_json",
            params![name, serde_json::to_string(&canonical)?],
        )?;
        grab_project_reply(&inner)
    }

    fn grab_projects(&self) -> Result<Reply> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        grab_project_reply(&inner)
    }

    fn grab_remove(&self, name: &str) -> Result<Reply> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        if inner
            .db
            .execute("DELETE FROM grab_projects WHERE name=?1", [name])?
            == 0
        {
            return Ok(Reply::error("project not found"));
        }
        grab_project_reply(&inner)
    }

    fn category_save(
        &self,
        name: &str,
        extensions: Vec<String>,
        destination: String,
        sites: Vec<String>,
    ) -> Result<Reply> {
        let rule = xdm_categories::CategoryRule {
            name: xdm_categories::CATEGORIES
                .iter()
                .find(|builtin| builtin.eq_ignore_ascii_case(name))
                .map_or_else(|| name.to_string(), |builtin| builtin.to_string()),
            extensions,
            destination,
            sites,
        };
        if let Err(message) = xdm_categories::valid_rule(&rule) {
            return Ok(Reply::error(message));
        }
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        inner.db.execute(
            "INSERT INTO categories(name,rule_json) VALUES (?1,?2) ON CONFLICT(name) DO UPDATE SET rule_json=excluded.rule_json",
            params![name, serde_json::to_string(&rule)?],
        )?;
        category_reply(&inner.db)
    }

    fn category_remove(&self, name: &str) -> Result<Reply> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        if xdm_categories::CATEGORIES
            .iter()
            .any(|builtin| builtin.eq_ignore_ascii_case(name))
        {
            return Ok(Reply::error("built-in categories can be edited but not removed"));
        }
        if inner
            .db
            .execute("DELETE FROM categories WHERE name=?1", [name])?
            == 0
        {
            return Ok(Reply::error("category not found"));
        }
        category_reply(&inner.db)
    }

    fn next_work(&self) -> Result<Work> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        loop {
            queue_tick(&mut inner)?;
            let now = unix_now();
            if inner.queue_stop_at.is_some_and(|at| now >= at) {
                set_setting(&inner.db, "queue_running", "0")?;
                inner
                    .db
                    .execute("DELETE FROM settings WHERE key='queue_stop_at'", [])?;
                inner.queue_running = false;
                inner.queue_stop_at = None;
                stop_all_independent(&inner.db)?;
                interrupt_queue_workers(&mut inner, None);
            }
            if inner.queue_start_at.is_some_and(|at| now >= at) {
                set_setting(&inner.db, "queue_running", "1")?;
                inner
                    .db
                    .execute("DELETE FROM settings WHERE key='queue_start_at'", [])?;
                inner.queue_running = true;
                inner.queue_start_at = None;
            }
            if inner.active.len() < inner.max_concurrent {
                let mut best: Option<(i64, i64)> = None;
                let queues = inner
                    .db
                    .prepare("SELECT id,max_concurrent FROM queues WHERE running=1 AND (solo=1 OR ?1)")?
                    .query_map([inner.queue_running], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                for (queue_id, limit) in queues {
                    if inner
                        .active
                        .values()
                        .filter(|active| active.queue_id == queue_id)
                        .count()
                        >= limit as usize
                    {
                        continue;
                    }
                    let candidate = inner.db.query_row("SELECT id,priority FROM tasks WHERE queue_id=?1 AND status='queued' AND (retry_at IS NULL OR retry_at<=?2) ORDER BY priority DESC,position ASC,id ASC LIMIT 1", params![queue_id, inner.clock.now()], |row| Ok((row.get::<_,i64>(0)?,row.get::<_,i64>(1)?))).optional()?;
                    if let Some((id, priority)) = candidate {
                        if best.is_none_or(|(old_id, old_priority)| {
                            priority > old_priority || (priority == old_priority && id < old_id)
                        }) {
                            best = Some((id, priority));
                        }
                    }
                }
                let selected = if let Some((id, _)) = best {
                    inner.db.query_row("SELECT id,url,output,headers_json,connections,kind,media_video,media_audio,media_mode_json,rate_limit,queue_id,method,body FROM tasks WHERE id=?1", [id], |row| Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,i64>(4)?,row.get::<_,String>(5)?,row.get::<_,Option<i64>>(6)?,row.get::<_,Option<i64>>(7)?,row.get::<_,Option<String>>(8)?,row.get::<_,Option<i64>>(9)?,row.get::<_,i64>(10)?,row.get::<_,String>(11)?,row.get::<_,Option<Vec<u8>>>(12)?))).optional()?
                } else {
                    None
                };
                if let Some((
                    id,
                    url,
                    output,
                    headers_json,
                    connections,
                    kind,
                    media_video,
                    media_audio,
                    media_mode_json,
                    rate_limit,
                    queue_id,
                    method,
                    body,
                )) = selected
                {
                    let (control, ftp) = if kind == "http" {
                        let control = Arc::new(TransferControl::new(
                            rate_limit.map(|rate| rate as u64),
                        )?);
                        (Some(control), None)
                    } else if kind == "ftp" {
                        let ftp = Arc::new(FtpLive::default());
                        (None, Some(ftp))
                    } else {
                        (None, None)
                    };
                    let mut headers: Vec<(String, String)> =
                        serde_json::from_str(&headers_json)?;
                    // A matching site login supplies Basic credentials for its
                    // own origin only, unless the submitter set a header already.
                    if kind == "http"
                        && !headers
                            .iter()
                            .any(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                        && let Ok(parsed) = Url::parse(&url)
                        && let Some(login) = inner.credentials.find(&parsed)
                    {
                        headers.push((
                            "Authorization".into(),
                            format!("Basic {}", credentials::basic_token(&login.username, &login.password)),
                        ));
                    }
                    let kind = if kind == "media" {
                        WorkKind::Media {
                            mode: stored_media_mode(media_mode_json, media_video, media_audio)
                                .ok_or("media task without a usable track selection")?,
                            headers,
                        }
                    } else if kind == "ftp" {
                        WorkKind::Ftp
                    } else {
                        WorkKind::Http {
                            headers,
                            connections: connections as usize,
                            method,
                            body,
                            control: control.clone().ok_or("HTTP control missing")?,
                        }
                    };
                    inner.db.execute("UPDATE tasks SET status='running',retry_at=NULL,updated_at=CURRENT_TIMESTAMP WHERE id=?1", [id])?;
                    let stop = Arc::new(AtomicBool::new(false));
                    inner.active.insert(
                        id,
                        Active {
                            queue_id,
                            stop: stop.clone(),
                            control,
                            ftp: ftp.clone(),
                        },
                    );
                    return Ok(Work {
                        id,
                        url,
                        output: PathBuf::from(output),
                        kind,
                        proxy: serde_json::from_str(&inner.proxy).unwrap_or_default(),
                        stop,
                        ftp,
                    });
                }
            }
            let retry_due: Option<i64> = inner.db.query_row(
                "SELECT MIN(retry_at) FROM tasks WHERE status='queued' AND retry_at IS NOT NULL",
                [],
                |row| row.get(0),
            )?;
            let completion_due = inner.pending.values().map(|pending| pending.deadline).min();
            let scheduled: bool = inner.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM queues WHERE sched_kind<>'none' OR sched_until IS NOT NULL)",
                [],
                |row| row.get(0),
            )?;
            let next_deadline =
                [inner.queue_start_at, inner.queue_stop_at, retry_due, completion_due]
                    .into_iter()
                    .flatten()
                    .min();
            let wait = next_deadline
                .map(|at| Duration::from_secs((at - now).max(1) as u64))
                .unwrap_or(Duration::from_secs(30))
                .min(Duration::from_secs(if scheduled { 1 } else { 30 }));
            inner = self
                .wake
                .wait_timeout(inner, wait)
                .map_err(|_| io::Error::other("task lock poisoned"))?
                .0;
        }
    }

    fn run(self: Arc<Self>) {
        loop {
            let work = match self.next_work() {
                Ok(work) => work,
                Err(e) => {
                    eprintln!("queue error: {e}");
                    thread::sleep(Duration::from_secs(1));
                    continue;
                }
            };
            let service = self.clone();
            thread::spawn(move || {
                let result: Result<()> = match work.kind {
                    WorkKind::Http {
                        headers,
                        connections,
                        method,
                        body,
                        control,
                    } => download_with_control(
                        Options {
                            url: work.url,
                            output: work.output,
                            connections,
                            restart: false,
                            headers,
                            method,
                            body,
                            bytes_per_second: None,
                            proxy: work.proxy.clone(),
                            stop: work.stop.clone(),
                        },
                        control,
                    ),
                    WorkKind::Ftp => match work.ftp.clone() {
                        None => Err("FTP progress holder missing".into()),
                        Some(live) => xdm_ftp::download(
                            xdm_ftp::FtpOptions {
                                url: work.url,
                                output: work.output,
                                restart: false,
                                stop: work.stop.clone(),
                            },
                            |done, total| live.report(done, total),
                        ),
                    },
                    WorkKind::Media { mode, headers } => match &mode {
                        MediaMode::Video {
                            video,
                            audio,
                            subtitles,
                        } => xdm_media::download_with_headers(
                            &work.url,
                            &work.output,
                            *video,
                            *audio,
                            subtitles,
                            &headers,
                            &work.stop,
                            true,
                        ),
                        MediaMode::Audio { track, format } => xdm_media::download_audio(
                            &work.url,
                            &work.output,
                            *track,
                            engine_audio_format(*format),
                            &headers,
                            &work.stop,
                        ),
                        MediaMode::Subtitles { track } => xdm_media::save_subtitles(
                            &work.url,
                            &work.output,
                            *track,
                            &headers,
                            &work.stop,
                        ),
                    },
                };
                if let Ok(mut inner) = service.inner.lock() {
                    let interrupted_for_queue = inner.queue_interrupted.remove(&work.id);
                    let (status, error, challenge) = worker_outcome(
                        &result,
                        work.stop.load(Ordering::Relaxed),
                        interrupted_for_queue,
                    );
                    let _ = inner.finish_task(
                        work.id,
                        status,
                        error,
                        challenge.and_then(|challenge| challenge.realm),
                    );
                    inner.active.remove(&work.id);
                    service.wake.notify_one();
                }
            });
        }
    }
}

/// Maps a worker result to the recorded task outcome. A 401/407 challenge
/// becomes `auth_required` instead of `failed` so credentials can be offered
/// once; the queue retry policy never sees authentication failures.
fn worker_outcome(
    result: &Result<()>,
    stopped: bool,
    interrupted: bool,
) -> (&'static str, Option<String>, Option<xdm_http::AuthChallenge>) {
    match result {
        Ok(()) => ("complete", None, None),
        Err(_) if stopped && interrupted => ("queued", None, None),
        Err(e) if stopped => ("paused", Some(e.to_string()), None),
        Err(e) => match xdm_http::auth_challenge(e.as_ref()) {
            Some(challenge) => ("auth_required", Some(e.to_string()), Some(challenge)),
            None => ("failed", Some(e.to_string()), None),
        },
    }
}

impl Inner {
    /// Records a worker outcome. Completed tasks leave their queue (history
    /// kept); failed tasks with a transient error are re-queued per the
    /// queue's retry policy. Manual and queue stops never reach the retry path.
    fn finish_task(
        &mut self,
        id: i64,
        status: &str,
        error: Option<String>,
        auth_realm: Option<String>,
    ) -> Result<()> {
        let (queue_id, count): (i64, i64) = self.db.query_row(
            "SELECT queue_id,retry_count FROM tasks WHERE id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if status == "complete" {
            self.db.execute("UPDATE tasks SET status='complete',error=NULL,auth_realm=NULL,history_queue_id=queue_id,queue_id=0,retry_at=NULL,updated_at=CURRENT_TIMESTAMP WHERE id=?1", [id])?;
            return Ok(());
        }
        if status == "auth_required" {
            self.db.execute(
                "UPDATE tasks SET status='auth_required',error=?1,auth_realm=?2,retry_at=NULL,updated_at=CURRENT_TIMESTAMP WHERE id=?3",
                params![error, auth_realm, id],
            )?;
            return Ok(());
        }
        if status == "failed" {
            let (max, delay): (i64, i64) = self
                .db
                .query_row(
                    "SELECT retry_max,retry_delay FROM queues WHERE id=?1",
                    [queue_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?
                .unwrap_or((0, 30));
            let transient = !error.as_deref().is_some_and(is_permanent_error);
            if transient && (max < 0 || count < max) {
                self.db.execute(
                    "UPDATE tasks SET status='queued',error=?1,retry_count=retry_count+1,retry_at=?2,updated_at=CURRENT_TIMESTAMP WHERE id=?3",
                    params![error, self.clock.now() + delay, id],
                )?;
                return Ok(());
            }
        }
        self.db.execute(
            "UPDATE tasks SET status=?1,error=?2,updated_at=CURRENT_TIMESTAMP WHERE id=?3",
            params![status, error, id],
        )?;
        // A task landing in a final failed state suppresses any destructive
        // countdown the queue still holds.
        if status == "failed" {
            cancel_pending(self, queue_id, "suppressed");
        }
        Ok(())
    }
}

fn handle(
    mut stream: interprocess::local_socket::Stream,
    service: Arc<Service>,
    token: String,
) -> Result<()> {
    let request: Request = read_frame(&mut stream)?;
    let reply = if request.version != 1 || request.token != token {
        Reply::error("unauthorized or unsupported protocol")
    } else {
        service
            .command(request.command)
            .unwrap_or_else(|e| Reply::error(e.to_string()))
    };
    write_frame(&mut stream, &reply)?;
    // This connection thread owns the shared service handle, so site grabs
    // queued by the command are spawned here.
    for job in {
        let mut inner = service
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        std::mem::take(&mut inner.pending_grabs)
    } {
        thread::spawn({
            let crawler = service.clone();
            move || {
                crawler.run_grab(
                    job.request_id,
                    job.project,
                    job.directory,
                    job.queue_id,
                    job.enqueue,
                    job.stop,
                    job.state,
                )
            }
        });
    }
    Ok(())
}

fn main() -> Result<()> {
    let paths = Paths::open()?;
    let listener = paths.listen()?;
    let service = Service::open(&paths)?;
    let worker = service.clone();
    thread::spawn(move || worker.run());
    eprintln!("xdm-daemon listening locally");
    for connection in listener.incoming() {
        match connection {
            Ok(stream) => {
                let service = service.clone();
                let token = paths.token.clone();
                thread::spawn(move || {
                    if let Err(e) = handle(stream, service, token) {
                        eprintln!("client error: {e}");
                    }
                });
            }
            Err(e) => eprintln!("accept error: {e}"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::schedule::tests::FakeClock;
    use super::*;
    use os_actions::RecordOnly;
    use std::sync::atomic::AtomicUsize;
    use xdm_ipc::ProxyChoice;

    struct Counting(AtomicUsize);

    impl CompletionAdapter for Counting {
        fn run(&self, _queue_id: i64, _action: &str, _armed: bool) -> String {
            self.0.fetch_add(1, Ordering::Relaxed);
            "fake".into()
        }
    }

    fn service(now: i64) -> (Arc<Service>, Arc<FakeClock>, Arc<Counting>) {
        let clock = Arc::new(FakeClock::new(now, 0));
        let adapter = Arc::new(Counting(AtomicUsize::new(0)));
        let service = Service::open_db(
            std::path::Path::new(":memory:"),
            clock.clone(),
            adapter.clone(),
        )
        .unwrap();
        (service, clock, adapter)
    }

    fn submit(service: &Service, name: &str) -> i64 {
        service
            .command(Command::Submit {
                request_id: name.into(),
                url: format!("http://127.0.0.1:1/{name}"),
                output: format!("/nonexistent-xdm-test/{name}"),
                headers: Vec::new(),
                priority: 1,
                connections: 1,
            })
            .unwrap()
            .task_id
            .unwrap()
    }

    fn queue(service: &Service, name: &str) -> i64 {
        service
            .command(Command::QueueCreate {
                name: name.into(),
                limit: 1,
            })
            .unwrap()
            .queue_id
            .unwrap()
    }

    fn submit_request(service: &Service, name: &str, method: &str, body: Option<&[u8]>) -> Reply {
        service
            .command(Command::SubmitRequest {
                request_id: name.into(),
                url: format!("http://127.0.0.1:1/{name}"),
                output: format!("/nonexistent-xdm-test/{name}"),
                headers: Vec::new(),
                method: method.into(),
                body: body.map(<[u8]>::to_vec),
                priority: 1,
                connections: 1,
            })
            .unwrap()
    }

    fn stored_details(service: &Service, id: i64) -> (String, Option<Vec<u8>>) {
        let inner = service.inner.lock().unwrap();
        inner
            .db
            .query_row("SELECT method,body FROM tasks WHERE id=?1", [id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap()
    }

    fn summary(service: &Service, queue_id: i64) -> QueueSummary {
        let inner = service.inner.lock().unwrap();
        queue_summaries(&inner)
            .unwrap()
            .into_iter()
            .find(|row| row.id == queue_id)
            .unwrap()
    }

    fn order(service: &Service, queue_id: i64) -> Vec<i64> {
        let inner = service.inner.lock().unwrap();
        inner
            .db
            .prepare("SELECT id FROM tasks WHERE queue_id=?1 ORDER BY priority DESC,position,id")
            .unwrap()
            .query_map([queue_id], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    fn finish(service: &Service, id: i64, status: &str, error: Option<&str>) {
        let mut inner = service.inner.lock().unwrap();
        inner
            .db
            .execute("UPDATE tasks SET status='running' WHERE id=?1", [id])
            .unwrap();
        inner
            .finish_task(id, status, error.map(str::to_string), None)
            .unwrap();
    }

    fn task_state(service: &Service, id: i64) -> (String, i64, Option<i64>, i64) {
        let inner = service.inner.lock().unwrap();
        inner
            .db
            .query_row(
                "SELECT status,retry_count,retry_at,queue_id FROM tasks WHERE id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap()
    }

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
    fn retry_policy_counts_transient_only() {
        let (service, clock, _) = service(1000);
        let q = queue(&service, "Retry");
        let id = submit(&service, "r0");
        service.command(Command::AssignQueue { queue_id: q, task_ids: vec![id] }).unwrap();
        assert!(!service.command(Command::QueueSetRetry { queue_id: q, max_retries: -2, delay_seconds: 1 }).unwrap().ok);
        // Default policy: no retries.
        finish(&service, id, "failed", Some("HTTP 503"));
        assert_eq!(task_state(&service, id).0, "failed");
        service.command(Command::Resume { task_id: id }).unwrap();
        service.command(Command::QueueSetRetry { queue_id: q, max_retries: 2, delay_seconds: 30 }).unwrap();
        finish(&service, id, "failed", Some("connection reset"));
        assert_eq!(task_state(&service, id), ("queued".into(), 1, Some(1030), q));
        clock.set(2000);
        finish(&service, id, "failed", Some("HTTP 503"));
        assert_eq!(task_state(&service, id), ("queued".into(), 2, Some(2030), q));
        finish(&service, id, "failed", Some("HTTP 503"));
        assert_eq!(task_state(&service, id).0, "failed");
        // Resume starts a fresh budget; permanent errors never retry.
        service.command(Command::Resume { task_id: id }).unwrap();
        assert_eq!(task_state(&service, id), ("queued".into(), 0, None, q));
        for status in ["401", "403", "404", "410"] {
            finish(&service, id, "failed", Some(&format!("download returned HTTP {status}")));
            assert_eq!(task_state(&service, id), ("failed".into(), 0, None, q));
            service.command(Command::Resume { task_id: id }).unwrap();
        }
        // Infinite retries and a manual pause (never the retry path).
        service.command(Command::QueueSetRetry { queue_id: q, max_retries: -1, delay_seconds: 5 }).unwrap();
        for expected in 1..=5 {
            finish(&service, id, "failed", Some("timeout"));
            assert_eq!(task_state(&service, id).1, expected);
        }
        finish(&service, id, "paused", Some("stopped"));
        assert_eq!(task_state(&service, id).1, 5);
        assert!(is_permanent_error("download returned HTTP 410"));
        assert!(!is_permanent_error("probe failed: HTTP 503"));
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
            inner.finish_task(id, "complete", None, None).unwrap();
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
    fn submit_request_validates_and_persists_method_body() {
        let (service, _, _) = service(1000);
        let ok = submit_request(&service, "post-ok", "post", Some(b"token=abc"));
        assert!(ok.ok);
        let details = stored_details(&service, ok.task_id.unwrap());
        assert_eq!(
            (details.0.as_str(), details.1.as_deref()),
            ("POST", Some(b"token=abc".as_slice()))
        );
        // Identical replay is idempotent; divergent fields are rejected.
        assert!(submit_request(&service, "post-ok", "POST", Some(b"token=abc")).ok);
        assert!(!submit_request(&service, "post-ok", "POST", Some(b"other=xy")).ok);
        for (name, method, body) in [
            ("get-body", "get", Some(b"a=1".as_slice())),
            ("over-size", "post", Some(&vec![0u8; xdm_http::MAX_BODY + 1][..])),
            ("bad-token", "bad method", Some(b"a=1")),
        ] {
            assert!(!submit_request(&service, name, method, body).ok, "{name}");
        }
        assert!(submit_request(&service, "plain-get", "GET", None).ok);
    }

    #[test]
    fn redownload_keeps_method_and_body() {        let (service, _, _) = service(1000);
        let source = submit_request(&service, "post-src", "POST", Some(b"token=abc"))
            .task_id
            .unwrap();
        let reply = service
            .command(Command::Redownload {
                task_id: source,
                request_id: "post-clone".into(),
                output: "/nonexistent-xdm-test/clone.bin".into(),
            })
            .unwrap();
        assert!(reply.ok);
        let details = stored_details(&service, reply.task_id.unwrap());
        assert_eq!(
            (details.0.as_str(), details.1.as_deref()),
            ("POST", Some(b"token=abc".as_slice()))
        );
    }

    #[test]
    fn queue_work_carries_method_body_and_global_proxy() {
        let (service, _, _) = service(1000);
        let manual = ProxyChoice::Manual {
            http: Some("http://127.0.0.1:8899".into()),
            https: None,
            bypass: vec!["10.0.0.0/8".into()],
        };
        assert!(service.command(Command::SetProxy { proxy: manual.clone() }).unwrap().ok);
        let id = submit_request(&service, "queued-post", "post", Some(b"token=abc"))
            .task_id
            .unwrap();
        let work = service.next_work().unwrap();
        assert_eq!(work.id, id);
        let expected: Proxy = serde_json::from_value(serde_json::to_value(&manual).unwrap()).unwrap();
        assert_eq!(work.proxy, expected);
        let WorkKind::Http { method, body, .. } = work.kind else { unreachable!() };
        assert_eq!((method.as_str(), body.as_deref()), ("POST", Some(b"token=abc".as_slice())));
    }

    #[test]
    fn proxy_setting_rejects_bad_urls_and_survives_reopen() {
        let (service, _, _) = service(1000);
        let proxy = |service: &Service| service.command(Command::GetProxy).unwrap().proxy.unwrap();
        assert_eq!(proxy(&service), serde_json::json!({"mode": "system"}));
        assert!(
            !service
                .command(Command::SetProxy {
                    proxy: ProxyChoice::Manual {
                        http: Some("ftp://proxy.invalid".into()),
                        https: None,
                        bypass: Vec::new(),
                    }
                })
                .unwrap()
                .ok
        );
        assert!(service.command(Command::SetProxy { proxy: ProxyChoice::Direct }).unwrap().ok);
        assert_eq!(proxy(&service), serde_json::json!({"mode": "direct"}));
        let path = std::env::temp_dir().join(format!("xdm-proxy-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let file_service =
                Service::open_db(&path, Arc::new(FakeClock::new(1000, 0)), Arc::new(RecordOnly))
                    .unwrap();
            assert!(file_service.command(Command::SetProxy { proxy: ProxyChoice::Direct }).unwrap().ok);
        }
        let persisted =
            Service::open_db(&path, Arc::new(FakeClock::new(1000, 0)), Arc::new(RecordOnly)).unwrap();
        assert_eq!(proxy(&persisted), serde_json::json!({"mode": "direct"}));
        drop(persisted);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn ftp_tasks_are_accepted_and_selected_as_work() {
        let (service, _, _) = service(1000);
        let ftp = service
            .command(Command::Submit {
                request_id: "ftp-1".into(),
                url: "ftp://downloads.invalid/pub/pubbig.bin".into(),
                output: "/nonexistent-xdm-test/ftp.bin".into(),
                headers: Vec::new(),
                priority: 1,
                connections: 1,
            })
            .unwrap();
        assert!(ftp.ok, "{:?}", ftp.error);
        let task = service
            .command(Command::GetTask { task_id: ftp.task_id.unwrap() })
            .unwrap()
            .tasks
            .remove(0);
        assert_eq!(task.kind, "ftp");
        assert!(task.category == "Programs" || task.category == "Other");
        // Other URL schemes stay rejected.
        assert!(!service
            .command(Command::Submit {
                request_id: "bad-scheme".into(),
                url: "ftps://downloads.invalid/x".into(),
                output: "/nonexistent-xdm-test/bad".into(),
                headers: Vec::new(),
                priority: 1,
                connections: 1,
            })
            .unwrap()
            .ok);
        let work = service.next_work().unwrap();
        assert_eq!(work.id, ftp.task_id.unwrap());
        assert!(matches!(work.kind, WorkKind::Ftp));
        assert!(work.ftp.is_some());
    }

    #[test]
    fn categories_list_save_and_remove_persist() {
        let (service, _, _) = service(1000);
        let listing = service.command(Command::CategoryList).unwrap();
        let names: Vec<String> = listing
            .categories
            .iter()
            .map(|rule| rule.name.clone())
            .collect();
        assert_eq!(names, xdm_categories::CATEGORIES.iter().map(|name| name.to_string()).collect::<Vec<_>>());
        service
            .command(Command::CategorySave {
                name: "Video".into(),
                extensions: vec!["mp4".into(), "webm".into()],
                destination: "/media".into(),
                sites: Vec::new(),
            })
            .unwrap();
        assert!(!service
            .command(Command::CategoryRemove { name: "video".into() })
            .unwrap()
            .ok);
        let rule = |service: &Service, name: &str| {
            service
                .command(Command::CategoryList)
                .unwrap()
                .categories
                .into_iter()
                .find(|rule| rule.name == name)
                .unwrap()
        };
        assert_eq!(rule(&service, "Video").destination, "/media");
        // Rule edits survive a reopen.
        let path = std::env::temp_dir().join(format!("xdm-category-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let file_service =
                Service::open_db(&path, Arc::new(FakeClock::new(1000, 0)), Arc::new(RecordOnly))
                    .unwrap();
            assert!(file_service
                .command(Command::CategorySave {
                    name: "Docs".into(),
                    extensions: Vec::new(),
                    destination: String::new(),
                    sites: vec!["d.test".into()],
                })
                .unwrap()
                .ok);
        }
        let reopened =
            Service::open_db(&path, Arc::new(FakeClock::new(1000, 0)), Arc::new(RecordOnly)).unwrap();
        assert_eq!(rule(&reopened, "Docs").sites, vec!["d.test".to_string()]);
        assert!(reopened
            .command(Command::CategoryRemove { name: "Docs".into() })
            .unwrap()
            .ok);
        assert!(!reopened
            .command(Command::CategoryRemove { name: "Docs".into() })
            .unwrap()
            .ok);
        drop(reopened);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn submissions_classify_through_user_rules() {
        let (service, _, _) = service(1000);
        for (name, extensions, sites) in [
            ("Ebooks", vec!["epub".to_string()], Vec::new()),
            ("Publisher", Vec::new(), vec!["*.pub.test".to_string()]),
        ] {
            assert!(service
                .command(Command::CategorySave {
                    name: name.into(),
                    extensions,
                    destination: String::new(),
                    sites,
                })
                .unwrap()
                .ok);
        }
        let category = |request_id: &str, url: &str, output: &str| {
            let reply = service
                .command(Command::Submit {
                    request_id: request_id.into(),
                    url: url.into(),
                    output: output.into(),
                    headers: Vec::new(),
                    priority: 1,
                    connections: 1,
                })
                .unwrap();
            assert!(reply.ok, "{:?}", reply.error);
            service
                .command(Command::GetTask { task_id: reply.task_id.unwrap() })
                .unwrap()
                .tasks
                .remove(0)
                .category
        };
        assert_eq!(
            category("rule-ext", "http://plain.test/x/letter.epub", "/nonexistent-xdm-test/letter.epub"),
            "Ebooks"
        );
        // A host match beats an extension match on the same submission.
        assert_eq!(
            category("rule-site", "http://www.pub.test/x.zip", "/nonexistent-xdm-test/marsh.zip"),
            "Publisher"
        );
    }

    #[test]
    fn grab_filenames_are_decoded_and_sanitized() {
        assert_eq!(
            grab_filename("https://a.test/dir/Big%20Report.PDF?q=1"),
            "Big Report.PDF"
        );
        assert_eq!(grab_filename("https://a.test/dir/no#anchor"), "no");
        assert_eq!(grab_filename("https://a.test/"), "download.bin");
        assert_eq!(grab_filename("bad url"), "download.bin");
    }

    #[test]
    fn media_job_validates_modes_and_persists_mode_and_headers() {
        let (service, _, _) = service(1000);
        let audio = |request_id: &str, output: &str, format: MediaAudioFormat| {
            service
                .command(Command::SubmitMediaJob {
                    request_id: request_id.into(),
                    url: "http://127.0.0.1:1/master.m3u8".into(),
                    output: output.into(),
                    headers: vec![("Cookie".into(), "xdm-test=1".into())],
                    mode: MediaMode::Audio { track: 1, format },
                    priority: 1,
                })
                .unwrap()
        };
        let accepted = audio("m-audio", "/nonexistent-xdm-test/song.mp3", MediaAudioFormat::Mp3 { bitrate_kbps: 96 });
        assert!(accepted.ok, "{:?}", accepted.error);
        // Identical replay is idempotent; a different target is rejected.
        assert!(accepted.task_id.unwrap() >= 1);
        assert!(audio("m-audio", "/nonexistent-xdm-test/song.mp3", MediaAudioFormat::Mp3 { bitrate_kbps: 96 }).ok);
        assert!(!audio("m-audio", "/nonexistent-xdm-test/song.mp3", MediaAudioFormat::Mp3 { bitrate_kbps: 128 }).ok);
        for (name, output, format) in [
            ("bad-ext", "/nonexistent-xdm-test/song.m4a", MediaAudioFormat::Mp3 { bitrate_kbps: 96 }),
            ("bad-bitrate", "/nonexistent-xdm-test/song.mp3", MediaAudioFormat::Mp3 { bitrate_kbps: 400 }),
            ("original-wrong-container", "/nonexistent-xdm-test/song.srt", MediaAudioFormat::Original),
        ] {
            assert!(!audio(name, output, format).ok, "{name}");
        }
        // Video modes with subtitles can target MKV; sidecars need .srt/.vtt.
        for (name, output, mode) in [
            (
                "mkv-subs-ok",
                "/nonexistent-xdm-test/film.mkv",
                MediaMode::Video { video: 2, audio: None, subtitles: vec![3, 4] },
            ),
            (
                "subs-ok",
                "/nonexistent-xdm-test/film.vtt",
                MediaMode::Subtitles { track: 3 },
            ),
            (
                "video-no-manifest",
                "/nonexistent-xdm-test/film.mp4",
                MediaMode::Video { video: 2, audio: None, subtitles: Vec::new() },
            ),
        ] {
            let reply = service
                .command(Command::SubmitMediaJob {
                    request_id: name.into(),
                    url: if name == "video-no-manifest" { "http://127.0.0.1:1/plain.mp4".into() } else { "http://127.0.0.1:1/master.m3u8".into() },
                    output: output.into(),
                    headers: Vec::new(),
                    mode,
                    priority: 1,
                })
                .unwrap();
            assert_eq!(
                reply.ok,
                name == "mkv-subs-ok" || name == "subs-ok",
                "{name}: {:?}",
                (output, reply.error)
            );
        }
        // Line-breaking headers and a media job whose output is relative stay out.
        assert!(
            !service
                .command(Command::SubmitMediaJob {
                    request_id: "m-evil".into(),
                    url: "http://127.0.0.1:1/master.m3u8".into(),
                    output: "/nonexistent-xdm-test/evil.mp4".into(),
                    headers: vec![("Cookie".into(), "a\r\nEvil: x".into())],
                    mode: MediaMode::Video { video: 0, audio: None, subtitles: Vec::new() },
                    priority: 1,
                })
                .unwrap()
                .ok
        );
        assert!(
            !service
                .command(Command::SubmitMediaJob {
                    request_id: "m-relative".into(),
                    url: "http://127.0.0.1:1/master.m3u8".into(),
                    output: "song.mp3".into(),
                    headers: Vec::new(),
                    mode: MediaMode::Audio { track: 1, format: MediaAudioFormat::Mp3 { bitrate_kbps: 96 } },
                    priority: 1,
                })
                .unwrap()
                .ok
        );
        let row: String = {
            let inner = service.inner.lock().unwrap();
            inner
                .db
                .query_row(
                    "SELECT media_mode_json FROM tasks WHERE id=?1",
                    [accepted.task_id.unwrap()],
                    |row| row.get(0),
                )
                .unwrap()
        };
        assert_eq!(row, r#"{"kind":"audio","track":1,"format":{"kind":"mp3","bitrate_kbps":96}}"#);
    }

    #[test]
    fn media_work_carries_mode_and_headers_and_export_stays_portable() {
        let (service, _, _) = service(1000);
        let job = |service: &Service, request_id: &str, output: &str, mode: MediaMode| {
            service
                .command(Command::SubmitMediaJob {
                    request_id: request_id.into(),
                    url: "http://127.0.0.1:1/master.m3u8".into(),
                    output: output.into(),
                    headers: vec![("Cookie".into(), "xdm=secret".into())],
                    mode,
                    priority: 1,
                })
                .unwrap()
        };
        let mkv = job(&service, "m-mkv", "/nonexistent-xdm-test/film.mkv", MediaMode::Video { video: 2, audio: Some(3), subtitles: vec![5] }).task_id.unwrap();
        let audio_id = job(
            &service,
            "m-song",
            "/nonexistent-xdm-test/song.mp3",
            MediaMode::Audio { track: 1, format: MediaAudioFormat::Mp3 { bitrate_kbps: 96 } },
        )
        .task_id
        .unwrap();
        let work_mode = |service: &Service, id: i64| {
            inner_resume(service, id);
            let work = service.next_work().unwrap();
            assert_eq!(work.id, id);
            work
        };
        let work = work_mode(&service, mkv);
        let WorkKind::Media { mode, headers } = work.kind else { unreachable!() };
        assert_eq!(mode, MediaMode::Video { video: 2, audio: Some(3), subtitles: vec![5] });
        assert_eq!(headers, vec![("Cookie".to_owned(), "xdm=secret".to_owned())]);
        let work = work_mode(&service, audio_id);
        let WorkKind::Media { mode, headers } = work.kind else { unreachable!() };
        assert_eq!(mode, MediaMode::Audio { track: 1, format: MediaAudioFormat::Mp3 { bitrate_kbps: 96 } });
        assert_eq!(headers.len(), 1);
        // Non-portable media is omitted from exports, and headers never leave.
        let export = service.command(Command::ExportList).unwrap();
        let entries = export.transfer_list.unwrap().entries;
        assert!(!entries.iter().any(|entry| ["film.mkv", "song.mp3"].contains(&entry.filename.as_str())));
        assert!(serde_json::to_string(&entries).unwrap().contains("xdm=secret") == false);
        // A legacy SubmitMedia row still exports in the portable media shape.
        assert!(
            service
                .command(Command::SubmitMedia {
                    request_id: "legacy-media".into(),
                    url: "http://127.0.0.1:1/master.m3u8".into(),
                    output: "/nonexistent-xdm-test/plain.mp4".into(),
                    video: 2,
                    audio: Some(3),
                    priority: 1,
                })
                .unwrap()
                .ok
        );
        let export = service.command(Command::ExportList).unwrap();
        let entry = export
            .transfer_list
            .unwrap()
            .entries
            .iter()
            .find(|entry| entry.filename == "plain.mp4")
            .unwrap()
            .clone();
        assert_eq!((entry.kind.as_str(), entry.video, entry.audio), ("media", Some(2), Some(3)));
    }

    /// Resets a task to queued so work selection can pick it again.
    fn inner_resume(service: &Service, id: i64) {
        let inner = service.inner.lock().unwrap();
        inner
            .db
            .execute(
                "UPDATE tasks SET status='queued',retry_at=NULL,updated_at=CURRENT_TIMESTAMP WHERE id=?1",
                [id],
            )
            .unwrap();
    }

    #[test]
    fn old_database_migrates_without_losing_rows() {
        let path = std::env::temp_dir().join(format!("xdm-migrate-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let db = Connection::open(&path).unwrap();
            db.execute_batch(
                "CREATE TABLE tasks (id INTEGER PRIMARY KEY, request_id TEXT NOT NULL UNIQUE, url TEXT NOT NULL, output TEXT NOT NULL UNIQUE, headers_json TEXT NOT NULL, priority INTEGER NOT NULL, connections INTEGER NOT NULL, status TEXT NOT NULL, error TEXT, queue_id INTEGER NOT NULL DEFAULT 1);
                 CREATE TABLE queues (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE COLLATE NOCASE, running INTEGER NOT NULL, max_concurrent INTEGER NOT NULL);
                 INSERT INTO queues(id,name,running,max_concurrent) VALUES (1,'Main',1,16),(2,'Old',0,3);
                 INSERT INTO tasks(id,request_id,url,output,headers_json,priority,connections,status,queue_id) VALUES (5,'a','http://x/a','/o/a','[]',1,1,'complete',2),(6,'b','http://x/b','/o/b','[]',1,1,'queued',2);",
            )
            .unwrap();
        }
        let service = Service::open_db(&path, Arc::new(FakeClock::new(1000, 0)), Arc::new(RecordOnly)).unwrap();
        assert_eq!(task_state(&service, 5).3, 0);
        assert_eq!(order(&service, 2), [6]);
        let row = summary(&service, 2);
        assert_eq!((row.complete, row.queued, row.limit, row.retry_delay), (1, 1, 3, 30));
        drop(service);
        // Reopening is idempotent.
        Service::open_db(&path, Arc::new(FakeClock::new(1000, 0)), Arc::new(RecordOnly)).unwrap();
        let _ = std::fs::remove_file(&path);
    }

    fn auth_error(status: u16, realm: Option<&str>) -> Error {
        Box::new(xdm_http::AuthRequired(xdm_http::AuthChallenge {
            status,
            scheme: "Basic".into(),
            realm: realm.map(str::to_owned),
        }))
    }

    #[test]
    fn login_commands_round_trip_without_passwords() {
        let (service, _, _) = service(1000);
        assert!(service.command(Command::LoginList).unwrap().logins.is_empty());
        let saved = service
            .command(Command::LoginSave {
                pattern: "http://127.0.0.1:1/".into(),
                username: "alice".into(),
                password: "s3cret".into(),
                enabled: true,
            })
            .unwrap();
        assert_eq!(saved.logins.len(), 1);
        let summary = &saved.logins[0];
        assert_eq!(
            (summary.pattern.as_str(), summary.username.as_str(), summary.enabled),
            ("http://127.0.0.1:1/", "alice", true)
        );
        // Replies never carry the password.
        assert!(!serde_json::to_string(&saved).unwrap().contains("s3cret"));
        assert!(!serde_json::to_string(&service.command(Command::LoginList).unwrap()).unwrap().contains("s3cret"));
        // Invalid patterns and fields are rejected with no row added.
        assert!(!service
            .command(Command::LoginSave {
                pattern: "ftp://host/".into(),
                username: "bob".into(),
                password: "pw".into(),
                enabled: true,
            })
            .unwrap()
            .ok);
        assert!(!service
            .command(Command::LoginSave {
                pattern: "http://host/".into(),
                username: String::new(),
                password: "pw".into(),
                enabled: true,
            })
            .unwrap()
            .ok);
        assert!(service.command(Command::LoginList).unwrap().logins.len() == 1);
        assert!(service.command(Command::LoginRemove { id: 999 }).unwrap().error.is_some());
        assert!(service.command(Command::LoginRemove { id: summary.id }).unwrap().ok);
        assert!(service.command(Command::LoginList).unwrap().logins.is_empty());
    }

    #[test]
    fn http_work_injects_matching_login_and_keeps_explicit_headers() {
        let (service, _, _) = service(1000);
        service.command(Command::SetConcurrency { limit: 16 }).unwrap();
        service
            .command(Command::LoginSave {
                pattern: "http://127.0.0.1:1/".into(),
                username: "alice".into(),
                password: "s3cret".into(),
                enabled: true,
            })
            .unwrap();
        let injected = submit(&service, "auth-inject");
        let work = service.next_work().unwrap();
        assert_eq!(work.id, injected);
        let WorkKind::Http { headers, .. } = work.kind else { unreachable!() };
        assert_eq!(
            headers,
            vec![("Authorization".into(), format!("Basic {}", credentials::basic_token("alice", "s3cret")))]
        );
        // A submitter-provided Authorization header is never overridden, and a
        // disabled login does not apply.
        service
            .command(Command::LoginSave {
                pattern: "http://127.0.0.1:1/".into(),
                username: "alice".into(),
                password: "s3cret".into(),
                enabled: false,
            })
            .unwrap();
        let explicit = service
            .command(Command::Submit {
                request_id: "auth-explicit".into(),
                url: "http://127.0.0.1:1/explicit.bin".into(),
                output: "/nonexistent-xdm-test/auth-explicit.bin".into(),
                headers: vec![("Authorization".into(), "Bearer supplied".into())],
                priority: 2,
                connections: 1,
            })
            .unwrap()
            .task_id
            .unwrap();
        let work = service.next_work().unwrap();
        assert_eq!(work.id, explicit);
        let WorkKind::Http { headers, .. } = work.kind else { unreachable!() };
        assert_eq!(headers, vec![("Authorization".into(), "Bearer supplied".into())]);
        // With no matching enabled login the task leaves with clean headers.
        let plain = submit(&service, "auth-plain");
        let work = service.next_work().unwrap();
        assert_eq!(work.id, plain);
        let WorkKind::Http { headers, .. } = work.kind else { unreachable!() };
        assert!(headers.is_empty());
    }

    #[test]
    fn auth_challenge_sets_auth_required_and_provide_login_requeues() {
        let (service, _, _) = service(1000);
        let id = submit(&service, "auth-flow");
        // The worker outcome of a 401 challenge is auth_required, never failed.
        {
            let mut inner = service.inner.lock().unwrap();
            inner
                .db
                .execute("UPDATE tasks SET status='running' WHERE id=?1", [id])
                .unwrap();
            let result: Result<()> = Err(auth_error(401, Some("xdm-test")));
            let (status, message, challenge) = worker_outcome(&result, false, false);
            assert_eq!(status, "auth_required");
            assert!(message.as_deref().is_some_and(|text| text.contains("HTTP 401")));
            assert_eq!(challenge.as_ref().unwrap().realm.as_deref(), Some("xdm-test"));
            inner
                .finish_task(id, status, message, challenge.and_then(|c| c.realm))
                .unwrap();
        }
        let task = service
            .command(Command::GetTask { task_id: id })
            .unwrap()
            .tasks
            .remove(0);
        assert_eq!(task.status, "auth_required");
        assert_eq!(task.auth_realm.as_deref(), Some("xdm-test"));
        // A paused worker stays paused even if the error carries a challenge.
        {
            let result: Result<()> = Err(auth_error(401, Some("xdm-test")));
            assert_eq!(worker_outcome(&result, true, false).0, "paused");
            assert_eq!(worker_outcome(&result, true, true).0, "queued");
            let plain: Result<()> = Err("probe failed: HTTP 403".into());
            assert_eq!(worker_outcome(&plain, false, false).0, "failed");
        }
        // Supplying a login requeues the task; without remember nothing is stored.
        let provided = service
            .command(Command::TaskProvideLogin {
                task_id: id,
                username: "alice".into(),
                password: "pw".into(),
                remember: false,
            })
            .unwrap();
        assert!(provided.ok, "{:?}", provided.error);
        assert!(provided.logins.is_empty());
        let task = service.command(Command::GetTask { task_id: id }).unwrap().tasks.remove(0);
        assert_eq!((task.status.as_str(), task.auth_realm.as_deref()), ("queued", None));
        // A second challenge, then the credentials are remembered for the origin.
        {
            let mut inner = service.inner.lock().unwrap();
            inner
                .db
                .execute("UPDATE tasks SET status='running' WHERE id=?1", [id])
                .unwrap();
            let result: Result<()> = Err(auth_error(401, Some("xdm-test")));
            let (status, message, challenge) = worker_outcome(&result, false, false);
            inner.finish_task(id, status, message, challenge.and_then(|c| c.realm)).unwrap();
        }
        let remembered = service
            .command(Command::TaskProvideLogin {
                task_id: id,
                username: "alice".into(),
                password: "pw2".into(),
                remember: true,
            })
            .unwrap();
        assert!(remembered.ok, "{:?}", remembered.error);
        assert!(!serde_json::to_string(&remembered).unwrap().contains("pw2"));
        let logins = remembered.logins;
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].pattern, "http://127.0.0.1:1/");
        assert_eq!(credentials::parse_pattern(&logins[0].pattern).unwrap().matches(&Url::parse("http://127.0.0.1:1/auth-flow").unwrap()), true);
        // Providing again is refused once the task left auth_required.
        assert!(!service
            .command(Command::TaskProvideLogin {
                task_id: id,
                username: "alice".into(),
                password: "pw".into(),
                remember: false,
            })
            .unwrap()
            .ok);
        assert!(!service
            .command(Command::TaskProvideLogin {
                task_id: 999,
                username: "alice".into(),
                password: "pw".into(),
                remember: false,
            })
            .unwrap()
            .ok);
    }

    #[test]
    fn credentials_file_lives_next_to_the_database() {
        let directory = std::env::temp_dir().join(format!("xdm-auth-db-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let database = directory.join("tasks.sqlite3");
        {
            let file_service =
                Service::open_db(&database, Arc::new(FakeClock::new(1000, 0)), Arc::new(RecordOnly))
                    .unwrap();
            assert!(file_service
                .command(Command::LoginSave {
                    pattern: "https://files.example.com/".into(),
                    username: "alice".into(),
                    password: "s3cret".into(),
                    enabled: true,
                })
                .unwrap()
                .ok);
        }
        let store = directory.join("credentials.json");
        let stored = std::fs::read_to_string(&store).unwrap();
        assert!(stored.contains("s3cret"), "the store is the only place a password appears");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&store).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let reopened =
            Service::open_db(&database, Arc::new(FakeClock::new(1000, 0)), Arc::new(RecordOnly))
                .unwrap();
        let logins = reopened.command(Command::LoginList).unwrap().logins;
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].pattern, "https://files.example.com/");
        assert!(!serde_json::to_string(&logins).unwrap().contains("s3cret"));
        drop(reopened);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
