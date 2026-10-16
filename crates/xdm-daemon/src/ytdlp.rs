//! Optional external yt-dlp: format probing (`-J`) and a fallback downloader for formats the
//! engine cannot fetch itself. yt-dlp is never required; without it a clear error says how to
//! install it. It is spawned without a shell, with a fixed argument vector: the only caller
//! supplied strings are the page URL (validated http/https, placed after `--`), a format
//! selector built from validated ids, and file paths.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};
use xdm_http::{Login, Proxy};
use xdm_ipc::{ProbeCookie, ProbeFormat, ProbeResult};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub const INSTALL_HINT: &str = "yt-dlp was not found. Install it (for example `pipx install yt-dlp`, `brew install yt-dlp` or from https://github.com/yt-dlp/yt-dlp#installation) and make sure it is on PATH, or set its location in XDM settings.";
const PROBE_TIMEOUT: Duration = Duration::from_secs(45);
const MAX_JSON_BYTES: usize = 32 * 1024 * 1024;
const CACHE_TTL: Duration = Duration::from_secs(300);
const MAX_FORMATS: usize = 400;

/// Marker in an error string for "yt-dlp is not installed", so callers can flag it.
pub fn is_missing(message: &str) -> bool {
    message.starts_with("yt-dlp was not found")
}

static CONFIGURED: Mutex<Option<String>> = Mutex::new(None);
static COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn set_configured(path: Option<String>) {
    if let Ok(mut slot) = CONFIGURED.lock() {
        *slot = path.filter(|path| !path.is_empty());
    }
}

pub fn configured() -> Option<String> {
    CONFIGURED.lock().ok().and_then(|slot| slot.clone())
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidates: &[&str] = if cfg!(windows) { &[".exe", ".cmd", ".bat", ""] } else { &[""] };
        for suffix in candidates {
            let candidate = dir.join(format!("{name}{suffix}"));
            if is_runnable(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn is_runnable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else { return false };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// The configured path (must be an existing program), otherwise `yt-dlp` on PATH.
pub fn locate() -> Option<PathBuf> {
    if let Some(path) = configured() {
        let path = PathBuf::from(path);
        return is_runnable(&path).then_some(path);
    }
    which("yt-dlp")
}

/// `--js-runtimes` for the best JavaScript runtime that is installed but not yt-dlp's default
/// (deno): YouTube's signature code needs one.
fn js_runtime_args() -> Vec<String> {
    if which("deno").is_some() {
        return Vec::new();
    }
    for name in ["node", "quickjs", "bun"] {
        if let Some(path) = which(name) {
            return vec!["--js-runtimes".into(), format!("{name}:{}", path.display())];
        }
    }
    Vec::new()
}

/// Page URLs handed to yt-dlp: plain web addresses only, never something it could read as an option.
pub fn validate_url(url: &str) -> std::result::Result<(), String> {
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err("only http and https page addresses can be probed".into());
    }
    if url.len() > 2048 || url.chars().any(|c| c.is_control() || c == ' ') {
        return Err("invalid page address".into());
    }
    Ok(())
}

/// A format selector is one or two format ids joined by `+`; ids are plain tokens.
pub fn valid_selector(selector: &str) -> bool {
    let parts: Vec<&str> = selector.split('+').collect();
    (1..=2).contains(&parts.len())
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.len() <= 64
                && !part.starts_with(['-', '.'])
                && part.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        })
}

pub struct ProxyArgs {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// Maps the daemon's proxy mode to yt-dlp. A proxy login goes through the environment (never argv).
pub fn proxy_args(proxy: &Proxy, login: Option<&Login>) -> ProxyArgs {
    let mut out = ProxyArgs { args: Vec::new(), env: Vec::new() };
    match proxy {
        Proxy::System => {}
        Proxy::Direct => out.args.extend(["--proxy".to_string(), String::new()]),
        Proxy::Manual { http, https, .. } => {
            let Some(url) = https.clone().or_else(|| http.clone()).filter(|url| !url.is_empty()) else { return out };
            match login.and_then(|login| url_with_login(&url, login)) {
                Some(with_login) => {
                    for name in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
                        out.env.push((name.into(), with_login.clone()));
                    }
                }
                None => out.args.extend(["--proxy".to_string(), url]),
            }
        }
    }
    out
}

