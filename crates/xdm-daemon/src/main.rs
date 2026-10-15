/// Test paths: a rooted path without a drive letter is not absolute on Windows.
#[cfg(all(test, windows))]
macro_rules! abs { ($s:literal) => { concat!("C:", $s) }; }
#[cfg(all(test, not(windows)))]
macro_rules! abs { ($s:literal) => { $s }; }

mod bandwidth;
mod dbsafety;
mod lock;
mod shutdown;
mod categories;
mod credentials;
mod grabber;
mod sync;
mod logins;
mod media;
mod os_actions;
mod queues;
mod schedule;
#[cfg(test)]
pub(crate) mod test_helpers;
mod task_files;
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
use xdm_http::{Options, Progress, Proxy, ResumePolicy, ResponseMetadata, TransferControl, download_ex};
use schedule::{Clock, SystemClock};
use xdm_ipc::{
    Command, MediaMode, Paths, Reply, Request, TaskSummary, read_frame,
};
#[cfg(test)]
use xdm_ipc::MediaAudioFormat;

pub(crate) type Error = Box<dyn std::error::Error + Send + Sync>;
pub(crate) type Result<T> = std::result::Result<T, Error>;

/// Submit policies carried by the extended `SubmitEx` variant; the legacy
/// `Submit`/`SubmitRequest` paths submit with every default (all false),
/// keeping their old semantics.
#[derive(Debug, Clone, Copy, Default)]
struct SubmitFlags {
    name_from_server: bool,
    ignore_modification_date: bool,
    /// Only the extended variant runs the duplicate-URL check.
    check_duplicates: bool,
    allow_duplicate: bool,
    /// N41 opt-out; the default keeps the server's Last-Modified on the file.
    skip_server_time: bool,
}

pub(crate) struct Inner {
    pub(crate) db: Connection,
    pub(crate) active: HashMap<i64, Active>,
    pub(crate) max_concurrent: usize,
    pub(crate) queue_running: bool,
    pub(crate) queue_start_at: Option<i64>,
    pub(crate) queue_stop_at: Option<i64>,
    pub(crate) queue_interrupted: HashSet<i64>,
    /// Completed tasks whose file is being moved; removal, restart,
    /// redownload and further moves are refused meanwhile.
    pub(crate) moving: HashSet<i64>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) adapter: Arc<dyn CompletionAdapter>,
    /// Destructive actions counting down, keyed by queue.
    pub(crate) pending: HashMap<i64, PendingCompletion>,
    /// Global proxy route for HTTP tasks, stored as canonical JSON.
    pub(crate) proxy: String,
    /// Global N31 stall timeout in seconds applied to every HTTP task.
    pub(crate) stall_timeout: u64,
    /// N38 setting: absolute directory for new tasks' `.part`/`.part.meta`.
    pub(crate) temp_dir: Option<String>,
    /// Process-wide rate caps, host connection budgets and the usage counter.
    pub(crate) governor: Arc<xdm_http::Governor>,
    pub(crate) quota: bandwidth::Quota,
    /// Saved site logins for HTTP authentication (N10).
    pub(crate) credentials: credentials::Store,
    /// Credentials typed for one task (`TaskProvideLogin` without `--remember`):
    /// kept in memory only, never persisted, dropped when the task completes.
    pub(crate) task_logins: HashMap<i64, xdm_http::Login>,
    /// Live crawls keyed by their site-grab request ID.
    pub(crate) grabs: HashMap<String, GrabHandle>,
    /// Crawls waiting for the worker loop, which owns the service handle.
    pub(crate) pending_grabs: Vec<GrabJob>,
    /// Synchronization checks requested but not yet started (Q10).
    pub(crate) pending_syncs: Vec<i64>,
    /// Queues whose synchronization check is in flight.
    pub(crate) sync_running: HashSet<i64>,
}

const PROXY_SCHEMES: [&str; 5] = ["http://", "https://", "socks4://", "socks5://", "socks5h://"];

/// Methods are strict HTTP tokens here; the engine accepts a superset.
fn valid_method(method: &str) -> bool {
    !method.is_empty() && method.len() <= 16 && method.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// HTTP statuses that retrying cannot fix.
fn is_permanent_error(message: &str) -> bool {
    ["HTTP 401", "HTTP 403", "HTTP 404", "HTTP 410", xdm_http::HTML_INSTEAD_OF_FILE]
        .iter()
        .any(|status| message.contains(status))
}

/// Stall timeout bounds shared by the setting, the CLI flag and the engine.
const STALL_TIMEOUT_RANGE: std::ops::RangeInclusive<u64> = 1..=600;

/// Failures that leave resumable partial data which can never be completed by
/// resuming: the safe way out is a full restart from byte zero.
fn restart_reason(error: &(dyn std::error::Error + Send + Sync + 'static)) -> Option<&'static str> {
    if xdm_http::range_unreliable(error).is_some() {
        return Some("server stopped honoring byte ranges");
    }
    let text = error.to_string();
    if text.contains("partial download identity changed or checkpoint invalid; use --restart") {
        return Some("partial download identity changed or checkpoint invalid");
    }
    if text.contains("remote validators changed") {
        return Some("remote validators changed during range transfer");
    }
    None
}

/// The engine's partial-file and checkpoint paths, mirrored here so
/// `TaskRestart` deletes exactly the files one task owns.
/// Every partial file one task may own: beside its destination and, when the task
/// assembles in a temporary directory (N38), the pair inside that directory.
pub(crate) fn owned_partial_paths(output: &Path, temp: Option<&str>) -> Vec<PathBuf> {
    let mut paths = http_part_paths(output).to_vec();
    paths.push(PathBuf::from(format!("{}.part.ftp", output.display())));
    if let Some(temp) = temp.filter(|temp| !temp.is_empty()) {
        let temp = std::fs::canonicalize(temp).unwrap_or_else(|_| PathBuf::from(temp));
        let (part, meta) = xdm_http::partial_paths(output, Some(&temp));
        paths.extend([part, meta]);
        // FTP partials share the HTTP part name; the checkpoint differs.
        paths.push(xdm_ftp::partial_paths(output, Some(&temp)).1);
    }
    paths
}

fn http_part_paths(output: &Path) -> [PathBuf; 2] {
    [
        PathBuf::from(format!("{}.part", output.display())),
        PathBuf::from(format!("{}.part.meta", output.display())),
    ]
}

/// Variant of `name` with a numeric suffix inserted before the extension:
/// `dup.txt` -> `dup (2).txt` (N14 duplicate-target rule).
fn varied_name(name: &str, suffix: u32) -> String {
    match name.rfind('.').filter(|&at| at > 0) {
        Some(at) => format!("{} ({}){}", &name[..at], suffix, &name[at..]),
        None => format!("{name} ({suffix})"),
    }
}

/// Resolves the server-chosen destination for an N13/N14 task: the name comes
/// from `xdm_http::server_filename` and stays inside the provisional file's
/// directory; occupied names (another task or an existing file) receive a
/// " (2)"-style suffix instead, and the chosen path is recorded in the row.
fn choose_server_destination(
    service: &Service,
    id: i64,
    provisional: &str,
    metadata: &ResponseMetadata,
) -> Result<PathBuf> {
    let output = Path::new(provisional);
    let provisional_name = output.file_name().and_then(|name| name.to_str()).unwrap_or("");
    let Some(name) = xdm_http::reconcile_server_name(metadata, provisional_name) else {
        return Ok(output.to_path_buf());
    };
    let directory = output.parent().unwrap_or(Path::new("/")).to_path_buf();
    let inner = service
        .inner
        .lock()
        .map_err(|_| io::Error::other("task lock poisoned"))?;
    let temp: Option<String> = inner
        .db
        .query_row("SELECT temp_dir FROM tasks WHERE id=?1", [id], |row| row.get(0))
        .optional()?
        .flatten();
    let mut suffix = 1u32;
    let mut chosen = name.clone();
    loop {
        let candidate = directory.join(&chosen);
        let output_text = candidate.to_string_lossy();
        // Someone else's partial files (another task, a browser) also occupy a name.
        let taken = std::fs::symlink_metadata(&candidate).is_ok()
            || owned_partial_paths(&candidate, temp.as_deref())
                .iter()
                .any(|path| std::fs::symlink_metadata(path).is_ok())
            || inner
                .db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM tasks WHERE output=?1)",
                    [output_text.as_ref()],
                    |row| row.get::<_, i64>(0),
                )? != 0;
        if !taken {
            break;
        }
        suffix += 1;
        chosen = varied_name(&name, suffix);
    }
    if chosen == provisional_name {
        return Ok(output.to_path_buf());
    }
    let destination = directory.join(&chosen);
    inner.db.execute(
        "UPDATE tasks SET output=?1,updated_at=CURRENT_TIMESTAMP WHERE id=?2",
        params![destination.to_string_lossy(), id],
    )?;
    Ok(destination)
}

pub(crate) struct Active {
    pub(crate) queue_id: i64,
    pub(crate) stop: Arc<AtomicBool>,
    /// Set only when the daemon itself asked the worker to stop. The engine
    /// also flips `stop` when it aborts on a failure, which must not read as
    /// a user pause in the recorded outcome.
    pub(crate) stop_requested: Arc<AtomicBool>,
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
fn ftp_checkpoint_progress(output: &Path, temp: Option<&Path>) -> Option<(u64, Option<u64>)> {
    let (part, encoded) = xdm_ftp::partial_paths(output, temp);
    let state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&encoded).ok()?).ok()?;
    let total = state.get("size").and_then(|value| value.as_u64());
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
    /// Wakes the synchronization scheduler when intervals or queues change.
    pub(crate) sync_wake: Arc<(Mutex<bool>, Condvar)>,
}

