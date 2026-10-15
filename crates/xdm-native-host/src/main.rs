//! Browser Native Messaging adapter. Stdout is reserved for framed JSON replies.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::{self, Write};
use std::path::PathBuf;
use xdm_http::MAX_BODY;
use xdm_ipc::{
    Command, ImportEntry, MediaAudioFormat, MediaMode, Paths, Reply, read_native, write_native,
};

/// Browser request headers for a submission from the shared requestContext
/// shape (headers, referrer, first cookie).
fn request_headers(payload: &Value) -> Result<Vec<(String, String)>, String> {
    let mut headers = Vec::new();
    if let Some(values) = payload
        .pointer("/requestContext/headers")
        .and_then(Value::as_array)
    {
        if values.len() > 32 {
            return Err("too many headers".into());
        }
        for value in values {
            let (Some(name), Some(text)) = (
                value.get("name").and_then(Value::as_str),
                value.get("value").and_then(Value::as_str),
            ) else {
                return Err("invalid header".into());
            };
            if name.len() > 128 || text.len() > 8192 {
                return Err("header too large".into());
            }
            headers.push((name.to_owned(), text.to_owned()));
        }
    }
    if let Some(referer) = payload
        .pointer("/requestContext/referrer")
        .and_then(Value::as_str)
    {
        headers.push(("Referer".into(), referer.into()));
    }
    if let Some(cookie) = payload
        .pointer("/requestContext/cookies/0/header")
        .and_then(Value::as_str)
    {
        headers.push(("Cookie".into(), cookie.into()));
    }
    Ok(headers)
}

/// Track index bound shared with the daemon's media validation.
fn bounded(value: u64) -> bool {
    value <= 10000
}

fn optional_index(payload: &Value, key: &str) -> Result<Option<usize>, &'static str> {
    match payload.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .filter(|index| bounded(*index))
            .map(|index| Some(index as usize))
            .ok_or("invalid media selection"),
    }
}

fn required_index(payload: &Value, key: &str) -> Result<usize, &'static str> {
    optional_index(payload, key)?.ok_or("invalid media selection")
}

/// One track index arriving either bare (a subtitle list element) or keyed.
fn index_value(value: &Value) -> Result<usize, &'static str> {
    value
        .as_u64()
        .filter(|index| bounded(*index))
        .map(|index| index as usize)
        .ok_or("invalid media selection")
}

fn subtitle_indices(payload: &Value) -> Result<Vec<usize>, &'static str> {
    match payload.get("subtitles") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(values) => values
            .as_array()
            .ok_or("invalid media selection")?
            .iter()
            .map(index_value)
            .collect(),
    }
}

/// Extracts the optional request method and replayable body from a submit
/// payload. The body arrives encoded (form-urlencoded) as a JSON string.
fn method_body(payload: &Value) -> Result<(String, Option<Vec<u8>>), &'static str> {
    let Some(method) = payload.get("method").and_then(Value::as_str) else {
        return Ok(("GET".into(), None));
    };
    let method = method.trim().to_ascii_uppercase();
    if method.is_empty()
        || method.len() > 16
        || !method.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return Err("invalid HTTP method");
    }
    let body = match payload.get("body").and_then(Value::as_str) {
        Some(text) => Some(text.as_bytes().to_vec()),
        None => None,
    };
    if method == "GET" {
        return body
            .is_none()
            .then(|| ("GET".into(), None))
            .ok_or("GET requests cannot carry a body");
    }
    let Some(body) = body else {
        return Err("method without a captured body");
    };
    if body.len() > MAX_BODY {
        return Err("request body exceeds the replay limit");
    }
    Ok((method, Some(body)))
}