fn url_with_login(url: &str, login: &Login) -> Option<String> {
    let mut parsed = url::Url::parse(url).ok()?;
    parsed.set_username(&login.username).ok()?;
    parsed.set_password(Some(&login.password)).ok()?;
    Some(parsed.to_string())
}

/// A private directory (0700) removed with the guard; holds the cookie jar for one run.
struct PrivateDir(PathBuf);

impl PrivateDir {
    fn create() -> Result<Self> {
        for _ in 0..16 {
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
            let dir = std::env::temp_dir().join(format!("xdm-ytdlp-{}-{nanos:x}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));
            #[cfg_attr(not(unix), allow(unused_mut))]
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&dir) {
                Ok(()) => return Ok(Self(dir)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err("cannot create a private temporary directory".into())
    }
}

impl Drop for PrivateDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write_cookie_file(dir: &Path, cookies: &[ProbeCookie]) -> Result<PathBuf> {
    use std::io::Write;
    let path = dir.join("cookies.txt");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    let bad = |text: &str| text.contains(['\t', '\r', '\n', '\0']);
    writeln!(file, "# Netscape HTTP Cookie File")?;
    for cookie in cookies.iter().take(2000) {
        if bad(&cookie.name) || bad(&cookie.value) || bad(&cookie.domain) || bad(&cookie.path) || cookie.domain.is_empty() {
            continue;
        }
        writeln!(
            file,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            cookie.domain,
            if cookie.domain.starts_with('.') { "TRUE" } else { "FALSE" },
            if cookie.path.is_empty() { "/" } else { &cookie.path },
            if cookie.secure { "TRUE" } else { "FALSE" },
            cookie.expires.filter(|expires| *expires > 0).unwrap_or(0),
            cookie.name,
            cookie.value
        )?;
    }
    Ok(path)
}

fn command(program: &Path, proxy: &ProxyArgs) -> Command {
    let mut command = Command::new(program);
    command.stdin(Stdio::null()).env("PYTHONIOENCODING", "utf-8");
    for (name, value) in &proxy.env {
        command.env(name, value);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
}

fn kill(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// The last `ERROR:` line of yt-dlp's stderr, bounded.
fn error_summary(stderr: &str) -> String {
    let line = stderr
        .lines()
        .rev()
        .find(|line| line.contains("ERROR"))
        .or_else(|| stderr.lines().rev().find(|line| !line.trim().is_empty()))
        .unwrap_or("no details");
    let line = line.trim().trim_start_matches("ERROR:").trim();
    format!("yt-dlp failed: {}", line.chars().take(300).collect::<String>())
}

/// Runs `yt-dlp -J` and returns its JSON. Stops on timeout, daemon shutdown or `stop`.
fn run_json(args: &[String], proxy: &ProxyArgs, stop: &AtomicBool) -> Result<serde_json::Value> {
    let program = locate().ok_or(INSTALL_HINT)?;
    let mut child = command(&program, proxy)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot start yt-dlp: {error}"))?;
    let mut stdout = child.stdout.take().ok_or("no stdout")?;
    let mut stderr = child.stderr.take().ok_or("no stderr")?;
    let out_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = (&mut stdout).take(MAX_JSON_BYTES as u64 + 1).read_to_end(&mut buffer);
        buffer
    });
    let err_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = (&mut stderr).take(64 * 1024).read_to_end(&mut buffer);
        buffer
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if stop.load(Ordering::Relaxed) || crate::shutdown::requested() {
            kill(&mut child);
            return Err("probe cancelled".into());
        }
        if started.elapsed() > PROBE_TIMEOUT {
            kill(&mut child);
            return Err("yt-dlp timed out while reading the page".into());
        }
        std::thread::sleep(Duration::from_millis(40));
    };
    let stdout = out_reader.join().unwrap_or_default();
    let stderr = String::from_utf8_lossy(&err_reader.join().unwrap_or_default()).into_owned();
    if !status.success() {
        return Err(error_summary(&stderr).into());
    }
    if stdout.len() > MAX_JSON_BYTES {
        return Err("yt-dlp output is too large".into());
    }
    serde_json::from_slice(&stdout).map_err(|_| "yt-dlp returned unreadable output".into())
}

fn str_of(value: &serde_json::Value, key: &str) -> String {
    value.get(key).and_then(|v| v.as_str()).unwrap_or("").to_owned()
}

fn codec(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key).and_then(|v| v.as_str()).map(str::to_owned).filter(|c| c != "none")
}

/// yt-dlp `-J` output to the normalized result; storyboards and formats without a stream are dropped.
pub fn normalize(page_url: &str, json: &serde_json::Value) -> Result<ProbeResult> {
    if json.get("_type").and_then(|v| v.as_str()) == Some("playlist") {
        return Err("this page is a playlist; open one video".into());
    }
    let live = json.get("is_live").and_then(|v| v.as_bool()).unwrap_or(false)
        || matches!(json.get("live_status").and_then(|v| v.as_str()), Some("is_live" | "is_upcoming" | "post_live"));
    let mut formats = Vec::new();
    for raw in json.get("formats").and_then(|v| v.as_array()).into_iter().flatten().take(MAX_FORMATS) {
        let id = str_of(raw, "format_id");
        let url = str_of(raw, "url");
        let protocol = str_of(raw, "protocol");
        let ext = str_of(raw, "ext");
        if id.is_empty() || url.is_empty() || protocol == "mhtml" || ext == "mhtml" || str_of(raw, "format_note") == "storyboard" {
            continue;
        }
        let vcodec = codec(raw, "vcodec");
        let acodec = codec(raw, "acodec");
        let has_dims = raw.get("height").is_some_and(|h| h.is_number());
        let unknown = |key: &str| raw.get(key).is_none_or(|v| v.is_null());
        // yt-dlp reports "none" for an absent stream and null for an unknown one (plain files).
        let kind = match (vcodec.is_some(), acodec.is_some()) {
            (true, true) => "muxed",
            (true, false) if unknown("acodec") => "muxed",
            (true, false) => "video",
            (false, true) if unknown("vcodec") && has_dims => "muxed",
            (false, true) => "audio",
            (false, false) if unknown("vcodec") && unknown("acodec") && has_dims => "muxed",
            (false, false) => continue,
        };
        let exact = raw.get("filesize").and_then(|v| v.as_u64());
        let approx = raw.get("filesize_approx").and_then(|v| v.as_f64()).map(|v| v as u64);
        let headers = raw
            .get("http_headers")
            .and_then(|v| v.as_object())
            .map(|map| map.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_owned()))).collect())
            .unwrap_or_default();
        formats.push(ProbeFormat {
            id,
            ext,
            kind: kind.into(),
            width: raw.get("width").and_then(|v| v.as_u64()),
            height: raw.get("height").and_then(|v| v.as_u64()),
            fps: raw.get("fps").and_then(|v| v.as_f64()),
            vcodec: vcodec.unwrap_or_default(),
            acodec: acodec.unwrap_or_default(),
            tbr: raw.get("tbr").and_then(|v| v.as_f64()),
            abr: raw.get("abr").and_then(|v| v.as_f64()),
            filesize: exact.or(approx),
            filesize_approx: exact.is_none() && approx.is_some(),
            protocol,
            note: str_of(raw, "format_note"),
            language: raw.get("language").and_then(|v| v.as_str()).map(str::to_owned),
            url,
            headers,
        });
    }
    Ok(ProbeResult {
        page_url: page_url.to_owned(),
        title: str_of(json, "title").chars().take(300).collect(),
        duration: json.get("duration").and_then(|v| v.as_f64()),
        thumbnail: json.get("thumbnail").and_then(|v| v.as_str()).map(str::to_owned),
        extractor: str_of(json, "extractor_key"),
        live,
        formats,
    })
}

