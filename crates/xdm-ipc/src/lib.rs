//! Versioned local protocol shared by CLI, native host, daemon, and future GUI.

use interprocess::local_socket::{
    GenericFilePath, GenericNamespaced, Listener, ListenerOptions, Stream, prelude::*,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::PathBuf;

pub const MAX_FRAME: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub version: u32,
    pub token: String,
    pub command: Command,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum Command {
    Submit {
        request_id: String,
        url: String,
        output: String,
        headers: Vec<(String, String)>,
        priority: i64,
        connections: usize,
    },
    /// Like `Submit` plus the HTTP method and replayable body for POST downloads.
    SubmitRequest {
        request_id: String,
        url: String,
        output: String,
        headers: Vec<(String, String)>,
        method: String,
        body: Option<Vec<u8>>,
        priority: i64,
        connections: usize,
    },
    SetProxy {
        proxy: ProxyChoice,
    },
    /// Like `SubmitMedia` plus web headers and a full job mode: audio-only,
    /// subtitle sidecar, or a video mux with subtitle renditions.
    SubmitMediaJob {
        request_id: String,
        url: String,
        output: String,
        #[serde(default)]
        headers: Vec<(String, String)>,
        mode: MediaMode,
        priority: i64,
    },
    GetProxy,
    SubmitMedia {
        request_id: String,
        url: String,
        output: String,
        video: usize,
        audio: Option<usize>,
        priority: i64,
    },
    List,
    ImportList {
        request_id: String,
        directory: String,
        entries: Vec<ImportEntry>,
        enqueue: bool,
    },
    ExportList,
    GetTask {
        task_id: i64,
    },
    Remove {
        task_id: i64,
    },
    Redownload {
        task_id: i64,
        request_id: String,
        output: String,
    },
    Pause {
        task_id: i64,
    },
    Resume {
        task_id: i64,
    },
    SetPriority {
        task_id: i64,
        priority: i64,
    },
    SetConcurrency {
        limit: usize,
    },
    SetTaskRate {
        task_id: i64,
        bytes_per_second: Option<u64>,
    },
    QueueStart,
    QueueStop,
    QueueCreate {
        name: String,
        limit: usize,
    },
    QueueSetRunning {
        queue_id: i64,
        running: bool,
    },
    QueueSetConcurrency {
        queue_id: i64,
        limit: usize,
    },
    QueueRename {
        queue_id: i64,
        name: String,
    },
    QueueRemove {
        queue_id: i64,
    },
    AssignQueue {
        queue_id: i64,
        task_ids: Vec<i64>,
    },
    QueueScheduleStart {
        at_unix: i64,
    },
    QueueScheduleStop {
        at_unix: i64,
    },
    /// Starts one queue without touching the global gate or other queues.
    QueueStartOne {
        queue_id: i64,
    },
    /// Stops one queue; its workers checkpoint and return to queued.
    QueueStopOne {
        queue_id: i64,
    },
    TaskMove {
        task_id: i64,
        target: MoveTarget,
    },
    /// `max_retries` -1 means infinite, 0 disables retries.
    QueueSetRetry {
        queue_id: i64,
        max_retries: i64,
        delay_seconds: u64,
    },
    /// `kind` is "none", "once" or "daily". Once: start/stop are Unix seconds.
    /// Daily: start/stop are local minutes since midnight and `days` is a
    /// Monday=bit0..Sunday=bit6 mask.
    QueueSetSchedule {
        queue_id: i64,
        kind: String,
        start: i64,
        stop: Option<i64>,
        days: u8,
    },
    /// `action` is a recorded setting only: none, notify, sleep, shutdown.
    QueueSetCompletion {
        queue_id: i64,
        action: String,
    },
    /// Sets a completion action with the opt-in model: destructive power
    /// actions require `confirm` and persist `countdown_seconds` (default 60)
    /// as a cancelable delay the daemon exposes through `CompletionStatus`.
    QueueSetCompletionAction {
        queue_id: i64,
        action: CompletionAction,
        #[serde(default)]
        confirm: bool,
        #[serde(default)]
        countdown_seconds: Option<u64>,
    },
    /// Reports the destructive actions currently inside their countdown.
    CompletionStatus,
    /// Cancels one queue's pending countdown before it executes.
    CompletionCancel {
        queue_id: i64,
    },
    QueueEvents {
        queue_id: i64,
    },
    /// Starts a site-grabber crawl in the daemon background; files it finds
    /// become HTTP tasks under `directory`.
    GrabSite {
        request_id: String,
        /// Serialised `xdm_grabber::Project`.
        project: serde_json::Value,
        directory: String,
        #[serde(default)]
        queue_id: Option<i64>,
        #[serde(default)]
        enqueue: bool,
    },
    /// Polls a crawl started with `GrabSite`.
    GrabStatus {
        request_id: String,
    },
    /// Saves a grabber project under a persistent name.
    GrabSave {
        name: String,
        project: serde_json::Value,
    },
    GrabProjects,
    GrabRemove {
        name: String,
    },
    CategoryList,
    /// Defines a user category or edits an existing (even built-in) one.
    CategorySave {
        name: String,
        #[serde(default)]
        extensions: Vec<String>,
        /// Absolute directory; empty keeps the default layout.
        #[serde(default)]
        destination: String,
        #[serde(default)]
        sites: Vec<String>,
    },
    /// Removes a user category; built-in names can never be removed.
    CategoryRemove {
        name: String,
    },
    /// Saved site logins, usernames and patterns only; never passwords.
    LoginList,
    /// Adds a site login or updates the existing entry with the same pattern
    /// and username. The password is stored at rest by the daemon only.
    LoginSave {
        pattern: String,
        username: String,
        password: String,
        enabled: bool,
    },
    LoginRemove {
        id: i64,
    },
    /// Supplies credentials for a task waiting in `auth_required`; with
    /// `remember` the login is also saved for the task's origin.
    TaskProvideLogin {
        task_id: i64,
        username: String,
        password: String,
        remember: bool,
    },
}

/// Mirrors `xdm_media::AudioFormat` with the identical serde shape so IPC stays
/// free of the media engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MediaAudioFormat {
    /// Stream-copy into the container that fits the source codec.
    #[serde(rename = "original")]
    Original,
    #[serde(rename = "mp3")]
    Mp3 { bitrate_kbps: u32 },
    #[serde(rename = "m4a")]
    M4a { bitrate_kbps: u32 },
    #[serde(rename = "opus")]
    Opus { bitrate_kbps: u32 },
}

