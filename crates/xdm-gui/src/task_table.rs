use std::path::Path;

pub fn format_filename(output: &str, url: &str) -> String {
    let p = Path::new(output);
    if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
        if !name.is_empty() {
            return name.to_string();
        }
    }
    if let Ok(parsed) = url::Url::parse(url) {
        if let Some(segments) = parsed.path_segments() {
            if let Some(last) = segments.last() {
                if !last.is_empty() {
                    return last.to_string();
                }
            }
        }
    }
    output.to_string()
}

pub fn format_size(downloaded: Option<u64>, total: Option<u64>) -> String {
    fn fmt(bytes: u64) -> String {
        if bytes < 1024 {
            format!("{bytes} B")
        } else if bytes < 1024 * 1024 {
            format!("{:.1} KB", bytes as f64 / 1024.0)
        } else if bytes < 1024 * 1024 * 1024 {
            format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
        } else {
            format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
        }
    }

    match (downloaded, total) {
        (_, Some(tot)) if tot > 0 => fmt(tot),
        (Some(done), None) if done > 0 => format!("{} / Unknown", fmt(done)),
        _ => "—".to_string(),
    }
}

/// The daemon stores finished tasks as `complete`; older GUI code and fixtures say `completed`.
pub fn is_complete(status: &str) -> bool {
    status == "complete" || status == "completed"
}

pub fn format_status(status: &str, downloaded: Option<u64>, total: Option<u64>) -> String {
    let progress = match (downloaded, total) {
        (Some(done), Some(tot)) if tot > 0 => (done.saturating_mul(100) / tot).min(100),
        _ => 0,
    };

    if status == "downloading" || status == "active" {
        format!("Downloading {progress}%")
    } else if is_complete(status) {
        "Complete (100%)".to_string()
    } else if status == "quota_wait" {
        "Quota wait".to_string()
    } else if status == "restart_required" {
        "Needs restart".to_string()
    } else if status == "paused" {
        format!("Paused {progress}%")
    } else if status == "failed" {
        "Failed".to_string()
    } else if status == "queued" {
        "Queued".to_string()
    } else {
        format!("{status} {progress}%")
    }
}

pub fn format_time_left(status: &str, eta_seconds: Option<u64>) -> String {
    if is_complete(status) {
        "0s".to_string()
    } else if let Some(sec) = eta_seconds {
        if sec < 60 {
            format!("{sec}s")
        } else if sec < 3600 {
            format!("{}m {}s", sec / 60, sec % 60)
        } else {
            format!("{}h {}m", sec / 3600, (sec % 3600) / 60)
        }
    } else {
        "—".to_string()
    }
}

pub fn format_rate(rate: Option<u64>) -> String {
    match rate {
        Some(b) if b > 0 => {
            if b < 1024 * 1024 {
                format!("{:.1} KB/s", b as f64 / 1024.0)
            } else {
                format!("{:.2} MB/s", b as f64 / (1024.0 * 1024.0))
            }
        }
        _ => "—".to_string(),
    }
}

pub fn format_date(updated_at: &str, created_at: &str) -> String {
    let d = if !updated_at.is_empty() {
        updated_at
    } else {
        created_at
    };
    if d.is_empty() {
        "—".to_string()
    } else {
        d.to_string()
    }
}

/// Program and arguments used to open `path` (never goes through a shell).
pub fn open_file_command(path: &str) -> (String, Vec<String>) {
    if cfg!(target_os = "windows") {
        ("explorer".into(), vec![path.to_string()])
    } else if cfg!(target_os = "macos") {
        ("open".into(), vec![path.to_string()])
    } else {
        ("xdg-open".into(), vec![path.to_string()])
    }
}

/// Program and arguments used to reveal `path` in the platform file manager.
pub fn open_folder_command(path: &str) -> (String, Vec<String>) {
    if cfg!(target_os = "windows") {
        ("explorer".into(), vec![format!("/select,{path}")])
    } else if cfg!(target_os = "macos") {
        ("open".into(), vec!["-R".into(), path.to_string()])
    } else {
        let parent = Path::new(path)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        ("xdg-open".into(), vec![parent.to_string_lossy().into_owned()])
    }
}

