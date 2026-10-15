//! `xdm-cli daemon diagnostics OUT.zip|OUT_DIR`: a support bundle with no
//! secrets (everything passes through `xdm_ipc::ops::redact`; task URLs lose
//! their query strings and userinfo; headers, bodies and logins are omitted).

use rusqlite::{Connection, OpenFlags};
use std::fs;
use std::io;
use std::path::Path;
use xdm_ipc::ops::{self, redact, redact_url};

fn database_report(database: &Path) -> (String, String, String) {
    let mut integrity = String::new();
    let mut settings = String::new();
    let mut tasks = String::new();
    match Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY) {
        Err(e) => integrity.push_str(&format!("cannot open database: {e}\n")),
        Ok(db) => {
            let check: Result<Vec<String>, _> = db
                .prepare("PRAGMA quick_check")
                .and_then(|mut s| s.query_map([], |r| r.get::<_, String>(0))?.collect());
            match check {
                Ok(rows) => integrity.push_str(&format!("quick_check: {}\n", rows.join("; "))),
                Err(e) => integrity.push_str(&format!("quick_check error: {e}\n")),
            }
            let version: Result<i64, _> = db.query_row("PRAGMA user_version", [], |r| r.get(0));
            integrity.push_str(&format!("schema user_version: {version:?}\n"));
            if let Ok(mut stmt) = db.prepare("SELECT key,value FROM settings ORDER BY key") {
                if let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))) {
                    for (key, value) in rows.flatten() {
                        settings.push_str(&redact(&format!("{key}={value}\n")));
                    }
                }
            }
            if let Ok(mut stmt) = db.prepare("SELECT id,status,url FROM tasks ORDER BY id DESC LIMIT 1000") {
                if let Ok(rows) = stmt.query_map([], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
                }) {
                    for (id, status, url) in rows.flatten() {
                        tasks.push_str(&format!("{id}\t{status}\t{}\n", redact_url(&url)));
                    }
                }
            }
        }
    }
    let name = database.file_name().and_then(|n| n.to_str()).unwrap_or("tasks.sqlite3");
    let prefix = format!("{name}.bak-");
    let backups: Vec<String> = fs::read_dir(database.parent().unwrap_or(Path::new(".")))
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| n.starts_with(&prefix))
                .collect()
        })
        .unwrap_or_default();
    integrity.push_str(&format!("backups: {}\n", backups.join(", ")));
    (integrity, settings, tasks)
}

/// Collects (archive name, redacted bytes) for every bundle member.
pub fn collect(dir: &Path, database: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    files.push((
        "version.txt".into(),
        format!(
            "{}\nos: {} {}\ndata dir: {}\n",
            ops::version_line("xdm-cli"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            dir.display()
        )
        .into_bytes(),
    ));
    let (integrity, settings, tasks) = database_report(database);
    files.push(("database.txt".into(), integrity.into_bytes()));
    files.push(("settings.txt".into(), settings.into_bytes()));
    files.push(("tasks.txt".into(), tasks.into_bytes()));
    let mut logs: Vec<_> = fs::read_dir(dir.join("logs")).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    logs.sort();
    logs.extend(ops::list_crash_reports(dir));
    for path in logs {
        let folder = path.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()).unwrap_or("");
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
        if let Ok(bytes) = fs::read(&path) {
            let text = redact(&String::from_utf8_lossy(&bytes));
            files.push((format!("{folder}/{name}"), text.into_bytes()));
        }
    }
    files
}

pub fn write_bundle(dir: &Path, database: &Path, out: &Path) -> io::Result<usize> {
    let files = collect(dir, database);
    if out.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")) {
        let file = fs::OpenOptions::new().write(true).create_new(true).open(out)?;
        let mut zip = ops::ZipWriter::new(io::BufWriter::new(file));
        for (name, data) in &files {
            zip.add(name, data)?;
        }
        zip.finish()?;
    } else {
        if out.exists() && fs::read_dir(out)?.next().is_some() {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, "output directory is not empty"));
        }
        for (name, data) in &files {
            let target = out.join(name);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(target, data)?;
        }
    }
    Ok(files.len())
}
