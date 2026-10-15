//! Synchronization queues (Q01/Q10): completed members stay in the queue and
//! every check probes the remote object with a lightweight request; only a
//! member whose validators (ETag, Last-Modified, length) changed is fetched
//! again, into a temporary file that atomically replaces the old one.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED, RANGE};
use rusqlite::{Connection, params};
use xdm_http::{Options, Proxy, TransferControl};
use xdm_ipc::Reply;

use crate::{Inner, Result, Service, unix_now};

/// Remote facts that identify one version of an object.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Validators {
    pub(crate) etag: Option<String>,
    pub(crate) last_modified: Option<String>,
    pub(crate) length: Option<u64>,
}

impl Validators {
    fn is_empty(&self) -> bool {
        self.etag.is_none() && self.last_modified.is_none() && self.length.is_none()
    }
}

#[derive(Debug)]
pub(crate) enum Probed {
    NotModified,
    Remote(Validators),
}

/// Whether the remote object differs from the stored baseline. With no
/// baseline yet the local file's size is the only evidence available.
pub(crate) fn changed(saved: &Validators, local_len: Option<u64>, remote: &Validators) -> bool {
    if saved.is_empty() {
        return match (local_len, remote.length) {
            (Some(local), Some(remote)) => local != remote,
            (None, _) => true,
            _ => false,
        };
    }
    if let (Some(a), Some(b)) = (&saved.etag, &remote.etag) {
        return a != b;
    }
    if let (Some(a), Some(b)) = (&saved.last_modified, &remote.last_modified)
        && a != b
    {
        return true;
    }
    matches!((saved.length, remote.length), (Some(a), Some(b)) if a != b)
}

fn client(proxy: &Proxy) -> Result<Client> {
    let mut builder = Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(15));
    match proxy {
        Proxy::System => {}
        Proxy::Direct => builder = builder.no_proxy(),
        Proxy::Manual { http, https, bypass } => {
            builder = builder.no_proxy();
            let no_proxy = reqwest::NoProxy::from_string(&bypass.join(","));
            if let Some(url) = http {
                builder = builder.proxy(reqwest::Proxy::http(url)?.no_proxy(no_proxy.clone()));
            }
            if let Some(url) = https {
                builder = builder.proxy(reqwest::Proxy::https(url)?.no_proxy(no_proxy));
            }
        }
    }
    Ok(builder.build()?)
}

fn header(response: &Response, name: reqwest::header::HeaderName) -> Option<String> {
    response.headers().get(name)?.to_str().ok().map(str::to_owned)
}

fn remote_validators(response: &Response) -> Validators {
    let range_total = header(response, CONTENT_RANGE)
        .and_then(|value| value.rsplit('/').next().and_then(|total| total.parse().ok()));
    Validators {
        etag: header(response, ETAG),
        last_modified: header(response, LAST_MODIFIED),
        length: range_total.or_else(|| header(response, CONTENT_LENGTH).and_then(|v| v.parse().ok())),
    }
}

/// Lightweight probe: a conditional HEAD, falling back to a one-byte ranged
/// GET for servers that refuse HEAD. 304 means unchanged.
pub(crate) fn probe(
    url: &str,
    headers: &[(String, String)],
    proxy: &Proxy,
    saved: &Validators,
) -> Result<Probed> {
    let client = client(proxy)?;
    let build = |method: reqwest::Method| {
        let mut request = client.request(method, url);
        for (name, value) in headers {
            request = request.header(name, value);
        }
        if let Some(etag) = &saved.etag {
            request = request.header(IF_NONE_MATCH, etag);
        }
        if let Some(date) = &saved.last_modified {
            request = request.header(IF_MODIFIED_SINCE, date);
        }
        request
    };
    let mut response = build(reqwest::Method::HEAD).send().map_err(|e| e.to_string())?;
    if matches!(
        response.status(),
        StatusCode::METHOD_NOT_ALLOWED | StatusCode::NOT_IMPLEMENTED
    ) {
        response = build(reqwest::Method::GET)
            .header(RANGE, "bytes=0-0")
            .send()
            .map_err(|e| e.to_string())?;
    }
    let status = response.status();
    if status == StatusCode::NOT_MODIFIED {
        return Ok(Probed::NotModified);
    }
    if !status.is_success() {
        return Err(format!("HTTP {}", status.as_u16()).into());
    }
    Ok(Probed::Remote(remote_validators(&response)))
}

struct Member {
    id: i64,
    url: String,
    output: String,
    headers: Vec<(String, String)>,
    connections: usize,
    kind: String,
    method: String,
    saved: Validators,
}