fn filename(payload: &Value) -> Option<String> {
    let proposed = payload
        .get("filenameHint")
        .and_then(Value::as_str)
        .or_else(|| {
            payload
                .get("url")
                .and_then(Value::as_str)
                .and_then(|url| url.split('?').next())
                .and_then(|url| url.rsplit('/').next())
        })
        .unwrap_or("download.bin");
    xdm_categories::valid_filename(proposed).then(|| proposed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_cross_platform_unsafe_download_names() {
        for value in [
            "CON.txt", "com9.bin", "LPT2", "file.", "file ", "a?b.mp4", "../a",
        ] {
            assert!(
                filename(&json!({"filenameHint":value})).is_none(),
                "{value}"
            );
        }
        assert_eq!(
            filename(&json!({"filenameHint":"film.mp4"})).as_deref(),
            Some("film.mp4")
        );
    }


    fn batch_payload(items: Vec<Value>) -> Value {
        json!({"batchId": "b1", "count": items.len(), "items": items})
    }

    fn link(url: &str) -> Value {
        json!({"requestId": "b1-0", "url": url, "requestContext": {"referrer": "https://page.test/", "cookies": [{"header": "a=b"}]}})
    }

    #[test]
    fn batches_are_validated_item_by_item() {
        let dir = std::path::Path::new("/dl");
        let (planned, repeats) = plan_batch(
            &batch_payload(vec![
                link("https://a.test/one.zip"),
                link("https://a.test/two.mp4"),
                link("https://a.test/one.zip"),
                link("https://b.test/x/one.zip"),
            ]),
            dir,
        )
        .unwrap();
        assert_eq!(repeats, vec![(2, 0)]);
        assert_eq!(planned.len(), 3);
        // Same name in the same folder is varied, headers travel along.
        assert_eq!(planned[0].entry.filename, "one.zip");
        assert_eq!(planned[2].entry.filename, "one (2).zip");
        assert!(planned[0].entry.headers.iter().any(|(n, v)| n == "Cookie" && v == "a=b"));
        assert_eq!(planned[1].directory, dir.join("Video"));
        let long = "x".repeat(9000);
        let mut many: Vec<Value> = (0..MAX_BATCH_ITEMS + 1).map(|i| link(&format!("https://a.test/{i}.bin"))).collect();
        for bad in [
            batch_payload(vec![]),
            batch_payload(vec![link("file:///etc/passwd")]),
            batch_payload(vec![link("javascript:alert(1)")]),
            batch_payload(vec![link("ftp://h.test/f.bin")]),
            batch_payload(vec![link("https://u:p@a.test/f.bin")]),
            batch_payload(vec![link("https://a.test/")]),
            batch_payload(vec![json!({"url": "https://a.test/f.bin", "method": "POST", "body": "x"})]),
            batch_payload(vec![json!({"url": "https://a.test/f.bin", "requestContext": {"headers": [{"name": "X", "value": long}]}})]),
            batch_payload(vec![json!({"url": "https://a.test/f.bin", "requestContext": {"headers": (0..33).map(|i| json!({"name": format!("h{i}"), "value": "v"})).collect::<Vec<_>>()}})]),
            json!({"items": [link("https://a.test/f.bin")], "count": 2}),
            json!({"count": 1}),
            batch_payload(std::mem::take(&mut many)),
        ] {
            assert!(plan_batch(&bad, dir).is_err(), "{bad:.80}");
        }
    }

    fn ok_reply(ids: Vec<i64>, skipped: Vec<xdm_ipc::SkippedDuplicate>) -> Reply {
        let mut reply = Reply::list(Vec::new());
        reply.task_ids = ids;
        reply.skipped_duplicates = skipped;
        reply
    }

    #[test]
    fn batch_results_report_duplicates_with_batch_indexes() {
        let dir = std::path::Path::new("/dl");
        let (planned, repeats) = plan_batch(
            &batch_payload(vec![link("https://a.test/a.bin"), link("https://a.test/b.bin"), link("https://a.test/a.bin")]),
            dir,
        )
        .unwrap();
        let mut sent = Vec::new();
        let reply = run_batch("b1", &planned, &repeats, &mut |command| {
            sent.push(command);
            Ok(ok_reply(vec![7], vec![xdm_ipc::SkippedDuplicate {
                index: 1, url: "https://a.test/b.bin".into(), duplicate_of: Some(3), duplicate_of_index: None,
            }]))
        });
        assert_eq!(sent.len(), 1);
        assert!(matches!(&sent[0], Command::ImportListEx { enqueue: true, allow_duplicate: false, entries, .. } if entries.len() == 2));
        assert_eq!(reply["type"], "batch.result");
        assert_eq!(reply["accepted"], true);
        assert_eq!(reply["count"], 3);
        assert_eq!(reply["acceptedCount"], 1);
        let skipped = reply["skippedDuplicates"].as_array().unwrap();
        assert!(skipped.iter().any(|d| d["index"] == 2 && d["duplicateOfIndex"] == 0));
        assert!(skipped.iter().any(|d| d["index"] == 1 && d["duplicateOf"] == 3));
    }

    #[test]
    fn a_failed_chunk_rolls_the_batch_back_as_an_error_reply() {
        let dir = std::path::Path::new("/dl");
        let (planned, repeats) = plan_batch(
            &batch_payload(vec![link("https://a.test/a.zip"), link("https://a.test/b.mp4")]),
            dir,
        )
        .unwrap();
        let mut calls = 0;
        let mut removed = Vec::new();
        let reply = run_batch("b2", &planned, &repeats, &mut |command| {
            calls += 1;
            match command {
                Command::Remove { task_id } => {
                    removed.push(task_id);
                    Ok(ok_reply(vec![], vec![]))
                }
                _ if calls == 1 => Ok(ok_reply(vec![11], vec![])),
                _ => Ok(Reply::error("destination already has a file or checkpoint")),
            }
        });
        assert_eq!(reply["type"], "error");
        assert_eq!(removed, vec![11]);
    }

    #[test]
    fn large_batches_are_chunked_within_daemon_limits() {
        let items: Vec<Value> = (0..1000).map(|i| link(&format!("https://a.test/f{i}.bin"))).collect();
        let (planned, _) = plan_batch(&batch_payload(items), std::path::Path::new("/dl")).unwrap();
        let chunks = chunk_batch(&planned);
        assert!(chunks.len() >= 3);
        assert!(chunks.iter().all(|c| c.len() <= CHUNK_ENTRIES));
        assert_eq!(chunks.iter().map(Vec::len).sum::<usize>(), 1000);
    }

    #[test]
    fn method_bodies_are_validated() {
        let post = json!({"method": "post", "body": "token=abc"});
        assert_eq!(method_body(&post).unwrap().0, "POST");
        assert_eq!(method_body(&post).unwrap().1.as_deref(), Some(b"token=abc".as_slice()));
        assert_eq!(
            method_body(&json!({})).unwrap(),
            ("GET".into(), None)
        );
        assert!(method_body(&json!({"method": "GET", "body": "x"})).is_err());
        assert!(method_body(&json!({"method": "POST"})).is_err());
        assert!(method_body(&json!({"method": "bad method", "body": "x"})).is_err());
        assert!(method_body(&json!({"method": "", "body": ""})).is_err());
        let oversized = method_body(&json!({"method": "POST", "body": "x".repeat(MAX_BODY + 1)}));
        assert!(oversized.is_err());
    }

    #[test]
    fn media_payloads_map_to_modes_or_reject() {
        // The legacy shape stays a plain video job.
        assert_eq!(
            media_mode(&json!({"videoIndex": 0, "audioIndex": 3})).unwrap(),
            MediaMode::Video { video: 0, audio: Some(3), subtitles: Vec::new() }
        );
        // Declared modes ride in typed jobs.
        assert_eq!(
            media_mode(&json!({"mode": "video", "videoIndex": 2, "subtitles": [7, 8]})).unwrap(),
            MediaMode::Video { video: 2, audio: None, subtitles: vec![7, 8] }
        );
        assert_eq!(
            media_mode(&json!({"mode": "audio", "track": 1, "format": {"kind": "mp3", "bitrate_kbps": 96}})).unwrap(),
            MediaMode::Audio { track: 1, format: MediaAudioFormat::Mp3 { bitrate_kbps: 96 } }
        );
        assert_eq!(
            media_mode(&json!({"mode": "audio", "track": 1})).unwrap(),
            MediaMode::Audio { track: 1, format: MediaAudioFormat::Mp3 { bitrate_kbps: 128 } }
        );
        assert_eq!(
            media_mode(&json!({"mode": "subtitles", "track": 4})).unwrap(),
            MediaMode::Subtitles { track: 4 }
        );
        // Live payloads carry the video/audio selection and an optional
        // bounded catch cap.
        assert_eq!(
            media_mode(&json!({"mode": "live", "videoIndex": 2, "audioIndex": 3, "maxSeconds": 120}))
                .unwrap(),
            MediaMode::Live { video: 2, audio: Some(3), max_seconds: Some(120) }
        );
        assert_eq!(
            media_mode(&json!({"mode": "live", "videoIndex": 0})).unwrap(),
            MediaMode::Live { video: 0, audio: None, max_seconds: None }
        );
        for payload in [
            json!({"mode": "mystery"}),
            json!({"mode": "audio", "track": 10001}),
            json!({"mode": "audio", "track": 1, "format": {"kind": "flac"}}),
            json!({"mode": "video", "subtitles": [1, 99999]}),
            json!({"mode": "live", "maxSeconds": xdm_ipc::MAX_LIVE_SECONDS + 1}),
            json!({"mode": "live"}),
            json!({"audioIndex": 1}),
        ] {
            assert!(media_mode(&payload).is_err(), "{payload}");
        }
        // Browser request context headers and the shared caps travel through.
        let headers = request_headers(&json!({
            "requestContext": {
                "headers": [{"name": "User-Agent", "value": "xdm-integration"}],
                "referrer": "https://page.test/",
            }
        }))
        .unwrap();
        assert_eq!(headers[0], ("User-Agent".to_owned(), "xdm-integration".to_owned()));
        assert_eq!(headers[1], ("Referer".to_owned(), "https://page.test/".to_owned()));
        assert_eq!(
            request_headers(&json!({"requestContext": {"headers": [{"name": "a"}]}})).unwrap_err(),
            "invalid header"
        );
    }
}

fn default_download_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join("Downloads"))
    } else {
        std::env::var_os("XDG_DOWNLOAD_DIR")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Downloads")))
    }
}

