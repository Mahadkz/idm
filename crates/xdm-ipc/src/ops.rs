//! Operational plumbing shared by every binary: version strings, secret
//! redaction, a small rotating file logger, crash reports, the diagnostics
//! zip writer and the IPC handshake check.

use crate::{Command, PROTOCOL, Paths};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const GIT_COMMIT: &str = env!("XDM_GIT_COMMIT");

pub fn version_line(binary: &str) -> String {
    format!("{binary} {VERSION} (commit {GIT_COMMIT}, ipc protocol {PROTOCOL})")
}

/// Prints the version and exits when the first argument asks for it.
pub fn handle_version_flag(binary: &str) {
    if matches!(std::env::args().nth(1).as_deref(), Some("--version" | "-V")) {
        println!("{}", version_line(binary));
        std::process::exit(0);
    }
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

// ---------------------------------------------------------------- redaction

const HEADER_NAMES: &[&str] = &["authorization", "proxy-authorization", "cookie", "set-cookie", "x-api-key"];
const KEY_NAMES: &[&str] = &[
    "password", "passwd", "pwd", "secret", "token", "apikey", "api_key", "api-key", "access_key", "signature",
    "sessionid", "session", "sig", "auth", "credential", "body",
];
const MASK: &str = "<redacted>";

fn redact_urls(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        let (head, tail) = rest.split_at(at + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | ')' | ',' | '}' | '`'))
            .unwrap_or(tail.len());
        let (url, after) = tail.split_at(end);
        let authority_end = url.find(['/', '?', '#']).unwrap_or(url.len());
        let (authority, path) = url.split_at(authority_end);
        match authority.rfind('@') {
            Some(i) => {
                out.push_str(MASK);
                out.push_str(&authority[i..]);
            }
            None => out.push_str(authority),
        }
        let cut = path.find(['?', '#']).unwrap_or(path.len());
        out.push_str(&path[..cut]);
        if cut < path.len() {
            out.push_str("?<redacted>");
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

fn is_name_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-')
}

/// Masks the value following every sensitive name (`name: v`, `name=v`,
/// `"name":"v"`). Header values run to end of line; other values stop at a
/// delimiter or at their closing quote.
fn redact_names(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let (bytes, lbytes) = (text.as_bytes(), lower.as_bytes());
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut copied = 0;
    while i < bytes.len() {
        if !is_name_char(lbytes[i]) || (i > 0 && is_name_char(lbytes[i - 1])) {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < bytes.len() && is_name_char(lbytes[j]) {
            j += 1;
        }
        let name = &lower[i..j];
        let header = HEADER_NAMES.contains(&name);
        if !(header || KEY_NAMES.contains(&name)) {
            i = j;
            continue;
        }
        let mut k = j;
        if k < bytes.len() && bytes[k] == b'"' {
            k += 1;
        }
        while k < bytes.len() && bytes[k] == b' ' {
            k += 1;
        }
        if k >= bytes.len() || !matches!(bytes[k], b':' | b'=') {
            i = j;
            continue;
        }
        k += 1;
        while k < bytes.len() && bytes[k] == b' ' {
            k += 1;
        }
        let quote = (k < bytes.len() && matches!(bytes[k], b'"' | b'\'')).then(|| bytes[k]);
        if quote.is_some() {
            k += 1;
        }
        let mut end = k;
        while end < bytes.len() {
            let c = bytes[end];
            let stop = match quote {
                Some(q) => c == q,
                None if header => c == b'\n' || c == b'\r',
                None => c.is_ascii_whitespace() || matches!(c, b'&' | b',' | b';' | b'}' | b'"' | b'\''),
            };
            if stop {
                break;
            }
            end += 1;
        }
        out.push_str(&text[copied..k]);
        out.push_str(MASK);
        copied = end;
        i = end;
    }
    out.push_str(&text[copied..]);
    out
}

/// Removes credentials, cookies, header secrets and URL userinfo/queries.
pub fn redact(text: &str) -> String {
    redact_names(&redact_urls(text))
}

/// A URL with userinfo, query and fragment removed.
pub fn redact_url(url: &str) -> String {
    redact_urls(url)
}

// ------------------------------------------------------------------ logging

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug)]
pub enum Level {
    Off,
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl Level {
    pub fn parse(text: &str) -> Option<Level> {
        Some(match text.trim().to_ascii_lowercase().as_str() {
            "off" | "none" => Level::Off,
            "error" => Level::Error,
            "warn" | "warning" => Level::Warn,
            "info" => Level::Info,
            "debug" => Level::Debug,
            "trace" => Level::Trace,
            _ => return None,
        })
    }
}

pub const LOG_MAX_BYTES: u64 = 5 * 1024 * 1024;
pub const LOG_FILES: usize = 3;

pub struct Logger {
    path: PathBuf,
    level: Level,
    max_bytes: u64,
    file: Mutex<Option<(File, u64)>>,
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

impl Logger {
    pub fn new(path: PathBuf, level: Level, max_bytes: u64) -> Logger {
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        Logger { path, level, max_bytes, file: Mutex::new(None) }
    }

