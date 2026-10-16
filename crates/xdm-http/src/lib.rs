//! Independent HTTP transfer engine. No browser or GUI dependencies.

use reqwest::{Method, StatusCode};
use reqwest::blocking::{Client, Response};
use reqwest::header::{
    ACCEPT_ENCODING, CONTENT_DISPOSITION, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE, ETAG,
    HeaderMap, HeaderName, HeaderValue, IF_RANGE, LAST_MODIFIED, PROXY_AUTHENTICATE, RANGE, RETRY_AFTER,
    WWW_AUTHENTICATE, AUTHORIZATION,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::error::Error;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use xdm_categories::valid_filename;

mod governor;
pub use governor::{
    DEFAULT_HOST_CONNECTIONS, Governor, GovernorSnapshot, Lease, RateBucket, host_of,
    pattern_matches, resolve,
};
use governor::pace;
mod diagnostics;
mod digest;
mod sysproxy;
pub use diagnostics::{Classified, ErrorKind, classify, classify_reqwest};
pub use sysproxy::{Pac, SystemProxy, status as system_proxy_status};
use diagnostics::classified;
pub use digest::{Answer as AuthAnswer, AuthState, Login};

pub type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Debug, Clone)]
pub struct Options {
    pub url: String,
    /// Origin credentials answered to 401 challenges (Basic or Digest, N11).
    pub login: Option<Login>,
    /// Credentials for a manual proxy, sent as `Proxy-Authorization: Basic` (N23).
    pub proxy_login: Option<Login>,
    pub output: PathBuf,
    pub connections: usize,
    pub restart: bool,
    pub headers: Vec<(String, String)>,
    /// HTTP method replayed for every request of this transfer. Never silently downgraded.
    pub method: String,
    /// Replayable request body, sent unchanged with every request when present.
    pub body: Option<Vec<u8>>,
    pub bytes_per_second: Option<u64>,
    pub proxy: Proxy,
    pub stop: Arc<AtomicBool>,
    /// Absolute directory that holds `.part`/`.part.meta` instead of the destination's
    /// directory (N38). The completed file is published at `output` afterwards.
    pub temp_dir: Option<PathBuf>,
    /// Sets the completed file's modification time from Last-Modified (N41).
    pub set_file_time: bool,
    /// Largest byte range one request may ask for. Some media hosts (googlevideo) answer 403 to an
    /// open-ended range and to ranges above about 10 MB, so the probe asks for one byte and the
    /// transfer is cut into segments no larger than this. `None` keeps the normal behavior.
    pub max_request_bytes: Option<u64>,
}

/// Connection route used by every request of a transfer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum Proxy {
    /// Proxy environment variables (`http_proxy`, `https_proxy`, `all_proxy`, `no_proxy`).
    #[default]
    System,
    /// Direct connections, ignoring the environment.
    Direct,
    /// Proxy URLs per target scheme: `http://`, `https://`, `socks4://`, `socks5://` or
    /// `socks5h://` (resolve names at the proxy). Credentials go in the URL user info.
    /// `bypass` entries are host names (matching subdomains), IP addresses or CIDR ranges.
    Manual {
        http: Option<String>,
        https: Option<String>,
        #[serde(default)]
        bypass: Vec<String>,
    },
}

impl Default for Options {
    fn default() -> Self {
        Self {
            url: String::new(),
            login: None,
            proxy_login: None,
            output: PathBuf::new(),
            connections: 8,
            restart: false,
            headers: Vec::new(),
            method: "GET".into(),
            body: None,
            bytes_per_second: None,
            proxy: Proxy::System,
            stop: Arc::new(AtomicBool::new(false)),
            temp_dir: None,
            set_file_time: false,
            max_request_bytes: None,
        }
    }
}

/// Resume policy of a saved segmented transfer (N13). With
/// `ignore_modification_date`, a resume may proceed when only Last-Modified
/// changed while a strong ETag and the length still match.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResumePolicy {
    pub ignore_modification_date: bool,
    /// Treat a redirect to another host of the same site (same scheme and path, same last two host
    /// labels) as the same destination. Media CDNs such as googlevideo answer one address from
    /// different cache hosts; object identity is still enforced by length, validators and Content-Range.
    pub same_site_redirects: bool,
}

/// Whether two effective URLs name the same destination under `policy`.
fn same_destination(a: &str, b: &str, policy: &ResumePolicy) -> bool {
    if a == b {
        return true;
    }
    if !policy.same_site_redirects {
        return false;
    }
    let (Ok(a), Ok(b)) = (reqwest::Url::parse(a), reqwest::Url::parse(b)) else {
        return false;
    };
    let site = |url: &reqwest::Url| {
        let host = url.host_str().unwrap_or("").to_ascii_lowercase();
        let labels: Vec<&str> = host.split('.').collect();
        (labels.len() >= 2).then(|| labels[labels.len() - 2..].join("."))
    };
    a.scheme() == b.scheme() && a.path() == b.path() && site(&a).is_some() && site(&a) == site(&b)
}

/// Response facts of the request that probed a transfer (N14). Delivered
/// through the naming callback of `download_ex` so callers can choose a
/// task's final filename before bytes stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseMetadata {
    pub effective_url: String,
    pub content_disposition: Option<String>,
    pub content_length: Option<u64>,
    /// Content-Type of the serving response (server-name reconciliation).
    pub content_type: Option<String>,
}

/// Shared controls and live counters for a single HTTP transfer.
#[derive(Debug)]
pub struct TransferControl {
    id: u64,
    task: RateBucket,
    attached: OnceLock<Attached>,
    downloaded: AtomicU64,
    total: AtomicU64,
    total_known: AtomicBool,
    sample: Mutex<Sample>,
    size_unverified: AtomicBool,
    live: OnceLock<LiveSegments>,
}

/// Reader of a running segmented transfer's per-connection positions.
struct LiveSegments(Box<dyn Fn() -> Vec<(u64, u64, u64)> + Send + Sync>);

impl std::fmt::Debug for LiveSegments {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LiveSegments")
    }
}

/// Shared limits a daemon attaches to a transfer.
#[derive(Debug)]
struct Attached {
    governor: Arc<Governor>,
    host_bucket: Arc<RateBucket>,
}

/// Error text of a transfer stopped because the usage quota was reached.
pub const QUOTA_REACHED: &str = "download quota reached";

#[derive(Debug)]
struct Sample {
    at: Instant,
    bytes: u64,
    speed: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub downloaded: u64,
    pub total: Option<u64>,
    pub bytes_per_second: u64,
    pub eta_seconds: Option<u64>,
}

impl TransferControl {
    pub fn new(rate: Option<u64>) -> Result<Self> {
        if rate == Some(0) {
            return Err(err("rate limit must be greater than zero"));
        }
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Ok(Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            task: RateBucket::new(rate),
            attached: OnceLock::new(),
            downloaded: AtomicU64::new(0),
            total: AtomicU64::new(0),
            total_known: AtomicBool::new(false),
            sample: Mutex::new(Sample {
                at: Instant::now(),
                bytes: 0,
                speed: 0,
            }),
            size_unverified: AtomicBool::new(false),
            live: OnceLock::new(),
        })
    }

    /// Segments `(start, inclusive end, next byte)` of the running transfer,
    /// including bytes not yet checkpointed; `None` when it is not segmented.
    pub fn live_segments(&self) -> Option<Vec<(u64, u64, u64)>> {
        self.live.get().map(|live| (live.0)())
    }

    /// True when the last transfer declared no Content-Length and ended at EOF, so
    /// its completeness could not be verified.
    pub fn size_unverified(&self) -> bool {
        self.size_unverified.load(Ordering::Relaxed)
    }

    pub fn set_rate(&self, rate: Option<u64>) -> Result<()> {
        if rate == Some(0) {
            return Err(err("rate limit must be greater than zero"));
        }
        self.task.set(rate);
        Ok(())
    }

    fn initialize(&self, done: u64, total: Option<u64>) {
        self.size_unverified.store(false, Ordering::Relaxed);
        self.downloaded.store(done, Ordering::Relaxed);
        self.total.store(total.unwrap_or(0), Ordering::Relaxed);
        self.total_known.store(total.is_some(), Ordering::Relaxed);
        if let Ok(mut sample) = self.sample.lock() {
            *sample = Sample {
                at: Instant::now(),
                bytes: done,
                speed: 0,
            };
        }
    }

    pub fn progress(&self) -> Progress {
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

    /// Puts this transfer under the process-wide governor: global and per-host
    /// caps keyed by the host of `url`, connection leases and the usage quota.
    pub fn attach(&self, governor: Arc<Governor>, url: &str) {
        let host_bucket = governor.host_bucket(&host_of(url));
        let _ = self.attached.set(Attached { governor, host_bucket });
    }

    fn governor(&self) -> Option<&Arc<Governor>> {
        self.attached.get().map(|attached| &attached.governor)
    }

    fn quota_hit(&self) -> bool {
        self.attached
            .get()
            .is_some_and(|attached| attached.governor.exceeded())
    }

    /// Counts bytes that arrived: progress and the usage quota.
    #[inline]
    fn record(&self, bytes: usize) {
        self.downloaded.fetch_add(bytes as u64, Ordering::Relaxed);
        if let Some(attached) = self.attached.get() {
            attached.governor.account(bytes as u64);
        }
    }

    #[inline]
    fn wait(&self, bytes: usize, stop: &AtomicBool) -> Result<()> {
        let (host, global) = match self.attached.get() {
            Some(attached) => (Some(&*attached.host_bucket), Some(&attached.governor.global)),
            None => (None, None),
        };
        if self.task.unlimited()
            && host.is_none_or(RateBucket::unlimited)
            && global.is_none_or(RateBucket::unlimited)
        {
            return Ok(());
        }
        pace([Some(&self.task), host, global], self.id, bytes, stop)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Segment {
    start: u64,
    end: u64,    // inclusive
    cursor: u64, // next byte to write
    #[serde(skip)]
    active: bool,
    #[serde(skip)]
    claimed: u64, // bytes before this are promised to the active worker
    #[serde(skip)]
    written: u64, // bytes before this are in the file, possibly not yet checkpointed
    #[serde(skip)]
    received: u64, // bytes before this arrived from the network; display only, never persisted
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Checkpoint {
    version: u32,
    url: String,
    effective_url: String,
    context_sha256: String,
    length: u64,
    etag: Option<String>,
    last_modified: Option<String>,
    segments: Vec<Segment>,
}

/// Read the latest durable byte count without opening a network connection.
pub fn checkpoint_progress(output: &Path) -> Option<(u64, u64)> {
    checkpoint_progress_in(output, None)
}

/// Segments `(start, inclusive end, cursor)` of a task's checkpoint.
pub fn checkpoint_segments_in(output: &Path, temp: Option<&Path>) -> Option<Vec<(u64, u64, u64)>> {
    let file = File::open(meta_path(output, temp)).ok()?;
    let state: Checkpoint = serde_json::from_reader(file).ok()?;
    Some(state.segments.iter().take(256).map(|s| (s.start, s.end, s.cursor)).collect())
}

/// Like `checkpoint_progress` for a task whose partials live in `temp`.
pub fn checkpoint_progress_in(output: &Path, temp: Option<&Path>) -> Option<(u64, u64)> {
    let path = meta_path(output, temp);
    let file = File::open(path).ok()?;
    let state: Checkpoint = serde_json::from_reader(file).ok()?;
    let done = state.segments.iter().try_fold(0u64, |sum, segment| {
        if segment.cursor < segment.start || segment.cursor > segment.end.saturating_add(1) {
            return None;
        }
        sum.checked_add(segment.cursor - segment.start)
    })?;
    if done > state.length {
        return None;
    }
    Some((done, state.length))
}

#[derive(Debug)]
struct Probe {
    effective_url: String,
    length: Option<u64>,
    ranges: bool,
    etag: Option<String>,
    last_modified: Option<String>,
    /// Content-Disposition of the serving response (N14 naming).
    disposition: Option<String>,
    content_type: Option<String>,
    /// First response, kept so its body is adopted instead of requesting the resource again.
    response: Option<Response>,
    /// Byte offset at which the adopted body begins.
    offset: u64,
}

impl Probe {
    /// Response facts for the N14 naming decision, taken before any byte streams.
    fn metadata(&self) -> ResponseMetadata {
        ResponseMetadata {
            effective_url: self.effective_url.clone(),
            content_disposition: self.disposition.clone(),
            content_length: self.length,
            content_type: self.content_type.clone(),
        }
    }
}

/// Largest request body replayed with each request of a transfer.
pub const MAX_BODY: usize = 1024 * 1024;
/// A running segment is split only if its unclaimed remainder is at least this large.
const MIN_SPLIT: u64 = 1024 * 1024;
const RETRIES: usize = 4;
/// Per-worker receive buffer flushed with one positional write.
const WRITE_BUFFER: usize = 256 * 1024;
/// Minimum spacing of periodic checkpoints; pause, stop and segment ends always checkpoint.
const CHECKPOINT_INTERVAL: Duration = Duration::from_millis(500);

/// Idle period after which a transfer abandons a connection that has delivered no
/// bytes and reopens from its cursor, counting toward the retry budget (N31).
/// The same budget bounds connect and response headers of every request.
/// Minimum connect/idle budget on Windows, above its ~2 s refused-connect latency.
const WINDOWS_REFUSED_CONNECT_FLOOR: Duration = Duration::from_secs(4);

pub const DEFAULT_STALL_TIMEOUT: Duration = Duration::from_secs(15);

/// Replayable request identity shared by the probe and every transfer request.
#[derive(Debug, Clone)]
struct Request {
    url: String,
    method: Method,
    body: Option<Vec<u8>>,
    auth: Option<Arc<AuthState>>,
}

impl Request {
    fn new(options: &Options) -> Result<Self> {
        let method = Method::from_bytes(options.method.trim().to_ascii_uppercase().as_bytes())
            .map_err(|_| err(format!("invalid HTTP method {:?}", options.method)))?;
        if [Method::HEAD, Method::CONNECT, Method::TRACE, Method::OPTIONS].contains(&method) {
            return Err(err(format!("HTTP {method} cannot download a resource")));
        }
        if let Some(body) = &options.body {
            if method == Method::GET {
                return Err(err("a request body requires a method such as POST"));
            }
            if body.len() > MAX_BODY {
                return Err(err("request body exceeds the replay limit"));
            }
        }
        Ok(Self {
            url: options.url.clone(),
            method,
            body: options.body.clone(),
            auth: options.login.clone().map(|login| Arc::new(AuthState::new(login))),
        })
    }

    /// Sends one request and answers a 401 challenge with the saved login. Every
    /// request, ranged ones included, carries its own `Authorization`; a repeated 401
    /// is returned to the caller instead of being retried.
    fn send(
        &self,
        client: &Client,
        range: Option<String>,
        validator: Option<&str>,
    ) -> reqwest::Result<Response> {
        let Some(auth) = &self.auth else {
            return self.send_once(client, range, validator);
        };
        let origin = reqwest::Url::parse(&self.url).ok().map(|url| url.origin());
        let mut sent = auth.has_scheme();
        let mut response = self.send_once(client, range.clone(), validator)?;
        for _ in 0..2 {
            if response.status() != StatusCode::UNAUTHORIZED {
                if sent && response.status().is_success() {
                    auth.confirm();
                }
                return Ok(response);
            }
            // Credentials belong to the transfer's origin only.
            if origin.as_ref() != Some(&response.url().origin()) {
                return Ok(response);
            }
            let values: Vec<String> = response
                .headers()
                .get_all(WWW_AUTHENTICATE)
                .iter()
                .filter_map(|value| value.to_str().ok().map(str::to_owned))
                .collect();
            match auth.answer(&values, response.url().as_str(), sent) {
                digest::Answer::GiveUp => return Ok(response),
                digest::Answer::Retry => {
                    sent = true;
                    response = self.send_once(client, range.clone(), validator)?;
                }
            }
        }
        if sent && response.status().is_success() {
            auth.confirm();
        }
        Ok(response)
    }

    /// The typed 401/407 error of a response, with the reason credentials were withheld.
    fn challenge(&self, response: &Response) -> Option<Box<dyn Error + Send + Sync>> {
        let error = challenge_error(response)?;
        let refusal = self.auth.as_ref().and_then(|auth| auth.refusal());
        match (error.downcast::<AuthRequired>(), refusal) {
            (Ok(mut required), Some(note)) if required.0.status == 401 => {
                required.0.note = Some(note);
                Some(required)
            }
            (Ok(required), _) => Some(required),
            (Err(other), _) => Some(other),
        }
    }

    fn send_once(
        &self,
        client: &Client,
        range: Option<String>,
        validator: Option<&str>,
    ) -> reqwest::Result<Response> {
        let mut request = client
            .request(self.method.clone(), &self.url)
            .header(ACCEPT_ENCODING, "identity");
        if let Some(range) = range {
            request = request.header(RANGE, range);
        }
        if let Some(value) = validator {
            request = request.header(IF_RANGE, value);
        }
        if let Some(body) = &self.body {
            request = request.body(body.clone());
        }
        if let Some(value) = self.auth.as_ref().and_then(|auth| auth.header(self.method.as_str())) {
            request = request.header(AUTHORIZATION, value);
        }
        request.send()
    }
}

/// Headers that may follow a cross-origin redirect. The HTTP client drops
/// credentials and cookies itself whenever the host or port changes.
const REDIRECT_SAFE: [&str; 7] = [
    "user-agent",
    "referer",
    "accept",
    "accept-language",
    "cookie",
    "authorization",
    "proxy-authorization",
];

fn err(message: impl Into<String>) -> Box<dyn Error + Send + Sync> {
    message.into().into()
}

/// A 401/407 authentication challenge offered by the server or a proxy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthChallenge {
    pub status: u16,
    pub scheme: String,
    pub realm: Option<String>,
    /// Why the stored credentials were not offered (for example Basic over plain http).
    pub note: Option<String>,
}

/// Error type carrying the challenge of a failed probe so callers can
/// downcast and offer credentials once instead of retrying blindly.
#[derive(Debug, Clone)]
pub struct AuthRequired(pub AuthChallenge);

impl std::fmt::Display for AuthRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "authentication required: HTTP {} {}",
            self.0.status, self.0.scheme
        )?;
        if let Some(realm) = &self.0.realm {
            write!(f, " realm \"{realm}\"")?;
        }
        if let Some(note) = &self.0.note {
            write!(f, ": {note}")?;
        }
        Ok(())
    }
}