fn submit(message: &Value, paths: &Paths) -> Value {
    let request_id = match message.get("requestId").and_then(Value::as_str) {
        Some(id) if !id.is_empty() && id.len() <= 128 => id.to_owned(),
        _ => {
            return json!({"version":1,"type":"download.result","accepted":false,"reason":"invalid requestId"});
        }
    };
    let payload = &message["payload"];
    let url = match payload.get("url").and_then(Value::as_str) {
        Some(url) if url.starts_with("http://") || url.starts_with("https://") => url.to_owned(),
        _ => {
            return json!({"version":1,"requestId":request_id,"type":"download.result","accepted":false,"reason":"invalid URL"});
        }
    };
    let name = match filename(payload) {
        Some(name) => name,
        None => {
            return json!({"version":1,"requestId":request_id,"type":"download.result","accepted":false,"reason":"invalid filename"});
        }
    };
    let directory = match default_download_dir() {
        Some(dir) => dir,
        None => {
            return json!({"version":1,"requestId":request_id,"type":"download.result","accepted":false,"reason":"Downloads directory unavailable"});
        }
    };
    let headers = match request_headers(payload) {
        Ok(headers) => headers,
        Err(reason) => {
            return json!({"version":1,"requestId":request_id,"type":"download.result","accepted":false,"reason":reason});
        }
    };
    let command = match method_body(payload) {
        Ok((method, None)) if method == "GET" => Command::Submit {
            request_id: request_id.clone(),
            url,
            output: xdm_categories::browser_destination(
                &directory,
                &name,
                payload.get("mime").and_then(Value::as_str),
            )
            .to_string_lossy()
            .into_owned(),
            headers,
            priority: 1,
            connections: 8,
        },
        Ok((method, body)) => Command::SubmitRequest {
            request_id: request_id.clone(),
            url,
            output: xdm_categories::browser_destination(
                &directory,
                &name,
                payload.get("mime").and_then(Value::as_str),
            )
            .to_string_lossy()
            .into_owned(),
            headers,
            method,
            body,
            priority: 1,
            connections: 8,
        },
        Err(reason) => {
            return json!({"version":1,"requestId":request_id,"type":"download.result","accepted":false,"reason":reason});
        }
    };
    match paths.request(command) {
        Ok(reply) if reply.ok => {
            json!({"version":1,"requestId":request_id,"type":"download.result","accepted":true,"taskId":reply.task_id})
        }
        Ok(reply) => {
            json!({"version":1,"requestId":request_id,"type":"download.result","accepted":false,"reason":reply.error})
        }
        Err(e) => {
            json!({"version":1,"requestId":request_id,"type":"download.result","accepted":false,"reason":format!("daemon unavailable: {e}")})
        }
    }
}