fn temp_path(output: &Path, id: i64) -> PathBuf {
    PathBuf::from(format!("{}.sync{id}.tmp", output.display()))
}

fn cleanup(temp: &Path) {
    for path in [
        temp.to_path_buf(),
        PathBuf::from(format!("{}.part", temp.display())),
        PathBuf::from(format!("{}.part.meta", temp.display())),
    ] {
        let _ = std::fs::remove_file(path);
    }
}

fn record(db: &Connection, queue_id: i64, execution: i64, outcome: &str, action: &str, status: &str) {
    let _ = db.execute(
        "INSERT INTO queue_events(queue_id,execution,outcome,action,action_status,at) VALUES (?1,?2,?3,?4,?5,?6)",
        params![queue_id, execution, outcome, action, status, unix_now()],
    );
}

/// Queues whose automatic interval elapsed; marks them as started now.
pub(crate) fn due_queues(inner: &mut Inner) -> Result<Vec<i64>> {
    let now = inner.clock.now();
    let rows = inner
        .db
        .prepare("SELECT id FROM queues WHERE sync=1 AND sync_interval IS NOT NULL AND ?1-sync_last>=sync_interval")?
        .query_map([now], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut due = Vec::new();
    for id in rows {
        if inner.sync_running.contains(&id) || inner.pending_syncs.contains(&id) {
            continue;
        }
        inner.db.execute("UPDATE queues SET sync_last=?1 WHERE id=?2", params![now, id])?;
        due.push(id);
    }
    Ok(due)
}

/// Time until the earliest automatic check; `None` when no sync queue has an
/// interval. Queues with a check in flight are re-evaluated when it ends.
pub(crate) fn next_due(inner: &Inner) -> Result<Option<Duration>> {
    let now = inner.clock.now();
    let rows = inner
        .db
        .prepare("SELECT id,sync_interval,sync_last FROM queues WHERE sync=1 AND sync_interval IS NOT NULL")?
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter(|(id, _, _)| !inner.sync_running.contains(id))
        .map(|(_, interval, last)| Duration::from_secs((last + interval - now).max(0) as u64))
        .min())
}

impl Service {
    pub(crate) fn notify_sync(&self) {
        let (flag, condvar) = &*self.sync_wake;
        if let Ok(mut pending) = flag.lock() {
            *pending = true;
            condvar.notify_one();
        }
    }

    pub(crate) fn queue_set_sync(&self, queue_id: i64, interval: Option<u64>) -> Result<Reply> {
        if interval.is_some_and(|seconds| !(1..=366 * 86400).contains(&seconds)) {
            return Ok(Reply::error("sync interval must be between 1 second and 366 days"));
        }
        let inner = self.inner.lock().map_err(|_| io::Error::other("task lock poisoned"))?;
        let sync: Option<bool> = inner
            .db
            .query_row("SELECT sync FROM queues WHERE id=?1", [queue_id], |row| row.get(0))
            .ok();
        match sync {
            None => return Ok(Reply::error("queue not found")),
            Some(false) => return Ok(Reply::error("not a synchronization queue")),
            Some(true) => {}
        }
        // The interval counts from now, so setting it never checks at once.
        inner.db.execute(
            "UPDATE queues SET sync_interval=?1,sync_last=?2 WHERE id=?3",
            params![interval.map(|v| v as i64), inner.clock.now(), queue_id],
        )?;
        self.notify_sync();
        crate::queues::queue_reply(&inner, queue_id)
    }

    pub(crate) fn queue_check(&self, queue_id: i64) -> Result<Reply> {
        let mut inner = self.inner.lock().map_err(|_| io::Error::other("task lock poisoned"))?;
        let sync: Option<bool> = inner
            .db
            .query_row("SELECT sync FROM queues WHERE id=?1", [queue_id], |row| row.get(0))
            .ok();
        match sync {
            None => return Ok(Reply::error("queue not found")),
            Some(false) => return Ok(Reply::error("not a synchronization queue")),
            Some(true) => {}
        }
        if inner.sync_running.contains(&queue_id) || inner.pending_syncs.contains(&queue_id) {
            return Ok(Reply::error("a synchronization check is already running"));
        }
        let now = inner.clock.now();
        inner.db.execute("UPDATE queues SET sync_last=?1 WHERE id=?2", params![now, queue_id])?;
        inner.pending_syncs.push(queue_id);
        self.notify_sync();
        crate::queues::queue_reply(&inner, queue_id)
    }