#[derive(Debug)]
struct Work {
    id: i64,
    url: String,
    output: PathBuf,
    kind: WorkKind,
    proxy: Proxy,
    /// Site login answered to 401 challenges by the HTTP engine (Basic/Digest, N11).
    login: Option<xdm_http::Login>,
    /// Credentials of the manual proxy, from the credential store (N23).
    proxy_login: Option<xdm_http::Login>,
    /// N31 idle cap handed to the HTTP engine for this task.
    stall_timeout: Duration,
    stop: Arc<AtomicBool>,
    /// Whether the stop flag was set by the daemon rather than an engine abort.
    stop_requested: Arc<AtomicBool>,
    /// N13 modification-date policy for this HTTP task.
    policy: ResumePolicy,
    /// N14: the final filename comes from the server response.
    name_from_server: bool,
    /// N38: directory holding this task's partial files, when separate.
    temp_dir: Option<PathBuf>,
    /// N41: set the completed file's modification time from the server.
    set_file_time: bool,
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

/// Task-details notice of a transfer that ended at EOF without a declared length.
const UNVERIFIED_NOTICE: &str = "The server did not declare a file size, so the download ended at the end of the stream and the file might be incomplete.";

fn task_summaries(inner: &Inner, task_id: Option<i64>) -> Result<Vec<TaskSummary>> {
    query_summaries(
        inner,
        "WHERE id BETWEEN COALESCE(?1,1) AND COALESCE(?1,9223372036854775807) ORDER BY id DESC LIMIT 1000",
        rusqlite::params_from_iter([task_id]),
    )
}

/// Task rows for a `WHERE ... ORDER BY ... LIMIT` tail; shared by `List`,
/// `GetTask` and `ListPage`.
pub(crate) fn query_summaries<P: rusqlite::Params>(
    inner: &Inner,
    tail: &str,
    params: P,
) -> Result<Vec<TaskSummary>> {
    let mut stmt = inner.db.prepare(&format!("SELECT id,url,output,status,priority,error,kind,category,rate_limit,connections,created_at,updated_at,media_video,media_audio,queue_id,history_queue_id,retry_count,retry_at,position,auth_realm,restart_reason,size_unverified,temp_dir,error_kind,auth_proxy FROM tasks {tail}"))?;
    let tasks = stmt
        .query_map(params, |row| {
            let output: String = row.get(2)?;
            let status: String = row.get(3)?;
            let id: i64 = row.get(0)?;
            let temp_dir: Option<String> = row.get(22)?;
            let size_unverified = row.get::<_, i64>(21)? != 0;
            let error: Option<String> = row.get(5)?;
            // A kind only describes a live error text.
            let error_kind: Option<String> = row.get::<_, Option<String>>(23)?.filter(|_| error.is_some());
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
                xdm_http::checkpoint_progress_in(
                    std::path::Path::new(&output),
                    temp_dir.as_deref().filter(|dir| !dir.is_empty()).map(std::path::Path::new),
                )
                    .map(|(done, total)| (done, Some(total)))
                    .or_else(|| {
                        ftp_checkpoint_progress(
                            std::path::Path::new(&output),
                            temp_dir.as_deref().filter(|dir| !dir.is_empty()).map(std::path::Path::new),
                        )
                    })
            };
            Ok(TaskSummary {
                id,
                url: row.get(1)?,
                output,
                status,
                priority: row.get(4)?,
                error,
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
                restart_reason: row.get(20)?,
                size_unverified,
                notice: size_unverified.then(|| UNVERIFIED_NOTICE.to_owned()).or_else(|| {
                    error_kind.as_deref().and_then(failure_label).map(|label| format!("Failure class: {label}"))
                }),
                error_kind,
                auth_proxy: row.get::<_, i64>(24)? != 0,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(tasks)
}

/// Human label of a stored `error_kind` for task details.
fn failure_label(kind: &str) -> Option<&'static str> {
    use xdm_http::ErrorKind::*;
    [Dns, ConnectRefused, ConnectTimeout, Tls, ReadTimeout, HttpStatus, Disk, Auth, ProxyConfig, Network, Other]
        .into_iter()
        .find(|candidate| candidate.as_str() == kind)
        .map(|candidate| candidate.label())
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

/// Normalizes a URL for duplicate comparison (IDM): scheme and host
/// lowercased — which `Url` parsing itself does — and no fragment. The
/// port, path case and query bytes stay as submitted.
fn normalize_url(raw: &str) -> Option<String> {
    let no_fragment = raw.split('#').next().unwrap_or(raw);
    Url::parse(no_fragment).ok().map(|parsed| parsed.as_str().to_owned())
}

/// The non-removed task owning the same normalized URL, if any.
fn duplicate_task(db: &Connection, url: &str) -> Result<Option<i64>> {
    let Some(target) = normalize_url(url) else {
        return Ok(None);
    };
    let mut statement = db.prepare("SELECT id,url FROM tasks LIMIT 10000")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let candidate = row.get::<_, String>(1)?;
        if normalize_url(&candidate).as_deref() == Some(target.as_str()) {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

/// Splits a batch of URLs into the indexes to keep and the duplicates to skip.
/// A URL is a duplicate of an existing task or of an earlier batch entry.
fn split_duplicates(
    db: &Connection,
    urls: &[&str],
    exclude_task: Option<i64>,
) -> Result<(Vec<usize>, Vec<xdm_ipc::SkippedDuplicate>)> {
    let mut known: HashMap<String, (Option<i64>, Option<usize>)> = HashMap::new();
    let mut statement = db.prepare("SELECT id,url FROM tasks ORDER BY id DESC LIMIT 10000")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        if Some(id) == exclude_task {
            continue;
        }
        if let Some(normalized) = normalize_url(&row.get::<_, String>(1)?) {
            known.insert(normalized, (Some(id), None));
        }
    }
    let mut kept = Vec::new();
    let mut skipped = Vec::new();
    for (index, url) in urls.iter().enumerate() {
        let normalized = normalize_url(url);
        match normalized.as_ref().and_then(|key| known.get(key)) {
            Some(&(duplicate_of, duplicate_of_index)) => skipped.push(xdm_ipc::SkippedDuplicate {
                index,
                url: (*url).to_string(),
                duplicate_of,
                duplicate_of_index,
            }),
            None => {
                if let Some(key) = normalized {
                    known.insert(key, (None, Some(index)));
                }
                kept.push(index);
            }
        }
    }
    Ok((kept, skipped))
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
        flags: SubmitFlags,
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
            // Duplicate detection (IDM prompt): the same normalized URL in a
            // non-removed task is refused with the task ID; an explicit
            // allow_duplicate repeats the download anyway. Only the extended
            // variant opts in, so legacy submissions keep their semantics.
            if flags.check_duplicates
                && !flags.allow_duplicate
                && let Some(other) = duplicate_task(&inner.db, &url)?
            {
                let mut reply = Reply::error(
                    "a task for this download already exists; pass allow_duplicate to download again",
                );
                reply.duplicate_of = Some(other);
                return Ok(reply);
            }
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
            let inserted = inner.db.execute("INSERT OR IGNORE INTO tasks(id,request_id,url,output,headers_json,priority,connections,status,kind,category,method,body,name_from_server,ignore_modified,preserve_time) VALUES (?9,?1,?2,?3,?4,?5,?6,'queued',?8,?7,?10,?11,?12,?13,?14)", params![request_id,url,output,headers_json,priority,connections as i64,category,kind,id,method,body,flags.name_from_server,flags.ignore_modification_date,!flags.skip_server_time])?;
            if inserted == 0 {
                return Ok(Reply::error("destination is already assigned to a task"));
            }
            let id = inner.db.last_insert_rowid();
            drop(inner);
            self.wake.notify_one();
            Ok(Reply::accepted(id))
        }
    }

    fn redownload(
        &self,
        task_id: i64,
        request_id: String,
        output: String,
        allow_duplicate: bool,
    ) -> Result<Reply> {

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
        if inner.moving.contains(&task_id) {
            return Ok(Reply::error("task file is being moved"));
        }
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
        if !allow_duplicate {
            let url: String = inner.db.query_row("SELECT url FROM tasks WHERE id=?1", [task_id], |row| row.get(0))?;
            let (_, skipped) = split_duplicates(&inner.db, &[url.as_str()], Some(task_id))?;
            if let Some(found) = skipped.first() {
                let mut reply = Reply::error(
                    "another task already downloads this URL; pass allow_duplicate to download again",
                );
                reply.duplicate_of = found.duplicate_of;
                return Ok(reply);
            }
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
        dbsafety::prepare(&db, database)?;
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.pragma_update(None, "synchronous", "NORMAL")?;
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
            ("restart_reason", "TEXT"),
            ("name_from_server", "INTEGER NOT NULL DEFAULT 0"),
            ("ignore_modified", "INTEGER NOT NULL DEFAULT 0"),
            ("sync_etag", "TEXT"),
            ("sync_lastmod", "TEXT"),
            ("sync_len", "INTEGER"),
            ("preserve_time", "INTEGER NOT NULL DEFAULT 1"),
            ("temp_dir", "TEXT"),
            ("size_unverified", "INTEGER NOT NULL DEFAULT 0"),
            ("last_try_at", "INTEGER"),
            ("size_done", "INTEGER"),
            ("size_total", "INTEGER"),
            ("error_kind", "TEXT"),
            ("auth_proxy", "INTEGER NOT NULL DEFAULT 0"),
        ] {
            if !columns.iter().any(|column| column == name) {
                db.execute_batch(&format!("ALTER TABLE tasks ADD COLUMN {name} {definition}"))?;
            }
        }
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS removed_tasks (task_id INTEGER PRIMARY KEY, request_id TEXT NOT NULL UNIQUE);
             CREATE TABLE IF NOT EXISTS imports (request_id TEXT PRIMARY KEY, payload_json TEXT NOT NULL, task_ids_json TEXT NOT NULL, skipped_json TEXT NOT NULL DEFAULT '[]');
             CREATE TABLE IF NOT EXISTS grab_projects (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE COLLATE NOCASE, project_json TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS categories (name TEXT PRIMARY KEY COLLATE NOCASE, rule_json TEXT NOT NULL);",
        )?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS queues (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE COLLATE NOCASE, running INTEGER NOT NULL, max_concurrent INTEGER NOT NULL);
            INSERT OR IGNORE INTO queues(id,name,running,max_concurrent) VALUES (1,'Main',1,16);
            CREATE INDEX IF NOT EXISTS tasks_by_queue ON tasks(queue_id,status,priority DESC,id ASC);
            CREATE INDEX IF NOT EXISTS tasks_by_status_id ON tasks(status,id);
            CREATE INDEX IF NOT EXISTS tasks_by_category ON tasks(category,id);")?;
        let has_skipped = db
            .prepare("PRAGMA table_info(imports)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .iter()
            .any(|column| column == "skipped_json");
        if !has_skipped {
            db.execute_batch("ALTER TABLE imports ADD COLUMN skipped_json TEXT NOT NULL DEFAULT '[]'")?;
        }
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
            ("sync", "INTEGER NOT NULL DEFAULT 0"),
            ("sync_interval", "INTEGER"),
            ("sync_last", "INTEGER NOT NULL DEFAULT 0"),
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
             UPDATE tasks SET history_queue_id=queue_id,queue_id=0 WHERE status='complete' AND queue_id<>0 AND NOT COALESCE((SELECT sync FROM queues WHERE id=tasks.queue_id),0);",
        )?;
        // A countdown belongs to the daemon instance that armed it: after a
        // restart its destructive action is suppressed rather than executed.
        db.execute(
            "UPDATE queue_events SET action_status='suppressed' WHERE action_status='pending'",
            [],
        )?;
        dbsafety::finish(&db, database)?;
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
        let stall_timeout = get_setting(&db, "stall_timeout_seconds")?
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| STALL_TIMEOUT_RANGE.contains(value))
            .unwrap_or(xdm_http::DEFAULT_STALL_TIMEOUT.as_secs());
        let temp_dir = get_setting(&db, "temp_dir")?.filter(|dir| Path::new(dir).is_absolute());
        // Site logins live beside the task database; in-memory test databases
        // keep the store in memory only.
        let credentials = credentials::Store::load(
            (database.as_os_str() != ":memory:")
                .then(|| database.parent().unwrap_or(Path::new(".")).join("credentials.json")),
        )?;
        let mut credentials = credentials;
        let mut proxy = proxy;
        // Older databases may hold `user:password@` inside the proxy URL; move it
        // to the credential store and rewrite the setting without it.
        if let Proxy::Manual { http, https, .. } = &mut proxy {
            let mut moved = None;
            let mut changed = false;
            for slot in [http, https] {
                let Some(url) = slot.as_mut() else { continue };
                match credentials::split_proxy_userinfo(url) {
                    Ok((clean, found)) => {
                        if found.is_some() {
                            *url = clean;
                            moved = found;
                            changed = true;
                        }
                    }
                    Err(_) => {
                        // Cannot separate the password safely: drop the URL instead of
                        // keeping a secret in settings.
                        eprintln!("warning: removed a stored proxy URL whose credentials could not be parsed; set the proxy again and use `daemon proxy-login`");
                        *slot = None;
                        changed = true;
                    }
                }
            }
            if let Some((user, password)) = moved {
                if credentials.set_proxy_login(&user, &password).is_err() {
                    changed = false;
                }
            }
            if changed {
                set_setting(&db, "proxy", &serde_json::to_string(&proxy)?)?;
            }
        }
        db.execute(
            "UPDATE tasks SET status='queued', error=NULL WHERE status IN ('running','pausing')",
            [],
        )?;
        let governor = xdm_http::Governor::new();
        let quota = bandwidth::load(&db, &governor);
        Ok(Arc::new(Self {
            inner: Arc::new(Mutex::new(Inner {
                db,
                active: HashMap::new(),
                max_concurrent,
                queue_running,
                queue_start_at,
                queue_stop_at,
                queue_interrupted: HashSet::new(),
                moving: HashSet::new(),
                clock,
                adapter,
                pending: HashMap::new(),
                proxy: serde_json::to_string(&proxy).unwrap_or_default(),
                stall_timeout,
                temp_dir,
                governor,
                quota,
                credentials,
                task_logins: HashMap::new(),
                grabs: HashMap::new(),
                pending_grabs: Vec::new(),
                pending_syncs: Vec::new(),
                sync_running: HashSet::new(),
            })),
            wake: Arc::new(Condvar::new()),
            sync_wake: Arc::new((Mutex::new(true), Condvar::new())),
        }))
    }

    fn command(&self, command: Command) -> Result<Reply> {
        let tick = |service: &Self| -> Result<()> {
            let mut inner = service
                .inner
                .lock()
                .map_err(|_| io::Error::other("task lock poisoned"))?;
            queue_tick(&mut inner)?;
            bandwidth::quota_tick(&mut inner)
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
            command @ (Command::SetGlobalLimit { .. }
            | Command::SetHostLimit { .. }
            | Command::SetHostConnections { .. }
            | Command::SetQuota { .. }
            | Command::BandwidthStatus) => self.bandwidth_command(command),
            Command::QueueCreate { name, limit } => self.queue_create(name, limit, false),
            Command::QueueCreateSync { name, limit } => self.queue_create(name, limit, true),
            Command::QueueSetSync { queue_id, interval_seconds } => {
                self.queue_set_sync(queue_id, interval_seconds)
            }
            Command::QueueCheck { queue_id } => self.queue_check(queue_id),
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
            } => self.import_list(request_id, directory, entries, enqueue, false),
            Command::ImportListEx {
                request_id,
                directory,
                entries,
                enqueue,
                allow_duplicate,
            } => self.import_list(request_id, directory, entries, enqueue, allow_duplicate),
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
                    SubmitFlags::default(),
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
                    SubmitFlags::default(),
                )
            }
            Command::SubmitEx {
                request_id,
                url,
                output,
                headers,
                method,
                body,
                priority,
                connections,
                name_from_server,
                ignore_modification_date,
                allow_duplicate,
                preserve_server_time,
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
                    SubmitFlags {
                        name_from_server,
                        ignore_modification_date,
                        check_duplicates: true,
                        allow_duplicate,
                        skip_server_time: !preserve_server_time,
                    },
                )
            }
            Command::SetProxy { proxy } => {
                let mut parsed: Proxy =
                    serde_json::from_value(serde_json::to_value(&proxy)?).map_err(|e| e.to_string())?;
                // Credentials in a proxy URL move to the credential store (N23).
                let mut extracted = None;
                if let Proxy::Manual { http, https, .. } = &mut parsed {
                    for url in http.iter_mut().chain(https.iter_mut()) {
                        match credentials::split_proxy_userinfo(url) {
                            Ok((clean, found)) => {
                                if found.is_some() {
                                    *url = clean;
                                    extracted = found;
                                }
                            }
                            // Never echo the URL: it holds a password.
                            Err(message) => return Ok(Reply::error(message)),
                        }
                    }
                }
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
                if let Some((user, password)) = extracted {
                    if let Err(error) = inner.credentials.set_proxy_login(&user, &password) {
                        return Ok(Reply::error(format!("proxy credentials rejected: {error}")));
                    }
                }
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
                reply.proxy = serde_json::from_str::<Proxy>(&inner.proxy).ok().map(|mut mode| {
                    // Defense in depth: a URL never leaves the daemon with user info.
                    if let Proxy::Manual { http, https, .. } = &mut mode {
                        for url in http.iter_mut().chain(https.iter_mut()) {
                            *url = credentials::redact_proxy_url(url);
                        }
                    }
                    serde_json::to_value(&mode).unwrap_or_default()
                });
                let mode: Proxy = serde_json::from_str(&inner.proxy).unwrap_or_default();
                reply.proxy_note = Some(match mode {
                    Proxy::System => format!("system: {}", xdm_http::system_proxy_status()),
                    Proxy::Direct => "direct connections".into(),
                    Proxy::Manual { .. } => match inner.credentials.proxy_login() {
                        Some(stored) => format!("manual proxy with stored credentials for {}", stored.username),
                        None => "manual proxy without credentials".into(),
                    },
                });
                Ok(reply)
            }
            Command::ProxyLogin { username, password } => self.proxy_login(username, password),
            Command::LoginSetInsecure { id, allow } => {
                let mut inner = self
                    .inner
                    .lock()
                    .map_err(|_| io::Error::other("task lock poisoned"))?;
                if !inner.credentials.set_allow_insecure(id, allow)? {
                    return Ok(Reply::error("login not found"));
                }
                let mut reply = Reply::accepted(id);
                reply.logins = inner.credentials.summaries();
                Ok(reply)
            }
            Command::ProxyLoginClear => {
                let mut inner = self
                    .inner
                    .lock()
                    .map_err(|_| io::Error::other("task lock poisoned"))?;
                if !inner.credentials.clear_proxy_login()? {
                    return Ok(Reply::error("no proxy credentials are stored"));
                }
                Ok(Reply::accepted(0))
            }
            Command::SetStallTimeout { seconds } => {
                if !STALL_TIMEOUT_RANGE.contains(&seconds) {
                    return Ok(Reply::error(
                        "stall timeout must be between 1 and 600 seconds",
                    ));
                }
                let mut inner = self
                    .inner
                    .lock()
                    .map_err(|_| io::Error::other("task lock poisoned"))?;
                set_setting(&inner.db, "stall_timeout_seconds", &seconds.to_string())?;
                inner.stall_timeout = seconds;
                drop(inner);
                self.wake.notify_one();
                let mut reply = Reply::accepted(0);
                reply.stall_timeout_seconds = Some(seconds);
                Ok(reply)
            }
            Command::SetTempDir { path } => {
                let path = path.filter(|path| !path.is_empty());
                if let Some(path) = &path {
                    let directory = Path::new(path);
                    if !directory.is_absolute()
                        || directory
                            .components()
                            .any(|part| matches!(part, std::path::Component::ParentDir))
                    {
                        return Ok(Reply::error(
                            "the temporary directory must be an absolute path without ..",
                        ));
                    }
                    if let Err(error) = std::fs::create_dir_all(directory) {
                        return Ok(Reply::error(format!("cannot use {path}: {error}")));
                    }
                    if !directory.is_dir() {
                        return Ok(Reply::error(format!("{path} is not a directory")));
                    }
                }
                // Stored canonical so ownership checks and scans compare equal paths.
                let path = match path {
                    Some(path) => match std::fs::canonicalize(&path) {
                        Ok(canonical) => Some(canonical.to_string_lossy().into_owned()),
                        Err(error) => return Ok(Reply::error(format!("cannot use {path}: {error}"))),
                    },
                    None => None,
                };
                let mut inner = self
                    .inner
                    .lock()
                    .map_err(|_| io::Error::other("task lock poisoned"))?;
                set_setting(&inner.db, "temp_dir", path.as_deref().unwrap_or(""))?;
                inner.temp_dir = path.clone();
                let mut reply = Reply::accepted(0);
                reply.temp_dir = path;
                Ok(reply)
            }
            Command::GetTempDir => {
                let inner = self
                    .inner
                    .lock()
                    .map_err(|_| io::Error::other("task lock poisoned"))?;
                let mut reply = Reply::accepted(0);
                reply.temp_dir = inner.temp_dir.clone();
                Ok(reply)
            }
            Command::GetStallTimeout => {
                let inner = self
                    .inner
                    .lock()
                    .map_err(|_| io::Error::other("task lock poisoned"))?;
                let mut reply = Reply::accepted(0);
                reply.stall_timeout_seconds = Some(inner.stall_timeout);
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
            Command::Remove { task_id } => self.remove_task(task_id, false),
            Command::TaskRemoveEx {
                task_id,
                delete_partial,
            } => self.remove_task(task_id, delete_partial),
            Command::ListPage {
                offset,
                limit,
                sort,
                filter,
            } => self.list_page(offset, limit, sort, filter),
            Command::TaskMoveFile { task_id, new_path } => self.move_task_file(task_id, new_path),
            Command::PartialScan { directory } => Ok(match self.scan_partials(&directory)? {
                Ok((found, truncated)) => {
                    let mut reply = Reply::list(Vec::new());
                    reply.partials = found;
                    reply.truncated = truncated;
                    reply.daemon_version = Some(env!("CARGO_PKG_VERSION").to_owned());
                    reply
                }
                Err(message) => Reply::error(message),
            }),
            Command::PartialDelete { directory, paths } => self.delete_partials(&directory, paths),
            Command::Redownload {
                task_id,
                request_id,
                output,
            } => self.redownload(task_id, request_id, output, false),
            Command::RedownloadEx {
                task_id,
                request_id,
                output,
                allow_duplicate,
            } => self.redownload(task_id, request_id, output, allow_duplicate),
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
                    active.stop_requested.store(true, Ordering::Relaxed);
                }
                let updated = inner.db.execute("UPDATE tasks SET status=CASE WHEN status='running' THEN 'pausing' ELSE 'paused' END, updated_at=CURRENT_TIMESTAMP WHERE id=?1 AND status IN ('queued','running','quota_wait')", [task_id])?;
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
                    "UPDATE tasks SET priority=?1,updated_at=CURRENT_TIMESTAMP WHERE id=?2 AND status IN ('queued','paused','failed','quota_wait')",
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
                    "UPDATE tasks SET rate_limit=?1,updated_at=CURRENT_TIMESTAMP WHERE id=?2 AND kind='http' AND status IN ('queued','running','paused','failed','quota_wait')",
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
            Command::TaskRestart { task_id } => self.task_restart(task_id),
            Command::Hello { .. } => Ok(Reply::error("handshake is answered by the connection handler")),
        }
    }

    /// Restarts one `restart_required` task from byte zero: only its own
    /// `.part`/`.part.meta` (derived from the task's output path) are deleted,
    /// then the task re-enters the queue with a fresh retry budget.
    fn task_restart(&self, task_id: i64) -> Result<Reply> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        if inner.moving.contains(&task_id) {
            return Ok(Reply::error("task file is being moved"));
        }
        if inner.active.contains_key(&task_id) {
            return Ok(Reply::error(
                "pause the task and wait for it to stop before restarting it",
            ));
        }
        let output: Option<(String, Option<String>)> = inner
            .db
            .query_row(
                "SELECT output,temp_dir FROM tasks WHERE id=?1 AND status='restart_required'",
                [task_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()?;
        let Some((output, temp)) = output else {
            return Ok(Reply::error("task is not waiting for a restart"));
        };
        for path in owned_partial_paths(Path::new(&output), temp.as_deref()) {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Ok(Reply::error(format!(
                        "could not remove {}: {e}",
                        path.display()
                    )))
                }
            }
        }
        inner.db.execute(
            "UPDATE tasks SET status='queued',error=NULL,restart_reason=NULL,retry_count=0,retry_at=NULL,updated_at=CURRENT_TIMESTAMP WHERE id=?1",
            [task_id],
        )?;
        drop(inner);
        self.wake.notify_one();
        Ok(Reply::accepted(task_id))
    }

    fn next_work(&self) -> Result<Work> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        loop {
            if shutdown::requested() {
                // No new transfers once shutdown began.
                inner = self
                    .wake
                    .wait_timeout(inner, Duration::from_secs(1))
                    .map_err(|_| io::Error::other("task lock poisoned"))?
                    .0;
                continue;
            }
            queue_tick(&mut inner)?;
            bandwidth::quota_tick(&mut inner)?;
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
                    inner.db.query_row("SELECT id,url,output,headers_json,connections,kind,media_video,media_audio,media_mode_json,rate_limit,queue_id,method,body,name_from_server,ignore_modified,preserve_time,temp_dir FROM tasks WHERE id=?1", [id], |row| Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,i64>(4)?,row.get::<_,String>(5)?,row.get::<_,Option<i64>>(6)?,row.get::<_,Option<i64>>(7)?,row.get::<_,Option<String>>(8)?,row.get::<_,Option<i64>>(9)?,row.get::<_,i64>(10)?,row.get::<_,String>(11)?,row.get::<_,Option<Vec<u8>>>(12)?,row.get::<_,i64>(13)?,row.get::<_,i64>(14)?,row.get::<_,i64>(15)?,row.get::<_,Option<String>>(16)?))).optional()?
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
                    name_from_server,
                    ignore_modified,
                    preserve_time,
                    stored_temp,
                )) = selected
                {
                    if kind == "http" && inner.governor.exceeded() {
                        inner.db.execute("UPDATE tasks SET status='quota_wait',updated_at=CURRENT_TIMESTAMP WHERE id=?1", [id])?;
                        continue;
                    }
                    let (control, ftp) = if kind == "http" {
                        let control = Arc::new(TransferControl::new(
                            rate_limit.map(|rate| rate as u64),
                        )?);
                        control.attach(inner.governor.clone(), &url);
                        (Some(control), None)
                    } else if kind == "ftp" {
                        let ftp = Arc::new(FtpLive::default());
                        (None, Some(ftp))
                    } else {
                        (None, None)
                    };
                    let headers: Vec<(String, String)> =
                        serde_json::from_str(&headers_json)?;
                    // A matching site login is handed to the engine (never baked into a
                    // header): HTTP tasks answer the server's own challenge, media
                    // manifests attach it to requests for the login's origin only, and
                    // nothing reaches ffmpeg's argv. A submitter-supplied Authorization
                    // header always wins.
                    let mut found: Option<xdm_http::Login> = None;
                    if (kind == "http" || kind == "media")
                        && !headers
                            .iter()
                            .any(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                        && let Ok(parsed) = Url::parse(&url)
                        && let Some(login) = inner.credentials.find(&parsed)
                    {
                        found = Some(xdm_http::Login {
                            username: login.username.clone(),
                            password: login.password.clone(),
                            allow_insecure: login.allow_insecure,
                        });
                    }
                    if kind == "http"
                        && let Some(typed) = inner.task_logins.get(&id)
                    {
                        found = Some(typed.clone());
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
                    // N38: a task keeps the partial location it started with; only a
                    // fresh HTTP task without partials beside its destination adopts
                    // the current setting.
                    let temp_dir = match stored_temp {
                        Some(dir) => (!dir.is_empty()).then(|| PathBuf::from(dir)),
                        None => {
                            let beside = http_part_paths(Path::new(&output)).iter().any(|path| path.exists())
                                || Path::new(&format!("{output}.part.ftp")).exists();
                            let chosen = (matches!(kind, WorkKind::Http { .. } | WorkKind::Ftp) && !beside)
                                .then(|| inner.temp_dir.clone())
                                .flatten();
                            inner.db.execute(
                                "UPDATE tasks SET temp_dir=?1 WHERE id=?2",
                                params![chosen.clone().unwrap_or_default(), id],
                            )?;
                            chosen.map(PathBuf::from)
                        }
                    };
                    let last_try = inner.clock.now();
                    inner.db.execute("UPDATE tasks SET status='running',retry_at=NULL,last_try_at=?2,updated_at=CURRENT_TIMESTAMP WHERE id=?1", params![id, last_try])?;
                    let stop = Arc::new(AtomicBool::new(false));
                    let stop_requested = Arc::new(AtomicBool::new(false));
                    inner.active.insert(
                        id,
                        Active {
                            queue_id,
                            stop: stop.clone(),
                            stop_requested: stop_requested.clone(),
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
                        login: found,
                        proxy_login: inner.credentials.proxy_login().map(|stored| xdm_http::Login {
                            username: stored.username.clone(),
                            password: stored.password.clone(),
                            allow_insecure: false,
                        }),
                        stall_timeout: Duration::from_secs(inner.stall_timeout),
                        stop,
                        stop_requested,
                        policy: ResumePolicy { ignore_modification_date: ignore_modified != 0 },
                        name_from_server: name_from_server != 0,
                        temp_dir,
                        set_file_time: preserve_time != 0,
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
            let quota_due = bandwidth::quota_due(&inner);
            let next_deadline =
                [inner.queue_start_at, inner.queue_stop_at, retry_due, completion_due, quota_due]
                    .into_iter()
                    .flatten()
                    .min();
            // While a quota is counting, tick each second so the ledger persists.
            let ticking = quota_due.is_some() && !inner.active.is_empty();
            let wait = next_deadline
                .map(|at| Duration::from_secs((at - now).max(1) as u64))
                .unwrap_or(Duration::from_secs(30))
                .min(Duration::from_secs(if scheduled || ticking { 1 } else { 30 }));
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
                let media_login = work.login.as_ref().map(|login| xdm_media::OriginLogin {
                    url: work.url.clone(),
                    username: login.username.clone(),
                    password: login.password.clone(),
                    allow_insecure: login.allow_insecure,
                });
                let result: Result<()> = match work.kind {
                    WorkKind::Http {
                        headers,
                        connections,
                        method,
                        body,
                        control,
                    } => {
                        // N14: the naming decision is offered once, right after
                        // probing; the daemon records the chosen destination.
                        let mut named = work
                            .name_from_server
                            .then(|| {
                                let service = service.clone();
                                let id = work.id;
                                let provisional = work.output.to_string_lossy().into_owned();
                                Box::new(
                                    move |metadata: xdm_http::ResponseMetadata| {
                                        choose_server_destination(&service, id, &provisional, &metadata)
                                    },
                                ) as Box<dyn FnMut(ResponseMetadata) -> Result<PathBuf> + Send>
                            });
                        let named = named.as_mut().map(|decide| {
                            &mut **decide
                                as &mut (dyn FnMut(ResponseMetadata) -> Result<PathBuf> + Send)
                        });
                        download_ex(
                            Options {
                                login: work.login.clone(),
                                proxy_login: work.proxy_login.clone(),
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
                                temp_dir: work.temp_dir.clone(),
                                set_file_time: work.set_file_time,
                            },
                            control,
                            work.stall_timeout,
                            work.policy,
                            named,
                        )
                    }
                    WorkKind::Ftp => match work.ftp.clone() {
                        None => Err("FTP progress holder missing".into()),
                        Some(live) => xdm_ftp::download(
                            xdm_ftp::FtpOptions {
                                url: work.url,
                                output: work.output,
                                restart: false,
                                stop: work.stop.clone(),
                                temp_dir: work.temp_dir.clone(),
                                set_file_time: work.set_file_time,
                            },
                            |done, total| live.report(done, total),
                        ),
                    },
                    WorkKind::Media { mode, headers } => xdm_media::with_origin_login(media_login, || match &mode {
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
                    }),
                };
                if let Ok(mut inner) = service.inner.lock() {
                    let interrupted_for_queue = inner.queue_interrupted.remove(&work.id);
                    let (status, error, challenge, reason) = worker_outcome(
                        &result,
                        daemon_stopped(&work.stop, &work.stop_requested),
                        interrupted_for_queue,
                    );
                    // N17: the failure class is kept typed next to the message.
                    let error_kind = match &result {
                        Err(e) if status != "paused" && status != "queued" => {
                            Some(xdm_http::classify(e.as_ref()).as_str())
                        }
                        _ => None,
                    };
                    let proxy_challenge = challenge.as_ref().is_some_and(|c| c.status == 407);
                    let _ = inner.finish_task(
                        work.id,
                        status,
                        error,
                        challenge.and_then(|challenge| challenge.realm),
                        reason,
                    );
                    let _ = inner.db.execute(
                        "UPDATE tasks SET error_kind=?1,auth_proxy=?2 WHERE id=?3",
                        params![error_kind, i64::from(proxy_challenge), work.id],
                    );
                    inner.active.remove(&work.id);
                    service.wake.notify_one();
                }
            });
        }
    }
}