/// Spawns the opener. With `XDM_FAKE_OS_ACTIONS=1` (tests) the command is
/// appended to `$XDM_DATA_DIR/gui-open.log` instead of being executed.
fn run_opener((program, args): (String, Vec<String>)) {
    if std::env::var("XDM_FAKE_OS_ACTIONS").as_deref() == Ok("1") {
        if let Ok(dir) = std::env::var("XDM_DATA_DIR") {
            use std::io::Write;
            if let Ok(mut log) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(Path::new(&dir).join("gui-open.log"))
            {
                let _ = writeln!(log, "{program}\t{}", args.join("\t"));
            }
        }
        return;
    }
    let _ = std::process::Command::new(program).args(args).spawn();
}

pub fn open_file(path: &str) {
    run_opener(open_file_command(path));
}

pub fn open_folder(path: &str) {
    run_opener(open_folder_command(path));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_filename() {
        assert_eq!(
            format_filename("/home/user/Downloads/archive.zip", "https://example.com/dl"),
            "archive.zip"
        );
        assert_eq!(
            format_filename("", "https://example.com/files/document.pdf"),
            "document.pdf"
        );
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(None, None), "—");
        assert_eq!(format_size(Some(500), Some(500)), "500 B");
        assert_eq!(format_size(Some(2048), Some(2048)), "2.0 KB");
        assert_eq!(format_size(Some(5 * 1024 * 1024), Some(5 * 1024 * 1024)), "5.0 MB");
        assert_eq!(
            format_size(Some(3 * 1024 * 1024 * 1024), Some(3 * 1024 * 1024 * 1024)),
            "3.00 GB"
        );
        assert_eq!(format_size(Some(1024 * 1024), None), "1.0 MB / Unknown");
    }

    #[test]
    fn test_format_status() {
        assert_eq!(format_status("completed", Some(100), Some(100)), "Complete (100%)");
        assert_eq!(format_status("downloading", Some(50), Some(100)), "Downloading 50%");
        assert_eq!(format_status("paused", Some(25), Some(100)), "Paused 25%");
        assert_eq!(format_status("failed", None, None), "Failed");
        assert_eq!(format_status("queued", None, None), "Queued");
        assert_eq!(format_status("complete", Some(10), Some(10)), "Complete (100%)");
        assert_eq!(format_status("quota_wait", Some(10), Some(100)), "Quota wait");
        assert_eq!(format_status("restart_required", Some(10), Some(100)), "Needs restart");
    }

    #[test]
    fn test_format_time_left() {
        assert_eq!(format_time_left("completed", None), "0s");
        assert_eq!(format_time_left("downloading", Some(45)), "45s");
        assert_eq!(format_time_left("downloading", Some(125)), "2m 5s");
        assert_eq!(format_time_left("downloading", Some(3665)), "1h 1m");
        assert_eq!(format_time_left("paused", None), "—");
    }

    #[test]
    fn test_format_rate() {
        assert_eq!(format_rate(None), "—");
        assert_eq!(format_rate(Some(0)), "—");
        assert_eq!(format_rate(Some(512 * 1024)), "512.0 KB/s");
        assert_eq!(format_rate(Some(2 * 1024 * 1024)), "2.00 MB/s");
    }

    #[test]
    fn opener_commands_do_not_use_a_shell() {
        let (program, args) = open_file_command("/tmp/a b;rm -rf x.bin");
        assert!(["xdg-open", "open", "explorer"].contains(&program.as_str()));
        assert_eq!(args, vec!["/tmp/a b;rm -rf x.bin".to_string()]);
        let (_, folder_args) = open_folder_command("/tmp/dir/a.bin");
        #[cfg(all(unix, not(target_os = "macos")))]
        assert_eq!(folder_args, vec!["/tmp/dir".to_string()]);
        #[cfg(target_os = "windows")]
        assert_eq!(folder_args, vec!["/select,/tmp/dir/a.bin".to_string()]);
        #[cfg(target_os = "macos")]
        assert_eq!(folder_args, vec!["-R".to_string(), "/tmp/dir/a.bin".to_string()]);
    }
}