/// What one clear-media job produces from a manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MediaMode {
    /// Video mux, optionally with an audio track and subtitle renditions.
    Video {
        video: usize,
        #[serde(default)]
        audio: Option<usize>,
        #[serde(default)]
        subtitles: Vec<usize>,
    },
    /// Audio-only job into a fixed or transcoded target format.
    Audio {
        track: usize,
        format: MediaAudioFormat,
    },
    /// Subtitle sidecar saved as `.srt` or `.vtt`.
    Subtitles {
        track: usize,
    },
}

impl MediaMode {
    /// The output extensions this mode produces; the destination must end in one.
    pub fn extensions(&self) -> &'static [&'static str] {
        match self {
            Self::Video { .. } => &["mp4", "mkv"],
            Self::Audio { format, .. } => match format {
                MediaAudioFormat::Original => &["m4a", "mp3", "opus", "webm"],
                MediaAudioFormat::Mp3 { bitrate_kbps } if (8..=320).contains(bitrate_kbps) => &["mp3"],
                MediaAudioFormat::M4a { bitrate_kbps } if (8..=320).contains(bitrate_kbps) => &["m4a"],
                MediaAudioFormat::Opus { bitrate_kbps } if (8..=320).contains(bitrate_kbps) => &["opus", "webm"],
                _ => &[],
            },
            Self::Subtitles { .. } => &["srt", "vtt"],
        }
    }

    /// Re-encode bitrates must sit in the range ffmpeg accepts; the
    /// `extensions` filter already encodes the same bound.
    pub fn valid(&self, extension: &str) -> bool {
        self.extensions()
            .iter()
            .any(|want| extension.eq_ignore_ascii_case(want))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MoveTarget {
    Up,
    Down,
    Top,
    Bottom,
    Index(usize),
}

/// A queue completion action. Sleep, hibernate, shutdown and restart are
/// destructive power states: they additionally require an explicit per-queue
/// opt-in (`confirm`) and a cancelable countdown before the daemon executes
/// them. `Program` launches a user-chosen executable with a literal argument
/// list (never through a shell).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CompletionAction {
    None,
    Notify,
    Sleep,
    Hibernate,
    Shutdown,
    Restart,
    /// Exits the daemon once the queue completes.
    Exit,
    Program {
        program: String,
        #[serde(default)]
        args: Vec<String>,
    },
}

