//! Site login store for HTTP authentication (contracts N10/N11): URL
//! patterns with Basic credentials, matched most-specific-first.
//!
//! Passwords live in a dedicated `credentials.json` next to the task
//! database, created with owner-only permissions on Unix and sealed with
//! DPAPI on Windows. They are never returned by IPC, exports or logs; the
//! daemon only turns them into a per-origin `Authorization` header.

use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use url::Url;
use xdm_ipc::LoginSummary;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// One saved login. `pattern` is `scheme://[*.]host[:port][/path prefix]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Login {
    pub id: i64,
    pub pattern: String,
    pub username: String,
    pub password: String,
    pub enabled: bool,
}

/// A parsed login pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern {
    scheme: String,
    host: Host,
    port: Option<u16>,
    path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Host {
    Exact(String),
    /// Suffix such as `example.com` for a `*.example.com` pattern: it matches
    /// proper subdomains only, never the bare domain.
    Wildcard(String),
}

const MAX_PATTERN: usize = 512;

/// Parses and validates a login pattern.
pub fn parse_pattern(pattern: &str) -> Result<Pattern> {
    if pattern.is_empty() || pattern.len() > MAX_PATTERN {
        return Err("login pattern must be 1–512 bytes".into());
    }
    let (scheme, rest) = pattern
        .split_once("://")
        .ok_or("login pattern must look like scheme://host[:port][/path]")?;
    if !matches!(scheme, "http" | "https") {
        return Err("login patterns apply to http and https URLs".into());
    }
    let (authority, path) = match rest.find('/') {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, ""),
    };
    if authority.is_empty() {
        return Err("login pattern needs a host".into());
    }
    let (host, port) = if let Some(literal) = authority.strip_prefix('[') {
        // IPv6 literal, optionally followed by ":port".
        let Some((literal, tail)) = literal.split_once(']') else {
            return Err("login pattern IPv6 host must close with ]".into());
        };
        let port = match tail.strip_prefix(':') {
            Some(port) => {
                let parsed = port.parse::<u16>().ok().filter(|port| *port > 0);
                if parsed.is_none() {
                    return Err("login pattern port must be 1–65535".into());
                }
                parsed
            }
            None if tail.is_empty() => None,
            None => return Err("login pattern has trailing characters after the IPv6 host".into()),
        };
        (literal.to_owned(), port)
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => {
                let parsed = port.parse::<u16>().ok().filter(|port| *port > 0);
                if parsed.is_none() {
                    return Err("login pattern port must be 1–65535".into());
                }
                (host.to_owned(), parsed)
            }
            None => (authority.to_owned(), None),
        }
    };
    if path.len() > 1024 || path.bytes().any(|b| b < 0x21 || b == 0x7f) {
        return Err("login pattern path has 1024 printable bytes at most".into());
    }
    let valid_host = |host: &str| {
        !host.is_empty()
            && host.bytes().all(|b| {
                b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':')
            })
    };
    let host = if let Some(suffix) = host.strip_prefix("*.") {
        if !valid_host(suffix) {
            return Err("wildcard host must look like *.example.com".into());
        }
        Host::Wildcard(suffix.to_ascii_lowercase())
    } else {
        if !valid_host(&host) {
            return Err("host may only start with *. as a wildcard".into());
        }
        Host::Exact(host.to_ascii_lowercase())
    };
    Ok(Pattern {
        scheme: scheme.to_owned(),
        host,
        port,
        path: path.to_owned(),
    })
}

impl Pattern {
    /// Whether a request URL falls under this pattern.
    pub fn matches(&self, url: &Url) -> bool {
        if url.scheme() != self.scheme {
            return false;
        }
        let Some(host) = url.host_str() else {
            return false;
        };
        let host = host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_ascii_lowercase();
        let host_matches = match &self.host {
            Host::Exact(want) => *want == host,
            Host::Wildcard(suffix) => host.ends_with(&format!(".{suffix}")),
        };
        if !host_matches {
            return false;
        }
        if self.port.is_some_and(|want| url.port_or_known_default() != Some(want)) {
            return false;
        }
        url.path().starts_with(&self.path)
    }

