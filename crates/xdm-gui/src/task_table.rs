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

pub fn format_status(status: &str, downloaded: Option<u64>, total: Option<u64>) -> String {
    let progress = match (downloaded, total) {
        (Some(done), Some(tot)) if tot > 0 => (done.saturating_mul(100) / tot).min(100),
        _ => 0,
    };

    if status == "downloading" || status == "active" {
        format!("Downloading {progress}%")
    } else if status == "completed" {
        "Complete (100%)".to_string()
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
    if status == "completed" {
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

pub fn move_file(source: &Path, dest: &Path) -> Result<(), String> {
    if !source.exists() {
        return Err(format!("Source file does not exist: {}", source.display()));
    }
    if dest.exists() {
        return Err(format!(
            "Destination file already exists: {}",
            dest.display()
        ));
    }
    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::rename(source, dest) {
        Ok(_) => Ok(()),
        Err(_) => {
            std::fs::copy(source, dest)
                .and_then(|_| std::fs::remove_file(source))
                .map_err(|e| format!("Move failed: {e}"))
        }
    }
}

pub fn sort_tasks(
    tasks: &mut [xdm_ipc::TaskSummary],
    sort_column: &str,
    sort_asc: bool,
    queue_map: &std::collections::HashMap<i64, String>,
) {
    tasks.sort_by(|a, b| {
        let cmp = match sort_column {
            "filename" => {
                let fa = format_filename(&a.output, &a.url).to_lowercase();
                let fb = format_filename(&b.output, &b.url).to_lowercase();
                fa.cmp(&fb)
            }
            "size" => {
                let sa = a.total.or(a.downloaded).unwrap_or(0);
                let sb = b.total.or(b.downloaded).unwrap_or(0);
                sa.cmp(&sb)
            }
            "status" => a.status.cmp(&b.status),
            "time_left" => {
                let ea = a.eta_seconds.unwrap_or(u64::MAX);
                let eb = b.eta_seconds.unwrap_or(u64::MAX);
                ea.cmp(&eb)
            }
            "transfer_rate" => {
                let ra = a.bytes_per_second.unwrap_or(0);
                let rb = b.bytes_per_second.unwrap_or(0);
                ra.cmp(&rb)
            }
            "last_try_date" => {
                let da = if !a.updated_at.is_empty() {
                    &a.updated_at
                } else {
                    &a.created_at
                };
                let db = if !b.updated_at.is_empty() {
                    &b.updated_at
                } else {
                    &b.created_at
                };
                da.cmp(db)
            }
            "description" => {
                let da = if !a.output.is_empty() {
                    &a.output
                } else {
                    &a.url
                };
                let db = if !b.output.is_empty() {
                    &b.output
                } else {
                    &b.url
                };
                da.cmp(db)
            }
            "category" => a.category.cmp(&b.category),
            "queue" => {
                let qa = queue_map
                    .get(&a.queue_id)
                    .map(|s| s.as_str())
                    .unwrap_or("");
                let qb = queue_map
                    .get(&b.queue_id)
                    .map(|s| s.as_str())
                    .unwrap_or("");
                qa.cmp(qb)
            }
            _ => a.id.cmp(&b.id),
        };
        if sort_asc {
            cmp
        } else {
            cmp.reverse()
        }
    });
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
    fn test_move_file_no_clobber_rule1() {
        let dir = std::env::temp_dir().join(format!("xdm-test-move-noclobber-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let src = dir.join("src.bin");
        let dst = dir.join("dst.bin");
        std::fs::write(&src, b"source data").unwrap();
        std::fs::write(&dst, b"destination existing data").unwrap();

        let res = move_file(&src, &dst);
        assert!(res.is_err(), "Move must never overwrite existing file (Rule 1)");
        assert_eq!(std::fs::read(&src).unwrap(), b"source data");
        assert_eq!(std::fs::read(&dst).unwrap(), b"destination existing data");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_move_file_success() {
        let dir = std::env::temp_dir().join(format!("xdm-test-move-success-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let src = dir.join("src.bin");
        let dst = dir.join("dst.bin");
        std::fs::write(&src, b"data to move").unwrap();

        let res = move_file(&src, &dst);
        assert!(res.is_ok());
        assert!(!src.exists());
        assert_eq!(std::fs::read(&dst).unwrap(), b"data to move");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_sort_tasks() {
        let t1 = xdm_ipc::TaskSummary {
            id: 1,
            url: "https://example.com/b_zebra.zip".into(),
            output: "/downloads/b_zebra.zip".into(),
            status: "downloading".into(),
            priority: 1,
            error: None,
            downloaded: Some(100),
            total: Some(1000),
            kind: "http".into(),
            category: "Compressed".into(),
            rate_limit: None,
            bytes_per_second: Some(500),
            eta_seconds: Some(10),
            connections: 4,
            created_at: "2026-01-01 10:00".into(),
            updated_at: "2026-01-01 10:05".into(),
            media_video: None,
            media_audio: None,
            queue_id: 1,
            history_queue_id: None,
            retry_count: 0,
            retry_at: None,
            position: 1,
            auth_realm: None,
            media_seconds: None,
            restart_reason: None,
        };
        let t2 = xdm_ipc::TaskSummary {
            id: 2,
            url: "https://example.com/a_apple.zip".into(),
            output: "/downloads/a_apple.zip".into(),
            status: "completed".into(),
            priority: 1,
            error: None,
            downloaded: Some(5000),
            total: Some(5000),
            kind: "http".into(),
            category: "Archives".into(),
            rate_limit: None,
            bytes_per_second: Some(2000),
            eta_seconds: Some(0),
            connections: 4,
            created_at: "2026-01-01 11:00".into(),
            updated_at: "2026-01-01 11:10".into(),
            media_video: None,
            media_audio: None,
            queue_id: 2,
            history_queue_id: None,
            retry_count: 0,
            retry_at: None,
            position: 2,
            auth_realm: None,
            media_seconds: None,
            restart_reason: None,
        };
        let mut queue_map = std::collections::HashMap::new();
        queue_map.insert(1, "Main".into());
        queue_map.insert(2, "Secondary".into());

        let mut tasks = vec![t1.clone(), t2.clone()];
        // Sort by filename ascending: a_apple before b_zebra
        sort_tasks(&mut tasks, "filename", true, &queue_map);
        assert_eq!(tasks[0].id, 2);
        assert_eq!(tasks[1].id, 1);

        // Sort by filename descending: b_zebra before a_apple
        sort_tasks(&mut tasks, "filename", false, &queue_map);
        assert_eq!(tasks[0].id, 1);
        assert_eq!(tasks[1].id, 2);

        // Sort by size ascending: 1000 before 5000
        sort_tasks(&mut tasks, "size", true, &queue_map);
        assert_eq!(tasks[0].id, 1);
        assert_eq!(tasks[1].id, 2);

        // Sort by size descending: 5000 before 1000
        sort_tasks(&mut tasks, "size", false, &queue_map);
        assert_eq!(tasks[0].id, 2);
        assert_eq!(tasks[1].id, 1);

        // Sort by transfer_rate ascending: 500 before 2000
        sort_tasks(&mut tasks, "transfer_rate", true, &queue_map);
        assert_eq!(tasks[0].id, 1);
        assert_eq!(tasks[1].id, 2);

        // Sort by queue ascending: "Main" before "Secondary"
        sort_tasks(&mut tasks, "queue", true, &queue_map);
        assert_eq!(tasks[0].id, 1);
        assert_eq!(tasks[1].id, 2);
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
