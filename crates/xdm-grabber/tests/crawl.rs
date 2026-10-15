//! Integration tests for the Site Grabber against a local HTTP server.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use xdm_grabber::{CrawlResult, FoundFile, MAX_PAGE_BYTES, Project, crawl};

struct Route {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
}

fn page(body: &str) -> Route {
    Route {
        status: 200,
        content_type: "text/html; charset=utf-8",
        body: body.as_bytes().to_vec(),
    }
}

fn body(content_type: &'static str, body: &str) -> Route {
    Route {
        status: 200,
        content_type,
        body: body.as_bytes().to_vec(),
    }
}

/// A disposable single-threaded HTTP/1.1 server on `127.0.0.1`.
struct TestServer {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    hits: Arc<Mutex<Vec<String>>>,
    cookies: Arc<Mutex<Vec<String>>>,
    handle: Option<JoinHandle<()>>,
}

impl TestServer {
    fn start(routes: Vec<(&str, Route)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let routes: Arc<HashMap<String, Route>> = Arc::new(
            routes
                .into_iter()
                .map(|(path, route)| (path.to_string(), route))
                .collect(),
        );
        let stop = Arc::new(AtomicBool::new(false));
        let hits = Arc::new(Mutex::new(Vec::new()));
        let cookies = Arc::new(Mutex::new(Vec::new()));
        let (thread_stop, thread_routes, thread_hits, thread_cookies) =
            (stop.clone(), routes.clone(), hits.clone(), cookies.clone());
        let handle = thread::spawn(move || {
            for stream in listener.incoming() {
                if thread_stop.load(Ordering::Relaxed) {
                    break;
                }
                if let Ok(mut stream) = stream {
                    let _ = respond(&mut stream, &thread_routes, &thread_hits, &thread_cookies);
                }
            }
        });
        Self {
            addr,
            stop,
            hits,
            cookies,
            handle: Some(handle),
        }
    }