/// The engine flips the shared stop flag when it aborts a failing transfer;
/// only a stop the daemon itself requested counts as a pause or queue stop.
fn daemon_stopped(stop: &AtomicBool, requested: &AtomicBool) -> bool {
    stop.load(Ordering::Relaxed) && requested.load(Ordering::Relaxed)
}

/// Maps a worker result to the recorded task outcome. A 401/407 challenge
/// becomes `auth_required` instead of `failed` so credentials can be offered
/// once; a range- or checkpoint-unusable failure becomes `restart_required`
/// so the queue retry policy never loops on it. Both carry their detail in
/// the summary instead of a bare failure.
fn worker_outcome(
    result: &Result<()>,
    stopped: bool,
    interrupted: bool,
) -> (
    &'static str,
    Option<String>,
    Option<xdm_http::AuthChallenge>,
    Option<&'static str>,
) {
    match result {
        Ok(()) => ("complete", None, None, None),
        Err(_) if stopped && interrupted => ("queued", None, None, None),
        Err(e) if !stopped && e.to_string().contains(xdm_http::QUOTA_REACHED) => {
            ("quota_wait", None, None, None)
        }
        Err(e) if stopped => ("paused", Some(e.to_string()), None, None),
        Err(e) => {
            if let Some(reason) = restart_reason(e.as_ref()) {
                return ("restart_required", Some(e.to_string()), None, Some(reason));
            }
            match xdm_http::auth_challenge(e.as_ref()) {
                Some(challenge) => ("auth_required", Some(e.to_string()), Some(challenge), None),
                None => ("failed", Some(e.to_string()), None, None),
            }
        }
    }
}

