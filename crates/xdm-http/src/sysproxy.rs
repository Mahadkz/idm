#![cfg_attr(windows, allow(dead_code))]
//! Operating-system proxy import for the `system` proxy mode (contract N26).
//!
//! Order: proxy environment variables (handled by the HTTP client itself),
//! then GNOME/GTK `org.gnome.system.proxy` through `gsettings`, KDE's
//! `kioslaverc`, and on Windows the per-user Internet Settings registry key.
//! A PAC script (`auto` mode) is recorded and reported as unsupported; it
//! never silently turns into a direct connection.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A proxy configuration read from the operating system.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SystemProxy {
    /// Where the settings came from (`gnome`, `kde`, `windows`).
    pub source: String,
    pub http: Option<String>,
    pub https: Option<String>,
    pub bypass: Vec<String>,
    /// Set when the system asks for a PAC script or WPAD discovery.
    pub pac: Option<Pac>,
    /// Settings that could not be expressed and were dropped on purpose.
    pub warnings: Vec<String>,
}

impl std::fmt::Debug for SystemProxy {
    // Proxy URLs may carry user info; never print it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SystemProxy")
            .field("source", &self.source)
            .field("http", &self.http.as_deref().map(redact))
            .field("https", &self.https.as_deref().map(redact))
            .field("bypass", &self.bypass)
            .field("pac", &self.pac.as_ref().map(|pac| match pac {
                Pac::Url(url) => Pac::Url(redact(url)),
                Pac::Discovery => Pac::Discovery,
            }))
            .field("warnings", &self.warnings)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pac {
    Url(String),
    Discovery,
}

/// How `Proxy::System` resolves right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// Proxy environment variables are set; the client honors them.
    Environment,
    /// Settings imported from the OS.
    Settings(SystemProxy),
    /// Nothing configured: connect directly.
    Direct,
}

impl SystemProxy {
    /// One-line human summary for diagnostics (never contains credentials).
    pub fn describe(&self) -> String {
        if let Some(pac) = &self.pac {
            return match pac {
                Pac::Url(url) => format!("{} proxy settings use the PAC script {}: PAC not supported", self.source, redact(url)),
                Pac::Discovery => format!("{} proxy settings use automatic discovery (WPAD/PAC): PAC not supported", self.source),
            };
        }
        let mut parts = Vec::new();
        if let Some(http) = &self.http {
            parts.push(format!("http={}", redact(http)));
        }
        if let Some(https) = &self.https {
            parts.push(format!("https={}", redact(https)));
        }
        if !self.bypass.is_empty() {
            parts.push(format!("bypass={}", self.bypass.join(",")));
        }
        for warning in &self.warnings {
            parts.push(format!("[{warning}]"));
        }
        format!("{} proxy settings: {}", self.source, parts.join(" "))
    }
}

/// Removes user info, query and fragment: PAC and proxy URLs often carry tokens there.
pub(crate) fn redact(url: &str) -> String {
    let (scheme, rest) = match url.split_once("://") {
        Some((scheme, rest)) => (Some(scheme), rest),
        None => (None, url),
    };
    let rest = &rest[..rest.find(['?', '#']).unwrap_or(rest.len())];
    // Everything up to the last '@' is user info (a password may hold '/').
    let rest = rest.rsplit_once('@').map_or(rest, |(_, host)| host);
    match scheme {
        Some(scheme) => format!("{scheme}://{rest}"),
        None => rest.to_owned(),
    }
}

/// Environment variables the client treats as proxy configuration.
pub fn environment_proxy_set() -> bool {
    ["http_proxy", "https_proxy", "all_proxy"].iter().any(|name| {
        [name.to_string(), name.to_uppercase()]
            .iter()
            .any(|key| std::env::var(key).is_ok_and(|value| !value.trim().is_empty()))
    })
}

/// How long a resolution is reused instead of spawning `gsettings` again.
fn cache_ttl() -> Duration {
    std::env::var("XDM_SYSTEM_PROXY_TTL_MS")
        .ok()
        .and_then(|value| value.parse().ok())
        .map_or(Duration::from_secs(5), Duration::from_millis)
}

