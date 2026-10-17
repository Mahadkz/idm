//! Single running GUI per data directory. The first instance listens on a local socket; a later launch
//! sends it a command ("show" or "url <address>") and exits. A crashed instance leaves a dead socket file,
//! which fails to connect and is replaced.
use interprocess::local_socket::{prelude::*, GenericFilePath, GenericNamespaced, ListenerOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::sync::mpsc::{channel, Receiver};

pub enum Signal {
    Show,
    Url(String),
}

fn name(dir: &Path) -> std::io::Result<interprocess::local_socket::Name<'static>> {
    if cfg!(windows) {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        dir.hash(&mut hasher);
        format!("xdm-gui-{:016x}", hasher.finish()).to_ns_name::<GenericNamespaced>()
    } else {
        dir.join("gui.sock").to_string_lossy().into_owned().to_fs_name::<GenericFilePath>()
    }
}

fn send(dir: &Path, line: &str) -> bool {
    let Ok(name) = name(dir) else { return false };
    let Ok(mut stream) = interprocess::local_socket::Stream::connect(name) else { return false };
    stream.write_all(line.as_bytes()).and_then(|_| stream.write_all(b"\n")).is_ok()
}

/// Ok(Some(receiver)) when this process is the instance; Ok(None) when a running one was signalled.
pub fn acquire(dir: &Path, url: Option<&str>) -> std::io::Result<Option<Receiver<Signal>>> {
    let line = url.map(|u| format!("url {u}")).unwrap_or_else(|| "show".into());
    for _ in 0..3 {
        if send(dir, &line) {
            return Ok(None);
        }
        #[cfg(unix)]
        let _ = std::fs::remove_file(dir.join("gui.sock"));
        match ListenerOptions::new().name(name(dir)?).create_sync() {
            Ok(listener) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(dir.join("gui.sock"), std::fs::Permissions::from_mode(0o600));
                }
                let (tx, rx) = channel();
                std::thread::spawn(move || {
                    for stream in listener.incoming().flatten() {
                        let mut text = String::new();
                        let _ = BufReader::new(stream.take_limited()).read_line(&mut text);
                        let text = text.trim();
                        let signal = match text.strip_prefix("url ") {
                            Some(url) => Signal::Url(url.to_owned()),
                            None => Signal::Show,
                        };
                        if tx.send(signal).is_err() {
                            break;
                        }
                    }
                });
                return Ok(Some(rx));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => std::thread::sleep(std::time::Duration::from_millis(200)),
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::AddrInUse, "another xdm-gui is starting"))
}

trait Limited: std::io::Read + Sized {
    fn take_limited(self) -> std::io::Take<Self> {
        std::io::Read::take(self, 9000)
    }
}
impl<T: std::io::Read> Limited for T {}

/// Un-minimizes, shows and asks the window system to focus a window. Wayland compositors may refuse
/// focus-stealing; X11 and Windows normally honor it.
pub fn raise<C: slint::ComponentHandle>(component: &C) {
    use slint::winit_030::WinitWindowAccessor;
    let window = component.window();
    window.set_minimized(false);
    let _ = component.show();
    window.with_winit_window(|w| {
        w.set_minimized(false);
        w.focus_window();
    });
}
