use url::Url;

/// Checks if a given text contains a valid http, https, or ftp URL whose path extension
/// matches one of the allowed extensions. Returns the cleaned URL if matched.
/// NEVER logs or stores the raw clipboard contents.
pub fn extract_matching_url(text: &str, allowed_extensions: &[String]) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    if !(trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("ftp://"))
    {
        return None;
    }

    let parsed = Url::parse(trimmed).ok()?;
    let path = parsed.path();
    let ext = path
        .rsplit('/')
        .next()?
        .rsplit('.')
        .next()?
        .trim()
        .to_ascii_lowercase();

    if ext.is_empty() || ext == path {
        return None;
    }

    let is_allowed = allowed_extensions.iter().any(|allowed| {
        let clean = allowed.trim().trim_start_matches('.').to_ascii_lowercase();
        !clean.is_empty() && clean == ext
    });

    if is_allowed {
        Some(trimmed.to_string())
    } else {
        None
    }
}

/// Reads current system clipboard text via standard system tools without a shell.
/// Uses xclip/wl-paste on Linux and PowerShell Get-Clipboard on Windows.
/// Never logs or outputs clipboard contents.
pub fn read_clipboard_system() -> Option<String> {
    #[cfg(windows)]
    {
        let output = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", "Get-Clipboard"])
            .output()
            .ok()?;
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !text.is_empty() {
                return Some(text);
            }
        }
        None
    }
    #[cfg(not(windows))]
    {
        let commands: &[(&str, &[&str])] = &[
            ("xclip", &["-selection", "clipboard", "-o"]),
            ("xclip", &["-o"]),
            ("wl-paste", &["--no-newline"]),
            ("wl-paste", &[]),
        ];

        for (cmd, args) in commands {
            if let Ok(output) = std::process::Command::new(cmd).args(*args).output() {
                if output.status.success() {
                    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !text.is_empty() {
                        return Some(text);
                    }
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_extensions() -> Vec<String> {
        vec![
            "zip".into(),
            "rar".into(),
            "7z".into(),
            "exe".into(),
            "msi".into(),
            "iso".into(),
            "mp3".into(),
            "mp4".into(),
            "mkv".into(),
            "pdf".into(),
        ]
    }

    #[test]
    fn test_extract_matching_url_success() {
        let exts = test_extensions();
        assert_eq!(
            extract_matching_url("https://example.com/downloads/archive.zip", &exts),
            Some("https://example.com/downloads/archive.zip".into())
        );
        assert_eq!(
            extract_matching_url("http://mirrors.kernel.org/dist/image.iso", &exts),
            Some("http://mirrors.kernel.org/dist/image.iso".into())
        );
        assert_eq!(
            extract_matching_url("ftp://ftp.example.com/pub/setup.exe", &exts),
            Some("ftp://ftp.example.com/pub/setup.exe".into())
        );
    }

    #[test]
    fn test_extract_matching_url_case_insensitivity() {
        let exts = test_extensions();
        assert_eq!(
            extract_matching_url("https://example.com/archive.ZIP", &exts),
            Some("https://example.com/archive.ZIP".into())
        );
        assert_eq!(
            extract_matching_url("https://example.com/movie.MKV", &exts),
            Some("https://example.com/movie.MKV".into())
        );
    }

    #[test]
    fn test_extract_matching_url_with_query_and_fragment() {
        let exts = test_extensions();
        assert_eq!(
            extract_matching_url("https://example.com/doc.pdf?auth=token#page=2", &exts),
            Some("https://example.com/doc.pdf?auth=token#page=2".into())
        );
    }

    #[test]
    fn test_extract_matching_url_unsupported_extensions() {
        let exts = test_extensions();
        assert_eq!(
            extract_matching_url("https://example.com/index.html", &exts),
            None
        );
        assert_eq!(
            extract_matching_url("https://example.com/script.js", &exts),
            None
        );
        assert_eq!(
            extract_matching_url("https://example.com/style.css", &exts),
            None
        );
    }

    #[test]
    fn test_extract_matching_url_no_extension() {
        let exts = test_extensions();
        assert_eq!(extract_matching_url("https://example.com/resource", &exts), None);
        assert_eq!(extract_matching_url("https://example.com/", &exts), None);
    }

    #[test]
    fn test_extract_matching_url_invalid_schemes_and_plain_text() {
        let exts = test_extensions();
        assert_eq!(
            extract_matching_url("file:///home/user/test.zip", &exts),
            None
        );
        assert_eq!(
            extract_matching_url("ssh://server/path/test.zip", &exts),
            None
        );
        assert_eq!(
            extract_matching_url("just some clipboard text with https://example.com/a.zip", &exts),
            None
        );
        assert_eq!(extract_matching_url("", &exts), None);
    }
}