/// Resolves the system proxy settings for this machine, reusing a recent answer.
pub fn resolve() -> Resolved {
    static CACHE: std::sync::Mutex<Option<(Instant, Resolved)>> = std::sync::Mutex::new(None);
    let ttl = cache_ttl();
    if let Ok(mut cache) = CACHE.lock() {
        if let Some((at, found)) = cache.as_ref() {
            if at.elapsed() < ttl {
                return found.clone();
            }
        }
        let fresh = resolve_uncached();
        *cache = Some((Instant::now(), fresh.clone()));
        return fresh;
    }
    resolve_uncached()
}

fn resolve_uncached() -> Resolved {
    if environment_proxy_set() {
        return Resolved::Environment;
    }
    #[cfg(windows)]
    if let Some(found) = windows_settings() {
        return Resolved::Settings(found);
    }
    #[cfg(unix)]
    {
        match gnome_settings() {
            Gnome::Active(found) => return Resolved::Settings(found),
            // GNOME says "none" explicitly: an old kioslaverc must not revive a proxy.
            Gnome::Disabled => return Resolved::Direct,
            Gnome::Unknown => {}
        }
        if let Some(found) = kde_settings() {
            return Resolved::Settings(found);
        }
    }
    Resolved::Direct
}

// ---- parsers -------------------------------------------------------------

fn unquote(value: &str) -> String {
    let value = value.trim();
    // GVariant prints typed values such as `uint32 8080` or `@as []`.
    let value = value.strip_prefix("uint32 ").unwrap_or(value);
    value
        .strip_prefix('\'')
        .and_then(|rest| rest.strip_suffix('\''))
        .unwrap_or(value)
        .to_owned()
}

/// Parses `['a', 'b']` (or `@as []`) from gsettings.
pub fn parse_gvariant_list(value: &str) -> Vec<String> {
    let value = value.trim();
    let inner = value
        .strip_prefix("@as ")
        .unwrap_or(value)
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']');
    inner
        .split(',')
        .map(unquote)
        .filter(|item| !item.is_empty())
        .collect()
}

fn endpoint(scheme: &str, host: &str, port: &str) -> Option<String> {
    let host = host.trim();
    let port = port.trim().parse::<u16>().ok().filter(|port| *port > 0)?;
    if host.is_empty() {
        return None;
    }
    // IPv6 literals need brackets in a URL.
    let host = if host.contains(':') && !host.starts_with('[') { format!("[{host}]") } else { host.to_owned() };
    Some(format!("{scheme}://{host}:{port}"))
}

/// What GNOME's settings say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gnome {
    /// Not a GNOME configuration (no usable `mode`).
    Unknown,
    /// Explicitly no proxy: later sources (KDE files) must not override it.
    Disabled,
    Active(SystemProxy),
}

/// Parses the output of `gsettings list-recursively org.gnome.system.proxy`.
pub fn parse_gnome(output: &str) -> Gnome {
    let mut mode = String::new();
    let mut pac_url = String::new();
    let mut ignore = Vec::new();
    let (mut http, mut https, mut socks) = ((String::new(), String::new()), (String::new(), String::new()), (String::new(), String::new()));
    for line in output.lines() {
        let mut fields = line.trim().splitn(3, ' ');
        let (Some(schema), Some(key), Some(value)) = (fields.next(), fields.next(), fields.next()) else {
            continue;
        };
        match (schema, key) {
            ("org.gnome.system.proxy", "mode") => mode = unquote(value),
            ("org.gnome.system.proxy", "autoconfig-url") => pac_url = unquote(value),
            ("org.gnome.system.proxy", "ignore-hosts") => ignore = parse_gvariant_list(value),
            ("org.gnome.system.proxy.http", "host") => http.0 = unquote(value),
            ("org.gnome.system.proxy.http", "port") => http.1 = unquote(value),
            ("org.gnome.system.proxy.https", "host") => https.0 = unquote(value),
            ("org.gnome.system.proxy.https", "port") => https.1 = unquote(value),
            ("org.gnome.system.proxy.socks", "host") => socks.0 = unquote(value),
            ("org.gnome.system.proxy.socks", "port") => socks.1 = unquote(value),
            _ => {}
        }
    }
    let mut found = SystemProxy {
        source: "gnome".into(),
        bypass: ignore,
        ..Default::default()
    };
    match mode.as_str() {
        "manual" => {
            found.http = endpoint("http", &http.0, &http.1);
            found.https = endpoint("http", &https.0, &https.1);
            if let Some(socks) = endpoint("socks5h", &socks.0, &socks.1) {
                found.http.get_or_insert_with(|| socks.clone());
                found.https.get_or_insert(socks);
            }
            if found.http.is_some() || found.https.is_some() { Gnome::Active(found) } else { Gnome::Disabled }
        }
        "auto" => {
            found.pac = Some(if pac_url.is_empty() { Pac::Discovery } else { Pac::Url(pac_url) });
            Gnome::Active(found)
        }
        "none" => Gnome::Disabled,
        _ => Gnome::Unknown,
    }
}

