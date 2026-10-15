//! Deterministic extension/MIME category rules shared by the daemon and native host.
//!
//! Besides the built-in rules a user may define named categories with file
//! extensions, a destination directory and host-glob site patterns. A site
//! match always beats an extension match, user rules always beat the built-ins
//! (including an edited built-in name), and the pure built-in `classify`
//! applies last.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const CATEGORIES: [&str; 6] = [
    "Compressed",
    "Documents",
    "Music",
    "Programs",
    "Video",
    "Other",
];

/// A user-defined category, possibly an edit of a built-in name: its
/// extensions, destination directory and site patterns replace the defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CategoryRule {
    pub name: String,
    #[serde(default)]
    pub extensions: Vec<String>,
    /// Absolute destination directory; empty means the default download root.
    #[serde(default)]
    pub destination: String,
    /// Host globs such as `*.example.com` (case-insensitive, `*` and `?`).
    #[serde(default)]
    pub sites: Vec<String>,
}

/// A rule is only as large as its bounded, serialisable parts.
pub fn valid_rule(rule: &CategoryRule) -> Result<(), String> {
    if rule.name.trim() != rule.name
        || rule.name.is_empty()
        || rule.name.len() > 64
        || rule.name.chars().any(char::is_control)
    {
        return Err("category name must be 1–64 trimmed characters".into());
    }
    if rule.extensions.len() > 64 || rule.sites.len() > 64 {
        return Err("a category allows at most 64 extensions and 64 site patterns".into());
    }
    for extension in &rule.extensions {
        let trimmed = extension.trim().trim_start_matches('.');
        if trimmed.is_empty() || trimmed.len() > 16 || trimmed.contains(char::is_whitespace) {
            return Err(format!("invalid extension: {extension}"));
        }
    }
    for site in &rule.sites {
        if site.trim().is_empty() || site.trim().len() > 250 || site.trim() != site.as_str() {
            return Err(format!("invalid site pattern: {site}"));
        }
    }
    if !rule.destination.is_empty()
        && (!Path::new(&rule.destination).is_absolute()
            || Path::new(&rule.destination)
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir)))
    {
        return Err("category destination must be an absolute directory without ..".into());
    }
    Ok(())
}

/// Normalises an extension for comparison: trimmed, lowercased, no leading dot.
pub fn normalize_extension(extension: &str) -> String {
    extension
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase()
}

/// Case-insensitive glob against host names supporting `*` and `?` only. The
/// conventional `*.` prefix also matches the bare domain: `*.example.com`
/// matches both `www.example.com` and `example.com`.
pub fn host_matches(pattern: &str, host: &str) -> bool {
    let (pattern, host) = (
        pattern.trim().to_ascii_lowercase(),
        host.trim().to_ascii_lowercase(),
    );
    // `*.example.com` also matches the bare domain; wildcards in the suffix still glob.
    if let Some(suffix) = pattern.strip_prefix("*.") {
        return glob_match(suffix, &host) || glob_match(&pattern, &host);
    }
    glob_match(&pattern, &host)
}

fn glob_match(pattern: &str, text: &str) -> bool {
    let (pattern, text) = (pattern.as_bytes(), text.as_bytes());
    let mut pattern_at = 0;
    let mut text_at = 0;
    let mut star: Option<(usize, usize)> = None;
    while text_at < text.len() {
        let byte = pattern.get(pattern_at).copied();
        if byte == Some(b'*') {
            star = Some((pattern_at, text_at));
            pattern_at += 1;
        } else if byte == Some(text[text_at]) || byte == Some(b'?') {
            pattern_at += 1;
            text_at += 1;
        } else {
            match star {
                None => return false,
                Some((star_pattern, star_text)) => {
                    star = Some((star_pattern, star_text + 1));
                    pattern_at = star_pattern + 1;
                    text_at = star_text + 1;
                }
            }
        }
    }
    while pattern.get(pattern_at) == Some(&b'*') {
        pattern_at += 1;
    }
    pattern_at == pattern.len()
}

