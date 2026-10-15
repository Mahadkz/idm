//! Database safety: fast integrity check at startup, backup before schema
//! migration, durable pragmas.

use rusqlite::Connection;
use std::path::{Path, PathBuf};

/// Bump whenever a migration is added; `PRAGMA user_version` records it.
pub const SCHEMA_VERSION: i64 = 1;
pub const BACKUPS_KEPT: usize = 3;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn quick_check(db: &Connection) -> Result<()> {
    let rows = db
        .prepare("PRAGMA quick_check")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if rows.len() == 1 && rows[0] == "ok" {
        Ok(())
    } else {
        Err(rows.into_iter().take(5).collect::<Vec<_>>().join("; ").into())
    }
}

pub fn list_backups(database: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(name)) = (database.parent(), database.file_name().and_then(|n| n.to_str())) else {
        return Vec::new();
    };
    let prefix = format!("{name}.bak-");
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&prefix)))
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    found
}

fn backup(db: &Connection, database: &Path) -> Result<PathBuf> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let mut n = 0;
    let target = loop {
        let mut name = database.as_os_str().to_owned();
        name.push(format!(".bak-{now:012}-{n}"));
        let candidate = PathBuf::from(name);
        if !candidate.exists() {
            break candidate;
        }
        n += 1;
    };
    db.execute("VACUUM INTO ?1", [target.to_string_lossy().as_ref()])?;
    let mut all = list_backups(database);
    while all.len() > BACKUPS_KEPT {
        let _ = std::fs::remove_file(all.remove(0));
    }
    Ok(target)
}

/// Runs before any migration. In-memory databases are skipped.
pub fn prepare(db: &Connection, database: &Path) -> Result<()> {
    if database.as_os_str() == ":memory:" {
        return Ok(());
    }
    if let Err(problem) = quick_check(db) {
        let backups = list_backups(database);
        xdm_ipc::ops::log_error(&format!("database integrity check failed: {problem}"));
        return Err(format!(
            "database {} failed its integrity check ({problem}). Refusing to start so no data is lost. Restore the newest backup ({}) over it, or move the file aside to start empty.",
            database.display(),
            backups.last().map_or("none found".to_owned(), |p| p.display().to_string()),
        )
        .into());
    }
    let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let has_data: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='tasks')",
        [],
        |r| r.get(0),
    )?;
    if has_data && version < SCHEMA_VERSION {
        let path = backup(db, database)?;
        xdm_ipc::ops::log_info(&format!("schema v{version} -> v{SCHEMA_VERSION}; backup {}", path.display()));
    }
    Ok(())
}

/// Runs after migrations succeeded.
pub fn finish(db: &Connection, database: &Path) -> Result<()> {
    if database.as_os_str() != ":memory:" {
        db.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }
    Ok(())
}
