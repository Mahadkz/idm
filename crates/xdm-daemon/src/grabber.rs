use crate::categories::{load_rules, url_host};
use crate::{Inner, Result, Service, destination_free, next_task_id, valid_absolute_directory};
use rusqlite::params;
use std::io;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use url::Url;
use xdm_ipc::{GrabProjectRecord, GrabReport, ImportEntry, Reply};

/// A manifest and counters for one background site grab.
pub(crate) struct GrabHandle {
    pub(crate) state: Arc<Mutex<GrabState>>,
}

/// Mutable crawl state shared with the crawl thread.
#[derive(Default)]
pub(crate) struct GrabState {
    pub(crate) state: String,
    pub(crate) files: usize,
    pub(crate) pages: usize,
    pub(crate) errors: usize,
    pub(crate) error: Option<String>,
    pub(crate) task_ids: Vec<i64>,
    pub(crate) skipped: Vec<xdm_ipc::SkippedDuplicate>,
}

/// A crawl registered for launch by the worker loop, which owns the shared
/// service handle background threads need.
pub(crate) struct GrabJob {
    pub(crate) request_id: String,
    pub(crate) project: xdm_grabber::Project,
    pub(crate) directory: String,
    pub(crate) queue_id: Option<i64>,
    pub(crate) enqueue: bool,
    pub(crate) stop: Arc<AtomicBool>,
    pub(crate) state: Arc<Mutex<GrabState>>,
}

impl GrabState {
    pub(crate) fn report(&self, request_id: &str) -> GrabReport {
        GrabReport {
            request_id: request_id.to_string(),
            state: self.state.clone(),
            files: self.files,
            pages: self.pages,
            errors: self.errors,
            error: self.error.clone(),
            task_ids: self.task_ids.clone(),
            skipped_duplicates: self.skipped.clone(),
        }
    }
}

pub(crate) fn valid_grab_name(name: &str) -> bool {
    !name.trim().is_empty()
        && name == name.trim()
        && name.len() <= 64
        && !name.chars().any(char::is_control)
}

/// Last percent-decoded URL path segment, or a safe placeholder.
pub(crate) fn grab_filename(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|url| {
            url.path_segments()
                .and_then(|mut segments| segments.next_back().map(str::to_owned))
        })
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            percent_encoding::percent_decode_str(&segment)
                .decode_utf8_lossy()
                .into_owned()
        })
        .filter(|name| xdm_categories::valid_filename(name))
        .unwrap_or_else(|| "download.bin".into())
}

const REDACTED: &str = "<redacted>";

/// Replaces manual-login header values so listings never expose them.
fn redact_auth(mut project: serde_json::Value) -> serde_json::Value {
    if let Some(headers) = project.get_mut("auth_headers").and_then(|v| v.as_array_mut()) {
        for pair in headers {
            if let Some(value) = pair.get_mut(1) {
                *value = serde_json::Value::String(REDACTED.into());
            }
        }
    }
    project
}

