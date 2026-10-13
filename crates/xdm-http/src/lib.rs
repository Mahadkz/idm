//! Independent HTTP transfer engine. No browser or GUI dependencies.

use reqwest::{Method, StatusCode};
use reqwest::blocking::{Client, Response};
use reqwest::header::{
    ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE, ETAG, HeaderMap, HeaderName,
    HeaderValue, IF_RANGE, LAST_MODIFIED, PROXY_AUTHENTICATE, RANGE, RETRY_AFTER,
    WWW_AUTHENTICATE,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::error::Error;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Debug, Clone)]
pub struct Options {
    pub url: String,
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
            output: PathBuf::new(),
            connections: 8,
            restart: false,
            headers: Vec::new(),
            method: "GET".into(),
            body: None,
            bytes_per_second: None,
            proxy: Proxy::System,
            stop: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// Shared controls and live counters for a single HTTP transfer.
#[derive(Debug)]
pub struct TransferControl {
    limiter: Mutex<Bucket>,
    limited: AtomicBool,
    downloaded: AtomicU64,
    total: AtomicU64,
    total_known: AtomicBool,
    sample: Mutex<Sample>,
}

#[derive(Debug)]
struct Bucket {
    rate: Option<u64>,
    tokens: f64,
    updated: Instant,
}

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
        Ok(Self {
            limited: AtomicBool::new(rate.is_some()),
            limiter: Mutex::new(Bucket {
                rate,
                tokens: 0.0,
                updated: Instant::now(),
            }),
            downloaded: AtomicU64::new(0),
            total: AtomicU64::new(0),
            total_known: AtomicBool::new(false),
            sample: Mutex::new(Sample {
                at: Instant::now(),
                bytes: 0,
                speed: 0,
            }),
        })
    }

    pub fn set_rate(&self, rate: Option<u64>) -> Result<()> {
        if rate == Some(0) {
            return Err(err("rate limit must be greater than zero"));
        }
        let mut bucket = self
            .limiter
            .lock()
            .map_err(|_| err("rate limiter lock poisoned"))?;
        bucket.rate = rate;
        bucket.tokens = 0.0;
        bucket.updated = Instant::now();
        self.limited.store(rate.is_some(), Ordering::Relaxed);
        Ok(())
    }

    fn initialize(&self, done: u64, total: Option<u64>) {
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

    fn wait(&self, bytes: usize, stop: &AtomicBool) -> Result<()> {
        if !self.limited.load(Ordering::Relaxed) {
            return Ok(());
        }
        let mut remaining = bytes as f64;
        while remaining > 0.0 {
            if stop.load(Ordering::Relaxed) {
                return Err(err("paused"));
            }
            let sleep = {
                let mut bucket = self
                    .limiter
                    .lock()
                    .map_err(|_| err("rate limiter lock poisoned"))?;
                let Some(rate) = bucket.rate else {
                    return Ok(());
                };
                let now = Instant::now();
                let capacity = (rate as f64 * 0.1).max(1.0);
                bucket.tokens = (bucket.tokens
                    + now.duration_since(bucket.updated).as_secs_f64() * rate as f64)
                    .min(capacity);
                bucket.updated = now;
                let granted = bucket.tokens.min(remaining);
                bucket.tokens -= granted;
                remaining -= granted;
                Duration::from_secs_f64((remaining / rate as f64).min(0.05))
            };
            if remaining > 0.0 {
                thread::sleep(sleep.max(Duration::from_millis(1)));
            }
        }
        Ok(())
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
    let path = meta_path(output);
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
    /// First response, kept so its body is adopted instead of requesting the resource again.
    response: Option<Response>,
    /// Byte offset at which the adopted body begins.
    offset: u64,
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

/// Replayable request identity shared by the probe and every transfer request.
#[derive(Debug, Clone)]
struct Request {
    url: String,
    method: Method,
    body: Option<Vec<u8>>,
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
        })
    }

    fn send(
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
    let value = response.headers().get(header_name)?.to_str().ok()?;
    let (scheme, realm) = parse_challenge(value)?;
    Some(Box::new(AuthRequired(AuthChallenge {
        status: status.as_u16(),
        scheme,
        realm,
    })))
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

fn split_segments(length: u64, workers: usize) -> Vec<Segment> {
    if length == 0 {
        return Vec::new();
    }
    let count = workers.max(1).min(16).min(length as usize);
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

fn part_path(output: &Path) -> PathBuf {
    PathBuf::from(format!("{}.part", output.display()))
}

fn meta_path(output: &Path) -> PathBuf {
    PathBuf::from(format!("{}.part.meta", output.display()))
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

fn validators_match(saved: &Checkpoint, current: &Probe) -> bool {
    if Some(saved.length) != current.length || saved.effective_url != current.effective_url {
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
    if etag.is_none() && modified.is_none() {
        return false;
    }
    etag.is_none_or(|(a, b)| a == b) && modified.is_none_or(|(a, b)| a == b)
}

/// Sizes the partial file. Linux allocates real blocks so concurrent range writes do not
/// allocate under the file lock; other systems, or filesystems without fallocate, get a
/// plain (possibly sparse) length.
fn preallocate(file: &File, length: u64) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        // SAFETY: fallocate only reads the descriptor and sizes we pass.
        if unsafe { libc::fallocate(file.as_raw_fd(), 0, 0, length as libc::off_t) } == 0 {
            return Ok(());
        }
    }
    file.set_len(length)
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


fn client(headers: &[(String, String)], method: &Method, proxy: &Proxy) -> Result<Client> {
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
    let mut builder = Client::builder()
        .timeout(Duration::from_secs(30))
        .default_headers(map)
        .referer(false)
        .redirect(policy);
    match proxy {
        Proxy::System => {}
        Proxy::Direct => builder = builder.no_proxy(),
        Proxy::Manual {
            http,
            https,
            bypass,
        } => {
            builder = builder.no_proxy();
            let bypass = reqwest::NoProxy::from_string(&bypass.join(","));
            if let Some(url) = http {
                builder = builder.proxy(reqwest::Proxy::http(url)?.no_proxy(bypass.clone()));
            }
            if let Some(url) = https {
                builder = builder.proxy(reqwest::Proxy::https(url)?.no_proxy(bypass));
            }
        }
    }
    Ok(builder.build()?)
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
    for attempt in 0..=RETRIES {
        let after = match request.send(client, range.clone(), None) {
            Ok(r) if retryable(r.status()) => {
                last = format!("HTTP {}", r.status());
                retry_after(&r)
            }
            Ok(r) => return Ok(r),
            Err(e) if e.is_redirect() => return Err(err(describe(&e))),
            Err(e) => {
                last = describe(&e);
                None
            }
        };
        if attempt < RETRIES {
            wait_retry(attempt, after, stop)?;
        }
    }
    Err(err(format!("request retries exhausted: {last}")))
}

/// Opens the resource once, from `offset` to its end, and keeps the response for adoption.
/// A server that ignores or garbles the range is asked once more for the whole representation.
fn probe(client: &Client, request: &Request, offset: u64, stop: &AtomicBool) -> Result<Probe> {
    let response = retry_send(client, request, Some(format!("bytes={offset}-")), stop)?;
    let status = response.status();
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
                return Err(challenge_error(&full)
                    .unwrap_or_else(|| err(format!("download returned HTTP {}", full.status()))));
            }
            full
        }
        status => {
            return Err(challenge_error(&response)
                .unwrap_or_else(|| err(format!("probe failed: HTTP {status}"))))
        }
    };
    Ok(Probe {
        effective_url: response.url().to_string(),
        length: header(&response, CONTENT_LENGTH).and_then(|s| s.parse().ok()),
        ranges: false,
        etag: header(&response, ETAG),
        last_modified: header(&response, LAST_MODIFIED),
        response: Some(response),
        offset: 0,
    })
}