/// Parses KDE's `kioslaverc` `[Proxy Settings]` group.
pub fn parse_kioslaverc(text: &str) -> Option<SystemProxy> {
    let mut in_group = false;
    let mut values = std::collections::HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(group) = line.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
            in_group = group == "Proxy Settings";
        } else if in_group && let Some((key, value)) = line.split_once('=') {
            values.insert(key.trim().to_owned(), value.trim().to_owned());
        }
    }
    let kind: u8 = values.get("ProxyType")?.parse().ok()?;
    let mut found = SystemProxy {
        source: "kde".into(),
        ..Default::default()
    };
    let url = |key: &str| {
        let value = values.get(key)?.trim();
        if value.is_empty() {
            return None;
        }
        // Old files use "host port"; newer ones a URL.
        let value = match value.rsplit_once(' ') {
            Some((host, port)) if !value.contains("://") => format!("{host}:{port}"),
            _ => value.to_owned(),
        };
        Some(if value.contains("://") { value } else { format!("http://{value}") })
    };
    match kind {
        1 => {
            found.http = url("httpProxy");
            found.https = url("httpsProxy").or_else(|| found.http.clone());
            if let Some(list) = values.get("NoProxyFor") {
                found.bypass = list.split(',').map(|item| item.trim().to_owned()).filter(|item| !item.is_empty()).collect();
            }
            (found.http.is_some() || found.https.is_some()).then_some(found)
        }
        2 => {
            let script = values.get("Proxy Config Script").cloned().unwrap_or_default();
            found.pac = Some(if script.is_empty() { Pac::Discovery } else { Pac::Url(script) });
            Some(found)
        }
        3 => {
            found.pac = Some(Pac::Discovery);
            Some(found)
        }
        // 0 is "no proxy"; 4 means "read the environment", which is empty here.
        _ => None,
    }
}

/// Parses the Windows Internet Settings values.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn parse_windows(enabled: bool, server: &str, over: &str, auto_config: &str) -> Option<SystemProxy> {
    let mut found = SystemProxy {
        source: "windows".into(),
        ..Default::default()
    };
    if !enabled || server.trim().is_empty() {
        // Enabled with no server named means the script (or nothing) decides.
        if auto_config.trim().is_empty() {
            return None;
        }
        found.pac = Some(Pac::Url(auto_config.trim().to_owned()));
        return Some(found);
    }
    let with_scheme = |scheme: &str, value: &str| {
        let value = value.trim();
        if value.is_empty() {
            None
        } else if value.contains("://") {
            Some(value.to_owned())
        } else {
            Some(format!("{scheme}://{value}"))
        }
    };
    if server.contains('=') {
        for entry in server.split(';') {
            let Some((protocol, value)) = entry.split_once('=') else { continue };
            match protocol.trim().to_ascii_lowercase().as_str() {
                "http" => found.http = with_scheme("http", value),
                "https" => found.https = with_scheme("http", value),
                "socks" => {
                    let socks = with_scheme("socks5h", value);
                    if found.http.is_none() {
                        found.http = socks.clone();
                    }
                    if found.https.is_none() {
                        found.https = socks;
                    }
                }
                _ => {}
            }
        }
    } else {
        found.http = with_scheme("http", server);
        found.https = found.http.clone();
    }
    for item in over.split(';').map(str::trim).filter(|item| !item.is_empty()) {
        // A bare "*" would silently turn every host into a bypass.
        if matches!(item, "*" | "*.*") {
            found.warnings.push(format!("ignored match-everything bypass entry {item}"));
        } else {
            found.bypass.extend(windows_bypass(item));
        }
    }
    (found.http.is_some() || found.https.is_some()).then_some(found)
}