/// Saved grabber projects as a reply payload.
pub(crate) fn grab_project_reply(inner: &Inner) -> Result<Reply> {
    let records = inner
        .db
        .prepare("SELECT name,project_json FROM grab_projects ORDER BY name")?
        .query_map([], |row| {
            Ok(GrabProjectRecord {
                name: row.get(0)?,
                project: redact_auth(
                    serde_json::from_str(&row.get::<_, String>(1)?)
                        .unwrap_or(serde_json::Value::Null),
                ),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut reply = Reply::list(Vec::new());
    reply.grab_projects = records;
    Ok(reply)
}

impl Service {
    /// Starts a site grabber crawl on a background thread; entrenched replays
    /// of the same request ID return the recorded crawl state.
    pub(crate) fn grab_start(
        &self,
        request_id: String,
        project: serde_json::Value,
        directory: String,
        queue_id: Option<i64>,
        enqueue: bool,
    ) -> Result<Reply> {
        if request_id.is_empty() || request_id.len() > 64 || !valid_absolute_directory(&directory) {
            return Ok(Reply::error(
                "grab needs a request ID and an absolute destination directory without ..",
            ));
        }
        let project: xdm_grabber::Project = serde_json::from_value(project)
            .map_err(|e| format!("invalid grabber project: {e}"))?;
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        if let Some(handle) = inner.grabs.get(&request_id) {
            let report = handle
                .state
                .lock()
                .map_err(|_| io::Error::other("grab state lock poisoned"))?
                .report(&request_id);
            let mut reply = Reply::list(Vec::new());
            reply.grab = Some(report);
            return Ok(reply);
        }
        if let Some(queue_id) = queue_id {
            let exists: bool = inner.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM queues WHERE id=?1)",
                [queue_id],
                |row| row.get(0),
            )?;
            if !exists {
                return Ok(Reply::error("queue not found"));
            }
        }
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(GrabState {
            state: "running".into(),
            ..GrabState::default()
        }));
        inner.grabs.insert(
            request_id.clone(),
            GrabHandle {
                state: state.clone(),
            },
        );
        drop(inner);
        let pending = request_id.clone();
        self.queue_grab(&pending, project, directory, queue_id, enqueue, stop, state);
        Ok(Reply::grab_running(&request_id))
    }

    /// Registers a crawl for the worker loop to spawn, waking it up.
    pub(crate) fn queue_grab(
        &self,
        request_id: &str,
        project: xdm_grabber::Project,
        directory: String,
        queue_id: Option<i64>,
        enqueue: bool,
        stop: Arc<AtomicBool>,
        state: Arc<Mutex<GrabState>>,
    ) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        inner.pending_grabs.push(GrabJob {
            request_id: request_id.to_string(),
            project,
            directory,
            queue_id,
            enqueue,
            stop,
            state,
        });
        drop(inner);
        self.wake.notify_one();
    }

    /// Crawl body for a background thread: runs the crawl, records counters
    /// and atomically submits the found files as HTTP tasks.
    pub(crate) fn run_grab(
        &self,
        request_id: String,
        project: xdm_grabber::Project,
        directory: String,
        queue_id: Option<i64>,
        enqueue: bool,
        stop: Arc<AtomicBool>,
        state: Arc<Mutex<GrabState>>,
    ) {
        let save_dir = project.save_pages.then(|| Path::new(&directory));
        let outcome = xdm_grabber::crawl_to(&project, stop, save_dir);
        // Counters first, so a long task submission is visible as done.
        match &outcome {
            Ok(result) => {
                if let Ok(mut state) = state.lock() {
                    state.files = result.files.len();
                    state.pages = result.pages.len();
                    state.errors = result.errors.len();
                }
                match self.add_grab_tasks(&request_id, result, &directory, queue_id, enqueue) {
                    Ok((task_ids, skipped)) => {
                        if let Ok(mut state) = state.lock() {
                            state.task_ids = task_ids;
                            state.skipped = skipped;
                            state.state = "done".into();
                        }
                    }
                    Err(error) => {
                        if let Ok(mut state) = state.lock() {
                            state.state = "failed".into();
                            state.error = Some(error.to_string());
                        }
                    }
                }
            }
            Err(error) => {
                if let Ok(mut state) = state.lock() {
                    state.state = "failed".into();
                    state.error = Some(error.to_string());
                }
            }
        }
        self.wake.notify_one();
    }

    /// Turns a crawl's file list into task rows inside a single transaction so
    /// a grab either adds every file or leaves nothing behind.
    pub(crate) fn add_grab_tasks(
        &self,
        request_id: &str,
        result: &xdm_grabber::CrawlResult,
        directory: &str,
        queue_id: Option<i64>,
        enqueue: bool,
    ) -> Result<(Vec<i64>, Vec<xdm_ipc::SkippedDuplicate>)> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        if !inner.grabs.contains_key(request_id) {
            return Ok((Vec::new(), Vec::new()));
        }
        let urls = result.files.iter().take(xdm_transfer_list::MAX_ENTRIES).map(|f| f.url.as_str()).collect::<Vec<_>>();
        let (kept, skipped) = crate::split_duplicates(&inner.db, &urls, None)?;
        let rules = load_rules(&inner.db)?;
        let mut entries = Vec::new();
        let mut headers = Vec::new();
        for file in kept.iter().map(|&index| &result.files[index]) {
            let filename = grab_filename(&file.url);
            headers.push(if file.referer.is_empty() {
                "[]".to_string()
            } else {
                serde_json::to_string(&vec![("Referer".to_string(), file.referer.clone())])?
            });
            let category = xdm_categories::classify_with(
                &rules,
                &filename,
                url_host(&file.url).as_deref(),
                None,
            );
            entries.push(ImportEntry {
                url: file.url.clone(),
                filename,
                kind: "http".into(),
                priority: 1,
                connections: 8,
                rate_limit: None,
                video: None,
                audio: None,
                category,
                headers: Vec::new(),
            });
        }
        if entries.is_empty() {
            return Ok((Vec::new(), skipped));
        }
        let outputs = if result.local_paths.is_empty() {
            // Reuses the transfer-list name helpers so duplicates and unsafe
            // filenames get the same treatment as imports.
            xdm_transfer_list::export_names(&mut entries);
            entries
                .iter()
                .map(|entry| {
                    Path::new(directory)
                        .join(&entry.filename)
                        .to_string_lossy()
                        .into_owned()
                })
                .collect::<Vec<_>>()
        } else {
            // Offline mode: files land at the mirrored paths the saved
            // pages' links were rewritten to.
            let mut outputs = Vec::new();
            for entry in &entries {
                let Some(relative) = result.local_paths.get(&entry.url) else {
                    return Err(format!("no mirrored path for {}", entry.url).into());
                };
                let path = Path::new(directory)
                    .join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                outputs.push(path.to_string_lossy().into_owned());
            }
            outputs
        };
        for output in &outputs {
            if !destination_free(Path::new(output)) {
                return Err(
                    format!("grab destination already has a file or checkpoint: {output}").into(),
                );
            }
            let occupied: bool = inner.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM tasks WHERE output=?1 COLLATE NOCASE)",
                [output],
                |row| row.get(0),
            )?;
            if occupied {
                return Err(format!("grab destination is assigned to a task: {output}").into());
            }
        }
        let first_id = next_task_id(&inner.db)?;
        let transaction = inner.db.transaction()?;
        let mut task_ids = Vec::with_capacity(entries.len());
        for (index, (entry, output)) in entries.iter().zip(&outputs).enumerate() {
            let id = first_id
                .checked_add(index as i64)
                .ok_or("task ID exhausted")?;
            transaction.execute(
                "INSERT INTO tasks(id,request_id,url,output,headers_json,priority,connections,status,kind,category,queue_id,method) VALUES (?1,?2,?3,?4,?5,1,8,?6,'http',?7,?8,'GET')",
                params![
                    id,
                    format!("grab:{request_id}:{}", kept[index]),
                    entry.url,
                    output,
                    &headers[index],
                    if enqueue { "queued" } else { "paused" },
                    entry.category,
                    queue_id.unwrap_or(1) as i64,
                ],
            )?;
            task_ids.push(id);
        }
        transaction.commit()?;
        drop(inner);
        if enqueue {
            self.wake.notify_one();
        }
        Ok((task_ids, skipped))
    }

    pub(crate) fn grab_status(&self, request_id: &str) -> Result<Reply> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        let Some(handle) = inner.grabs.get(request_id) else {
            return Ok(Reply::error("grab request ID not found"));
        };
        let mut reply = Reply::list(Vec::new());
        reply.grab = Some(
            handle
                .state
                .lock()
                .map_err(|_| io::Error::other("grab state lock poisoned"))?
                .report(request_id),
        );
        Ok(reply)
    }

    pub(crate) fn grab_save(&self, name: &str, project: serde_json::Value) -> Result<Reply> {
        if !valid_grab_name(name) {
            return Ok(Reply::error("project name must be 1–64 trimmed characters"));
        }
        let parsed: xdm_grabber::Project = serde_json::from_value(project)
            .map_err(|e| format!("invalid grabber project: {e}"))?;
        parsed.validate().map_err(|e| e.to_string())?;
        let mut parsed = parsed;
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        // A listing shows redacted values; saving those back keeps the stored secret.
        if parsed.auth_headers.0.iter().any(|(_, value)| value == REDACTED) {
            let stored: Option<String> = inner
                .db
                .query_row("SELECT project_json FROM grab_projects WHERE name=?1", [name], |row| row.get(0))
                .ok();
            let old: Option<xdm_grabber::Project> = stored.and_then(|text| serde_json::from_str(&text).ok());
            for (header, value) in &mut parsed.auth_headers.0 {
                if value == REDACTED {
                    *value = old
                        .as_ref()
                        .and_then(|p| p.auth_headers.0.iter().find(|(n, _)| n == header))
                        .map(|(_, v)| v.clone())
                        .ok_or("auth header value is redacted and no stored value exists")?;
                }
            }
        }
        let canonical = serde_json::to_value(&parsed)?;
        inner.db.execute(
            "INSERT INTO grab_projects(name,project_json) VALUES (?1,?2) ON CONFLICT(name) DO UPDATE SET project_json=excluded.project_json",
            params![name, serde_json::to_string(&canonical)?],
        )?;
        grab_project_reply(&inner)
    }

    pub(crate) fn grab_projects(&self) -> Result<Reply> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        grab_project_reply(&inner)
    }

    pub(crate) fn grab_remove(&self, name: &str) -> Result<Reply> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        if inner
            .db
            .execute("DELETE FROM grab_projects WHERE name=?1", [name])?
            == 0
        {
            return Ok(Reply::error("project not found"));
        }
        grab_project_reply(&inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grab_filenames_are_decoded_and_sanitized() {
        assert_eq!(
            grab_filename("https://a.test/dir/Big%20Report.PDF?q=1"),
            "Big Report.PDF"
        );
        assert_eq!(grab_filename("https://a.test/dir/no#anchor"), "no");
        assert_eq!(grab_filename("https://a.test/"), "download.bin");
        assert_eq!(grab_filename("bad url"), "download.bin");
    }
}