/// Largest number of links one `batch.submit` may carry.
const MAX_BATCH_ITEMS: usize = 2000;
/// Serialized size of one import command; the daemon caps a list at 512 KiB.
const CHUNK_BYTES: usize = 384 * 1024;
const CHUNK_ENTRIES: usize = 400;
/// Header bytes one item may carry in total.
const MAX_ITEM_HEADER_BYTES: usize = 16 * 1024;

/// One validated link of a batch, ready to become an import entry.
#[derive(Debug, Clone)]
struct BatchItem {
    index: usize,
    entry: ImportEntry,
    directory: PathBuf,
}

/// Name with a numeric suffix before the extension: `a.zip` -> `a (2).zip`.
fn suffixed(name: &str, n: usize) -> String {
    match name.rfind('.').filter(|&at| at > 0) {
        Some(at) => format!("{} ({n}){}", &name[..at], &name[at..]),
        None => format!("{name} ({n})"),
    }
}

/// Validates every item of a batch payload. Any bad item rejects the whole
/// batch (the extension then submits the links one by one). Repeated URLs are
/// returned separately so the reply can report them as duplicates.
fn plan_batch(
    payload: &Value,
    directory: &std::path::Path,
) -> Result<(Vec<BatchItem>, Vec<(usize, usize)>), String> {
    let items = payload
        .get("items")
        .and_then(Value::as_array)
        .ok_or("batch needs an items array")?;
    if items.is_empty() || items.len() > MAX_BATCH_ITEMS {
        return Err(format!("a batch must contain 1-{MAX_BATCH_ITEMS} links"));
    }
    if let Some(count) = payload.get("count") {
        if count.as_u64() != Some(items.len() as u64) {
            return Err("batch count does not match its items".into());
        }
    }
    let mut planned: Vec<BatchItem> = Vec::new();
    let mut repeats = Vec::new();
    let mut first_by_url = std::collections::HashMap::new();
    let mut names = std::collections::HashSet::new();
    for (index, item) in items.iter().enumerate() {
        let fail = |reason: &str| format!("item {index}: {reason}");
        let url = item
            .get("url")
            .and_then(Value::as_str)
            .filter(|url| url.len() <= 8192)
            .ok_or_else(|| fail("invalid URL"))?;
        let parsed = url::Url::parse(url).map_err(|_| fail("invalid URL"))?;
        match parsed.scheme() {
            "http" | "https" if parsed.host_str().is_some() => {}
            "ftp" if parsed.host_str().is_some() => {
                return Err(fail("FTP links are submitted one by one"));
            }
            _ => return Err(fail("invalid URL")),
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(fail("embedded credentials are not accepted"));
        }
        method_body(item).and_then(|(method, body)| {
            (method == "GET" && body.is_none()).then_some(()).ok_or("only GET links can be batched")
        }).map_err(|reason| fail(reason))?;
        let headers = request_headers(item).map_err(|reason| fail(&reason))?;
        if headers.iter().map(|(n, v)| n.len() + v.len()).sum::<usize>() > MAX_ITEM_HEADER_BYTES {
            return Err(fail("headers too large"));
        }
        if let Some(&first) = first_by_url.get(url) {
            repeats.push((index, first));
            continue;
        }
        first_by_url.insert(url.to_owned(), index);
        let name = filename(item).ok_or_else(|| fail("invalid filename"))?;
        let mime = item.get("mime").and_then(Value::as_str);
        let destination = xdm_categories::browser_destination(directory, &name, mime);
        let folder = destination.parent().unwrap_or(directory).to_path_buf();
        let mut unique = name.clone();
        let mut n = 1;
        while !names.insert((folder.clone(), unique.to_lowercase())) {
            n += 1;
            unique = suffixed(&name, n);
        }
        planned.push(BatchItem {
            index,
            entry: ImportEntry {
                url: url.to_owned(),
                category: xdm_categories::classify(&name, mime).into(),
                filename: unique,
                kind: "http".into(),
                priority: 1,
                connections: 8,
                rate_limit: None,
                video: None,
                audio: None,
                headers,
            },
            directory: folder,
        });
    }
    Ok((planned, repeats))
}

