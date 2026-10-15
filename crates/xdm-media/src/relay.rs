//! Private loopback relay between ffmpeg/ffprobe and the real origin.
//!
//! ffmpeg only ever sees `http://127.0.0.1:<port>/<token>/<id>/<name>`: no cookie,
//! password or header travels on its command line (visible in /proc to other
//! local users). The relay forwards GET/HEAD (with Range) to the origin, rewrites
//! every HLS/DASH reference to a relay URL so segments and keys flow through it
//! too, and attaches captured headers and the saved login only to requests whose
//! origin equals the origin of the job's manifest. Redirects are followed with
//! the same per-hop rule. Basic and Digest come from `xdm_http::AuthState`.
//! The listener lives exactly as long as the [`Relay`] value.

use crate::{err, Result};
use reqwest::blocking::{Client, Response};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::Url;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// Largest manifest the relay or the walk will buffer.
pub(crate) const MANIFEST_LIMIT: u64 = 8 * 1024 * 1024;
const MAX_REDIRECTS: usize = 8;

/// Origin-scoped upstream access shared by the relay and the manifest walk.
pub(crate) struct Upstream {
    client: Client,
    home: reqwest::Url,
    headers: Vec<(String, String)>,
    auth: Option<xdm_http::AuthState>,
}

impl Upstream {
    fn new(home: &Url, headers: &[(String, String)], login: Option<crate::OriginLogin>) -> Result<Self> {
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(20))
            .build()?;
        let auth = login
            .filter(|login| Url::parse(&login.url).is_ok_and(|url| url.origin() == home.origin()))
            .map(|login| {
                xdm_http::AuthState::new(xdm_http::Login {
                    username: login.username,
                    password: login.password,
                    allow_insecure: login.allow_insecure,
                })
            });
        Ok(Self { client, home: home.clone(), headers: headers.to_vec(), auth })
    }

    /// Headers for one hop. User-Agent and Referer ride everywhere (the browser would
    /// send them too); Cookie, Authorization and every other captured header only
    /// go to the manifest's own origin.
    fn hop_headers(&self, target: &Url) -> Result<HeaderMap> {
        let same = target.origin() == self.home.origin();
        let mut map = HeaderMap::new();
        for (name, value) in &self.headers {
            let lower = name.to_ascii_lowercase();
            if matches!(
                lower.as_str(),
                "host" | "content-length" | "range" | "connection" | "transfer-encoding" | "te" | "upgrade"
            ) {
                continue;
            }
            if !same && !matches!(lower.as_str(), "user-agent" | "referer") {
                continue;
            }
            map.insert(HeaderName::from_bytes(name.as_bytes())?, HeaderValue::from_str(value)?);
        }
        Ok(map)
    }

    /// Sends one request, following redirects hop by hop. Returns the response and
    /// the URL that produced it (the base for relative references).
    pub(crate) fn send(
        &self,
        method: &str,
        url: &Url,
        range: Option<&str>,
        timeout: Option<Duration>,
    ) -> Result<(Response, Url)> {
        let mut current = url.clone();
        for _ in 0..=MAX_REDIRECTS {
            if !matches!(current.scheme(), "http" | "https") {
                return Err(err("unsupported media URL scheme"));
            }
            let response = self.send_once(method, &current, range, timeout)?;
            if response.status().is_redirection() {
                if let Some(location) = response.headers().get(reqwest::header::LOCATION) {
                    current = current.join(location.to_str()?)?;
                    continue;
                }
            }
            return Ok((response, current));
        }
        Err(err("too many redirects"))
    }

    fn send_once(&self, method: &str, url: &Url, range: Option<&str>, timeout: Option<Duration>) -> Result<Response> {
        let same = url.origin() == self.home.origin();
        let auth = self.auth.as_ref().filter(|_| same);
        for _ in 0..3 {
            let mut headers = self.hop_headers(url)?;
            let mut sent = false;
            if let Some(value) = auth.and_then(|auth| auth.header(method)) {
                headers.insert(reqwest::header::AUTHORIZATION, HeaderValue::from_str(&value)?);
                sent = true;
            }
            if let Some(range) = range {
                headers.insert(reqwest::header::RANGE, HeaderValue::from_str(range)?);
            }
            let mut request = self.client.request(method.parse()?, url.clone()).headers(headers);
            if let Some(timeout) = timeout {
                request = request.timeout(timeout);
            }
            let response = request.send()?;
            if let Some(auth) = auth {
                if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                    let challenges: Vec<String> = response
                        .headers()
                        .get_all(reqwest::header::WWW_AUTHENTICATE)
                        .iter()
                        .filter_map(|value| value.to_str().ok().map(str::to_owned))
                        .collect();
                    if matches!(auth.answer(&challenges, url.as_str(), sent), xdm_http::AuthAnswer::Retry) {
                        continue;
                    }
                } else if sent && response.status().is_success() {
                    auth.confirm();
                }
            }
            return Ok(response);
        }
        Err(err("authentication was not accepted"))
    }
}

