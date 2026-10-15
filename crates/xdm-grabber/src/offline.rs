//! Offline-browsing support: maps crawled URLs to safe relative paths under
//! a destination directory and rewrites links in saved HTML to those paths.

use crate::html;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use url::Url;

const MAX_SEGMENT_BYTES: usize = 100;
const MAX_SEGMENTS: usize = 32;
/// Longest relative path produced; deeper paths get a hashed directory.
const MAX_REL_BYTES: usize = 240;
const RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// One path segment made safe on every supported platform.
fn sanitize(raw: &str) -> String {
    let mut out: String = raw
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    while out.len() > MAX_SEGMENT_BYTES {
        out.pop();
    }
    let trimmed = out.trim_end_matches(['.', ' ']).to_string();
    let mut out = if trimmed.is_empty() { "_".to_string() } else { trimmed };
    // Windows also reserves COM and LPT followed by superscript digits.
    let stem: String = out
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end_matches([' '])
        .chars()
        .map(|c| match c {
            '\u{b9}' => '1',
            '\u{b2}' => '2',
            '\u{b3}' => '3',
            other => other.to_ascii_lowercase(),
        })
        .collect();
    if RESERVED.contains(&stem.as_str()) {
        out.insert(0, '_');
    }
    out
}

fn fnv_hex(text: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{:08x}", hash & 0xffff_ffff)
}

/// Splits `name` into stem and extension (with its dot).
fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(index) if index > 0 => name.split_at(index),
        _ => (name, ""),
    }
}

pub struct Mapper {
    root: std::path::PathBuf,
    start_host: String,
    used: HashSet<String>,
    dirs: HashSet<String>,
}

impl Mapper {
    pub fn new(root: &Path, start_host: &str) -> Self {
        Self {
            root: root.to_path_buf(),
            start_host: start_host.to_ascii_lowercase(),
            used: HashSet::new(),
            dirs: HashSet::new(),
        }
    }

    fn taken(&self, candidate: &str) -> bool {
        let key = candidate.to_ascii_lowercase();
        if self.used.contains(&key) || self.dirs.contains(&key) {
            return true;
        }
        if self.root.join(candidate).symlink_metadata().is_ok() {
            return true;
        }
        // A prefix of the candidate already used as a file blocks the path.
        let mut prefix = String::new();
        let parts: Vec<&str> = key.split('/').collect();
        for part in &parts[..parts.len() - 1] {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            if self.used.contains(&prefix) {
                return true;
            }
        }
        false
    }

    /// Assigns a unique relative path (forward slashes) for a URL.
    pub fn assign(&mut self, url: &Url, page: bool) -> String {
        let mut segments = Vec::new();
        let host = url.host_str().unwrap_or("").to_ascii_lowercase();
        if host != self.start_host {
            let with_port = match url.port() {
                Some(port) => format!("{host}_{port}"),
                None => host,
            };
            segments.push(sanitize(&with_port));
        }
        let decoded: Vec<String> = url
            .path_segments()
            .map(|parts| {
                parts
                    .map(|part| {
                        percent_encoding::percent_decode_str(part)
                            .decode_utf8_lossy()
                            .into_owned()
                    })
                    .collect()
            })
            .unwrap_or_default();
        let directory_like = decoded.last().is_none_or(|last| last.is_empty());
        let mut names: Vec<String> = decoded
            .into_iter()
            .filter(|part| !part.is_empty() && part != "." && part != "..")
            .map(|part| sanitize(&part))
            .collect();
        if names.len() > MAX_SEGMENTS {
            names.drain(..names.len() - MAX_SEGMENTS);
        }
        if directory_like || names.is_empty() {
            names.push("index.html".into());
        }
        segments.extend(names);
        let file = segments.pop().unwrap_or_else(|| "index.html".into());
        let (stem, extension) = split_extension(&file);
        let (mut stem, mut extension) = (stem.to_string(), extension.to_string());
        if let Some(query) = url.query() {
            stem = format!("{stem}_q{}", fnv_hex(query));
        }
        if page {
            let lower = extension.to_ascii_lowercase();
            if lower != ".html" && lower != ".htm" {
                // Keep the original name visible: page.php -> page.php.html.
                stem = format!("{stem}{extension}");
                extension = ".html".into();
            }
        }
        let directory = segments.join("/");
        let make = |suffix: &str| {
            let name = format!("{stem}{suffix}{extension}");
            let directory = if directory.len() + name.len() + 1 > MAX_REL_BYTES {
                format!("d{}", fnv_hex(&directory))
            } else {
                directory.clone()
            };
            if directory.is_empty() {
                name
            } else {
                format!("{directory}/{name}")
            }
        };
        let mut candidate = make("");
        let mut counter = 1u32;
        while self.taken(&candidate) {
            counter += 1;
            candidate = make(&format!("-{counter}"));
        }
        self.used.insert(candidate.to_ascii_lowercase());
        let mut dir = String::new();
        for part in candidate.split('/').take(candidate.split('/').count() - 1) {
            if !dir.is_empty() {
                dir.push('/');
            }
            dir.push_str(&part.to_ascii_lowercase());
            self.dirs.insert(dir.clone());
        }
        candidate
    }
}

