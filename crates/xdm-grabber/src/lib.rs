//! Site grabber: a bounded, polite website crawler that discovers
//! downloadable files, in the spirit of IDM's Site Grabber but implemented
//! independently. HTTP pages are fetched and tokenized for links; candidate
//! files are classified by URL extension only and are never requested, and
//! links to known non-HTML resources are dropped without a request.

mod glob;
mod html;
mod offline;
mod pattern;
mod robots;

pub use offline::create_mirror_dirs;

use reqwest::blocking::{Client, Response};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, USER_AGENT};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::error::Error;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use url::Url;

pub type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

/// Largest HTML page body ever parsed. Anything longer becomes an error entry.
pub const MAX_PAGE_BYTES: usize = 4 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_REDIRECTS: usize = 10;
const DEFAULT_USER_AGENT: &str = "XDM-Grabber/0.1";

/// Path extensions that identify known non-HTML resources: images, audio,
/// video, archives, documents, executables, fonts, stylesheets and scripts.
/// Links to these are dropped instead of fetched when the project does not
/// collect them as files. Elements are grouped by those categories.
const RESOURCE_EXTENSIONS: &[&str] = &[
    "avif", "bmp", "gif", "heic", "ico", "jpeg", "jpg", "png", "svg", "tif", "tiff",
    "webp", // images
    "aac", "flac", "m4a", "mp3", "oga", "ogg", "opus", "wav", "wma", // audio
    "3gp", "avi", "flv", "m4v", "mkv", "mov", "mp4", "mpeg", "mpg", "webm", "wmv", // video
    "m3u8", "mpd", // streaming manifests
    "7z", "bz2", "cab", "gz", "iso", "rar", "tar", "tgz", "xz", "zip", // archives
    "csv", "doc", "docx", "epub", "odp", "ods", "odt", "pdf", "ppt", "pptx", "rtf",
    "txt", // documents
    "xls", "xlsx", // documents, continued
    "apk", "appimage", "deb", "dmg", "exe", "msi", "pkg", "rpm", // executables and packages
    "eot", "otf", "ttf", "woff", "woff2", // fonts
    "css", "js", "mjs", // stylesheets and scripts
];

/// Path extensions that always denote a fetchable page, server-side scripts
/// included; they take precedence over `RESOURCE_EXTENSIONS`.
const PAGE_EXTENSIONS: &[&str] = &["html", "htm", "php", "asp", "aspx", "jsp", "cgi"];

fn err(message: impl Into<String>) -> Box<dyn Error + Send + Sync> {
    message.into().into()
}

fn default_max_depth() -> u32 {
    3
}

fn default_max_pages() -> usize {
    500
}

fn default_max_file_count() -> usize {
    1000
}

fn default_obey_robots() -> bool {
    true
}

/// Request headers/cookies for a manual login. Sent only to the start host
/// and `allowed_hosts`; never logged (Debug is redacted).
#[derive(Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AuthHeaders(pub Vec<(String, String)>);

impl std::fmt::Debug for AuthHeaders {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AuthHeaders(<{} redacted>)", self.0.len())
    }
}

const MAX_AUTH_HEADERS: usize = 16;
const MAX_PATTERNS: usize = 200;
const MAX_HEADER_VALUE: usize = 8192;

