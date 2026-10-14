//! Paged task listing, completed-file moves and orphan partial cleanup.
use crate::{Result, Service, query_summaries};
use rusqlite::types::Value;
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use xdm_ipc::{PartialEntry, Reply, TaskFilter};

const MAX_PAGE: u64 = 2000;

fn sibling(output: &Path, suffix: &str) -> PathBuf {
    PathBuf::from(format!("{}{suffix}", output.display()))
}

fn sort_clause(sort: Option<&str>) -> Option<String> {
    let raw = sort.unwrap_or("-id");
    let (descending, name) = match raw.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, raw.strip_prefix('+').unwrap_or(raw)),
    };
    let column = match name {
        "id" => "id",
        "url" => "url COLLATE NOCASE",
        "output" => "output COLLATE NOCASE",
        "status" => "status",
        "category" => "category COLLATE NOCASE",
        "priority" => "priority",
        "position" => "position",
        "created" => "created_at",
        "updated" => "updated_at",
        _ => return None,
    };
    // The id tiebreak keeps pages stable while other columns tie.
    Some(format!(
        "{column} {}, id DESC",
        if descending { "DESC" } else { "ASC" }
    ))
}

fn like_escape(text: &str) -> String {
    let mut out = String::from("%");
    for ch in text.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('%');
    out
}

/// Moves `from` to a destination that must not exist. A hard link publishes
/// atomically on one volume; otherwise the data is copied to a sibling
/// temporary file, synced, and published without ever replacing a file.
fn move_no_overwrite(from: &Path, to: &Path) -> io::Result<()> {
    if std::fs::symlink_metadata(to).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "destination already exists",
        ));
    }
    match std::fs::hard_link(from, to) {
        Ok(()) => return std::fs::remove_file(from),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Err(error),
        Err(_) => {}
    }
    let temp = sibling(to, ".xdm-move.tmp");
    let copied = (|| {
        let mut source = std::fs::File::open(from)?;
        let mut target = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        io::copy(&mut source, &mut target)?;
        target.sync_all()
    })();
    if let Err(error) = copied {
        let _ = std::fs::remove_file(&temp);
        return Err(error);
    }
    let published = match std::fs::hard_link(&temp, to) {
        Ok(()) => std::fs::remove_file(&temp),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Err(error),
        // No hard links on this filesystem: checked rename, still no overwrite.
        Err(_) if std::fs::symlink_metadata(to).is_err() => std::fs::rename(&temp, to),
        Err(error) => Err(error),
    };
    if let Err(error) = published {
        let _ = std::fs::remove_file(&temp);
        return Err(error);
    }
    std::fs::remove_file(from)
}

fn regular_size(path: &Path) -> Option<u64> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    meta.file_type().is_file().then(|| meta.len())
}

