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
use std::sync::atomic::{AtomicBool, Ordering};
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
}

impl Default for FtpOptions {
    fn default() -> Self {
        Self {
            url: String::new(),
            output: PathBuf::new(),
            restart: false,
            stop: Arc::new(AtomicBool::new(false)),
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

fn part_path(output: &Path) -> PathBuf {
    PathBuf::from(format!("{}.part", output.display()))
}

fn checkpoint_path(output: &Path) -> PathBuf {
    PathBuf::from(format!("{}.part.ftp", output.display()))
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

/// Installs the completed file without replacing anything created meanwhile.
fn publish(part: &Path, output: &Path) -> Result<()> {
    match fs::hard_link(part, output) {
        Ok(()) => Ok(fs::remove_file(part)?),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(err(format!(
            "{} appeared during the download; the completed data was kept at {}",
            output.display(),
            part.display()
        ))),
        // Filesystems without hard links fall back to a checked rename.
        Err(_) if fs::symlink_metadata(output).is_err() => Ok(fs::rename(part, output)?),
        Err(_) => Err(err(format!(
            "{} appeared during the download; the completed data was kept at {}",
            output.display(),
            part.display()
        ))),
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
    let part = part_path(&options.output);
    let checkpoint = checkpoint_path(&options.output);

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
            mdtm,
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