impl CompletionAction {
    pub fn is_destructive(&self) -> bool {
        matches!(
            self,
            Self::Sleep | Self::Hibernate | Self::Shutdown | Self::Restart
        )
    }

    /// Bare simple name, so legacy clients keep reading plain strings;
    /// `Program` serializes as tagged JSON.
    pub fn encode(&self) -> String {
        match self {
            Self::None => "none".into(),
            Self::Notify => "notify".into(),
            Self::Sleep => "sleep".into(),
            Self::Hibernate => "hibernate".into(),
            Self::Shutdown => "shutdown".into(),
            Self::Restart => "restart".into(),
            Self::Exit => "exit".into(),
            Self::Program { .. } => serde_json::to_string(self).unwrap_or_else(|_| "none".into()),
        }
    }

    /// Decodes a stored setting: bare simple names or `Program` JSON.
    pub fn decode(text: &str) -> Option<Self> {
        match text {
            "none" => Some(Self::None),
            "notify" => Some(Self::Notify),
            "sleep" => Some(Self::Sleep),
            "hibernate" => Some(Self::Hibernate),
            "shutdown" => Some(Self::Shutdown),
            "restart" => Some(Self::Restart),
            "exit" => Some(Self::Exit),
            _ => serde_json::from_str::<Self>(text)
                .ok()
                .filter(|action| matches!(action, Self::Program { .. })),
        }
    }
}

