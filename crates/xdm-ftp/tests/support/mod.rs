//! Minimal in-process FTP server used by the integration tests.
//!
//! It speaks just enough of the protocol for the client under test:
//! USER/PASS, TYPE, SIZE, MDTM, EPSV/PASV, REST, RETR and QUIT. Behaviour that
//! the tests need to vary (login failure, multiline banner, REST rejection,
//! foreign PASV address, mid-transfer disconnect, throttling) is configurable.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub struct Config {
    pub content: Vec<u8>,
    pub path: String,
    pub mdtm: String,
    pub multiline_banner: bool,
    pub login_ok: bool,
    pub epsv_supported: bool,
    pub rest_supported: bool,
    pub pasv_foreign_ip: bool,
    /// Drop the data connection after this many bytes on the first RETR only.
    pub disconnect_after: Option<usize>,
    /// Write the payload in chunks of this size (0 = one chunk).
    pub chunk_size: usize,
    pub chunk_delay: Duration,
    /// Reported SIZE, when it should differ from the real payload length.
    pub size_override: Option<u64>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            content: b"hello world".to_vec(),
            path: "/file.bin".to_string(),
            mdtm: "20240102030405".to_string(),
            multiline_banner: false,
            login_ok: true,
            epsv_supported: true,
            rest_supported: true,
            pasv_foreign_ip: false,
            disconnect_after: None,
            chunk_size: 0,
            chunk_delay: Duration::ZERO,
            size_override: None,
        }
    }
}

struct Shared {
    config: Config,
    transfers: AtomicUsize,
    rest_supported: AtomicBool,
    mdtm: Mutex<String>,
}