    /// Sort key ranking patterns: exact hosts beat wildcards, deeper wildcard
    /// suffixes beat shallower ones, and only then longer path prefixes win.
    fn specificity(&self) -> (u8, usize, usize) {
        match &self.host {
            Host::Exact(_) => (1, 0, self.path.len()),
            Host::Wildcard(suffix) => (
                0,
                suffix.split('.').filter(|label| !label.is_empty()).count(),
                self.path.len(),
            ),
        }
    }
}

/// Standard base64 (RFC 4648, with padding) for Basic credentials.
pub fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let word = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(TABLE[(word >> 18) as usize & 63] as char);
        out.push(TABLE[(word >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(word >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[word as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// The value of a `Basic` Authorization header for these credentials.
pub fn basic_token(username: &str, password: &str) -> String {
    base64_encode(format!("{username}:{password}").as_bytes())
}

/// Fields a saved login carries, so callers can validate before storing.
pub fn valid_login_fields(username: &str, password: &str) -> bool {
    !username.is_empty()
        && username.len() <= 128
        && password.len() <= 1024
        && !username.chars().any(char::is_control)
        && !password.chars().any(char::is_control)
}

#[derive(Debug, Serialize, Deserialize)]
struct File {
    version: u32,
    logins: Vec<Login>,
}

/// In-memory login table with protected at-rest persistence.
#[derive(Debug)]
pub struct Store {
    /// `None` keeps the store in memory (test databases).
    path: Option<PathBuf>,
    logins: Vec<Login>,
    next_id: i64,
}

impl Store {
    /// Loads the store from `credentials.json` when present.
    pub fn load(path: Option<PathBuf>) -> Result<Self> {
        let mut store = Self {
            path,
            logins: Vec::new(),
            next_id: 1,
        };
        let Some(path) = &store.path else {
            return Ok(store);
        };
        let Ok(sealed) = fs::read(path) else {
            return Ok(store);
        };
        let plain = unseal(&sealed)?;
        let file: File = serde_json::from_slice(&plain).map_err(|e| -> Error {
            format!("credentials file is corrupt: {e}").into()
        })?;
        if file.version != 1 {
            return Err("unsupported credentials file version".into());
        }
        store.next_id = file
            .logins
            .iter()
            .map(|login| login.id + 1)
            .max()
            .unwrap_or(1);
        store.logins = file.logins;
        Ok(store)
    }

    /// Adds a login, or updates the entry with the same pattern and username.
    pub fn save(
        &mut self,
        pattern: &str,
        username: &str,
        password: &str,
        enabled: bool,
    ) -> Result<i64> {
        parse_pattern(pattern)?;
        if !valid_login_fields(username, password) {
            return Err(
                "username needs 1–128 printable bytes and the password 0–1024".into(),
            );
        }
        let id = match self.logins.iter_mut().find(|login| {
            login.pattern == pattern && login.username == username
        }) {
            Some(existing) => {
                existing.password = password.to_owned();
                existing.enabled = enabled;
                existing.id
            }
            None => {
                let id = self.next_id;
                self.next_id += 1;
                self.logins.push(Login {
                    id,
                    pattern: pattern.to_owned(),
                    username: username.to_owned(),
                    password: password.to_owned(),
                    enabled,
                });
                id
            }
        };
        self.persist()?;
        Ok(id)
    }

    pub fn remove(&mut self, id: i64) -> Result<bool> {
        let before = self.logins.len();
        self.logins.retain(|login| login.id != id);
        let removed = self.logins.len() != before;
        if removed {
            self.persist()?;
        }
        Ok(removed)
    }

    /// Client-facing rows: usernames and patterns only.
    pub fn summaries(&self) -> Vec<LoginSummary> {
        self.logins
            .iter()
            .map(|login| LoginSummary {
                id: login.id,
                pattern: login.pattern.clone(),
                username: login.username.clone(),
                enabled: login.enabled,
            })
            .collect()
    }

    /// The enabled login that best matches a request URL, if any.
    pub fn find(&self, url: &Url) -> Option<&Login> {
        self.logins
            .iter()
            .filter(|login| login.enabled)
            .filter_map(|login| {
                parse_pattern(&login.pattern)
                    .ok()
                    .filter(|pattern| pattern.matches(url))
                    .map(|pattern| (pattern, login))
            })
            .max_by_key(|(pattern, _)| pattern.specificity())
            .map(|(_, login)| login)
    }

    /// Rewrites the store atomically: a temporary file with owner-only
    /// permissions is synced and renamed over the previous one.
    fn persist(&self) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let plain = serde_json::to_vec(&File {
            version: 1,
            logins: self.logins.clone(),
        })?;
        let sealed = seal(&plain)?;
        let tmp = path.with_extension("json.tmp");
        // A stale temporary file would keep its old permissions when reopened.
        let _ = fs::remove_file(&tmp);
        {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut writer = options.open(&tmp)?;
            writer.write_all(&sealed)?;
            writer.sync_all()?;
        }
        fs::rename(&tmp, path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
}

type Error = Box<dyn std::error::Error + Send + Sync>;

/// Windows seals the file with DPAPI (user scope, no UI prompts). The path
/// is compile-structural only and is not yet runtime-tested on Windows.
#[cfg(windows)]
fn seal(plain: &[u8]) -> io::Result<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };
    unsafe {
        let input = CRYPT_INTEGER_BLOB {
            cbData: plain.len() as u32,
            pbData: plain.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        if CryptProtectData(
            &input,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let sealed = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData.cast());
        Ok(sealed)
    }
}

#[cfg(windows)]
fn unseal(sealed: &[u8]) -> io::Result<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };
    unsafe {
        let input = CRYPT_INTEGER_BLOB {
            cbData: sealed.len() as u32,
            pbData: sealed.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        let mut description: *mut u16 = std::ptr::null_mut();
        if CryptUnprotectData(
            &input,
            &mut description,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        if !description.is_null() {
            LocalFree(description.cast());
        }
        let plain = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData.cast());
        Ok(plain)
    }
}

/// Other platforms rely on the file's 0600 permissions; no obfuscation.
#[cfg(not(windows))]
fn seal(plain: &[u8]) -> io::Result<Vec<u8>> {
    Ok(plain.to_vec())
}

#[cfg(not(windows))]
fn unseal(sealed: &[u8]) -> io::Result<Vec<u8>> {
    Ok(sealed.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn patterns_parse_and_match() {
        let pattern = parse_pattern("http://127.0.0.1:8080/files").unwrap();
        assert!(pattern.matches(&url("http://127.0.0.1:8080/files/a.zip")));
        assert!(pattern.matches(&url("http://127.0.0.1:8080/files")));
        assert!(!pattern.matches(&url("http://127.0.0.1:8080/other/a.zip")));
        assert!(!pattern.matches(&url("http://127.0.0.1:8081/files/a.zip")));
        assert!(!pattern.matches(&url("https://127.0.0.1:8080/files/a.zip")));
        // A pattern without a port matches any port; the path is a prefix.
        let any_port = parse_pattern("https://cdn.example.com").unwrap();
        assert!(any_port.matches(&url("https://cdn.example.com/x.bin")));
        assert!(any_port.matches(&url("https://cdn.example.com:8443/x.bin")));
        assert!(!any_port.matches(&url("http://cdn.example.com/x.bin")));
        // Wildcards cover subdomains only, never the bare domain.
        let wildcard = parse_pattern("https://*.example.com/files").unwrap();
        assert!(wildcard.matches(&url("https://a.example.com/files/x")));
        assert!(wildcard.matches(&url("https://deep.b.example.com/files/x")));
        assert!(!wildcard.matches(&url("https://example.com/files/x")));
        assert!(!wildcard.matches(&url("https://notexample.com/files/x")));
        // Host case and IPv6 literals.
        assert!(parse_pattern("HTTP://Example.COM/").is_err(), "scheme is validated");
        assert!(
            parse_pattern("http://[::1]:8080/").unwrap().matches(&url("http://[::1]:8080/x")),
            "IPv6 hosts match"
        );
    }

    #[test]
    fn invalid_patterns_are_rejected() {
        for pattern in [
            "",
            "example.com/path",
            "ftp://example.com/",
            "http://",
            "http://host:0/",
            "http://host:notaport/",
            "http://*/path",
            "http://*./path",
            "http://host/ /path",
            "http://host/a b",
            "http://ho]st/",
        ] {
            assert!(parse_pattern(pattern).is_err(), "{pattern}");
        }
    }

    #[test]
    fn base64_matches_rfc_4648_vectors() {
        let cases: [(&[u8], &str); 7] = [
            (b"", ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
            (b"fooba", "Zm9vYmE="),
            (b"foobar", "Zm9vYmFy"),
        ];
        for data in cases {
            assert_eq!(base64_encode(data.0), data.1);
        }
        assert_eq!(basic_token("alice", "s3cret"), "YWxpY2U6czNjcmV0");
    }

    #[test]
    fn specific_patterns_win_over_generic_ones() {
        let store = |entries: &[(&str, bool)]| {
            let mut store = Store::load(None).unwrap();
            for (index, (pattern, enabled)) in entries.iter().enumerate() {
                store
                    .save(pattern, &format!("user{index}"), "pw", *enabled)
                    .unwrap();
            }
            store
        };
        let store = store(&[
            ("http://a.example.com/", true),
            ("http://*.example.com/", true),
            ("http://*.example.com/private", true),
            ("http://a.example.com/private/docs", true),
            ("http://disabled.example.com/", false),
            ("http://skipped.example.org/", false),
        ]);
        let best = |address: &str| {
            store.find(&url(address)).map(|login| login.username.to_owned())
        };
        // Exact host beats wildcard; deeper wildcard suffixes beat shallower
        // ones; only with equal hosts does the longest path prefix win.
        assert_eq!(best("http://a.example.com/private/docs/x.bin"), Some("user3".into()));
        assert_eq!(best("http://a.example.com/private/x.bin"), Some("user0".into()));
        assert_eq!(best("http://b.example.com/private/x.bin"), Some("user2".into()));
        assert_eq!(best("http://b.example.com/other/x.bin"), Some("user1".into()));
        assert_eq!(
            best("http://disabled.example.com/x.bin"),
            Some("user1".into()),
            "the disabled entry is skipped, the wildcard still applies"
        );
        assert_eq!(
            best("http://skipped.example.org/x.bin"),
            None,
            "a host only covered by a disabled entry never matches"
        );
        assert_eq!(best("http://other.invalid/x.bin"), None);
        assert!(store.find(&url("https://a.example.com/")).is_none(), "schemes differ");
    }

    #[test]
    fn store_persists_owner_only_and_hides_passwords() {
        let directory = std::env::temp_dir().join(format!("xdm-credentials-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("credentials.json");
        let mut store = Store::load(Some(path.clone())).unwrap();
        store.save("http://127.0.0.1:1/", "alice", "s3cret", true).unwrap();
        store.save("http://127.0.0.1:1/", "alice", "s3cret", true).unwrap();
        store.save("http://127.0.0.1:1/secure", "bob", "hunter2", false).unwrap();
        assert!(fs::metadata(&path).unwrap().len() > 0);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let reloaded = Store::load(Some(path.clone())).unwrap();
        let summaries = reloaded.summaries();
        assert_eq!(summaries.len(), 2);
        assert!(summaries.iter().all(|row| !serde_json::to_string(row).unwrap().contains("s3cret")));
        assert_eq!(summaries[1].enabled, false);
        // The disabled entry is skipped; the origin login still matches.
        assert_eq!(
            reloaded.find(&url("http://127.0.0.1:1/secure/x")).unwrap().username,
            "alice"
        );
        let mut reloaded = reloaded;
        assert!(reloaded.remove(summaries[0].id).unwrap());
        assert!(!reloaded.remove(999).unwrap());
        assert!(
            reloaded.find(&url("http://127.0.0.1:1/secure/x")).is_none(),
            "nothing enabled remains after removal"
        );
        // Saving the same pattern and username again updates, not duplicates.
        reloaded.save("http://127.0.0.1:1/secure", "bob", "hunter2", true).unwrap();
        assert_eq!(reloaded.summaries().len(), 1);
        assert_eq!(
            reloaded.find(&url("http://127.0.0.1:1/secure/x")).unwrap().username,
            "bob"
        );
        // Corrupt files surface a clear error instead of silent data loss.
        fs::write(&path, b"not json at all").unwrap();
        assert!(Store::load(Some(path.clone())).is_err());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn login_fields_are_bounded() {
        assert!(valid_login_fields("alice", ""));
        assert!(valid_login_fields("a".repeat(128).as_str(), "x".repeat(1024).as_str()));
        assert!(!valid_login_fields("", "pw"));
        assert!(!valid_login_fields("a".repeat(129).as_str(), "pw"));
        assert!(!valid_login_fields("alice", "x".repeat(1025).as_str()));
        assert!(!valid_login_fields("ali\nce", "pw"));
        assert!(!valid_login_fields("alice", "pw\r\nEvil: 1"));
    }
}