/// Splits planned items into import commands: one destination folder each and
/// bounded in entries and serialized size.
fn chunk_batch(planned: &[BatchItem]) -> Vec<Vec<&BatchItem>> {
    let mut chunks: Vec<(Vec<&BatchItem>, usize)> = Vec::new();
    for item in planned {
        let size = serde_json::to_vec(&item.entry).map_or(0, |bytes| bytes.len());
        match chunks.iter_mut().find(|(chunk, bytes)| {
            chunk[0].directory == item.directory
                && chunk.len() < CHUNK_ENTRIES
                && *bytes + size <= CHUNK_BYTES
        }) {
            Some((chunk, bytes)) => {
                chunk.push(item);
                *bytes += size;
            }
            None => chunks.push((vec![item], size)),
        }
    }
    chunks.into_iter().map(|(chunk, _)| chunk).collect()
}

fn batch_error(request_id: Option<&str>, reason: impl Into<String>) -> Value {
    let mut reply = json!({"version":1,"type":"error","reason":reason.into()});
    if let Some(id) = request_id {
        reply["requestId"] = json!(id);
    }
    reply
}

/// Runs the import commands of a planned batch through `send`. Everything is
/// accepted or nothing is: tasks created by earlier chunks are removed when a
/// later chunk fails. Rejections are `type:"error"` replies, which the
/// extensions read as "fall back to per-link submission".
fn run_batch(
    request_id: &str,
    planned: &[BatchItem],
    repeats: &[(usize, usize)],
    send: &mut dyn FnMut(Command) -> io::Result<Reply>,
) -> Value {
    let digest = format!("{:x}", Sha256::digest(request_id.as_bytes()));
    let mut task_ids: Vec<i64> = Vec::new();
    let mut skipped: Vec<Value> = repeats
        .iter()
        .map(|(index, first)| json!({"index": index, "duplicateOfIndex": first}))
        .collect();
    for (number, chunk) in chunk_batch(planned).into_iter().enumerate() {
        let command = Command::ImportListEx {
            request_id: format!("batch-{}-{number}", &digest[..16]),
            directory: chunk[0].directory.to_string_lossy().into_owned(),
            entries: chunk.iter().map(|item| item.entry.clone()).collect(),
            enqueue: true,
            allow_duplicate: false,
        };
        let failure = match send(command) {
            Ok(reply) if reply.ok => {
                task_ids.extend(&reply.task_ids);
                skipped.extend(reply.skipped_duplicates.iter().map(|dup| {
                    json!({
                        "index": chunk[dup.index].index,
                        "url": dup.url,
                        "duplicateOf": dup.duplicate_of,
                        "duplicateOfIndex": dup.duplicate_of_index.map(|i| chunk[i].index),
                    })
                }));
                continue;
            }
            Ok(reply) => reply.error.unwrap_or_else(|| "daemon rejected the batch".into()),
            Err(error) => format!("daemon unavailable: {error}"),
        };
        for id in task_ids {
            let _ = send(Command::Remove { task_id: id });
        }
        return batch_error(Some(request_id), failure);
    }
    let count = planned.len() + repeats.len();
    json!({
        "version": 1,
        "requestId": request_id,
        "type": "batch.result",
        "accepted": true,
        "count": count,
        "acceptedCount": task_ids.len(),
        "taskIds": task_ids,
        "skippedDuplicates": skipped,
    })
}

