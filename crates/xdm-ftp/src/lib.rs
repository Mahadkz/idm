//! Independent FTP download client. No browser or GUI dependencies.
//!
//! Implements the control-channel subset needed for a resumable single-stream
//! download: USER/PASS, TYPE I, SIZE, MDTM, EPSV with PASV fallback, REST and
//! RETR. Passive data connections are always made to the control peer so a
//! malicious server cannot redirect the data stream to a third host.

use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use sha2::{Digest, Sha256};
use std::time::Duration;

pub type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const CONTROL_TIMEOUT: Duration = Duration::from_secs(30);
const DATA_TIMEOUT: Duration = Duration::from_secs(30);
/// Largest accepted control reply line.
const MAX_LINE: usize = 8 * 1024;
/// Bound on `NNN-` continuation lines so a hostile server cannot stream forever.
const MAX_REPLY_LINES: usize = 128;
/// Flush interval for the partial file, so a crash loses at most this much.
const SYNC_INTERVAL: u64 = 1024 * 1024;
/// Default port for an FTP URL without an explicit port.
const FTP_PORT: u16 = 21;

#[derive(Debug, Clone)]
pub struct FtpOptions {
    pub url: String,
    pub output: PathBuf,
    pub restart: bool,
    pub stop: Arc<AtomicBool>,
    /// Separate directory for the partial file and checkpoint (N38).
    pub temp_dir: Option<PathBuf>,
    /// Stamp the finished file with the server's MDTM time.
    pub set_file_time: bool,
}

impl Default for FtpOptions {
    fn default() -> Self {
        Self {
            url: String::new(),
            output: PathBuf::new(),
            restart: false,
            stop: Arc::new(AtomicBool::new(false)),
            temp_dir: None,
            set_file_time: false,
        }
    }
}

/// Identity recorded next to a partial file so a later run can prove it is
/// still resuming the same object.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Checkpoint {
    size: Option<u64>,
    mdtm: Option<String>,
}

/// A parsed `ftp://[user[:password]@]host[:port]/path`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FtpUrl {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    /// Percent-decoded path sent verbatim in SIZE/MDTM/RETR commands.
    pub path: String,
}

impl FtpUrl {
    pub fn parse(url: &str) -> Result<Self> {
        let parsed = url::Url::parse(url).map_err(|e| err(format!("invalid FTP URL: {e}")))?;
        if parsed.scheme() != "ftp" {
            return Err(err(format!("not an FTP URL: {}", parsed.scheme())));
        }
        let host = parsed
            .host_str()
            .ok_or_else(|| err("FTP URL has no host"))?
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_string();
        if host.is_empty() {
            return Err(err("FTP URL has no host"));
        }
        let port = parsed.port().unwrap_or(FTP_PORT);
        let anonymous = parsed.username().is_empty();
        let user = if anonymous {
            "anonymous".to_string()
        } else {
            percent_decode(parsed.username())?
        };
        let password = match parsed.password() {
            Some(raw) => percent_decode(raw)?,
            None if anonymous => "anonymous@".to_string(),
            None => String::new(),
        };
        let path = percent_decode(parsed.path())?;
        if path.is_empty() {
            return Err(err("FTP URL has an empty path"));
        }
        for value in [&user, &password, &path] {
            reject_control(value)?;
        }
        Ok(Self {
            host,
            port,
            user,
            password,
            path,
        })
    }
}