/// A persisted grabber project, analogous to IDM's grabber project files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub start_url: String,
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
    #[serde(default = "default_max_pages")]
    pub max_pages: usize,
    #[serde(default)]
    pub same_host_only: bool,
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
    /// Glob-like patterns (`*`, `?`) matched against the full URL; an empty
    /// list accepts everything. Files rejected by include or hit by exclude
    /// patterns are dropped.
    #[serde(default)]
    pub include_patterns: Vec<String>,
    #[serde(default)]
    pub exclude_patterns: Vec<String>,
    /// Patterns matched against page URLs before they are fetched; matching
    /// pages are neither requested nor followed. `exclude_patterns` also
    /// apply to pages; this list is for crawl-only exclusions (nothing is
    /// excluded by default).
    #[serde(default)]
    pub crawl_exclude_patterns: Vec<String>,
    /// File extensions to collect (no dot), matched case-insensitively
    /// against the URL path, e.g. `["zip", "pdf", "mp4"]`.
    #[serde(default)]
    pub file_types: Vec<String>,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    /// Politeness: minimum inactivity before every request but the first.
    #[serde(default)]
    pub request_delay_ms: u64,
    /// Stop the whole crawl once this many files have been collected.
    #[serde(default = "default_max_file_count")]
    pub max_file_count: usize,
    /// Respect robots.txt `Disallow` rules of our user-agent group.
    #[serde(default = "default_obey_robots")]
    pub obey_robots: bool,
    /// Manual-login headers (for example `Cookie`), used for every request
    /// on the start host and `allowed_hosts` only. Known limitation: the
    /// daemon stores them in `grab_projects` like the rest of the project
    /// (redacted in every listing and Debug output) rather than in the sealed
    /// site-login store, which only holds username/password pairs.
    #[serde(default)]
    pub auth_headers: AuthHeaders,
    /// Save fetched HTML pages under the destination, mirroring URL paths,
    /// and rewrite links to saved pages and files to relative paths.
    /// Existing files are never overwritten (collisions get suffixes).
    #[serde(default)]
    pub save_pages: bool,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            start_url: String::new(),
            max_depth: default_max_depth(),
            max_pages: default_max_pages(),
            same_host_only: false,
            allowed_hosts: Vec::new(),
            include_patterns: Vec::new(),
            exclude_patterns: Vec::new(),
            crawl_exclude_patterns: Vec::new(),
            file_types: Vec::new(),
            headers: Vec::new(),
            request_delay_ms: 0,
            max_file_count: default_max_file_count(),
            obey_robots: true,
            auth_headers: AuthHeaders::default(),
            save_pages: false,
        }
    }
}

impl Project {
    /// Bounds on user-supplied lists and header values.
    pub fn validate(&self) -> Result<()> {
        if self.include_patterns.len() > MAX_PATTERNS
            || self.exclude_patterns.len() > MAX_PATTERNS
            || self.crawl_exclude_patterns.len() > MAX_PATTERNS
            || self.file_types.len() > MAX_PATTERNS
            || self.allowed_hosts.len() > MAX_PATTERNS
        {
            return Err(err("too many patterns, file types or hosts"));
        }
        if self.auth_headers.0.len() > MAX_AUTH_HEADERS {
            return Err(err("too many auth headers"));
        }
        for (name, value) in &self.auth_headers.0 {
            if name.is_empty()
                || name.len() > 128
                || value.len() > MAX_HEADER_VALUE
                || name.bytes().any(|b| b.is_ascii_control() || b == b':')
                || value.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0)
            {
                return Err(err("invalid auth header"));
            }
            let lower = name.to_ascii_lowercase();
            if ["host", "content-length", "transfer-encoding", "connection"].contains(&lower.as_str())
            {
                return Err(err("auth header name is not allowed"));
            }
        }
        Ok(())
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        fs::write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        Ok(serde_json::from_slice(&fs::read(path)?)?)
    }
}

/// A downloadable file discovered on a page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoundFile {
    pub url: String,
    /// URL of the page the link was extracted from; empty for the start URL.
    pub referer: String,
    /// Depth of the file itself: start page links are at depth 1.
    pub depth: u32,
    pub extension: String,
}

/// Outcome of a crawl. Files are classified by URL extension only; they are
/// never requested, so their identity can be wrong in rare edge cases.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrawlResult {
    /// URLs of successfully fetched and parsed HTML pages, in fetch order.
    pub pages: Vec<String>,
    pub files: Vec<FoundFile>,
    /// `(url, reason)` for failed fetches and malformed links, deduplicated
    /// per URL. Host- and robots-filtered candidates are skipped silently.
    pub errors: Vec<(String, String)>,
    /// With `save_pages`: `(page URL, relative path)` of each saved page.
    #[serde(default)]
    pub saved_pages: Vec<(String, String)>,
    /// With `save_pages`: relative path planned for each collected file URL.
    #[serde(default)]
    pub local_paths: BTreeMap<String, String>,
}

/// Runs a crawl with a fresh stop flag.
pub fn run(project: &Project) -> Result<CrawlResult> {
    run_with_stop(project, Arc::new(AtomicBool::new(false)))
}

/// Runs a crawl, polling `stop` between requests and during politeness waits.
pub fn run_with_stop(project: &Project, stop: Arc<AtomicBool>) -> Result<CrawlResult> {
    crawl(project, stop)
}