#[derive(Default)]
struct Entries {
    urls: Vec<Url>,
    index: HashMap<String, usize>,
}

pub(crate) struct Shared {
    token: String,
    port: u16,
    upstream: Upstream,
    entries: Mutex<Entries>,
    shutdown: AtomicBool,
}

/// A running relay for one job. Dropping it closes the listener.
pub(crate) struct Relay {
    shared: Arc<Shared>,
    accept: Option<JoinHandle<()>>,
}

fn safe_name(url: &Url) -> String {
    let last = url.path().rsplit('/').next().unwrap_or("");
    let cleaned: String = last
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
        .collect();
    if last.is_empty() { String::new() } else { cleaned }
}

impl Shared {
    fn register(&self, url: &Url) -> usize {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(&id) = entries.index.get(url.as_str()) {
            return id;
        }
        let id = entries.urls.len();
        entries.urls.push(url.clone());
        entries.index.insert(url.to_string(), id);
        id
    }

    fn url_of(&self, id: usize) -> Option<Url> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner()).urls.get(id).cloned()
    }

    fn file_url(&self, url: &Url) -> String {
        let id = self.register(url);
        format!("http://127.0.0.1:{}/{}/{id}/{}", self.port, self.token, safe_name(url))
    }

    /// Relay directory standing in for an upstream directory URL.
    fn dir_url(&self, dir: &Url) -> String {
        let id = self.register(dir);
        format!("http://127.0.0.1:{}/{}/{id}/", self.port, self.token)
    }
}

impl Relay {
    /// Binds 127.0.0.1 on an ephemeral port and serves until dropped. The saved login
    /// (see [`crate::with_origin_login`]) and the captured headers belong to `manifest`'s origin.
    pub(crate) fn start(manifest: &Url, headers: &[(String, String)]) -> Result<Self> {
        let login = crate::current_origin_login();
        let upstream = Upstream::new(manifest, headers, login)?;
        let mut raw = [0u8; 24];
        getrandom::fill(&mut raw).map_err(|e| err(format!("no secure randomness: {e}")))?;
        let token: String = raw.iter().map(|b| format!("{b:02x}")).collect();
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))?;
        let port = listener.local_addr()?.port();
        let shared = Arc::new(Shared {
            token,
            port,
            upstream,
            entries: Mutex::new(Entries::default()),
            shutdown: AtomicBool::new(false),
        });
        shared.register(manifest);
        let serving = shared.clone();
        let accept = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if serving.shutdown.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(stream) = stream {
                    let shared = serving.clone();
                    std::thread::spawn(move || {
                        let _ = handle(stream, &shared);
                    });
                }
            }
        });
        Ok(Self { shared, accept: Some(accept) })
    }

    /// The relay URL ffmpeg opens for the job's manifest.
    pub(crate) fn manifest_url(&self) -> String {
        let url = self.shared.url_of(0).expect("manifest registered");
        self.shared.file_url(&url)
    }

    /// Relay URL for another upstream URL (rendition playlists).
    pub(crate) fn url_for(&self, upstream: &str) -> Result<String> {
        Ok(self.shared.file_url(&Url::parse(upstream)?))
    }

    pub(crate) fn upstream(&self) -> &Upstream {
        &self.shared.upstream
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::SeqCst);
        // Wake the blocking accept so the listener is closed before returning.
        let _ = TcpStream::connect_timeout(
            &SocketAddr::from((Ipv4Addr::LOCALHOST, self.shared.port)),
            Duration::from_secs(2),
        );
        if let Some(handle) = self.accept.take() {
            let _ = handle.join();
        }
    }
}