/// Turns one ProxyOverride pattern into entries the client understands.
#[cfg_attr(not(windows), allow(dead_code))]
fn windows_bypass(item: &str) -> Vec<String> {
    if item.eq_ignore_ascii_case("<local>") {
        return vec!["localhost".into(), "127.0.0.1".into(), "::1".into()];
    }
    if matches!(item, "*" | "*.*") {
        return Vec::new();
    }
    if let Some(domain) = item.strip_prefix("*.") {
        if domain.contains('*') {
            return Vec::new();
        }
        return vec![domain.to_owned()];
    }
    // `192.168.*` style prefixes become CIDR ranges.
    let octets: Vec<&str> = item.split('.').collect();
    if octets.last() == Some(&"*")
        && (2..=4).contains(&octets.len())
        && octets[..octets.len() - 1].iter().all(|octet| octet.parse::<u8>().is_ok())
    {
        let fixed = octets.len() - 1;
        let mut address: Vec<String> = octets[..fixed].iter().map(|s| s.to_string()).collect();
        address.resize(4, "0".into());
        return vec![format!("{}/{}", address.join("."), fixed * 8)];
    }
    if item.contains('*') {
        // Mid-string wildcards cannot be expressed; dropping them errs toward the proxy.
        return Vec::new();
    }
    vec![item.to_owned()]
}

// ---- platform readers ----------------------------------------------------

/// Runs a program with a short deadline; `None` when missing, failing or slow.
/// Stdout is drained on its own thread so a chatty child cannot fill the pipe and stall.
fn run_bounded(program: &str, args: &[&str]) -> Option<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        // A runaway child cannot grow the buffer past 1 MiB.
        let _ = std::io::Read::read_to_string(&mut std::io::Read::take(&mut stdout, 1 << 20), &mut text);
        text
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    let success = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => break false,
        }
    };
    let text = reader.join().ok()?;
    success.then_some(text)
}

#[cfg(unix)]
fn gnome_settings() -> Gnome {
    match run_bounded("gsettings", &["list-recursively", "org.gnome.system.proxy"]) {
        Some(output) => parse_gnome(&output),
        None => Gnome::Unknown,
    }
}

#[cfg(unix)]
fn kde_settings() -> Option<SystemProxy> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| std::path::Path::new(&home).join(".config")))?;
    parse_kioslaverc(&std::fs::read_to_string(base.join("kioslaverc")).ok()?)
}

#[cfg(windows)]
fn name_text(name: &[u16]) -> String {
    String::from_utf16_lossy(&name[..name.len().saturating_sub(1)])
}