/// Runs the crawl described by `project`, polling `stop` between requests.
pub fn crawl(project: &Project, stop: Arc<AtomicBool>) -> Result<CrawlResult> {
    crawl_to(project, stop, None)
}

/// Like `crawl`; with `save_dir` and `project.save_pages` the fetched pages
/// are written under `save_dir` and their links rewritten for offline use.
pub fn crawl_to(
    project: &Project,
    stop: Arc<AtomicBool>,
    save_dir: Option<&Path>,
) -> Result<CrawlResult> {
    let crawler = Crawler::new(project, stop, save_dir)?;
    Ok(crawler.run())
}

struct Queued {
    url: Url,
    depth: u32,
}

/// Origin serialization used as the robots.txt cache and lookup key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Origin(String);

fn origin_of(url: &Url) -> Origin {
    Origin(format!(
        "{}://{}{}",
        url.scheme(),
        url.host_str().unwrap_or(""),
        url.port_or_known_default()
            .map_or_else(String::new, |port| format!(":{port}"))
    ))
}

/// True when a link's path ends in a known non-HTML resource extension that
/// the project does not collect. Extension-less paths, page extensions and
/// trailing-slash URLs are pages and must still be fetched.
fn skips_resource(url: &Url) -> bool {
    if url.path().ends_with('/') {
        return false;
    }
    let Some(extension) = pattern::file_extension(url) else {
        return false;
    };
    RESOURCE_EXTENSIONS.contains(&extension.as_str())
        && !PAGE_EXTENSIONS.contains(&extension.as_str())
}

struct Crawler<'a> {
    project: &'a Project,
    file_types: HashSet<String>,
    hosts: HashSet<String>,
    client: Client,
    robots: HashMap<Origin, robots::Robots>,
    /// robots.txt user-agent token derived from the effective User-Agent.
    robots_token: String,
    stop: Arc<AtomicBool>,
    /// Every URL ever considered, so each is processed at most once.
    seen: HashSet<String>,
    /// URLs already recorded in `result.errors`.
    errored: HashSet<String>,
    queue: VecDeque<Queued>,
    result: CrawlResult,
    auth: HeaderMap,
    auth_hosts: HashSet<String>,
    save_dir: Option<PathBuf>,
    mapper: Option<offline::Mapper>,
    requests: usize,
    halted: bool,
}

