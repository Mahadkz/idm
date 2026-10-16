//! Clear, public HLS/DASH inspection, FFmpeg stream-copy jobs, and live
//! "stop and save" recording. DRM, credential replay, and transcoding are
//! outside this implementation. ffmpeg only talks to a private loopback relay (see `relay.rs`).

mod relay;
use relay::Relay;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::error::Error;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

pub type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

/// One stream of a manifest: video, audio, or subtitle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub index: usize,
    pub kind: String,
    pub codec: String,
    pub width: Option<u64>,
    pub height: Option<u64>,
    pub bitrate: Option<u64>,
    pub language: Option<String>,
    #[serde(default)]
    pub sample_rate: Option<u64>,
    #[serde(default)]
    pub channels: Option<u64>,
    /// ffprobe disposition flag, true for streams marked DEFAULT in the manifest.
    #[serde(default)]
    pub default: Option<bool>,
}

/// Inspection result: the track list plus the live-stream verdict.
///
/// Deserializes from either the wrapped `{tracks, live}` shape or a bare track
/// array so older stored inspections keep loading; `live` defaults to false.
#[derive(Debug, Clone, Serialize)]
pub struct Inspection {
    pub tracks: Vec<Track>,
    /// True when the manifest chain is still growing: an HLS media playlist
    /// without `#EXT-X-ENDLIST`, or a DASH MPD with type="dynamic".
    #[serde(default)]
    pub live: bool,
}

impl<'de> Deserialize<'de> for Inspection {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Shape {
            Wrapped {
                #[serde(default)]
                tracks: Vec<Track>,
                #[serde(default)]
                live: bool,
            },
            Bare(Vec<Track>),
        }
        match Shape::deserialize(deserializer)? {
            Shape::Wrapped { tracks, live } => Ok(Inspection { tracks, live }),
            Shape::Bare(tracks) => Ok(Inspection { tracks, live: false }),
        }
    }
}

/// Container and quality target for [`download_audio`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AudioFormat {
    /// Stream-copy into the container that fits the source codec:
    /// `.m4a` for AAC, `.opus`/`.webm` for Opus, `.mp3` for MP3.
    #[serde(rename = "original")]
    Original,
    /// Re-encode to MP3 at the requested bitrate.
    #[serde(rename = "mp3")]
    Mp3 { bitrate_kbps: u32 },
    /// Re-encode to AAC inside an `.m4a` container.
    #[serde(rename = "m4a")]
    M4a { bitrate_kbps: u32 },
    /// Re-encode to Opus in `.opus` or `.webm`.
    #[serde(rename = "opus")]
    Opus { bitrate_kbps: u32 },
}

fn err(message: impl Into<String>) -> Box<dyn Error + Send + Sync> {
    message.into().into()
}

/// Shared HTTP input options that every ffprobe/ffmpeg call needs.
const CLEAR_INPUT_ARGS: [&str; 4] = ["-protocol_whitelist", "http,tcp", "-rw_timeout", "15000000"];

/// Rejects header lines that could inject further ffmpeg options over the wire.
fn validate_headers(headers: &[(String, String)]) -> Result<()> {
    for (name, value) in headers {
        if name.is_empty() || name.contains(['\r', '\n', '\0']) {
            return Err(err("invalid header name"));
        }
        if value.contains(['\r', '\n', '\0']) {
            return Err(err(format!("invalid header value for {name}")));
        }
    }
    Ok(())
}

fn manifest_kind(url: &str) -> Result<&'static str> {
    let path = reqwest::Url::parse(url)?.path().to_ascii_lowercase();
    if path.ends_with(".m3u8") {
        Ok("hls")
    } else if path.ends_with(".mpd") {
        Ok("dash")
    } else {
        Err(err("media URL must end in .m3u8 or .mpd"))
    }
}

/// True when the MPD root declares type="dynamic" (a live DASH presentation).
fn dash_is_dynamic(text: &str) -> bool {
    let Some(start) = text.find("<MPD") else {
        return false;
    };
    let tag = text[start..].split_once('>').map_or(&text[start..], |(tag, _)| tag);
    let lower: String = tag
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    lower.contains("type=\"dynamic\"") || lower.contains("type='dynamic'")
}

/// `Some(is_live)` for an HLS media playlist (segment-bearing document),
/// `None` for a master playlist. Live means no `#EXT-X-ENDLIST` yet.
fn hls_playlist_liveness(text: &str) -> Option<bool> {
    let mut segments = false;
    let mut ended = false;
    for line in text.lines() {
        let line = line.trim();
        segments |= line.starts_with("#EXTINF");
        ended |= line.starts_with("#EXT-X-ENDLIST");
    }
    segments.then_some(!ended)
}

/// A saved site login for the manifest walk. It is attached only to requests whose
/// origin equals the origin of `url`; ffmpeg and ffprobe never receive it (not in argv,
/// not in `-headers`), so segment, key and sub-playlist hosts never see it.
#[derive(Clone)]
pub struct OriginLogin {
    pub url: String,
    pub username: String,
    pub password: String,
    /// Allow Basic over cleartext http:// to a non-loopback host.
    pub allow_insecure: bool,
}

impl std::fmt::Debug for OriginLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OriginLogin").field("url", &self.url).field("password", &"<hidden>").finish()
    }
}

thread_local! {
    static ORIGIN_LOGIN: std::cell::RefCell<Option<OriginLogin>> = const { std::cell::RefCell::new(None) };
}

/// Runs `job` with `login` available to manifest requests made on this thread.
pub fn with_origin_login<R>(login: Option<OriginLogin>, job: impl FnOnce() -> R) -> R {
    let previous = ORIGIN_LOGIN.with(|slot| slot.replace(login));
    let result = job();
    ORIGIN_LOGIN.with(|slot| *slot.borrow_mut() = previous);
    result
}