pub struct Server {
    pub addr: SocketAddr,
    shared: Arc<Shared>,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Server {
    pub fn start(config: Config) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let shared = Arc::new(Shared {
            rest_supported: AtomicBool::new(config.rest_supported),
            mdtm: Mutex::new(config.mdtm.clone()),
            transfers: AtomicUsize::new(0),
            config,
        });
        let shutdown = Arc::new(AtomicBool::new(false));
        let handle = thread::spawn({
            let shared = shared.clone();
            let shutdown = shutdown.clone();
            move || accept_loop(listener, shared, shutdown)
        });
        Server {
            addr,
            shared,
            shutdown,
            handle: Some(handle),
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("ftp://{}{}", self.addr, path)
    }

    pub fn set_rest_supported(&self, value: bool) {
        self.shared.rest_supported.store(value, Ordering::SeqCst);
    }

    pub fn set_mdtm(&self, value: &str) {
        *self.shared.mdtm.lock().unwrap() = value.to_string();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn accept_loop(listener: TcpListener, shared: Arc<Shared>, shutdown: Arc<AtomicBool>) {
    while !shutdown.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).unwrap();
                let shared = shared.clone();
                thread::spawn(move || {
                    let _ = handle_session(stream, shared);
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(_) => break,
        }
    }
}

fn handle_session(stream: TcpStream, shared: Arc<Shared>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    let config = &shared.config;

    if config.multiline_banner {
        writer.write_all(b"220-Welcome to the test server\r\n220-All commands ready\r\n220 Service ready\r\n")?;
    } else {
        writer.write_all(b"220 Service ready\r\n")?;
    }
    writer.flush()?;

    let mut data_listener: Option<TcpListener> = None;
    let mut rest_offset = 0usize;
    let mut user_seen = false;

    loop {
        let Some(line) = read_command(&mut reader)? else {
            return Ok(());
        };
        let mut parts = line.splitn(2, ' ');
        let command = parts.next().unwrap_or("").to_ascii_uppercase();
        let argument = parts.next().unwrap_or("").to_string();

        match command.as_str() {
            "USER" => {
                user_seen = true;
                writer.write_all(b"331 Password required\r\n")?;
            }
            "PASS" => {
                if config.login_ok && user_seen {
                    writer.write_all(b"230 Logged in\r\n")?;
                } else {
                    writer.write_all(b"530 Login incorrect\r\n")?;
                }
            }
            "TYPE" => writer.write_all(b"200 Type set to I\r\n")?,
            "SIZE" => {
                if argument == config.path {
                    let size = config.size_override.unwrap_or(config.content.len() as u64);
                    writer.write_all(format!("213 {size}\r\n").as_bytes())?;
                } else {
                    writer.write_all(b"550 No such file\r\n")?;
                }
            }
            "MDTM" => {
                if argument == config.path {
                    let mdtm = shared.mdtm.lock().unwrap().clone();
                    writer.write_all(format!("213 {mdtm}\r\n").as_bytes())?;
                } else {
                    writer.write_all(b"550 No such file\r\n")?;
                }
            }
            "EPSV" => {
                if config.epsv_supported {
                    let listener = TcpListener::bind("127.0.0.1:0")?;
                    let port = listener.local_addr()?.port();
                    data_listener = Some(listener);
                    writer.write_all(
                        format!("229 Entering Extended Passive Mode (|||{port}|)\r\n").as_bytes(),
                    )?;
                } else {
                    writer.write_all(b"500 EPSV not understood\r\n")?;
                }
            }
            "PASV" => {
                let listener = TcpListener::bind("127.0.0.1:0")?;
                let port = listener.local_addr()?.port();
                data_listener = Some(listener);
                let ip = if config.pasv_foreign_ip {
                    [10, 0, 0, 1]
                } else {
                    [127, 0, 0, 1]
                };
                writer.write_all(
                    format!(
                        "227 Entering Passive Mode ({},{},{},{},{},{})\r\n",
                        ip[0],
                        ip[1],
                        ip[2],
                        ip[3],
                        port / 256,
                        port % 256
                    )
                    .as_bytes(),
                )?;
            }
            "REST" => {
                if shared.rest_supported.load(Ordering::SeqCst) {
                    rest_offset = argument.trim().parse().unwrap_or(0);
                    writer.write_all(b"350 Restarting at requested offset\r\n")?;
                } else {
                    writer.write_all(b"502 REST not implemented\r\n")?;
                }
            }
            "RETR" => {
                if argument != config.path {
                    writer.write_all(b"550 No such file\r\n")?;
                    continue;
                }
                let Some(listener) = data_listener.take() else {
                    writer.write_all(b"425 Use EPSV or PASV first\r\n")?;
                    continue;
                };
                writer.write_all(b"150 Opening data connection\r\n")?;
                writer.flush()?;
                let Some(mut data) = accept_with_timeout(&listener) else {
                    writer.write_all(b"425 Cannot open data connection\r\n")?;
                    continue;
                };
                let attempt = shared.transfers.fetch_add(1, Ordering::SeqCst);
                let start = rest_offset.min(config.content.len());
                let body = &config.content[start..];
                let truncate = config.disconnect_after.filter(|_| attempt == 0);
                let complete = send_data(&mut data, body, config, truncate);
                drop(data);
                if complete {
                    writer.write_all(b"226 Transfer complete\r\n")?;
                } else {
                    writer.write_all(b"426 Connection closed; transfer aborted\r\n")?;
                }
                rest_offset = 0;
            }
            "QUIT" => {
                writer.write_all(b"221 Goodbye\r\n")?;
                writer.flush()?;
                return Ok(());
            }
            _ => writer.write_all(b"500 Unknown command\r\n")?,
        }
        writer.flush()?;
    }
}

fn accept_with_timeout(listener: &TcpListener) -> Option<TcpStream> {
    listener.set_nonblocking(true).ok()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = stream.set_nonblocking(false);
                return Some(stream);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() > deadline {
                    return None;
                }
                thread::sleep(Duration::from_millis(5));
            }
            Err(_) => return None,
        }
    }
}

/// Returns true when the whole payload was sent and the data connection can be
/// closed cleanly, false when the transfer was truncated or the peer vanished.
fn send_data(
    data: &mut TcpStream,
    body: &[u8],
    config: &Config,
    truncate: Option<usize>,
) -> bool {
    let _ = data.set_write_timeout(Some(Duration::from_secs(5)));
    let chunk = if config.chunk_size == 0 {
        body.len().max(1)
    } else {
        config.chunk_size
    };
    let limit = truncate.unwrap_or(body.len());
    let mut sent = 0;
    while sent < limit {
        let end = (sent + chunk).min(limit);
        if data.write_all(&body[sent..end]).is_err() {
            return false;
        }
        let _ = data.flush();
        sent = end;
        if !config.chunk_delay.is_zero() {
            thread::sleep(config.chunk_delay);
        }
    }
    truncate.is_none()
}

fn read_command(reader: &mut BufReader<TcpStream>) -> std::io::Result<Option<String>> {
    let mut buffer = String::new();
    if reader.read_line(&mut buffer)? == 0 {
        return Ok(None);
    }
    while buffer.ends_with('\n') || buffer.ends_with('\r') {
        buffer.pop();
    }
    Ok(Some(buffer))
}