impl Service {
    pub(crate) fn list_page(
        &self,
        offset: u64,
        limit: u64,
        sort: Option<String>,
        filter: Option<TaskFilter>,
    ) -> Result<Reply> {
        if limit == 0 || limit > MAX_PAGE {
            return Ok(Reply::error("limit must be 1..=2000"));
        }
        let Some(order) = sort_clause(sort.as_deref()) else {
            return Ok(Reply::error("unknown sort column"));
        };
        let filter = filter.unwrap_or_default();
        let mut clauses = Vec::new();
        let mut values: Vec<Value> = Vec::new();
        if let Some(status) = filter.status {
            clauses.push("status=?");
            values.push(Value::Text(status));
        }
        if let Some(category) = filter.category {
            clauses.push("category=? COLLATE NOCASE");
            values.push(Value::Text(category));
        }
        if let Some(queue) = filter.queue_id {
            clauses.push("queue_id=?");
            values.push(Value::Integer(queue));
        }
        if let Some(search) = filter.search.filter(|text| !text.is_empty()) {
            clauses.push("(url LIKE ? ESCAPE '\\' OR output LIKE ? ESCAPE '\\')");
            values.push(Value::Text(like_escape(&search)));
            values.push(Value::Text(like_escape(&search)));
        }
        let wher = if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" AND "))
        };
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        let total: i64 = inner.db.query_row(
            &format!("SELECT COUNT(*) FROM tasks {wher}"),
            rusqlite::params_from_iter(values.iter()),
            |row| row.get(0),
        )?;
        values.push(Value::Integer(limit as i64));
        values.push(Value::Integer(offset.min(i64::MAX as u64) as i64));
        let tasks = query_summaries(
            &inner,
            &format!("{wher} ORDER BY {order} LIMIT ? OFFSET ?"),
            rusqlite::params_from_iter(values.iter()),
        )?;
        let mut reply = Reply::list(tasks);
        reply.total = total as u64;
        reply.daemon_version = Some(env!("CARGO_PKG_VERSION").to_owned());
        Ok(reply)
    }

    pub(crate) fn move_task_file(&self, task_id: i64, new_path: String) -> Result<Reply> {
        let to = PathBuf::from(&new_path);
        if !to.is_absolute()
            || to
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
            || to.file_name().is_none()
        {
            return Ok(Reply::error("new_path must be an absolute path without .."));
        }
        let from = {
            let inner = self
                .inner
                .lock()
                .map_err(|_| io::Error::other("task lock poisoned"))?;
            if inner.active.contains_key(&task_id) {
                return Ok(Reply::error("task is running"));
            }
            let row: Option<(String, String)> = rusqlite::OptionalExtension::optional(
                inner.db.query_row(
                    "SELECT output,status FROM tasks WHERE id=?1",
                    [task_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                ),
            )?;
            let Some((output, status)) = row else {
                return Ok(Reply::error("task not found"));
            };
            if status != "complete" {
                return Ok(Reply::error("only completed tasks can be moved"));
            }
            let taken: bool = inner.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM tasks WHERE output=?1)",
                [&new_path],
                |row| row.get(0),
            )?;
            if taken {
                return Ok(Reply::error("another task already owns that path"));
            }
            PathBuf::from(output)
        };
        if from == to {
            return Ok(Reply::error("destination is the current path"));
        }
        if !to.parent().is_some_and(Path::is_dir) {
            return Ok(Reply::error("destination directory does not exist"));
        }
        if regular_size(&from).is_none() {
            return Ok(Reply::error("completed file is missing"));
        }
        // Disk work runs without the daemon lock (rule 6).
        if let Err(error) = move_no_overwrite(&from, &to) {
            return Ok(Reply::error(format!("move failed: {error}")));
        }
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        let updated = inner.db.execute(
            "UPDATE tasks SET output=?1, updated_at=CURRENT_TIMESTAMP WHERE id=?2 AND status='complete' AND output=?3",
            rusqlite::params![new_path, task_id, from.to_string_lossy()],
        );
        if !matches!(updated, Ok(1)) {
            // The record did not change, so put the file back.
            let _ = move_no_overwrite(&to, &from);
            return Ok(Reply::error("task changed during the move; file restored"));
        }
        Ok(Reply::accepted(task_id))
    }

    pub(crate) fn remove_task(&self, task_id: i64, delete_partial: bool) -> Result<Reply> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        if inner.active.contains_key(&task_id) {
            return Ok(Reply::error(
                "pause the task and wait for it to stop before removing it",
            ));
        }
        let row: Option<(String, String)> = rusqlite::OptionalExtension::optional(
            inner.db.query_row(
                "SELECT output,status FROM tasks WHERE id=?1",
                [task_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ),
        )?;
        let transaction = inner.db.transaction()?;
        let removed = transaction.execute(
            "INSERT INTO removed_tasks(task_id,request_id) SELECT id,request_id FROM tasks WHERE id=?1",
            [task_id],
        )?;
        if removed == 0 {
            return Ok(Reply::error("task not found"));
        }
        transaction.execute("DELETE FROM tasks WHERE id=?1", [task_id])?;
        transaction.commit()?;
        inner.queue_interrupted.remove(&task_id);
        drop(inner);
        if let (true, Some((output, status))) = (delete_partial, row) {
            if status != "complete" {
                let output = PathBuf::from(output);
                for suffix in [".part", ".part.meta"] {
                    let path = sibling(&output, suffix);
                    if regular_size(&path).is_some() {
                        let _ = std::fs::remove_file(path);
                    }
                }
            }
        }
        Ok(Reply::accepted(task_id))
    }

    fn owned_outputs(&self, directory: &Path) -> Result<HashSet<PathBuf>> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        let mut statement = inner.db.prepare("SELECT output FROM tasks")?;
        let mut rows = statement.query([])?;
        let mut owned = HashSet::new();
        while let Some(row) = rows.next()? {
            let output = PathBuf::from(row.get::<_, String>(0)?);
            let (Some(parent), Some(name)) = (output.parent(), output.file_name()) else {
                continue;
            };
            if parent == directory || std::fs::canonicalize(parent).ok().as_deref() == Some(directory)
            {
                owned.insert(directory.join(name));
            }
        }
        Ok(owned)
    }

    /// Unowned partial pairs directly in `directory`; symlinks and
    /// non-regular files are never listed.
    pub(crate) fn scan_partials(&self, directory: &str) -> Result<Result_<Vec<PartialEntry>>> {
        let directory = Path::new(directory);
        if !directory.is_absolute() {
            return Ok(Err("directory must be absolute".into()));
        }
        let Ok(directory) = std::fs::canonicalize(directory) else {
            return Ok(Err("directory does not exist".into()));
        };
        let owned = self.owned_outputs(&directory)?;
        let mut pairs: std::collections::BTreeMap<PathBuf, PartialEntry> = Default::default();
        for entry in std::fs::read_dir(&directory)?.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let (base, is_meta) = if let Some(base) = name.strip_suffix(".part.meta") {
                (base, true)
            } else if let Some(base) = name.strip_suffix(".part") {
                (base, false)
            } else {
                continue;
            };
            if base.is_empty() {
                continue;
            }
            let output = directory.join(base);
            if owned.contains(&output) {
                continue;
            }
            let Some(size) = regular_size(&path) else {
                continue;
            };
            let slot = pairs.entry(output.clone()).or_insert_with(|| PartialEntry {
                output: output.to_string_lossy().into_owned(),
                ..PartialEntry::default()
            });
            if is_meta {
                slot.meta = Some(path.to_string_lossy().into_owned());
                slot.meta_size = size;
            } else {
                slot.part = Some(path.to_string_lossy().into_owned());
                slot.part_size = size;
            }
        }
        Ok(Ok(pairs.into_values().collect()))
    }

    pub(crate) fn delete_partials(&self, directory: &str, paths: Vec<String>) -> Result<Reply> {
        let found = match self.scan_partials(directory)? {
            Ok(found) => found,
            Err(message) => return Ok(Reply::error(message)),
        };
        let eligible: HashSet<&str> = found
            .iter()
            .flat_map(|entry| [entry.part.as_deref(), entry.meta.as_deref()])
            .flatten()
            .collect();
        if let Some(bad) = paths.iter().find(|path| !eligible.contains(path.as_str())) {
            return Ok(Reply::error(format!(
                "not an unowned partial in the scan: {bad}"
            )));
        }
        let mut deleted = 0;
        for path in &paths {
            if regular_size(Path::new(path)).is_some() && std::fs::remove_file(path).is_ok() {
                deleted += 1;
            }
        }
        let mut reply = Reply::list(Vec::new());
        reply.total = deleted;
        Ok(reply)
    }
}

type Result_<T> = std::result::Result<T, String>;