fn percent_decode(input: &str) -> Result<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = bytes
                .get(index + 1)
                .and_then(|b| hex(*b))
                .ok_or_else(|| err("invalid percent-encoding in FTP URL"))?;
            let low = bytes
                .get(index + 2)
                .and_then(|b| hex(*b))
                .ok_or_else(|| err("invalid percent-encoding in FTP URL"))?;
            out.push(high << 4 | low);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).map_err(|_| err("percent-decoded FTP text is not valid UTF-8"))
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Reject values that would inject extra FTP commands or truncate a reply.
fn reject_control(value: &str) -> Result<()> {
    if value.bytes().any(|b| b == 0 || b == b'\r' || b == b'\n') {
        return Err(err("FTP URL contains a control character"));
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct Reply {
    code: u16,
    text: String,
}

impl Reply {
    /// Text following the three-digit code on the first line.
    fn arg(&self) -> &str {
        self.text.get(4..).map(str::trim).unwrap_or("")
    }
}

struct Control {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    /// Address of the control connection, reused for every data connection.
    peer: IpAddr,
}

impl Control {
    fn connect(host: &str, port: u16) -> Result<Self> {
        let stream = connect(host, port, CONTROL_TIMEOUT)?;
        stream.set_read_timeout(Some(CONTROL_TIMEOUT))?;
        stream.set_write_timeout(Some(CONTROL_TIMEOUT))?;
        let peer = stream.peer_addr()?.ip();
        Ok(Self {
            writer: stream.try_clone()?,
            reader: BufReader::new(stream),
            peer,
        })
    }

    fn read_reply(&mut self) -> Result<Reply> {
        read_reply(&mut self.reader)
    }

    fn command(&mut self, line: &str) -> Result<Reply> {
        self.writer.write_all(line.as_bytes())?;
        self.writer.write_all(b"\r\n")?;
        self.writer.flush()?;
        self.read_reply()
    }

    /// Open the data connection using EPSV, falling back to PASV. The
    /// advertised PASV address is deliberately ignored: only the control peer
    /// is contacted, which defeats FTP bounce to an arbitrary third party.
    fn passive(&mut self) -> Result<TcpStream> {
        let reply = self.command("EPSV")?;
        if reply.code == 229 {
            let port = parse_epsv(&reply.text)
                .ok_or_else(|| err("EPSV reply did not contain a data port"))?;
            return data_connect(self.peer, port);
        }
        let reply = self.command("PASV")?;
        if reply.code != 227 {
            return Err(err(format!(
                "FTP server refused passive mode: {}",
                reply.text
            )));
        }
        let (_advertised, port) = parse_pasv(&reply.text)
            .ok_or_else(|| err("malformed PASV reply from FTP server"))?;
        data_connect(self.peer, port)
    }
}

fn connect(host: &str, port: u16, timeout: Duration) -> Result<TcpStream> {
    let addresses: Vec<SocketAddr> = (host, port)
        .to_socket_addrs()
        .map_err(|e| err(format!("cannot resolve {host}: {e}")))?
        .collect();
    let mut last = String::from("no address resolved");
    for address in addresses {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(stream) => return Ok(stream),
            Err(e) => last = e.to_string(),
        }
    }
    Err(err(format!("cannot connect to {host}:{port}: {last}")))
}

fn data_connect(peer: IpAddr, port: u16) -> Result<TcpStream> {
    let stream = connect(&peer.to_string(), port, CONNECT_TIMEOUT)?;
    stream.set_read_timeout(Some(DATA_TIMEOUT))?;
    Ok(stream)
}

fn read_reply<R: BufRead>(reader: &mut R) -> Result<Reply> {
    let first = read_reply_line(reader)?;
    let code = parse_code(&first)?;
    let mut text = first.clone();
    if first.as_bytes().get(3) == Some(&b'-') {
        for _ in 0..MAX_REPLY_LINES {
            let line = read_reply_line(reader)?;
            text.push('\n');
            text.push_str(&line);
            let done = line.len() >= 4
                && line.as_bytes()[3] == b' '
                && parse_code(&line).is_ok_and(|c| c == code);
            if done {
                return Ok(Reply { code, text });
            }
        }
        return Err(err("FTP reply has too many continuation lines"));
    }
    Ok(Reply { code, text })
}

fn read_reply_line<R: BufRead>(reader: &mut R) -> Result<String> {
    let mut buffer = Vec::new();
    let read = reader
        .by_ref()
        .take(MAX_LINE as u64 + 1)
        .read_until(b'\n', &mut buffer)?;
    if read == 0 {
        return Err(err("FTP server closed the control connection"));
    }
    if buffer.len() > MAX_LINE || !buffer.ends_with(b"\n") {
        return Err(err("FTP reply line is too long or unterminated"));
    }
    while buffer.last().is_some_and(|b| *b == b'\n' || *b == b'\r') {
        buffer.pop();
    }
    Ok(String::from_utf8_lossy(&buffer).into_owned())
}

fn parse_code(line: &str) -> Result<u16> {
    let digits = line
        .get(..3)
        .filter(|d| d.bytes().all(|b| b.is_ascii_digit()))
        .ok_or_else(|| err(format!("malformed FTP reply: {line:?}")))?;
    digits
        .parse()
        .map_err(|_| err(format!("malformed FTP reply: {line:?}")))
}

/// Port from a `229 Entering Extended Passive Mode (|||port|)` reply.
fn parse_epsv(text: &str) -> Option<u16> {
    let start = text.find("|||")? + 3;
    let rest = &text[start..];
    let end = rest.find('|')?;
    let port = rest[..end].parse::<u16>().ok()?;
    (port != 0).then_some(port)
}

/// Address and port from a `227 Entering Passive Mode (h1,h2,h3,h4,p1,p2)` reply.
fn parse_pasv(text: &str) -> Option<(IpAddr, u16)> {
    let open = text.find('(')?;
    let close = text[open..].find(')')? + open;
    let numbers: Option<Vec<u16>> = text[open + 1..close]
        .split(',')
        .map(|part| part.trim().parse::<u16>().ok())
        .collect();
    let numbers = numbers?;
    if numbers.len() != 6 || numbers.iter().any(|n| *n > 255) {
        return None;
    }
    let ip = IpAddr::from([
        numbers[0] as u8,
        numbers[1] as u8,
        numbers[2] as u8,
        numbers[3] as u8,
    ]);
    let port = numbers[4] * 256 + numbers[5];
    (port != 0).then_some((ip, port))
}

/// Canonical form of a destination for hashing; identical to the HTTP engine's so both
/// engines name partials in a temporary directory the same way.
fn normalized_output(output: &Path) -> PathBuf {
    let clean: PathBuf = output.components().collect();
    match (clean.parent(), clean.file_name()) {
        (Some(parent), Some(name)) => fs::canonicalize(parent)
            .map(|parent| parent.join(name))
            .unwrap_or(clean),
        _ => clean,
    }
}

fn temp_base(output: &Path, temp: &Path) -> PathBuf {
    let digest = format!(
        "{:x}",
        Sha256::digest(normalized_output(output).to_string_lossy().as_bytes())
    );
    let name: String = output
        .file_name()
        .map(|n| n.to_string_lossy().chars().take(100).collect())
        .unwrap_or_default();
    temp.join(format!("{}-{name}", &digest[..16]))
}

/// The `.part` and `.part.ftp` paths of `output`, beside it or inside `temp`.
pub fn partial_paths(output: &Path, temp: Option<&Path>) -> (PathBuf, PathBuf) {
    let base = temp.map_or_else(|| output.to_path_buf(), |dir| temp_base(output, dir));
    (
        PathBuf::from(format!("{}.part", base.display())),
        PathBuf::from(format!("{}.part.ftp", base.display())),
    )
}

/// Parses an MDTM value (`YYYYMMDDHHMMSS[.fff]`, UTC) into a system time.
pub fn parse_mdtm(value: &str) -> Option<std::time::SystemTime> {
    let (whole, fraction) = match value.trim().split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (value.trim(), None),
    };
    if whole.len() != 14
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || fraction.is_some_and(|f| f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let n = |range: std::ops::Range<usize>| whole[range].parse::<i64>().ok();
    let (y, m, d, h, mi, sec) = (n(0..4)?, n(4..6)?, n(6..8)?, n(8..10)?, n(10..12)?, n(12..14)?);
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let month_days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if !(1970..=9999).contains(&y)
        || !(1..=12).contains(&m)
        || d < 1
        || d > month_days[(m - 1) as usize]
        || h > 23
        || mi > 59
        || sec > 59
    {
        return None;
    }
    // Days from civil (Howard Hinnant).
    let yy = if m <= 2 { y - 1 } else { y };
    let era = yy / 400;
    let yoe = yy - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let seconds = u64::try_from(days * 86400 + h * 3600 + mi * 60 + sec).ok()?;
    Some(std::time::UNIX_EPOCH + Duration::from_secs(seconds))
}

fn store_checkpoint(path: &Path, state: &Checkpoint) -> Result<()> {
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

fn load_checkpoint(path: &Path) -> Result<Checkpoint> {
    let file = File::open(path)?;
    serde_json::from_reader(file)
        .map_err(|e| err(format!("invalid FTP checkpoint {}: {e}", path.display())))
}

/// Flushes the directory entry of a freshly installed file (Unix).
fn sync_parent(path: &Path) {
    #[cfg(unix)]
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        if let Ok(dir) = File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Removal of the source after a successful install is housekeeping, never a failure.
fn discard(path: &Path) {
    if let Err(error) = fs::remove_file(path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            eprintln!("warning: could not remove {}: {error}", path.display());
        }
    }
}

/// Linux `renameat2(RENAME_NOREPLACE)`; other systems refuse rather than risk replacing.
fn rename_noreplace(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let (from, to) = (
            CString::new(from.as_os_str().as_bytes())?,
            CString::new(to.as_os_str().as_bytes())?,
        );
        // SAFETY: both pointers are valid NUL-terminated strings for the call's duration.
        let result = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                libc::AT_FDCWD,
                from.as_ptr(),
                libc::AT_FDCWD,
                to.as_ptr(),
                1u32, // RENAME_NOREPLACE
            )
        };
        if result == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (from, to);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "no-replace rename is unavailable on this system",
        ))
    }
}

