mod categories;
mod credentials;
mod grabber;
mod logins;
mod media;
mod os_actions;
mod queues;
mod schedule;
#[cfg(test)]
pub(crate) mod test_helpers;
mod transfer_lists;

use categories::*;
use grabber::*;
use interprocess::local_socket::prelude::*;
use media::*;
use queues::*;
use transfer_lists::*;
use os_actions::CompletionAdapter;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use url::Url;
use xdm_http::{Options, Progress, Proxy, TransferControl, download_with_control};
use schedule::{Clock, SystemClock};
use xdm_ipc::{
    Command, MediaMode, Paths, Reply, Request, TaskSummary, read_frame, write_frame,
};
#[cfg(test)]
use xdm_ipc::MediaAudioFormat;

pub(crate) type Error = Box<dyn std::error::Error + Send + Sync>;
pub(crate) type Result<T> = std::result::Result<T, Error>;

pub(crate) struct Inner {
    pub(crate) db: Connection,
    pub(crate) active: HashMap<i64, Active>,
    pub(crate) max_concurrent: usize,
    pub(crate) queue_running: bool,
    pub(crate) queue_start_at: Option<i64>,
    pub(crate) queue_stop_at: Option<i64>,
    pub(crate) queue_interrupted: HashSet<i64>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) adapter: Arc<dyn CompletionAdapter>,
    /// Destructive actions counting down, keyed by queue.
    pub(crate) pending: HashMap<i64, PendingCompletion>,
    /// Global proxy route for HTTP tasks, stored as canonical JSON.
    pub(crate) proxy: String,
    /// Saved site logins for HTTP authentication (N10).
    pub(crate) credentials: credentials::Store,
    /// Live crawls keyed by their site-grab request ID.
    pub(crate) grabs: HashMap<String, GrabHandle>,
    /// Crawls waiting for the worker loop, which owns the service handle.
    pub(crate) pending_grabs: Vec<GrabJob>,
}

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

pub(crate) struct Active {
    pub(crate) queue_id: i64,
    pub(crate) stop: Arc<AtomicBool>,
    pub(crate) control: Option<Arc<TransferControl>>,
    /// Live counters for single-stream FTP transfers.
    pub(crate) ftp: Option<Arc<FtpLive>>,
    /// Recorded-bytes/seconds progress for live media jobs.
    pub(crate) media: Option<Arc<MediaLive>>,
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

/// Live progress for one live-recording media job: recorded bytes with the
/// wall-clock capture seconds, both kept in atomics so the daemon lock never
/// waits on the recorder.
#[derive(Debug)]
pub(crate) struct MediaLive {
    /// Bytes ffmpeg has written so far, sampled from the temporary file.
    downloaded: AtomicU64,
    started: Instant,
}

impl MediaLive {
    fn new() -> Self {
        Self { downloaded: AtomicU64::new(0), started: Instant::now() }
    }

    /// Called from the recorder's sampling thread while ffmpeg is running.
    fn report(&self, downloaded: u64) {
        self.downloaded.store(downloaded, Ordering::Relaxed);
    }