impl Error for AuthRequired {}

/// The 401/407 challenge carried by a failed transfer, when present.
pub fn auth_challenge(error: &(dyn Error + Send + Sync + 'static)) -> Option<AuthChallenge> {
    error
        .downcast_ref::<AuthRequired>()
        .map(|required| required.0.clone())
}

/// A server that accepted byte ranges during probing later answered a range request
/// with a response that cannot be assembled safely: a wrong status, Content-Range or
/// length (N05). Workers are stopped, the checkpoint is kept resumable, and the
/// caller must choose a full restart instead of completing with mixed data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeUnreliable {
    /// Byte offset the invalid response was opened at.
    pub at: u64,
    /// What was wrong with the response.
    pub reason: String,
}

impl std::fmt::Display for RangeUnreliable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "server stopped honoring byte ranges at byte {}: {}; partial data kept, rerun with --restart",
            self.at, self.reason
        )
    }
}

impl Error for RangeUnreliable {}

/// The typed range failure of an aborted transfer, when present (N05).
pub fn range_unreliable(error: &(dyn Error + Send + Sync + 'static)) -> Option<RangeUnreliable> {
    error.downcast_ref::<RangeUnreliable>().cloned()
}

fn unreliable(at: u64, reason: impl Into<String>) -> Box<dyn Error + Send + Sync> {
    Box::new(RangeUnreliable {
        at,
        reason: reason.into(),
    })
}

/// Filenames from `Content-Disposition` (RFC 6266): the RFC 5987 extended
/// `filename*` value wins when present and decodable as UTF-8, otherwise the
/// plain `filename` parameter.
pub fn disposition_filename(disposition: Option<&str>) -> Option<String> {
    let mut legacy = None;
    let mut extended = None;
    for parameter in split_parameters(disposition?) {
        let Some((name, value)) = parameter.trim().split_once('=') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        if name == "filename*" {
            if let Some((charset, coded)) = value.trim().split_once("''") {
                if charset.trim().eq_ignore_ascii_case("utf-8") {
                    extended = percent_decode(coded.trim());
                }
            }
        } else if name == "filename" {
            legacy = Some(unquote(value.trim()));
        }
    }
    extended.or(legacy).filter(|value| !value.is_empty())
}

/// Preserves quoted runs while splitting `;` parameters, so arguments like
/// `filename="a;b.zip"` stay whole.
fn split_parameters(value: &str) -> Vec<&str> {
    let mut parameters = Vec::new();
    let mut quoted = false;
    let mut start = 0;
    for (at, ch) in value.char_indices() {
        match ch {
            '"' => quoted = !quoted,
            ';' if !quoted => {
                parameters.push(&value[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    parameters.push(&value[start..]);
    parameters
}

/// Straps surrounding quotes off and unfolds the `\"` escapes inside them.
fn unquote(value: &str) -> String {
    if let Some(rest) = value.strip_prefix('"') {
        let inner = rest.strip_suffix('"').unwrap_or(rest);
        return inner
            .replace("\\\\", "\\")
            .replace("\\\"", "\"");
    }
    value.to_owned()
}

fn percent_decode(value: &str) -> Option<String> {
    percent_encoding::percent_decode_str(value)
        .decode_utf8()
        .ok()
        .filter(|decoded| !decoded.is_empty())
        .map(|decoded| decoded.into_owned())
}

/// The URL's last percent-decoded path segment, without query or fragment; a
/// redirected probe reports its final URL so the file takes the real name.
pub fn url_filename(url: &str) -> Option<String> {
    let no_fragment = url.split('#').next().unwrap_or(url);
    let no_query = no_fragment.split('?').next().unwrap_or(no_fragment);
    let segment = no_query.rsplit('/').next().unwrap_or("");
    percent_decode(segment)
}

/// Cross-platform filename built from an untrusted server value: path
/// components are stripped, Windows-invalid characters replaced, trailing
/// dots and spaces removed and reserved device names disambiguated (N14).
pub fn sanitize_filename(raw: &str) -> Option<String> {
    let name = raw.rsplit(['/', '\\']).next().unwrap_or(raw).trim();
    if name.is_empty() {
        return None;
    }
    let mut cleaned: String = name
        .chars()
        .map(|c| if c.is_control() || "\\/:<>\"|?*".contains(c) { '_' } else { c })
        .collect();
    while cleaned.ends_with(['.', ' ']) {
        cleaned.pop();
    }
    if cleaned.is_empty() {
        return None;
    }
    if !valid_filename(&cleaned) {
        // Reserved device names (CON, COM1.., NUL) never name a file on
        // Windows; keep the intended stem by underlining it.
        let at = cleaned.rfind('.').filter(|&at| at > 0);
        let repaired = match at {
            Some(at) => format!("{}_.{}", &cleaned[..at], &cleaned[at + 1..]),
            None => format!("{cleaned}_"),
        };
        if !valid_filename(&repaired) {
            return None;
        }
        cleaned = repaired;
    }
    (cleaned.len() <= 240).then_some(cleaned)
}

/// The server-chosen filename of a task (N14): Content-Disposition first,
/// else the final URL's last segment; `None` keeps the provisional name,
/// including when a sanitization loses every meaningful character.
pub fn server_filename(metadata: &ResponseMetadata, provisional: &str) -> Option<String> {
    let candidates = [
        disposition_filename(metadata.content_disposition.as_deref()),
        url_filename(&metadata.effective_url),
    ];
    candidates
        .into_iter()
        .flatten()
        .find_map(|raw| sanitize_filename(&raw))
        .filter(|chosen| *chosen != provisional)
}

/// Message of a task whose server answered with a web page for a binary file.
pub const HTML_INSTEAD_OF_FILE: &str = "server returned an HTML page instead of a file";

/// Content-Type media type to its canonical extension, and the other extensions that
/// already satisfy it.
const MIME_EXTENSIONS: &[(&str, &str, &[&str])] = &[
    ("application/zip", "zip", &[]),
    ("application/x-zip-compressed", "zip", &[]),
    ("application/pdf", "pdf", &[]),
    ("video/mp4", "mp4", &["m4v"]),
    ("video/x-matroska", "mkv", &["webm"]),
    ("audio/mpeg", "mp3", &[]),
    ("audio/mp4", "m4a", &[]),
    ("audio/x-m4a", "m4a", &[]),
    ("image/png", "png", &[]),
    ("image/jpeg", "jpg", &["jpeg", "jpe"]),
    ("application/vnd.microsoft.portable-executable", "exe", &[]),
    ("application/x-msdownload", "exe", &[]),
    ("application/x-msi", "msi", &[]),
    ("application/x-iso9660-image", "iso", &[]),
    ("application/x-7z-compressed", "7z", &[]),
    ("application/vnd.rar", "rar", &[]),
    ("application/x-rar-compressed", "rar", &[]),
    ("application/gzip", "gz", &["tgz"]),
    ("application/x-gzip", "gz", &["tgz"]),
    ("application/x-tar", "tar", &[]),
];

/// Extensions of server-side pages that never name the real payload type.
const DYNAMIC_EXTENSIONS: &[&str] = &[
    "php", "php3", "php4", "php5", "asp", "aspx", "jsp", "jspx", "cgi", "pl", "do", "action",
    "cfm", "html", "htm", "shtml", "axd", "ashx",
];

/// True when the first bytes of a body open like a web page.
fn looks_like_html(head: &[u8]) -> bool {
    let text: Vec<u8> = head
        .iter()
        .copied()
        .skip_while(|b| b.is_ascii_whitespace() || *b == 0xEF || *b == 0xBB || *b == 0xBF)
        .take(15)
        .map(|b| b.to_ascii_lowercase())
        .collect();
    text.starts_with(b"<!doctype html") || text.starts_with(b"<html") || text.starts_with(b"<head") || text.starts_with(b"<body")
}

/// A text/html answer for a request whose name targets a binary file is suspect; the
/// body is sniffed before any byte is kept.
fn html_guard(content_type: Option<&str>, effective_url: &str, output: &Path) -> bool {
    if content_type.map(media_type).as_deref() != Some("text/html") {
        return false;
    }
    let binary = |name: &str| {
        name.rfind('.')
            .is_some_and(|at| is_binary_extension(&name[at + 1..].to_ascii_lowercase()))
    };
    url_filename(effective_url).is_some_and(|n| binary(&n))
        || output.file_name().is_some_and(|n| binary(&n.to_string_lossy()))
}

fn is_binary_extension(ext: &str) -> bool {
    MIME_EXTENSIONS
        .iter()
        .any(|(_, canonical, others)| *canonical == ext || others.contains(&ext))
}

fn media_type(content_type: &str) -> String {
    content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase()
}

/// N14 plus MIME reconciliation: the final name of a server-named task. `None` keeps the
/// provisional name. Only a missing or dynamic-page extension (`.php`, `.asp`, ...) is
/// fixed from Content-Type (`a.php` + application/zip becomes `a.zip`); a real extension
/// is never rewritten.
pub fn reconcile_server_name(metadata: &ResponseMetadata, provisional: &str) -> Option<String> {
    let name = server_filename(metadata, provisional).unwrap_or_else(|| provisional.to_owned());
    let extension = name
        .rfind('.')
        .filter(|at| *at > 0 && *at + 1 < name.len())
        .map(|at| name[at + 1..].to_ascii_lowercase());
    let mime = metadata.content_type.as_deref().map(media_type).unwrap_or_default();
    let mut result = name.clone();
    if let Some((_, wanted, _)) = MIME_EXTENSIONS.iter().find(|(known, _, _)| *known == mime) {
        match extension.as_deref() {
            None => result = format!("{name}.{wanted}"),
            Some(ext) if DYNAMIC_EXTENSIONS.contains(&ext) => {
                let stem = name.rfind('.').unwrap_or(name.len());
                result = format!("{}.{wanted}", &name[..stem]);
            }
            Some(_) => {}
        }
    }
    let result = sanitize_filename(&result)?;
    (result != provisional).then_some(result)
}

/// Parses the first challenge of a WWW-Authenticate or Proxy-Authenticate
/// header: `Basic realm="xdm-test"`, `Bearer`, or a comma list of schemes.
fn parse_challenge(header: &str) -> Option<(String, Option<String>)> {
    let header = header.trim();
    let (scheme, rest) = match header.find(char::is_whitespace) {
        Some(at) => (&header[..at], header[at..].trim()),
        None => (header, ""),
    };
    if scheme.is_empty()
        || !scheme
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return None;
    }
    let mut realm = None;
    for parameter in rest.split(',') {
        let Some((name, value)) = parameter.split_once('=') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("realm") {
            let value = value.trim().trim_matches('"');
            realm = (!value.is_empty()).then(|| value.to_string());
            break;
        }
    }
    Some((scheme.to_owned(), realm))
}

/// Turns a 401/407 response into a typed error so a failed transfer surfaces
/// its challenge; any other status returns `None`.
fn challenge_error(response: &Response) -> Option<Box<dyn Error + Send + Sync>> {
    let (header_name, status) = match response.status() {
        StatusCode::UNAUTHORIZED => (WWW_AUTHENTICATE, StatusCode::UNAUTHORIZED),
        StatusCode::PROXY_AUTHENTICATION_REQUIRED => {
            (PROXY_AUTHENTICATE, StatusCode::PROXY_AUTHENTICATION_REQUIRED)
        }
        _ => return None,
    };
    let Some(value) = response.headers().get(header_name).and_then(|v| v.to_str().ok()) else {
        // A proxy that answers 407 without naming a scheme still needs credentials.
        return (status == StatusCode::PROXY_AUTHENTICATION_REQUIRED).then(|| {
            Box::new(AuthRequired(AuthChallenge {
                note: None,
                status: 407,
                scheme: "Basic".into(),
                realm: None,
            })) as Box<dyn Error + Send + Sync>
        });
    };
    let (scheme, realm) = parse_challenge(value)?;
    Some(Box::new(AuthRequired(AuthChallenge {
        note: None,
        status: status.as_u16(),
        scheme,
        realm,
    })))
}

/// A CONNECT tunnel refused with 407 reaches the client as a transport error without
/// the proxy's headers, so the realm is unknown.
fn proxy_auth_failure(error: &reqwest::Error) -> Option<Box<dyn Error + Send + Sync>> {
    let text = describe(error).to_ascii_lowercase();
    // hyper-util words a tunnel's 407 as "proxy authorization required".
    (text.contains("proxy authentication required") || text.contains("proxy authorization required"))
        .then(|| {
            Box::new(AuthRequired(AuthChallenge {
                note: None,
                status: 407,
                scheme: "Basic".into(),
                realm: None,
            })) as Box<dyn Error + Send + Sync>
        })
}

/// Strict Content-Range parser. Returns inclusive start/end and complete size.
pub fn parse_content_range(value: &str) -> Option<(u64, u64, u64)> {
    let rest = value.trim().strip_prefix("bytes ")?;
    let (interval, total) = rest.split_once('/')?;
    let (start, end) = interval.split_once('-')?;
    let (start, end, total) = (
        start.parse::<u64>().ok()?,
        end.parse::<u64>().ok()?,
        total.parse::<u64>().ok()?,
    );
    (total > 0 && start <= end && end < total).then_some((start, end, total))
}

#[cfg(test)]
fn split_segments(length: u64, workers: usize) -> Vec<Segment> {
    split_segments_capped(length, workers, None)
}

/// Like `split_segments`; with a cap, there are at least enough segments that none exceeds it.
fn split_segments_capped(length: u64, workers: usize, cap: Option<u64>) -> Vec<Segment> {
    if length == 0 {
        return Vec::new();
    }
    let mut count = workers.max(1).min(16).min(length as usize);
    if let Some(cap) = cap.filter(|cap| *cap > 0) {
        count = count.max(length.div_ceil(cap).min(8192) as usize).min(length as usize);
    }
    let base = length / count as u64;
    let extra = length % count as u64;
    let mut start = 0;
    (0..count)
        .map(|i| {
            let size = base + u64::from((i as u64) < extra);
            let segment = Segment {
                start,
                end: start + size - 1,
                cursor: start,
                active: false,
                claimed: start,
                written: start,
                received: start,
            };
            start += size;
            segment
        })
        .collect()
}

fn header(response: &Response, key: reqwest::header::HeaderName) -> Option<String> {
    response
        .headers()
        .get(key)?
        .to_str()
        .ok()
        .map(str::to_owned)
}

fn identity_encoded(response: &Response) -> bool {
    header(response, CONTENT_ENCODING).is_none_or(|value| value.eq_ignore_ascii_case("identity"))
}

/// Partial-file name inside a separate temporary directory: a digest of the final path
/// keeps equal names of different folders apart (N38).
/// Canonical form of a destination for hashing: lexically clean (no `.`, doubled or
/// trailing separators) with the parent resolved when it exists.
fn normalized_output(output: &Path) -> PathBuf {
    let clean: PathBuf = output.components().collect();
    match (clean.parent(), clean.file_name()) {
        (Some(parent), Some(name)) => fs::canonicalize(parent)
            .map(|parent| parent.join(name))
            .unwrap_or(clean),
        _ => clean,
    }
}

fn temp_base(output: &Path, temp: &Path) -> PathBuf {
    let digest = format!("{:x}", Sha256::digest(normalized_output(output).to_string_lossy().as_bytes()));
    let name: String = output
        .file_name()
        .map(|n| n.to_string_lossy().chars().take(100).collect())
        .unwrap_or_default();
    temp.join(format!("{}-{name}", &digest[..16]))
}

/// The `.part` and `.part.meta` paths of `output`, beside it or inside `temp`.
pub fn partial_paths(output: &Path, temp: Option<&Path>) -> (PathBuf, PathBuf) {
    (part_path(output, temp), meta_path(output, temp))
}

fn part_path(output: &Path, temp: Option<&Path>) -> PathBuf {
    let base = temp.map_or_else(|| output.to_path_buf(), |dir| temp_base(output, dir));
    PathBuf::from(format!("{}.part", base.display()))
}

fn meta_path(output: &Path, temp: Option<&Path>) -> PathBuf {
    let base = temp.map_or_else(|| output.to_path_buf(), |dir| temp_base(output, dir));
    PathBuf::from(format!("{}.part.meta", base.display()))
}

fn store_checkpoint(file: &File, path: &Path, state: &Checkpoint) -> Result<()> {
    file.sync_data()?;
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
    {
        let mut writer = File::create(&tmp)?;
        serde_json::to_writer(&mut writer, state)?;
        writer.flush()?;
        writer.sync_all()?;
    }
    fs::rename(tmp, path)?;
    Ok(())
}

fn validators_match(saved: &Checkpoint, current: &Probe, policy: &ResumePolicy) -> bool {
    if Some(saved.length) != current.length || !same_destination(&saved.effective_url, &current.effective_url, policy) {
        return false;
    }
    let etag = saved
        .etag
        .as_ref()
        .filter(|v| !v.starts_with("W/"))
        .zip(current.etag.as_ref());
    let modified = saved
        .last_modified
        .as_ref()
        .zip(current.last_modified.as_ref());
    match etag {
        // A strong ETag that changed always refuses, policy or not (N13).
        Some((a, b)) if a != b => false,
        // A still-matching strong ETag alone protects the content identity;
        // only then may the modification date be ignored (N13).
        Some(_) => policy.ignore_modification_date || modified.is_none_or(|(a, b)| a == b),
        // Without a strong ETag the date is the only signal left and stays binding:
        // disabling the date check alone must never splice changed bytes.
        None => modified.is_some_and(|(a, b)| a == b),
    }
}

/// Sizes the partial file. Linux allocates real blocks so concurrent range writes do not
/// allocate under the file lock; other systems, or filesystems without fallocate, get a
/// plain (possibly sparse) length.
fn preallocate(file: &File, length: u64) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        // SAFETY: fallocate only reads the descriptor and sizes we pass.
        let previous = file.metadata()?.len();
        // Preflight: never start allocating what the filesystem cannot hold.
        // SAFETY: fstatvfs only writes into the zeroed struct we own.
        let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstatvfs(file.as_raw_fd(), &mut stats) } == 0 {
            let available = (stats.f_bavail as u64).saturating_mul(stats.f_frsize as u64);
            if length.saturating_sub(previous) > available {
                return Err(std::io::Error::from_raw_os_error(libc::ENOSPC));
            }
        }
        // SAFETY: fallocate only reads the descriptor and sizes we pass.
        if unsafe { libc::fallocate(file.as_raw_fd(), 0, 0, length as libc::off_t) } == 0 {
            return Ok(());
        }
        // A full disk or a size limit is a real answer, not a reason to fall back to sparse.
        // ext4 keeps blocks from a failed fallocate, so give them back before reporting.
        let error = std::io::Error::last_os_error();
        if matches!(error.raw_os_error(), Some(libc::ENOSPC | libc::EFBIG | libc::EDQUOT)) {
            let _ = file.set_len(previous);
            return Err(error);
        }
    }
    file.set_len(length)
}

/// Preflight of the final volume when partials live elsewhere (N38): the completed
/// file must fit where it will be published, not only where it is assembled. Skipped
/// when both directories share a device, where the partial's own preallocation already
/// accounts for the space.
fn ensure_final_space(output: &Path, temp: &Path, length: u64) -> Result<()> {
    let dir = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::MetadataExt;
        if let (Ok(a), Ok(b)) = (fs::metadata(dir), fs::metadata(temp)) {
            if a.dev() == b.dev() {
                return Ok(());
            }
        }
        if let Ok(path) = CString::new(dir.as_os_str().as_bytes()) {
            // SAFETY: statvfs only writes into the zeroed struct we own.
            let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
            if unsafe { libc::statvfs(path.as_ptr(), &mut stats) } == 0 {
                let available = (stats.f_bavail as u64).saturating_mul(stats.f_frsize as u64);
                if length > available {
                    return Err(std::io::Error::from_raw_os_error(libc::ENOSPC).into());
                }
            }
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let volume = |path: &Path| fs::canonicalize(path).ok().and_then(|p| p.components().next().map(|c| c.as_os_str().to_owned()));
        if volume(dir) == volume(temp) {
            return Ok(());
        }
        let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let mut available = 0u64;
        // SAFETY: `wide` is NUL-terminated and the out pointer is a valid u64; the other
        // two optional outputs are null.
        let ok = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut available,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if ok != 0 && length > available {
            return Err(std::io::Error::new(
                std::io::ErrorKind::StorageFull,
                "not enough space on the destination volume",
            )
            .into());
        }
    }
    let _ = (dir, temp, length);
    Ok(())
}

/// Rewrites storage failures into actionable messages; other errors pass through.
fn explain_storage(error: Box<dyn Error + Send + Sync>, output: &Path) -> Box<dyn Error + Send + Sync> {
    let Some(io) = error.downcast_ref::<std::io::Error>() else {
        return error;
    };
    let reason = match io.kind() {
        std::io::ErrorKind::StorageFull | std::io::ErrorKind::QuotaExceeded => {
            "not enough disk space (or quota) for the download"
        }
        std::io::ErrorKind::FileTooLarge => {
            "the file exceeds the filesystem or process file-size limit"
        }
        // Windows reports an impossible SetEndOfFile size as ERROR_INVALID_PARAMETER (87) or
        // ERROR_FILE_TOO_LARGE (223) instead of a storage kind.
        _ if cfg!(windows) && matches!(io.raw_os_error(), Some(87 | 223)) => {
            "the file exceeds the filesystem or process file-size limit"
        }
        _ => return error,
    };
    err(format!(
        "{reason} at {}; the partial download was kept for resuming after space is freed",
        output.display()
    ))
}

/// Without this, exceeding a file-size limit (RLIMIT_FSIZE) kills the process with SIGXFSZ
/// instead of returning EFBIG that can be reported and recovered from.
fn ignore_file_size_signal() {
    #[cfg(unix)]
    {
        static ONCE: std::sync::Once = std::sync::Once::new();
        // SAFETY: setting a signal's disposition to ignore installs no handler code.
        ONCE.call_once(|| unsafe {
            libc::signal(libc::SIGXFSZ, libc::SIG_IGN);
        });
    }
}

fn write_at(file: &File, offset: u64, mut data: &[u8]) -> std::io::Result<()> {
    let mut position = offset;
    while !data.is_empty() {
        #[cfg(unix)]
        let written = {
            use std::os::unix::fs::FileExt;
            file.write_at(data, position)?
        };
        #[cfg(windows)]
        let written = {
            use std::os::windows::fs::FileExt;
            file.seek_write(data, position)?
        };
        if written == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::WriteZero));
        }
        position += written as u64;
        data = &data[written..];
    }
    Ok(())
}

