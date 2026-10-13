//! Live-recording checks against a local sliding-window HLS feed: stop and
//! save, max duration, self-completion on ENDLIST, and VOD pause semantics.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use xdm_media::{inspect_report_with_headers, record_live};

/// Segment count of the shared fixture pool; every recording stays well below
/// the pool length so the playlist keeps growing and ffmpeg never stalls.
const SEGMENT_POOL: usize = 40;
const WINDOW: usize = 6;

static POOL: OnceLock<PathBuf> = OnceLock::new();

fn run_ffmpeg(args: &[String]) {
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "ffmpeg {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

/// One shared segment pool for all tests: 1 s muxed h264+aac slices with
/// globally continuous timestamps.
fn segment_pool() -> &'static PathBuf {
    POOL.get_or_init(|| {
        let folder =
            std::env::temp_dir().join(format!("xdm-media-live-pool-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        run_ffmpeg(&[
            "-f".into(), "lavfi".into(),
            "-i".into(), "testsrc2=size=160x90:rate=10".into(),
            "-f".into(), "lavfi".into(),
            "-i".into(), "sine=frequency=440:sample_rate=48000".into(),
            "-t".into(), format!("{SEGMENT_POOL}"),
            "-c:v".into(), "libx264".into(),
            "-preset".into(), "ultrafast".into(),
            "-pix_fmt".into(), "yuv420p".into(),
            "-g".into(), "10".into(),
            "-c:a".into(), "aac".into(),
            "-b:a".into(), "32k".into(),
            "-f".into(), "segment".into(),
            "-segment_time".into(), "1".into(),
            "-reset_timestamps".into(), "0".into(),
            folder.join("seg%02d.ts").to_string_lossy().into_owned(),
        ]);
        folder
    })
}

struct LiveFeed {
    folder: PathBuf,
    started: Mutex<Option<Instant>>,
    endlist_after: Option<u64>,
}

impl LiveFeed {
    /// Current playlist: `count` grows once per segment duration and the
    /// clock starts at the first fetch, so every recording sees a growing
    /// feed regardless of test order.
    fn playlist(&self) -> String {
        let mut started = self.started.lock().unwrap();
        let start = *started.get_or_insert_with(Instant::now);
        let finished = start.elapsed().as_secs();
        // The pool files are complete on disk, so the first segment is
        // served right away and the window grows once per second.
        let mut count = finished.max(1).min(SEGMENT_POOL as u64) as usize;
        let ended = self.endlist_after.is_some_and(|after| finished >= after);
        if let Some(after) = self.endlist_after {
            count = count.min(after as usize);
        }
        let first = count.saturating_sub(WINDOW);
        let mut lines = vec![
            "#EXTM3U".to_owned(),
            "#EXT-X-VERSION:3".to_owned(),
            "#EXT-X-TARGETDURATION:2".to_owned(),
            format!("#EXT-X-MEDIA-SEQUENCE:{first}"),
        ];
        for number in first..count {
            lines.push("#EXTINF:1.0,".to_owned());
            lines.push(format!("seg{number:02}.ts"));
        }
        if ended {
            lines.push("#EXT-X-ENDLIST".to_owned());
        }
        lines.join("\n") + "\n"
    }
}

fn start_live_server(endlist_after: Option<u64>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let feed = Arc::new(LiveFeed {
        folder: segment_pool().clone(),
        started: Mutex::new(None),
        endlist_after,
    });
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let feed = feed.clone();
            thread::spawn(move || {
                let _ = serve_connection(stream, &feed);
            });
        }
    });
    base
}

fn serve_connection(stream: TcpStream, feed: &LiveFeed) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(());
    }
    let requested = line
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .split('?')
        .next()
        .unwrap_or("/")
        .to_owned();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" {
            break;
        }
    }
    let (status, body) = match requested.as_str() {
        "/live.m3u8" | "/end.m3u8" => ("200 OK", feed.playlist().into_bytes()),
        "/vod.m3u8" => (
            "200 OK",
            "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:1.0,\nseg00.ts\n#EXTINF:1.0,\nseg01.ts\n#EXT-X-ENDLIST\n"
                .as_bytes()
                .to_vec(),
        ),
        path => match fs::read(feed.folder.join(path.trim_start_matches('/'))) {
            Ok(bytes) => ("200 OK", bytes),
            Err(_) => ("404 Not Found", Vec::new()),
        },
    };
    let mut response = format!(
        "HTTP/1.0 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    response.extend(body);
    let mut writer = stream;
    writer.write_all(&response)?;
    Ok(())
}

fn live_url(base: &str) -> String {
    format!("{base}/live.m3u8")
}