    fn rotated(&self, n: usize) -> PathBuf {
        let mut name = self.path.clone().into_os_string();
        name.push(format!(".{n}"));
        PathBuf::from(name)
    }

    fn open(&self) -> io::Result<(File, u64)> {
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&self.path)?;
        let size = file.metadata()?.len();
        Ok((file, size))
    }

    pub fn log(&self, level: Level, message: &str) {
        if level == Level::Off || level > self.level {
            return;
        }
        let line = format!(
            "{} {:5} {}\n",
            unix_now(),
            format!("{level:?}").to_ascii_uppercase(),
            redact(message).replace('\n', "\n    ")
        );
        let Ok(mut guard) = self.file.lock() else { return };
        if guard.as_ref().is_some_and(|(_, size)| *size + line.len() as u64 > self.max_bytes) {
            *guard = None;
            for n in (1..LOG_FILES - 1).rev() {
                let _ = fs::rename(self.rotated(n), self.rotated(n + 1));
            }
            let _ = fs::rename(&self.path, self.rotated(1));
        }
        if guard.is_none() {
            match self.open() {
                Ok(opened) if opened.1 + line.len() as u64 > self.max_bytes && opened.1 > 0 => {
                    drop(opened);
                    for n in (1..LOG_FILES - 1).rev() {
                        let _ = fs::rename(self.rotated(n), self.rotated(n + 1));
                    }
                    let _ = fs::rename(&self.path, self.rotated(1));
                    *guard = self.open().ok();
                }
                Ok(opened) => *guard = Some(opened),
                Err(_) => return,
            }
        }
        if let Some((file, size)) = guard.as_mut() {
            if file.write_all(line.as_bytes()).is_ok() {
                *size += line.len() as u64;
            }
        }
    }
}

/// Starts the process-wide logger (`<dir>/logs/<name>.log`); level comes from
/// `XDM_LOG` (default `info`).
pub fn init_logging(dir: &Path, name: &str) {
    let level = std::env::var("XDM_LOG").ok().and_then(|v| Level::parse(&v)).unwrap_or(Level::Info);
    let _ = LOGGER.set(Logger::new(dir.join("logs").join(format!("{name}.log")), level, LOG_MAX_BYTES));
}

pub fn log(level: Level, message: &str) {
    if let Some(logger) = LOGGER.get() {
        logger.log(level, message);
    }
}

pub fn log_error(message: &str) {
    log(Level::Error, message);
}
pub fn log_warn(message: &str) {
    log(Level::Warn, message);
}
pub fn log_info(message: &str) {
    log(Level::Info, message);
}
pub fn log_debug(message: &str) {
    log(Level::Debug, message);
}

// ------------------------------------------------------------ crash reports

pub const CRASH_KEEP: usize = 10;

fn crash_report(binary: &str, thread: &str, info: &std::panic::PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    let message = payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic payload".into());
    let location = info.location().map_or_else(String::new, |l| format!("{}:{}", l.file(), l.line()));
    let backtrace = std::backtrace::Backtrace::force_capture();
    redact(&format!(
        "{}\nthread: {thread}\nmessage: {message}\nlocation: {location}\ntime: {}\nos: {} {}\n\nbacktrace:\n{backtrace}\n",
        version_line(binary),
        unix_now(),
        std::env::consts::OS,
        std::env::consts::ARCH,
    ))
}

/// Writes `report` into `<dir>/crash/` and prunes to the newest `CRASH_KEEP`.
pub fn write_crash_report(dir: &Path, report: &str) -> io::Result<PathBuf> {
    let crash = dir.join("crash");
    fs::create_dir_all(&crash)?;
    let mut n = 0;
    let path = loop {
        let candidate = crash.join(format!("crash-{:012}-{}-{n}.txt", unix_now(), std::process::id()));
        if !candidate.exists() {
            break candidate;
        }
        n += 1;
    };
    fs::write(&path, report)?;
    let mut files = list_crash_reports(dir);
    files.sort();
    while files.len() > CRASH_KEEP {
        let _ = fs::remove_file(files.remove(0));
    }
    Ok(path)
}

pub fn list_crash_reports(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir.join("crash"))
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("crash-")))
                .collect()
        })
        .unwrap_or_default()
}