#[cfg(windows)]
fn windows_settings() -> Option<SystemProxy> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RRF_RT_REG_SZ};
    let key: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings\0".encode_utf16().collect();
    let string = |name: &str| -> String {
        let name: Vec<u16> = format!("{name}\0").encode_utf16().collect();
        let mut buffer = vec![0u16; 4096];
        let mut size = (buffer.len() * 2) as u32;
        let mut status;
        // Two rounds: the second uses the size the first reported for an oversize value.
        for _ in 0..2 {
            // SAFETY: both strings are NUL-terminated and the buffer size matches `size`.
            status = unsafe {
                RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buffer.as_mut_ptr().cast(), &mut size)
            };
            if status == 234 && (size as usize) <= (1 << 20) {
                buffer = vec![0u16; (size as usize).div_ceil(2) + 1];
                size = (buffer.len() * 2) as u32;
                continue;
            }
            if status != 0 {
                if status == 234 {
                    eprintln!("warning: proxy registry value {} exceeds 1 MiB and was ignored", name_text(&name));
                }
                return String::new();
            }
            break;
        }
        let chars = (size as usize / 2).saturating_sub(1);
        String::from_utf16_lossy(&buffer[..chars.min(buffer.len())])
    };
    let enabled = {
        let name: Vec<u16> = "ProxyEnable\0".encode_utf16().collect();
        let mut value = 0u32;
        let mut size = 4u32;
        // SAFETY: the output is a 4-byte DWORD.
        let status = unsafe {
            RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), (&mut value as *mut u32).cast(), &mut size)
        };
        status == 0 && value != 0
    };
    parse_windows(enabled, &string("ProxyServer"), &string("ProxyOverride"), &string("AutoConfigURL"))
}