/// Classifies with user rules: a site-pattern match wins over extensions,
/// extensions win over the built-in table, MIME wins for extension-less names.
/// Rule names may be custom or edited built-ins.
pub fn classify_with(
    rules: &[CategoryRule],
    name: &str,
    host: Option<&str>,
    mime: Option<&str>,
) -> String {
    let host = host
        .unwrap_or("")
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']');
    let extension = Path::new(name)
        .extension()
        .and_then(|part| part.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    for rule in rules {
        if rule
            .sites
            .iter()
            .any(|site| !host.is_empty() && host_matches(site, host))
        {
            return rule.name.clone();
        }
    }
    for rule in rules {
        if !extension.is_empty()
            && rule
                .extensions
                .iter()
                .any(|candidate| normalize_extension(candidate) == extension)
        {
            return rule.name.clone();
        }
    }
    classify(name, mime).into()
}

/// Destination override for a rule that classified `name`; empty when the
/// built-in layout applies.
pub fn destination_for<'a>(rules: &'a [CategoryRule], category: &str) -> Option<&'a str> {
    rules
        .iter()
        .find(|rule| rule.name == category)
        .map(|rule| rule.destination.as_str())
        .filter(|destination| !destination.is_empty())
}

/// Static extension table behind `classify`, exposed so callers can render
/// built-in categories as editable rules.
pub const BUILTIN_EXTENSIONS: [(&str, &[&str]); 6] = [
    ("Compressed", &["zip", "7z", "rar", "tar", "gz", "bz2", "xz", "zst"]),
    (
        "Documents",
        &["pdf", "doc", "docx", "odt", "txt", "rtf", "xls", "xlsx", "ppt", "pptx", "epub"],
    ),
    ("Music", &["mp3", "m4a", "aac", "flac", "wav", "ogg", "opus"]),
    (
        "Programs",
        &["exe", "msi", "msix", "deb", "rpm", "appimage", "dmg", "pkg"],
    ),
    ("Video", &["mp4", "mkv", "webm", "mov", "avi", "m4v", "ts"]),
    ("Other", &[]),
];

/// Extensions the built-in table assigns to a category.
pub fn builtin_extensions(category: &str) -> Vec<String> {
    BUILTIN_EXTENSIONS
        .iter()
        .find(|(name, _)| *name == category)
        .map(|(_, extensions)| extensions.iter().map(|ext| ext.to_string()).collect())
        .unwrap_or_default()
}

/// Invisible Unicode format and bidirectional controls that can disguise a file's real
/// name or extension.
fn is_format_char(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
}