fn retryable(status: StatusCode) -> bool {
    status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}


/// Applies the proxy mode (and manual-proxy credentials) to a client builder. Shared
/// with the daemon's own HTTP checks so every request honors one routing policy.
pub fn apply_proxy(
    mut builder: reqwest::blocking::ClientBuilder,
    proxy: &Proxy,
    login: Option<&Login>,
) -> Result<reqwest::blocking::ClientBuilder> {
    let manual = |builder: reqwest::blocking::ClientBuilder,
                  http: &Option<String>,
                  https: &Option<String>,
                  bypass: &[String],
                  login: Option<&Login>| {
        let mut builder = builder.no_proxy();
        let bypass = reqwest::NoProxy::from_string(&bypass.join(","));
        let configure = |url: &str, secure: bool| -> Result<reqwest::Proxy> {
            let socks = url.starts_with("socks");
            let (url, credentials) = match login {
                Some(login) if socks => (with_userinfo(url, login), None),
                other => (url.to_owned(), other),
            };
            let proxy = if secure { reqwest::Proxy::https(&url)? } else { reqwest::Proxy::http(&url)? };
            let proxy = match credentials {
                Some(login) => proxy.basic_auth(&login.username, &login.password),
                None => proxy,
            };
            Ok(proxy.no_proxy(bypass.clone()))
        };
        if let Some(url) = http {
            builder = builder.proxy(configure(url, false)?);
        }
        if let Some(url) = https {
            builder = builder.proxy(configure(url, true)?);
        }
        Ok::<_, Box<dyn Error + Send + Sync>>(builder)
    };
    match proxy {
        Proxy::System => match sysproxy::resolve() {
            sysproxy::Resolved::Environment | sysproxy::Resolved::Direct => {}
            sysproxy::Resolved::Settings(found) => {
                if found.pac.is_some() {
                    return Err(classified(
                        ErrorKind::ProxyConfig,
                        format!(
                            "{}; choose a manual proxy or direct connections in the proxy settings",
                            found.describe()
                        ),
                    ));
                }
                // Stored credentials belong to the manual proxy only; an OS-discovered
                // proxy never receives them.
                builder = manual(builder, &found.http, &found.https, &found.bypass, None)?;
            }
        },
        Proxy::Direct => builder = builder.no_proxy(),
        Proxy::Manual { http, https, bypass } => builder = manual(builder, http, https, bypass, login)?,
    }
    Ok(builder)
}

/// URL with percent-encoded user info (SOCKS credentials travel in the URL).
fn with_userinfo(url: &str, login: &Login) -> String {
    use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
    let (scheme, rest) = url.split_once("://").unwrap_or(("socks5", url));
    let rest = rest.rsplit_once('@').map_or(rest, |(_, host)| host);
    format!(
        "{scheme}://{}:{}@{rest}",
        utf8_percent_encode(&login.username, NON_ALPHANUMERIC),
        utf8_percent_encode(&login.password, NON_ALPHANUMERIC)
    )
}

fn client(
    headers: &[(String, String)],
    method: &Method,
    proxy: &Proxy,
    proxy_login: Option<&Login>,
    stall_timeout: Duration,
) -> Result<Client> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let key = HeaderName::from_bytes(name.as_bytes())?;
        if [
            "host",
            "range",
            "if-range",
            "accept-encoding",
            "content-length",
            "transfer-encoding",
        ]
        .contains(&key.as_str())
        {
            return Err(err(format!(
                "header {name} is managed by the transfer engine"
            )));
        }
        map.append(key, HeaderValue::from_str(value)?);
    }
    let scoped = map.keys().any(|key| !REDIRECT_SAFE.contains(&key.as_str()));
    let contextual = !headers.is_empty();
    let keep_method = *method != Method::GET;
    let policy = reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= 10 {
            return attempt.stop();
        }
        if keep_method
            && [
                StatusCode::MOVED_PERMANENTLY,
                StatusCode::FOUND,
                StatusCode::SEE_OTHER,
            ]
            .contains(&attempt.status())
        {
            return attempt.error("redirect would replace the request method with GET");
        }
        let previous = attempt.previous().last();
        if contextual
            && previous.is_some_and(|url| url.scheme() == "https")
            && attempt.url().scheme() != "https"
        {
            return attempt.stop();
        }
        if scoped
            && attempt
                .previous()
                .first()
                .is_some_and(|first| first.origin() != attempt.url().origin())
        {
            return attempt.stop();
        }
        attempt.follow()
    });
    // The blocking client has no `read_timeout` (that builder is async-only), but its
    // `timeout` is re-armed for every body read, so it doubles as the N31 socket idle
    // timeout; it also bounds connect and response headers, where no byte flows either.
    // The connect budget ends just before the overall one so a connection attempt that
    // never completes is told apart from a server that stalls after connecting (N17).
    // Windows retries a refused SYN to loopback (about 2 s) before reporting
    // WSAECONNREFUSED, so a shorter budget would call a closed port a timeout.
    let stall_timeout = if cfg!(windows) {
        stall_timeout.max(WINDOWS_REFUSED_CONNECT_FLOOR)
    } else {
        stall_timeout
    };
    let connect = stall_timeout
        .saturating_sub(Duration::from_millis(250).min(stall_timeout / 4))
        .max(Duration::from_millis(1));
    let builder = Client::builder()
        .timeout(stall_timeout)
        .connect_timeout(connect)
        .default_headers(map)
        .referer(false)
        .redirect(policy);
    Ok(apply_proxy(builder, proxy, proxy_login)?.build()?)
}

fn retry_after(response: &Response) -> Option<Duration> {
    header(response, RETRY_AFTER)?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

/// Waits before a retry, honoring a capped Retry-After and returning early when paused.
fn wait_retry(attempt: usize, after: Option<Duration>, stop: &AtomicBool) -> Result<()> {
    let delay = after
        .map(|value| value.min(Duration::from_secs(30)))
        .unwrap_or_else(|| Duration::from_millis(200 * (1 << attempt.min(5))));
    let until = Instant::now() + delay;
    while Instant::now() < until {
        if stop.load(Ordering::Relaxed) {
            return Err(err("paused"));
        }
        thread::sleep((until - Instant::now()).min(Duration::from_millis(50)));
    }
    Ok(())
}

/// Error text including its causes, which reqwest keeps out of its own message.
fn describe(error: &reqwest::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text = format!("{text}: {cause}");
        source = cause.source();
    }
    text
}

