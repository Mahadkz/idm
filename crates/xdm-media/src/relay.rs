//! Private loopback relay between ffmpeg/ffprobe and the real origin.
//!
//! ffmpeg only ever sees `http://127.0.0.1:<port>/<token>/<id>/<name>`: no cookie,
//! password or header travels on its command line. The token is NOT a secret
//! against local users (argv is readable through /proc), so what it grants is
//! kept small: the relay serves only URLs it registered itself while rewriting the
//! job's manifests (opaque ids it issued), plus safe relative paths below the
//! directories it registered for DASH BaseURLs. Credentials are attached only to
//! requests whose origin equals the manifest origin, redirects are checked hop by
//! hop, and private/loopback/link-local targets are refused (see [`Upstream`]).
//! Basic and Digest come from `xdm_http::AuthState`. The listener lives exactly as
//! long as the [`Relay`] value.

use crate::{err, Result};
use reqwest::blocking::{Client, Response};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::Url;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Largest manifest the relay or the walk will buffer.
pub(crate) const MANIFEST_LIMIT: u64 = 8 * 1024 * 1024;
const MAX_REDIRECTS: usize = 8;
const MAX_CONNECTIONS: usize = 32;
const MAX_MANIFESTS_BUFFERED: usize = 4;
const HEADER_DEADLINE: Duration = Duration::from_secs(10);
const IO_TIMEOUT: Duration = Duration::from_secs(30);

/// Addresses an upstream may never be (SSRF guard): loopback, private, link-local,
/// unspecified, multicast, CGNAT, `0.0.0.0/8`, and their IPv6 / v4-mapped forms.
pub(crate) fn blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_multicast()
                || v4.is_broadcast()
                || o[0] == 0
                || (o[0] == 100 && (64..128).contains(&o[1]))
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return blocked_ip(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
        }
    }
}

fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Debug builds only: `XDM_MEDIA_TEST_HOSTS=name=ip,...` pins fake public names to
/// local fixtures; such names are treated as public. Release builds ignore it.
fn test_hosts() -> HashMap<String, IpAddr> {
    let mut map = HashMap::new();
    if cfg!(debug_assertions) {
        if let Ok(value) = std::env::var("XDM_MEDIA_TEST_HOSTS") {
            for pair in value.split(',') {
                if let Some((name, ip)) = pair.split_once('=') {
                    if let Ok(ip) = ip.trim().parse() {
                        map.insert(name.trim().to_ascii_lowercase(), ip);
                    }
                }
            }
        }
    }
    map
}

/// Resolves a name once and hands the checked addresses to the connector, so a
/// rebinding DNS answer cannot swap in a private address after the check.
struct GuardedResolver {
    home_host: String,
    home_local: bool,
    overrides: HashMap<String, IpAddr>,
}

