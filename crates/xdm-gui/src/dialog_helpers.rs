use xdm_categories::CategoryRule;
use xdm_grabber::Project;
use xdm_ipc::{ImportEntry, ProxyChoice};

// --- Batch Download Helpers ---

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchPreview {
    pub count: usize,
    pub first: String,
    pub last: String,
    pub valid: bool,
    pub message: String,
}

pub fn preview_batch(template: &str) -> BatchPreview {
    let trimmed = template.trim();
    if trimmed.is_empty() {
        return BatchPreview {
            count: 0,
            first: "First: —".into(),
            last: "Last: —".into(),
            valid: false,
            message: "Enter a URL template with ranges like [1-10]".into(),
        };
    }
    match xdm_transfer_list::expand_batch(trimmed, 10_000) {
        Ok(urls) if !urls.is_empty() => {
            let count = urls.len();
            let first = urls.first().cloned().unwrap_or_default();
            let last = urls.last().cloned().unwrap_or_default();
            BatchPreview {
                count,
                first: format!("First: {first}"),
                last: format!("Last: {last}"),
                valid: true,
                message: format!("Valid template: {count} URL(s) generated"),
            }
        }
        Ok(_) => BatchPreview {
            count: 0,
            first: "First: —".into(),
            last: "Last: —".into(),
            valid: false,
            message: "Template produced 0 URLs".into(),
        },
        Err(err) => BatchPreview {
            count: 0,
            first: "First: —".into(),
            last: "Last: —".into(),
            valid: false,
            message: format!("Invalid template: {err}"),
        },
    }
}

pub fn build_batch_entries(template: &str) -> Result<Vec<ImportEntry>, String> {
    let trimmed = template.trim();
    let urls = xdm_transfer_list::expand_batch(trimmed, 10_000)?;
    if urls.is_empty() {
        return Err("No URLs produced by template".into());
    }
    let list = xdm_transfer_list::parse(&urls.join("\n"))?;
    Ok(list.entries)
}

// --- Site Grabber Wizard Helpers ---