fn submit_batch(message: &Value, paths: &Paths) -> Value {
    let request_id = match message.get("requestId").and_then(Value::as_str) {
        Some(id) if !id.is_empty() && id.len() <= 128 => id.to_owned(),
        _ => return batch_error(None, "invalid requestId"),
    };
    let Some(directory) = default_download_dir() else {
        return batch_error(Some(&request_id), "Downloads directory unavailable");
    };
    match plan_batch(&message["payload"], &directory) {
        Ok((planned, repeats)) => run_batch(&request_id, &planned, &repeats, &mut |command| {
            paths.request(command)
        }),
        Err(reason) => batch_error(Some(&request_id), reason),
    }
}

fn inspect_media(message: &Value) -> Value {
    let url = match message.pointer("/payload/url").and_then(Value::as_str) {
        Some(url) if url.starts_with("http://") || url.starts_with("https://") => url,
        _ => {
            return json!({"version":1,"type":"media.inspect.result","ok":false,"reason":"invalid URL"});
        }
    };
    match xdm_media::inspect(url) {
        Ok(tracks) => json!({"version":1,"type":"media.inspect.result","ok":true,"tracks":tracks}),
        Err(error) => {
            json!({"version":1,"type":"media.inspect.result","ok":false,"reason":error.to_string()})
        }
    }
}

/// Optional media job mode in the payload: outright `mode`, or the legacy
/// `videoIndex`/`audioIndex` pair read as a plain video job.
fn media_mode(payload: &Value) -> Result<MediaMode, &'static str> {
    match payload.get("mode").and_then(Value::as_str) {
        Some("video") => Ok(MediaMode::Video {
            video: required_index(payload, "videoIndex")?,
            audio: optional_index(payload, "audioIndex")?,
            subtitles: subtitle_indices(payload)?,
        }),
        Some("audio") => Ok(MediaMode::Audio {
            track: required_index(payload, "audioIndex").or(required_index(payload, "track"))?,
            format: match payload.get("format") {
                Some(value) => serde_json::from_value(value.clone())
                    .map_err(|_| "invalid media format"),
                None => Ok(MediaAudioFormat::Mp3 { bitrate_kbps: 128 }),
            }?,
        }),
        Some("subtitles") => Ok(MediaMode::Subtitles {
            track: required_index(payload, "track")
                .or(required_index(payload, "audioIndex"))?,
        }),
        Some("live") => {
            let max_seconds = match payload.get("maxSeconds") {
                None | Some(Value::Null) => None,
                Some(value) => Some(
                    value
                        .as_u64()
                        .filter(|seconds| (1..=xdm_ipc::MAX_LIVE_SECONDS).contains(seconds))
                        .ok_or("invalid media selection")?,
                ),
            };
            Ok(MediaMode::Live {
                video: required_index(payload, "videoIndex")?,
                audio: optional_index(payload, "audioIndex")?,
                max_seconds,
            })
        }
        None => Ok(MediaMode::Video {
            video: required_index(payload, "videoIndex")?,
            audio: optional_index(payload, "audioIndex")?,
            subtitles: Vec::new(),
        }),
        Some(_) => Err("invalid media mode"),
    }
}