/// Installs a panic hook that writes a redacted crash report to `<dir>/crash/`.
pub fn install_panic_hook(dir: &Path, binary: &'static str) {
    let dir = dir.to_path_buf();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current().name().unwrap_or("unnamed").to_owned();
        let report = crash_report(binary, &thread, info);
        match write_crash_report(&dir, &report) {
            Ok(path) => log_error(&format!("panic in thread {thread}; crash report {}", path.display())),
            Err(e) => log_error(&format!("panic in thread {thread}; crash report failed: {e}")),
        }
        previous(info);
    }));
}

// -------------------------------------------------------------- handshake

/// Checks a `Hello` reply; the error text is user-facing.
pub fn check_hello(reply: &crate::Reply) -> Result<(), String> {
    if !reply.ok {
        return Err(format!(
            "daemon does not understand the version handshake ({}); it is probably an older xdm-daemon, restart it after upgrading",
            reply.error.as_deref().unwrap_or("no details")
        ));
    }
    match reply.protocol {
        Some(p) if p == PROTOCOL => Ok(()),
        Some(p) => Err(format!(
            "incompatible daemon: it speaks IPC protocol {p} (version {}), this client speaks {PROTOCOL} (version {VERSION}); upgrade or restart so both match",
            reply.daemon_version.as_deref().unwrap_or("unknown")
        )),
        None => Err("daemon returned no protocol number; restart it after upgrading".into()),
    }
}

impl Paths {
    /// Sends `Hello` and refuses an incompatible daemon with a clear message.
    pub fn handshake(&self) -> io::Result<crate::Reply> {
        let reply = self.request(Command::Hello { client_version: VERSION.into(), protocol: PROTOCOL })?;
        check_hello(&reply).map_err(io::Error::other)?;
        Ok(reply)
    }
}

// ------------------------------------------------------------------ zip

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

/// Minimal stored (uncompressed) zip archive writer.
pub struct ZipWriter<W: Write> {
    out: W,
    offset: u64,
    entries: Vec<(String, u32, u64, u64)>,
}

impl<W: Write> ZipWriter<W> {
    pub fn new(out: W) -> Self {
        Self { out, offset: 0, entries: Vec::new() }
    }

    pub fn add(&mut self, name: &str, data: &[u8]) -> io::Result<()> {
        let crc = crc32(data);
        let mut header = Vec::new();
        header.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        header.extend_from_slice(&[20, 0, 0, 8, 0, 0, 0, 0, 0x21, 0]); // version, flags(utf8), stored, time, date
        header.extend_from_slice(&crc.to_le_bytes());
        header.extend_from_slice(&(data.len() as u32).to_le_bytes());
        header.extend_from_slice(&(data.len() as u32).to_le_bytes());
        header.extend_from_slice(&(name.len() as u16).to_le_bytes());
        header.extend_from_slice(&0u16.to_le_bytes());
        header.extend_from_slice(name.as_bytes());
        self.out.write_all(&header)?;
        self.out.write_all(data)?;
        self.entries.push((name.to_owned(), crc, data.len() as u64, self.offset));
        self.offset += (header.len() + data.len()) as u64;
        Ok(())
    }