fn retry_send(
    client: &Client,
    request: &Request,
    range: Option<String>,
    stop: &AtomicBool,
) -> Result<Response> {
    let mut last = String::new();
    let mut kind = ErrorKind::Other;
    for attempt in 0..=RETRIES {
        let after = match request.send(client, range.clone(), None) {
            Ok(r) if retryable(r.status()) => {
                last = format!("HTTP {}", r.status());
                kind = ErrorKind::HttpStatus;
                retry_after(&r)
            }
            Ok(r) => return Ok(r),
            Err(e) if e.is_redirect() => return Err(err(describe(&e))),
            Err(e) => {
                if let Some(proxy_auth) = proxy_auth_failure(&e) {
                    return Err(proxy_auth);
                }
                last = describe(&e);
                kind = classify_reqwest(&e);
                None
            }
        };
        if attempt < RETRIES {
            wait_retry(attempt, after, stop)?;
        }
    }
    Err(classified(kind, format!("request retries exhausted: {last}")))
}

/// Opens the resource once, from `offset` to its end, and keeps the response for adoption.
/// A server that ignores or garbles the range is asked once more for the whole representation.
fn probe(client: &Client, request: &Request, offset: u64, stop: &AtomicBool, bounded: bool) -> Result<Probe> {
    if bounded {
        return probe_bounded(client, request, offset, stop);
    }
    let response = retry_send(client, request, Some(format!("bytes={offset}-")), stop)?;
    let status = response.status();
    let disposition = header(&response, CONTENT_DISPOSITION);
    let content_type = header(&response, reqwest::header::CONTENT_TYPE);
    if status == StatusCode::PARTIAL_CONTENT && identity_encoded(&response) {
        let declared = header(&response, CONTENT_LENGTH).map(|v| v.parse::<u64>().ok());
        if let Some((start, end, length)) = header(&response, CONTENT_RANGE)
            .as_deref()
            .and_then(parse_content_range)
        {
            if start == offset
                && end + 1 == length
                && declared.is_none_or(|value| value == Some(length - offset))
            {
                return Ok(Probe {
                    effective_url: response.url().to_string(),
                    length: Some(length),
                    ranges: true,
                    etag: header(&response, ETAG),
                    last_modified: header(&response, LAST_MODIFIED),
                    disposition,
                    content_type,
                    response: Some(response),
                    offset,
                });
            }
        }
    }
    let response = match status {
        StatusCode::OK => response,
        StatusCode::PARTIAL_CONTENT | StatusCode::RANGE_NOT_SATISFIABLE => {
            drop(response);
            let full = retry_send(client, request, None, stop)?;
            if full.status() != StatusCode::OK {
                return Err(request.challenge(&full)
                    .unwrap_or_else(|| err(format!("download returned HTTP {}", full.status()))));
            }
            full
        }
        status => {
            return Err(request.challenge(&response)
                .unwrap_or_else(|| err(format!("probe failed: HTTP {status}"))))
        }
    };
    Ok(Probe {
        effective_url: response.url().to_string(),
        length: header(&response, CONTENT_LENGTH).and_then(|s| s.parse().ok()),
        ranges: false,
        etag: header(&response, ETAG),
        last_modified: header(&response, LAST_MODIFIED),
        disposition,
        content_type,
        response: Some(response),
        offset: 0,
    })
}

/// Probe for hosts that refuse open-ended ranges: one byte at `offset` yields the total length from
/// Content-Range. The body is not adopted; segments fetch their own bounded ranges. A host that does not
/// answer 206 is probed the normal way afterwards.
fn probe_bounded(client: &Client, request: &Request, offset: u64, stop: &AtomicBool) -> Result<Probe> {
    let response = retry_send(client, request, Some(format!("bytes={offset}-{offset}")), stop)?;
    if response.status() == StatusCode::PARTIAL_CONTENT && identity_encoded(&response) {
        if let Some((start, end, length)) = header(&response, CONTENT_RANGE).as_deref().and_then(parse_content_range) {
            if start == offset && end == offset {
                return Ok(Probe {
                    effective_url: response.url().to_string(),
                    length: Some(length),
                    ranges: true,
                    etag: header(&response, ETAG),
                    last_modified: header(&response, LAST_MODIFIED),
                    disposition: header(&response, CONTENT_DISPOSITION),
                    content_type: header(&response, reqwest::header::CONTENT_TYPE),
                    response: None,
                    offset,
                });
            }
        }
    }
    drop(response);
    probe(client, request, offset, stop, false)
}

/// Renames without ever replacing an existing destination. Linux uses
/// `renameat2(RENAME_NOREPLACE)`; other systems refuse rather than risk replacing a file
/// that appeared after the last check.
fn rename_noreplace(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let (from, to) = (
            CString::new(from.as_os_str().as_bytes())?,
            CString::new(to.as_os_str().as_bytes())?,
        );
        // SAFETY: both pointers are valid NUL-terminated strings for the call's duration.
        let result = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                libc::AT_FDCWD,
                from.as_ptr(),
                libc::AT_FDCWD,
                to.as_ptr(),
                1u32, // RENAME_NOREPLACE
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (from, to);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "no-replace rename is unavailable on this system",
        ))
    }
}

fn appeared(output: &Path, part: &Path) -> Box<dyn Error + Send + Sync> {
    err(format!(
        "{} appeared during the download; the completed data was kept at {}",
        output.display(),
        part.display()
    ))
}

/// Flushes the directory entry of a freshly installed file (Unix).
fn sync_parent(path: &Path) {
    #[cfg(unix)]
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        if let Ok(dir) = File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Removal of the source after a successful install is housekeeping, never a failure.
fn discard(path: &Path) {
    if let Err(error) = fs::remove_file(path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            eprintln!("warning: could not remove {}: {error}", path.display());
        }
    }
}

/// Installs the completed file without replacing anything created at the destination meanwhile.
fn publish(part: &Path, output: &Path) -> Result<()> {
    match fs::hard_link(part, output) {
        Ok(()) => {
            discard(part);
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(appeared(output, part)),
        // Filesystems without hard links (or another volume): a no-replace rename, then
        // copy + fsync + no-clobber publish.
        Err(_) => match rename_noreplace(part, output) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(appeared(output, part)),
            Err(_) => copy_publish(part, output),
        },
    }
}

/// Cross-volume publish (N38): the data is copied beside the destination under a unique
/// staging name and synced, installed without replacing anything, the directory is
/// synced, and only then is the partial removed. Any failure leaves the partial intact,
/// the destination untouched and removes only the staging file this call created.
fn copy_publish(part: &Path, output: &Path) -> Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let staging = PathBuf::from(format!(
        "{}.xdm-copy-{}-{nanos}-{}",
        output.display(),
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let copied = (|| -> std::io::Result<()> {
        let mut source = File::open(part)?;
        let modified = source.metadata()?.modified().ok();
        let mut target = OpenOptions::new().write(true).create_new(true).open(&staging)?;
        std::io::copy(&mut source, &mut target)?;
        if let Some(modified) = modified {
            let _ = target.set_modified(modified);
        }
        target.sync_all()
    })();
    if let Err(error) = copied {
        // Only a staging file this call created exists; create_new failing means it is not ours.
        if error.kind() != std::io::ErrorKind::AlreadyExists {
            let _ = fs::remove_file(&staging);
        }
        return Err(err(format!(
            "copying the finished download to {} failed ({error}); the data was kept at {}",
            output.display(),
            part.display()
        )));
    }
    let installed = match fs::hard_link(&staging, output) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(()),
        Err(_) => rename_noreplace(&staging, output).map_err(|_| ()),
    };
    if installed.is_err() {
        let _ = fs::remove_file(&staging);
        return Err(appeared(output, part));
    }
    // After a rename the staging name is already gone; after a link it is removed here.
    let _ = fs::remove_file(&staging);
    sync_parent(output);
    discard(part);
    Ok(())
}

/// Parses an HTTP-date (IMF-fixdate, RFC 850 or asctime) into a system time.
pub fn parse_http_date(value: &str) -> Option<std::time::SystemTime> {
    const MONTHS: [&str; 12] =
        ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let text = value.trim();
    let tokens: Vec<&str> = text
        .split(|c: char| c == ' ' || c == ',' || c == '-')
        .filter(|t| !t.is_empty())
        .collect();
    let month = |t: &str| MONTHS.iter().position(|m| t.len() >= 3 && t[..3].eq_ignore_ascii_case(m));
    let (day, mon, year, clock) = match tokens.as_slice() {
        // Sun, 06 Nov 1994 08:49:37 GMT  |  Sunday, 06-Nov-94 08:49:37 GMT
        [_, d, m, y, t, tz] if tz.eq_ignore_ascii_case("GMT") => (*d, month(m)?, *y, *t),
        // Sun Nov  6 08:49:37 1994
        [_, m, d, t, y] => (*d, month(m)?, *y, *t),
        _ => return None,
    };
    let day: i64 = day.parse().ok()?;
    let mut y: i64 = year.parse().ok()?;
    if year.len() == 2 {
        y += if y < 70 { 2000 } else { 1900 };
    }
    let mut parts = clock.split(':');
    let (h, mi, sec): (i64, i64, i64) = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    if parts.next().is_some() || !(1..=31).contains(&day) || h > 23 || mi > 59 || sec > 60 || !(1970..=9999).contains(&y) {
        return None;
    }
    let m = mon as i64 + 1;
    // Days from civil (Howard Hinnant).
    let yy = if m <= 2 { y - 1 } else { y };
    let era = yy / 400;
    let yoe = yy - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let seconds = days * 86400 + h * 3600 + mi * 60 + sec;
    u64::try_from(seconds)
        .ok()
        .map(|s| std::time::UNIX_EPOCH + Duration::from_secs(s))
}

/// The modification time to stamp on a finished file, from a valid Last-Modified value.
fn server_time(
    enabled: bool,
    last_modified: Option<&str>,
) -> Option<std::time::SystemTime> {
    enabled.then(|| last_modified.and_then(parse_http_date)).flatten()
}

/// Stamps the still-open file; hard links, renames and copies carry the time along.
fn stamp(file: &File, time: Option<std::time::SystemTime>) {
    if let Some(time) = time {
        let _ = file.set_modified(time);
    }
}

/// State shared by the range workers of one transfer.
struct Shared {
    client: Client,
    request: Request,
    file: File,
    meta: PathBuf,
    state: Mutex<Checkpoint>,
    stop: Arc<AtomicBool>,
    control: Arc<TransferControl>,
    adopted: Mutex<Option<(u64, Response)>>,
    /// Set when the server refuses extra connections; no new requests are opened afterward.
    refused: Mutex<Option<StatusCode>>,
    /// Last periodic checkpoint; bounds disk syncs for all workers together.
    checkpointed: Mutex<Instant>,
    /// N13 resume policy, applied to the validators of every range answer.
    policy: ResumePolicy,
    /// First terminal worker failure (N05 `RangeUnreliable`, pause, I/O error).
    aborted: Mutex<Option<Box<dyn Error + Send + Sync>>>,
    /// Host whose connection budget the workers draw from.
    host: String,
    /// Lease of the probe connection, handed to the first worker that asks.
    probe_lease: Mutex<Option<Lease>>,
}

enum Outcome {
    Done,
    Refused(StatusCode),
}

impl Shared {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Checkpoint>> {
        self.state.lock().map_err(|_| err("checkpoint lock poisoned"))
    }

    fn refused(&self) -> bool {
        self.refused.lock().map(|r| r.is_some()).unwrap_or(true)
    }

    /// Records the first terminal failure and stops every worker. Written bytes stay
    /// in the checkpoint, so a resumable state is left behind for pause and a safe
    /// full restart alike.
    fn abort(&self, error: Box<dyn Error + Send + Sync>) {
        if let Ok(mut slot) = self.aborted.lock() {
            let _ = slot.get_or_insert(error);
        }
        self.stop.store(true, Ordering::Relaxed);
    }

    fn take_failure(&self) -> Option<Box<dyn Error + Send + Sync>> {
        self.aborted.lock().ok().and_then(|mut slot| slot.take())
    }

    fn persist(&self, start: u64, cursor: u64) -> Result<()> {
        let mut guard = self.lock()?;
        let index = find(&guard, start)?;
        guard.segments[index].cursor = cursor;
        // One sync covers every worker's flushed bytes, so record them all, not only the caller's.
        for segment in &mut guard.segments {
            segment.cursor = segment.cursor.max(segment.written);
        }
        store_checkpoint(&self.file, &self.meta, &guard)
    }

    /// Whether a periodic checkpoint may run now. Each checkpoint syncs the whole file,
    /// so workers share one interval instead of syncing per megabyte each.
    fn checkpoint_due(&self) -> bool {
        let Ok(mut last) = self.checkpointed.lock() else {
            return true;
        };
        if last.elapsed() < CHECKPOINT_INTERVAL {
            return false;
        }
        *last = Instant::now();
        true
    }

    fn release(&self, start: u64) -> Result<()> {
        let mut guard = self.lock()?;
        let index = find(&guard, start)?;
        guard.segments[index].active = false;
        Ok(())
    }

    /// Claims the first idle unfinished segment, or splits the largest unclaimed remainder.
    fn claim(&self) -> Result<Option<u64>> {
        if self.refused() {
            return Ok(None);
        }
        let mut guard = self.lock()?;
        if let Some(segment) = guard
            .segments
            .iter_mut()
            .find(|s| !s.active && s.cursor <= s.end)
        {
            segment.active = true;
            segment.claimed = segment.cursor;
            segment.written = segment.cursor;
            return Ok(Some(segment.start));
        }
        let Some((index, from)) = guard
            .segments
            .iter()
            .enumerate()
            .filter(|(_, s)| s.active)
            .map(|(i, s)| (i, s.claimed.max(s.cursor)))
            .max_by_key(|&(i, from)| (guard.segments[i].end + 1).saturating_sub(from))
        else {
            return Ok(None);
        };
        let remaining = (guard.segments[index].end + 1).saturating_sub(from);
        if remaining < MIN_SPLIT {
            return Ok(None);
        }
        let middle = from + remaining / 2;
        let end = guard.segments[index].end;
        guard.segments[index].end = middle - 1;
        guard.segments.insert(
            index + 1,
            Segment {
                start: middle,
                end,
                cursor: middle,
                active: true,
                claimed: middle,
                written: middle,
                received: middle,
            },
        );
        store_checkpoint(&self.file, &self.meta, &guard)?;
        Ok(Some(middle))
    }

    /// Waits for a connection lease before a worker claims work, so a worker
    /// blocked on the host budget never holds a segment hostage.
    fn lease(&self) -> Result<Option<Lease>> {
        let Some(governor) = self.control.governor() else {
            return Ok(None);
        };
        if let Some(lease) = self.probe_lease.lock().ok().and_then(|mut slot| slot.take()) {
            return Ok(Some(lease));
        }
        governor.acquire(&self.host, &self.stop).map(Some)
    }

    fn take_adopted(&self, offset: u64) -> Option<Response> {
        let mut slot = self.adopted.lock().ok()?;
        if slot.as_ref().is_some_and(|(at, _)| *at == offset) {
            slot.take().map(|(_, response)| response)
        } else {
            None
        }
    }
}

fn find(state: &Checkpoint, start: u64) -> Result<usize> {
    state
        .segments
        .binary_search_by_key(&start, |s| s.start)
        .map_err(|_| err("segment disappeared from checkpoint"))
}

