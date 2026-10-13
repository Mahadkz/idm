//! Browser Native Messaging adapter. Stdout is reserved for framed JSON replies.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::{self, Write};
use std::path::PathBuf;
use xdm_http::MAX_BODY;
use xdm_ipc::{Command, MediaAudioFormat, MediaMode, Paths, read_native, write_native};

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

fn process(message: Value, paths: &Paths) -> Value {
    if message.get("version").and_then(Value::as_u64) != Some(1) {
        return json!({"version":1,"type":"error","reason":"unsupported version"});
    }
    match message.get("type").and_then(Value::as_str) {
        Some("hello") => {
            json!({"version":1,"type":"hello.result","capabilities":["download.submit","media.inspect","media.submit"]})
        }
        Some("download.submit") => submit(&message, paths),
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