    pub fn finish(mut self) -> io::Result<()> {
        let start = self.offset;
        let mut size = 0u64;
        for (name, crc, len, offset) in &self.entries {
            let mut c = Vec::new();
            c.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            c.extend_from_slice(&[20, 0, 20, 0, 0, 8, 0, 0, 0, 0, 0x21, 0]);
            c.extend_from_slice(&crc.to_le_bytes());
            c.extend_from_slice(&(*len as u32).to_le_bytes());
            c.extend_from_slice(&(*len as u32).to_le_bytes());
            c.extend_from_slice(&(name.len() as u16).to_le_bytes());
            c.extend_from_slice(&[0; 12]); // extra, comment, disk, attrs
            c.extend_from_slice(&(*offset as u32).to_le_bytes());
            c.extend_from_slice(name.as_bytes());
            self.out.write_all(&c)?;
            size += c.len() as u64;
        }
        let mut end = Vec::new();
        end.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        end.extend_from_slice(&[0, 0, 0, 0]);
        end.extend_from_slice(&(self.entries.len() as u16).to_le_bytes());
        end.extend_from_slice(&(self.entries.len() as u16).to_le_bytes());
        end.extend_from_slice(&(size as u32).to_le_bytes());
        end.extend_from_slice(&(start as u32).to_le_bytes());
        end.extend_from_slice(&0u16.to_le_bytes());
        self.out.write_all(&end)?;
        self.out.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_secrets_in_every_shape() {
        let input = "GET https://bob:hunter2@example.com/a/b?token=abc123&x=1#frag failed\n\
            Authorization: Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ==\nCookie: sid=SECRETCOOKIE; a=b\n\
            {\"password\":\"p@ss w0rd\",\"url\":\"http://h/p\"} password=topsecret&k=v body=PAYLOAD";
        let out = redact(input);
        for secret in ["hunter2", "abc123", "frag", "QWxhZGRpbjpvcGVuIHNlc2FtZQ", "SECRETCOOKIE", "p@ss w0rd", "topsecret", "PAYLOAD"] {
            assert!(!out.contains(secret), "{secret} leaked in {out}");
        }
        assert!(out.contains("example.com/a/b") && out.contains("http://h/p"), "{out}");
    }

    #[test]
    fn logger_redacts_and_rotates() {
        let dir = std::env::temp_dir().join(format!("xdm-log-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("logs/t.log");
        let logger = Logger::new(path.clone(), Level::Info, 400);
        logger.log(Level::Info, "fetch http://u:pw123@h/f?sig=SIGVAL Cookie: SESS=zzz");
        logger.log(Level::Debug, "hidden below level");
        for i in 0..60 {
            logger.log(Level::Warn, &format!("line {i} Authorization: Bearer TOK{i}"));
        }
        let mut all = String::new();
        let mut files = 0;
        for p in [path.clone(), logger.rotated(1), logger.rotated(2), logger.rotated(3)] {
            if let Ok(text) = fs::read_to_string(&p) {
                files += 1;
                assert!(text.len() <= 400 + 80, "{} too big", p.display());
                all.push_str(&text);
            }
        }
        assert_eq!(files, LOG_FILES, "exactly {LOG_FILES} files kept");
        for secret in ["pw123", "SIGVAL", "SESS=zzz", "TOK5", "hidden below"] {
            assert!(!all.contains(secret), "{secret} leaked");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn crash_reports_keep_ten_newest() {
        let dir = std::env::temp_dir().join(format!("xdm-crash-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for i in 0..14 {
            write_crash_report(&dir, &format!("report {i}")).unwrap();
        }
        assert_eq!(list_crash_reports(&dir).len(), CRASH_KEEP);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn hello_check_rejects_other_protocols() {
        let mut reply = crate::Reply::list(Vec::new());
        reply.protocol = Some(PROTOCOL + 1);
        assert!(check_hello(&reply).unwrap_err().contains("incompatible"));
        reply.protocol = Some(PROTOCOL);
        assert!(check_hello(&reply).is_ok());
    }

    #[test]
    fn zip_has_valid_structure() {
        let mut buf = Vec::new();
        let mut zip = ZipWriter::new(&mut buf);
        zip.add("a.txt", b"hello").unwrap();
        zip.finish().unwrap();
        assert_eq!(&buf[..4], b"PK\x03\x04");
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(&buf[buf.len() - 22..buf.len() - 18], b"PK\x05\x06");
    }

    #[cfg(unix)]
    #[test]
    fn data_dir_is_private_and_token_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("xdm-perm-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        // SAFETY: this is the only test in the crate touching XDM_DATA_DIR.
        unsafe { std::env::set_var("XDM_DATA_DIR", &dir) };
        let paths = Paths::open().unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&paths.dir), 0o700);
        assert_eq!(mode(&paths.dir.join("ipc-token")), 0o600);
        let _ = fs::remove_dir_all(&dir);
    }
}