impl Inner {
    /// Records a worker outcome. Completed tasks leave their queue (history
    /// kept); failed tasks with a transient error are re-queued per the
    /// queue's retry policy. Manual and queue stops never reach the retry
    /// path, and neither do `auth_required` nor `restart_required`: both wait
    /// for an explicit user action instead of looping.
    fn finish_task(
        &mut self,
        id: i64,
        status: &str,
        error: Option<String>,
        auth_realm: Option<String>,
        restart_reason: Option<&str>,
    ) -> Result<()> {
        let (queue_id, count): (i64, i64) = self.db.query_row(
            "SELECT queue_id,retry_count FROM tasks WHERE id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        // Last-known size and the unverified-length flag outlive the live counters.
        if let Some(active) = self.active.get(&id) {
            let snapshot = match (&active.control, &active.ftp) {
                (Some(control), _) => Some((control.progress(), control.size_unverified())),
                (None, Some(ftp)) => Some((ftp.progress(), false)),
                _ => None,
            };
            if let Some((progress, unverified)) = snapshot {
                let total = progress.total.or((status == "complete").then_some(progress.downloaded));
                self.db.execute(
                    "UPDATE tasks SET size_done=?1,size_total=?2,size_unverified=?3 WHERE id=?4",
                    params![
                        progress.downloaded as i64,
                        total.map(|total| total as i64),
                        i64::from(unverified && status == "complete"),
                        id
                    ],
                )?;
            }
        }
        if status == "complete" {
            self.task_logins.remove(&id);
            self.db.execute("UPDATE tasks SET status='complete',error=NULL,auth_realm=NULL,restart_reason=NULL,history_queue_id=queue_id,queue_id=CASE WHEN COALESCE((SELECT sync FROM queues WHERE id=tasks.queue_id),0) THEN queue_id ELSE 0 END,retry_at=NULL,updated_at=CURRENT_TIMESTAMP WHERE id=?1", [id])?;
            // Replay secrets are not kept once a task is done, except where a sync
            // queue still revalidates it with them.
            let row: Option<(String, i64)> = self
                .db
                .query_row("SELECT headers_json,queue_id FROM tasks WHERE id=?1", [id], |row| Ok((row.get(0)?, row.get(1)?)))
                .optional()?;
            if let Some((json, 0)) = row {
                if let Ok(mut headers) = serde_json::from_str::<Vec<(String, String)>>(&json) {
                    let before = headers.len();
                    headers.retain(|(name, _)| !transfer_lists::secret_header(name));
                    if headers.len() != before {
                        self.db.execute("UPDATE tasks SET headers_json=?1 WHERE id=?2", params![serde_json::to_string(&headers)?, id])?;
                    }
                }
            }
            return Ok(());
        }
        if status == "auth_required" {
            // The typed login was rejected (or never fit); it is not kept.
            self.task_logins.remove(&id);
            self.db.execute(
                "UPDATE tasks SET status='auth_required',error=?1,auth_realm=?2,retry_at=NULL,updated_at=CURRENT_TIMESTAMP WHERE id=?3",
                params![error, auth_realm, id],
            )?;
            return Ok(());
        }
        if status == "restart_required" {
            self.task_logins.remove(&id);
            self.db.execute(
                "UPDATE tasks SET status='restart_required',error=?1,restart_reason=?2,retry_at=NULL,updated_at=CURRENT_TIMESTAMP WHERE id=?3",
                params![error, restart_reason, id],
            )?;
            cancel_pending(self, queue_id, "suppressed");
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
        // A task that failed for good no longer needs its typed credentials.
        if status == "failed" {
            self.task_logins.remove(&id);
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
    } else if shutdown::requested() {
        Reply::error("daemon is shutting down")
    } else if let Command::Hello { client_version, protocol } = &request.command {
        xdm_ipc::ops::log_debug(&format!("hello from client {client_version} protocol {protocol}"));
        let mut reply = Reply::list(Vec::new());
        reply.daemon_version = Some(xdm_ipc::ops::VERSION.to_owned());
        reply.protocol = Some(xdm_ipc::PROTOCOL);
        if *protocol != xdm_ipc::PROTOCOL {
            reply.ok = false;
            reply.error = Some(format!(
                "incompatible protocol: client speaks {protocol}, daemon speaks {}",
                xdm_ipc::PROTOCOL
            ));
        }
        reply
    } else {
        service
            .command(request.command)
            .unwrap_or_else(|e| Reply::error(e.to_string()))
    };
    xdm_ipc::write_frame_max(&mut stream, &reply, xdm_ipc::MAX_REPLY_FRAME)?;
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

impl Service {
    /// Stops accepting work, stops every transfer so it checkpoints (the
    /// workers record them as queued, which resumes after restart), waits a
    /// bounded time, then flushes the WAL into the database file.
    fn shutdown(&self, wait: Duration) {
        shutdown::request();
        xdm_ipc::ops::log_info("shutdown requested: stopping transfers");
        if let Ok(mut inner) = self.inner.lock() {
            interrupt_queue_workers(&mut inner, None);
        }
        self.wake.notify_all();
        let deadline = Instant::now() + wait;
        loop {
            let remaining = self.inner.lock().map_or(0, |inner| inner.active.len());
            if remaining == 0 {
                break;
            }
            if Instant::now() >= deadline {
                xdm_ipc::ops::log_warn(&format!("shutdown timed out with {remaining} transfers still stopping"));
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        if let Ok(inner) = self.inner.lock() {
            let _ = inner.db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");
        }
        xdm_ipc::ops::log_info("shutdown complete");
    }
}

const SHUTDOWN_WAIT: Duration = Duration::from_secs(15);

fn fatal(message: impl std::fmt::Display) -> ! {
    xdm_ipc::ops::log_error(&message.to_string());
    eprintln!("xdm-daemon: {message}");
    std::process::exit(1);
}

fn main() -> Result<()> {
    xdm_ipc::ops::handle_version_flag("xdm-daemon");
    shutdown::block_signals();
    let paths = Paths::open()?;
    xdm_ipc::ops::init_logging(&paths.dir, "xdm-daemon");
    xdm_ipc::ops::install_panic_hook(&paths.dir, "xdm-daemon");
    let _instance = lock::acquire(&paths.dir).unwrap_or_else(|e| fatal(e));
    xdm_ipc::ops::log_info(&format!("starting {}", xdm_ipc::ops::version_line("xdm-daemon")));
    let listener = paths.listen().unwrap_or_else(|e| fatal(format!("cannot listen: {e}")));
    let service = Service::open(&paths).unwrap_or_else(|e| fatal(e));
    {
        let service = service.clone();
        let socket = paths.socket.clone();
        thread::spawn(move || {
            shutdown::wait_for_signal();
            if cfg!(unix) {
                let _ = std::fs::remove_file(&socket);
            }
            service.shutdown(SHUTDOWN_WAIT);
            std::process::exit(0);
        });
    }
    let worker = service.clone();
    thread::spawn(move || worker.run());
    let syncer = service.clone();
    thread::spawn(move || {
        loop {
            let next = syncer.sync_pump().unwrap_or_else(|error| {
                eprintln!("sync scheduler error: {error}");
                Some(Duration::from_secs(5))
            });
            // Sleep until the next due check, or indefinitely with none; any
            // sync change notifies and takes effect at once.
            let (flag, condvar) = &*syncer.sync_wake;
            let Ok(guard) = flag.lock() else { return };
            let mut guard = match next {
                Some(limit) => match condvar.wait_timeout_while(guard, limit, |pending| !*pending) {
                    Ok((guard, _)) => guard,
                    Err(_) => return,
                },
                None => match condvar.wait_while(guard, |pending| !*pending) {
                    Ok(guard) => guard,
                    Err(_) => return,
                },
            };
            *guard = false;
        }
    });
    eprintln!("xdm-daemon listening locally");
    for connection in listener.incoming() {
        match connection {
            Ok(stream) => {
                let service = service.clone();
                let token = paths.token.clone();
                thread::spawn(move || {
                    if let Err(e) = handle(stream, service, token) {
                        xdm_ipc::ops::log_warn(&format!("client error: {e}"));
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
                output: format!("{}/{name}", abs!("/nonexistent-xdm-test")),
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
                output: format!("{}/{name}", abs!("/nonexistent-xdm-test")),
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
            .finish_task(id, status, error.map(str::to_string), None, None)
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
    fn quota_window_pauses_then_resumes_on_the_injected_clock() {
        let (service, clock, _) = service(1000);
        let id = submit(&service, "quota0");
        let status = |service: &Service, id: i64| -> String {
            service.inner.lock().unwrap().db
                .query_row("SELECT status FROM tasks WHERE id=?1", [id], |row| row.get(0))
                .unwrap()
        };
        assert!(!service.command(Command::SetQuota { mib: 5, window_seconds: 0 }).unwrap().ok);
        let reply = service.command(Command::SetQuota { mib: 1, window_seconds: 3600 }).unwrap();
        assert_eq!(reply.bandwidth.unwrap().quota_mib, 1);
        // Usage over the limit opens the window and flags it.
        {
            let inner = service.inner.lock().unwrap();
            inner.governor.set_used(2 * 1024 * 1024);
            inner.db.execute("UPDATE tasks SET status='quota_wait' WHERE id=?1", [id]).unwrap();
        }
        let report = service.command(Command::BandwidthStatus).unwrap().bandwidth.unwrap();
        assert!(report.quota_exceeded);
        assert_eq!(report.quota_resets_at, Some(1000 + 3600));
        clock.set(1000 + 3599);
        service.command(Command::BandwidthStatus).unwrap();
        assert_eq!(status(&service, id), "quota_wait");
        clock.set(1000 + 3600);
        let report = service.command(Command::BandwidthStatus).unwrap().bandwidth.unwrap();
        assert!(!report.quota_exceeded && report.quota_used_bytes == 0);
        assert_eq!(status(&service, id), "queued");
        // Turning the quota off also wakes waiting tasks.
        {
            let inner = service.inner.lock().unwrap();
            inner.governor.set_used(2 * 1024 * 1024);
            inner.db.execute("UPDATE tasks SET status='quota_wait' WHERE id=?1", [id]).unwrap();
        }
        service.command(Command::SetQuota { mib: 0, window_seconds: 0 }).unwrap();
        assert_eq!(status(&service, id), "queued");
        let error: Result<()> = Err(xdm_http::QUOTA_REACHED.into());
        assert_eq!(worker_outcome(&error, false, false).0, "quota_wait");
        assert_eq!(worker_outcome(&error, true, false).0, "paused");
    }

    #[test]
    fn bandwidth_commands_validate_and_persist_rules() {
        let (service, _, _) = service(1000);
        let bw = |command| service.command(command).unwrap();
        assert!(!bw(Command::SetGlobalLimit { kib: Some(0) }).ok);
        assert!(!bw(Command::SetHostLimit { pattern: "a b".into(), kib: Some(1) }).ok);
        assert!(!bw(Command::SetHostConnections { pattern: "*".into(), count: Some(0) }).ok);
        bw(Command::SetGlobalLimit { kib: Some(512) });
        bw(Command::SetHostLimit { pattern: "*.Example.com".into(), kib: Some(64) });
        bw(Command::SetHostConnections { pattern: "*".into(), count: Some(4) });
        let report = bw(Command::BandwidthStatus).bandwidth.unwrap();
        assert_eq!(report.global_kib, Some(512));
        assert_eq!(report.host_limits, vec![("*.example.com".to_string(), 64)]);
        assert_eq!(report.host_connections, vec![("*".to_string(), 4)]);
        // The persisted settings restore into a fresh governor.
        let inner = service.inner.lock().unwrap();
        let governor = xdm_http::Governor::new();
        bandwidth::load(&inner.db, &governor);
        assert_eq!(governor.global_kib(), Some(512));
        assert_eq!(governor.snapshot().host_budgets, vec![("*".to_string(), 4)]);
        drop(inner);
        bw(Command::SetHostLimit { pattern: "*.example.com".into(), kib: None });
        bw(Command::SetGlobalLimit { kib: None });
        let report = bw(Command::BandwidthStatus).bandwidth.unwrap();
        assert!(report.global_kib.is_none() && report.host_limits.is_empty());
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
    fn submit_ex_detects_normalized_duplicates_and_allows_with_flag() {
        let (service, _, _) = service(1000);
        let ex = |request_id: &str, url: &str, name: &str, allow: bool| {
            service
                .command(Command::SubmitEx {
                    request_id: request_id.into(),
                    url: url.into(),
                    output: format!("{}/{name}", abs!("/nonexistent-xdm-test")),
                    headers: Vec::new(),
                    method: "GET".into(),
                    body: None,
                    priority: 1,
                    connections: 1,
                    name_from_server: false,
                    ignore_modification_date: false,
                    allow_duplicate: allow,
                    preserve_server_time: true,
                })
                .unwrap()
        };
        let first = ex("dup-first", "http://example.test/path", "dup-first.bin", false);
        assert!(first.ok);
        let first = first.task_id.unwrap();
        // Scheme/host case and fragments normalize away; path case does not.
        let fragment = ex("dup-frag", "http://example.test/path#anchor", "dup-frag.bin", false);
        assert!(!fragment.ok);
        assert_eq!(fragment.duplicate_of, Some(first));
        let host = ex("dup-host", "http://EXAMPLE.test/path", "dup-host.bin", false);
        assert!(!host.ok && host.duplicate_of == Some(first));
        // The path case is not normalized away, so this is a new download.
        assert!(ex("dup-case", "http://example.test/PATH", "dup-case.bin", false).ok);
        // An explicit can-download-again creates a second task.
        let again = ex("dup-allow", "http://example.test/path", "dup-allow.bin", true);
        assert!(again.ok);
        assert_ne!(again.task_id.unwrap(), first);
        // The legacy submit keeps its old no-duplicate-check semantics.
        let legacy = service
            .command(Command::Submit {
                request_id: "dup-legacy".into(),
                url: "http://example.test/path".into(),
                output: abs!("/nonexistent-xdm-test/dup-legacy.bin").into(),
                headers: Vec::new(),
                priority: 1,
                connections: 1,
            })
            .unwrap();
        assert!(legacy.ok, "{:?}", legacy.error);
    }

    #[test]
    fn submit_ex_flags_travel_to_work() {
        let (service, _, _) = service(1000);
        let id = service
            .command(Command::SubmitEx {
                request_id: "flags-in".into(),
                url: "http://example.test/f.bin".into(),
                output: abs!("/nonexistent-xdm-test/flags.bin").into(),
                headers: Vec::new(),
                method: "GET".into(),
                body: None,
                priority: 1,
                connections: 1,
                name_from_server: true,
                ignore_modification_date: true,
                allow_duplicate: false,
                preserve_server_time: true,
            })
            .unwrap()
            .task_id
            .unwrap();
        let work = service.next_work().unwrap();
        assert_eq!(work.id, id);
        assert!(work.name_from_server && work.policy.ignore_modification_date);
        assert_eq!(work.output, PathBuf::from(abs!("/nonexistent-xdm-test/flags.bin")));
        // Legacy submissions default both policies off.
        let legacy = submit(&service, "flags-legacy");
        let work = service.next_work().unwrap();
        assert_eq!(work.id, legacy);
        assert!(!work.name_from_server && !work.policy.ignore_modification_date);
    }

    #[test]
    fn server_names_stay_in_their_directory_and_never_overwrite() {
        let (service, _, _) = service(1000);
        let directory = std::env::temp_dir().join(format!("xdm-naming-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let row = |request_id: &str, output: &std::path::Path| {
            let inner = service.inner.lock().unwrap();
            let id = next_task_id(&inner.db).unwrap();
            inner
                .db
                .execute(
                    "INSERT INTO tasks(id,request_id,url,output,headers_json,priority,connections,status,kind) VALUES (?1,?2,'http://h/x',?3,'[]',1,1,'queued','http')",
                    params![id, request_id, output.display().to_string()],
                )
                .unwrap();
            id
        };
        let metadata = |disposition: &str| ResponseMetadata {
            effective_url: "http://h/dir/final.bin".into(),
            content_disposition: Some(disposition.into()),
            content_length: Some(8),
            content_type: None,
        };
        // The naming resolution runs before streaming; the row records it.
        let first = row("srv-first", &directory.join("plain.bin"));
        let chosen = choose_server_destination(
            &service,
            first,
            &directory.join("plain.bin").display().to_string(),
            &metadata(r#"attachment; filename="server pick.pdf""#),
        )
        .unwrap();
        assert_eq!(chosen, directory.join("server pick.pdf"));
        assert_eq!(
            service.command(Command::GetTask { task_id: first }).unwrap().tasks[0].output,
            chosen.display().to_string()
        );
        // An existing file is never replaced: the name gets a " (2)" suffix.
        std::fs::write(directory.join("conflict.tar.gz"), b"keep").unwrap();
        let second = row("srv-second", &directory.join("other.bin"));
        let conflict = choose_server_destination(
            &service,
            second,
            &directory.join("other.bin").display().to_string(),
            &metadata(r#"attachment; filename="conflict.tar.gz""#),
        )
        .unwrap();
        assert_eq!(conflict, directory.join("conflict.tar (2).gz"));
        assert_eq!(std::fs::read(directory.join("conflict.tar.gz")).unwrap(), b"keep");
        assert!(!directory.join("conflict.tar.gz.part").exists());
        // A server name equal to the provisional name keeps it untouched.
        let third = row("srv-third", &directory.join("same.zip"));
        let kept = choose_server_destination(
            &service,
            third,
            &directory.join("same.zip").display().to_string(),
            &metadata(r#"attachment; filename="same.zip""#),
        )
        .unwrap();
        assert_eq!(kept, directory.join("same.zip"));
        assert_eq!(
            service.command(Command::GetTask { task_id: third }).unwrap().tasks[0].output,
            directory.join("same.zip").display().to_string()
        );
        std::fs::remove_dir_all(directory).unwrap();
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
                output: abs!("/nonexistent-xdm-test/clone.bin").into(),
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
    fn stall_timeout_setting_validates_and_survives_reopen() {
        let (service, _, _) = service(1000);
        let get = |service: &Service| {
            service
                .command(Command::GetStallTimeout)
                .unwrap()
                .stall_timeout_seconds
                .unwrap()
        };
        assert_eq!(get(&service), xdm_http::DEFAULT_STALL_TIMEOUT.as_secs());
        assert!(!service.command(Command::SetStallTimeout { seconds: 0 }).unwrap().ok);
        assert!(!service.command(Command::SetStallTimeout { seconds: 601 }).unwrap().ok);
        assert!(service.command(Command::SetStallTimeout { seconds: 2 }).unwrap().ok);
        assert_eq!(get(&service), 2);
        // Queued HTTP work carries the setting to the engine.
        let id = submit(&service, "stall-work");
        let work = service.next_work().unwrap();
        assert_eq!(work.id, id);
        assert_eq!(work.stall_timeout, Duration::from_secs(2));
        // The setting persists like the proxy setting.
        let path = std::env::temp_dir().join(format!("xdm-stall-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let file_service =
                Service::open_db(&path, Arc::new(FakeClock::new(1000, 0)), Arc::new(RecordOnly))
                    .unwrap();
            assert!(
                file_service
                    .command(Command::SetStallTimeout { seconds: 600 })
                    .unwrap()
                    .ok
            );
        }
        let persisted =
            Service::open_db(&path, Arc::new(FakeClock::new(1000, 0)), Arc::new(RecordOnly))
                .unwrap();
        assert_eq!(get(&persisted), 600);
        drop(persisted);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn restart_required_parks_instead_of_retrying() {
        let (service, _, _) = service(1000);
        let q = queue(&service, "Restart");
        let id = submit(&service, "rr0");
        service.command(Command::AssignQueue { queue_id: q, task_ids: vec![id] }).unwrap();
        // Infinite retries must still not loop on a restart-required failure.
        service
            .command(Command::QueueSetRetry { queue_id: q, max_retries: -1, delay_seconds: 1 })
            .unwrap();
        {
            let mut inner = service.inner.lock().unwrap();
            inner
                .db
                .execute("UPDATE tasks SET status='running' WHERE id=?1", [id])
                .unwrap();
            inner
                .finish_task(
                    id,
                    "restart_required",
                    Some("remote validators changed during range transfer".into()),
                    None,
                    Some("remote validators changed during range transfer"),
                )
                .unwrap();
        }
        assert_eq!(task_state(&service, id), ("restart_required".into(), 0, None, q));
        let task = service.command(Command::GetTask { task_id: id }).unwrap().tasks.remove(0);
        assert_eq!(task.status, "restart_required");
        assert_eq!(
            task.restart_reason.as_deref(),
            Some("remote validators changed during range transfer")
        );
        // Classification: the three restart causes versus ordinary failures.
        let flip: Result<()> = Err(Box::new(xdm_http::RangeUnreliable {
            at: 7,
            reason: "invalid Content-Range".into(),
        }));
        assert_eq!(worker_outcome(&flip, false, false).0, "restart_required");
        let identity: Result<()> =
            Err("partial download identity changed or checkpoint invalid; use --restart".into());
        assert_eq!(worker_outcome(&identity, false, false).0, "restart_required");
        let validators: Result<()> = Err("remote validators changed during range transfer".into());
        assert_eq!(worker_outcome(&validators, false, false).0, "restart_required");
        let plain: Result<()> = Err("connection reset".into());
        assert_eq!(worker_outcome(&plain, false, false).0, "failed");
        // A manual stop still wins: the partial data waits for the decision.
        assert_eq!(worker_outcome(&flip, true, false).0, "paused");
        assert_eq!(worker_outcome(&flip, true, true).0, "queued");
        // The engine flips the shared stop flag when it aborts a transfer;
        // only a stop the daemon requested counts as a pause.
        let stop = Arc::new(AtomicBool::new(false));
        let requested = Arc::new(AtomicBool::new(false));
        stop.store(true, Ordering::Relaxed);
        assert!(!daemon_stopped(&stop, &requested), "engine abort is not a pause");
        requested.store(true, Ordering::Relaxed);
        assert!(daemon_stopped(&stop, &requested));
    }

    #[test]
    fn moving_task_refuses_remove_restart_redownload_and_move() {
        let (service, _, _) = service(1000);
        let id = submit(&service, "moving");
        service.inner.lock().unwrap().moving.insert(id);
        assert!(!service.command(Command::Remove { task_id: id }).unwrap().ok);
        assert!(!service.command(Command::TaskRestart { task_id: id }).unwrap().ok);
        assert!(!service.command(Command::TaskMoveFile { task_id: id, new_path: abs!("/tmp/x-moving.bin").into() }).unwrap().ok);
        let reply = service
            .command(Command::Redownload { task_id: id, request_id: "rd".into(), output: abs!("/nonexistent-xdm-test/rd.bin").into() })
            .unwrap();
        assert!(!reply.ok && reply.error.unwrap().contains("moved"));
        service.inner.lock().unwrap().moving.clear();
        assert!(service.command(Command::Remove { task_id: id }).unwrap().ok);
    }

    fn secret_entry(name: &str, cookie: &str) -> xdm_ipc::ImportEntry {
        xdm_ipc::ImportEntry {
            url: format!("http://h.test/{name}"),
            filename: name.into(),
            kind: "http".into(),
            priority: 1,
            connections: 1,
            rate_limit: None,
            video: None,
            audio: None,
            category: "Other".into(),
            headers: vec![("Cookie".into(), cookie.into()), ("Authorization".into(), "Bearer tok".into()), ("User-Agent".into(), "ua".into())],
        }
    }

    #[test]
    fn import_receipts_never_store_credentials_and_still_detect_changed_secrets() {
        let (service, _, _) = service(1000);
        let import = |cookie: &str| {
            service
                .command(Command::ImportListEx {
                    request_id: "secret-1".into(),
                    directory: abs!("/nonexistent-xdm-test/dir").into(),
                    entries: vec![secret_entry("s.bin", cookie)],
                    enqueue: false,
                    allow_duplicate: false,
                })
                .unwrap()
        };
        let first = import("session=TOPSECRET");
        assert!(first.ok, "{:?}", first.error);
        let inner = service.inner.lock().unwrap();
        let payload: String = inner.db.query_row("SELECT payload_json FROM imports WHERE request_id='secret-1'", [], |r| r.get(0)).unwrap();
        assert!(!payload.contains("TOPSECRET") && !payload.contains("Bearer") && payload.contains("User-Agent"), "{payload}");
        let headers: String = inner.db.query_row("SELECT headers_json FROM tasks", [], |r| r.get(0)).unwrap();
        assert!(headers.contains("TOPSECRET"), "the task itself keeps its replay headers");
        drop(inner);
        let replay = import("session=TOPSECRET");
        assert!(replay.ok && replay.task_ids == first.task_ids);
        let changed = import("session=OTHER");
        assert!(!changed.ok && changed.error.unwrap().contains("different batch"));
    }

    #[test]
    fn completed_tasks_drop_credentials_from_their_stored_headers() {
        let (service, _, _) = service(1000);
        let reply = service
            .command(Command::ImportListEx {
                request_id: "secret-2".into(),
                directory: abs!("/nonexistent-xdm-test/dir").into(),
                entries: vec![secret_entry("done.bin", "session=TOPSECRET")],
                enqueue: false,
                allow_duplicate: false,
            })
            .unwrap();
        let id = reply.task_ids[0];
        finish(&service, id, "complete", None);
        let headers: String = service.inner.lock().unwrap().db
            .query_row("SELECT headers_json FROM tasks WHERE id=?1", [id], |r| r.get(0)).unwrap();
        assert!(!headers.contains("TOPSECRET") && !headers.contains("Bearer") && headers.contains("User-Agent"), "{headers}");
    }

    #[test]
    fn imports_reject_forbidden_request_headers() {
        let (service, _, _) = service(1000);
        for name in ["Host", "Range", "if-range", "Accept-Encoding", "Content-Length", "Transfer-Encoding"] {
            let mut entry = secret_entry("f.bin", "a=b");
            entry.headers.push((name.into(), "x".into()));
            let reply = service
                .command(Command::ImportListEx {
                    request_id: format!("forbid-{name}"),
                    directory: abs!("/nonexistent-xdm-test/dir").into(),
                    entries: vec![entry],
                    enqueue: false,
                    allow_duplicate: false,
                })
                .unwrap();
            assert!(!reply.ok, "{name}");
        }
    }

    #[test]
    fn ftp_partials_share_the_http_temp_naming_and_are_owned() {
        let temp = std::env::temp_dir().join(format!("xdm-ftp-own-{}", std::process::id()));
        std::fs::create_dir_all(&temp).unwrap();
        let output = Path::new(abs!("/nonexistent-xdm-test/dir/file.bin"));
        let (ftp_part, ftp_check) = xdm_ftp::partial_paths(output, Some(&temp));
        assert_eq!(ftp_part, xdm_http::partial_paths(output, Some(&temp)).0);
        let owned = owned_partial_paths(output, Some(temp.to_str().unwrap()));
        assert!(owned.contains(&ftp_part) && owned.contains(&ftp_check));
        assert!(owned.contains(&PathBuf::from(abs!("/nonexistent-xdm-test/dir/file.bin.part.ftp"))));
    }

    #[test]
    fn task_restart_deletes_only_own_part_files_and_requeues() {
        let (service, _, _) = service(1000);
        let directory = std::env::temp_dir().join(format!("xdm-task-restart-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let output = directory.join("own.bin");
        let neighbor = directory.join("neighbor.bin");
        let own = http_part_paths(&output);
        let foreign = http_part_paths(&neighbor);
        for path in own.iter().chain(foreign.iter()) {
            std::fs::write(path, b"x").unwrap();
        }
        let id = service
            .command(Command::Submit {
                request_id: "restart-own".into(),
                url: format!("http://127.0.0.1:1/{}", output.display()),
                output: output.display().to_string(),
                headers: Vec::new(),
                priority: 1,
                connections: 1,
            })
            .unwrap()
            .task_id
            .unwrap();
        // Only a task parked in restart_required may restart from zero.
        assert!(!service.command(Command::TaskRestart { task_id: id }).unwrap().ok);
        {
            let mut inner = service.inner.lock().unwrap();
            inner
                .db
                .execute("UPDATE tasks SET status='running' WHERE id=?1", [id])
                .unwrap();
            inner
                .finish_task(
                    id,
                    "restart_required",
                    Some("stopped honoring ranges".into()),
                    None,
                    Some("server stopped honoring byte ranges"),
                )
                .unwrap();
        }
        assert!(service.command(Command::TaskRestart { task_id: id }).unwrap().ok);
        assert_eq!(task_state(&service, id).0, "queued");
        let task = service.command(Command::GetTask { task_id: id }).unwrap().tasks.remove(0);
        assert_eq!(task.status, "queued");
        assert_eq!(task.restart_reason, None);
        assert_eq!(task.error, None);
        assert!(own.iter().all(|path| !path.exists()));
        assert!(foreign.iter().all(|path| path.exists()));
        assert!(!service.command(Command::TaskRestart { task_id: 999 }).unwrap().ok);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn ftp_tasks_are_accepted_and_selected_as_work() {
        let (service, _, _) = service(1000);
        let ftp = service
            .command(Command::Submit {
                request_id: "ftp-1".into(),
                url: "ftp://downloads.invalid/pub/pubbig.bin".into(),
                output: abs!("/nonexistent-xdm-test/ftp.bin").into(),
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
                output: abs!("/nonexistent-xdm-test/bad").into(),
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
        let mkv = job(&service, "m-mkv", abs!("/nonexistent-xdm-test/film.mkv"), MediaMode::Video { video: 2, audio: Some(3), subtitles: vec![5] }).task_id.unwrap();
        let audio_id = job(
            &service,
            "m-song",
            abs!("/nonexistent-xdm-test/song.mp3"),
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
                    output: abs!("/nonexistent-xdm-test/plain.mp4").into(),
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

    #[test]
    fn migration_backs_up_old_database_and_keeps_three() {
        let dir = std::env::temp_dir().join(format!("xdm-backup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tasks.sqlite3");
        let make_old = || {
            let _ = std::fs::remove_file(&path);
            let db = Connection::open(&path).unwrap();
            db.execute_batch(
                "CREATE TABLE tasks (id INTEGER PRIMARY KEY, request_id TEXT NOT NULL UNIQUE, url TEXT NOT NULL, output TEXT NOT NULL UNIQUE, headers_json TEXT NOT NULL, priority INTEGER NOT NULL, connections INTEGER NOT NULL, status TEXT NOT NULL, error TEXT);
                 INSERT INTO tasks(id,request_id,url,output,headers_json,priority,connections,status) VALUES (7,'a','http://x/a','/o/a','[]',1,1,'queued');",
            )
            .unwrap();
        };
        make_old();
        Service::open_db(&path, Arc::new(FakeClock::new(1000, 0)), Arc::new(RecordOnly)).unwrap();
        let backups = dbsafety::list_backups(&path);
        assert_eq!(backups.len(), 1);
        // The backup holds the pre-migration schema and rows.
        let old = Connection::open(&backups[0]).unwrap();
        let columns: Vec<String> = old
            .prepare("PRAGMA table_info(tasks)")
            .unwrap()
            .query_map([], |r| r.get(1))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(!columns.contains(&"position".to_owned()));
        assert_eq!(old.query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        drop(old);
        let version: i64 = Connection::open(&path).unwrap().query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, dbsafety::SCHEMA_VERSION);
        // Already current: no further backup.
        Service::open_db(&path, Arc::new(FakeClock::new(1000, 0)), Arc::new(RecordOnly)).unwrap();
        assert_eq!(dbsafety::list_backups(&path).len(), 1);
        for _ in 0..4 {
            make_old();
            Service::open_db(&path, Arc::new(FakeClock::new(1000, 0)), Arc::new(RecordOnly)).unwrap();
        }
        assert_eq!(dbsafety::list_backups(&path).len(), dbsafety::BACKUPS_KEPT);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_database_is_refused_with_guidance() {
        let dir = std::env::temp_dir().join(format!("xdm-corrupt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tasks.sqlite3");
        std::fs::write(&path, vec![0x5au8; 8192]).unwrap();
        let error = Service::open_db(&path, Arc::new(FakeClock::new(1000, 0)), Arc::new(RecordOnly))
            .err()
            .expect("corrupt database must not open");
        assert!(error.to_string().contains("integrity check"), "{error}");
        assert_eq!(std::fs::read(&path).unwrap(), vec![0x5au8; 8192], "file untouched");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn auth_error(status: u16, realm: Option<&str>) -> Error {
        Box::new(xdm_http::AuthRequired(xdm_http::AuthChallenge {
            note: None,
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
        // HTTP tasks hand the login to the engine, which answers the server's own
        // challenge (Basic or Digest); no scheme is preselected as a header.
        assert_eq!(
            work.login,
            Some(xdm_http::Login::new("alice", "s3cret"))
        );
        let WorkKind::Http { headers, .. } = work.kind else { unreachable!() };
        assert!(headers.is_empty());
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
                output: abs!("/nonexistent-xdm-test/auth-explicit.bin").into(),
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
        assert_eq!(work.login, None);
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
                    output: format!("{}/{request_id}.mkv", abs!("/nonexistent-xdm-test")),
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
        // The login travels beside the headers (origin-scoped inside the manifest
        // walk), never as an Authorization header that ffmpeg would receive.
        assert!(headers.is_empty(), "{headers:?}");
        assert_eq!(work.login, Some(xdm_http::Login::new("alice", "s3cret")));
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
    fn typed_task_logins_are_dropped_on_every_terminal_state() {
        let (service, _, _) = service(1000);
        for (status, name) in [
            ("complete", "t-complete"),
            ("failed", "t-failed"),
            ("restart_required", "t-restart"),
            ("auth_required", "t-auth"),
        ] {
            let id = submit(&service, name);
            let mut inner = service.inner.lock().unwrap();
            inner.task_logins.insert(id, xdm_http::Login::new("u", "p"));
            inner.db.execute("UPDATE tasks SET status='running' WHERE id=?1", [id]).unwrap();
            inner.finish_task(id, status, Some("e".into()), None, Some("reason")).unwrap();
            assert!(!inner.task_logins.contains_key(&id), "{status} kept the typed login");
        }
        // Paused tasks keep it for the resume; removal drops it.
        let id = submit(&service, "t-pause");
        service.inner.lock().unwrap().task_logins.insert(id, xdm_http::Login::new("u", "p"));
        {
            let mut inner = service.inner.lock().unwrap();
            inner.db.execute("UPDATE tasks SET status='running' WHERE id=?1", [id]).unwrap();
            inner.finish_task(id, "paused", Some("paused".into()), None, None).unwrap();
            assert!(inner.task_logins.contains_key(&id));
        }
        assert!(service.command(Command::Remove { task_id: id }).unwrap().ok);
        assert!(!service.inner.lock().unwrap().task_logins.contains_key(&id));
    }

    #[test]
    fn proxy_urls_with_unparsable_credentials_are_rejected_and_never_returned() {
        let (service, _, _) = service(1000);
        let set = |url: &str| {
            service
                .command(Command::SetProxy {
                    proxy: ProxyChoice::Manual { http: Some(url.into()), https: None, bypass: Vec::new() },
                })
                .unwrap()
        };
        let rejected = set("http://u:pw/secret-part@h:1/extra/path");
        assert!(!rejected.ok);
        assert!(!format!("{rejected:?}").contains("secret-part"));
        let accepted = set("http://u:p/w?x#y@h:3128");
        assert!(accepted.ok, "{accepted:?}");
        let text = serde_json::to_string(&service.command(Command::GetProxy).unwrap()).unwrap();
        assert!(!text.contains("p/w") && text.contains("http://h:3128"), "{text}");
        let stored = service.inner.lock().unwrap().credentials.proxy_login().map(|c| c.password.clone());
        assert_eq!(stored.as_deref(), Some("p/w?x#y"));
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
            let (status, message, challenge, reason) = worker_outcome(&result, false, false);
            assert_eq!(status, "auth_required");
            assert!(message.as_deref().is_some_and(|text| text.contains("HTTP 401")));
            assert_eq!(challenge.as_ref().unwrap().realm.as_deref(), Some("xdm-test"));
            assert!(reason.is_none());
            inner
                .finish_task(id, status, message, challenge.and_then(|c| c.realm), reason)
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
            let (status, message, challenge, reason) = worker_outcome(&result, false, false);
            inner.finish_task(id, status, message, challenge.and_then(|c| c.realm), reason).unwrap();
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
