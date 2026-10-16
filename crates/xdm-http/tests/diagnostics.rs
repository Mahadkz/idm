//! N17: each failure stage ends with its own `ErrorKind`, against local fixtures.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use xdm_http::{ErrorKind, Options, TransferControl, classify, download_with_stall_timeout};

fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xdm-diag-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(url: String, output: PathBuf, stall: Duration) -> Box<dyn std::error::Error + Send + Sync> {
    let options = Options {
        url,
        output,
        connections: 1,
        proxy: xdm_http::Proxy::Direct,
        ..Default::default()
    };
    let control = Arc::new(TransferControl::new(None).unwrap());
    download_with_stall_timeout(options, control, stall).expect_err("the transfer should fail")
}

fn kind_of(error: &(dyn std::error::Error + Send + Sync + 'static)) -> ErrorKind {
    classify(error)
}

fn serve(handler: impl Fn(TcpStream) + Send + Sync + 'static) -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let handler = Arc::new(handler);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let handler = handler.clone();
            std::thread::spawn(move || handler(stream));
        }
    });
    port
}

fn read_request(stream: &mut TcpStream) {
    let mut seen = Vec::new();
    let mut byte = [0u8; 1];
    while !seen.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).unwrap_or(0) == 0 {
            break;
        }
        seen.push(byte[0]);
    }
}

#[test]
fn refused_port_is_connect_refused() {
    let port = TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
    // Windows retries a refused loopback connect for ~2 s; the client floors its connect
    // budget above that, so a 2 s stall timeout still classifies as refused.
    let error = run(format!("http://127.0.0.1:{port}/f"), dir("refused").join("f"), Duration::from_secs(2));
    assert_eq!(kind_of(error.as_ref()), ErrorKind::ConnectRefused, "{error}");
}

#[test]
fn unresolvable_name_is_dns() {
    let error = run("http://no-such-host.invalid/f".into(), dir("dns").join("f"), Duration::from_secs(5));
    assert_eq!(kind_of(error.as_ref()), ErrorKind::Dns, "{error}");
}

#[test]
fn unroutable_address_is_connect_timeout_or_unreachable() {
    // 192.0.2.1 (TEST-NET-1) never answers; a sandbox without a route fails at once.
    let error = run("http://192.0.2.1:81/f".into(), dir("unroutable").join("f"), Duration::from_secs(1));
    let kind = kind_of(error.as_ref());
    assert!(
        matches!(kind, ErrorKind::ConnectTimeout | ErrorKind::Network),
        "{kind:?}: {error}"
    );
}

#[test]
fn full_accept_queue_is_connect_timeout() {
    // A listener that never accepts, with its backlog exhausted, leaves SYNs unanswered.
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut fillers = Vec::new();
    for _ in 0..4096 {
        match TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(100)) {
            Ok(stream) => fillers.push(stream),
            Err(_) => break,
        }
    }
    let error = run(format!("http://127.0.0.1:{port}/f"), dir("backlog").join("f"), Duration::from_secs(1));
    assert_eq!(kind_of(error.as_ref()), ErrorKind::ConnectTimeout, "{error}");
    drop((listener, fillers));
}

#[test]
fn silent_server_is_read_timeout() {
    let port = serve(|mut stream| {
        read_request(&mut stream);
        std::thread::sleep(Duration::from_secs(30));
    });
    let error = run(format!("http://127.0.0.1:{port}/f"), dir("silent").join("f"), Duration::from_secs(1));
    assert_eq!(kind_of(error.as_ref()), ErrorKind::ReadTimeout, "{error}");
}

#[test]
fn error_status_is_http_status() {
    let port = serve(|mut stream| {
        read_request(&mut stream);
        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    });
    let error = run(format!("http://127.0.0.1:{port}/f"), dir("status").join("f"), Duration::from_secs(2));
    assert_eq!(kind_of(error.as_ref()), ErrorKind::HttpStatus, "{error}");
}

#[test]
fn unavailable_server_exhausting_retries_is_http_status() {
    let port = serve(|mut stream| {
        read_request(&mut stream);
        let _ = stream.write_all(b"HTTP/1.1 503 Busy\r\nContent-Length: 0\r\nRetry-After: 0\r\nConnection: close\r\n\r\n");
    });
    let error = run(format!("http://127.0.0.1:{port}/f"), dir("busy").join("f"), Duration::from_secs(2));
    assert_eq!(kind_of(error.as_ref()), ErrorKind::HttpStatus, "{error}");
}

#[cfg(unix)]
#[test]
fn unwritable_destination_is_disk() {
    let port = serve(|mut stream| {
        read_request(&mut stream);
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndata");
    });
    // /dev/full accepts opens and fails writes with ENOSPC.
    if !std::path::Path::new("/dev/full").exists() {
        return;
    }
    let base = dir("disk");
    let output = base.join("f");
    let part = base.join("f.part");
    std::os::unix::fs::symlink("/dev/full", &part).unwrap();
    let error = run(format!("http://127.0.0.1:{port}/f"), output, Duration::from_secs(2));
    assert_eq!(kind_of(error.as_ref()), ErrorKind::Disk, "{error}");
}