fn submit_media(message: &Value, paths: &Paths) -> Value {
    let request_id = match message.get("requestId").and_then(Value::as_str) {
        Some(value) if !value.is_empty() && value.len() <= 128 => value.to_owned(),
        _ => {
            return json!({"version":1,"type":"media.result","accepted":false,"reason":"invalid requestId"});
        }
    };
    let payload = &message["payload"];
    let Some(url) = payload.get("url").and_then(Value::as_str) else {
        return json!({"version":1,"requestId":request_id,"type":"media.result","accepted":false,"reason":"invalid URL"});
    };
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return json!({"version":1,"requestId":request_id,"type":"media.result","accepted":false,"reason":"invalid URL"});
    }
    let headers = match request_headers(payload) {
        Ok(headers) => headers,
        Err(reason) => {
            return json!({"version":1,"requestId":request_id,"type":"media.result","accepted":false,"reason":reason});
        }
    };
    let mode = match media_mode(payload) {
        Ok(mode) => mode,
        Err(reason) => {
            return json!({"version":1,"requestId":request_id,"type":"media.result","accepted":false,"reason":reason});
        }
    };
    let directory = match default_download_dir() {
        Some(value) => value,
        None => {
            return json!({"version":1,"requestId":request_id,"type":"media.result","accepted":false,"reason":"Downloads directory unavailable"});
        }
    };
    let name = filename(payload).unwrap_or_else(|| "video".into());
    let stem = std::path::Path::new(&name)
        .file_stem()
        .and_then(|part| part.to_str())
        .unwrap_or("video");
    let digest = format!("{:x}", Sha256::digest(request_id.as_bytes()));
    // Legacy payloads keep their historical name shape; typed jobs name the
    // mode so video, audio, sidecar and live outputs of one stream never
    // collide.
    let output = match &mode {
        MediaMode::Video { video, .. } => directory
            .join("Video")
            .join(format!("{stem}-v{video}-{}.mp4", &digest[..10])),
        MediaMode::Live { video, .. } => directory
            .join("Video")
            .join(format!("{stem}-live-v{video}-{}.mp4", &digest[..10])),
        MediaMode::Audio { format, .. } => directory
            .join("Music")
            .join(format!(
                "{stem}-a-{}.{extension}",
                &digest[..10],
                extension = match format {
                    MediaAudioFormat::Mp3 { .. } => "mp3",
                    MediaAudioFormat::Opus { .. } => "opus",
                    MediaAudioFormat::M4a { .. } | MediaAudioFormat::Original => "m4a",
                }
            )),
        MediaMode::Subtitles { .. } => directory
            .join("Video")
            .join(format!("{stem}-s-{}.srt", &digest[..10])),
    };
    let command = if headers.is_empty() && payload.get("mode").is_none() {
        Command::SubmitMedia {
            request_id: request_id.clone(),
            url: url.into(),
            output: output.to_string_lossy().into_owned(),
            video: match &mode {
                MediaMode::Video { video, .. } => *video,
                _ => unreachable!("legacy payloads are always video jobs"),
            },
            audio: match &mode {
                MediaMode::Video { audio, .. } => *audio,
                _ => unreachable!("legacy payloads are always video jobs"),
            },
            priority: 1,
        }
    } else {
        Command::SubmitMediaJob {
            request_id: request_id.clone(),
            url: url.into(),
            output: output.to_string_lossy().into_owned(),
            headers,
            mode,
            priority: 1,
        }
    };
    match paths.request(command) {
        Ok(reply) if reply.ok => {
            json!({"version":1,"requestId":request_id,"type":"media.result","accepted":true,"taskId":reply.task_id})
        }
        Ok(reply) => {
            json!({"version":1,"requestId":request_id,"type":"media.result","accepted":false,"reason":reply.error})
        }
        Err(error) => {
            json!({"version":1,"requestId":request_id,"type":"media.result","accepted":false,"reason":format!("daemon unavailable: {error}")})
        }
    }
}