impl reqwest::dns::Resolve for GuardedResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_ascii_lowercase();
        let exempt = self.home_local && host == self.home_host;
        let pinned = self.overrides.get(&host).copied();
        Box::pin(async move {
            if let Some(ip) = pinned {
                let one: Vec<SocketAddr> = vec![SocketAddr::new(ip, 0)];
                return Ok(Box::new(one.into_iter()) as reqwest::dns::Addrs);
            }
            let all: Vec<SocketAddr> = (host.as_str(), 0).to_socket_addrs()?.collect();
            let allowed: Vec<SocketAddr> =
                all.into_iter().filter(|a| exempt || !blocked_ip(a.ip())).collect();
            if allowed.is_empty() {
                return Err("refusing to connect to a private or loopback address".into());
            }
            Ok(Box::new(allowed.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// Origin-scoped upstream access shared by the relay and the manifest walk.
pub(crate) struct Upstream {
    client: Client,
    home: reqwest::Url,
    /// The manifest origin itself is a local address (a fixture on 127.0.0.1): only
    /// that exact origin may be local.
    home_local: bool,
    overrides: HashMap<String, IpAddr>,
    headers: Vec<(String, String)>,
    auth: Option<xdm_http::AuthState>,
}

impl Upstream {
    fn new(home: &Url, headers: &[(String, String)], login: Option<crate::OriginLogin>) -> Result<Self> {
        let (proxy, proxy_login) = crate::current_proxy();
        let overrides = test_hosts();
        let host = home.host_str().unwrap_or("").trim_matches(['[', ']']).to_ascii_lowercase();
        let home_local = match host.parse::<IpAddr>() {
            Ok(ip) => blocked_ip(ip),
            Err(_) if overrides.contains_key(&host) => false,
            Err(_) => (host.as_str(), 0)
                .to_socket_addrs()
                .is_ok_and(|mut addrs| addrs.any(|a| blocked_ip(a.ip()))),
        };
        // `timeout` re-arms per body read, so it is the upstream idle timeout.
        let builder = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(20))
            .timeout(IO_TIMEOUT)
            .dns_resolver(Arc::new(GuardedResolver {
                home_host: host,
                home_local,
                overrides: overrides.clone(),
            }));
        let client = xdm_http::apply_proxy(builder, &proxy, proxy_login.as_ref())?.build()?;
        let auth = login
            .filter(|login| Url::parse(&login.url).is_ok_and(|url| url.origin() == home.origin()))
            .map(|login| {
                xdm_http::AuthState::new(xdm_http::Login {
                    username: login.username,
                    password: login.password,
                    allow_insecure: login.allow_insecure,
                })
            });
        Ok(Self { client, home: home.clone(), home_local, overrides, headers: headers.to_vec(), auth })
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

    /// Refuses private targets before any bytes leave (every hop, redirects included).
    fn check_target(&self, url: &Url) -> Result<()> {
        let host = url.host_str().unwrap_or("").trim_matches(['[', ']']).to_ascii_lowercase();
        if host.is_empty() {
            return Err(err("media URL has no host"));
        }
        let exact_home = self.home_local && url.origin() == self.home.origin();
        match host.parse::<IpAddr>() {
            Ok(ip) if blocked_ip(ip) && !exact_home => {
                Err(err("refusing a private, loopback or link-local media address"))
            }
            Err(_) if !self.overrides.contains_key(&host) && self.home_local
                && Some(host.as_str()) == self.home.host_str().map(str::to_ascii_lowercase).as_deref()
                && !exact_home =>
            {
                Err(err("refusing a local media address on another origin"))
            }
            _ => Ok(()),
        }
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
            self.check_target(&current)?;
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

struct Entry {
    url: Url,
    /// A registered directory (DASH BaseURL): safe relative paths below it are served.
    dir: bool,
}

#[derive(Default)]
struct Entries {
    items: Vec<Entry>,
    index: HashMap<String, usize>,
}

pub(crate) struct Shared {
    token: String,
    port: u16,
    upstream: Upstream,
    entries: Mutex<Entries>,
    shutdown: AtomicBool,
    active: AtomicUsize,
    manifests: AtomicUsize,
    conns: Mutex<Vec<TcpStream>>,
}

/// A running relay for one job. Dropping it closes the listener and its connections.
pub(crate) struct Relay {
    shared: Arc<Shared>,
    accept: Option<JoinHandle<()>>,
}

fn safe_name(url: &Url) -> String {
    let last = url.path().rsplit('/').next().unwrap_or("");
    last.chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
        .collect()
}

/// A relative path that stays below its directory: no scheme, authority, root,
/// traversal, backslash, control characters or encoded dots/slashes.
fn safe_tail(tail: &str) -> bool {
    let path = tail.split('?').next().unwrap_or("");
    let lower = tail.to_ascii_lowercase();
    !path.is_empty()
        && !tail.starts_with('/')
        && !tail.contains(['\\', ':', '\0', '\r', '\n', '\t', '#'])
        && !path.contains("//")
        && !path.split('/').any(|part| part == ".." || part == ".")
        && !lower.contains("%2e")
        && !lower.contains("%2f")
        && !lower.contains("%5c")
        && !tail.chars().any(char::is_control)
}

impl Shared {
    fn register_as(&self, url: &Url, dir: bool) -> usize {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let key = format!("{}{}", if dir { "d:" } else { "f:" }, url);
        if let Some(&id) = entries.index.get(&key) {
            return id;
        }
        let id = entries.items.len();
        entries.items.push(Entry { url: url.clone(), dir });
        entries.index.insert(key, id);
        id
    }

    fn entry(&self, id: usize) -> Option<(Url, bool)> {
        self.entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .items
            .get(id)
            .map(|entry| (entry.url.clone(), entry.dir))
    }

    fn file_url(&self, url: &Url) -> String {
        let id = self.register_as(url, false);
        format!("http://127.0.0.1:{}/{}/{id}/{}", self.port, self.token, safe_name(url))
    }

    /// Relay directory standing in for an upstream directory URL (query dropped).
    fn dir_url(&self, dir: &Url) -> String {
        let mut dir = dir.clone();
        dir.set_query(None);
        dir.set_fragment(None);
        let id = self.register_as(&dir, true);
        format!("http://127.0.0.1:{}/{}/{id}/", self.port, self.token)
    }

    fn prefix(&self) -> String {
        format!("http://127.0.0.1:{}/{}/", self.port, self.token)
    }
}

struct ActiveGuard<'a>(&'a AtomicUsize);
impl Drop for ActiveGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
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
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let shared = Arc::new(Shared {
            token,
            port,
            upstream,
            entries: Mutex::new(Entries::default()),
            shutdown: AtomicBool::new(false),
            active: AtomicUsize::new(0),
            manifests: AtomicUsize::new(0),
            conns: Mutex::new(Vec::new()),
        });
        shared.register_as(manifest, false);
        let serving = shared.clone();
        let accept = std::thread::spawn(move || {
            while !serving.shutdown.load(Ordering::SeqCst) {
                let mut stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(_) => {
                        std::thread::sleep(Duration::from_millis(15));
                        continue;
                    }
                };
                let _ = stream.set_nonblocking(false);
                if serving.active.load(Ordering::SeqCst) >= MAX_CONNECTIONS {
                    let _ = respond(&mut stream, 503, "Service Unavailable", &[], b"", false);
                    continue;
                }
                serving.active.fetch_add(1, Ordering::SeqCst);
                if let Ok(clone) = stream.try_clone() {
                    let mut conns = serving.conns.lock().unwrap_or_else(|e| e.into_inner());
                    conns.retain(|s| s.peer_addr().is_ok());
                    conns.push(clone);
                }
                let shared = serving.clone();
                std::thread::spawn(move || {
                    let _guard = ActiveGuard(&shared.active);
                    let _ = handle(stream, &shared);
                });
            }
        });
        Ok(Self { shared, accept: Some(accept) })
    }

    /// The relay URL ffmpeg opens for the job's manifest.
    pub(crate) fn manifest_url(&self) -> String {
        let (url, _) = self.shared.entry(0).expect("manifest registered");
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
        if let Some(handle) = self.accept.take() {
            let _ = handle.join(); // polling accept loop: returns within ~15 ms
        }
        for stream in self.shared.conns.lock().unwrap_or_else(|e| e.into_inner()).drain(..) {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.shared.active.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
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
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    stream.set_write_timeout(Some(IO_TIMEOUT)).ok()?;
    let deadline = Instant::now() + HEADER_DEADLINE;
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 2048];
    while !buffer.windows(4).any(|w| w == b"\r\n\r\n") {
        if buffer.len() > 16 * 1024 || Instant::now() > deadline {
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

struct ManifestSlot<'a>(&'a AtomicUsize);
impl Drop for ManifestSlot<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn handle(mut stream: TcpStream, shared: &Shared) -> std::io::Result<()> {
    let Some((method, target, range)) = read_request(&mut stream) else {
        return Ok(());
    };
    let not_found = |stream: &mut TcpStream| respond(stream, 404, "Not Found", &[], b"", false);
    // Every request needs the token prefix; anything else is a plain 404.
    let path = target.strip_prefix('/').unwrap_or("");
    let Some((token, rest)) = path.split_once('/') else {
        return not_found(&mut stream);
    };
    if !ct_eq(token, &shared.token) {
        return not_found(&mut stream);
    }
    let head = method == "HEAD";
    if method != "GET" && !head {
        return respond(&mut stream, 405, "Method Not Allowed", &[], b"", false);
    }
    let Some((id, tail)) = rest.split_once('/') else {
        return not_found(&mut stream);
    };
    let Some((entry, is_dir)) = id.parse::<usize>().ok().and_then(|id| shared.entry(id)) else {
        return not_found(&mut stream);
    };
    // Only URLs the relay issued are served: a file entry by its exact name, a
    // registered directory by safe relative paths that stay on its origin and below it.
    let upstream_url = if is_dir {
        if !safe_tail(tail) {
            return not_found(&mut stream);
        }
        match entry.join(tail) {
            Ok(url) if url.origin() == entry.origin() && url.path().starts_with(entry.path()) => url,
            _ => return not_found(&mut stream),
        }
    } else if tail == safe_name(&entry) {
        entry
    } else {
        return not_found(&mut stream);
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
        if shared.manifests.fetch_add(1, Ordering::SeqCst) >= MAX_MANIFESTS_BUFFERED {
            shared.manifests.fetch_sub(1, Ordering::SeqCst);
            return respond(&mut stream, 503, "Service Unavailable", &[], b"", false);
        }
        let _slot = ManifestSlot(&shared.manifests);
        let mut body = Vec::new();
        let _ = response.take(MANIFEST_LIMIT + 1).read_to_end(&mut body);
        if body.len() as u64 > MANIFEST_LIMIT {
            return respond(&mut stream, 502, "Bad Gateway", &[], b"", false);
        }
        let Ok(text) = String::from_utf8(body) else {
            return respond(&mut stream, 502, "Bad Gateway", &[], b"", false);
        };
        let rewritten = if hls { rewrite_hls(&text, &final_url, shared) } else { rewrite_dash(&text, &final_url, shared) };
        return match rewritten {
            Ok(text) => respond(&mut stream, 200, "OK", &[("Content-Type".into(), content_type)], text.as_bytes(), false),
            Err(_) => respond(&mut stream, 502, "Bad Gateway", &[], b"", false),
        };
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

/// A reference must be a single clean http(s) URL: no control characters (which
/// `Url::parse` would silently strip) and no other scheme.
fn resolve_ref(reference: &str, base: &Url) -> Result<Url> {
    if reference.contains(['\r', '\n', '\t', '\0']) {
        return Err(err("control character inside a media reference"));
    }
    let url = base.join(reference.trim())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(err("manifest references a non-http(s) URL"));
    }
    Ok(url)
}

/// Every `URI="..."`/`URI='...'` attribute and bare URI line points back through the
/// relay; anything that cannot be rewritten rejects the manifest, and the output is
/// verified to contain relay URLs only.
pub(crate) fn rewrite_hls(text: &str, base: &Url, shared: &Shared) -> Result<String> {
    let prefix = shared.prefix();
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(tag) = line.strip_prefix('#') {
            out.push('#');
            let mut rest = tag;
            loop {
                let found = ["URI=\"", "URI='"]
                    .iter()
                    .filter_map(|n| rest.find(n).map(|i| (i, n.as_bytes()[4] as char)))
                    .min_by_key(|(i, _)| *i);
                let Some((start, quote)) = found else { break };
                let (before, after) = rest.split_at(start + 5);
                out.push_str(before);
                let end = after.find(quote).ok_or_else(|| err("unterminated URI attribute"))?;
                let value = &after[..end];
                out.push_str(&shared.file_url(&resolve_ref(value, base)?));
                rest = &after[end..];
            }
            if rest.to_ascii_lowercase().contains("uri=") && !rest.contains(&prefix) {
                // An unquoted URI= would reach ffmpeg unrewritten.
                if rest.to_ascii_lowercase().split("uri=").skip(1).any(|r| !r.starts_with(['"', '\''])) {
                    return Err(err("unquoted URI attribute"));
                }
            }
            out.push_str(rest);
        } else if line.trim().is_empty() {
            out.push_str(line);
        } else {
            out.push_str(&shared.file_url(&resolve_ref(line, base)?));
        }
        out.push('\n');
    }
    for line in out.lines() {
        if !line.starts_with('#') && !line.trim().is_empty() && !line.starts_with(&prefix) {
            return Err(err("unrewritten manifest line"));
        }
    }
    Ok(out)
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
/// directories; MPD-level BaseURLs resolve against the manifest's final (post-redirect)
/// URL, and one is inserted when absent, so relative templates always land on a
/// registered directory.
pub(crate) fn rewrite_dash(text: &str, base: &Url, shared: &Shared) -> Result<String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let period = text.find("<Period").unwrap_or(text.len());
    let mut has_top_base = false;
    while let Some(start) = rest.find("<BaseURL") {
        let top = text.len() - rest.len() + start < period;
        has_top_base |= top;
        let Some(open_end) = rest[start..].find('>') else { break };
        let content_start = start + open_end + 1;
        let Some(close) = rest[content_start..].find("</BaseURL>") else { break };
        let content = rest[content_start..content_start + close].trim();
        out.push_str(&rest[..content_start]);
        if content.contains(['\r', '\n', '\t', '\0']) {
            return Err(err("control character inside a media reference"));
        }
        let scheme_relative = content.starts_with("//");
        match Url::parse(content) {
            Ok(url) if !matches!(url.scheme(), "http" | "https") => {
                return Err(err("manifest references a non-http(s) URL"));
            }
            Ok(url) => out.push_str(&shared.dir_url(&parent_dir(&url))),
            Err(_) if top || scheme_relative => {
                out.push_str(&shared.dir_url(&parent_dir(&resolve_ref(content, base)?)))
            }
            Err(_) => out.push_str(content),
        }
        rest = &rest[content_start + close..];
    }
    out.push_str(rest);
    let mut text = out;
    if !has_top_base {
        if let Some(at) = text.find("<MPD").and_then(|i| text[i..].find('>').map(|j| i + j + 1)) {
            if !text[..at].ends_with("/>") {
                let tag = format!("<BaseURL>{}</BaseURL>", shared.dir_url(&parent_dir(base)));
                text.insert_str(at, &tag);
            }
        }
    }
    for attribute in ["media", "initialization", "sourceURL", "index"] {
        for quote in ['"', '\''] {
            let needle = format!("{attribute}={quote}");
            let mut out = String::with_capacity(text.len());
            let mut rest = text.as_str();
            while let Some(start) = rest.find(&needle) {
                let value_start = start + needle.len();
                let boundary = start == 0 || rest.as_bytes()[start - 1].is_ascii_whitespace();
                let Some(end) = rest[value_start..].find(quote) else { break };
                let value = &rest[value_start..value_start + end];
                out.push_str(&rest[..value_start]);
                if boundary {
                    if value.contains(['\r', '\n', '\t', '\0']) {
                        return Err(err("control character inside a media reference"));
                    }
                    let value = if value.starts_with("//") {
                        std::borrow::Cow::Owned(format!("{}:{value}", base.scheme()))
                    } else {
                        std::borrow::Cow::Borrowed(value)
                    };
                    match Url::parse(&value) {
                        Ok(url) if !matches!(url.scheme(), "http" | "https") => {
                            return Err(err("manifest references a non-http(s) URL"));
                        }
                        Ok(_) => {
                            // Split at the last '/' before the first template variable.
                            let head_end = value.find(['$', '?']).unwrap_or(value.len());
                            let cut = value[..head_end].rfind('/').map_or(0, |i| i + 1);
                            let dir = Url::parse(&value[..cut])?;
                            out.push_str(&shared.dir_url(&dir));
                            out.push_str(&value[cut..]);
                        }
                        Err(_) => out.push_str(&value),
                    }
                } else {
                    out.push_str(value);
                }
                rest = &rest[value_start + end..];
            }
            out.push_str(rest);
            text = out;
        }
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relay() -> Relay {
        Relay::start(&Url::parse("https://pub.example/v/index.m3u8").unwrap(), &[]).unwrap()
    }

    #[test]
    fn private_addresses_are_blocked() {
        for blocked in ["127.0.0.1", "10.1.2.3", "172.16.0.1", "192.168.1.1", "169.254.169.254", "0.0.0.0",
            "100.64.0.1", "224.0.0.1", "::1", "fe80::1", "fc00::1", "fd12::1", "::ffff:127.0.0.1", "::"] {
            assert!(blocked_ip(blocked.parse().unwrap()), "{blocked}");
        }
        for open in ["8.8.8.8", "93.184.216.34", "2606:4700::1111"] {
            assert!(!blocked_ip(open.parse().unwrap()), "{open}");
        }
    }

    #[test]
    fn token_compare_and_tails() {
        assert!(ct_eq("abc", "abc") && !ct_eq("abc", "abd") && !ct_eq("abc", "ab"));
        for bad in ["http://169.254.169.254/x", "//127.0.0.1:6379/", "/etc", "../x", "a/../../x", "a%2e%2e/x", "a:b", "a\\b", ""] {
            assert!(!safe_tail(bad), "{bad}");
        }
        assert!(safe_tail("seg-1.m4s") && safe_tail("sub/seg_2.m4s?x=1"));
    }

    #[test]
    fn hls_references_all_become_relay_urls() {
        let relay = relay();
        let base = Url::parse("https://pub.example/v/index.m3u8").unwrap();
        let text = "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI='key.bin'\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:2,\nhttps://cdn.example/s1.ts\n";
        let out = rewrite_hls(text, &base, &relay.shared).unwrap();
        let prefix = relay.shared.prefix();
        assert!(!out.contains("pub.example") && !out.contains("cdn.example"), "{out}");
        assert_eq!(out.matches(&prefix).count(), 3, "{out}");
    }

    #[test]
    fn hls_rejects_foreign_schemes_and_control_characters() {
        let relay = relay();
        let base = Url::parse("https://pub.example/v/index.m3u8").unwrap();
        for bad in ["#EXTM3U\n#EXTINF:2,\ntcp://10.0.0.1:6379/x\n", "#EXTM3U\n#EXT-X-KEY:URI='tcp://10.0.0.1/k'\n",
            "#EXTM3U\n#EXTINF:2,\nhttp://a\t.example/x\n", "#EXTM3U\n#EXT-X-MAP:URI=init\n", "#EXTM3U\n#EXTINF:2,\nfile:///etc/passwd\n"] {
            assert!(rewrite_hls(bad, &base, &relay.shared).is_err(), "{bad}");
        }
    }

    #[test]
    fn dash_directories_drop_queries_and_foreign_schemes_fail() {
        let relay = relay();
        let base = Url::parse("https://pub.example/v/m.mpd").unwrap();
        let ok = "<MPD><BaseURL>https://cdn.example/a/?sig=SECRET</BaseURL><Period><SegmentTemplate media='https://o.example/x/s-$Number$.m4s'/></Period></MPD>";
        let out = rewrite_dash(ok, &base, &relay.shared).unwrap();
        assert!(!out.contains("SECRET") && !out.contains("cdn.example") && !out.contains("o.example"), "{out}");
        assert!(rewrite_dash("<MPD><BaseURL>tcp://10.0.0.1/</BaseURL></MPD>", &base, &relay.shared).is_err());
        assert!(rewrite_dash("<MPD><Period><SegmentTemplate media=\"tcp://10.0.0.1/s\"/></Period></MPD>", &base, &relay.shared).is_err());
        let inserted = rewrite_dash("<MPD><Period/></MPD>", &base, &relay.shared).unwrap();
        assert!(inserted.contains(&relay.shared.prefix()), "{inserted}");
    }

    #[test]
    fn drop_closes_listener_promptly() {
        let relay = relay();
        let port = relay.shared.port;
        let started = Instant::now();
        drop(relay);
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
    }
}