fn scratch(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("xdm-media-live-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    directory
}

fn probe_streams(path: &Path) -> serde_json::Value {
    let output = Command::new("ffprobe")
        .args(["-v", "error", "-show_streams", "-of", "json"])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "ffprobe {} failed: {}",
        path.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn decodes_cleanly(path: &Path) {
    let output = Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(path)
        .args(["-f", "null", "-"])
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "decode of {} failed: {}",
        path.display(),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn has_video_and_audio(path: &Path) {
    let streams = probe_streams(path)["streams"].as_array().unwrap().clone();
    assert!(
        streams.iter().any(|s| s["codec_type"] == "video"),
        "no video in {path:?}: {streams:?}"
    );
    assert!(
        streams.iter().any(|s| s["codec_type"] == "audio"),
        "no audio in {path:?}: {streams:?}"
    );
}

fn part_file(output: &Path) -> PathBuf {
    let extension = output.extension().unwrap().to_str().unwrap();
    output.with_extension(format!("{extension}.xdm-part"))
}

#[test]
fn inspect_flags_live_and_vod_playlists() {
    let base = start_live_server(None);
    let report = inspect_report_with_headers(&live_url(&base), &[]).unwrap();
    assert!(report.live, "rolling playlist must report live");
    assert!(report.tracks.iter().any(|t| t.kind == "video"));
    assert!(report.tracks.iter().any(|t| t.kind == "audio"));
    let vod = inspect_report_with_headers(&format!("{base}/vod.m3u8"), &[]).unwrap();
    assert!(!vod.live, "ended playlist must not report live");
}

#[test]
fn record_live_stop_saves_a_playable_file() {
    let base = start_live_server(None);
    let url = live_url(&base);
    let report = inspect_report_with_headers(&url, &[]).unwrap();
    let video = report.tracks.iter().find(|t| t.kind == "video").unwrap().index;
    let audio = report.tracks.iter().find(|t| t.kind == "audio").unwrap().index;

    let folder = scratch("stop-save");
    let output = folder.join("take.mkv");
    let stop = Arc::new(AtomicBool::new(false));
    let stopper = {
        let stop = stop.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(5));
            stop.store(true, Ordering::Relaxed);
        })
    };
    let summary = record_live(&url, &output, video, Some(audio), &[], &stop, None).unwrap();
    stopper.join().unwrap();
    assert!(summary.duration_secs >= 2.0, "too short: {summary:?}");
    assert!(summary.bytes > 0, "empty recording: {summary:?}");
    assert_eq!(fs::metadata(&output).unwrap().len(), summary.bytes);
    has_video_and_audio(&output);
    decodes_cleanly(&output);
    assert!(!part_file(&output).exists(), "temp file survived the publish");

    // An existing destination is never overwritten.
    let busy = folder.join("busy.mkv");
    fs::write(&busy, b"keep").unwrap();
    let error = record_live(&url, &busy, video, Some(audio), &[], &stop, None).unwrap_err();
    assert!(error.to_string().contains("already exists"), "{error}");
    assert_eq!(fs::read(&busy).unwrap(), b"keep");
}

#[test]
fn record_live_respects_max_duration() {
    let base = start_live_server(None);
    let url = live_url(&base);
    let report = inspect_report_with_headers(&url, &[]).unwrap();
    let video = report.tracks.iter().find(|t| t.kind == "video").unwrap().index;
    let audio = report.tracks.iter().find(|t| t.kind == "audio").unwrap().index;

    // The fragmented MP4 path must produce a playable file on its own.
    let folder = scratch("max-duration");
    let output = folder.join("take.mp4");
    let started = Instant::now();
    let summary = record_live(
        &url,
        &output,
        video,
        Some(audio),
        &[],
        &AtomicBool::new(false),
        Some(Duration::from_secs(3)),
    )
    .unwrap();
    let elapsed = started.elapsed();
    assert!(summary.duration_secs >= 2.0, "too short: {summary:?}");
    assert!(
        elapsed < Duration::from_secs(15),
        "max_duration did not stop the job: {elapsed:?}"
    );
    has_video_and_audio(&output);
    decodes_cleanly(&output);
    assert!(!part_file(&output).exists());
}

#[test]
fn record_live_finishes_when_endlist_arrives() {
    let base = start_live_server(Some(3));
    let report = inspect_report_with_headers(&format!("{base}/end.m3u8"), &[]).unwrap();
    let video = report.tracks.iter().find(|t| t.kind == "video").unwrap().index;
    let audio = report.tracks.iter().find(|t| t.kind == "audio").unwrap().index;

    let folder = scratch("endlist");
    let output = folder.join("take.mkv");
    let summary = record_live(
        &format!("{base}/end.m3u8"),
        &output,
        video,
        Some(audio),
        &[],
        &AtomicBool::new(false),
        None,
    )
    .unwrap();
    assert!(
        (1.5..=6.0).contains(&summary.duration_secs),
        "unexpected duration: {summary:?}"
    );
    has_video_and_audio(&output);
    decodes_cleanly(&output);
    assert!(!part_file(&output).exists());
}

#[test]
fn record_live_vod_stop_is_a_pause() {
    let base = start_live_server(None);
    let report = inspect_report_with_headers(&format!("{base}/vod.m3u8"), &[]).unwrap();
    let video = report.tracks.iter().find(|t| t.kind == "video").unwrap().index;
    let audio = report.tracks.iter().find(|t| t.kind == "audio").unwrap().index;

    let folder = scratch("vod-pause");
    let output = folder.join("take.mkv");
    let stop = AtomicBool::new(true);
    let error = record_live(&format!("{base}/vod.m3u8"), &output, video, Some(audio), &[], &stop, None)
        .unwrap_err();
    assert_eq!(error.to_string(), "paused");
    assert!(!output.exists());
    assert!(!part_file(&output).exists());
}