fn appeared(output: &Path, part: &Path) -> Box<dyn Error + Send + Sync> {
    err(format!(
        "{} appeared during the download; the completed data was kept at {}",
        output.display(),
        part.display()
    ))
}

/// Installs the completed file without replacing anything created at the destination
/// meanwhile; falls back to a staged, fsynced cross-volume copy (N38).
fn publish(part: &Path, output: &Path) -> Result<()> {
    match fs::hard_link(part, output) {
        Ok(()) => {
            sync_parent(output);
            discard(part);
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(appeared(output, part)),
        Err(_) => match rename_noreplace(part, output) {
            Ok(()) => {
                sync_parent(output);
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(appeared(output, part)),
            Err(_) => copy_publish(part, output),
        },
    }
}

/// Copies beside the destination under a unique staging name, syncs, installs without
/// replacing, syncs the directory, and only then removes the partial. Any failure leaves
/// the partial and the destination untouched.
fn copy_publish(part: &Path, output: &Path) -> Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let staging = PathBuf::from(format!(
        "{}.xdm-copy-{}-{nanos}-{}",
        output.display(),
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let copied = (|| -> std::io::Result<()> {
        let mut source = File::open(part)?;
        let modified = source.metadata()?.modified().ok();
        let mut target = OpenOptions::new().write(true).create_new(true).open(&staging)?;
        std::io::copy(&mut source, &mut target)?;
        if let Some(modified) = modified {
            let _ = target.set_modified(modified);
        }
        target.sync_all()
    })();
    if let Err(error) = copied {
        if error.kind() != std::io::ErrorKind::AlreadyExists {
            let _ = fs::remove_file(&staging);
        }
        return Err(err(format!(
            "copying the finished download to {} failed ({error}); the data was kept at {}",
            output.display(),
            part.display()
        )));
    }
    let installed = match fs::hard_link(&staging, output) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(()),
        Err(_) => rename_noreplace(&staging, output).map_err(|_| ()),
    };
    if installed.is_err() {
        let _ = fs::remove_file(&staging);
        return Err(appeared(output, part));
    }
    let _ = fs::remove_file(&staging);
    sync_parent(output);
    discard(part);
    Ok(())
}

/// Stamps the open partial with the server time when enabled and valid.
fn stamp(file: &File, enabled: bool, mdtm: Option<&str>) {
    if let Some(time) = enabled.then(|| mdtm.and_then(parse_mdtm)).flatten() {
        let _ = file.set_modified(time);
    }
}

fn probe_size(control: &mut Control, path: &str) -> Result<Option<u64>> {
    let reply = control.command(&format!("SIZE {path}"))?;
    if reply.code == 213 {
        return Ok(reply.arg().parse::<u64>().ok());
    }
    Ok(None)
}

fn probe_mdtm(control: &mut Control, path: &str) -> Result<Option<String>> {
    let reply = control.command(&format!("MDTM {path}"))?;
    if reply.code == 213 {
        let value = reply.arg().trim().to_string();
        if !value.is_empty() {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

fn login(control: &mut Control, url: &FtpUrl) -> Result<()> {
    let reply = control.command(&format!("USER {}", url.user))?;
    match reply.code {
        230 => Ok(()),
        331 => {
            let reply = control.command(&format!("PASS {}", url.password))?;
            if reply.code == 230 {
                Ok(())
            } else {
                Err(err(format!("FTP login failed: {}", reply.text)))
            }
        }
        _ => Err(err(format!("FTP login failed: {}", reply.text))),
    }
}

fn quit(control: &mut Control) {
    let _ = control.command("QUIT");
}

/// Download `options.url` into `<output>.part`, then publish it at `output`.
///
/// `progress(downloaded, total)` is called after every received chunk. A stop
/// flag set during the transfer returns an error whose message contains
/// `paused`, leaving `.part` and `.part.ftp` in place for a later resume.
pub fn download(options: FtpOptions, progress: impl Fn(u64, Option<u64>)) -> Result<()> {
    if options.stop.load(Ordering::Relaxed) {
        return Err(err("paused"));
    }
    let url = FtpUrl::parse(&options.url)?;
    if fs::symlink_metadata(&options.output).is_ok() {
        return Err(err(format!(
            "{} already exists; choose another path",
            options.output.display()
        )));
    }
    if let Some(parent) = options.output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let temp = options.temp_dir.as_deref();
    if let Some(temp) = temp {
        fs::create_dir_all(temp)?;
    }
    let (part, checkpoint) = partial_paths(&options.output, temp);

    let mut control = Control::connect(&url.host, url.port)?;
    let greeting = control.read_reply()?;
    if greeting.code != 220 {
        return Err(err(format!(
            "FTP server did not greet correctly: {}",
            greeting.text
        )));
    }
    login(&mut control, &url)?;
    let reply = control.command("TYPE I")?;
    if reply.code != 200 {
        return Err(err(format!("FTP TYPE I rejected: {}", reply.text)));
    }

    let size = probe_size(&mut control, &url.path)?;
    let mdtm = probe_mdtm(&mut control, &url.path)?;

    if options.restart {
        let _ = fs::remove_file(&part);
        let _ = fs::remove_file(&checkpoint);
    }

    // Decide whether an existing partial file may be resumed.
    let mut offset = 0u64;
    if !options.restart && (part.exists() || checkpoint.exists()) {
        let saved = checkpoint
            .exists()
            .then(|| load_checkpoint(&checkpoint))
            .transpose()?;
        let part_len = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        let matches = saved.as_ref().is_some_and(|saved| {
            saved.size.is_some() && saved.size == size && saved.mdtm == mdtm
        });
        if !matches {
            return Err(err(format!(
                "partial download does not match the remote object; restart the download of {}",
                options.output.display()
            )));
        }
        if size.is_some_and(|total| part_len > total) {
            return Err(err(format!(
                "partial file is longer than the remote object; restart the download of {}",
                options.output.display()
            )));
        }
        offset = part_len;
    }

    // A previous run may have written every byte but failed to publish.
    if let Some(total) = size {
        if total > 0 && offset == total && part.exists() {
            if options.set_file_time {
                if let Ok(file) = OpenOptions::new().write(true).open(&part) {
                    stamp(&file, true, mdtm.as_deref());
                    let _ = file.sync_all();
                }
            }
            publish(&part, &options.output)?;
            let _ = fs::remove_file(&checkpoint);
            progress(total, Some(total));
            quit(&mut control);
            return Ok(());
        }
    }

    let mut file = OpenOptions::new().create(true).write(true).open(&part)?;
    file.set_len(offset)?;
    file.seek(SeekFrom::Start(offset))?;
    store_checkpoint(
        &checkpoint,
        &Checkpoint {
            size,
            mdtm: mdtm.clone(),
        },
    )?;

    if offset > 0 {
        let reply = control.command(&format!("REST {offset}"))?;
        if reply.code != 350 {
            file.sync_all()?;
            return Err(err(format!(
                "FTP server rejected REST at {offset}; restart the download of {}",
                options.output.display()
            )));
        }
    }

    let mut data = control.passive()?;
    let reply = control.command(&format!("RETR {}", url.path))?;
    if reply.code != 150 && reply.code != 125 {
        file.sync_all()?;
        return Err(err(format!("FTP RETR failed: {}", reply.text)));
    }

    let mut received = offset;
    let mut synced = offset;
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        if options.stop.load(Ordering::Relaxed) {
            file.sync_all()?;
            return Err(err("paused"));
        }
        match data.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                file.write_all(&buffer[..n])?;
                received += n as u64;
                if received - synced >= SYNC_INTERVAL {
                    file.sync_data()?;
                    synced = received;
                }
                progress(received, size);
            }
            Err(e) if is_timeout(&e) => {
                file.sync_all()?;
                return Err(err(format!(
                    "FTP data connection timed out after {received} bytes"
                )));
            }
            Err(e) => {
                file.sync_all()?;
                drop(data);
                return Err(err(format!(
                    "FTP data connection failed after {received} bytes: {e}"
                )));
            }
        }
    }
    file.sync_all()?;
    drop(data);

    let reply = control.read_reply()?;
    if reply.code != 226 && reply.code != 250 {
        return Err(err(format!(
            "FTP transfer did not complete: {}",
            reply.text
        )));
    }
    if let Some(total) = size {
        if received != total {
            return Err(err(format!(
                "FTP transfer length mismatch: received {received}, expected {total}"
            )));
        }
    }

    stamp(&file, options.set_file_time, mdtm.as_deref());
    file.sync_all()?;
    drop(file);
    publish(&part, &options.output)?;
    let _ = fs::remove_file(&checkpoint);
    progress(received, size);
    quit(&mut control);
    Ok(())
}

fn is_timeout(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    )
}

fn err(message: impl Into<String>) -> Box<dyn Error + Send + Sync> {
    message.into().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mdtm_values_parse_strictly() {
        let secs = |v: &str| {
            parse_mdtm(v).map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs())
        };
        assert_eq!(secs("19700101000000"), Some(0));
        assert_eq!(secs("20240229235959.5"), Some(1_709_251_199));
        for bad in ["", "2024", "20240230000000", "20241301000000", "20240101240000", "2024010100000x", "20240101000000.", "19691231235959"] {
            assert_eq!(secs(bad), None, "{bad}");
        }
    }

    #[test]
    fn parses_default_anonymous_login() {
        let url = FtpUrl::parse("ftp://ftp.example.com/pub/file.txt").unwrap();
        assert_eq!(url.host, "ftp.example.com");
        assert_eq!(url.port, 21);
        assert_eq!(url.user, "anonymous");
        assert_eq!(url.password, "anonymous@");
        assert_eq!(url.path, "/pub/file.txt");
    }

    #[test]
    fn parses_credentials_port_and_percent_decoding() {
        let url = FtpUrl::parse("ftp://alice:p%40ss@127.0.0.1:2121/dir/my%20file%25.txt").unwrap();
        assert_eq!(url.host, "127.0.0.1");
        assert_eq!(url.port, 2121);
        assert_eq!(url.user, "alice");
        assert_eq!(url.password, "p@ss");
        assert_eq!(url.path, "/dir/my file%.txt");
    }

    #[test]
    fn rejects_non_ftp_schemes_and_control_characters() {
        assert!(FtpUrl::parse("http://example.com/x").is_err());
        assert!(FtpUrl::parse("ftp://example.com/bad%0aname").is_err());
        assert!(FtpUrl::parse("ftp://").is_err());
        assert!(FtpUrl::parse("ftp://example.com/%zz").is_err());
    }

    #[test]
    fn parses_ipv6_host_and_empty_password() {
        let url = FtpUrl::parse("ftp://bob@[::1]:2121/").unwrap();
        assert_eq!(url.host, "::1");
        assert_eq!(url.port, 2121);
        assert_eq!(url.user, "bob");
        assert_eq!(url.password, "");
    }

    #[test]
    fn parses_epsv_and_pasv_replies() {
        assert_eq!(parse_epsv("Entering Extended Passive Mode (|||6446|)"), Some(6446));
        assert_eq!(parse_epsv("welcome (|||0|)"), None);
        assert_eq!(parse_epsv("no marker here"), None);
        assert_eq!(
            parse_pasv("Entering Passive Mode (127,0,0,1,195,80)"),
            Some((IpAddr::from([127, 0, 0, 1]), 195 * 256 + 80))
        );
        assert_eq!(parse_pasv("Entering Passive Mode (10,0,0,1,1,2)").unwrap().0, IpAddr::from([10, 0, 0, 1]));
        assert_eq!(parse_pasv("bad reply"), None);
        assert_eq!(parse_pasv("Entering Passive Mode (1,2,3,4,5)"), None);
        assert_eq!(parse_pasv("Entering Passive Mode (1,2,3,4,0,0)"), None);
    }

    #[test]
    fn reads_single_and_multiline_replies() {
        let mut single = std::io::Cursor::new(b"213 4096\r\n".to_vec());
        let reply = read_reply(&mut single).unwrap();
        assert_eq!(reply.code, 213);
        assert_eq!(reply.arg(), "4096");

        let mut multi =
            std::io::Cursor::new(b"220-Welcome\r\n220-Ready to serve\r\n220 go\r\n".to_vec());
        let reply = read_reply(&mut multi).unwrap();
        assert_eq!(reply.code, 220);
        assert!(reply.text.contains("Welcome"));
        assert!(reply.text.contains("go"));

        let mut malformed = std::io::Cursor::new(b"not a reply\r\n".to_vec());
        assert!(read_reply(&mut malformed).is_err());
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

        fn byte(&mut self) -> u8 {
            self.next_u64() as u8
        }

        fn bytes(&mut self, max_len: usize) -> Vec<u8> {
            let len = self.below(max_len as u64 + 1) as usize;
            (0..len).map(|_| self.byte()).collect()
        }

        fn string(&mut self, alphabet: &[u8], max_len: usize) -> String {
            let len = self.below(max_len as u64 + 1) as usize;
            let bytes: Vec<u8> = (0..len)
                .map(|_| alphabet[self.below(alphabet.len() as u64) as usize])
                .collect();
            String::from_utf8_lossy(&bytes).into_owned()
        }

        fn token(&mut self, alphabet: &[u8], max_len: usize) -> String {
            let len = 1 + self.below(max_len as u64) as usize;
            let bytes: Vec<u8> = (0..len)
                .map(|_| alphabet[self.below(alphabet.len() as u64) as usize])
                .collect();
            String::from_utf8_lossy(&bytes).into_owned()
        }
    }

    #[test]
    fn reply_and_reply_helpers_never_panic_on_arbitrary_bytes() {
        let mut rng = Rng::new(0xf7b0_0001);
        for _ in 0..20_000 {
            let bytes = rng.bytes(80);
            let _ = read_reply(&mut std::io::Cursor::new(bytes.clone()));
            let text = String::from_utf8_lossy(&bytes).into_owned();
            let _ = parse_code(&text);
            let _ = parse_epsv(&text);
            let _ = parse_pasv(&text);
            let _ = percent_decode(&text);
        }
    }

    #[test]
    fn reply_parser_bounds_continuation_lines() {
        let mut text = String::from("220-");
        for _ in 0..(MAX_REPLY_LINES + 10) {
            text.push_str("220-cont\r\n");
        }
        let mut reader = std::io::Cursor::new(text.into_bytes());
        assert!(read_reply(&mut reader).is_err());
    }

    #[test]
    fn single_line_replies_round_trip_codes_and_text() {
        let mut rng = Rng::new(0xf7b0_0002);
        for _ in 0..10_000 {
            let code = 100 + rng.below(500) as u16;
            let text = rng.string(b"abc XYZ019_-", 20);
            let line = format!("{code} {text}\r\n");
            let mut reader = std::io::Cursor::new(line.into_bytes());
            let reply = read_reply(&mut reader).unwrap();
            assert_eq!(reply.code, code);
            assert_eq!(reply.arg(), text.trim());
        }
    }

    #[test]
    fn epsv_and_pasv_round_trip_their_ports() {
        let mut rng = Rng::new(0xf7b0_0003);
        for _ in 0..20_000 {
            let port = 1 + rng.below(65535) as u16;
            let epsv = format!("229 Entering Extended Passive Mode (|||{port}|)");
            assert_eq!(parse_epsv(&epsv), Some(port));

            let ip = [rng.byte(), rng.byte(), rng.byte(), rng.byte()];
            let pasv = format!(
                "227 Entering Passive Mode ({},{},{},{},{},{})",
                ip[0],
                ip[1],
                ip[2],
                ip[3],
                port / 256,
                port % 256
            );
            assert_eq!(parse_pasv(&pasv), Some((IpAddr::from(ip), port)));
        }
        assert_eq!(parse_epsv("(|||0|)"), None);
        assert_eq!(parse_pasv("Entering Passive Mode (1,2,3,4,0,0)"), None);
    }

    #[test]
    fn percent_decode_round_trips_arbitrary_encoded_bytes() {
        let mut rng = Rng::new(0xf7b0_0004);
        for _ in 0..10_000 {
            let bytes = rng.bytes(16);
            let encoded: String = bytes.iter().map(|b| format!("%{b:02X}")).collect();
            match percent_decode(&encoded) {
                Ok(decoded) => assert_eq!(decoded.as_bytes(), bytes.as_slice()),
                Err(_) => assert!(std::str::from_utf8(&bytes).is_err()),
            }
        }
    }

    #[test]
    fn ftp_url_parser_never_panics_on_arbitrary_text() {
        let mut rng = Rng::new(0xf7b0_0005);
        let alphabet = b"ftp:/%@[]:-._~?#0123456789abcXYZ \t";
        for _ in 0..30_000 {
            let text = rng.string(alphabet, 40);
            let _ = FtpUrl::parse(&text);
        }
    }

    #[test]
    fn ftp_url_round_trips_safe_components() {
        let mut rng = Rng::new(0xf7b0_0006);
        let alphabet = b"abcXYZ019._-~";
        for _ in 0..5_000 {
            let port = 1 + rng.below(65535) as u16;
            let user = rng.token(alphabet, 8);
            let password = rng.string(alphabet, 8);
            let path = format!("/{}", rng.string(alphabet, 10));
            // The URL parser collapses the `.` and `..` path segments.
            if path == "/." || path == "/.." {
                continue;
            }
            let url = format!("ftp://{user}:{password}@127.0.0.1:{port}{path}");
            let parsed = FtpUrl::parse(&url).unwrap_or_else(|e| panic!("{url}: {e}"));
            assert_eq!(parsed.host, "127.0.0.1");
            assert_eq!(parsed.port, port);
            assert_eq!(parsed.user, user);
            assert_eq!(parsed.password, password);
            assert_eq!(parsed.path, path);
        }
    }
}