fn worker(shared: Arc<Shared>) -> Result<()> {
    loop {
        if shared.stop.load(Ordering::Relaxed) {
            shared.abort(err("paused"));
            return Ok(());
        }
        if shared.control.quota_hit() {
            shared.abort(err(QUOTA_REACHED));
            return Ok(());
        }
        let _lease = match shared.lease() {
            Ok(lease) => lease,
            Err(error) => {
                shared.abort(error);
                return Ok(());
            }
        };
        let Some(start) = shared.claim().unwrap_or_else(|error| {
            shared.abort(error);
            None
        }) else {
            return Ok(());
        };
        match transfer_segment(&shared, start) {
            Ok(Outcome::Done) => {}
            Ok(Outcome::Refused(status)) => {
                if let Err(error) = shared.release(start) {
                    shared.abort(error);
                    return Ok(());
                }
                if let Ok(mut refused) = shared.refused.lock() {
                    refused.get_or_insert(status);
                }
                return Ok(());
            }
            // The first failure aborts the siblings instead of racing them to the join.
            Err(error) => {
                shared.abort(error);
                return Ok(());
            }
        }
    }
}

fn transfer_segment(shared: &Shared, start: u64) -> Result<Outcome> {
    let mut failures = 0;
    loop {
        if shared.stop.load(Ordering::Relaxed) {
            return Err(err("paused"));
        }
        let (cursor, end, total, validator, etag, last_modified, effective_url) = {
            let guard = shared.lock()?;
            let segment = &guard.segments[find(&guard, start)?];
            let validator = guard
                .etag
                .as_ref()
                .filter(|s| !s.starts_with("W/"))
                .or(guard.last_modified.as_ref())
                .cloned();
            (
                segment.cursor,
                segment.end,
                guard.length,
                validator,
                guard.etag.clone(),
                guard.last_modified.clone(),
                guard.effective_url.clone(),
            )
        };
        if cursor > end {
            shared.release(start)?;
            return Ok(Outcome::Done);
        }
        // An adopted probe body runs to the end of the resource and may absorb following segments.
        let (mut response, open_ended) = match shared.take_adopted(cursor) {
            Some(response) => (response, true),
            None => {
                let response = match shared.request.send(
                    &shared.client,
                    Some(format!("bytes={cursor}-{end}")),
                    validator.as_deref(),
                ) {
                    Ok(r) => r,
                    Err(e) if failures < RETRIES && proxy_auth_failure(&e).is_none() => {
                        failures += 1;
                        wait_retry(failures, None, &shared.stop)?;
                        continue;
                    }
                    Err(e) => {
                        return Err(proxy_auth_failure(&e)
                            .unwrap_or_else(|| classified(classify_reqwest(&e), describe(&e))))
                    }
                };
                let status = response.status();
                if status != StatusCode::PARTIAL_CONTENT {
                    if retryable(status) && failures < RETRIES {
                        failures += 1;
                        wait_retry(failures, retry_after(&response), &shared.stop)?;
                        continue;
                    }
                    if failures == 0
                        && cursor == start
                        && [
                            StatusCode::UNAUTHORIZED,
                            StatusCode::FORBIDDEN,
                            StatusCode::NOT_FOUND,
                            StatusCode::GONE,
                        ]
                        .contains(&status)
                    {
                        return Ok(Outcome::Refused(status));
                    }
                    // A 200 or 416 for a range request means the server stopped
                    // honoring ranges; splicing that response would mix data (N05).
                    return Err(unreliable(
                        cursor,
                        format!("HTTP {status} for a range request"),
                    ));
                }
                if !identity_encoded(&response) {
                    return Err(unreliable(cursor, "range response was not identity-encoded"));
                }
                if !same_destination(response.url().as_str(), &effective_url, &shared.policy) {
                    return Err(unreliable(cursor, "range response arrived from another destination"));
                }
                let range = header(&response, CONTENT_RANGE);
                if range.as_deref().and_then(parse_content_range) != Some((cursor, end, total)) {
                    return Err(unreliable(
                        cursor,
                        format!(
                            "invalid Content-Range {}",
                            range.as_deref().unwrap_or("missing")
                        ),
                    ));
                }
                if etag
                    .as_ref()
                    .zip(header(&response, ETAG).as_ref())
                    .is_some_and(|(old, new)| old != new)
                {
                    return Err(err("remote validators changed during range transfer"));
                }
                if last_modified
                    .as_ref()
                    .zip(header(&response, LAST_MODIFIED).as_ref())
                    .is_some_and(|(old, new)| old != new)
                    && !shared.policy.ignore_modification_date
                {
                    return Err(err("remote validators changed during range transfer"));
                }
                if let Some(value) = header(&response, CONTENT_LENGTH) {
                    if value.parse::<u64>().ok() != Some(end - cursor + 1) {
                        return Err(unreliable(
                            cursor,
                            format!("body length {value} does not match the range"),
                        ));
                    }
                }
                (response, false)
            }
        };
        // `position` counts received bytes; `written` trails it until the buffer is flushed.
        let mut position = cursor;
        let mut written = cursor;
        let mut persisted = cursor;
        let mut buffer = vec![0u8; WRITE_BUFFER];
        let mut failed = false;
        let mut shrunk = false;
        loop {
            if shared.stop.load(Ordering::Relaxed) || shared.control.quota_hit() {
                break;
            }
            // Reserve the next chunk so a concurrent split never hands these bytes to another worker.
            let want = {
                let mut guard = shared.lock()?;
                let index = find(&guard, start)?;
                if position > guard.segments[index].end && open_ended {
                    let untouched = guard.segments.get(index + 1).filter(|next| {
                        next.cursor == next.start && next.start == position
                    });
                    match untouched {
                        Some(next) if !next.active => {
                            let next = guard.segments.remove(index + 1);
                            guard.segments[index].end = next.end;
                        }
                        // The neighbor has no response yet; if it is refused, keep streaming into it.
                        Some(next) if next.claimed == next.start => {
                            drop(guard);
                            thread::sleep(Duration::from_millis(10));
                            continue;
                        }
                        _ => {}
                    }
                }
                let segment = &mut guard.segments[index];
                segment.written = segment.written.max(written);
                segment.received = segment.received.max(position.min(segment.end.saturating_add(1)));
                shrunk |= segment.end < end;
                if position > segment.end {
                    0
                } else {
                    let room = buffer.len() - (position - written) as usize;
                    let want = room.min((segment.end - position + 1) as usize);
                    segment.claimed = position + want as u64;
                    want
                }
            };
            if want == 0 {
                break;
            }
            let filled = (position - written) as usize;
            // Read failures include the N31 idle timeout: the connection delivered
            // nothing for the stall period and is abandoned below, keeping written bytes.
            match response.read(&mut buffer[filled..filled + want]) {
                Ok(0) => {
                    failed = true;
                    break;
                }
                Ok(n) => {
                    shared.control.wait(n, &shared.stop)?;
                    shared.control.record(n);
                    position += n as u64;
                    failures = 0;
                }
                Err(_) => {
                    failed = true;
                    break;
                }
            }
            // Large positional writes keep concurrent workers off the file lock.
            if (position - written) as usize == buffer.len() {
                write_at(&shared.file, written, &buffer[..buffer.len()])?;
                written = position;
            }
            if written - persisted >= 1024 * 1024 && shared.checkpoint_due() {
                shared.persist(start, written)?;
                persisted = written;
            }
        }
        if position > written {
            write_at(&shared.file, written, &buffer[..(position - written) as usize])?;
            written = position;
        }
        if written > persisted {
            shared.persist(start, written)?;
        }
        if shared.stop.load(Ordering::Relaxed) {
            return Err(err("paused"));
        }
        let complete = {
            let guard = shared.lock()?;
            position > guard.segments[find(&guard, start)?].end
        };
        if complete {
            if !open_ended && !shrunk {
                let mut extra = [0u8; 1];
                if response.read(&mut extra)? != 0 {
                    return Err(unreliable(cursor, "body exceeded the declared range end"));
                }
            }
            shared.release(start)?;
            return Ok(Outcome::Done);
        }
        if shared.control.quota_hit() {
            return Err(err(QUOTA_REACHED));
        }
        if failed && failures < RETRIES {
            failures += 1;
            wait_retry(failures, None, &shared.stop)?;
            continue;
        }
        return Err(err("range stream ended before its declared end"));
    }
}

/// Streams a whole representation into `part`. `Ok(false)` reports a retryable network loss.
fn stream_body(
    response: &mut Response,
    part: &Path,
    length: Option<u64>,
    options: &Options,
    control: &TransferControl,
    guard_html: bool,
    modified: Option<std::time::SystemTime>,
) -> Result<bool> {
    control.initialize(0, length);
    let mut sniffed = !guard_html;
    let mut output = std::io::BufWriter::with_capacity(WRITE_BUFFER, File::create(part)?);
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        if options.stop.load(Ordering::Relaxed) {
            output.flush()?;
            output.get_ref().sync_all()?;
            return Err(err(
                "paused; this server requires restarting from byte zero",
            ));
        }
        if control.quota_hit() {
            output.flush()?;
            return Err(err(QUOTA_REACHED));
        }
        let n = match response.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => return Ok(false),
        };
        if !sniffed {
            sniffed = true;
            if looks_like_html(&buffer[..n]) {
                return Err(err(HTML_INSTEAD_OF_FILE));
            }
        }
        control.wait(n, &options.stop)?;
        output.write_all(&buffer[..n])?;
        control.record(n);
        total += n as u64;
    }
    if length.is_some_and(|length| length != total) {
        return Ok(false);
    }
    if length.is_none() {
        control.size_unverified.store(true, Ordering::Relaxed);
    }
    output.flush()?;
    stamp(output.get_ref(), modified);
    output.get_ref().sync_all()?;
    Ok(true)
}

fn single_stream(
    client: &Client,
    request: &Request,
    options: &Options,
    mut probe: Probe,
    part: &Path,
    control: &TransferControl,
) -> Result<()> {
    let mut failures = 0;
    loop {
        let mut response = match probe.response.take() {
            Some(response) => response,
            None => {
                let response = retry_send(client, request, None, &options.stop)?;
                if response.status() != StatusCode::OK {
                    return Err(request.challenge(&response).unwrap_or_else(|| {
                        err(format!("download returned HTTP {}", response.status()))
                    }));
                }
                if response.url().as_str() != probe.effective_url {
                    return Err(err("resource destination changed after probing"));
                }
                if header(&response, CONTENT_LENGTH).and_then(|s| s.parse::<u64>().ok())
                    != probe.length
                {
                    return Err(err("resource length changed after probing"));
                }
                response
            }
        };
        if !identity_encoded(&response) {
            return Err(err("encoded response cannot be safely assembled"));
        }
        let guard = html_guard(probe.content_type.as_deref(), &probe.effective_url, &options.output);
        let modified = server_time(options.set_file_time, probe.last_modified.as_deref());
        if stream_body(&mut response, part, probe.length, options, control, guard, modified)? {
            return publish(part, &options.output);
        }
        if failures == RETRIES {
            return Err(err("download ended before declared length"));
        }
        failures += 1;
        wait_retry(failures, None, &options.stop)?;
    }
}

pub fn download(options: Options) -> Result<()> {
    let control = Arc::new(TransferControl::new(options.bytes_per_second)?);
    download_with_control(options, control)
}

pub fn download_with_control(options: Options, control: Arc<TransferControl>) -> Result<()> {
    download_with_stall_timeout(options, control, DEFAULT_STALL_TIMEOUT)
}

/// Downloads with an explicit N31 stall timeout: any request whose connection
/// delivers no bytes for `stall_timeout` is abandoned and reopened from its
/// cursor, counting toward the retry budget. Rate-limiter waits never count;
/// only the socket idle time does. Must be greater than zero.
pub fn download_with_stall_timeout(
    options: Options,
    control: Arc<TransferControl>,
    stall_timeout: Duration,
) -> Result<()> {
    download_ex(options, control, stall_timeout, ResumePolicy::default(), None)
}

/// Full-control entry point (daemon use): `policy` carries the N13 resume
/// policy and, when `decide` is set, it is called once with the probing
/// response's metadata; the returned path becomes the real destination,
/// with any checkpoint files moved to it (N14 server-chosen naming). The
/// destination must stay in `options.output`'s directory.
pub fn download_ex(
    options: Options,
    control: Arc<TransferControl>,
    stall_timeout: Duration,
    policy: ResumePolicy,
    decide: Option<&mut (dyn FnMut(ResponseMetadata) -> Result<PathBuf> + Send)>,
) -> Result<()> {
    if stall_timeout.is_zero() {
        return Err(err("stall timeout must be greater than zero"));
    }
    ignore_file_size_signal();
    let output = options.output.clone();
    run_transfer(options, control, stall_timeout, &policy, decide)
        .map_err(|error| explain_storage(error, &output))
}