/// Installs the completed file without replacing anything created at the destination meanwhile.
fn publish(part: &Path, output: &Path) -> Result<()> {
    match fs::hard_link(part, output) {
        Ok(()) => Ok(fs::remove_file(part)?),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(err(format!(
            "{} appeared during the download; the completed data was kept at {}",
            output.display(),
            part.display()
        ))),
        // Filesystems without hard links fall back to a checked rename.
        Err(_) if fs::symlink_metadata(output).is_err() => Ok(fs::rename(part, output)?),
        Err(_) => Err(err(format!(
            "{} appeared during the download; the completed data was kept at {}",
            output.display(),
            part.display()
        ))),
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
            },
        );
        store_checkpoint(&self.file, &self.meta, &guard)?;
        Ok(Some(middle))
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
            return Err(err("paused"));
        }
        let Some(start) = shared.claim()? else {
            return Ok(());
        };
        match transfer_segment(&shared, start)? {
            Outcome::Done => {}
            Outcome::Refused(status) => {
                shared.release(start)?;
                if let Ok(mut refused) = shared.refused.lock() {
                    refused.get_or_insert(status);
                }
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
                    Err(_) if failures < RETRIES => {
                        failures += 1;
                        wait_retry(failures, None, &shared.stop)?;
                        continue;
                    }
                    Err(e) => return Err(e.into()),
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
                    return Err(err(format!(
                        "range request returned HTTP {status} at byte {cursor}"
                    )));
                }
                if !identity_encoded(&response)
                    || response.url().as_str() != effective_url
                    || header(&response, CONTENT_RANGE)
                        .as_deref()
                        .and_then(parse_content_range)
                        != Some((cursor, end, total))
                {
                    return Err(err(format!("invalid Content-Range at byte {cursor}")));
                }
                if etag
                    .as_ref()
                    .zip(header(&response, ETAG).as_ref())
                    .is_some_and(|(old, new)| old != new)
                    || last_modified
                        .as_ref()
                        .zip(header(&response, LAST_MODIFIED).as_ref())
                        .is_some_and(|(old, new)| old != new)
                {
                    return Err(err("remote validators changed during range transfer"));
                }
                if let Some(value) = header(&response, CONTENT_LENGTH) {
                    if value.parse::<u64>().ok() != Some(end - cursor + 1) {
                        return Err(err("invalid range body length"));
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
            if shared.stop.load(Ordering::Relaxed) {
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
            match response.read(&mut buffer[filled..filled + want]) {
                Ok(0) => {
                    failed = true;
                    break;
                }
                Ok(n) => {
                    shared.control.wait(n, &shared.stop)?;
                    shared
                        .control
                        .downloaded
                        .fetch_add(n as u64, Ordering::Relaxed);
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
                    return Err(err("range body exceeded declared end"));
                }
            }
            shared.release(start)?;
            return Ok(Outcome::Done);
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
) -> Result<bool> {
    control.initialize(0, length);
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
        let n = match response.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => return Ok(false),
        };
        control.wait(n, &options.stop)?;
        output.write_all(&buffer[..n])?;
        control.downloaded.fetch_add(n as u64, Ordering::Relaxed);
        total += n as u64;
    }
    if length.is_some_and(|length| length != total) {
        return Ok(false);
    }
    output.flush()?;
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
                    return Err(challenge_error(&response).unwrap_or_else(|| {
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
        if stream_body(&mut response, part, probe.length, options, control)? {
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
    let part = part_path(&options.output);
    let meta = meta_path(&options.output);
    if options.restart {
        let _ = fs::remove_file(&part);
        let _ = fs::remove_file(&meta);
    }
    let request = Request::new(&options)?;
    let client = client(&options.headers, &request.method, &options.proxy)?;
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
    let mut probe = probe(&client, &request, offset, &options.stop)?;
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
            || !validators_match(&saved, &probe)
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
            segments: split_segments(length, options.connections),
        }
    };
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
    });
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
    shared.file.sync_all()?;
    drop(shared.file);
    publish(&part, &options.output)?;
    fs::remove_file(&meta)?;
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
            status: 401,
            scheme: "Basic".into(),
            realm: Some("xdm-test".into()),
        };
        let error: Box<dyn Error + Send + Sync> = Box::new(AuthRequired(challenge.clone()));
        assert_eq!(auth_challenge(error.as_ref()), Some(challenge));
        assert_eq!(auth_challenge(err("probe failed: HTTP 403").as_ref()), None);
        assert_eq!(
            AuthRequired(AuthChallenge {
                status: 401,
                scheme: "Basic".into(),
                realm: Some("xdm-test".into())
            })
            .to_string(),
            "authentication required: HTTP 401 Basic realm \"xdm-test\""
        );
    }
}

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
            },
            file,
            meta,
            state: Mutex::new(state),
            stop: Arc::new(AtomicBool::new(false)),
            control: Arc::new(TransferControl::new(None).unwrap()),
            adopted: Mutex::new(None),
            refused: Mutex::new(None),
            checkpointed: Mutex::new(Instant::now()),
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