    /// Starts every due or requested check on its own thread.
    pub(crate) fn sync_pump(self: &Arc<Self>) -> Result<Option<Duration>> {
        let jobs = {
            let mut inner = self.inner.lock().map_err(|_| io::Error::other("task lock poisoned"))?;
            let due = due_queues(&mut inner)?;
            inner.pending_syncs.extend(due);
            let jobs = std::mem::take(&mut inner.pending_syncs);
            for id in &jobs {
                inner.sync_running.insert(*id);
            }
            jobs
        };
        let next = {
            let inner = self.inner.lock().map_err(|_| io::Error::other("task lock poisoned"))?;
            next_due(&inner)?
        };
        for id in jobs {
            let service = self.clone();
            std::thread::spawn(move || {
                if let Err(error) = service.run_sync_check(id) {
                    eprintln!("sync check error: {error}");
                }
                if let Ok(mut inner) = service.inner.lock() {
                    inner.sync_running.remove(&id);
                }
                service.notify_sync();
            });
        }
        Ok(next)
    }

    /// Checks every completed member of one synchronization queue.
    pub(crate) fn run_sync_check(&self, queue_id: i64) -> Result<()> {
        let (execution, proxy, stall, members) = {
            let inner = self.inner.lock().map_err(|_| io::Error::other("task lock poisoned"))?;
            inner.db.execute("UPDATE queues SET run_exec=run_exec+1 WHERE id=?1", [queue_id])?;
            let execution: i64 =
                inner.db.query_row("SELECT run_exec FROM queues WHERE id=?1", [queue_id], |row| row.get(0))?;
            let members = inner
                .db
                .prepare("SELECT id,url,output,headers_json,connections,kind,method,sync_etag,sync_lastmod,sync_len FROM tasks WHERE queue_id=?1 AND status='complete' ORDER BY position,id")?
                .query_map([queue_id], |row| {
                    Ok(Member {
                        id: row.get(0)?,
                        url: row.get(1)?,
                        output: row.get(2)?,
                        headers: serde_json::from_str(&row.get::<_, String>(3)?).unwrap_or_default(),
                        connections: row.get::<_, i64>(4)?.clamp(1, 16) as usize,
                        kind: row.get(5)?,
                        method: row.get(6)?,
                        saved: Validators {
                            etag: row.get(7)?,
                            last_modified: row.get(8)?,
                            length: row.get::<_, Option<i64>>(9)?.map(|v| v as u64),
                        },
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            (
                execution,
                serde_json::from_str::<Proxy>(&inner.proxy).unwrap_or_default(),
                Duration::from_secs(inner.stall_timeout),
                members,
            )
        };
        let (mut unchanged, mut updated, mut failed) = (0, 0, 0);
        for member in members {
            let result = self.sync_member(queue_id, &member, &proxy, stall);
            let (outcome, detail) = match &result {
                Ok(true) => {
                    updated += 1;
                    ("sync_updated", "ok".to_string())
                }
                Ok(false) => {
                    unchanged += 1;
                    ("sync_unchanged", "ok".to_string())
                }
                Err(error) => {
                    failed += 1;
                    ("sync_failed", error.to_string())
                }
            };
            let inner = self.inner.lock().map_err(|_| io::Error::other("task lock poisoned"))?;
            record(&inner.db, queue_id, execution, outcome, &format!("task:{}", member.id), &detail);
        }
        let inner = self.inner.lock().map_err(|_| io::Error::other("task lock poisoned"))?;
        record(
            &inner.db,
            queue_id,
            execution,
            "sync_checked",
            &format!("unchanged={unchanged} updated={updated} failed={failed}"),
            "done",
        );
        Ok(())
    }

    /// Returns whether the member was replaced by a newer remote version.
    fn sync_member(
        &self,
        queue_id: i64,
        member: &Member,
        proxy: &Proxy,
        stall: Duration,
    ) -> Result<bool> {
        if member.kind != "http" || member.method != "GET" || !member.url.starts_with("http") {
            return Err("only plain HTTP(S) GET members can be synchronized".into());
        }
        let output = Path::new(&member.output);
        let local_len = std::fs::metadata(output).ok().map(|meta| meta.len());
        let remote = match probe(&member.url, &member.headers, proxy, &member.saved)? {
            Probed::NotModified => return Ok(false),
            Probed::Remote(remote) => remote,
        };
        let store = |remote: &Validators| -> Result<()> {
            let inner = self.inner.lock().map_err(|_| io::Error::other("task lock poisoned"))?;
            inner.db.execute(
                "UPDATE tasks SET sync_etag=?1,sync_lastmod=?2,sync_len=?3 WHERE id=?4",
                params![remote.etag, remote.last_modified, remote.length.map(|v| v as i64), member.id],
            )?;
            Ok(())
        };
        if !changed(&member.saved, local_len, &remote) {
            if member.saved.is_empty() {
                store(&remote)?;
            }
            return Ok(false);
        }
        let temp = temp_path(output, member.id);
        cleanup(&temp);
        let options = Options {
            url: member.url.clone(),
            output: temp.clone(),
            connections: member.connections,
            restart: true,
            headers: member.headers.clone(),
            method: "GET".into(),
            body: None,
            bytes_per_second: None,
            proxy: proxy.clone(),
            stop: Arc::new(AtomicBool::new(false)),
            temp_dir: None,
            set_file_time: false,
        };
        let control = Arc::new(TransferControl::new(None)?);
        if let Err(error) = xdm_http::download_with_stall_timeout(options, control, stall) {
            cleanup(&temp);
            return Err(error.to_string().into());
        }
        // The member must still exist and still belong to this queue.
        let still_member = {
            let inner = self.inner.lock().map_err(|_| io::Error::other("task lock poisoned"))?;
            inner
                .db
                .query_row(
                    "SELECT COUNT(*) FROM tasks WHERE id=?1 AND queue_id=?2 AND status='complete'",
                    params![member.id, queue_id],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0)
                > 0
        };
        if !still_member {
            cleanup(&temp);
            return Err("member was removed during the check".into());
        }
        if let Err(error) = std::fs::rename(&temp, output) {
            cleanup(&temp);
            return Err(format!("could not replace the file: {error}").into());
        }
        store(&remote)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pump_reports_none_without_sync_queues_and_remaining_time_otherwise() {
        let (service, _clock, _counting) = crate::test_helpers::service(1_000);
        let remaining = |service: &Arc<Service>| {
            next_due(&service.inner.lock().unwrap()).unwrap()
        };
        assert_eq!(remaining(&service), None);
        service.queue_create("plain".into(), 2, false).unwrap();
        assert_eq!(remaining(&service), None);
        let reply = service.queue_create("sync".into(), 2, true).unwrap();
        let id = reply.queue_id.unwrap();
        assert_eq!(remaining(&service), None, "no interval yet");
        assert!(service.queue_set_sync(id, Some(600)).unwrap().ok);
        assert_eq!(remaining(&service), Some(Duration::from_secs(600)));
        service.inner.lock().unwrap().db.execute("UPDATE queues SET sync_last=sync_last-250 WHERE id=?1", [id]).unwrap();
        assert_eq!(remaining(&service), Some(Duration::from_secs(350)));
        service.inner.lock().unwrap().db.execute("UPDATE queues SET sync_last=sync_last-1000 WHERE id=?1", [id]).unwrap();
        assert_eq!(remaining(&service), Some(Duration::ZERO));
        service.inner.lock().unwrap().sync_running.insert(id);
        assert_eq!(remaining(&service), None, "in-flight check re-notifies when done");
        assert!(service.queue_set_sync(id, None).unwrap().ok);
    }

    fn v(etag: Option<&str>, lm: Option<&str>, len: Option<u64>) -> Validators {
        Validators { etag: etag.map(Into::into), last_modified: lm.map(Into::into), length: len }
    }

    #[test]
    fn change_detection_prefers_etag_then_date_then_length() {
        let saved = v(Some("\"a\""), Some("d1"), Some(5));
        assert!(!changed(&saved, Some(5), &v(Some("\"a\""), Some("d2"), Some(9))));
        assert!(changed(&saved, Some(5), &v(Some("\"b\""), Some("d1"), Some(5))));
        let dated = v(None, Some("d1"), Some(5));
        assert!(changed(&dated, Some(5), &v(None, Some("d2"), Some(5))));
        assert!(!changed(&dated, Some(5), &v(None, Some("d1"), Some(5))));
        assert!(changed(&v(None, None, Some(5)), Some(5), &v(None, None, Some(6))));
    }

    #[test]
    fn missing_baseline_compares_local_size() {
        let none = Validators::default();
        assert!(!changed(&none, Some(5), &v(Some("x"), None, Some(5))));
        assert!(changed(&none, Some(4), &v(Some("x"), None, Some(5))));
        assert!(changed(&none, None, &v(None, None, None)));
    }
}