    fn progress(&self) -> (u64, u64) {
        (
            self.downloaded.load(Ordering::Relaxed),
            self.started.elapsed().as_secs(),
        )
    }
}

pub(crate) struct Service {
    /// Shared state handle so short-lived background threads (site grabs) can
    /// be spawned from any `&self` command handler.
    pub(crate) inner: Arc<Mutex<Inner>>,
    pub(crate) wake: Arc<Condvar>,
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
    /// Live-recording progress holder, mirrored inside the active entry.
    media: Option<Arc<MediaLive>>,
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

pub(crate) fn unix_now() -> i64 {
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

pub(crate) fn set_setting(db: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    db.execute("INSERT INTO settings(key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key,value])?;
    Ok(())
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
            // A live recording reports its captured bytes and seconds; the
            // recorder has no total to show, so only the counters fill in.
            let recording = inner
                .active
                .get(&id)
                .and_then(|active| active.media.as_ref())
                .map(|media| media.progress());
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
                    .or(recording.map(|(bytes, _)| bytes))
                    .or_else(|| progress.map(|(done, _)| done)),
                total: live
                    .and_then(|p| p.total)
                    .or_else(|| progress.and_then(|(_, total)| total)),
                media_seconds: recording.map(|(_, seconds)| seconds),
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

pub(crate) fn next_task_id(db: &Connection) -> rusqlite::Result<i64> {
    db.query_row("SELECT MAX(COALESCE((SELECT MAX(id) FROM tasks),0),COALESCE((SELECT MAX(task_id) FROM removed_tasks),0))+1", [], |row| row.get(0))
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
            Command::QueueCreate { name, limit } => self.queue_create(name, limit),
            Command::QueueSetRunning { queue_id, running } => {
                self.queue_set_running(queue_id, running)
            }
            Command::QueueSetConcurrency { queue_id, limit } => {
                self.queue_set_concurrency(queue_id, limit)
            }
            Command::QueueRename { queue_id, name } => self.queue_rename(queue_id, name),
            Command::QueueRemove { queue_id } => self.queue_remove(queue_id),
            Command::AssignQueue { queue_id, task_ids } => self.assign_queue(queue_id, task_ids),
            Command::ImportList {
                request_id,
                directory,
                entries,
                enqueue,
            } => self.import_list(request_id, directory, entries, enqueue),
            Command::ExportList => self.export_list(),
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
                    MediaMode::Video { .. } | MediaMode::Live { .. } => "Video".to_string(),
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
                    MediaMode::Video { video, audio, .. }
                    | MediaMode::Live { video, audio, .. } => {
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
            Command::QueueStart => self.queue_start(),
            Command::QueueStop => self.queue_stop(),
            Command::QueueScheduleStart { at_unix } => self.queue_schedule_start(at_unix),
            Command::QueueScheduleStop { at_unix } => self.queue_schedule_stop(at_unix),
            Command::QueueStartOne { queue_id } => self.queue_start_one(queue_id),
            Command::QueueStopOne { queue_id } => self.queue_stop_one(queue_id),
            Command::TaskMove { task_id, target } => self.task_move(task_id, target),
            Command::QueueSetRetry {
                queue_id,
                max_retries,
                delay_seconds,
            } => self.queue_set_retry(queue_id, max_retries, delay_seconds),
            Command::QueueSetSchedule {
                queue_id,
                kind,
                start,
                stop,
                days,
            } => self.queue_set_schedule(queue_id, kind, start, stop, days),
            Command::QueueSetCompletion { queue_id, action } => {
                self.queue_set_completion(queue_id, action)
            }
            Command::QueueSetCompletionAction {
                queue_id,
                action,
                confirm,
                countdown_seconds,
            } => self.queue_set_completion_action(queue_id, action, confirm, countdown_seconds),
            Command::CompletionStatus => self.completion_status(),
            Command::CompletionCancel { queue_id } => self.completion_cancel(queue_id),
            Command::QueueEvents { queue_id } => self.queue_events_command(queue_id),
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
            Command::LoginList => self.login_list(),
            Command::LoginSave {
                pattern,
                username,
                password,
                enabled,
            } => self.login_save(pattern, username, password, enabled),
            Command::LoginRemove { id } => self.login_remove(id),
            Command::TaskProvideLogin {
                task_id,
                username,
                password,
                remember,
            } => self.task_provide_login(task_id, username, password, remember),
        }
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
                    // own origin only, unless the submitter set a header
                    // already. Media manifests replay them the same way so
                    // ffmpeg opens every chain document authenticated.
                    if (kind == "http" || kind == "media")
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
                    let stored_mode = if kind == "media" {
                        Some(
                            stored_media_mode(media_mode_json, media_video, media_audio)
                                .ok_or("media task without a usable track selection")?,
                        )
                    } else {
                        None
                    };
                    // Only live recordings have progress to report while they run.
                    let media = stored_mode
                        .as_ref()
                        .is_some_and(|mode| matches!(mode, MediaMode::Live { .. }))
                        .then(|| Arc::new(MediaLive::new()));
                    let kind = if kind == "media" {
                        WorkKind::Media {
                            mode: stored_mode.expect("resolved above"),
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
                            media: media.clone(),
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
                        media,
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
                        MediaMode::Live {
                            video,
                            audio,
                            max_seconds,
                        } => match work.media.clone() {
                            None => Err("live media task without a progress holder".into()),
                            Some(live) => {
                                // The recorder's stop-and-save publish keeps
                                // the capture usable: pausing a live task
                                // finishes ffmpeg gracefully and `record_live`
                                // returns the completed file (the task
                                // completes, not pauses). A VOD manifest keeps
                                // the pause semantics and is the documented
                                // fallback for Live submissions.
                                let temporary = media::media_part_path(&work.output);
                                // A restart re-queues interrupted jobs; like
                                // the video mode's stale-part recovery, the
                                // leftover recorder temporary of the same task
                                // is removed so the capture can restart.
                                let _ = std::fs::remove_file(&temporary);
                                let finished = Arc::new(AtomicBool::new(false));
                                // A sampling thread keeps the recorded-byte
                                // counter fresh; the daemon lock reads the
                                // atomic only.
                                let sampler = {
                                    let temporary = temporary.clone();
                                    let live = live.clone();
                                    let finished = finished.clone();
                                    thread::spawn(move || {
                                        while !finished.load(Ordering::Relaxed) {
                                            if let Ok(meta) = std::fs::metadata(&temporary) {
                                                live.report(meta.len());
                                            }
                                            thread::sleep(Duration::from_secs(1));
                                        }
                                    })
                                };
                                let result = xdm_media::record_live(
                                    &work.url,
                                    &work.output,
                                    *video,
                                    *audio,
                                    &headers,
                                    &work.stop,
                                    max_seconds.map(|seconds| Duration::from_secs(seconds)),
                                );
                                finished.store(true, Ordering::Relaxed);
                                let _ = sampler.join();
                                result.map(|summary| live.report(summary.bytes))
                            }
                        },
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
    use xdm_ipc::QueueSummary;
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
    fn media_work_injects_matching_login_and_keeps_explicit_headers() {
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
        let job = |request_id: &str, headers: Vec<(String, String)>| {
            service
                .command(Command::SubmitMediaJob {
                    request_id: request_id.into(),
                    url: "http://127.0.0.1:1/master.m3u8".into(),
                    output: format!("/nonexistent-xdm-test/{request_id}.mkv"),
                    headers,
                    mode: MediaMode::Live {
                        video: 0,
                        audio: Some(1),
                        max_seconds: Some(5),
                    },
                    priority: 1,
                })
                .unwrap()
                .task_id
                .unwrap()
        };
        // A saved login supplies the Basic credentials of the manifest origin.
        let id = job("media-login", Vec::new());
        let work = service.next_work().unwrap();
        assert_eq!(work.id, id);
        let WorkKind::Media { mode, headers } = work.kind else { unreachable!() };
        assert!(matches!(mode, MediaMode::Live { .. }));
        assert!(work.media.is_some());
        assert_eq!(
            headers,
            vec![("Authorization".into(), format!("Basic {}", credentials::basic_token("alice", "s3cret")))]
        );
        // A submitter-provided Authorization header is never overridden.
        let explicit = job(
            "media-login-explicit",
            vec![("Authorization".into(), "Bearer supplied".into())],
        );
        let work = service.next_work().unwrap();
        assert_eq!(work.id, explicit);
        let WorkKind::Media { headers, .. } = work.kind else { unreachable!() };
        assert_eq!(headers, vec![("Authorization".into(), "Bearer supplied".into())]);
        // With a disabled login the task keeps its clean headers.
        service
            .command(Command::LoginSave {
                pattern: "http://127.0.0.1:1/".into(),
                username: "alice".into(),
                password: "s3cret".into(),
                enabled: false,
            })
            .unwrap();
        let plain = job("media-login-plain", Vec::new());
        let work = service.next_work().unwrap();
        assert_eq!(work.id, plain);
        let WorkKind::Media { headers, .. } = work.kind else { unreachable!() };
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
}