impl<'a> Crawler<'a> {
    fn new(project: &'a Project, stop: Arc<AtomicBool>, save_dir: Option<&Path>) -> Result<Self> {
        project.validate()?;
        if project.max_pages == 0 {
            return Err(err("max_pages must be greater than zero"));
        }
        let start = Url::parse(project.start_url.trim())
            .map_err(|e| err(format!("invalid start URL: {e}")))?;
        if start.scheme() != "http" && start.scheme() != "https" {
            return Err(err("start_url must use http or https"));
        }
        let mut auth_hosts = project
            .allowed_hosts
            .iter()
            .map(|host| host.trim().to_ascii_lowercase())
            .collect::<HashSet<String>>();
        if let Some(host) = start.host_str() {
            auth_hosts.insert(host.to_ascii_lowercase());
        }
        let mut auth = HeaderMap::new();
        for (name, value) in &project.auth_headers.0 {
            let mut value = HeaderValue::from_str(value)?;
            value.set_sensitive(true);
            auth.append(HeaderName::from_bytes(name.as_bytes())?, value);
        }
        let client = build_client(project, auth_hosts.clone(), true)?;
        let agent = project
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("user-agent"))
            .map(|(_, value)| value.clone())
            .unwrap_or_else(|| DEFAULT_USER_AGENT.to_owned());
        // The token is the agent name before any version suffix, which is
        // what robots.txt user-agent lines refer to.
        let robots_token = agent
            .split('/')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        let file_types = project
            .file_types
            .iter()
            .filter_map(|extension| {
                let name = extension.trim().trim_start_matches('.');
                (!name.is_empty()).then(|| name.to_ascii_lowercase())
            })
            .collect();
        // The start host is always allowed; `allowed_hosts` adds more hosts
        // for `same_host_only` crawls.
        let mut hosts = project
            .allowed_hosts
            .iter()
            .map(|host| host.trim().to_ascii_lowercase())
            .collect::<HashSet<String>>();
        if project.same_host_only {
            if let Some(host) = start.host_str() {
                hosts.insert(host.to_ascii_lowercase());
            }
        }
        Ok(Self {
            project,
            file_types,
            hosts,
            client,
            robots: HashMap::new(),
            robots_token,
            stop,
            seen: HashSet::new(),
            errored: HashSet::new(),
            queue: VecDeque::new(),
            result: CrawlResult::default(),
            auth,
            auth_hosts,
            mapper: save_dir
                .filter(|_| project.save_pages)
                .map(|dir| offline::Mapper::new(dir, start.host_str().unwrap_or(""))),
            save_dir: save_dir.filter(|_| project.save_pages).map(Path::to_path_buf),
            requests: 0,
            halted: false,
        })
    }

    fn run(mut self) -> CrawlResult {
        self.consider(self.project.start_url.clone(), "", 0);
        while !self.halted && !self.stopped() {
            let Some(entry) = self.queue.pop_front() else {
                break;
            };
            if self.page_budget_reached() {
                break;
            }
            self.fetch_page(entry);
            if self.result.files.len() >= self.project.max_file_count {
                self.halted = true;
            }
        }
        self.rewrite_saved_pages();
        self.result
    }

    /// Rewrites links in every saved page to the saved pages and planned
    /// file paths; everything else stays absolute.
    fn rewrite_saved_pages(&mut self) {
        let Some(dir) = self.save_dir.clone() else {
            return;
        };
        let mut local: HashMap<String, String> = self
            .result
            .saved_pages
            .iter()
            .cloned()
            .collect();
        local.extend(self.result.local_paths.clone());
        for (url, rel) in self.result.saved_pages.clone() {
            let path = dir.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
            if offline::check_mirror_file(&dir, &rel).is_err() {
                continue;
            }
            let (Ok(page_url), Ok(text)) = (Url::parse(&url), fs::read_to_string(&path)) else {
                continue;
            };
            if let Some(rewritten) = offline::rewrite(&text, &page_url, &rel, &local) {
                if let Err(error) = fs::write(&path, rewritten) {
                    self.record_error(&url, format!("cannot rewrite saved page: {error}"));
                }
            }
        }
    }

    /// Writes a fetched page into the mirror without overwriting anything.
    fn save_page(&mut self, url: &Url, text: &str) {
        let (Some(dir), Some(mapper)) = (self.save_dir.clone(), self.mapper.as_mut()) else {
            return;
        };
        let rel = mapper.assign(url, true);
        let path = dir.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        let written = offline::create_mirror_dirs(&dir, &rel).and_then(|()| {
                use std::io::Write;
                fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .and_then(|mut file| file.write_all(text.as_bytes()))
            });
        match written {
            Ok(()) => self.result.saved_pages.push((url.to_string(), rel)),
            Err(error) => self.record_error(url.as_str(), format!("cannot save page: {error}")),
        }
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    fn page_budget_reached(&self) -> bool {
        self.result.pages.len() >= self.project.max_pages
    }

    fn record_error(&mut self, url: &str, reason: impl Into<String>) {
        if self.errored.insert(url.to_string()) {
            self.result.errors.push((url.to_string(), reason.into()));
        }
    }

    /// Returns `Some(extension)` when the URL path ends in one of the
    /// project's file types.
    fn classifies_as_file(&self, url: &Url) -> Option<String> {
        let extension = pattern::file_extension(url)?;
        self.file_types.contains(&extension).then_some(extension)
    }

    /// Include/exclude filtering of a file URL, also against its decoded form.
    fn file_wanted(&self, url: &str) -> bool {
        let included = self.project.include_patterns.is_empty()
            || glob::matches_any_url(&self.project.include_patterns, url);
        included && !glob::matches_any_url(&self.project.exclude_patterns, url)
    }

    /// Page-level admission shared by discovered links and redirect hops:
    /// scope, exclusions and robots.txt (fetched only for in-scope,
    /// non-excluded URLs).
    fn page_admitted(&mut self, url: &Url) -> bool {
        if url.scheme() != "http" && url.scheme() != "https" {
            return false;
        }
        if self.project.same_host_only {
            let host = url.host_str().unwrap_or("").to_ascii_lowercase();
            if !self.hosts.contains(&host) {
                return false;
            }
        }
        let text = url.as_str();
        if glob::matches_any_url(&self.project.exclude_patterns, text)
            || glob::matches_any_url(&self.project.crawl_exclude_patterns, text)
        {
            return false;
        }
        self.robots_permits(url)
    }

    fn robots_permits(&mut self, url: &Url) -> bool {
        if !self.project.obey_robots {
            return true;
        }
        let path = url.path().to_string()
            + url
                .query()
                .map_or_else(String::new, |query| format!("?{query}"))
                .as_str();
        self.robots_allows(url, &path)
    }

    /// Records a candidate as a file after applying include/exclude patterns
    /// against the full URL. The crawl is halted once the file budget is hit.
    fn record_file(&mut self, url: &str, referer: &str, depth: u32, extension: String) {
        if !self.file_wanted(url) {
            return;
        }
        if self.result.files.len() >= self.project.max_file_count {
            self.halted = true;
            return;
        }
        if let (Some(mapper), Ok(parsed)) = (self.mapper.as_mut(), Url::parse(url)) {
            let rel = mapper.assign(&parsed, false);
            self.result.local_paths.insert(url.to_string(), rel);
        }
        self.result.files.push(FoundFile {
            url: url.to_string(),
            referer: referer.to_string(),
            depth,
            extension,
        });
    }

    /// Decides whether a URL (page or file link) is worth fetching or
    /// collecting. `depth` is the candidate's own depth: the start URL is at
    /// depth 0 and every link on it is at depth 1; anything deeper than
    /// `max_depth` is dropped, whether it is a page or a file.
    fn consider(&mut self, raw: String, referer: &str, depth: u32) {
        if self.stopped() {
            return;
        }
        let text = raw.trim();
        let Ok(mut candidate) = Url::parse(text) else {
            self.record_error(text, "invalid URL");
            return;
        };
        if candidate.scheme() != "http" && candidate.scheme() != "https" {
            // javascript:, mailto:, data: and friends are never touched.
            return;
        }
        candidate.set_fragment(None);
        let url_text = candidate.to_string();
        if !self.seen.insert(url_text.clone()) {
            return;
        }
        if self.project.same_host_only {
            let host = candidate.host_str().unwrap_or("").to_ascii_lowercase();
            if !self.hosts.contains(&host) {
                return;
            }
        }
        if depth > self.project.max_depth {
            return;
        }
        if let Some(extension) = self.classifies_as_file(&candidate) {
            if self.file_wanted(&url_text) && self.robots_permits(&candidate) {
                self.record_file(&url_text, referer, depth, extension);
            }
            return;
        }
        if depth > 0 && skips_resource(&candidate) {
            // A discovered link to a known non-HTML resource the project does
            // not collect is dropped instead of being requested as a page.
            return;
        }
        if self.page_budget_reached() {
            return;
        }
        // Excluded pages are never fetched, so their links are not followed.
        if !self.page_admitted(&candidate) {
            return;
        }
        self.queue.push_back(Queued {
            url: candidate,
            depth,
        });
    }

    fn robots_allows(&mut self, url: &Url, path: &str) -> bool {
        let origin = origin_of(url);
        if !self.robots.contains_key(&origin) {
            let rules = self.fetch_robots(&origin);
            self.robots.insert(origin.clone(), rules);
        }
        self.robots[&origin].allowed(&self.robots_token, path)
    }

    /// Fetches and parses robots.txt for one origin. A missing file or any
    /// fetch failure is treated as "no restrictions".
    fn fetch_robots(&mut self, origin: &Origin) -> robots::Robots {
        let robots_url = format!("{}/robots.txt", origin.0);
        match self.client.get(robots_url).send() {
            Ok(response) if response.status().is_success() => match read_capped(response) {
                Ok(text) => robots::Robots::parse(&text),
                Err(_) => robots::Robots::default(),
            },
            _ => robots::Robots::default(),
        }
    }

    fn fetch_page(&mut self, entry: Queued) {
        if !self.throttle() {
            return;
        }
        self.requests += 1;
        let page = entry.url.to_string();
        let mut current = entry.url.clone();
        let mut hops = 0;
        let response = loop {
            let mut request = self.client.get(current.as_str());
            if current
                .host_str()
                .is_some_and(|host| self.auth_hosts.contains(&host.to_ascii_lowercase()))
            {
                request = request.headers(self.auth.clone());
            }
            let response = match request.send() {
                Ok(response) => response,
                Err(request_error) => {
                    self.record_error(&page, describe(&request_error));
                    return;
                }
            };
            if !response.status().is_redirection() {
                break response;
            }
            let Some(location) = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| current.join(value.trim()).ok())
            else {
                break response;
            };
            hops += 1;
            let mut next = location;
            next.set_fragment(None);
            if hops > MAX_REDIRECTS {
                self.record_error(&page, "too many redirects");
                return;
            }
            // A hop to another host, an excluded URL or a robots-disallowed
            // path is neither fetched nor followed.
            if !self.page_admitted(&next) || !self.seen.insert(next.to_string()) {
                return;
            }
            current = next;
        };
        let entry = Queued {
            url: current,
            depth: entry.depth,
        };
        let page = entry.url.to_string();
        if !response.status().is_success() {
            self.record_error(&page, format!("HTTP {}", response.status()));
            return;
        }
        let page_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map_or(String::new(), |value| value.to_ascii_lowercase());
        let is_html =
            page_type.contains("text/html") || page_type.contains("application/xhtml+xml");
        if !is_html {
            // Fetched but not parsed: most extensions belong to files, which
            // are collected by URL alone.
            return;
        }
        let text = match read_capped(response) {
            Ok(body) => body,
            Err(read_error) => {
                self.record_error(&page, read_error.to_string());
                return;
            }
        };
        self.result.pages.push(page.clone());
        self.save_page(&entry.url, &text);
        let extracted = html::extract(&text);
        let base = extracted
            .base
            .as_deref()
            .and_then(|value| entry.url.join(value.trim()).ok())
            .unwrap_or(entry.url.clone());
        for value in extracted.urls {
            if let Ok(candidate) = base.join(value.trim()) {
                self.consider(candidate.to_string(), &page, entry.depth + 1);
            }
        }
    }

    /// Sleeps the politeness delay before every request but the first,
    /// breaking early when stopped. `Ok` means the request may proceed.
    fn throttle(&mut self) -> bool {
        if self.requests == 0 {
            return true;
        }
        let until = Instant::now() + Duration::from_millis(self.project.request_delay_ms);
        while Instant::now() < until {
            if self.stopped() {
                return false;
            }
            let remaining = until - Instant::now();
            std::thread::sleep(remaining.min(Duration::from_millis(50)));
        }
        true
    }
}