/// Mirrors `xdm_http::Proxy` with the identical serde shape so IPC stays free of
/// the transfer engine.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ProxyChoice {
    #[default]
    System,
    Direct,
    Manual {
        http: Option<String>,
        https: Option<String>,
        #[serde(default)]
        bypass: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferList {
    pub version: u32,
    pub entries: Vec<ImportEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportEntry {
    pub url: String,
    pub filename: String,
    pub kind: String,
    pub priority: i64,
    pub connections: usize,
    pub rate_limit: Option<u64>,
    pub video: Option<usize>,
    pub audio: Option<usize>,
    pub category: String,
}

/// Live state of a daemon background crawl, reported by `GrabStatus`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GrabReport {
    pub request_id: String,
    /// running, done or failed.
    pub state: String,
    #[serde(default)]
    pub files: usize,
    #[serde(default)]
    pub pages: usize,
    #[serde(default)]
    pub errors: usize,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub task_ids: Vec<i64>,
}

/// A persisted grabber project returned by `GrabProjects`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrabProjectRecord {
    pub name: String,
    pub project: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSummary {
    pub id: i64,
    pub url: String,
    pub output: String,
    pub status: String,
    pub priority: i64,
    pub error: Option<String>,
    #[serde(default)]
    pub downloaded: Option<u64>,
    #[serde(default)]
    pub total: Option<u64>,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub rate_limit: Option<u64>,
    #[serde(default)]
    pub bytes_per_second: Option<u64>,
    #[serde(default)]
    pub eta_seconds: Option<u64>,
    #[serde(default)]
    pub connections: usize,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub media_video: Option<usize>,
    #[serde(default)]
    pub media_audio: Option<usize>,
    /// Live queue membership; 0 once a completed task has left its queue.
    #[serde(default = "main_queue_id")]
    pub queue_id: i64,
    /// Queue the task belonged to when it completed (history only).
    #[serde(default)]
    pub history_queue_id: Option<i64>,
    #[serde(default)]
    pub retry_count: i64,
    #[serde(default)]
    pub retry_at: Option<i64>,
    #[serde(default)]
    pub position: i64,
    /// Realm of the 401/407 challenge a task is waiting on in `auth_required`.
    #[serde(default)]
    pub auth_realm: Option<String>,
}

fn main_queue_id() -> i64 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueSummary {
    pub id: i64,
    pub name: String,
    pub running: bool,
    pub limit: usize,
    pub active: usize,
    pub queued: usize,
    pub paused: usize,
    pub failed: usize,
    pub complete: usize,
    #[serde(default)]
    pub started_alone: bool,
    #[serde(default)]
    pub retry_max: i64,
    #[serde(default)]
    pub retry_delay: u64,
    #[serde(default)]
    pub schedule_kind: String,
    #[serde(default)]
    pub schedule_start: i64,
    #[serde(default)]
    pub schedule_stop: Option<i64>,
    #[serde(default)]
    pub schedule_days: u8,
    #[serde(default)]
    pub completion_action: String,
    /// idle, running, completed, completed_with_failures or stopped.
    #[serde(default)]
    pub run_state: String,
    #[serde(default)]
    pub run_exec: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueEvent {
    pub id: i64,
    pub queue_id: i64,
    pub execution: i64,
    pub outcome: String,
    pub action: String,
    pub action_status: String,
    pub at: i64,
}

/// A saved site login as exposed to clients: username and pattern only, so
/// passwords never leave the daemon's protected store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginSummary {
    pub id: i64,
    pub pattern: String,
    pub username: String,
    pub enabled: bool,
}

/// A destructive completion action counting down before it executes,
/// reported by `CompletionStatus` and removed by `CompletionCancel`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionPending {
    pub queue_id: i64,
    /// sleep, hibernate, shutdown or restart.
    pub action: String,
    /// Unix instant at which the countdown expires.
    pub deadline_unix: i64,
    /// Seconds left before the action executes.
    pub seconds_remaining: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reply {
    pub version: u32,
    pub ok: bool,
    pub task_id: Option<i64>,
    pub tasks: Vec<TaskSummary>,
    pub error: Option<String>,
    #[serde(default)]
    pub queue_running: Option<bool>,
    #[serde(default)]
    pub queue_start_at: Option<i64>,
    #[serde(default)]
    pub queue_stop_at: Option<i64>,
    #[serde(default)]
    pub task_ids: Vec<i64>,
    #[serde(default)]
    pub transfer_list: Option<TransferList>,
    #[serde(default)]
    pub queues: Vec<QueueSummary>,
    #[serde(default)]
    pub queue_id: Option<i64>,
    #[serde(default)]
    pub queue_events: Vec<QueueEvent>,
    /// Current proxy mode; only set by `GetProxy` and `SetProxy` replies.
    #[serde(default)]
    pub proxy: Option<serde_json::Value>,
    /// Crawl state for `GrabSite`/`GrabStatus`.
    #[serde(default)]
    pub grab: Option<GrabReport>,
    /// Saved grabber projects for `GrabProjects`.
    #[serde(default)]
    pub grab_projects: Vec<GrabProjectRecord>,
    /// Category rules, built-ins first, then user categories.
    #[serde(default)]
    pub categories: Vec<xdm_categories::CategoryRule>,
    /// Destructive completion actions inside their cancelable countdown.
    #[serde(default)]
    pub completion_pending: Vec<CompletionPending>,
    /// Saved site logins for `LoginList`/`LoginSave`/`LoginRemove`; never
    /// carries passwords.
    #[serde(default)]
    pub logins: Vec<LoginSummary>,
}

impl Reply {
    pub fn accepted(task_id: i64) -> Self {
        Self {
            version: 1,
            ok: true,
            task_id: Some(task_id),
            tasks: Vec::new(),
            error: None,
            queue_running: None,
            queue_start_at: None,
            queue_stop_at: None,
            task_ids: Vec::new(),
            transfer_list: None,
            queues: Vec::new(),
            queue_id: None,
            queue_events: Vec::new(),
            proxy: None,
            grab: None,
            grab_projects: Vec::new(),
            categories: Vec::new(),
            completion_pending: Vec::new(),
            logins: Vec::new(),
        }
    }
    pub fn list(tasks: Vec<TaskSummary>) -> Self {
        Self {
            version: 1,
            ok: true,
            task_id: None,
            tasks,
            error: None,
            queue_running: None,
            queue_start_at: None,
            queue_stop_at: None,
            task_ids: Vec::new(),
            transfer_list: None,
            queues: Vec::new(),
            queue_id: None,
            queue_events: Vec::new(),
            proxy: None,
            grab: None,
            grab_projects: Vec::new(),
            categories: Vec::new(),
            completion_pending: Vec::new(),
            logins: Vec::new(),
        }
    }
    /// A crawl just registered with the daemon, still running.
    pub fn grab_running(request_id: &str) -> Self {
        let mut reply = Self::list(Vec::new());
        reply.grab = Some(GrabReport {
            request_id: request_id.to_string(),
            state: "running".into(),
            ..GrabReport::default()
        });
        reply
    }
    pub fn error(message: impl Into<String>) -> Self {        Self {
            version: 1,
            ok: false,
            task_id: None,
            tasks: Vec::new(),
            error: Some(message.into()),
            queue_running: None,
            queue_start_at: None,
            queue_stop_at: None,
            task_ids: Vec::new(),
            transfer_list: None,
            queues: Vec::new(),
            queue_id: None,
            queue_events: Vec::new(),
            proxy: None,
            grab: None,
            grab_projects: Vec::new(),
            categories: Vec::new(),
            completion_pending: Vec::new(),
            logins: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Paths {
    pub dir: PathBuf,
    pub token: String,
    pub database: PathBuf,
    pub socket: String,
}

impl Paths {
    pub fn open() -> io::Result<Self> {
        let dir = if let Ok(override_dir) = std::env::var("XDM_DATA_DIR") {
            PathBuf::from(override_dir)
        } else if cfg!(windows) {
            PathBuf::from(
                std::env::var("LOCALAPPDATA")
                    .or_else(|_| std::env::var("APPDATA"))
                    .map_err(io::Error::other)?,
            )
            .join("XDM")
        } else {
            PathBuf::from(std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| {
                format!("{}/.local/share", std::env::var("HOME").unwrap_or_default())
            }))
            .join("xdm")
        };
        fs::create_dir_all(&dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        }
        let token_file = dir.join("ipc-token");
        let token = if token_file.exists() {
            fs::read_to_string(&token_file)?.trim().to_owned()
        } else {
            let mut random = [0u8; 32];
            getrandom::getrandom(&mut random).map_err(|e| io::Error::other(e.to_string()))?;
            let token = random
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&token_file) {
                Ok(mut file) => {
                    file.write_all(token.as_bytes())?;
                    file.sync_all()?;
                    token
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    fs::read_to_string(&token_file)?.trim().to_owned()
                }
                Err(e) => return Err(e),
            }
        };
        if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid IPC token",
            ));
        }
        let digest = format!("{:x}", Sha256::digest(token.as_bytes()));
        let socket = if cfg!(windows) {
            format!("xdm-{}", &digest[..20])
        } else {
            dir.join("daemon.sock").to_string_lossy().into_owned()
        };
        Ok(Self {
            database: dir.join("tasks.sqlite3"),
            dir,
            token,
            socket,
        })
    }

    pub fn listen(&self) -> io::Result<Listener> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileTypeExt;
            if let Ok(metadata) = fs::symlink_metadata(&self.socket) {
                if !metadata.file_type().is_socket() {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "IPC path is not a socket",
                    ));
                }
                if self.connect().is_ok() {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        "daemon already running",
                    ));
                }
                fs::remove_file(&self.socket)?;
            }
        }
        let listener = if cfg!(windows) {
            ListenerOptions::new()
                .name(self.socket.as_str().to_ns_name::<GenericNamespaced>()?)
                .create_sync()?
        } else {
            ListenerOptions::new()
                .name(self.socket.as_str().to_fs_name::<GenericFilePath>()?)
                .create_sync()?
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.socket, fs::Permissions::from_mode(0o600))?;
        }
        Ok(listener)
    }

    pub fn connect(&self) -> io::Result<Stream> {
        if cfg!(windows) {
            Stream::connect(self.socket.as_str().to_ns_name::<GenericNamespaced>()?)
        } else {
            Stream::connect(self.socket.as_str().to_fs_name::<GenericFilePath>()?)
        }
    }

    pub fn request(&self, command: Command) -> io::Result<Reply> {
        let mut stream = self.connect()?;
        write_frame(
            &mut stream,
            &Request {
                version: 1,
                token: self.token.clone(),
                command,
            },
        )?;
        read_frame(&mut stream)
    }
}