fn respond(stream: &mut TcpStream, code: u16, reason: &str, extra: &[(String, String)], body: &[u8], head: bool) -> std::io::Result<()> {
    let mut out = format!("HTTP/1.1 {code} {reason}\r\nConnection: close\r\n");
    for (name, value) in extra {
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    if !extra.iter().any(|(n, _)| n.eq_ignore_ascii_case("content-length")) {
        out.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    out.push_str("\r\n");
    stream.write_all(out.as_bytes())?;
    if !head {
        stream.write_all(body)?;
    }
    stream.flush()
}

fn read_request(stream: &mut TcpStream) -> Option<(String, String, Option<String>)> {
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok()?;
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 2048];
    while !buffer.windows(4).any(|w| w == b"\r\n\r\n") {
        if buffer.len() > 16 * 1024 {
            return None;
        }
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..n]);
    }
    let text = String::from_utf8_lossy(&buffer).into_owned();
    let mut lines = text.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_owned();
    let target = first.next()?.to_owned();
    let range = lines
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("range"))
        .map(|(_, value)| value.trim().to_owned());
    Some((method, target, range))
}

fn handle(mut stream: TcpStream, shared: &Shared) -> std::io::Result<()> {
    let Some((method, target, range)) = read_request(&mut stream) else {
        return Ok(());
    };
    // Every request needs the unguessable prefix; anything else is a plain 404.
    let path = target.strip_prefix('/').unwrap_or("");
    let Some((token, rest)) = path.split_once('/') else {
        return respond(&mut stream, 404, "Not Found", &[], b"", false);
    };
    if token != shared.token {
        return respond(&mut stream, 404, "Not Found", &[], b"", false);
    }
    let head = method == "HEAD";
    if method != "GET" && !head {
        return respond(&mut stream, 405, "Method Not Allowed", &[], b"", false);
    }
    let Some((id, tail)) = rest.split_once('/') else {
        return respond(&mut stream, 404, "Not Found", &[], b"", false);
    };
    let Some(entry) = id.parse::<usize>().ok().and_then(|id| shared.url_of(id)) else {
        return respond(&mut stream, 404, "Not Found", &[], b"", false);
    };
    let tail_path = tail.split('?').next().unwrap_or("");
    let upstream_url = if tail_path == safe_name(&entry) {
        entry
    } else {
        match entry.join(tail) {
            Ok(url) => url,
            Err(_) => return respond(&mut stream, 404, "Not Found", &[], b"", false),
        }
    };
    let (response, final_url) = match shared.upstream.send(&method, &upstream_url, range.as_deref(), None) {
        Ok(pair) => pair,
        Err(_) => return respond(&mut stream, 502, "Bad Gateway", &[], b"", head),
    };
    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let lower_path = final_url.path().to_ascii_lowercase();
    let lower_type = content_type.to_ascii_lowercase();
    let hls = lower_path.ends_with(".m3u8") || lower_type.contains("mpegurl");
    let dash = lower_path.ends_with(".mpd") || lower_type.contains("dash+xml");
    let reason = status.canonical_reason().unwrap_or("OK");
    if status.is_success() && (hls || dash) {
        if head {
            return respond(&mut stream, 200, "OK", &[("Content-Type".into(), content_type)], b"", true);
        }
        let mut body = Vec::new();
        let _ = response.take(MANIFEST_LIMIT + 1).read_to_end(&mut body);
        if body.len() as u64 > MANIFEST_LIMIT {
            return respond(&mut stream, 502, "Bad Gateway", &[], b"", false);
        }
        let Ok(text) = String::from_utf8(body) else {
            return respond(&mut stream, 502, "Bad Gateway", &[], b"", false);
        };
        let rewritten = if hls { rewrite_hls(&text, &final_url, shared) } else { rewrite_dash(&text, &final_url, shared) };
        return respond(&mut stream, 200, "OK", &[("Content-Type".into(), content_type)], rewritten.as_bytes(), false);
    }
    if !status.is_success() {
        return respond(&mut stream, status.as_u16(), reason, &[], b"", head);
    }
    let mut extra = Vec::new();
    for name in ["content-type", "content-length", "content-range", "accept-ranges"] {
        if let Some(value) = response.headers().get(name).and_then(|v| v.to_str().ok()) {
            extra.push((name.to_owned(), value.to_owned()));
        }
    }
    let mut out = format!("HTTP/1.1 {} {reason}\r\nConnection: close\r\n", status.as_u16());
    for (name, value) in &extra {
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    out.push_str("\r\n");
    stream.write_all(out.as_bytes())?;
    if head {
        return Ok(());
    }
    // Stream the body in chunks; never buffer a whole segment.
    let mut response = response;
    let mut chunk = vec![0u8; 64 * 1024];
    while !shared.shutdown.load(Ordering::SeqCst) {
        let n = match response.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        stream.write_all(&chunk[..n])?;
    }
    Ok(())
}

fn rewrite_ref(reference: &str, base: &Url, shared: &Shared) -> Option<String> {
    let url = base.join(reference.trim()).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| shared.file_url(&url))
}