/// Builds a blocking client with the project's headers; a default User-Agent
/// is installed unless the project supplies one.
fn build_client(
    project: &Project,
    auth_hosts: HashSet<String>,
    manual_redirects: bool,
) -> Result<Client> {
    let mut headers = HeaderMap::new();
    if !project
        .headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("user-agent"))
    {
        headers.insert(USER_AGENT, HeaderValue::from_static(DEFAULT_USER_AGENT));
    }
    for (name, value) in &project.headers {
        let key = HeaderName::from_bytes(name.as_bytes())?;
        headers.append(key, HeaderValue::from_str(value)?);
    }
    let has_auth = !project.auth_headers.0.is_empty();
    // Auth headers must not follow a redirect to a host outside the allowed set.
    let policy = reqwest::redirect::Policy::custom(move |attempt| {
        let host_ok = attempt
            .url()
            .host_str()
            .is_some_and(|host| auth_hosts.contains(&host.to_ascii_lowercase()));
        if attempt.previous().len() >= 10 {
            attempt.error("too many redirects")
        } else if has_auth && !host_ok {
            attempt.stop()
        } else {
            attempt.follow()
        }
    });
    // The crawler follows redirects itself so every hop passes the same
    // scope, robots and exclusion checks as a discovered link.
    let policy = if manual_redirects {
        reqwest::redirect::Policy::none()
    } else {
        policy
    };
    Ok(Client::builder()
        .redirect(policy)
        .timeout(REQUEST_TIMEOUT)
        .default_headers(headers)
        .build()?)
}