/// ffmpeg/ffprobe only ever talk to the loopback relay, which applies the job's proxy
/// itself; the environment proxy must not capture that hop.
fn tool(name: &str) -> Command {
    let mut command = Command::new(name);
    for variable in ["http_proxy", "https_proxy", "all_proxy", "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"] {
        command.env_remove(variable);
    }
    command.env("no_proxy", "127.0.0.1,localhost").env("NO_PROXY", "127.0.0.1,localhost");
    command
}

thread_local! {
    static PROXY: std::cell::RefCell<(xdm_http::Proxy, Option<xdm_http::Login>)> =
        std::cell::RefCell::new((xdm_http::Proxy::System, None));
}

/// Runs `job` routed through the job's configured proxy (and stored manual-proxy
/// login); the relay's upstream client and the manifest walk both use it.
pub fn with_proxy<R>(proxy: xdm_http::Proxy, login: Option<xdm_http::Login>, job: impl FnOnce() -> R) -> R {
    let previous = PROXY.with(|slot| slot.replace((proxy, login)));
    let result = job();
    PROXY.with(|slot| *slot.borrow_mut() = previous);
    result
}

fn current_proxy() -> (xdm_http::Proxy, Option<xdm_http::Login>) {
    PROXY.with(|slot| slot.borrow().clone())
}

fn current_origin_login() -> Option<OriginLogin> {
    ORIGIN_LOGIN.with(|slot| slot.borrow().clone())
}

/// Manifest walk results: subtitle rendition playlists plus live detection.
struct ManifestScan {
    renditions: Vec<String>,
    /// True when the chain carries a media playlist without `#EXT-X-ENDLIST`
    /// or a DASH MPD typed dynamic.
    live: bool,
}