    fn base(&self) -> String {
        format!("http://{}/", self.addr)
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    fn request_count(&self) -> usize {
        self.hits.lock().unwrap().len()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Wake the blocking accept and let the thread observe the stop flag.
        let _ = TcpStream::connect(self.addr);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn respond(
    stream: &mut TcpStream,
    routes: &HashMap<String, Route>,
    hits: &Mutex<Vec<String>>,
    cookies: &Mutex<Vec<String>>,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("cookie:") {
            cookies.lock().unwrap().push(value.trim().to_string());
        }
    }
    let path = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .to_string();
    hits.lock().unwrap().push(path.clone());
    let key = path.split('?').next().unwrap_or("/");
    let (status, content_type, body) = match routes.get(key) {
        Some(route) => (route.status, route.content_type, route.body.as_slice()),
        None => (404, "text/plain", b"not found".as_slice()),
    };
    let reason = if status == 200 { "OK" } else { "Not Found" };
    // Status 302 routes carry their Location target as the body.
    let (location, body) = if status == 302 {
        (
            format!("Location: {}\r\n", String::from_utf8_lossy(body)),
            b"".as_slice(),
        )
    } else {
        (String::new(), body)
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\n{location}\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()
}

/// The site graph shared by most tests.
fn site() -> TestServer {
    let root = r##"<!doctype html>
        <html><head><title>root</title>
        <a href="/b.html">b</a>
        <a href='c.html'>c</a>
        <a href="c.html">c again</a>
        <a href="c.html#section">c fragment</a>
        <a href="#top">top</a>
        <a href="javascript:void(0)">js</a>
        <a href="mailto:a@b.c">mail</a>
        <a href="data:text/plain,hi">data</a>
        <a href="http://external.invalid/x.html">external</a>
        <a href="/files/archive.zip">zip</a>
        <a href="/files/movie.mp4">mp4</a>
        <img src="/files/pic.jpg">
        <a href="/files/doc.pdf">pdf</a>
        <a href="/files/private/secret.zip">secret</a>
        <a href="/nothtml.html">nothtml</a>
        <a href="/oversize.html">oversize</a>
        </head></html>"##;
    let b = r#"<html><head><base href="/sub/"></head><body>
        <a href="d.html">d</a>
        <a href="../files/notes.txt">notes</a>
        </body></html>"#;
    let c = r#"<html><body><a href="/">home</a></body></html>"#;
    let d = r#"<html><body><a href="/nested.html">nested</a></body></html>"#;
    let nested = r#"<html><body><a href="/c.html">cycle</a></body></html>"#;
    let not_html = r#"<html><body><a href="/files/leaked.zip">leak</a></body></html>"#;
    TestServer::start(vec![
        ("/", page(root)),
        ("/b.html", page(b)),
        ("/c.html", page(c)),
        ("/sub/d.html", page(d)),
        ("/nested.html", page(nested)),
        ("/nothtml.html", body("application/json", not_html)),
        (
            "/oversize.html",
            Route {
                status: 200,
                content_type: "text/html",
                body: vec![b'x'; MAX_PAGE_BYTES + 16],
            },
        ),
        (
            "/robots.txt",
            body("text/plain", "User-agent: *\nDisallow: /files/private/\n"),
        ),
    ])
}

fn project(server: &TestServer) -> Project {
    Project {
        start_url: server.base(),
        max_depth: 3,
        max_pages: 50,
        same_host_only: true,
        file_types: ["zip", "pdf", "mp4", "jpg", "txt"]
            .iter()
            .map(|kind| kind.to_string())
            .collect(),
        ..Project::default()
    }
}

fn run(_server: &TestServer, project: &Project) -> CrawlResult {
    crawl(project, Arc::new(AtomicBool::new(false))).unwrap()
}

fn file_urls(result: &CrawlResult) -> Vec<String> {
    let mut urls: Vec<String> = result.files.iter().map(|file| file.url.clone()).collect();
    urls.sort();
    urls
}

#[test]
fn discovers_the_expected_pages_and_files() {
    let server = site();
    let result = run(&server, &project(&server));

    let mut pages = result.pages.clone();
    pages.sort();
    let mut expected_pages = vec![
        server.base(),
        server.url("/b.html"),
        server.url("/c.html"),
        server.url("/nested.html"),
        server.url("/sub/d.html"),
    ];
    expected_pages.sort();
    assert_eq!(pages, expected_pages);

    let mut expected = vec![
        server.url("/files/archive.zip"),
        server.url("/files/doc.pdf"),
        server.url("/files/movie.mp4"),
        server.url("/files/notes.txt"),
        server.url("/files/pic.jpg"),
    ];
    expected.sort();
    assert_eq!(file_urls(&result), expected);
}

#[test]
fn records_depth_and_referer_for_files() {
    let server = site();
    let result = run(&server, &project(&server));

    let zip = result
        .files
        .iter()
        .find(|file| file.url.ends_with("archive.zip"))
        .unwrap();
    assert_eq!(zip.depth, 1);
    assert_eq!(zip.referer, server.base());
    assert_eq!(zip.extension, "zip");

    let notes = result
        .files
        .iter()
        .find(|file| file.url.ends_with("notes.txt"))
        .unwrap();
    assert_eq!(notes.depth, 2);
    assert_eq!(notes.referer, server.url("/b.html"));
}

#[test]
fn respects_robots_disallow_and_can_be_disabled() {
    let server = site();
    let mut obeying = project(&server);
    obeying.obey_robots = true;
    let result = run(&server, &obeying);
    assert!(
        !result
            .files
            .iter()
            .any(|file| file.url.contains("secret.zip"))
    );

    let mut ignoring = project(&server);
    ignoring.obey_robots = false;
    let result = run(&server, &ignoring);
    assert!(
        result
            .files
            .iter()
            .any(|file| file.url.contains("secret.zip"))
    );
}

#[test]
fn skips_external_hosts_and_non_html_bodies() {
    let server = site();
    let result = run(&server, &project(&server));
    assert!(
        !result
            .pages
            .iter()
            .any(|page| page.contains("external.invalid"))
    );
    assert!(
        !result
            .pages
            .iter()
            .any(|page| page.contains("nothtml.html"))
    );
    // A link inside a non-HTML body is never harvested.
    assert!(
        !result
            .files
            .iter()
            .any(|file| file.url.contains("leaked.zip"))
    );
}

#[test]
fn reports_oversize_pages_as_errors_without_adding_them() {
    let server = site();
    let result = run(&server, &project(&server));
    assert!(
        !result
            .pages
            .iter()
            .any(|page| page.contains("oversize.html"))
    );
    assert_eq!(
        result.errors.len(),
        1,
        "unexpected errors: {:?}",
        result.errors
    );
    assert!(result.errors[0].0.contains("oversize.html"));
}

#[test]
fn max_file_count_stops_the_crawl() {
    let server = site();
    let mut limited = project(&server);
    limited.max_file_count = 2;
    let result = run(&server, &limited);
    assert_eq!(result.files.len(), 2);
    assert_eq!(
        result.files,
        vec![
            FoundFile {
                url: server.url("/files/archive.zip"),
                referer: server.base(),
                depth: 1,
                extension: "zip".into(),
            },
            FoundFile {
                url: server.url("/files/movie.mp4"),
                referer: server.base(),
                depth: 1,
                extension: "mp4".into(),
            },
        ]
    );
    assert_eq!(result.pages, vec![server.base()]);
}

#[test]
fn max_depth_bounds_link_following() {
    let server = site();
    let mut limited = project(&server);
    limited.max_depth = 1;
    let result = run(&server, &limited);
    let mut pages = result.pages.clone();
    pages.sort();
    let mut expected = vec![server.base(), server.url("/b.html"), server.url("/c.html")];
    expected.sort();
    assert_eq!(pages, expected);
    assert!(
        !result
            .files
            .iter()
            .any(|file| file.url.ends_with("notes.txt"))
    );
}

#[test]
fn max_pages_bounds_the_fetched_pages() {
    let server = site();
    let mut limited = project(&server);
    limited.max_pages = 1;
    let result = run(&server, &limited);
    assert_eq!(result.pages, vec![server.base()]);
    assert!(
        result
            .files
            .iter()
            .any(|file| file.url.ends_with("archive.zip"))
    );
}

#[test]
fn include_and_exclude_patterns_filter_files() {
    let server = site();
    let mut filtered = project(&server);
    filtered.include_patterns = vec!["*.zip".into()];
    filtered.exclude_patterns = vec!["*/private/*".into()];
    let result = run(&server, &filtered);
    assert_eq!(result.files.len(), 1);
    assert!(result.files[0].url.ends_with("archive.zip"));
}

#[test]
fn stop_flag_prevents_any_request() {
    let server = site();
    let stop = Arc::new(AtomicBool::new(true));
    let result = crawl(&project(&server), stop).unwrap();
    assert!(result.pages.is_empty());
    assert!(result.files.is_empty());
    // Only the initial connection attempt may have raced; the crawl itself
    // must not have walked the graph.
    assert!(server.request_count() <= 1);
}

#[test]
fn rejects_a_non_http_start_url() {
    let project = Project {
        start_url: "ftp://example.com/file".into(),
        ..Project::default()
    };
    assert!(crawl(&project, Arc::new(AtomicBool::new(false))).is_err());
}

#[test]
fn known_resource_links_are_never_requested() {
    let root = r#"<!doctype html><html><head>
        <link rel="stylesheet" href="/assets/site.css">
        <img src="/images/c.jpg">
        <a href="/page.php">php page</a>
        <a href="/no-extension">bare</a>
        <a href="/folder/">folder</a>
        <a href="/files/report.pdf">pdf</a>
        <a href="/videos/clip.webm">webm</a>
        <a href="/scripts/app.js">js</a>
        </head><body></body></html>"#;
    let server = TestServer::start(vec![
        ("/", page(root)),
        ("/page.php", page("<html><body>php</body></html>")),
        ("/no-extension", page("<html><body>bare</body></html>")),
        ("/folder/", page("<html><body>folder</body></html>")),
    ]);
    let mut project = project(&server);
    project.file_types = vec!["pdf".into()];
    project.obey_robots = false;
    let result = run(&server, &project);

    let mut hits = server.hits.lock().unwrap().clone();
    hits.sort();
    let mut expected_hits = vec![
        "/".to_string(),
        "/folder/".into(),
        "/no-extension".into(),
        "/page.php".into(),
    ];
    expected_hits.sort();
    assert_eq!(hits, expected_hits, "resource links must not be requested");

    let mut pages = result.pages.clone();
    pages.sort();
    let mut expected_pages = vec![
        server.base(),
        server.url("/folder/"),
        server.url("/no-extension"),
        server.url("/page.php"),
    ];
    expected_pages.sort();
    assert_eq!(pages, expected_pages);
    assert_eq!(file_urls(&result), vec![server.url("/files/report.pdf")]);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn excluded_pages_are_never_fetched_or_followed() {
    let server = TestServer::start(vec![
        ("/", page(r#"<a href="/logout">out</a><a href="/ok.html">ok</a><a href="/a.zip">a</a>"#)),
        ("/logout", page(r#"<a href="/secret.zip">s</a>"#)),
        ("/ok.html", page("<p>ok</p>")),
    ]);
    let project = Project {
        exclude_patterns: vec!["*logout*".into()],
        obey_robots: false,
        ..project(&server)
    };
    let result = run(&server, &project);
    assert!(!server.hits.lock().unwrap().iter().any(|p| p == "/logout"));
    assert!(result.files.iter().all(|f| !f.url.contains("secret")));
    // The crawl-only list excludes pages without touching file filtering.
    let project = Project {
        crawl_exclude_patterns: vec!["*ok.html".into()],
        obey_robots: false,
        ..self::project(&server)
    };
    let before = server.request_count();
    let result = run(&server, &project);
    assert!(result.pages.iter().all(|p| !p.ends_with("ok.html")));
    assert!(server.request_count() > before);
}

#[test]
fn login_cookies_go_only_to_allowed_hosts() {
    let other = TestServer::start(vec![("/x.html", page("<p>x</p>"))]);
    let port = other.addr.port();
    let home = page(&format!(r#"<a href="http://localhost:{port}/x.html">x</a>"#));
    let server = TestServer::start(vec![("/", home)]);
    let project = Project {
        auth_headers: xdm_grabber::AuthHeaders(vec![("Cookie".into(), "sid=secret".into())]),
        same_host_only: false,
        obey_robots: false,
        ..project(&server)
    };
    let result = run(&server, &project);
    assert_eq!(server.cookies.lock().unwrap().as_slice(), ["sid=secret"]);
    assert_eq!(other.request_count(), 1, "{result:?}");
    assert!(other.cookies.lock().unwrap().is_empty(), "cookie leaked to another host");
    assert!(!format!("{project:?}").contains("secret"), "Debug must redact");
}

#[test]
fn saved_pages_mirror_paths_and_rewrite_links_without_overwriting() {
    let server = TestServer::start(vec![
        ("/", page(r#"<a href="/d/p.html?id=1">1</a><a href="/d/p.html?id=2">2</a><a href="/files/a.zip">a</a><a href="http://elsewhere.invalid/z.zip">z</a>"#)),
        ("/d/p.html", page(r#"<a href="../files/a.zip">a</a><a href="/">home</a>"#)),
    ]);
    let dir = std::env::temp_dir().join(format!("xdm-offline-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("index.html"), "mine").unwrap();
    let project = Project {
        save_pages: true,
        obey_robots: false,
        ..project(&server)
    };
    let result =
        xdm_grabber::crawl_to(&project, Arc::new(AtomicBool::new(false)), Some(&dir)).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("index.html")).unwrap(), "mine");
    let root_rel = &result.saved_pages[0].1;
    assert_eq!(root_rel, "index-2.html");
    let root = std::fs::read_to_string(dir.join(root_rel)).unwrap();
    assert!(root.contains(r#"href="files/a.zip""#), "{root}");
    assert!(root.contains("http://elsewhere.invalid/z.zip"), "{root}");
    assert_eq!(result.saved_pages.len(), 3, "{:?}", result.saved_pages);
    let variants: Vec<_> = result.saved_pages[1..].iter().map(|(_, rel)| rel.clone()).collect();
    assert_ne!(variants[0], variants[1]);
    let page = std::fs::read_to_string(dir.join(&variants[0])).unwrap();
    assert!(page.contains(r#"href="../files/a.zip""#), "{page}");
    assert!(page.contains(r#"href="../index-2.html""#), "{page}");
    assert_eq!(result.local_paths.values().next().unwrap(), "files/a.zip");
    std::fs::remove_dir_all(&dir).unwrap();
}

fn redirect(target: &str) -> Route {
    Route { status: 302, content_type: "text/plain", body: target.as_bytes().to_vec() }
}

#[test]
fn redirects_to_other_hosts_or_excluded_paths_are_not_followed() {
    for auth in [false, true] {
        let other = TestServer::start(vec![("/x.html", page(r#"<a href="/leak.zip">l</a>"#))]);
        let server = TestServer::start(vec![
            ("/", page(r#"<a href="/hop1">1</a><a href="/hop2">2</a><a href="/ok.html">ok</a>"#)),
            ("/hop1", redirect(&format!("http://localhost:{}/x.html", other.addr.port()))),
            ("/hop2", redirect("/logout")),
            ("/logout", page(r#"<a href="/secret.zip">s</a>"#)),
            ("/ok.html", page("<p>ok</p>")),
        ]);
        let dir = std::env::temp_dir().join(format!("xdm-redir-{}-{auth}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let project = Project {
            exclude_patterns: vec!["*logout*".into()],
            obey_robots: false,
            save_pages: true,
            auth_headers: if auth {
                xdm_grabber::AuthHeaders(vec![("Cookie".into(), "sid=1".into())])
            } else {
                Default::default()
            },
            ..project(&server)
        };
        let result =
            xdm_grabber::crawl_to(&project, Arc::new(AtomicBool::new(false)), Some(&dir)).unwrap();
        assert_eq!(other.request_count(), 0, "auth={auth}");
        assert!(!server.hits.lock().unwrap().iter().any(|p| p == "/logout"), "auth={auth}");
        assert!(result.files.iter().all(|f| !f.url.contains("secret") && !f.url.contains("leak")));
        assert_eq!(result.pages.len(), 2, "{:?}", result.pages);
        assert_eq!(result.saved_pages.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn allowed_redirects_are_followed_and_checked_against_robots() {
    let server = TestServer::start(vec![
        ("/", page(r#"<a href="/go">go</a><a href="/to-private">p</a>"#)),
        ("/go", redirect("/landed.html")),
        ("/landed.html", page(r#"<a href="/a.zip">a</a>"#)),
        ("/to-private", redirect("/private/x.html")),
        ("/private/x.html", page(r#"<a href="/b.zip">b</a>"#)),
        ("/robots.txt", body("text/plain", "User-agent: *\nDisallow: /private/\n")),
    ]);
    let result = run(&server, &project(&server));
    assert!(result.pages.iter().any(|p| p.ends_with("/landed.html")), "{:?}", result.pages);
    assert_eq!(file_urls(&result), vec![server.url("/a.zip")]);
    assert!(!server.hits.lock().unwrap().iter().any(|p| p == "/private/x.html"));
}

#[test]
fn percent_encoded_urls_cannot_bypass_exclusions() {
    let server = TestServer::start(vec![
        ("/", page(r#"<a href="/log%6Fut">out</a><a href="/s%2Fa.zip">z</a>"#)),
        ("/logout", page("<p>bye</p>")),
        ("/log%6Fut", page("<p>bye</p>")),
    ]);
    let project = Project {
        exclude_patterns: vec!["*logout*".into(), "*/s/a.zip".into()],
        obey_robots: false,
        ..project(&server)
    };
    let result = run(&server, &project);
    assert_eq!(server.request_count(), 1, "{:?}", server.hits.lock().unwrap());
    assert!(result.files.is_empty());
}

#[test]
fn robots_is_not_fetched_for_excluded_or_out_of_scope_hosts() {
    let other = TestServer::start(vec![("/logout", page("<p>x</p>"))]);
    let port = other.addr.port();
    let server = TestServer::start(vec![(
        "/",
        page(&format!(r#"<a href="http://localhost:{port}/logout">x</a><a href="http://localhost:{port}/deep.zip">z</a>"#)),
    )]);
    let project = Project {
        same_host_only: false,
        exclude_patterns: vec!["*logout*".into(), "*deep.zip".into()],
        obey_robots: true,
        ..project(&server)
    };
    run(&server, &project);
    assert_eq!(other.request_count(), 0);
}

#[cfg(unix)]
#[test]
fn saving_never_follows_symlinked_directories_out_of_the_destination() {
    let server = TestServer::start(vec![
        ("/", page(r#"<a href="/files/p.html">p</a>"#)),
        ("/files/p.html", page("<p>p</p>")),
    ]);
    let base = std::env::temp_dir().join(format!("xdm-symlink-crawl-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (root, outside) = (base.join("root"), base.join("outside"));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("files")).unwrap();
    let project = Project { save_pages: true, obey_robots: false, ..project(&server) };
    let result =
        xdm_grabber::crawl_to(&project, Arc::new(AtomicBool::new(false)), Some(&root)).unwrap();
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
    assert!(result.errors.iter().any(|(_, why)| why.contains("cannot save page")), "{result:?}");
    std::fs::remove_dir_all(&base).unwrap();
}
