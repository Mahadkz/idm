use crate::categories::{load_rules, url_host};
use crate::media::stored_media_mode;
use crate::{Result, Service, next_task_id};
use rusqlite::params;
use rusqlite::OptionalExtension;
use std::io;
use std::path::Path;
use xdm_ipc::{ImportEntry, MediaMode, Reply, TransferList};

/// Shares the directory validation used by import and grab destinations.
pub(crate) fn valid_absolute_directory(directory: &str) -> bool {
    std::path::Path::new(directory).is_absolute()
        && !std::path::Path::new(directory)
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
}

/// An output slot is free when none of its file or checkpoint artefacts exist.
pub(crate) fn destination_free(output: &Path) -> bool {
    ["", ".part", ".part.meta", ".xdm-part", ".part.ftp"]
        .into_iter()
        .all(|suffix| std::fs::symlink_metadata(format!("{}{suffix}", output.display())).is_err())
}

impl Service {
    pub(crate) fn import_list(
        &self,
        request_id: String,
        directory: String,
        entries: Vec<ImportEntry>,
        enqueue: bool,
    ) -> Result<Reply> {
        if request_id.is_empty()
            || request_id.len() > 64
            || !valid_absolute_directory(&directory)
        {
            return Ok(Reply::error(
                "import needs a request ID and an absolute destination directory without ..",
            ));
        }
        if let Err(error) = xdm_transfer_list::validate(&entries) {
            return Ok(Reply::error(error));
        }
        if std::path::Path::new(&directory).exists()
            && !std::path::Path::new(&directory).is_dir()
        {
            return Ok(Reply::error("destination directory is a file"));
        }
        let payload_json = serde_json::to_string(&(&directory, &entries, enqueue))?;
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        let previous = inner
            .db
            .query_row(
                "SELECT payload_json,task_ids_json FROM imports WHERE request_id=?1",
                [&request_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        if let Some((old_payload, ids)) = previous {
            if old_payload != payload_json {
                return Ok(Reply::error(
                    "import request ID belongs to a different batch",
                ));
            }
            let ids: Vec<i64> = serde_json::from_str(&ids)?;
            for id in &ids {
                let exists: bool = inner.db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM tasks WHERE id=?1)",
                    [id],
                    |row| row.get(0),
                )?;
                if !exists {
                    return Ok(Reply::error(
                        "a task from this import was removed; use a new import request ID",
                    ));
                }
            }
            let mut reply = Reply::list(Vec::new());
            reply.task_ids = ids;
            return Ok(reply);
        }
        let outputs = entries
            .iter()
            .map(|entry| {
                std::path::Path::new(&directory)
                    .join(&entry.filename)
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        // The transfer list's own classification is its best guess;
        // when it matches the built-in fallback, user rules may know
        // better via extensions or site patterns.
        let rules = load_rules(&inner.db)?;
        let categories = entries
            .iter()
            .map(|entry| {
                if entry.category == xdm_categories::classify(&entry.filename, None) {
                    xdm_categories::classify_with(
                        &rules,
                        &entry.filename,
                        url_host(&entry.url).as_deref(),
                        None,
                    )
                } else {
                    entry.category.clone()
                }
            })
            .collect::<Vec<_>>();
        for output in &outputs {
            if !destination_free(Path::new(output)) {
                return Ok(Reply::error(format!(
                    "destination already has a file or checkpoint: {output}"
                )));
            }
            let occupied: bool = inner.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM tasks WHERE output=?1 COLLATE NOCASE)",
                [output],
                |row| row.get(0),
            )?;
            if occupied {
                return Ok(Reply::error(format!(
                    "destination is assigned to a task: {output}"
                )));
            }
        }
        let first_id = next_task_id(&inner.db)?;
        let transaction = inner.db.transaction()?;
        let mut ids = Vec::with_capacity(entries.len());
        for (index, (entry, output)) in entries.iter().zip(&outputs).enumerate() {
            let id = first_id
                .checked_add(index as i64)
                .ok_or("task ID exhausted")?;
            transaction.execute(
                "INSERT INTO tasks(id,request_id,url,output,headers_json,priority,connections,status,kind,media_video,media_audio,category,rate_limit) VALUES (?1,?2,?3,?4,'[]',?5,?6,?7,?8,?9,?10,?11,?12)",
                params![id,format!("import:{request_id}:{index}"),entry.url,output,entry.priority,entry.connections as i64,if enqueue {"queued"} else {"paused"},entry.kind,entry.video.map(|i| i as i64),entry.audio.map(|i| i as i64),&categories[index],entry.rate_limit.map(|rate| rate as i64)],
            )?;
            ids.push(id);
        }
        transaction.execute(
            "INSERT INTO imports(request_id,payload_json,task_ids_json) VALUES (?1,?2,?3)",
            params![request_id, payload_json, serde_json::to_string(&ids)?],
        )?;
        transaction.commit()?;
        drop(inner);
        if enqueue {
            self.wake.notify_one();
        }
        let mut reply = Reply::list(Vec::new());
        reply.task_ids = ids;
        Ok(reply)
    }

    pub(crate) fn export_list(&self) -> Result<Reply> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("task lock poisoned"))?;
        let mut statement = inner.db.prepare("SELECT url,output,kind,priority,connections,rate_limit,media_video,media_audio,media_mode_json,category FROM tasks ORDER BY id LIMIT 501")?;
        let mut entries = statement
            .query_map([], |row| {
                let output: String = row.get(1)?;
                let filename = std::path::Path::new(&output)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let stored: String = row.get(9)?;
                let kind: String = row.get(2)?;
                let mode_json: Option<String> = row.get(8)?;
                let video: Option<i64> = row.get(6)?;
                let audio: Option<i64> = row.get(7)?;
                // Portable lists carry only the media shape the reader
                // side validates: a video track muxed into .mp4.
                // Audio-only, sidecar, subtitle-carrying and MKV jobs
                // are omitted: as HTTP entries they would re-import as
                // a download of the manifest itself. Request headers
                // are never exported (see lib docs).
                let mode = stored_media_mode(mode_json, video, audio);
                let portable = kind != "media"
                    || matches!(
                        mode,
                        Some(MediaMode::Video { subtitles, .. }) if subtitles.is_empty()
                    )
                        && std::path::Path::new(&output)
                            .extension()
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"))
                        && video.is_some();
                // Portable lists only carry built-in category names, so
                // user categories fall back to the shared extension table.
                let category =
                    if xdm_categories::CATEGORIES.contains(&stored.as_str()) {
                        stored
                    } else {
                        xdm_categories::classify(&filename, None).into()
                    };
                if !portable {
                    return Ok(None);
                }
                Ok(Some(ImportEntry {
                    url: row.get(0)?,
                    filename,
                    kind,
                    priority: row.get(3)?,
                    connections: row.get::<_, i64>(4)? as usize,
                    rate_limit: row.get::<_, Option<i64>>(5)?.map(|rate| rate as u64),
                    video: video.map(|i| i as usize),
                    audio: audio.map(|i| i as usize),
                    category,
                }))
            })?
            .filter_map(|row| row.transpose())
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for entry in &mut entries {
            entry.url =
                xdm_transfer_list::export_url(&entry.url).map_err(io::Error::other)?;
        }
        xdm_transfer_list::export_names(&mut entries);
        if !entries.is_empty() {
            if let Err(error) = xdm_transfer_list::validate(&entries) {
                return Ok(Reply::error(error));
            }
        }
        let mut reply = Reply::list(Vec::new());
        reply.transfer_list = Some(TransferList {
            version: 1,
            entries,
        });
        Ok(reply)
    }
}