/// Validates the whole clear manifest chain and returns the subtitle rendition
/// playlist URLs (master only, manifest order, matching ffprobe stream order)
/// alongside the live-stream verdict.
fn walk_manifest_report(url: &str, relay: &Relay) -> Result<ManifestScan> {
    let master = manifest_kind(url)?;
    let mut pending = vec![reqwest::Url::parse(url)?];
    let mut visited = HashSet::new();
    let mut scan = ManifestScan { renditions: Vec::new(), live: false };
    while let Some(current) = pending.pop() {
        if !visited.insert(current.to_string()) {
            continue;
        }
        if visited.len() > 32 {
            return Err(err("too many nested playlists"));
        }
        let (response, final_url) =
            relay.upstream().send("GET", &current, None, Some(Duration::from_secs(20)))?;
        let response = response.error_for_status()?;
        if response.content_length().is_some_and(|n| n > relay::MANIFEST_LIMIT) {
            return Err(err("manifest too large"));
        }
        let mut bytes = Vec::new();
        response.take(relay::MANIFEST_LIMIT + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > relay::MANIFEST_LIMIT {
            return Err(err("manifest too large"));
        }
        // Relative references resolve against where the manifest was finally served.
        let current = final_url;
        let text = std::str::from_utf8(&bytes)?;
        if master == "hls" {
            if !text.starts_with("#EXTM3U") {
                return Err(err("invalid HLS playlist"));
            }
            if text.lines().any(|line| {
                line.starts_with("#EXT-X-KEY") || line.starts_with("#EXT-X-SESSION-KEY")
            }) {
                return Err(err("encrypted HLS is unsupported"));
            }
            if hls_playlist_liveness(text) == Some(true) {
                scan.live = true;
            }
            // Only the first fetched document (the master) declares renditions.
            if visited.len() == 1 {
                for line in text.lines().map(str::trim) {
                    if !line.starts_with("#EXT-X-MEDIA:") {
                        continue;
                    }
                    let attributes = line["#EXT-X-MEDIA:".len()..].trim_start();
                    if !attributes.split(',').any(|attribute| {
                        match attribute.split_once('=') {
                            Some((key, value)) => {
                                key.trim().eq_ignore_ascii_case("type")
                                    && value.trim_matches('"').eq_ignore_ascii_case("subtitles")
                            }
                            None => false,
                        }
                    }) {
                        continue;
                    }
                    let uri = line
                        .split("URI=\"")
                        .nth(1)
                        .and_then(|rest| rest.split('"').next())
                        .ok_or_else(|| {
                            err("HLS subtitle rendition is missing a URI")
                        })?;
                    let rendition = current.join(uri)?;
                    if !matches!(rendition.scheme(), "http" | "https") {
                        return Err(err("unsupported playlist URL scheme"));
                    }
                    scan.renditions.push(rendition.to_string());
                }
            }
            // Variant lines and EXT-X-MEDIA audio URIs can lead to nested playlists.
            for line in text.lines().map(str::trim) {
                let reference = if line.starts_with("#EXT-X-MEDIA:")
                    || line.starts_with("#EXT-X-I-FRAME-STREAM-INF:")
                {
                    line.split("URI=\"")
                        .nth(1)
                        .and_then(|rest| rest.split('"').next())
                } else if !line.starts_with('#') && line.to_ascii_lowercase().contains(".m3u8") {
                    Some(line)
                } else {
                    None
                };
                if let Some(reference) = reference {
                    let child = current.join(reference)?;
                    if !matches!(child.scheme(), "http" | "https") {
                        return Err(err("unsupported playlist URL scheme"));
                    }
                    pending.push(child);
                }
            }
        }
        if master == "dash" {
            if !text.contains("<MPD") || text.to_ascii_lowercase().contains("contentprotection") {
                return Err(err("protected or invalid DASH manifest"));
            }
            scan.live |= dash_is_dynamic(text);
        }
    }
    Ok(scan)
}

fn walk_manifest(url: &str, relay: &Relay) -> Result<Vec<String>> {
    Ok(walk_manifest_report(url, relay)?.renditions)
}

/// Starts the job's relay after validating the headers.
fn start_relay(url: &str, headers: &[(String, String)]) -> Result<Relay> {
    validate_headers(headers)?;
    manifest_kind(url)?;
    Relay::start(&reqwest::Url::parse(url)?, headers)
}

fn probe(url: &str, relay: &Relay) -> Result<Value> {
    walk_manifest(url, relay)?;
    ffprobe(relay)
}

/// ffprobe stream JSON for an already validated manifest, read through the relay.
fn ffprobe(relay: &Relay) -> Result<Value> {
    let mut command = tool("ffprobe");
    command.args(CLEAR_INPUT_ARGS);
    command.args(["-v", "error", "-show_streams", "-of", "json"]);
    command.arg(relay.manifest_url());
    let output = command.output()?;
    if !output.status.success() {
        return Err(err(format!(
            "ffprobe failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn numeric(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

fn tracks_from_probe(data: &Value) -> Result<Vec<Track>> {
    let streams = data["streams"]
        .as_array()
        .ok_or_else(|| err("ffprobe returned no streams"))?;
    Ok(streams
        .iter()
        .filter_map(|stream| {
            let kind = stream["codec_type"].as_str()?;
            if kind != "video" && kind != "audio" && kind != "subtitle" {
                return None;
            }
            Some(Track {
                index: stream["index"].as_u64()? as usize,
                kind: kind.into(),
                codec: stream["codec_name"].as_str().unwrap_or("unknown").into(),
                width: stream["width"].as_u64(),
                height: stream["height"].as_u64(),
                bitrate: numeric(&stream["tags"]["variant_bitrate"])
                    .or_else(|| numeric(&stream["bit_rate"])),
                language: stream["tags"]["language"].as_str().map(str::to_owned),
                sample_rate: numeric(&stream["sample_rate"]),
                channels: numeric(&stream["channels"]),
                default: match numeric(&stream["disposition"]["default"]) {
                    Some(1) => Some(true),
                    Some(0) => Some(false),
                    _ => None,
                },
            })
        })
        .collect())
}

pub fn inspect(url: &str) -> Result<Vec<Track>> {
    inspect_with_headers(url, &[])
}

pub fn inspect_with_headers(url: &str, headers: &[(String, String)]) -> Result<Vec<Track>> {
    let relay = start_relay(url, headers)?;
    inspect_in(url, &relay)
}

fn inspect_in(url: &str, relay: &Relay) -> Result<Vec<Track>> {
    tracks_from_probe(&probe(url, relay)?)
}

pub fn inspect_report(url: &str) -> Result<Inspection> {
    inspect_report_with_headers(url, &[])
}

/// [`inspect_with_headers`] plus the live verdict, in one manifest walk.
pub fn inspect_report_with_headers(url: &str, headers: &[(String, String)]) -> Result<Inspection> {
    let relay = start_relay(url, headers)?;
    inspect_report_in(url, &relay)
}

fn inspect_report_in(url: &str, relay: &Relay) -> Result<Inspection> {
    let scan = walk_manifest_report(url, relay)?;
    let data = ffprobe(relay)?;
    Ok(Inspection { tracks: tracks_from_probe(&data)?, live: scan.live })
}

/// Installs the completed file without replacing anything created at the destination meanwhile.
fn publish(part: &Path, output: &Path) -> Result<()> {
    match fs::hard_link(part, output) {
        Ok(()) => Ok(fs::remove_file(part)?),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(err(format!(
            "{} appeared during the download; the completed data was kept at {}",
            output.display(),
            part.display()
        ))),
        // Filesystems without hard links fall back to a checked rename.
        Err(_) if fs::symlink_metadata(output).is_err() => Ok(fs::rename(part, output)?),
        Err(_) => Err(err(format!(
            "{} appeared during the download; the completed data was kept at {}",
            output.display(),
            part.display()
        ))),
    }
}

fn part_path(output: &Path) -> PathBuf {
    let extension = output
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("media");
    output.with_extension(format!("{extension}.xdm-part"))
}

/// Last ffprobe/ffmpeg complaints, folded into one line for the error message.
fn summarize(text: &str) -> String {
    let mut lines: Vec<String> = text
        .lines()
        .rev()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .take(4)
        .collect();
    lines.reverse();
    lines.join("; ")
}

/// Collects a child's stderr on a side thread so the pipe never fills up.
fn drain_stderr(stream: impl Read + Send + 'static) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut raw = Vec::new();
        let mut stream = stream;
        let _ = stream.read_to_end(&mut raw);
        String::from_utf8_lossy(&raw).into_owned()
    })
}

/// Drives one ffmpeg job to completion; removes the temporary file on pause or
/// failure and publishes the finished file through `publish`.
fn run_ffmpeg_job(
    mut command: Command,
    temporary: &Path,
    output: &Path,
    stop: &AtomicBool,
) -> Result<()> {
    command.stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let complaints = child.stderr.take().map(drain_stderr);
    let status = loop {
        if stop.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = fs::remove_file(temporary);
            let _ = complaints.map(|handle| handle.join());
            return Err(err("paused"));
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        thread::sleep(Duration::from_millis(100));
    };
    let complaints = complaints
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    if !status.success() {
        let _ = fs::remove_file(temporary);
        let summary = summarize(&complaints);
        return Err(err(if summary.is_empty() {
            format!("ffmpeg failed with {status}")
        } else {
            format!("ffmpeg failed with {status}: {summary}")
        }));
    }
    if fs::metadata(temporary)?.len() == 0 {
        let _ = fs::remove_file(temporary);
        return Err(err("mux produced an empty file"));
    }
    publish(temporary, output)
}

/// How long ffmpeg may take to finalize the container after the stop signal.
const STOP_GRACE: Duration = Duration::from_secs(15);

/// Size and media duration of a finished recording.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RecordSummary {
    /// Media duration of the finished file; the wall-clock capture time is
    /// used when the container carries no readable duration.
    pub duration_secs: f64,
    /// Published file size in bytes.
    pub bytes: u64,
}

/// Drives one live recording. A stop request is a graceful "stop and save":
/// ffmpeg is asked to finish (`q` on stdin) so the crash-safe container stays
/// playable, and the file is published even if ffmpeg had to be killed after
/// [`STOP_GRACE`]. `max_duration` reaches the same stop path by itself.
fn run_live_ffmpeg_job(
    mut command: Command,
    temporary: &Path,
    output: &Path,
    max_duration: Option<Duration>,
    stop: &AtomicBool,
) -> Result<()> {
    command.stdout(Stdio::null()).stderr(Stdio::piped()).stdin(Stdio::piped());
    let mut child = command.spawn()?;
    let mut stdin = child.stdin.take();
    let complaints = child.stderr.take().map(drain_stderr);
    let started = Instant::now();
    let mut stopping = false;
    let mut stop_at: Option<Instant> = None;
    let status = loop {
        if !stopping
            && (stop.load(Ordering::Relaxed)
                || max_duration.is_some_and(|max| started.elapsed() >= max))
        {
            stopping = true;
            if let Some(handle) = stdin.as_mut() {
                let _ = handle.write_all(b"q");
                let _ = handle.flush();
            }
            drop(stdin.take());
            stop_at = Some(Instant::now());
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if stop_at.is_some_and(|at| at.elapsed() >= STOP_GRACE) {
            let _ = child.kill();
            break child.wait()?;
        }
        thread::sleep(Duration::from_millis(100));
    };
    let complaints = complaints
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    if !status.success() && !stopping {
        let _ = fs::remove_file(temporary);
        let summary = summarize(&complaints);
        return Err(err(if summary.is_empty() {
            format!("ffmpeg failed with {status}")
        } else {
            format!("ffmpeg failed with {status}: {summary}")
        }));
    }
    if fs::metadata(temporary)?.len() == 0 {
        let _ = fs::remove_file(temporary);
        return Err(err("mux produced an empty file"));
    }
    publish(temporary, output)
}

/// Media duration of a finished local file, when ffprobe can read one.
fn probe_duration_secs(path: &Path) -> Option<f64> {
    let output = tool("ffprobe")
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "json"])
        .arg(path)
        .output()
        .ok()?;
    let value: Value = serde_json::from_slice(&output.stdout).ok()?;
    value["format"]["duration"].as_str()?.parse().ok()
}

/// Measures the published recording; `capture_secs` stands in for containers
/// whose duration ffprobe cannot read.
fn record_summary(output: &Path, capture_secs: f64) -> Result<RecordSummary> {
    let bytes = fs::metadata(output)?.len();
    let duration_secs = probe_duration_secs(output)
        .filter(|duration| *duration > 0.0)
        .unwrap_or(capture_secs);
    Ok(RecordSummary { duration_secs, bytes })
}

/// Records a live manifest until `stop` is set or `max_duration` elapses.
///
/// Live streams stop-and-save: the recording is written to a crash-safe
/// container next to the destination (fragmented MP4 for `.mp4`, Matroska for
/// `.mkv`) and ffmpeg finishes gracefully on stop so the file stays playable.
/// The stream ending by itself (HLS `#EXT-X-ENDLIST`) finishes normally. A
/// stop on a VOD keeps the pause semantics of the download jobs, as does a
/// stop that arrives before anything was recorded; `max_duration` only
/// applies to live captures. Track indices follow [`inspect_with_headers`];
/// when ffprobe cannot open a live manifest (some dynamic DASH setups) they
/// are passed to ffmpeg unvalidated.
#[allow(clippy::too_many_arguments)]
pub fn record_live(
    url: &str,
    output: &Path,
    video: usize,
    audio: Option<usize>,
    headers: &[(String, String)],
    stop: &AtomicBool,
    max_duration: Option<Duration>,
) -> Result<RecordSummary> {
    let muxer = match output.extension().and_then(|value| value.to_str()) {
        Some(value) if value.eq_ignore_ascii_case("mp4") => "mp4",
        Some(value) if value.eq_ignore_ascii_case("mkv") => "matroska",
        _ => return Err(err("media output must have an .mp4 or .mkv extension")),
    };
    if output.exists() {
        return Err(err("media destination already exists"));
    }
    if stop.load(Ordering::Relaxed) {
        return Err(err("paused"));
    }
    let relay = start_relay(url, headers)?;
    let report = inspect_report_in(url, &relay).ok();
    let live = match &report {
        Some(report) => report.live,
        // ffprobe can fail on live manifests; the walk alone decides.
        None => walk_manifest_report(url, &relay)?.live,
    };
    if let Some(report) = &report {
        if !report
            .tracks
            .iter()
            .any(|track| track.kind == "video" && track.index == video)
        {
            return Err(err("selected video track is unavailable"));
        }
        if audio.is_some_and(|index| {
            !report
                .tracks
                .iter()
                .any(|track| track.kind == "audio" && track.index == index)
        }) {
            return Err(err("selected audio track is unavailable"));
        }
    }
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let temporary = part_path(output);
    if temporary.exists() {
        return Err(err("media temporary output already exists"));
    }
    let mut command = tool("ffmpeg");
    if !live {
        // VOD recordings pause like every other job; no stdin interaction.
        command.arg("-nostdin");
    }
    command.args(["-hide_banner", "-v", "error"]);
    command.args(CLEAR_INPUT_ARGS);
    command.arg("-i").arg(relay.manifest_url());
    command.args(["-map", &format!("0:{video}")]);
    if let Some(audio) = audio {
        command.args(["-map", &format!("0:{audio}")]);
    }
    command.args(["-c", "copy"]);
    if muxer == "mp4" {
        // ADTS audio from TS needs the filter; already-ASC packets pass through.
        if audio.is_some_and(|index| {
            report
                .as_ref()
                .is_some_and(|report| {
                    report.tracks.iter().any(|track| {
                        track.kind == "audio" && track.index == index
                            && track.codec.eq_ignore_ascii_case("aac")
                    })
                })
        }) {
            command.args(["-bsf:a", "aac_adtstoasc"]);
        }
        command.args(["-movflags", "+frag_keyframe+empty_moov"]);
    }
    command.args(["-f", muxer, "-y"]).arg(&temporary);
    let started = Instant::now();
    if live {
        run_live_ffmpeg_job(command, &temporary, output, max_duration, stop)?;
    } else {
        run_ffmpeg_job(command, &temporary, output, stop)?;
    }
    record_summary(output, started.elapsed().as_secs_f64())
}

pub fn download(url: &str, output: &Path, video: usize, audio: Option<usize>) -> Result<()> {
    download_with_stop(url, output, video, audio, &AtomicBool::new(false), false)
}

/// Run a clear stream job; interrupted jobs restart from the manifest on resume.
pub fn download_with_stop(
    url: &str,
    output: &Path,
    video: usize,
    audio: Option<usize>,
    stop: &AtomicBool,
    recover_stale_part: bool,
) -> Result<()> {
    download_with_headers(
        url,
        output,
        video,
        audio,
        &[],
        &[],
        stop,
        recover_stale_part,
    )
}

/// Web headers plus subtitle stream indices to mux into the output (MP4 stores
/// them as mov_text, MKV copies them); one input per HLS subtitle rendition
/// because master-opened subtitle streams lose their packets in some ffmpeg builds.
#[allow(clippy::too_many_arguments)]
pub fn download_with_headers(
    url: &str,
    output: &Path,
    video: usize,
    audio: Option<usize>,
    subtitles: &[usize],
    headers: &[(String, String)],
    stop: &AtomicBool,
    recover_stale_part: bool,
) -> Result<()> {
    let muxer = match output.extension().and_then(|value| value.to_str()) {
        Some(value) if value.eq_ignore_ascii_case("mp4") => "mp4",
        Some(value) if value.eq_ignore_ascii_case("mkv") => "matroska",
        _ => return Err(err("media output must have an .mp4 or .mkv extension")),
    };
    if output.exists() {
        return Err(err("media destination already exists"));
    }
    let relay = start_relay(url, headers)?;
    let tracks = inspect_in(url, &relay)?;
    if !tracks
        .iter()
        .any(|track| track.kind == "video" && track.index == video)
    {
        return Err(err("selected video track is unavailable"));
    }
    if audio.is_some_and(|index| {
        !tracks
            .iter()
            .any(|track| track.kind == "audio" && track.index == index)
    }) {
        return Err(err("selected audio track is unavailable"));
    }
    let renditions = walk_manifest(url, &relay)?;
    let subtitle_tracks: Vec<usize> = tracks
        .iter()
        .filter(|track| track.kind == "subtitle")
        .map(|track| track.index)
        .collect();
    let mut chosen: Vec<(usize, usize)> = subtitles
        .iter()
        .map(|&index| {
            let slot = subtitle_tracks
                .iter()
                .position(|&track| track == index)
                .ok_or_else(|| err("selected subtitle track is unavailable"))?;
            Ok((slot, index))
        })
        .collect::<Result<Vec<_>>>()?;
    chosen.sort_unstable();
    chosen.dedup();
    if !chosen.is_empty() && chosen.iter().any(|&(slot, _)| slot >= renditions.len()) {
        return Err(err("a selected subtitle track has no rendition playlist"));
    }
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let temporary = part_path(output);
    if temporary.exists() {
        if recover_stale_part {
            fs::remove_file(&temporary)?;
        } else {
            return Err(err("media temporary output already exists"));
        }
    }
    let mut command = tool("ffmpeg");
    command.args(["-nostdin", "-hide_banner", "-v", "error"]);
    command.args(CLEAR_INPUT_ARGS);
    command.arg("-i").arg(relay.manifest_url());
    // Each selected subtitle track rides in as its own rendition playlist
    // input: master-opened subtitle streams lose their packets.
    for &(slot, _) in &chosen {
        command.args(CLEAR_INPUT_ARGS);
        command.arg("-i").arg(relay.url_for(&renditions[slot])?);
    }
    command.args(["-map", &format!("0:{video}")]);
    if let Some(audio) = audio {
        command.args(["-map", &format!("0:{audio}")]);
    }
    for offset in 0..chosen.len() {
        command.args(["-map", &format!("{}:0", offset + 1)]);
    }
    command.args(["-c", "copy"]);
    if !chosen.is_empty() {
        command.args(["-c:s", if muxer == "mp4" { "mov_text" } else { "copy" }]);
    }
    command.args(["-f", muxer, "-y"]).arg(&temporary);
    run_ffmpeg_job(command, &temporary, output, stop)
}

/// Stream-copies one video-only and one audio-only local file into `output`
/// (mp4, mkv or webm by extension). The inputs are plain files produced by the
/// segmented HTTP engine, so ffmpeg is limited to the `file` protocol here.
pub fn merge_streams(video: &Path, audio: &Path, output: &Path, stop: &AtomicBool) -> Result<()> {
    let muxer = match output.extension().and_then(|value| value.to_str()) {
        Some(value) if value.eq_ignore_ascii_case("mp4") => "mp4",
        Some(value) if value.eq_ignore_ascii_case("mkv") => "matroska",
        Some(value) if value.eq_ignore_ascii_case("webm") => "webm",
        _ => return Err(err("merged output must have an .mp4, .mkv or .webm extension")),
    };
    if output.exists() {
        return Err(err("media destination already exists"));
    }
    let temporary = part_path(output);
    let _ = fs::remove_file(&temporary);
    let mut command = tool("ffmpeg");
    command.args(["-nostdin", "-hide_banner", "-v", "error", "-protocol_whitelist", "file"]);
    command.arg("-i").arg(video);
    command.args(["-protocol_whitelist", "file"]);
    command.arg("-i").arg(audio);
    command.args(["-map", "0:v:0", "-map", "1:a:0", "-c", "copy"]);
    if muxer == "mp4" {
        command.args(["-movflags", "+faststart"]);
    }
    command.args(["-f", muxer, "-y"]).arg(&temporary);
    run_ffmpeg_job(command, &temporary, output, stop)
}

pub fn download_audio(
    url: &str,
    output: &Path,
    audio_track: usize,
    format: AudioFormat,
    headers: &[(String, String)],
    stop: &AtomicBool,
) -> Result<()> {
    let relay = start_relay(url, headers)?;
    if output.exists() {
        return Err(err("audio destination already exists"));
    }
    let extension = output
        .extension()
        .and_then(|value| value.to_str())
        .ok_or_else(|| err("audio output needs a file extension"))?;
    let tracks = inspect_in(url, &relay)?;
    let source = tracks
        .iter()
        .find(|track| track.kind == "audio" && track.index == audio_track)
        .map(|track| track.codec.clone())
        .ok_or_else(|| err("selected audio track is unavailable"))?;
    let plan = plan_audio(format, extension, &source)?;
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let temporary = part_path(output);
    if temporary.exists() {
        return Err(err("audio temporary output already exists"));
    }
    let mut command = tool("ffmpeg");
    command.args(["-nostdin", "-hide_banner", "-v", "error"]);
    command.args(CLEAR_INPUT_ARGS);
    command.args(["-i", &relay.manifest_url(), "-map", &format!("0:{audio_track}")]);
    match plan.reencode {
        Some((encoder, bitrate_kbps)) => command.args([
            "-c:a",
            encoder,
            "-b:a",
            &format!("{bitrate_kbps}k"),
        ]),
        None => command.args(["-c:a", "copy"]),
    };
    command.args(["-f", plan.muxer, "-y"]).arg(&temporary);
    run_ffmpeg_job(command, &temporary, output, stop)
}

pub fn save_subtitles(
    url: &str,
    output: &Path,
    track: usize,
    headers: &[(String, String)],
    stop: &AtomicBool,
) -> Result<()> {
    let relay = start_relay(url, headers)?;
    if output.exists() {
        return Err(err("subtitle destination already exists"));
    }
    let extension = output
        .extension()
        .and_then(|value| value.to_str())
        .ok_or_else(|| err("subtitle sidecar needs a file extension"))?;
    let (encoder, muxer) = if extension.eq_ignore_ascii_case("srt") {
        ("srt", "srt")
    } else if extension.eq_ignore_ascii_case("vtt") {
        ("webvtt", "webvtt")
    } else {
        return Err(err("subtitle sidecar extension must be .srt or .vtt"));
    };
    let tracks = inspect_in(url, &relay)?;
    let subtitle_tracks: Vec<usize> = tracks
        .iter()
        .filter(|track| track.kind == "subtitle")
        .map(|track| track.index)
        .collect();
    let slot = subtitle_tracks
        .iter()
        .position(|&index| index == track)
        .ok_or_else(|| err("selected subtitle track is unavailable"))?;
    let renditions = walk_manifest(url, &relay)?;
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let temporary = part_path(output);
    if temporary.exists() {
        return Err(err("subtitle temporary output already exists"));
    }
    let mut command = tool("ffmpeg");
    command.args(["-nostdin", "-hide_banner", "-v", "error"]);
    command.args(CLEAR_INPUT_ARGS);
    if renditions.len() > slot {
        // Demux the rendition playlist itself: master-opened subtitle streams
        // lose their packets in current ffmpeg builds.
        command.args(["-i", &relay.url_for(&renditions[slot])?]);
        command.args(["-map", "0:0"]);
    } else {
        command.args(["-i", &relay.manifest_url(), "-map", &format!("0:{track}")]);
    }
    command.args(["-c:s", encoder, "-f", muxer, "-y"]).arg(&temporary);
    run_ffmpeg_job(command, &temporary, output, stop)
}

/// Extensions the named clear codec can be stream-copied into.
fn copy_extensions(codec: &str) -> Option<&'static [&'static str]> {
    match codec {
        "aac" => Some(&["m4a"]),
        "opus" => Some(&["opus", "webm"]),
        "mp3" => Some(&["mp3"]),
        _ => None,
    }
}

/// ffmpeg muxer behind the audio-only extensions we accept.
fn audio_muxer(extension: &str) -> Result<&'static str> {
    Some(extension)
        .filter(|value| value.eq_ignore_ascii_case("m4a"))
        .map(|_| "mp4")
        .or_else(|| {
            Some(extension)
                .filter(|value| value.eq_ignore_ascii_case("opus"))
                .map(|_| "ogg")
        })
        .or_else(|| {
            Some(extension)
                .filter(|value| value.eq_ignore_ascii_case("webm"))
                .map(|_| "webm")
        })
        .or_else(|| {
            Some(extension)
                .filter(|value| value.eq_ignore_ascii_case("mp3"))
                .map(|_| "mp3")
        })
        .ok_or_else(|| err("audio output extension must be .m4a, .mp3, .opus, or .webm"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AudioPlan {
    muxer: &'static str,
    /// (ffmpeg encoder, bitrate in kbps) for re-encode targets.
    reencode: Option<(&'static str, u32)>,
}

fn checked_bitrate(bitrate_kbps: u32) -> Result<u32> {
    if (8..=320).contains(&bitrate_kbps) {
        Ok(bitrate_kbps)
    } else {
        Err(err("audio bitrate must be 8..=320 kbps"))
    }
}

/// An ffmpeg audio job: the output muxer plus either stream copy or a re-encode.
fn plan_audio(format: AudioFormat, extension: &str, source_codec: &str) -> Result<AudioPlan> {
    let matches = |wanted: &[&str]| {
        wanted
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(extension))
    };
    Ok(match format {
        AudioFormat::Original => {
            let allowed = copy_extensions(source_codec).ok_or_else(|| {
                err(format!(
                    "stream copy needs an AAC, MP3, or Opus audio track; encode {source_codec} with Mp3, M4a, or Opus instead"
                ))
            })?;
            if !matches(allowed) {
                return Err(err(format!(
                    "{source_codec} audio does not fit .{extension}; copy it to {}",
                    allowed.join(" or ")
                )));
            }
            AudioPlan { muxer: audio_muxer(extension)?, reencode: None }
        }
        AudioFormat::Mp3 { bitrate_kbps } => {
            if !matches(&["mp3"]) {
                return Err(err("Mp3 output needs an .mp3 extension"));
            }
            AudioPlan { muxer: "mp3", reencode: Some(("libmp3lame", checked_bitrate(bitrate_kbps)?)) }
        }
        AudioFormat::M4a { bitrate_kbps } => {
            if !matches(&["m4a"]) {
                return Err(err("M4a output needs an .m4a extension"));
            }
            AudioPlan { muxer: "mp4", reencode: Some(("aac", checked_bitrate(bitrate_kbps)?)) }
        }
        AudioFormat::Opus { bitrate_kbps } => {
            if !matches(&["opus", "webm"]) {
                return Err(err("Opus output needs an .opus or .webm extension"));
            }
            AudioPlan {
                muxer: audio_muxer(extension)?,
                reencode: Some(("libopus", checked_bitrate(bitrate_kbps)?)),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_manifest_urls_enter_stream_pipeline() {
        assert_eq!(manifest_kind("https://example.test/path/master.m3u8?token=x").unwrap(), "hls");
        assert_eq!(manifest_kind("https://example.test/manifest.mpd").unwrap(), "dash");
        assert!(manifest_kind("https://example.test/video.mp4").is_err());
    }

    #[test]
    fn newline_in_headers_is_rejected() {
        assert!(validate_headers(&[("Cookie".to_owned(), "a\r\nEvil: x".to_owned())]).is_err());
        assert!(validate_headers(&[("Bad\nName".to_owned(), "v".to_owned())]).is_err());
    }

    #[test]
    fn track_metadata_reads_subtitle_and_audio_values() {
        let data = serde_json::json!({"streams": [
            {"index": 0, "codec_type": "video", "codec_name": "h264", "width": 320, "height": 180,
             "tags": {"variant_bitrate": "600000"}},
            {"index": 1, "codec_type": "audio", "codec_name": "aac", "bit_rate": "64000",
             "sample_rate": "48000", "channels": 2, "disposition": {"default": 0},
             "tags": {"language": "und"}},
            {"index": 2, "codec_type": "subtitle", "codec_name": "webvtt",
             "disposition": {"default": 1}, "tags": {"language": "en"}},
            {"index": 9, "codec_type": "data", "codec_name": "bin_data"},
        ]});
        let tracks = tracks_from_probe(&data).unwrap();
        assert_eq!(tracks.len(), 3);
        assert_eq!(tracks[0].bitrate, Some(600000));
        assert_eq!(tracks[1].kind, "audio");
        assert_eq!(tracks[1].sample_rate, Some(48000));
        assert_eq!(tracks[1].channels, Some(2));
        assert_eq!(tracks[2].kind, "subtitle");
        assert_eq!(tracks[2].language.as_deref(), Some("en"));
        assert_eq!(tracks[2].default, Some(true));
        assert_eq!(tracks[1].default, Some(false));
    }

    #[test]
    fn older_track_json_keeps_deserializing() {
        let old: Track = serde_json::from_value(serde_json::json!({
            "index": 3, "kind": "audio", "codec": "aac",
            "width": null, "height": null, "bitrate": 64000, "language": null
        }))
        .unwrap();
        assert_eq!(old.sample_rate, None);
        assert_eq!(old.channels, None);
        assert_eq!(old.default, None);
    }

    #[test]
    fn audio_format_json_round_trips() {
        let value = serde_json::to_value(AudioFormat::Mp3 { bitrate_kbps: 128 }).unwrap();
        assert_eq!(value["kind"], "mp3");
        assert_eq!(value["bitrate_kbps"], 128);
        let back: AudioFormat = serde_json::from_value(value).unwrap();
        assert_eq!(back, AudioFormat::Mp3 { bitrate_kbps: 128 });
        let original: AudioFormat = serde_json::from_value(serde_json::json!({"kind": "original"})).unwrap();
        assert_eq!(original, AudioFormat::Original);
    }

    #[test]
    fn audio_plans_match_codec_and_extension() {
        let original = plan_audio(AudioFormat::Original, "m4a", "aac").unwrap();
        assert_eq!(original.muxer, "mp4");
        assert_eq!(original.reencode, None);
        assert_eq!(plan_audio(AudioFormat::Original, "opus", "opus").unwrap().muxer, "ogg");
        assert_eq!(plan_audio(AudioFormat::Original, "WEBM", "opus").unwrap().muxer, "webm");
        assert_eq!(plan_audio(AudioFormat::Original, "mp3", "mp3").unwrap().muxer, "mp3");
        let transcoded = plan_audio(AudioFormat::Mp3 { bitrate_kbps: 128 }, "mp3", "aac").unwrap();
        assert_eq!(transcoded.muxer, "mp3");
        assert_eq!(transcoded.reencode, Some(("libmp3lame", 128)));
        assert_eq!(plan_audio(AudioFormat::Opus { bitrate_kbps: 96 }, "opus", "aac").unwrap().muxer, "ogg");
        assert_eq!(plan_audio(AudioFormat::M4a { bitrate_kbps: 64 }, "M4A", "mp3").unwrap().muxer, "mp4");
        assert!(plan_audio(AudioFormat::Original, "m4a", "eac3").is_err());
        assert!(plan_audio(AudioFormat::Original, "m4a", "opus").is_err());
        assert!(plan_audio(AudioFormat::Mp3 { bitrate_kbps: 8 }, "mp3", "aac").is_ok());
        assert!(plan_audio(AudioFormat::Mp3 { bitrate_kbps: 7 }, "mp3", "aac").is_err());
        assert!(plan_audio(AudioFormat::M4a { bitrate_kbps: 320 }, "m4a", "aac").is_ok());
        assert!(plan_audio(AudioFormat::M4a { bitrate_kbps: 400 }, "m4a", "aac").is_err());
        assert!(plan_audio(AudioFormat::Opus { bitrate_kbps: 0 }, "opus", "aac").is_err());
        // A disguised video-only render must not be accepted as audio.
        assert!(plan_audio(AudioFormat::Original, "mp4", "h264").is_err());
    }

    #[test]
    fn dash_dynamic_manifests_are_detected() {
        assert!(dash_is_dynamic(
            r#"<?xml version="1.0"?><MPD xmlns="urn:mpeg:dash" type="dynamic" profiles="p"><Period/></MPD>"#
        ));
        assert!(dash_is_dynamic("<MPD Type = 'Dynamic'>"));
        assert!(!dash_is_dynamic("<MPD type=\"static\"><Period/></MPD>"));
        assert!(!dash_is_dynamic("<MPD><Period/></MPD>"));
        assert!(!dash_is_dynamic("not a manifest"));
    }

    #[test]
    fn hls_liveness_comes_from_media_playlists_only() {
        assert_eq!(
            hls_playlist_liveness("#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXTINF:1,\na.ts\n"),
            Some(true)
        );
        assert_eq!(
            hls_playlist_liveness("#EXTM3U\n#EXTINF:1,\na.ts\n#EXT-X-ENDLIST\n"),
            Some(false)
        );
        assert_eq!(
            hls_playlist_liveness("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\nv.m3u8\n"),
            None
        );
    }

    #[test]
    fn inspection_json_keeps_old_shapes_working() {
        let track = serde_json::json!({
            "index": 0, "kind": "video", "codec": "h264",
            "width": null, "height": null, "bitrate": null, "language": null
        });
        let wrapped: Inspection = serde_json::from_value(serde_json::json!({
            "tracks": [track], "live": true
        }))
        .unwrap();
        assert_eq!(wrapped.tracks.len(), 1);
        assert!(wrapped.live);
        let bare: Inspection = serde_json::from_value(serde_json::json!([track])).unwrap();
        assert_eq!(bare.tracks.len(), 1);
        assert!(!bare.live);
        let without_live: Inspection =
            serde_json::from_value(serde_json::json!({"tracks": []})).unwrap();
        assert!(!without_live.live);
    }

    #[test]
    fn record_summary_json_round_trips() {
        let summary = RecordSummary { duration_secs: 4.5, bytes: 900 };
        let value = serde_json::to_value(summary).unwrap();
        assert_eq!(value["duration_secs"], 4.5);
        assert_eq!(value["bytes"], 900);
        assert_eq!(serde_json::from_value::<RecordSummary>(value).unwrap(), summary);
    }

    #[test]
    fn record_live_refuses_non_container_outputs() {
        let stop = AtomicBool::new(false);
        let error = record_live(
            "http://example.test/live.m3u8",
            Path::new("/tmp/xdm-media-record-test/rec.ts"),
            0,
            None,
            &[],
            &stop,
            None,
        )
        .unwrap_err();
        assert!(error.to_string().contains(".mp4 or .mkv"), "{error}");
    }

    #[test]
    fn finished_media_never_replaces_a_concurrent_destination() {
        let directory = std::env::temp_dir().join(format!("xdm-media-publish-{}", std::process::id()));
        let part = directory.join("out.part");
        let output = directory.join("out.bin");
        fs::create_dir_all(&directory).unwrap();
        // Success removes the part and places the file.
        fs::write(&part, b"payload").unwrap();
        publish(&part, &output).unwrap();
        assert!(!part.exists());
        assert_eq!(fs::read(&output).unwrap(), b"payload");
        // A destination created meanwhile is kept; the part survives for retrieval.
        fs::write(&part, b"later").unwrap();
        publish(&part, &output).unwrap_err();
        assert!(part.exists());
        assert_eq!(fs::read(&output).unwrap(), b"payload");
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn relay_rewrites_references_and_scopes_headers() {
        let base = reqwest::Url::parse("https://a.example/v/master.m3u8").unwrap();
        let headers = vec![("Cookie".to_owned(), "sid=1".to_owned())];
        let relay = Relay::start(&base, &headers).unwrap();
        let manifest = relay.manifest_url();
        assert!(manifest.starts_with("http://127.0.0.1:") && manifest.ends_with("/master.m3u8"));
        assert!(!manifest.contains("a.example"));
        let other = relay.url_for("https://b.example/seg.ts").unwrap();
        assert!(other.ends_with("/seg.ts") && !other.contains("b.example"));
        // 128+ bits of token in the path.
        let token = manifest.split('/').nth(3).unwrap();
        assert!(token.len() >= 32 && token.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