/// Fetches a small text document (such as a playlist) over HTTP(S), bounded
/// by `MAX_PAGE_BYTES`.
pub fn fetch_text(url: &str) -> Result<String> {
    let parsed = Url::parse(url.trim()).map_err(|e| err(format!("invalid URL: {e}")))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(err("only http and https URLs can be fetched"));
    }
    let client = build_client(&Project::default(), HashSet::new(), false)?;
    let response = client.get(parsed).send()?;
    if !response.status().is_success() {
        return Err(err(format!("HTTP {}", response.status())));
    }
    read_capped(response)
}

/// Reads a whole body as a lossy UTF-8 string, bounded by MAX_PAGE_BYTES.
fn read_capped(mut response: Response) -> Result<String> {
    let mut text = String::new();
    let mut buffer = [0u8; 16 * 1024];
    loop {
        let read = response.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        if text.len() + read > MAX_PAGE_BYTES {
            return Err(err(format!("page body exceeds {} bytes", MAX_PAGE_BYTES)));
        }
        text.push_str(&String::from_utf8_lossy(&buffer[..read]));
    }
    Ok(text)
}

/// Error text including its cause chain, which reqwest hides.
fn describe(error: &reqwest::Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        message = format!("{message}: {cause}");
        source = cause.source();
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_defaults_fill_in_mmng_fields() {
        let bare: Project =
            serde_json::from_str(r#"{"start_url": "http://example.com/", "file_types": ["zip"]}"#)
                .unwrap();
        assert_eq!(bare.start_url, "http://example.com/");
        assert_eq!(bare.max_depth, 3);
        assert_eq!(bare.max_pages, 500);
        assert_eq!(bare.max_file_count, 1000);
        assert!(bare.obey_robots);
        assert!(!bare.same_host_only);
    }

    #[test]
    fn project_round_trips_through_json() {
        let project = Project {
            start_url: "http://example.com/".into(),
            max_depth: 4,
            max_pages: 100,
            same_host_only: true,
            allowed_hosts: vec!["files.example.net".into()],
            include_patterns: vec!["*.zip".into()],
            exclude_patterns: vec!["*/private/*".into()],
            file_types: vec!["zip".into(), "pdf".into(), "mp4".into()],
            headers: vec![("Cookie".into(), "sid=1".into())],
            request_delay_ms: 250,
            max_file_count: 77,
            obey_robots: false,
            crawl_exclude_patterns: vec!["*/logout*".into()],
            auth_headers: AuthHeaders(vec![("Cookie".into(), "s=1".into())]),
            save_pages: true,
        };
        let path = std::env::temp_dir().join(format!("xdm-grabber-project-{}", std::process::id()));
        project.save(&path).unwrap();
        assert_eq!(Project::load(&path).unwrap(), project);
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn invalid_start_urls_are_rejected() {
        for start_url in ["::not a url::", "ftp://example.com/", "http://"] {
            let project = Project {
                start_url: start_url.into(),
                ..Project::default()
            };
            assert!(run(&project).is_err(), "{start_url}");
        }
    }

    #[test]
    fn stop_flag_before_start_produces_nothing() {
        let project = Project {
            start_url: "http://127.0.0.1:9/".into(),
            max_depth: 1,
            max_pages: 5,
            file_types: vec!["zip".into()],
            ..Project::default()
        };
        let result = run_with_stop(&project, Arc::new(AtomicBool::new(true))).unwrap();
        assert_eq!(result, CrawlResult::default());
    }

    #[test]
    fn file_types_are_normalized() {
        let url = Url::parse("http://a/x.ZiP").unwrap();
        let project = Project {
            start_url: "http://a/".into(),
            file_types: vec!["  .Zip ".into()],
            ..Project::default()
        };
        let crawler = Crawler::new(&project, Arc::new(AtomicBool::new(true)), None).unwrap();
        assert_eq!(crawler.classifies_as_file(&url).as_deref(), Some("zip"));
    }

    #[test]
    fn known_resources_are_skipped_but_pages_are_not() {
        let skipped = |url: &str| skips_resource(&Url::parse(url).unwrap());
        for url in [
            "http://a/img/c.jpg",
            "http://a/img/c.JPG",
            "http://a/site.css",
            "http://a/app.js",
            "http://a/movie.mp4",
            "http://a/stream.m3u8",
            "http://a/pack.7z",
            "http://a/report.pdf",
            "http://a/font.woff2",
            "http://a/payload.exe",
        ] {
            assert!(skipped(url), "{url}");
        }
        for url in [
            "http://a/index.html",
            "http://a/view.php",
            "http://a/scripts/run.cgi",
            "http://a/no-extension",
            "http://a/dir/",
            "http://a/dir/site.css/",
            "http://a/unknown.xyz",
            "http://a/notes.txt.bak",
        ] {
            assert!(!skipped(url), "{url}");
        }
    }
}