/// Human-readable account of how `system` mode resolves right now.
pub fn status() -> String {
    match resolve() {
        Resolved::Environment => "environment proxy variables".into(),
        Resolved::Settings(found) => found.describe(),
        Resolved::Direct => "no system proxy configured; connecting directly".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GNOME: &str = "org.gnome.system.proxy autoconfig-url ''\n\
org.gnome.system.proxy ignore-hosts ['localhost', '127.0.0.0/8', '::1']\n\
org.gnome.system.proxy mode 'manual'\n\
org.gnome.system.proxy use-same-proxy true\n\
org.gnome.system.proxy.ftp host ''\n\
org.gnome.system.proxy.ftp port uint32 0\n\
org.gnome.system.proxy.http host 'proxy.corp'\n\
org.gnome.system.proxy.http port 8080\n\
org.gnome.system.proxy.https host 'secure.corp'\n\
org.gnome.system.proxy.https port 8443\n\
org.gnome.system.proxy.socks host ''\n\
org.gnome.system.proxy.socks port 0\n";

    #[test]
    fn gnome_manual_auto_and_none() {
        let Gnome::Active(manual) = parse_gnome(GNOME) else { panic!() };
        assert_eq!(manual.http.as_deref(), Some("http://proxy.corp:8080"));
        assert_eq!(manual.https.as_deref(), Some("http://secure.corp:8443"));
        assert_eq!(manual.bypass, vec!["localhost", "127.0.0.0/8", "::1"]);
        let auto = GNOME.replace("'manual'", "'auto'").replace("autoconfig-url ''", "autoconfig-url 'http://wpad/proxy.pac'");
        let Gnome::Active(found) = parse_gnome(&auto) else { panic!() };
        assert_eq!(found.pac, Some(Pac::Url("http://wpad/proxy.pac".into())));
        assert!(found.describe().contains("PAC not supported"));
        assert_eq!(parse_gnome(&GNOME.replace("'manual'", "'none'")), Gnome::Disabled);
        assert_eq!(parse_gnome(""), Gnome::Unknown);
        // Manual mode with only SOCKS set routes both protocols through it.
        let socks = GNOME
            .replace("http host 'proxy.corp'", "http host ''")
            .replace("https host 'secure.corp'", "https host ''")
            .replace("socks host ''\norg.gnome.system.proxy.socks port 0", "socks host '10.0.0.1'\norg.gnome.system.proxy.socks port 1080");
        let Gnome::Active(found) = parse_gnome(&socks) else { panic!() };
        assert_eq!(found.http.as_deref(), Some("socks5h://10.0.0.1:1080"));
    }

    #[test]
    fn kde_manual_pac_and_none() {
        let manual = "[Proxy Settings]\nProxyType=1\nhttpProxy=http://kde.proxy:3128\nhttpsProxy=kde.proxy 3129\nNoProxyFor=localhost,.lan\n[Other]\nProxyType=0\n";
        let found = parse_kioslaverc(manual).unwrap();
        assert_eq!(found.http.as_deref(), Some("http://kde.proxy:3128"));
        assert_eq!(found.https.as_deref(), Some("http://kde.proxy:3129"));
        assert_eq!(found.bypass, vec!["localhost", ".lan"]);
        let pac = parse_kioslaverc("[Proxy Settings]\nProxyType=2\nProxy Config Script=http://x/p.pac\n").unwrap();
        assert_eq!(pac.pac, Some(Pac::Url("http://x/p.pac".into())));
        assert_eq!(parse_kioslaverc("[Proxy Settings]\nProxyType=3\n").unwrap().pac, Some(Pac::Discovery));
        assert!(parse_kioslaverc("[Proxy Settings]\nProxyType=0\n").is_none());
        assert!(parse_kioslaverc("[General]\nx=1\n").is_none());
    }

    #[test]
    fn windows_values() {
        let single = parse_windows(true, "proxy.corp:8080", "*.corp;192.168.*;<local>;a*b", "").unwrap();
        assert_eq!(single.http.as_deref(), Some("http://proxy.corp:8080"));
        assert_eq!(single.https, single.http);
        assert_eq!(single.bypass, vec!["corp", "192.168.0.0/16", "localhost", "127.0.0.1", "::1"]);
        let per = parse_windows(true, "http=h:80;https=s:443;socks=k:1080", "", "").unwrap();
        assert_eq!((per.http.as_deref(), per.https.as_deref()), (Some("http://h:80"), Some("http://s:443")));
        let socks_only = parse_windows(true, "socks=k:1080", "", "").unwrap();
        assert_eq!(socks_only.http.as_deref(), Some("socks5h://k:1080"));
        assert!(parse_windows(false, "x:1", "", "").is_none());
        assert_eq!(parse_windows(false, "", "", "http://pac/x.pac").unwrap().pac, Some(Pac::Url("http://pac/x.pac".into())));
        assert_eq!(windows_bypass("10.*"), vec!["10.0.0.0/8"]);
    }

    #[test]
    fn descriptions_hide_credentials() {
        let found = SystemProxy { source: "x".into(), http: Some("http://u:pw@h:1".into()), ..Default::default() };
        assert!(!found.describe().contains("pw"));
    }

    #[test]
    fn urls_are_redacted_and_debug_hides_user_info() {
        assert_eq!(redact("http://u:pa/ss@proxy:80"), "http://proxy:80");
        assert_eq!(redact("http://wpad/p.pac?token=abc#x"), "http://wpad/p.pac");
        assert_eq!(redact("https://u:p@h/pac.js?k=v"), "https://h/pac.js");
        let found = SystemProxy {
            source: "x".into(),
            http: Some("http://u:topsecret@h:1".into()),
            pac: Some(Pac::Url("http://a:tok@h/p?s=1".into())),
            ..Default::default()
        };
        let text = format!("{found:?} {}", found.describe());
        assert!(!text.contains("topsecret") && !text.contains("tok@") && !text.contains("s=1"), "{text}");
    }

    #[test]
    fn windows_match_everything_and_empty_server_cases() {
        let found = parse_windows(true, "p:1", "*;*.*;*.corp", "").unwrap();
        assert_eq!(found.bypass, vec!["corp"]);
        assert_eq!(found.warnings.len(), 2);
        assert!(windows_bypass("*").is_empty() && windows_bypass("*.*").is_empty());
        // Enabled without a server but with a script: PAC, never direct.
        let pac = parse_windows(true, "", "", "http://pac/x.pac").unwrap();
        assert_eq!(pac.pac, Some(Pac::Url("http://pac/x.pac".into())));
        assert!(parse_windows(true, "", "", "").is_none());
    }

    #[test]
    fn chatty_children_do_not_deadlock_and_slow_ones_time_out() {
        // 512 KiB of output exceeds the pipe buffer; the reader thread drains it.
        let out = run_bounded("sh", &["-c", "head -c 524288 /dev/zero | tr '\\0' x"]).unwrap();
        assert_eq!(out.len(), 524288);
        let start = Instant::now();
        assert!(run_bounded("sleep", &["30"]).is_none());
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