/// Native messaging manifests this host can detect for the current user.
fn manifest_report() -> Value {
    const NAME: &str = "org.xdm.browser";
    #[cfg(unix)]
    let exe = std::env::current_exe().ok();
    let mut found = Vec::new();
    #[cfg(unix)]
    {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let places = [
            ("firefox", ".mozilla/native-messaging-hosts"),
            ("chrome", ".config/google-chrome/NativeMessagingHosts"),
            ("chromium", ".config/chromium/NativeMessagingHosts"),
            ("edge", ".config/microsoft-edge/NativeMessagingHosts"),
        ];
        for (browser, dir) in places {
            let Some(home) = &home else { break };
            let path = home.join(dir).join(format!("{NAME}.json"));
            if !path.is_file() {
                continue;
            }
            let manifest: Value = std::fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or(Value::Null);
            let target = manifest.get("path").and_then(Value::as_str);
            let target_exists = target.is_some_and(|p| std::path::Path::new(p).is_file());
            let matches_self = match (target, &exe) {
                (Some(target), Some(exe)) => {
                    std::fs::canonicalize(target).ok() == std::fs::canonicalize(exe).ok()
                }
                _ => false,
            };
            found.push(json!({
                "browser": browser,
                "manifest": path.to_string_lossy(),
                "valid": !manifest.is_null(),
                "hostPath": target,
                "hostPathExists": target_exists,
                "hostPathIsThisBinary": matches_self,
                "allowedOrigins": manifest.get("allowed_origins").cloned().unwrap_or(Value::Null),
                "allowedExtensions": manifest.get("allowed_extensions").cloned().unwrap_or(Value::Null),
            }));
        }
    }
    #[cfg(windows)]
    {
        let keys = [
            ("firefox", r"HKCU\Software\Mozilla\NativeMessagingHosts"),
            ("chrome", r"HKCU\Software\Google\Chrome\NativeMessagingHosts"),
            ("chromium", r"HKCU\Software\Chromium\NativeMessagingHosts"),
            ("edge", r"HKCU\Software\Microsoft\Edge\NativeMessagingHosts"),
        ];
        for (browser, key) in keys {
            let output = std::process::Command::new("reg")
                .args(["query", &format!(r"{key}\{NAME}"), "/ve"])
                .output();
            if let Ok(output) = output {
                if output.status.success() {
                    let text = String::from_utf8_lossy(&output.stdout);
                    let manifest = text.split("REG_SZ").nth(1).map(|s| s.trim().to_owned());
                    let exists = manifest
                        .as_deref()
                        .is_some_and(|p| std::path::Path::new(p).is_file());
                    found.push(json!({"browser": browser, "manifest": manifest, "valid": exists}));
                }
            }
        }
    }
    json!({"name": NAME, "registered": !found.is_empty(), "entries": found})
}

/// Reports which boundary is failing: manifest registration, the host
/// itself, or the daemon. Browser fallback stays the extension's call.
fn health(paths: &Paths) -> Value {
    let daemon = match paths.request(Command::ListPage {
        offset: 0,
        limit: 1,
        sort: None,
        filter: None,
    }) {
        Ok(reply) if reply.ok => json!({"reachable": true, "version": reply.daemon_version, "tasks": reply.total}),
        Ok(reply) => json!({"reachable": true, "error": reply.error}),
        Err(error) => json!({"reachable": false, "error": format!("daemon unavailable: {error}")}),
    };
    let manifest = manifest_report();
    let mut failing = Vec::new();
    if daemon["reachable"] != json!(true) {
        failing.push("daemon");
    }
    if manifest["registered"] != json!(true) {
        failing.push("manifest");
    }
    json!({
        "version": 1,
        "type": "health.result",
        "nativeHost": {"version": env!("CARGO_PKG_VERSION"), "path": std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned())},
        "daemon": daemon,
        "manifest": manifest,
        "failing": failing,
        "ok": failing.is_empty(),
    })
}

fn process(message: Value, paths: &Paths) -> Value {
    if message.get("version").and_then(Value::as_u64) != Some(1) {
        return json!({"version":1,"type":"error","reason":"unsupported version"});
    }
    match message.get("type").and_then(Value::as_str) {
        Some("hello") => {
            json!({"version":1,"type":"hello.result","capabilities":["health","download.submit","batch.submit","media.inspect","media.submit"]})
        }
        Some("health") => health(paths),
        Some("download.submit") => submit(&message, paths),
        Some("batch.submit") => submit_batch(&message, paths),
        Some("media.inspect") => inspect_media(&message),
        Some("media.submit") => submit_media(&message, paths),
        _ => json!({"version":1,"type":"error","reason":"unsupported message type"}),
    }
}

fn main() {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    let paths = match Paths::open() {
        Ok(paths) => paths,
        Err(e) => {
            eprintln!("native host setup: {e}");
            std::process::exit(1);
        }
    };
    loop {
        let message: Value = match read_native(&mut input) {
            Ok(message) => message,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => {
                eprintln!("native host input: {e}");
                break;
            }
        };
        if let Err(e) = write_native(&mut output, &process(message, &paths)) {
            eprintln!("native host output: {e}");
            break;
        }
        let _ = output.flush();
    }
}