fn run_transfer(
    mut options: Options,
    control: Arc<TransferControl>,
    stall_timeout: Duration,
    policy: &ResumePolicy,
    decide: Option<&mut (dyn FnMut(ResponseMetadata) -> Result<PathBuf> + Send)>,
) -> Result<()> {
    if options.connections == 0 || options.connections > 16 {
        return Err(err("connections must be between 1 and 16"));
    }
    if options.output.exists() {
        return Err(err("output already exists; choose another path"));
    }
    if let Some(parent) = options.output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let temp = options.temp_dir.clone();
    if let Some(dir) = &temp {
        if !dir.is_absolute() {
            return Err(err("the temporary directory must be an absolute path"));
        }
        fs::create_dir_all(dir)?;
    }
    let part = part_path(&options.output, temp.as_deref());
    let meta = meta_path(&options.output, temp.as_deref());
    if options.restart {
        let _ = fs::remove_file(&part);
        let _ = fs::remove_file(&meta);
    }
    let request = Request::new(&options)?;
    let client = client(&options.headers, &request.method, &options.proxy, options.proxy_login.as_ref(), stall_timeout)?;
    // GET transfers keep the original header-only identity so older checkpoints stay resumable.
    let context = if request.method == Method::GET && request.body.is_none() {
        serde_json::to_vec(&options.headers)?
    } else {
        serde_json::to_vec(&(&options.headers, request.method.as_str(), &request.body))?
    };
    let context_sha256 = format!("{:x}", Sha256::digest(context));
    let saved: Option<Checkpoint> = if meta.exists() {
        Some(serde_json::from_reader(File::open(&meta)?)?)
    } else {
        None
    };
    // Probe from the first missing byte so that request also carries useful data.
    let offset = saved
        .as_ref()
        .and_then(|s| s.segments.iter().find(|x| x.cursor <= x.end))
        .map_or(0, |x| x.cursor);
    let url_host = host_of(&options.url);
    let mut probe_lease = control
        .governor()
        .map(|governor| governor.acquire(&url_host, &options.stop))
        .transpose()?;
    let mut probe = probe(&client, &request, offset, &options.stop, options.max_request_bytes.is_some())?;
    let host = host_of(&probe.effective_url);
    if host != url_host {
        probe_lease = None;
    }
    if let Some(decide) = decide {
        let provisional = options.output.clone();
        let chosen = decide(probe.metadata())?;
        if chosen != provisional {
            let same_directory = chosen.parent() == provisional.parent();
            if !same_directory || chosen.file_name().is_none() {
                return Err(err(
                    "a server-chosen filename must stay in the task's directory",
                ));
            }
            // Move any checkpoint files so resume state follows the final name.
            let (from_part, from_meta) = partial_paths(&provisional, temp.as_deref());
            let (to_part, to_meta) = partial_paths(&chosen, temp.as_deref());
            // Never move our partial onto someone else's file (another task or a browser).
            for target in [&to_part, &to_meta] {
                if fs::symlink_metadata(target).is_ok() {
                    return Err(err(format!(
                        "{} already exists; refusing to overwrite it with this task's partial data",
                        target.display()
                    )));
                }
            }
            let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
            for (from, to) in [(from_meta, to_meta), (from_part, to_part)] {
                match fs::rename(&from, &to) {
                    Ok(()) => moved.push((from, to)),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => {
                        for (from, to) in moved.into_iter().rev() {
                            let _ = fs::rename(to, from);
                        }
                        return Err(e.into());
                    }
                }
            }
            options.output = chosen;
        }
    }
    let part = part_path(&options.output, temp.as_deref());
    let meta = meta_path(&options.output, temp.as_deref());
    if !probe.ranges || probe.length.unwrap_or(0) == 0 {
        if saved.is_some() {
            return Err(err(
                "saved segmented task cannot resume as a single stream; use --restart",
            ));
        }
        return single_stream(&client, &request, &options, probe, &part, &control);
    }
    let length = probe
        .length
        .ok_or_else(|| err("range-capable resource has no length"))?;
    let state = if let Some(saved) = saved {
        if saved.version != 2
            || saved.url != options.url
            || saved.context_sha256 != context_sha256
            || !validators_match(&saved, &probe, policy)
            || !part.exists()
            || part.metadata()?.len() != length
        {
            return Err(err(
                "partial download identity changed or checkpoint invalid; use --restart",
            ));
        }
        if saved.segments.is_empty()
            || saved.segments.first().unwrap().start != 0
            || saved.segments.last().unwrap().end != length - 1
            || saved
                .segments
                .iter()
                .any(|s| s.cursor < s.start || s.cursor > s.end + 1)
            || saved
                .segments
                .windows(2)
                .any(|w| w[0].end + 1 != w[1].start)
        {
            return Err(err("invalid checkpoint intervals"));
        }
        saved
    } else {
        if part.exists() {
            return Err(err("partial file has no checkpoint; use --restart"));
        }
        Checkpoint {
            version: 2,
            url: options.url.clone(),
            effective_url: probe.effective_url.clone(),
            context_sha256,
            length,
            etag: probe.etag.clone(),
            last_modified: probe.last_modified.clone(),
            segments: split_segments_capped(length, options.connections, options.max_request_bytes),
        }
    };
    if temp.is_some() {
        ensure_final_space(&options.output, temp.as_deref().unwrap_or(Path::new("")), length)?;
    }
    control.initialize(
        state.segments.iter().map(|s| s.cursor - s.start).sum(),
        Some(length),
    );
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&part)?;
    preallocate(&file, length)?;
    store_checkpoint(&file, &meta, &state)?;
    let shared = Arc::new(Shared {
        client,
        request,
        file,
        meta: meta.clone(),
        state: Mutex::new(state),
        stop: options.stop.clone(),
        control,
        adopted: Mutex::new(probe.response.take().map(|r| (probe.offset, r))),
        refused: Mutex::new(None),
        checkpointed: Mutex::new(Instant::now()),
        policy: *policy,
        aborted: Mutex::new(None),
        host,
        probe_lease: Mutex::new(probe_lease.take()),
    });
    {
        let weak = Arc::downgrade(&shared);
        let _ = shared.control.live.set(LiveSegments(Box::new(move || {
            weak.upgrade()
                .and_then(|shared| {
                    shared.state.lock().ok().map(|state| {
                        state
                            .segments
                            .iter()
                            .take(256)
                            .map(|s| (s.start, s.end, s.cursor.max(s.written).max(s.received)))
                            .collect()
                    })
                })
                .unwrap_or_default()
        })));
    }
    let handles: Vec<_> = (0..options.connections)
        .map(|_| {
            let shared = shared.clone();
            thread::spawn(move || worker(shared))
        })
        .collect();
    let mut failure = None;
    for handle in handles {
        let result = handle
            .join()
            .unwrap_or_else(|_| Err(err("download worker panicked")));
        if let Err(e) = result {
            if failure.is_none() {
                failure = Some(e);
                options.stop.store(true, Ordering::Relaxed);
            }
        }
    }
    // The first worker failure aborted the others; surface that cause, not a sibling's "paused".
    if let Some(error) = shared.take_failure() {
        return Err(error);
    }
    if let Some(e) = failure {
        return Err(e);
    }
    let shared = Arc::into_inner(shared).ok_or_else(|| err("download worker still running"))?;
    drop(shared.adopted);
    let state = shared
        .state
        .into_inner()
        .map_err(|_| err("checkpoint lock poisoned"))?;
    if state.segments.iter().any(|s| s.cursor != s.end + 1) {
        return Err(match shared.refused.into_inner().ok().flatten() {
            Some(status) => err(format!(
                "server refused additional connections (HTTP {status}); partial data kept"
            )),
            None => err("download incomplete after workers ended"),
        });
    }
    stamp(
        &shared.file,
        server_time(
            options.set_file_time,
            probe.last_modified.as_deref().or(state.last_modified.as_deref()),
        ),
    );
    shared.file.sync_all()?;
    drop(shared.file);
    publish(&part, &options.output)?;
    discard(&meta);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn removing_rate_cap_releases_waiting_worker() {
        let control = Arc::new(TransferControl::new(Some(1)).unwrap());
        let waiting = control.clone();
        let worker = thread::spawn(move || waiting.wait(65536, &AtomicBool::new(false)));
        thread::sleep(Duration::from_millis(30));
        let changed = Instant::now();
        control.set_rate(None).unwrap();
        worker.join().unwrap().unwrap();
        assert!(changed.elapsed() < Duration::from_secs(1));
        assert!(control.set_rate(Some(0)).is_err());
    }

    #[test]
    fn same_site_redirects_are_opt_in_and_stay_on_one_site_and_path() {
        let strict = ResumePolicy::default();
        let lenient = ResumePolicy { same_site_redirects: true, ..ResumePolicy::default() };
        let a = "https://rr2---sn-aaa.googlevideo.com/videoplayback?itag=18&x=1";
        let b = "https://rr3---sn-bbb.googlevideo.com/videoplayback?itag=18&x=2";
        assert!(same_destination(a, a, &strict));
        assert!(!same_destination(a, b, &strict), "different hosts are another destination by default");
        assert!(same_destination(a, b, &lenient));
        for other in [
            "https://rr3---sn-bbb.googlevideo.com/other?itag=18",
            "http://rr3---sn-bbb.googlevideo.com/videoplayback?itag=18",
            "https://evil.example/videoplayback?itag=18",
            "https://googlevideo.com.evil.example/videoplayback?itag=18",
        ] {
            assert!(!same_destination(a, other, &lenient), "{other}");
        }
        assert!(!same_destination("https://localhost/a", "https://other/a", &lenient), "single-label hosts have no site");
    }

    #[test]
    fn capped_split_never_exceeds_the_request_cap() {
        let capped = split_segments_capped(100 << 20, 8, Some(8 << 20));
        assert!(capped.len() >= 13 && capped.iter().all(|s| s.end - s.start < (8 << 20)));
        assert_eq!(capped.last().unwrap().end, (100 << 20) - 1);
        assert_eq!(split_segments_capped(1000, 8, Some(1 << 20)).len(), 8);
        assert_eq!(split_segments_capped(100 << 20, 8, None).len(), 8);
    }

    #[test]
    fn splits_exactly_without_gaps() {
        for length in 1..1000 {
            for workers in 1..=16 {
                let segments = split_segments(length, workers);
                assert_eq!(segments.first().unwrap().start, 0);
                assert_eq!(segments.last().unwrap().end, length - 1);
                assert!(segments.windows(2).all(|w| w[0].end + 1 == w[1].start));
                assert_eq!(
                    segments.iter().map(|s| s.end - s.start + 1).sum::<u64>(),
                    length
                );
            }
        }
    }
    #[test]
    fn rejects_invalid_content_ranges() {
        assert_eq!(parse_content_range("bytes 3-7/8"), Some((3, 7, 8)));
        for value in [
            "bytes 4-8/8",
            "bytes 8-4/9",
            "bytes */9",
            "bytes 0-0/*",
            "items 0-0/1",
            "bytes 0-0/0",
        ] {
            assert_eq!(parse_content_range(value), None, "{value}");
        }
    }
    #[test]
    fn request_methods_and_bodies_are_validated() {
        let with = |method: &str, body: Option<&[u8]>| {
            Request::new(&Options {
                url: "http://example.invalid/".into(),
                method: method.into(),
                body: body.map(<[u8]>::to_vec),
                ..Default::default()
            })
        };
        assert_eq!(with("post", Some(b"a=1")).unwrap().method, Method::POST);
        assert!(with("GET", None).is_ok());
        assert!(with("GET", Some(b"a=1")).is_err());
        assert!(with("HEAD", None).is_err());
        assert!(with("bad method", None).is_err());
        assert!(with("POST", Some(&vec![0; MAX_BODY + 1])).is_err());
    }

    fn mtime_secs(path: &Path) -> u64 {
        fs::metadata(path).unwrap().modified().unwrap()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
    }

    fn workspace(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xdm-unit-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn meta(url: &str, content_type: Option<&str>) -> ResponseMetadata {
        ResponseMetadata {
            effective_url: url.into(),
            content_disposition: None,
            content_length: None,
            content_type: content_type.map(str::to_owned),
        }
    }

    #[test]
    fn http_dates_parse_and_garbage_is_rejected() {
        let epoch = |t: std::time::SystemTime| t.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        assert_eq!(epoch(parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT").unwrap()), 784111777);
        assert_eq!(epoch(parse_http_date("Sunday, 06-Nov-94 08:49:37 GMT").unwrap()), 784111777);
        assert_eq!(epoch(parse_http_date("Sun Nov  6 08:49:37 1994").unwrap()), 784111777);
        for bad in ["", "yesterday", "Sun, 32 Nov 1994 08:49:37 GMT", "Sun, 06 Foo 1994 08:49:37 GMT", "Sun, 06 Nov 1994 25:00:00 GMT"] {
            assert!(parse_http_date(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn server_time_changes_only_for_valid_dates() {
        let dir = workspace("servertime");
        let file = dir.join("f.bin");
        fs::write(&file, b"x").unwrap();
        let handle = OpenOptions::new().write(true).open(&file).unwrap();
        for bad in [Some("garbage"), None, Some("Sun, 06 Nov 99999 08:49:37 GMT"), Some("Sun, 06 Nov 12345678901234567890 08:49:37 GMT")] {
            stamp(&handle, server_time(true, bad));
        }
        stamp(&handle, server_time(false, Some("Sun, 06 Nov 1994 08:49:37 GMT")));
        assert!(fs::metadata(&file).unwrap().modified().unwrap().elapsed().unwrap() < Duration::from_secs(600));
        stamp(&handle, server_time(true, Some("Sun, 06 Nov 1994 08:49:37 GMT")));
        assert_eq!(mtime_secs(&file), 784111777);
        // The time survives a publish by link.
        let out = dir.join("g.bin");
        publish(&file, &out).unwrap();
        assert_eq!(mtime_secs(&out), 784111777);
    }

    #[test]
    fn publish_fallbacks_never_replace_and_removal_failures_are_not_errors() {
        let dir = workspace("noreplace");
        let (a, b) = (dir.join("a"), dir.join("b"));
        fs::write(&a, b"1").unwrap();
        fs::write(&b, b"2").unwrap();
        #[cfg(target_os = "linux")]
        {
            let error = rename_noreplace(&a, &b).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
            assert_eq!(fs::read(&b).unwrap(), b"2");
            fs::remove_file(&b).unwrap();
            rename_noreplace(&a, &b).unwrap();
            assert_eq!(fs::read(&b).unwrap(), b"1");
        }
        // A part that is already gone does not fail a publish that installed the file.
        discard(&dir.join("missing"));
    }

    #[test]
    fn html_sniffing_and_guard() {
        assert!(looks_like_html(b"  \n<!DOCTYPE html><html>"));
        assert!(looks_like_html(b"<html><body>"));
        assert!(!looks_like_html(b"PK\x03\x04"));
        let out = Path::new("/d/x.dat");
        assert!(html_guard(Some("text/html; charset=utf-8"), "http://h/a.zip", out));
        assert!(html_guard(Some("text/html"), "http://h/get.php", Path::new("/d/a.iso")));
        assert!(!html_guard(Some("text/html"), "http://h/page.php", out));
        assert!(!html_guard(Some("application/zip"), "http://h/a.zip", out));
    }

    #[test]
    fn temp_partials_are_distinct_per_destination() {
        let temp = Path::new("/tmp/xdm-temp");
        let a = partial_paths(Path::new("/a/f.bin"), Some(temp));
        let b = partial_paths(Path::new("/b/f.bin"), Some(temp));
        assert_ne!(a.0, b.0);
        assert!(a.0.starts_with(temp) && a.1.starts_with(temp));
        assert!(a.0.to_string_lossy().ends_with("-f.bin.part"));
        assert_eq!(
            partial_paths(Path::new("/a/f.bin"), None).1,
            Path::new("/a/f.bin.part.meta")
        );
    }

    #[test]
    fn copy_publish_installs_without_clobbering_and_keeps_part_on_conflict() {
        let dir = workspace("copypublish");
        let (part, out) = (dir.join("a.part"), dir.join("a"));
        fs::write(&part, b"data").unwrap();
        copy_publish(&part, &out).unwrap();
        assert_eq!(fs::read(&out).unwrap(), b"data");
        assert!(!part.exists());
        // A foreign file named like the old fixed staging name is never touched.
        fs::write(dir.join("a.xdm-copy"), b"foreign").unwrap();
        fs::write(&part, b"new").unwrap();
        assert!(copy_publish(&part, &out).is_err());
        assert_eq!(fs::read(&out).unwrap(), b"data");
        assert_eq!(fs::read(&part).unwrap(), b"new");
        assert_eq!(fs::read(dir.join("a.xdm-copy")).unwrap(), b"foreign");
        let leftovers: Vec<_> = fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(leftovers.len(), 3, "{leftovers:?}");
    }

    #[test]
    fn mime_extension_reconciliation() {
        let name = |url: &str, ct: Option<&str>| reconcile_server_name(&meta(url, ct), "prov.bin");
        assert_eq!(name("http://h/get.php", Some("application/zip")), Some("get.zip".into()));
        assert_eq!(name("http://h/dl", Some("application/pdf; charset=binary")), Some("dl.pdf".into()));
        assert_eq!(name("http://h/a.zip", Some("application/zip")), Some("a.zip".into()));
        // A real extension is never rewritten.
        assert_eq!(name("http://h/a.pdf", Some("application/zip")), Some("a.pdf".into()));
        assert_eq!(name("http://h/archive.tar.gz", Some("application/x-tar")), Some("archive.tar.gz".into()));
        assert_eq!(name("http://h/clip.mp4", Some("audio/mpeg")), Some("clip.mp4".into()));
        assert_eq!(name("http://h/a.dat", Some("application/zip")), Some("a.dat".into()));
        assert_eq!(name("http://h/a.php", Some("text/html")), Some("a.php".into()));
        assert_eq!(name("http://h/a.zip", None), Some("a.zip".into()));
    }

    #[test]
    fn publish_never_replaces_an_existing_destination() {
        let directory = std::env::temp_dir().join(format!("xdm-publish-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let (part, output) = (directory.join("a.part"), directory.join("a"));
        fs::write(&part, b"new").unwrap();
        fs::write(&output, b"old").unwrap();
        assert!(publish(&part, &output).is_err());
        assert_eq!(fs::read(&output).unwrap(), b"old");
        assert_eq!(fs::read(&part).unwrap(), b"new");
        fs::remove_file(&output).unwrap();
        publish(&part, &output).unwrap();
        assert_eq!(fs::read(&output).unwrap(), b"new");
        assert!(!part.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn version_two_checkpoints_without_runtime_fields_still_load() {
        let saved: Checkpoint = serde_json::from_str(
            r#"{"version":2,"url":"u","effective_url":"u","context_sha256":"x","length":10,
            "etag":null,"last_modified":null,"segments":[{"start":0,"end":9,"cursor":4}]}"#,
        )
        .unwrap();
        assert!(!saved.segments[0].active);
        assert!(!serde_json::to_string(&saved).unwrap().contains("claimed"));
    }

    #[test]
    fn retry_classification_is_bounded() {
        assert!(retryable(StatusCode::SERVICE_UNAVAILABLE));
        assert!(retryable(StatusCode::TOO_MANY_REQUESTS));
        assert!(!retryable(StatusCode::FORBIDDEN));
        assert!(!retryable(StatusCode::NOT_FOUND));
    }

    #[test]
    fn challenges_are_parsed_from_authenticate_headers() {
        let parse = |value: &str| parse_challenge(value).map(|(scheme, realm)| (scheme, realm));
        assert_eq!(
            parse(r#"Basic realm="xdm-test""#),
            Some(("Basic".into(), Some("xdm-test".into())))
        );
        assert_eq!(parse("Basic realm=basic-area"), Some(("Basic".into(), Some("basic-area".into()))));
        assert_eq!(parse("BASIC"), Some(("BASIC".into(), None)));
        assert_eq!(
            parse(r#"Bearer realm="x", Basic realm="y""#),
            Some(("Bearer".into(), Some("x".into())))
        );
        assert_eq!(parse(""), None);
        assert_eq!(parse(",-x"), None);
    }

    #[test]
    fn auth_challenges_survive_the_error_box() {
        let challenge = AuthChallenge {
            note: None,
            status: 401,
            scheme: "Basic".into(),
            realm: Some("xdm-test".into()),
        };
        let error: Box<dyn Error + Send + Sync> = Box::new(AuthRequired(challenge.clone()));
        assert_eq!(auth_challenge(error.as_ref()), Some(challenge));
        assert_eq!(auth_challenge(err("probe failed: HTTP 403").as_ref()), None);
        assert_eq!(
            AuthRequired(AuthChallenge {
                note: None,
                status: 401,
                scheme: "Basic".into(),
                realm: Some("xdm-test".into())
            })
            .to_string(),
            "authentication required: HTTP 401 Basic realm \"xdm-test\""
        );
    }

    #[test]
    fn range_unreliable_errors_survive_the_error_box() {
        let error: Box<dyn Error + Send + Sync> =
            unreliable(4096, "invalid Content-Range bytes 4097-8191/16384");
        assert_eq!(
            range_unreliable(error.as_ref()),
            Some(RangeUnreliable {
                at: 4096,
                reason: "invalid Content-Range bytes 4097-8191/16384".into()
            })
        );
        assert_eq!(range_unreliable(err("paused").as_ref()), None);
        let text = error.to_string();
        assert!(text.contains("stopped honoring byte ranges at byte 4096"), "{text}");
        assert!(text.contains("--restart"), "{text}");
    }

    #[test]
    fn validators_match_never_amounts_to_splice() {
        let saved = |etag: Option<&str>, modified: Option<&str>| Checkpoint {
            version: 2,
            url: "u".into(),
            effective_url: "u".into(),
            context_sha256: "0".repeat(64),
            length: 8,
            etag: etag.map(str::to_owned),
            last_modified: modified.map(str::to_owned),
            segments: Vec::new(),
        };
        let probe = |etag: Option<&str>, modified: Option<&str>, url: &str, length: u64| Probe {
            effective_url: url.into(),
            length: Some(length),
            ranges: true,
            etag: etag.map(str::to_owned),
            last_modified: modified.map(str::to_owned),
            disposition: None,
            content_type: None,
            response: None,
            offset: 0,
        };
        let (etag, date) = ("\"x\"", "Mon, 01 Jan 2024 00:00:00 GMT");
        let (other_tag, other_date, other_url) = ("\"z\"", "Tue, 02 Jan 2024 00:00:00 GMT", "v");
        for policy in [ResumePolicy::default(), ResumePolicy { ignore_modification_date: true, ..ResumePolicy::default() }] {
            let old = saved(Some(etag), Some(date));
            // A changed length, a changed strong ETag or a changed effective URL
            // refuse in every policy.
            assert!(!validators_match(&old, &probe(Some(etag), Some(date), "u", 4), &policy));
            assert!(!validators_match(&old, &probe(Some(other_tag), Some(date), "u", 8), &policy));
            assert!(!validators_match(&old, &probe(Some(etag), Some(date), other_url, 8), &policy));
        }
        let old = saved(Some(etag), Some(date));
        // Only the date changed: refuse by default, allowed only behind the
        // N13 flag while the strong ETag and length still match.
        assert!(!validators_match(&old, &probe(Some(etag), Some(other_date), "u", 8), &ResumePolicy::default()));
        assert!(validators_match(&old, &probe(Some(etag), Some(other_date), "u", 8), &ResumePolicy { ignore_modification_date: true, ..ResumePolicy::default() }));
        // Without a strong ETag the date is the only identity signal and stays
        // binding: disabling the date check alone can never splice new bytes.
        let dated = saved(None, Some(date));
        assert!(validators_match(&dated, &probe(None, Some(date), "u", 8), &ResumePolicy::default()));
        assert!(!validators_match(&dated, &probe(None, Some(other_date), "u", 8), &ResumePolicy { ignore_modification_date: true, ..ResumePolicy::default() }));
        assert!(!validators_match(&dated, &probe(None, None, "u", 8), &ResumePolicy { ignore_modification_date: true, ..ResumePolicy::default() }));
        let weak = saved(Some("W/\"a\""), Some(date));
        assert!(!validators_match(&weak, &probe(Some("W/\"a\""), Some(other_date), "u", 8), &ResumePolicy { ignore_modification_date: true, ..ResumePolicy::default() }));
    }
    #[test]
    fn disposition_and_url_filenames_parse() {
        assert_eq!(
            disposition_filename(Some(r#"attachment; filename="plain name.pdf""#)),
            Some("plain name.pdf".into())
        );
        // Quoted separators stay whole and escaped quotes unfold.
        assert_eq!(
            disposition_filename(Some(r#"attachment; filename="a;b\\c.zip""#)),
            Some("a;b\\c.zip".into())
        );
        // RFC 5987: UTF-8 extended value percent-decoded, and preferred.
        assert_eq!(
            disposition_filename(Some("attachment; filename=\"fallback.pdf\"; filename*=UTF-8''caf%C3%A9%20%E2%82%AC.zip")),
            Some("café €.zip".into())
        );
        assert_eq!(disposition_filename(Some("attachment; filename*=iso-8859-1''x.zip; filename=\"fallback\"")),
            Some("fallback".into()));
        assert_eq!(disposition_filename(None), None);
        assert_eq!(url_filename("http://h/final/tree.tar.gz?x=1#frag"), Some("tree.tar.gz".into()));
        assert_eq!(url_filename("http://127.0.0.1:2/finaldir/red.tar.gz"), Some("red.tar.gz".into()));
        assert_eq!(url_filename("http://h/"), None);
    }

    #[test]
    fn server_names_are_sanitized_and_never_provisional() {
        let meta = |disposition: Option<&str>, url: &str| ResponseMetadata {
            effective_url: url.into(),
            content_disposition: disposition.map(str::to_owned),
            content_length: Some(8),
            content_type: None,
        };
        assert_eq!(
            server_filename(&meta(Some(r#"attachment; filename="C:\\evil\\report.pdf""#), "http://h/x"),
                "plain.pdf"),
            Some("report.pdf".into())
        );
        assert_eq!(
            server_filename(&meta(Some(r#"attachment; filename="dangerous?q*name.zip""#), "http://h/x"),
                "plain.pdf"),
            Some("dangerous_q_name.zip".into())
        );
        // Reserved device names and trailing dots/spaces lose their collisions.
        assert_eq!(
            server_filename(&meta(Some(r#"attachment; filename="CON""#), "http://h/x"), "plain.pdf"),
            Some("CON_".into())
        );
        assert_eq!(
            server_filename(&meta(Some(r#"attachment; filename="com1.bin.""#), "http://h/x"), "plain.pdf"),
            Some("com1_.bin".into())
        );
        // A lost disposition name falls back to the URL's final segment.
        assert_eq!(
            server_filename(&meta(Some(r#"attachment; filename="a\..\..""#), "http://h/x/real-name.bin"), "p"),
            Some("real-name.bin".into())
        );
        assert_eq!(server_filename(&meta(Some(r#"attachment; filename="a\..\..""#), "http://h/"), "p"), None);
        // The URL segment fills in when no disposition came.
        assert_eq!(
            server_filename(&meta(None, "http://h/final/served.tar.gz"), "plain"),
            Some("served.tar.gz".into())
        );
        // Equal to the provisional name: keep it (no rename, no suffix churn).
        assert_eq!(server_filename(&meta(None, "http://h/plain.pdf"), "plain.pdf"), None);
    }
}

/// Property-style checks for the parsing helpers: server names and
/// Content-Range values come from untrusted bytes, so garbage must only
/// produce rejections, never panics.
#[cfg(test)]
mod property_tests {
    use super::*;

    /// Deterministic xorshift64 PRNG: reproducible property runs without deps.
    struct Rng(u64);

    impl Rng {
        fn new(seed: u64) -> Self {
            Rng(seed | 1)
        }

        fn next_u64(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }

        fn below(&mut self, bound: u64) -> u64 {
            if bound == 0 { 0 } else { self.next_u64() % bound }
        }

        fn string(&mut self, alphabet: &[u8], max_len: usize) -> String {
            let len = self.below(max_len as u64 + 1) as usize;
            let bytes: Vec<u8> = (0..len)
                .map(|_| alphabet[self.below(alphabet.len() as u64) as usize])
                .collect();
            String::from_utf8_lossy(&bytes).into_owned()
        }
    }

    #[test]
    fn parse_content_range_never_panics_on_arbitrary_input() {
        let mut rng = Rng::new(0x11c0_0001);
        let mut alphabet: Vec<u8> = (0x20u8..0x7f).collect();
        alphabet.extend_from_slice(&[0, b'\n', b'\t', 0xff]);
        for _ in 0..20_000 {
            let text = rng.string(&alphabet, 40);
            let _ = parse_content_range(&text);
        }
    }

    #[test]
    fn parse_content_range_round_trips_every_valid_range() {
        let mut rng = Rng::new(0x11c0_0002);
        for _ in 0..20_000 {
            let total = 1 + rng.below(u64::from(u32::MAX));
            let start = rng.below(total);
            let end = start + rng.below(total - start);
            let value = format!("bytes {start}-{end}/{total}");
            assert_eq!(parse_content_range(&value), Some((start, end, total)), "{value}");
            assert_eq!(
                parse_content_range(&format!("\t {value} \n")),
                Some((start, end, total)),
                "whitespace around {value}"
            );
        }
    }

    #[test]
    fn split_segments_is_a_contiguous_partition_matching_the_count_formula() {
        let mut rng = Rng::new(0x11c0_0003);
        for _ in 0..5_000 {
            let length = 1 + rng.below(1 << 40);
            let workers = rng.below(70) as usize;
            let segments = split_segments(length, workers);
            let expected = workers.max(1).min(16).min(length as usize);
            assert_eq!(segments.len(), expected);
            assert!(segments.iter().all(|s| s.start <= s.end));
            assert_eq!(segments.first().unwrap().start, 0);
            assert_eq!(segments.last().unwrap().end, length - 1);
            assert!(segments.windows(2).all(|w| w[0].end + 1 == w[1].start));
            assert_eq!(
                segments.iter().map(|s| s.end - s.start + 1).sum::<u64>(),
                length
            );
            assert!(segments.iter().all(|s| s.cursor == s.start && s.claimed == s.start));
        }
    }

    /// `split_segments` computes `length / count` after clamping `count` to
    /// `length`, so a zero length divides by zero. The download path only calls
    /// it for a range-capable object with `length > 0`, so it is latent today.
    #[test]
    fn split_segments_of_zero_length_is_empty() {
        assert!(split_segments(0, 8).is_empty());
    }

    #[test]
    fn checkpoint_records_every_workers_flushed_bytes() {
        let dir = std::env::temp_dir().join(format!("xdm-ckpt-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let shared = claim_shared(8 << 20, 2, &dir);
        let first = shared.claim().unwrap().unwrap();
        let second = shared.claim().unwrap().unwrap();
        {
            let mut guard = shared.lock().unwrap();
            let index = find(&guard, second).unwrap();
            guard.segments[index].written = second + 4096;
        }
        shared.persist(first, first + 1024).unwrap();
        let saved: Checkpoint = serde_json::from_reader(File::open(&shared.meta).unwrap()).unwrap();
        let cursor = |at| saved.segments.iter().find(|s| s.start == at).unwrap().cursor;
        assert_eq!(cursor(first), first + 1024);
        assert_eq!(cursor(second), second + 4096);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn impossible_preallocation_fails_without_growing_the_file() {
        let path = std::env::temp_dir().join(format!("xdm-prealloc-{}", std::process::id()));
        let file = File::create(&path).unwrap();
        // Far beyond any disk and any filesystem's maximum file size.
        let error = preallocate(&file, 1 << 62).unwrap_err();
        assert!(
            matches!(
                error.kind(),
                std::io::ErrorKind::StorageFull
                    | std::io::ErrorKind::FileTooLarge
                    | std::io::ErrorKind::QuotaExceeded
                    | std::io::ErrorKind::InvalidInput
            ),
            "{error:?}"
        );
        assert_eq!(file.metadata().unwrap().len(), 0);
        let explained = explain_storage(Box::new(error), &path).to_string();
        assert!(explained.contains("disk space") || explained.contains("size limit"), "{explained}");
        fs::remove_file(path).unwrap();
    }

    fn claim_shared(length: u64, connections: usize, dir: &Path) -> Shared {
        let meta = dir.join("task.part.meta");
        let file = File::create(dir.join("task.part")).unwrap();
        let state = Checkpoint {
            version: 2,
            url: "http://example.invalid/f".into(),
            effective_url: "http://example.invalid/f".into(),
            context_sha256: "0".repeat(64),
            length,
            etag: None,
            last_modified: None,
            segments: split_segments(length, connections),
        };
        Shared {
            client: Client::builder().build().unwrap(),
            request: Request {
                url: "http://example.invalid/f".into(),
                method: Method::GET,
                body: None,
                auth: None,
            },
            file,
            meta,
            state: Mutex::new(state),
            stop: Arc::new(AtomicBool::new(false)),
            control: Arc::new(TransferControl::new(None).unwrap()),
            adopted: Mutex::new(None),
            refused: Mutex::new(None),
            checkpointed: Mutex::new(Instant::now()),
            policy: ResumePolicy::default(),
            aborted: Mutex::new(None),
            host: String::new(),
            probe_lease: Mutex::new(None),
        }
    }

    #[test]
    fn claim_keeps_the_segments_a_contiguous_partition() {
        let mut rng = Rng::new(0x11c0_0004);
        for case in 0..80u64 {
            let length = 1 + rng.below(4 * 1024 * 1024);
            let connections = 1 + rng.below(16) as usize;
            let dir = std::env::temp_dir().join(format!(
                "xdm-http-claim-{}-{case}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();

            let shared = claim_shared(length, connections, &dir);
            let mut claims = 0usize;
            while let Some(start) = shared.claim().unwrap() {
                claims += 1;
                assert!(claims < 10_000, "claim never terminated");
                let guard = shared.state.lock().unwrap();
                assert_eq!(guard.segments.first().unwrap().start, 0);
                assert_eq!(guard.segments.last().unwrap().end, length - 1);
                assert!(guard.segments.iter().all(|s| s.start <= s.end));
                assert!(guard.segments.windows(2).all(|w| w[0].end + 1 == w[1].start));
                assert_eq!(
                    guard.segments.iter().map(|s| s.end - s.start + 1).sum::<u64>(),
                    length
                );
                assert!(guard.segments.iter().all(|s| {
                    s.cursor >= s.start && s.cursor <= s.end + 1 && s.claimed <= s.end + 1
                }));
                for (index, segment) in guard.segments.iter().enumerate() {
                    assert_eq!(find(&guard, segment.start).unwrap(), index);
                }
                assert!(guard.segments.iter().any(|s| s.start == start));
                drop(guard);
            }
            assert!(claims >= connections.min(16).min(length as usize));
            let _ = fs::remove_dir_all(&dir);
        }
    }
}

/// Local disposable servers for the N31 stall and N05 range-corruption behaviors (rule 13).
#[cfg(test)]
mod net_tests {
    use super::*;

    #[derive(Clone)]
    struct Faults {
        len: u64,
        ignore_ranges: bool,
        /// This request number delivers half its body, then holds the connection open.
        stall_attempt: Option<u32>,
        /// Requests after this number announce a wrong Content-Range.
        flip_after: Option<u32>,
        /// Pause before answering a request whose Content-Range will be wrong.
        flip_delay: Duration,
    }

    impl Faults {
        fn new(len: u64) -> Self {
            Self {
                len,
                ignore_ranges: false,
                stall_attempt: None,
                flip_after: None,
                flip_delay: Duration::ZERO,
            }
        }
    }

    struct Server {
        port: u16,
        /// One `(start, end, open_ended)` entry per request, in arrival order.
        spans: Arc<Mutex<Vec<(u64, u64, bool)>>>,
        requests: Arc<AtomicU64>,
    }

    /// Spawns a threaded HTTP/1.1 server that closes every connection after one response.
    fn spawn_server(faults: Faults) -> Server {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let spans = Arc::new(Mutex::new(Vec::new()));
        let requests = Arc::new(AtomicU64::new(0));
        let server_spans = spans.clone();
        let server_requests = requests.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let faults = faults.clone();
                let spans = server_spans.clone();
                let requests = server_requests.clone();
                thread::spawn(move || serve(stream, faults, spans, requests));
            }
        });
        Server {
            port,
            spans,
            requests,
        }
    }

    fn serve(
        stream: std::net::TcpStream,
        faults: Faults,
        spans: Arc<Mutex<Vec<(u64, u64, bool)>>>,
        requests: Arc<AtomicU64>,
    ) {
        use std::io::{BufRead, BufReader, Write};
        let mut stream = stream;
        let mut reader = BufReader::new(match stream.try_clone() {
            Ok(clone) => clone,
            Err(_) => return,
        });
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
            return;
        }
        let mut range: Option<String> = None;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                if name.trim().eq_ignore_ascii_case("range") {
                    range = Some(value.trim().to_owned());
                }
            }
        }
        let attempt = (requests.fetch_add(1, Ordering::Relaxed) + 1) as u32;
        let len = faults.len;
        let (start, end, open) = match range.as_deref() {
            Some(value) if !faults.ignore_ranges => {
                let value = value.strip_prefix("bytes=").unwrap_or(value);
                let (first, last) = value.split_once('-').unwrap_or((value, ""));
                let start = first.parse::<u64>().unwrap_or(0).min(len - 1);
                let open = last.is_empty();
                let end = if open {
                    len - 1
                } else {
                    last.parse::<u64>().unwrap_or(len - 1).min(len - 1)
                };
                (start, end, open)
            }
            _ => (0, len - 1, true),
        };
        if let Ok(mut spans) = spans.lock() {
            spans.push((start, end, open));
        }
        let ranged = range.is_some() && !faults.ignore_ranges;
        let flip = faults.flip_after.is_some_and(|after| attempt > after);
        if flip && !faults.flip_delay.is_zero() {
            // Let the honest streams bank checkpointed progress before the corrupt answer.
            thread::sleep(faults.flip_delay);
        }
        let status = if ranged { "206 Partial Content" } else { "200 OK" };
        let mut head = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nETag: \"net-tests\"\r\nLast-Modified: Sun, 06 Nov 1994 08:49:37 GMT\r\nConnection: close\r\n",
            end - start + 1
        );
        if ranged {
            // N05 fault: the announced window is shifted one byte off the requested one.
            let announced = if flip { start + 1 } else { start };
            head.push_str(&format!("Content-Range: bytes {announced}-{end}/{len}\r\n"));
        }
        head.push_str("\r\n");
        if stream.write_all(head.as_bytes()).is_err() {
            return;
        }
        let half = start + (end - start + 1) / 2;
        let mut position = start;
        while position <= end {
            let count = (end - position + 1).min(16 * 1024) as usize;
            if faults.stall_attempt == Some(attempt) && position >= half {
                // N31 fault: keep the socket open without delivering another byte.
                thread::sleep(Duration::from_secs(30));
            }
            let chunk: Vec<u8> = (0..count)
                .map(|offset| ((position + offset as u64) % 256) as u8)
                .collect();
            if stream.write_all(&chunk).is_err() || stream.flush().is_err() {
                return;
            }
            position += count as u64;
        }
    }

    impl Server {
        fn url(&self) -> String {
            format!("http://127.0.0.1:{}/f.bin", self.port)
        }

        fn total(&self) -> usize {
            self.requests.load(Ordering::Relaxed) as usize
        }

        fn closed_spans(&self) -> Vec<(u64, u64)> {
            self.spans
                .lock()
                .map(|spans| {
                    spans
                        .iter()
                        .filter(|&&(_, _, open)| !open)
                        .map(|&(start, end, _)| (start, end))
                        .collect()
                })
                .unwrap_or_default()
        }
    }

    fn workspace(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xdm-stall-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn options(url: String, output: &Path, connections: usize) -> Options {
        Options {
            url,
            output: output.to_path_buf(),
            connections,
            ..Default::default()
        }
    }

    fn assert_pattern(path: &Path, len: u64) {
        let data = fs::read(path).unwrap();
        assert_eq!(data.len() as u64, len);
        assert!(
            data.iter()
                .enumerate()
                .all(|(offset, byte)| *byte == (offset % 256) as u8),
            "downloaded bytes do not match the served pattern"
        );
    }

    #[test]
    fn html_page_for_a_binary_name_fails_but_labelled_binaries_download() {
        use std::io::{BufRead, BufReader};
        let serve = |body: &'static [u8]| {
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let port = listener.local_addr().unwrap().port();
            thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                            break;
                        }
                    }
                    let mut stream = stream;
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(head.as_bytes());
                    let _ = stream.write_all(body);
                }
            });
            port
        };
        let dir = workspace("htmlguard");
        let page = serve(b"<!doctype html><html>Not found</html>");
        let error = download(options(format!("http://127.0.0.1:{page}/a.zip"), &dir.join("a.zip"), 1)).unwrap_err();
        assert!(error.to_string().contains(HTML_INSTEAD_OF_FILE), "{error}");
        assert!(!dir.join("a.zip").exists());
        let real = serve(b"PK\x03\x04 real zip bytes");
        download(options(format!("http://127.0.0.1:{real}/b.zip"), &dir.join("b.zip"), 1)).unwrap();
        assert_eq!(fs::read(dir.join("b.zip")).unwrap()[..2], *b"PK");
        // A page for a non-binary name is accepted.
        let ok = serve(b"<html>fine</html>");
        download(options(format!("http://127.0.0.1:{ok}/c.php"), &dir.join("c.php"), 1)).unwrap();
    }

    #[test]
    fn server_name_never_overwrites_existing_partials() {
        let dir = workspace("renameguard");
        let server = spawn_server(Faults::new(64 * 1024));
        let provisional = dir.join("prov.bin");
        fs::write(dir.join("chosen.bin.part"), b"someone else").unwrap();
        let mut decide = |_: ResponseMetadata| -> Result<PathBuf> { Ok(dir.join("chosen.bin")) };
        let error = download_ex(
            options(server.url(), &provisional, 1),
            Arc::new(TransferControl::new(None).unwrap()),
            DEFAULT_STALL_TIMEOUT,
            ResumePolicy::default(),
            Some(&mut decide),
        )
        .unwrap_err();
        assert!(error.to_string().contains("refusing to overwrite"), "{error}");
        assert_eq!(fs::read(dir.join("chosen.bin.part")).unwrap(), b"someone else");
    }

    fn mtime_secs(path: &Path) -> u64 {
        fs::metadata(path).unwrap().modified().unwrap()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
    }

    #[test]
    fn segmented_download_uses_temp_dir_and_sets_server_time() {
        let dir = workspace("tempdir");
        let temp = dir.join("scratch");
        let dest = dir.join("final");
        let len = 300 * 1024;
        let server = spawn_server(Faults::new(len));
        let mut opts = options(server.url(), &dest.join("f.bin"), 4);
        opts.temp_dir = Some(temp.clone());
        opts.set_file_time = true;
        download(opts).unwrap();
        assert_pattern(&dest.join("f.bin"), len);
        assert_eq!(mtime_secs(&dest.join("f.bin")), 784111777);
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 1, "no partials beside the destination");
        assert_eq!(fs::read_dir(&temp).unwrap().count(), 0, "temp partials are gone after publish");
    }

    #[test]
    fn single_stream_download_keeps_time_unchanged_when_disabled() {
        let dir = workspace("notime");
        let mut faults = Faults::new(64 * 1024);
        faults.ignore_ranges = true;
        let server = spawn_server(faults);
        let mut opts = options(server.url(), &dir.join("f.bin"), 2);
        opts.temp_dir = Some(dir.join("t"));
        download(opts).unwrap();
        assert!(mtime_secs(&dir.join("f.bin")) > 1_600_000_000);
    }

    #[test]
    fn unknown_length_eof_is_flagged_unverified() {
        use std::io::{BufRead, BufReader};
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                        break;
                    }
                }
                let mut stream = stream;
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nhello world");
            }
        });
        let dir = workspace("unknown");
        let control = Arc::new(TransferControl::new(None).unwrap());
        let opts = options(format!("http://127.0.0.1:{port}/x"), &dir.join("x.bin"), 1);
        download_with_control(opts, control.clone()).unwrap();
        assert_eq!(fs::read(dir.join("x.bin")).unwrap(), b"hello world");
        assert!(control.size_unverified());
    }

    #[test]
    fn live_segments_report_received_bytes_before_any_flush() {
        let len = 4 * 1024 * 1024;
        let server = spawn_server(Faults::new(len));
        let dir = workspace("live-segments");
        let output = dir.join("f.bin");
        let opts = options(server.url(), &output, 4);
        let stop = opts.stop.clone();
        let control = Arc::new(TransferControl::new(Some(400 * 1024)).unwrap());
        let runner = {
            let control = control.clone();
            thread::spawn(move || download_with_control(opts, control))
        };
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut live_total = 0;
        while Instant::now() < deadline {
            let live = control.live_segments().unwrap_or_default();
            live_total = live.iter().map(|(start, _, cursor)| cursor - start).sum::<u64>();
            if live_total > 0 {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let flushed = checkpoint_segments_in(&output, None)
            .unwrap_or_default()
            .iter()
            .map(|(start, _, cursor)| cursor - start)
            .sum::<u64>();
        stop.store(true, Ordering::Relaxed);
        let _ = runner.join();
        assert!(live_total > 0, "no received bytes were reported");
        // Received bytes are a display value; the checkpoint still holds flushed bytes only.
        assert!(flushed < live_total, "flushed {flushed} live {live_total}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn declared_length_download_is_not_flagged() {
        let dir = workspace("known");
        let mut faults = Faults::new(32 * 1024);
        faults.ignore_ranges = true;
        let server = spawn_server(faults);
        let control = Arc::new(TransferControl::new(None).unwrap());
        download_with_control(options(server.url(), &dir.join("f.bin"), 1), control.clone()).unwrap();
        assert!(!control.size_unverified());
    }

    #[test]
    fn stalled_range_connection_is_abandoned_and_reopened_from_cursor() {
        let len = 512 * 1024;
        let mut faults = Faults::new(len);
        faults.stall_attempt = Some(2);
        let server = spawn_server(faults);
        let dir = workspace("range-stall");
        let output = dir.join("f.bin");
        // Long enough that a starved healthy worker is not mistaken for a stall.
        let stall_timeout = Duration::from_secs(3);
        let started = Instant::now();
        download_with_stall_timeout(
            options(server.url(), &output, 4),
            Arc::new(TransferControl::new(None).unwrap()),
            stall_timeout,
        )
        .unwrap();
        // The stalled server thread sleeps 30 s; finishing well before that means the
        // connection was abandoned rather than waited out. The bound is deliberately
        // loose so CPU starvation cannot trip it; the reopened span below is the
        // precise proof.
        let elapsed = started.elapsed();
        assert!(elapsed >= stall_timeout, "finished before any stall could be detected: {elapsed:?}");
        assert!(elapsed < Duration::from_secs(25), "stalled connection was waited out: {elapsed:?}");
        assert_pattern(&output, len);
        // The probe, three workers and at least one reopen of the stalled range.
        assert!(server.total() >= 5, "{} requests", server.total());
        let spans = server.closed_spans();
        assert!(
            spans
                .iter()
                .any(|&(start, end)| spans.iter().any(|&(other, _)| other > start && other <= end)),
            "no request reopened inside the stalled range: {spans:?}"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unreliable_ranges_stop_workers_and_keep_a_resumable_checkpoint() {
        let len = 512 * 1024;
        let mut faults = Faults::new(len);
        faults.flip_after = Some(2);
        faults.flip_delay = Duration::from_millis(150);
        let server = spawn_server(faults);
        let dir = workspace("flip");
        let output = dir.join("f.bin");
        let error = download_with_stall_timeout(
            options(server.url(), &output, 4),
            Arc::new(TransferControl::new(None).unwrap()),
            Duration::from_secs(1),
        )
        .unwrap_err();
        let typed = range_unreliable(error.as_ref()).expect(&error.to_string());
        assert!(typed.reason.contains("Content-Range"), "{}", typed.reason);
        assert!(error.to_string().contains("--restart"));
        assert!(!output.exists());
        assert!(dir.join("f.bin.part").exists());
        let meta: Checkpoint =
            serde_json::from_reader(File::open(dir.join("f.bin.part.meta")).unwrap()).unwrap();
        let done: u64 = meta.segments.iter().map(|s| s.cursor - s.start).sum();
        assert!(0 < done && done < len, "{done} of {len} bytes checkpointed");
        // Every worker stopped: only the probe and the honest range requests were made.
        assert!(server.total() <= 4, "{} requests", server.total());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rate_limited_transfer_is_not_treated_as_a_stall() {
        let len = 128 * 1024;
        let server = spawn_server(Faults::new(len));
        let dir = workspace("throttle");
        let output = dir.join("f.bin");
        let started = Instant::now();
        download_with_stall_timeout(
            options(server.url(), &output, 4),
            Arc::new(TransferControl::new(Some(32 * 1024)).unwrap()),
            // The transfer outlives the stall timeout many times over; the limiter's
            // waits happen between reads, so the socket idle clock never sees them.
            Duration::from_millis(500),
        )
        .unwrap();
        assert!(started.elapsed() >= Duration::from_secs(3));
        assert_pattern(&output, len);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn stalled_single_stream_restarts_from_zero() {
        let len = 128 * 1024;
        let mut faults = Faults::new(len);
        faults.ignore_ranges = true;
        faults.stall_attempt = Some(1);
        let server = spawn_server(faults);
        let dir = workspace("stream-stall");
        let output = dir.join("f.bin");
        download_with_stall_timeout(
            options(server.url(), &output, 2),
            Arc::new(TransferControl::new(None).unwrap()),
            Duration::from_secs(1),
        )
        .unwrap();
        assert_pattern(&output, len);
        // The adopted probe response stalled; exactly one retry from byte zero followed.
        assert_eq!(server.total(), 2);
        fs::remove_dir_all(dir).unwrap();
    }
}