pub fn valid_filename(name: &str) -> bool {
    if name.is_empty()
        || name.len() > 240
        || name == "."
        || name == ".."
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|c| "/\\:<>\"|?*".contains(c) || c.is_control() || is_format_char(c))
    {
        return false;
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    !(["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].contains(&stem.as_str())
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0'))
}

pub fn classify(name: &str, mime: Option<&str>) -> &'static str {
    let extension = Path::new(name)
        .extension()
        .and_then(|part| part.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match extension.as_str() {
        "zip" | "7z" | "rar" | "tar" | "gz" | "bz2" | "xz" | "zst" => return "Compressed",
        "pdf" | "doc" | "docx" | "odt" | "txt" | "rtf" | "xls" | "xlsx" | "ppt" | "pptx"
        | "epub" => return "Documents",
        "mp3" | "m4a" | "aac" | "flac" | "wav" | "ogg" | "opus" => return "Music",
        "exe" | "msi" | "msix" | "deb" | "rpm" | "appimage" | "dmg" | "pkg" => return "Programs",
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "ts" => return "Video",
        _ => {}
    }
    let mime = mime
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if mime.starts_with("video/") {
        "Video"
    } else if mime.starts_with("audio/") {
        "Music"
    } else if mime == "application/pdf" || mime.starts_with("text/") {
        "Documents"
    } else if [
        "application/zip",
        "application/x-7z-compressed",
        "application/gzip",
    ]
    .contains(&mime.as_str())
    {
        "Compressed"
    } else {
        "Other"
    }
}

pub fn browser_destination(downloads: &Path, name: &str, mime: Option<&str>) -> PathBuf {
    let category = classify(name, mime);
    if category == "Other" {
        downloads.join(name)
    } else {
        downloads.join(category).join(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invisible_and_bidi_characters_are_not_valid_in_names() {
        for c in ['\u{200B}', '\u{200C}', '\u{200D}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}', '\u{FEFF}'] {
            assert!(!valid_filename(&format!("report{c}fdp.exe")), "{c:?}");
            assert!(!valid_filename(&format!("{c}a.txt")), "{c:?}");
        }
        assert!(valid_filename("r\u{e9}sum\u{e9} \u{65E5}\u{672C}.pdf"));
    }

    #[test]
    fn extension_and_mime_rules_are_stable() {
        assert_eq!(
            classify("film.MP4", Some("application/octet-stream")),
            "Video"
        );
        assert_eq!(
            classify("unknown", Some("audio/mp4; charset=binary")),
            "Music"
        );
        assert_eq!(classify("report.pdf", Some("video/mp4")), "Documents");
        assert_eq!(classify("archive.tar.gz", None), "Compressed");
        assert_eq!(classify("opaque.bin", None), "Other");
    }

    fn rules() -> Vec<CategoryRule> {
        vec![
            CategoryRule {
                name: "Magazines".into(),
                extensions: Vec::new(),
                destination: "/data/mags".into(),
                sites: vec!["*.mag.example".into()],
            },
            CategoryRule {
                name: "Ebooks".into(),
                extensions: vec!["epub".into(), ".PDF".into()],
                destination: String::new(),
                sites: Vec::new(),
            },
        ]
    }

    #[test]
    fn site_rule_beats_extension_rule() {
        let rules = rules();
        assert_eq!(
            classify_with(&rules, "file.zip", Some("www.mag.example"), None),
            "Magazines"
        );
        assert_eq!(
            classify_with(&rules, "file.zip", Some("mag.example"), None),
            "Magazines"
        );
        assert_eq!(
            classify_with(&rules, "file.zip", Some("other.example"), None),
            "Compressed"
        );
    }

    #[test]
    fn user_extension_rule_beats_builtin() {
        let rules = rules();
        assert_eq!(
            classify_with(&rules, "book.epub", Some("a.test"), None),
            "Ebooks"
        );
        // A rule extension can override a built-in class for the same name.
        assert_eq!(
            classify_with(&rules, "paper.pdf", Some("a.test"), None),
            "Ebooks"
        );
        assert_eq!(
            classify_with(&rules, "song.mp3", Some("a.test"), None),
            "Music"
        );
        // No host: site patterns never match.
        assert_eq!(classify_with(&rules, "paper.pdf", None, None), "Ebooks");
        assert_eq!(
            classify_with(&rules, "file.zip", Some("www.mag.example"), None),
            "Magazines"
        );
    }

    #[test]
    fn host_globs_match_wildcards_only() {
        assert!(host_matches("*.example.com", "www.example.com"));
        assert!(host_matches("*.example.com", "example.com"));
        assert!(!host_matches("*.example.com", "badexample.com"));
        assert!(host_matches("db?.local", "db1.local"));
        assert!(!host_matches("db?.local", "db10.local"));
        assert!(host_matches("a*b.c", "axxyb.c"));
        assert!(!host_matches("*.example.com", "EXAMPLE.ORG"));
    }

    #[test]
    fn rules_validate_bounds() {
        let mut rule = CategoryRule {
            name: " Ok ".into(),
            extensions: Vec::new(),
            destination: String::new(),
            sites: Vec::new(),
        };
        assert!(valid_rule(&rule).is_err());
        rule.name = "Docs".into();
        rule.extensions = vec!["pdf".into()];
        assert!(valid_rule(&rule).is_ok());
        rule.destination = "relative".into();
        assert!(valid_rule(&rule).is_err());
        rule.destination = "/abs/../evil".into();
        assert!(valid_rule(&rule).is_err());
        rule.destination = "/abs/ok".into();
        rule.sites = vec!["*.a.test".into(), " ".into()];
        assert!(valid_rule(&rule).is_err());
        rule.sites = vec!["*.a.test".into()];
        assert!(valid_rule(&rule).is_ok());
        assert_eq!(
            serde_json::to_value(&rule).unwrap(),
            serde_json::json!({"name": "Docs", "extensions": ["pdf"], "destination": "/abs/ok", "sites": ["*.a.test"]})
        );
    }

    #[test]
    fn destination_lookup_uses_matching_rule() {
        let rules = rules();
        let category = classify_with(&rules, "f.zip", Some("www.mag.example"), None);
        assert_eq!(destination_for(&rules, &category), Some("/data/mags"));
        assert_eq!(destination_for(&rules, "Ebooks"), None);
    }

    #[test]
    fn builtin_table_stays_in_sync_with_classify() {
        for (category, extensions) in BUILTIN_EXTENSIONS {
            for extension in extensions {
                assert_eq!(classify(&format!("x.{extension}"), None), category);
            }
            assert_eq!(builtin_extensions(category), extensions.iter().map(|ext| ext.to_string()).collect::<Vec<_>>());
        }
    }
}