/// Every `URI="..."` attribute and bare URI line points back through the relay.
pub(crate) fn rewrite_hls(text: &str, base: &Url, shared: &Shared) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(tag) = line.strip_prefix('#') {
            let mut rest = tag;
            out.push('#');
            while let Some(start) = rest.find("URI=\"") {
                let (before, after) = rest.split_at(start + 5);
                out.push_str(before);
                let end = after.find('"').unwrap_or(after.len());
                let value = &after[..end];
                out.push_str(&rewrite_ref(value, base, shared).unwrap_or_else(|| value.to_owned()));
                rest = &after[end..];
            }
            out.push_str(rest);
        } else if line.trim().is_empty() {
            out.push_str(line);
        } else {
            out.push_str(&rewrite_ref(line, base, shared).unwrap_or_else(|| line.to_owned()));
        }
        out.push('\n');
    }
    out
}

fn parent_dir(url: &Url) -> Url {
    let mut dir = url.clone();
    dir.set_query(None);
    dir.set_fragment(None);
    let path = dir.path();
    let trimmed = &path[..path.rfind('/').map_or(0, |i| i + 1)];
    let trimmed = if trimmed.is_empty() { "/".to_owned() } else { trimmed.to_owned() };
    dir.set_path(&trimmed);
    dir
}

/// Absolute `<BaseURL>` values and absolute template/segment attributes become relay
/// directories; relative ones already resolve against the relay manifest URL.
pub(crate) fn rewrite_dash(text: &str, base: &Url, shared: &Shared) -> String {
    let _ = base;
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<BaseURL") {
        let Some(open_end) = rest[start..].find('>') else { break };
        let content_start = start + open_end + 1;
        let Some(close) = rest[content_start..].find("</BaseURL>") else { break };
        let content = &rest[content_start..content_start + close];
        out.push_str(&rest[..content_start]);
        match Url::parse(content.trim()) {
            Ok(url) if matches!(url.scheme(), "http" | "https") => out.push_str(&shared.dir_url(&parent_dir(&url))),
            _ => out.push_str(content),
        }
        rest = &rest[content_start + close..];
    }
    out.push_str(rest);
    let mut text = out;
    for attribute in ["media", "initialization", "sourceURL", "index"] {
        let needle = format!("{attribute}=\"");
        let mut out = String::with_capacity(text.len());
        let mut rest = text.as_str();
        while let Some(start) = rest.find(&needle) {
            let value_start = start + needle.len();
            let boundary = start == 0 || rest.as_bytes()[start - 1].is_ascii_whitespace();
            let Some(end) = rest[value_start..].find('"') else { break };
            let value = &rest[value_start..value_start + end];
            out.push_str(&rest[..value_start]);
            let absolute = value.starts_with("http://") || value.starts_with("https://");
            if boundary && absolute {
                // Split at the last '/' before the first template variable.
                let head_end = value.find('$').unwrap_or(value.len());
                let cut = value[..head_end].rfind('/').map_or(0, |i| i + 1);
                match Url::parse(&value[..cut]) {
                    Ok(dir) => {
                        out.push_str(&shared.dir_url(&dir));
                        out.push_str(&value[cut..]);
                    }
                    Err(_) => out.push_str(value),
                }
            } else {
                out.push_str(value);
            }
            rest = &rest[value_start + end..];
        }
        out.push_str(rest);
        text = out;
    }
    text
}