static CACHE: Mutex<Option<HashMap<String, (Instant, ProbeResult)>>> = Mutex::new(None);

/// Cache key: YouTube watch, shorts, embed, live and youtu.be addresses share one entry per video id
/// (extra parameters such as `t=` or `list=` do not matter); everything else is keyed by URL.
pub fn cache_key(url: &str) -> String {
    youtube_id(url).map(|id| format!("youtube:{id}")).unwrap_or_else(|| url.to_owned())
}

fn youtube_id(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    let host = host.strip_prefix("www.").or_else(|| host.strip_prefix("m.")).or_else(|| host.strip_prefix("music.")).unwrap_or(&host);
    let id = if host == "youtu.be" {
        parsed.path_segments()?.next().map(str::to_owned)
    } else if host == "youtube.com" || host == "youtube-nocookie.com" {
        let mut segments = parsed.path_segments()?;
        match segments.next()? {
            "watch" => parsed.query_pairs().find(|(k, _)| k == "v").map(|(_, v)| v.into_owned()),
            "shorts" | "embed" | "live" | "v" => segments.next().map(str::to_owned),
            _ => None,
        }
    } else {
        None
    }?;
    (!id.is_empty() && id.len() <= 32 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')).then_some(id)
}

fn cache_get(key: &str) -> Option<ProbeResult> {
    let guard = CACHE.lock().ok()?;
    guard.as_ref()?.get(key).filter(|(at, _)| at.elapsed() < CACHE_TTL).map(|(_, result)| result.clone())
}

fn cache_put(key: &str, result: &ProbeResult) {
    if let Ok(mut guard) = CACHE.lock() {
        let map = guard.get_or_insert_with(HashMap::new);
        map.retain(|_, (at, _)| at.elapsed() < CACHE_TTL);
        if map.len() >= 16 {
            map.clear();
        }
        map.insert(key.to_owned(), (Instant::now(), result.clone()));
    }
}

/// True when yt-dlp's error says the video needs a signed-in account (login, age, members, private,
/// bot check), the only case where forwarding browser cookies can help.
pub fn needs_login(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    ["sign in", "log in", "login", "your age", "age-restricted", "age restricted", "age verification", "inappropriate for some", "members-only", "members only", "join this channel", "private video", "confirm you", "not a bot", "cookies", "authentication", "premium"]
        .iter()
        .any(|needle| lower.contains(needle))
}

/// Cookieless first; one retry with cookies only when the first error needs an account. If the retry
/// also fails, the cookieless error is the one reported (a partial cookie jar can cause its own errors).
fn probe_ordered(cookies_available: bool, mut run: impl FnMut(bool) -> Result<ProbeResult>) -> Result<ProbeResult> {
    match run(false) {
        Ok(result) => Ok(result),
        Err(first) if cookies_available && needs_login(&first.to_string()) => run(true).map_err(|_| first),
        Err(first) => Err(first),
    }
}

fn probe_once(page_url: &str, cookies: Option<&[ProbeCookie]>, proxy: &ProxyArgs, stop: &AtomicBool) -> Result<ProbeResult> {
    let mut args: Vec<String> = ["-J", "--no-playlist", "--no-warnings", "--no-cache-dir"].map(String::from).to_vec();
    args.extend(proxy.args.iter().cloned());
    args.extend(js_runtime_args());
    let dir;
    if let Some(cookies) = cookies {
        dir = PrivateDir::create()?;
        let file = write_cookie_file(&dir.0, cookies)?;
        args.push("--cookies".into());
        args.push(file.to_string_lossy().into_owned());
    }
    args.push("--".into());
    args.push(page_url.to_owned());
    normalize(page_url, &run_json(&args, proxy, stop)?)
}

/// Probes `page_url`. Results are cached per URL (per video id for YouTube) for a few minutes (stream
/// addresses expire) unless `refresh` is set. The first attempt carries no cookies; they are used only for
/// a retry when yt-dlp reports that an account is needed. Cookies live in a 0600 file inside a private
/// directory that is deleted before returning.
pub fn probe(page_url: &str, cookies: &[ProbeCookie], refresh: bool, proxy: &ProxyArgs, stop: &AtomicBool) -> Result<ProbeResult> {
    validate_url(page_url)?;
    let key = cache_key(page_url);
    if !refresh && let Some(hit) = cache_get(&key) {
        return Ok(hit);
    }
    let result = probe_ordered(!cookies.is_empty(), |with_cookies| probe_once(page_url, with_cookies.then_some(cookies), proxy, stop))?;
    cache_put(&key, &result);
    Ok(result)
}

/// Downloads `selector` of `page_url` with yt-dlp itself into `output`, through a sibling temporary
/// name that is published without replacing an existing file. `progress(downloaded, total)` is called
/// as lines arrive. Stopping kills yt-dlp; running again continues the partial files.
pub fn download(
    page_url: &str,
    selector: &str,
    output: &Path,
    expected_total: Option<u64>,
    proxy: &ProxyArgs,
    stop: &AtomicBool,
    progress: &dyn Fn(u64, Option<u64>),
) -> Result<()> {
    validate_url(page_url)?;
    if !valid_selector(selector) {
        return Err("invalid format selection".into());
    }
    if output.exists() {
        return Err("media destination already exists".into());
    }
    let program = locate().ok_or(INSTALL_HINT)?;
    let extension = output.extension().and_then(|e| e.to_str()).unwrap_or("mp4").to_ascii_lowercase();
    let name = output.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "video".into());
    let temporary = output.with_file_name(format!("{name}.xdm-ytdlp.{extension}"));
    let template = temporary.to_string_lossy().replace('%', "%%");
    let mut args: Vec<String> = [
        "-f", selector, "--newline", "--no-playlist", "--no-warnings", "--no-cache-dir", "--continue", "--no-overwrites",
        "--progress-template", "download:XDMP %(progress.downloaded_bytes)s %(progress.total_bytes)s %(progress.total_bytes_estimate)s",
    ]
    .map(String::from)
    .to_vec();
    if matches!(extension.as_str(), "mp4" | "mkv" | "webm") {
        args.extend(["--merge-output-format".to_string(), extension.clone()]);
    }
    args.extend(["-o".to_string(), template]);
    args.extend(proxy.args.iter().cloned());
    args.extend(js_runtime_args());
    args.push("--".into());
    args.push(page_url.to_owned());
    let mut child = command(&program, proxy)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot start yt-dlp: {error}"))?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let mut stderr = child.stderr.take().ok_or("no stderr")?;
    let (sender, receiver) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(std::result::Result::ok) {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let err_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = (&mut stderr).take(64 * 1024).read_to_end(&mut buffer);
        buffer
    });
    // Two-format jobs download one stream after the other: add the finished stream's bytes back in.
    let (mut base, mut last) = (0u64, 0u64);
    let status = loop {
        if stop.load(Ordering::Relaxed) || crate::shutdown::requested() {
            kill(&mut child);
            return Err("stopped".into());
        }
        match receiver.recv_timeout(Duration::from_millis(150)) {
            Ok(line) => {
                if let Some((done, total)) = parse_progress(&line) {
                    if done < last {
                        base += last;
                    }
                    last = done;
                    progress(base + done, expected_total.or(total.filter(|_| base == 0)));
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break child.wait()?,
        }
    };
    let stderr = String::from_utf8_lossy(&err_reader.join().unwrap_or_default()).into_owned();
    if !status.success() {
        return Err(error_summary(&stderr).into());
    }
    if !temporary.exists() {
        return Err("yt-dlp finished without producing the expected file".into());
    }
    if output.exists() {
        return Err("media destination already exists".into());
    }
    if std::fs::hard_link(&temporary, output).is_ok() {
        let _ = std::fs::remove_file(&temporary);
    } else {
        std::fs::rename(&temporary, output)?;
    }
    Ok(())
}

/// `XDMP downloaded total estimate` with `NA` for unknown values.
fn parse_progress(line: &str) -> Option<(u64, Option<u64>)> {
    let mut parts = line.strip_prefix("XDMP ")?.split_whitespace();
    let number = |text: Option<&str>| text.and_then(|t| t.parse::<f64>().ok()).filter(|n| *n >= 0.0).map(|n| n as u64);
    let done = number(parts.next())?;
    let total = number(parts.next()).or_else(|| number(parts.next()));
    Some((done, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_and_selectors_are_validated() {
        assert!(validate_url("https://www.youtube.com/watch?v=abc").is_ok());
        assert!(validate_url("-o /tmp/x").is_err());
        assert!(validate_url("--exec=rm https://x").is_err());
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("https://x/ y").is_err());
        assert!(valid_selector("137+140"));
        assert!(valid_selector("hls-1080p"));
        assert!(!valid_selector("-x"));
        assert!(!valid_selector("a+b+c"));
        assert!(!valid_selector("bestvideo[height<=1080]"));
        assert!(!valid_selector(""));
    }

    #[test]
    fn progress_lines_parse() {
        assert_eq!(parse_progress("XDMP 100 NA 400"), Some((100, Some(400))));
        assert_eq!(parse_progress("XDMP 100 300 NA"), Some((100, Some(300))));
        assert_eq!(parse_progress("XDMP NA NA NA"), None);
        assert_eq!(parse_progress("[download] 5%"), None);
    }

    #[test]
    fn normalize_classifies_formats() {
        let json = serde_json::json!({
            "title": "T", "duration": 10.0, "extractor_key": "Youtube",
            "formats": [
                {"format_id": "sb0", "ext": "mhtml", "protocol": "mhtml", "url": "https://x/sb", "vcodec": "none", "acodec": "none", "format_note": "storyboard"},
                {"format_id": "137", "ext": "mp4", "protocol": "https", "url": "https://x/v", "vcodec": "avc1", "acodec": "none", "height": 1080, "fps": 30, "filesize_approx": 1000.0},
                {"format_id": "140", "ext": "m4a", "protocol": "https", "url": "https://x/a", "vcodec": "none", "acodec": "mp4a", "abr": 128.0, "filesize": 50},
                {"format_id": "18", "ext": "mp4", "protocol": "https", "url": "https://x/m", "vcodec": "avc1", "acodec": "mp4a", "height": 360},
                {"format_id": "web", "ext": "mp4", "protocol": "https", "url": "https://x/w", "height": 480}
            ]
        });
        let result = normalize("https://p", &json).unwrap();
        let kinds: Vec<_> = result.formats.iter().map(|f| (f.id.as_str(), f.kind.as_str())).collect();
        assert_eq!(kinds, [("137", "video"), ("140", "audio"), ("18", "muxed"), ("web", "muxed")]);
        assert!(result.formats[0].filesize_approx);
        assert!(!result.formats[1].filesize_approx);
    }

    #[test]
    fn youtube_urls_share_a_cache_key() {
        let key = cache_key("https://www.youtube.com/watch?v=TTz9NtVHJM4");
        assert_eq!(key, "youtube:TTz9NtVHJM4");
        for url in [
            "https://youtu.be/TTz9NtVHJM4?t=5",
            "https://www.youtube.com/shorts/TTz9NtVHJM4",
            "https://m.youtube.com/watch?list=x&v=TTz9NtVHJM4&t=9s",
            "https://youtube.com/embed/TTz9NtVHJM4",
        ] {
            assert_eq!(cache_key(url), key, "{url}");
        }
        assert_eq!(cache_key("https://example.com/v?v=1"), "https://example.com/v?v=1");
    }

    #[test]
    fn login_errors_are_classified() {
        for yes in [
            "yt-dlp failed: [youtube] x: Sign in to confirm you're not a bot",
            "yt-dlp failed: This video may be inappropriate for some users. Sign in to confirm your age",
            "yt-dlp failed: Join this channel to get access to members-only content",
            "yt-dlp failed: Private video. Sign in if you've been granted access",
            "yt-dlp failed: login required",
        ] {
            assert!(needs_login(yes), "{yes}");
        }
        for no in ["yt-dlp failed: [youtube] x: The page needs to be reloaded.", "yt-dlp failed: HTTP Error 404", "yt-dlp timed out while reading the page"] {
            assert!(!needs_login(no), "{no}");
        }
    }

    fn fake_result() -> ProbeResult {
        ProbeResult { page_url: "p".into(), title: "T".into(), duration: None, thumbnail: None, extractor: "x".into(), live: false, formats: Vec::new() }
    }

    #[test]
    fn cookies_are_a_retry_only_for_login_errors() {
        let mut calls = Vec::new();
        let ok = probe_ordered(true, |c| {
            calls.push(c);
            Ok(fake_result())
        });
        assert!(ok.is_ok());
        assert_eq!(calls, [false]);

        calls.clear();
        let ok = probe_ordered(true, |c| {
            calls.push(c);
            if c { Ok(fake_result()) } else { Err("yt-dlp failed: Sign in to confirm your age".into()) }
        });
        assert!(ok.is_ok());
        assert_eq!(calls, [false, true]);

        calls.clear();
        let err = probe_ordered(true, |c| {
            calls.push(c);
            Err(if c { "yt-dlp failed: The page needs to be reloaded." } else { "yt-dlp failed: Sign in to confirm you're not a bot" }.into())
        })
        .unwrap_err();
        assert_eq!(calls, [false, true]);
        assert!(err.to_string().contains("not a bot"), "{err}");

        calls.clear();
        assert!(probe_ordered(true, |c| {
            calls.push(c);
            Err("yt-dlp failed: HTTP Error 404".into())
        })
        .is_err());
        assert_eq!(calls, [false]);
    }

    /// A stub yt-dlp that fails with the reload message whenever --cookies is passed.
    #[cfg(unix)]
    #[test]
    fn stub_ytdlp_never_sees_cookies_when_cookieless_works() {
        use std::os::unix::fs::PermissionsExt;
        let dir = PrivateDir::create().unwrap();
        let stub = dir.0.join("yt-dlp");
        std::fs::write(
            &stub,
            "#!/bin/sh\nfor a in \"$@\"; do if [ \"$a\" = --cookies ]; then echo 'ERROR: [youtube] abc: The page needs to be reloaded.' >&2; exit 1; fi; done\necho '{\"title\":\"T\",\"extractor_key\":\"Youtube\",\"formats\":[{\"format_id\":\"18\",\"ext\":\"mp4\",\"protocol\":\"https\",\"url\":\"https://x/m\",\"vcodec\":\"avc1\",\"acodec\":\"mp4a\",\"height\":360}]}'\n",
        )
        .unwrap();
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        set_configured(Some(stub.to_string_lossy().into_owned()));
        let cookie = ProbeCookie { name: "a".into(), value: "b".into(), domain: ".youtube.com".into(), path: "/".into(), secure: true, expires: None };
        let proxy = ProxyArgs { args: Vec::new(), env: Vec::new() };
        let stop = AtomicBool::new(false);
        let result = probe("https://www.youtube.com/watch?v=stubstub001", &[cookie], true, &proxy, &stop);
        set_configured(None);
        assert_eq!(result.unwrap().formats.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn cookie_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = PrivateDir::create().unwrap();
        let cookie = ProbeCookie { name: "a".into(), value: "b".into(), domain: ".example.com".into(), path: "/".into(), secure: true, expires: Some(5) };
        let file = write_cookie_file(&dir.0, &[cookie]).unwrap();
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(&dir.0).unwrap().permissions().mode() & 0o777, 0o700);
        let kept = dir.0.clone();
        drop(dir);
        assert!(!kept.exists());
    }
}
