//! End-to-end audio-only and subtitle checks against locally served HLS fixtures.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::thread;

use xdm_media::{download_audio, download_with_headers, inspect, inspect_with_headers, save_subtitles, AudioFormat, Track};

const COOKIE: &str = "xdm-e2e=granted";
const SUBTITLE_TEXT: &[&str] = &["hello", "world"];
const NO_STOP: AtomicBool = AtomicBool::new(false);

static OPEN: OnceLock<(PathBuf, String)> = OnceLock::new();
static LOCKED: OnceLock<(PathBuf, String)> = OnceLock::new();

/// One fixture directory plus one server per variant, built once for all tests.
fn open_fixture() -> &'static (PathBuf, String) {
    OPEN.get_or_init(|| fixture("open", None))
}

fn locked_fixture() -> &'static (PathBuf, String) {
    LOCKED.get_or_init(|| fixture("locked", Some(COOKIE)))
}

fn fixture(name: &'static str, cookie: Option<&'static str>) -> (PathBuf, String) {
    let folder = scratch(name);
    generate(&folder);
    let base = start_server(&folder, cookie);
    (folder, base)
}

fn scratch(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("xdm-media-e2e-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    directory
}

fn start_server(folder: &Path, cookie: Option<&'static str>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let cookie = cookie.map(str::to_owned);
    let folder = folder.to_path_buf();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (folder, cookie) = (folder.clone(), cookie.clone());
            thread::spawn(move || {
                let _ = serve_connection(stream, &folder, cookie.as_deref());
            });
        }
    });
    base
}

fn serve_connection(
    stream: std::net::TcpStream,
    folder: &Path,
    cookie: Option<&str>,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(());
    }
    let target = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
    let mut authorized = cookie.is_none();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" {
            break;
        }
        if let Some(required) = cookie {
            let lowered = line.to_ascii_lowercase();
            if let Some(rest) = lowered.strip_prefix("cookie:") {
                if rest.trim().contains(required) {
                    authorized = true;
                }
            }
        }
    }
    let requested = target.split('?').next().unwrap_or("/");
    let file = folder.join(requested.trim_start_matches('/'));
    let mut response = if !authorized {
        b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
    } else {
        match fs::read(&file) {
            Ok(bytes) => {
                let mut response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    bytes.len()
                )
                .into_bytes();
                response.extend(bytes);
                response
            }
            Err(_) => {
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_vec()
            }
        }
    };
    let mut writer = stream;
    writer.write_all(&mut response)?;
    Ok(())
}

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

fn file_in(folder: &Path, name: &str) -> String {
    folder.join(name).to_string_lossy().into_owned()
}

fn generate(folder: &Path) {
    run_ffmpeg(&[
        "-f".into(), "lavfi".into(),
        "-i".into(), "testsrc2=size=320x180:rate=10".into(),
        "-t".into(), "2".into(),
        "-c:v".into(), "mpeg2video".into(),
        "-f".into(), "mpegts".into(),
        file_in(folder, "v180.ts"),
    ]);
    run_ffmpeg(&[
        "-f".into(), "lavfi".into(),
        "-i".into(), "sine=frequency=440:sample_rate=48000".into(),
        "-t".into(), "2".into(),
        "-c:a".into(), "aac".into(),
        "-f".into(), "mpegts".into(),
        file_in(folder, "audio.ts"),
    ]);
    fs::write(
        folder.join("subs.vtt"),
        "WEBVTT\n\n00:00:00.000 --> 00:00:01.000\nhello\n\n00:00:01.000 --> 00:00:02.000\nworld\n",
    )
    .unwrap();
    fs::write(
        folder.join("subs.m3u8"),
        "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:1,\nsubs.vtt\n#EXTINF:1,\nsubs.vtt\n#EXT-X-ENDLIST\n",
    )
    .unwrap();
    fs::write(
        folder.join("audio.m3u8"),
        "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\naudio.ts\n#EXT-X-ENDLIST\n",
    )
    .unwrap();
    fs::write(
        folder.join("v180.m3u8"),
        "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\nv180.ts\nv180.ts\n#EXT-X-ENDLIST\n",
    )
    .unwrap();
    fs::write(
        folder.join("master.m3u8"),
        "#EXTM3U\n\
         #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"sound\",DEFAULT=YES,AUTOSELECT=YES,LANGUAGE=\"en\",URI=\"audio.m3u8\"\n\
         #EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"s\",NAME=\"subs\",DEFAULT=YES,AUTOSELECT=YES,FORCED=NO,LANGUAGE=\"en\",URI=\"subs.m3u8\"\n\
         #EXT-X-STREAM-INF:BANDWIDTH=500000,RESOLUTION=320x180,AUDIO=\"a\",SUBTITLES=\"s\"\nv180.m3u8\n",
    )
    .unwrap();
}