fn encode_segment(segment: &str) -> String {
    let mut out = String::new();
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Relative link from the saved page `from` to the saved path `to`.
pub fn relative_link(from: &str, to: &str) -> String {
    let from_dirs: Vec<&str> = from.split('/').collect();
    let from_dirs = &from_dirs[..from_dirs.len() - 1];
    let to_parts: Vec<&str> = to.split('/').collect();
    let common = from_dirs
        .iter()
        .zip(&to_parts[..to_parts.len() - 1])
        .take_while(|(a, b)| a == b)
        .count();
    let mut parts: Vec<String> = vec!["..".into(); from_dirs.len() - common];
    parts.extend(to_parts[common..].iter().map(|part| encode_segment(part)));
    parts.join("/")
}

/// Escapes a value spliced into an HTML attribute of any quoting style.
fn escape_attribute(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '\'' => out.push_str("&#39;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '`' => out.push_str("&#96;"),
            c if c.is_whitespace() => out.push_str(&format!("&#{};", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Creates the parent directories of `relative` (forward slashes) under
/// `root`, refusing any existing component that is a symlink or not a
/// directory so a pre-planted link cannot redirect writes out of `root`.
pub fn create_mirror_dirs(root: &Path, relative: &str) -> std::io::Result<()> {
    let denied = |what: &str| std::io::Error::new(std::io::ErrorKind::PermissionDenied, what.to_string());
    std::fs::create_dir_all(root)?;
    let mut current = root.to_path_buf();
    let parts: Vec<&str> = relative.split('/').collect();
    for part in &parts[..parts.len().saturating_sub(1)] {
        current.push(part);
        match current.symlink_metadata() {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(denied("destination contains a symlinked directory"));
            }
            Ok(meta) if !meta.is_dir() => return Err(denied("destination component is a file")),
            Ok(_) => {}
            Err(_) => std::fs::create_dir(&current)?,
        }
    }
    Ok(())
}

/// Like `create_mirror_dirs` for an existing file: also refuses a symlink as
/// the final component.
pub fn check_mirror_file(root: &Path, relative: &str) -> std::io::Result<()> {
    create_mirror_dirs(root, relative)?;
    let path = root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
    if path.symlink_metadata()?.file_type().is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "saved file is a symlink",
        ));
    }
    Ok(())
}

/// Rewrites links in `text` (saved at `page_rel`) that point at saved
/// pages or files; other links stay absolute. Returns `None` when nothing
/// changed. Pages with a `<base href>` are left alone because relative
/// links would resolve against that base.
pub fn rewrite(
    text: &str,
    page_url: &Url,
    page_rel: &str,
    local: &HashMap<String, String>,
) -> Option<String> {
    let extracted = html::extract(text);
    if extracted.base.is_some() {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    let mut changed = false;
    for reference in &extracted.refs {
        if reference.start < cursor || reference.end > text.len() {
            continue;
        }
        let Ok(mut target) = page_url.join(reference.value.trim()) else {
            continue;
        };
        let fragment = target.fragment().map(str::to_owned);
        target.set_fragment(None);
        let Some(saved) = local.get(target.as_str()) else {
            continue;
        };
        let mut link = relative_link(page_rel, saved);
        if let Some(fragment) = fragment {
            link.push('#');
            link.push_str(&fragment);
        }
        out.push_str(&text[cursor..reference.start]);
        out.push_str(&escape_attribute(&link));
        cursor = reference.end;
        changed = true;
    }
    out.push_str(&text[cursor..]);
    changed.then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapper() -> Mapper {
        Mapper::new(Path::new("/nonexistent-xdm-root"), "site.test")
    }

    #[test]
    fn paths_are_sanitized_and_collisions_get_suffixes() {
        let mut m = mapper();
        let u = |s: &str| Url::parse(s).unwrap();
        assert_eq!(m.assign(&u("http://site.test/"), true), "index.html");
        let mapped = m.assign(&u("http://site.test/a/../../b%2F..%2Fc"), true);
        assert!(mapped.split('/').all(|part| part != ".." && part != "."), "{mapped}");
        let first = m.assign(&u("http://site.test/p.html?id=1"), true);
        let second = m.assign(&u("http://site.test/p.html?id=2"), true);
        assert_ne!(first, second);
        assert_eq!(m.assign(&u("http://site.test/x/y.zip"), false), "x/y.zip");
        assert_eq!(m.assign(&u("http://site.test/X/Y.zip"), false), "X/Y-2.zip");
        assert_eq!(m.assign(&u("http://other.test/z.zip"), false), "other.test/z.zip");
        assert_eq!(m.assign(&u("http://site.test/con"), true), "_con.html");
    }

    #[test]
    fn links_are_relative_and_only_rewritten_when_saved() {
        assert_eq!(relative_link("a/p.html", "a/c/x.zip"), "c/x.zip");
        assert_eq!(relative_link("a/b/p.html", "a/x.zip"), "../x.zip");
        let mut local = HashMap::new();
        local.insert("http://site.test/files/a.zip".to_string(), "files/a.zip".to_string());
        let page = Url::parse("http://site.test/p/q.html").unwrap();
        let text = r#"<a href="/files/a.zip#f">a</a><a href='http://x.test/o.zip'>o</a>"#;
        let out = rewrite(text, &page, "p/q.html", &local).unwrap();
        assert!(out.contains(r#"href="../files/a.zip#f""#), "{out}");
        assert!(out.contains("http://x.test/o.zip"), "{out}");
    }

    fn one_local() -> (Url, HashMap<String, String>) {
        let mut local = HashMap::new();
        local.insert("http://site.test/p.html".to_string(), "p.html".to_string());
        (Url::parse("http://site.test/").unwrap(), local)
    }

    #[test]
    fn fragments_cannot_break_out_of_attributes() {
        let (page, local) = one_local();
        for quote in ['\'', '"'] {
            let text = format!("<a href={quote}/p.html#x&apos;&quot;onmouseover=alert(1)//{quote}>x</a>");
            let out = rewrite(&text, &page, "index.html", &local).unwrap();
            let value = &out[out.find('=').unwrap() + 2..out.rfind(quote).unwrap()];
            assert!(!value.contains('\'') && !value.contains('"'), "{out}");
            assert!(value.starts_with("p.html#x&#39;%22onmouseover"), "{out}");
        }
        let out = rewrite("<a href=/p.html#a&apos;b>x</a>", &page, "index.html", &local).unwrap();
        assert!(!out.contains("#a'"), "{out}");
    }

    #[test]
    fn valueless_attributes_leave_the_page_untouched() {
        let (page, local) = one_local();
        let text = "<a href>x</a><img src><a href=''>e</a><a href=\"/p.html\">p</a>";
        let out = rewrite(text, &page, "index.html", &local).unwrap();
        assert_eq!(out, "<a href>x</a><img src><a href=''>e</a><a href=\"p.html\">p</a>");
        assert!(rewrite("<a href>x</a>", &page, "index.html", &local).is_none());
    }

    #[test]
    fn rewriting_many_links_is_linear() {
        let (page, local) = one_local();
        let text = "<a href=\"/p.html\">x</a>".repeat(100_000);
        let started = std::time::Instant::now();
        let out = rewrite(&text, &page, "index.html", &local).unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        assert_eq!(out.matches("p.html").count(), 100_000);
    }

    #[test]
    fn superscript_reserved_names_and_deep_paths_are_handled() {
        for name in ["COM\u{b9}", "lpt\u{b2}.txt", "COM\u{b3}.html"] {
            assert!(sanitize(name).starts_with('_'), "{name}");
        }
        let mut m = mapper();
        let deep: String = (0..30).map(|i| format!("{}{i}/", "d".repeat(90))).collect();
        let url = Url::parse(&format!("http://site.test/{deep}file.zip")).unwrap();
        let mapped = m.assign(&url, false);
        assert!(mapped.len() <= MAX_REL_BYTES, "{}", mapped.len());
        assert!(mapped.ends_with("file.zip"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_ancestors_are_refused() {
        let base = std::env::temp_dir().join(format!("xdm-symlink-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (root, outside) = (base.join("root"), base.join("outside"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        assert!(create_mirror_dirs(&root, "link/sub/x.html").is_err());
        assert!(!outside.join("sub").exists());
        assert!(create_mirror_dirs(&root, "ok/sub/x.html").is_ok());
        std::fs::remove_dir_all(&base).unwrap();
    }
}