pub fn build_grabber_project(
    start_url: &str,
    depth_str: &str,
    max_pages_str: &str,
    file_types_str: &str,
    include_str: &str,
    exclude_str: &str,
    same_host: bool,
    obey_robots: bool,
) -> Result<Project, String> {
    let url = start_url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("Start URL must begin with http:// or https://".into());
    }
    let max_depth = depth_str
        .trim()
        .parse::<u32>()
        .map_err(|_| "Invalid crawl depth: must be a positive integer")?;
    let max_pages = max_pages_str
        .trim()
        .parse::<usize>()
        .map_err(|_| "Invalid max pages: must be a positive integer")?;
    let file_types = file_types_str
        .split([',', ' ', ';'])
        .map(|s| s.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    let include_patterns = include_str
        .split([',', ';'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let exclude_patterns = exclude_str
        .split([',', ';'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    Ok(Project {
        start_url: url.to_string(),
        max_depth,
        max_pages,
        same_host_only: same_host,
        allowed_hosts: Vec::new(),
        include_patterns,
        exclude_patterns,
        file_types,
        headers: Vec::new(),
        request_delay_ms: 0,
        max_file_count: 1000,
        obey_robots,
    })
}

// --- Categories Editor Helpers ---

pub fn parse_extensions(input: &str) -> Vec<String> {
    input
        .split([',', ' ', ';'])
        .map(|s| s.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn parse_sites(input: &str) -> Vec<String> {
    input
        .split([',', ';'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn format_category_rule(rule: &CategoryRule) -> String {
    let exts = if rule.extensions.is_empty() {
        "—".to_string()
    } else {
        rule.extensions.join(", ")
    };
    let dest = if rule.destination.is_empty() {
        "default".to_string()
    } else {
        rule.destination.clone()
    };
    let sites = if rule.sites.is_empty() {
        "—".to_string()
    } else {
        rule.sites.join(", ")
    };
    format!(
        "{:<12}  ·  ext: [{}]  ·  dest: {}  ·  sites: [{}]",
        rule.name, exts, dest, sites
    )
}

// --- Connection / Proxy Settings Helpers ---

pub fn parse_bypass(input: &str) -> Vec<String> {
    input
        .split([',', ';'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn build_proxy_choice(
    mode: &str,
    http: &str,
    https: &str,
    bypass: &str,
) -> Result<ProxyChoice, String> {
    match mode {
        "system" => Ok(ProxyChoice::System),
        "direct" => Ok(ProxyChoice::Direct),
        "manual" => {
            let http_opt = if http.trim().is_empty() {
                None
            } else {
                Some(http.trim().to_string())
            };
            let https_opt = if https.trim().is_empty() {
                None
            } else {
                Some(https.trim().to_string())
            };
            if http_opt.is_none() && https_opt.is_none() {
                return Err("Specify at least an HTTP or HTTPS proxy URL for manual proxy".into());
            }
            let bypass_vec = parse_bypass(bypass);
            Ok(ProxyChoice::Manual {
                http: http_opt,
                https: https_opt,
                bypass: bypass_vec,
            })
        }
        _ => Err(format!("Unknown proxy mode: {mode}")),
    }
}

// --- URL Validation Helpers ---

pub fn validate_download_url(url: &str) -> Result<String, String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err("URL cannot be empty".into());
    }
    if !(trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("ftp://"))
    {
        return Err("URL must start with http://, https://, or ftp://".into());
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_batch_preview_success() {
        let preview = preview_batch("http://example.com/item[1-3].bin");
        assert!(preview.valid);
        assert_eq!(preview.count, 3);
        assert_eq!(preview.first, "First: http://example.com/item1.bin");
        assert_eq!(preview.last, "Last: http://example.com/item3.bin");
    }

    #[test]
    fn test_batch_preview_invalid() {
        let preview = preview_batch("http://example.com/item[1-a].bin");
        assert!(!preview.valid);
        assert_eq!(preview.count, 0);
    }

    #[test]
    fn test_build_batch_entries() {
        let entries = build_batch_entries("http://example.com/part[01-03].zip").unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].url, "http://example.com/part01.zip");
        assert_eq!(entries[2].url, "http://example.com/part03.zip");
    }

    #[test]
    fn test_grabber_project_creation() {
        let proj = build_grabber_project(
            "https://mysite.com",
            "3",
            "50",
            "zip, pdf",
            "*download*",
            "*thumb*",
            true,
            false,
        )
        .unwrap();
        assert_eq!(proj.start_url, "https://mysite.com");
        assert_eq!(proj.max_depth, 3);
        assert_eq!(proj.max_pages, 50);
        assert_eq!(proj.file_types, vec!["zip", "pdf"]);
        assert!(proj.same_host_only);
        assert!(!proj.obey_robots);
    }

    #[test]
    fn test_category_parsing() {
        let exts = parse_extensions(".tar, zip; gz   7z");
        assert_eq!(exts, vec!["tar", "zip", "gz", "7z"]);

        let sites = parse_sites("*.test.com, sub.example.org");
        assert_eq!(sites, vec!["*.test.com", "sub.example.org"]);
    }

    #[test]
    fn test_proxy_choice_modes() {
        assert_eq!(
            build_proxy_choice("system", "", "", "").unwrap(),
            ProxyChoice::System
        );
        assert_eq!(
            build_proxy_choice("direct", "", "", "").unwrap(),
            ProxyChoice::Direct
        );
        let manual = build_proxy_choice(
            "manual",
            "http://127.0.0.1:8080",
            "",
            "localhost, 127.0.0.1",
        )
        .unwrap();
        match manual {
            ProxyChoice::Manual {
                http,
                https,
                bypass,
            } => {
                assert_eq!(http, Some("http://127.0.0.1:8080".into()));
                assert_eq!(https, None);
                assert_eq!(bypass, vec!["localhost", "127.0.0.1"]);
            }
            _ => panic!("Expected manual proxy"),
        }
    }

    #[test]
    fn test_validate_download_url() {
        assert!(validate_download_url("http://example.com/file").is_ok());
        assert!(validate_download_url("https://example.com/file").is_ok());
        assert!(validate_download_url("ftp://example.com/file").is_ok());
        assert!(validate_download_url("ssh://example.com/file").is_err());
        assert!(validate_download_url("").is_err());
    }
}