fn master_url(base: &str) -> String {
    format!("{base}/master.m3u8")
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

fn probe_duration(path: &Path) -> f64 {
    let output = Command::new("ffprobe")
        .args(["-v", "error", "-show_format", "-of", "json"])
        .arg(path)
        .output()
        .unwrap();
    serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["format"]["duration"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
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

fn audio_track(tracks: &[Track]) -> Track {
    tracks.iter().find(|t| t.kind == "audio").unwrap().clone()
}

fn video_track(tracks: &[Track]) -> Track {
    tracks.iter().find(|t| t.kind == "video").unwrap().clone()
}

fn subtitle_track(tracks: &[Track]) -> Track {
    tracks.iter().find(|t| t.kind == "subtitle").unwrap().clone()
}

/// Re-extracts subtitle text from a finished container.
fn subtitle_text(path: &Path) -> String {
    let output = Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(path)
        .args(["-map", "0:s", "-c:s", "srt", "-y", "-f", "srt", "-"])
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn cookie_header() -> Vec<(String, String)> {
    vec![("Cookie".to_owned(), COOKIE.to_owned())]
}

#[test]
fn inspect_lists_tracks_with_metadata() {
    let (_, base) = open_fixture();
    let tracks = inspect(&master_url(base)).unwrap();
    let video = video_track(&tracks);
    assert_eq!(video.codec, "mpeg2video");
    assert_eq!(video.height, Some(180));
    let audio = audio_track(&tracks);
    assert_eq!(audio.codec, "aac");
    let subtitle = subtitle_track(&tracks);
    assert_eq!(subtitle.codec, "webvtt");
    assert_eq!(subtitle.language.as_deref(), Some("en"));
    assert_eq!(subtitle.default, Some(true));
}

#[test]
fn audio_mp3_encode_matches_source_duration() {
    let (_, base) = open_fixture();
    let url = master_url(base);
    let audio = audio_track(&inspect(&url).unwrap());
    let folder = scratch("audio-mp3-out");
    let output = folder.join("song.mp3");
    download_audio(
        &url,
        &output,
        audio.index,
        AudioFormat::Mp3 { bitrate_kbps: 96 },
        &[],
        &AtomicBool::new(false),
    )
    .unwrap();
    let stream = &probe_streams(&output)["streams"][0];
    assert_eq!(stream["codec_type"], "audio");
    assert_eq!(stream["codec_name"], "mp3");
    let duration = probe_duration(&output);
    assert!((1.6..=2.5).contains(&duration), "unexpected duration {duration}");
    decodes_cleanly(&output);
}

#[test]
fn audio_m4a_stream_copy_and_encode() {
    let (_, base) = open_fixture();
    let url = master_url(base);
    let audio = audio_track(&inspect(&url).unwrap());
    let folder = scratch("audio-m4a-out");
    let copied = folder.join("copied.m4a");
    download_audio(&url, &copied, audio.index, AudioFormat::Original, &[], &AtomicBool::new(false))
        .unwrap();
    let stream = &probe_streams(&copied)["streams"][0];
    assert_eq!(stream["codec_type"], "audio");
    assert_eq!(stream["codec_name"], "aac");
    assert!((1.6..=2.5).contains(&probe_duration(&copied)));
    decodes_cleanly(&copied);
    let encoded = folder.join("encoded.m4a");
    download_audio(
        &url,
        &encoded,
        audio.index,
        AudioFormat::M4a { bitrate_kbps: 128 },
        &[],
        &AtomicBool::new(false),
    )
    .unwrap();
    decodes_cleanly(&encoded);
}

#[test]
fn audio_opus_variants() {
    let (_, base) = open_fixture();
    let url = master_url(base);
    let audio = audio_track(&inspect(&url).unwrap());
    let folder = scratch("audio-opus-out");
    let encoded = folder.join("song.opus");
    download_audio(
        &url,
        &encoded,
        audio.index,
        AudioFormat::Opus { bitrate_kbps: 64 },
        &[],
        &AtomicBool::new(false),
    )
    .unwrap();
    let stream = &probe_streams(&encoded)["streams"][0];
    assert_eq!(stream["codec_name"], "opus");
    decodes_cleanly(&encoded);
    // A container extension that does not fit the requested format is refused.
    let guarded = folder.join("song.mp3");
    let error = download_audio(
        &url,
        &guarded,
        audio.index,
        AudioFormat::Opus { bitrate_kbps: 64 },
        &[],
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(error.to_string().contains(".opus or .webm"), "{error}");
    // AAC tracks cannot be stream-copied into .webm.
    let moved = folder.join("moved.webm");
    let error =
        download_audio(&url, &moved, audio.index, AudioFormat::Original, &[], &AtomicBool::new(false))
            .unwrap_err();
    assert!(error.to_string().contains("does not fit"), "{error}");
    assert!(error.to_string().contains(".webm"), "{error}");
}

#[test]
fn subtitles_mux_into_mp4_and_mkv_with_sidecars() {
    let (_, base) = open_fixture();
    let url = master_url(base);
    let tracks = inspect(&url).unwrap();
    let video = video_track(&tracks);
    let audio = audio_track(&tracks);
    let subtitle = subtitle_track(&tracks);
    let folder = scratch("subs-mux-out");

    let mp4 = folder.join("movie.mp4");
    let srt = folder.join("movie.srt");
    download_with_headers(
        &url,
        &mp4,
        video.index,
        Some(audio.index),
        &[subtitle.index],
        &[],
        &NO_STOP,
        false,
    )
    .unwrap();
    let probed = probe_streams(&mp4);
    let subtitle_stream = probed["streams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["codec_type"] == "subtitle")
        .unwrap();
    assert_eq!(subtitle_stream["codec_name"], "mov_text");
    save_subtitles(&url, &srt, subtitle.index, &[], &NO_STOP).unwrap();
    let text = fs::read_to_string(&srt).unwrap();
    for marker in SUBTITLE_TEXT {
        assert!(text.contains(marker), "missing {marker} in {text}");
    }
    let muxed = subtitle_text(&mp4);
    for marker in SUBTITLE_TEXT {
        assert!(muxed.contains(marker), "missing {marker} in muxed text");
    }
    decodes_cleanly(&mp4);

    let mkv = folder.join("movie.mkv");
    let vtt = folder.join("movie.vtt");
    download_with_headers(
        &url,
        &mkv,
        video.index,
        Some(audio.index),
        &[subtitle.index],
        &[],
        &NO_STOP,
        false,
    )
    .unwrap();
    let probed = probe_streams(&mkv);
    let subtitle_stream = probed["streams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["codec_type"] == "subtitle")
        .unwrap();
    assert_eq!(subtitle_stream["codec_name"], "webvtt");
    save_subtitles(&url, &vtt, subtitle.index, &[], &NO_STOP).unwrap();
    let text = fs::read_to_string(&vtt).unwrap();
    assert!(text.starts_with("WEBVTT"));
    for marker in SUBTITLE_TEXT {
        assert!(text.contains(marker));
    }
    decodes_cleanly(&mkv);
}

#[test]
fn protected_manifest_needs_the_cookie() {
    let (_, base) = locked_fixture();
    let url = master_url(base);
    // Without headers both inspection and download fail at the 403.
    let error = inspect(&url).unwrap_err();
    assert!(error.to_string().contains("403"), "unexpected error: {error}");
    let error = download_audio(
        &url,
        &scratch("cookie-out").join("missing.mp3"),
        0,
        AudioFormat::Mp3 { bitrate_kbps: 96 },
        &[],
        &NO_STOP,
    )
    .unwrap_err();
    assert!(error.to_string().contains("403"), "unexpected error: {error}");
    // With the required Cookie both inspection and downloads work.
    let tracks = inspect_with_headers(&url, &cookie_header()).unwrap();
    let audio = audio_track(&tracks);
    let folder = scratch("cookie-out");
    let output = folder.join("song.mp3");
    download_audio(
        &url,
        &output,
        audio.index,
        AudioFormat::Mp3 { bitrate_kbps: 96 },
        &cookie_header(),
        &NO_STOP,
    )
    .unwrap();
    decodes_cleanly(&output);
    let video = video_track(&tracks);
    let subtitle = subtitle_track(&tracks);
    let mkv = folder.join("safe.mkv");
    download_with_headers(
        &url,
        &mkv,
        video.index,
        Some(audio.index),
        &[subtitle.index],
        &cookie_header(),
        &NO_STOP,
        false,
    )
    .unwrap();
    decodes_cleanly(&mkv);
}

#[test]
fn jobs_refuse_bad_requests() {
    let (_, base) = open_fixture();
    let url = master_url(base);
    let tracks = inspect(&url).unwrap();
    let audio = audio_track(&tracks);
    let video = video_track(&tracks);
    let folder = scratch("jobs-refusals-out");

    // .mp3 output with a stream-copy request for an AAC track.
    let wrong_extension = folder.join("song.mp3");
    let error =
        download_audio(&url, &wrong_extension, audio.index, AudioFormat::Original, &[], &NO_STOP)
            .unwrap_err();
    assert!(error.to_string().contains("does not fit"), "{error}");
    assert!(!wrong_extension.exists());

    // A video track index is not an audio track.
    let output = folder.join("song2.mp3");
    let error = download_audio(
        &url,
        &output,
        video.index,
        AudioFormat::Mp3 { bitrate_kbps: 96 },
        &[],
        &NO_STOP,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("selected audio track is unavailable"),
        "{error}"
    );
    assert!(!output.exists());

    // Existing destinations are never overwritten.
    let existing = folder.join("existing.mp3");
    fs::write(&existing, b"keep").unwrap();
    let error = download_audio(
        &url,
        &existing,
        audio.index,
        AudioFormat::Mp3 { bitrate_kbps: 96 },
        &[],
        &NO_STOP,
    )
    .unwrap_err();
    assert!(error.to_string().contains("already exists"), "{error}");
    assert_eq!(fs::read(&existing).unwrap(), b"keep");

    // A stop flag set before the job starts pauses it without artifacts.
    let paused = folder.join("paused.mp3");
    let stop = AtomicBool::new(true);
    let error = download_audio(
        &url,
        &paused,
        audio.index,
        AudioFormat::Mp3 { bitrate_kbps: 96 },
        &[],
        &stop,
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "paused");
    assert!(!paused.exists() && !folder.join("paused.mp3.xdm-part").exists());
    stop.store(false, Ordering::Relaxed);

    // A sidecar without an .srt/.vtt extension is refused.
    let output = folder.join("notes.txt");
    let error = save_subtitles(&url, &output, 1, &[], &NO_STOP).unwrap_err();
    assert!(error.to_string().contains(".srt or .vtt"), "{error}");
    assert!(!output.exists());
}