pub fn write_frame<W: Write, T: Serialize>(writer: &mut W, value: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    if bytes.len() > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()
}

pub fn read_frame<R: Read, T: DeserializeOwned>(reader: &mut R) -> io::Result<T> {
    let mut prefix = [0u8; 4];
    reader.read_exact(&mut prefix)?;
    let length = u32::from_le_bytes(prefix) as usize;
    if length == 0 || length > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid frame size",
        ));
    }
    let mut bytes = vec![0u8; length];
    reader.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn read_native<R: Read, T: DeserializeOwned>(reader: &mut R) -> io::Result<T> {
    let mut prefix = [0u8; 4];
    reader.read_exact(&mut prefix)?;
    let length = u32::from_ne_bytes(prefix) as usize;
    if length == 0 || length > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid native frame size",
        ));
    }
    let mut bytes = vec![0u8; length];
    reader.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn write_native<W: Write, T: Serialize>(writer: &mut W, value: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    if bytes.len() > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    writer.write_all(&(bytes.len() as u32).to_ne_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()
}

pub fn token_file_exists(paths: &Paths) -> bool {
    File::open(paths.dir.join("ipc-token")).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frames_survive_short_reads() {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &Reply::accepted(42)).unwrap();
        let reply: Reply = read_frame(&mut bytes.as_slice()).unwrap();
        assert_eq!(reply.task_id, Some(42));
        bytes[0..4].copy_from_slice(&((MAX_FRAME + 1) as u32).to_le_bytes());
        assert!(read_frame::<_, Reply>(&mut bytes.as_slice()).is_err());
    }

    #[test]
    fn submit_request_keeps_binary_bodies() {
        let command = Command::SubmitRequest {
            request_id: "r1".into(),
            url: "http://example.invalid/".into(),
            output: "/tmp/x".into(),
            headers: vec![("Content-Type".into(), "application/x-www-form-urlencoded".into())],
            method: "POST".into(),
            body: Some(vec![0, 1, 255]),
            priority: 1,
            connections: 8,
        };
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &command).unwrap();
        let Command::SubmitRequest { body, method, .. } = read_frame(&mut bytes.as_slice()).unwrap()
        else {
            unreachable!()
        };
        assert_eq!(body.as_deref(), Some(&[0u8, 1, 255][..]));
        assert_eq!(method, "POST");
    }

    #[test]
    fn submit_media_job_round_trip_and_extension_rules() {
        let modes = [
            MediaMode::Video {
                video: 2,
                audio: Some(3),
                subtitles: vec![4, 5],
            },
            MediaMode::Audio {
                track: 1,
                format: MediaAudioFormat::Mp3 { bitrate_kbps: 96 },
            },
            MediaMode::Audio {
                track: 1,
                format: MediaAudioFormat::Original,
            },
            MediaMode::Subtitles { track: 4 },
        ];
        for mode in modes {
            let command = Command::SubmitMediaJob {
                request_id: "m1".into(),
                url: "http://a.test/master.m3u8".into(),
                output: "/dl/out.mp4".into(),
                headers: vec![("Cookie".into(), "sid=1".into())],
                mode: mode.clone(),
                priority: 1,
            };
            let mut bytes = Vec::new();
            write_frame(&mut bytes, &command).unwrap();
            let restored: Command = read_frame(&mut bytes.as_slice()).unwrap();
            let Command::SubmitMediaJob { mode: back, headers, .. } = restored else {
                unreachable!()
            };
            assert_eq!(back, mode);
            assert_eq!(headers, vec![("Cookie".to_owned(), "sid=1".to_owned())]);
            let value = serde_json::to_value(&mode).unwrap();
            let loose: MediaMode = serde_json::from_value(value).unwrap();
            assert_eq!(loose, mode);
        }
        // Missing audio and subtitles default to empty in older payloads.
        let legacy: MediaMode = serde_json::from_value(serde_json::json!({
            "kind": "video", "video": 0
        }))
        .unwrap();
        assert_eq!(
            legacy,
            MediaMode::Video {
                video: 0,
                audio: None,
                subtitles: Vec::new()
            }
        );
        // Extensions: video muxes, audio formats pin their container, and an
        // out-of-range bitrate admits nothing.
        let video = MediaMode::Video { video: 0, audio: None, subtitles: Vec::new() };
        assert!(video.valid("MP4") && video.valid("mkv") && !video.valid("srt"));
        let mp3 = MediaMode::Audio { track: 0, format: MediaAudioFormat::Mp3 { bitrate_kbps: 96 } };
        assert!(mp3.valid("mp3") && !mp3.valid("m4a"));
        assert!(!mp3.valid("opus"));
        let bad_bitrate = MediaMode::Audio { track: 0, format: MediaAudioFormat::Opus { bitrate_kbps: 400 } };
        assert!(!bad_bitrate.valid("opus"));
        let sidecar = MediaMode::Subtitles { track: 0 };
        assert!(sidecar.valid("srt") && sidecar.valid("vtt") && !sidecar.valid("mp3"));
    }

    #[test]
    fn proxy_choice_matches_engine_json_shape() {
        assert_eq!(serde_json::to_value(ProxyChoice::System).unwrap(), serde_json::json!({"mode": "system"}));
        assert_eq!(serde_json::to_value(ProxyChoice::Direct).unwrap(), serde_json::json!({"mode": "direct"}));
        assert_eq!(
            serde_json::to_value(ProxyChoice::Manual {
                http: Some("http://127.0.0.1:1".into()),
                https: None,
                bypass: vec!["10.0.0.0/8".into()],
            })
            .unwrap(),
            serde_json::json!({"mode": "manual", "http": "http://127.0.0.1:1", "https": null, "bypass": ["10.0.0.0/8"]})
        );
    }

    #[test]
    fn completion_actions_round_trip() {
        let program = Command::QueueSetCompletionAction {
            queue_id: 2,
            action: CompletionAction::Program {
                program: "/usr/local/bin/finish.sh".into(),
                args: vec!["hello world".into(), "a;b|>$HOME".into()],
            },
            confirm: false,
            countdown_seconds: Some(5),
        };
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &program).unwrap();
        let restored: Command = read_frame(&mut bytes.as_slice()).unwrap();
        assert_eq!(serde_json::to_value(&restored).unwrap(), serde_json::to_value(&program).unwrap());
        // Destructive opt-in + cancelable countdown status travel in the reply.
        let mut reply = Reply::list(Vec::new());
        reply.completion_pending = vec![CompletionPending {
            queue_id: 2,
            action: "shutdown".into(),
            deadline_unix: 1_700_000_060,
            seconds_remaining: 42,
        }];
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &reply).unwrap();
        let back: Reply = read_frame(&mut bytes.as_slice()).unwrap();
        assert_eq!(back.completion_pending[0].seconds_remaining, 42);        // Bare simple names and Program JSON round-trip through the store.
        assert_eq!(CompletionAction::decode(&CompletionAction::Shutdown.encode()), Some(CompletionAction::Shutdown));
        let program_text = CompletionAction::Program {
            program: "/bin/x".into(),
            args: Vec::new(),
        }
        .encode();
        assert_eq!(CompletionAction::decode(&program_text), Some(CompletionAction::Program { program: "/bin/x".into(), args: Vec::new() }));
        assert!(CompletionAction::decode("format-disk").is_none());
        assert!(CompletionAction::Shutdown.is_destructive());
        assert!(!CompletionAction::Exit.is_destructive());
        // Legacy commands keep their wire shape.
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &Command::QueueSetCompletion { queue_id: 1, action: "shutdown".into() }).unwrap();
        let Command::QueueSetCompletion { queue_id, action } = read_frame(&mut bytes.as_slice()).unwrap() else {
            unreachable!()
        };
        assert_eq!((queue_id, action.as_str()), (1, "shutdown"));
    }

    #[test]
    fn grab_and_category_commands_round_trip() {        let commands = vec![
            Command::GrabSite {
                request_id: "g1".into(),
                project: serde_json::json!({"start_url": "http://a.test/", "file_types": ["zip"]}),
                directory: "/tmp/site".into(),
                queue_id: Some(1),
                enqueue: true,
            },
            Command::GrabStatus { request_id: "g1".into() },
            Command::GrabSave {
                name: "docs".into(),
                project: serde_json::json!({"start_url": "http://a.test/"}),
            },
            Command::GrabProjects,
            Command::GrabRemove { name: "docs".into() },
            Command::CategoryList,
            Command::CategorySave {
                name: "Ebooks".into(),
                extensions: vec!["epub".into()],
                destination: "/lib/books".into(),
                sites: vec!["*.books.test".into()],
            },
            Command::CategoryRemove { name: "Ebooks".into() },
        ];
        for command in commands {
            let mut bytes = Vec::new();
            write_frame(&mut bytes, &command).unwrap();
            let restored: Command = read_frame(&mut bytes.as_slice()).unwrap();
            assert_eq!(serde_json::to_value(&command).unwrap(), serde_json::to_value(&restored).unwrap());
        }
        let mut reply = Reply::accepted(3);
        reply.grab = Some(GrabReport {
            request_id: "g1".into(),
            state: "running".into(),
            files: 2,
            pages: 1,
            errors: 0,
            error: None,
            task_ids: vec![7, 8],
        });
        reply.grab_projects = vec![GrabProjectRecord {
            name: "docs".into(),
            project: serde_json::json!({"start_url": "http://a.test/"}),
        }];
        reply.categories = vec![xdm_categories::CategoryRule {
            name: "Video".into(),
            extensions: Vec::new(),
            destination: "/video".into(),
            sites: Vec::new(),
        }];
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &reply).unwrap();
        let back: Reply = read_frame(&mut bytes.as_slice()).unwrap();
        assert_eq!(back.grab.as_ref().unwrap().task_ids, vec![7, 8]);
        assert_eq!(back.categories[0].destination, "/video");
    }

    #[test]
    fn login_commands_and_auth_realm_round_trip() {
        let commands = vec![
            Command::LoginList,
            Command::LoginSave {
                pattern: "https://*.example.com/files".into(),
                username: "alice".into(),
                password: "secret123".into(),
                enabled: true,
            },
            Command::LoginRemove { id: 3 },
            Command::TaskProvideLogin {
                task_id: 7,
                username: "alice".into(),
                password: "secret123".into(),
                remember: true,
            },
        ];
        for command in commands {
            let mut bytes = Vec::new();
            write_frame(&mut bytes, &command).unwrap();
            let restored: Command = read_frame(&mut bytes.as_slice()).unwrap();
            assert_eq!(serde_json::to_value(&command).unwrap(), serde_json::to_value(&restored).unwrap());
        }
        // Login summaries expose pattern and username, never the password.
        let mut reply = Reply::list(Vec::new());
        reply.logins = vec![LoginSummary {
            id: 5,
            pattern: "https://*.example.com/files".into(),
            username: "alice".into(),
            enabled: false,
        }];
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &reply).unwrap();
        let back: Reply = read_frame(&mut bytes.as_slice()).unwrap();
        assert_eq!(back.logins, reply.logins);
        assert!(!serde_json::to_string(&back).unwrap().contains("secret123"));
        // Older clients read task rows without the auth field.
        let legacy = serde_json::from_str::<TaskSummary>(
            r#"{"id":1,"url":"http://a/","output":"/o/a","status":"queued","priority":1,
            "error":null}"#,
        )
        .unwrap();
        assert_eq!(legacy.auth_realm, None);
        let mut waiting = legacy.clone();
        waiting.auth_realm = Some("xdm-test".into());
        assert!(serde_json::to_string(&waiting).unwrap().contains("xdm-test"));
    }
}

