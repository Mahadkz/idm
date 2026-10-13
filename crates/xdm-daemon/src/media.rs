use std::path::{Path, PathBuf};
use xdm_ipc::{MediaAudioFormat, MediaMode};

/// Web headers a media job replays: bounded count and lengths, no
/// line-breaking that could smuggle further ffmpeg options into the child.
pub(crate) fn valid_media_headers(headers: &[(String, String)]) -> bool {
    headers.len() <= 32
        && headers.iter().all(|(name, value)| {
            !name.is_empty()
                && name.len() <= 128
                && value.len() <= 8192
                && !name.contains(['\r', '\n', '\0'])
                && !value.contains(['\r', '\n', '\0'])
        })
}

/// Fields a `SubmitMediaJob` carries: the manifest-URL and track-index guards
/// of `SubmitMedia`, plus a mode-aware destination extension check. A live
/// recording accepts the same containers as a video mux and its catch cap
/// (`max_seconds`, when set) stays within one day.
pub(crate) fn valid_media_request(url: &str, output: &str, mode: &MediaMode) -> bool {
    let manifest = url.split('?').next().unwrap_or("").to_ascii_lowercase();
    let extension = std::path::Path::new(output)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    let bounded = match mode {
        MediaMode::Video {
            video,
            audio,
            subtitles,
        } => {
            *video <= 10000
                && audio.is_none_or(|index| index <= 10000)
                && subtitles.len() <= 16
                && subtitles.iter().all(|index| *index <= 10000)
        }
        MediaMode::Audio { track, .. } => *track <= 10000,
        MediaMode::Subtitles { track } => *track <= 10000,
        MediaMode::Live {
            video,
            audio,
            max_seconds,
        } => {
            *video <= 10000
                && audio.is_none_or(|index| index <= 10000)
                && max_seconds.is_none_or(|seconds| {
                    (1..=xdm_ipc::MAX_LIVE_SECONDS).contains(&seconds)
                })
        }
    };
    (manifest.ends_with(".m3u8") || manifest.ends_with(".mpd"))
        && mode.valid(extension)
        && bounded
}

/// The crash-safe temporary file xdm-media records a live capture into before
/// publishing; it mirrors the engine's private `part_path` so the work runner
/// can report recorded bytes while ffmpeg is still writing.
pub(crate) fn media_part_path(output: &Path) -> PathBuf {
    let extension = output
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("media");
    output.with_extension(format!("{extension}.xdm-part"))
}

/// Stored-mode decode: rows written before `media_mode_json` exist are the
/// legacy `SubmitMedia` video job, and a corrupt or unknown JSON falls back to
/// the legacy interpretation when the columns still name a video track.
pub(crate) fn stored_media_mode(
    media_mode_json: Option<String>,
    media_video: Option<i64>,
    media_audio: Option<i64>,
) -> Option<MediaMode> {
    match media_mode_json.as_deref() {
        Some(text) => serde_json::from_str(text).ok(),
        None => Some(MediaMode::Video {
            video: media_video? as usize,
            audio: media_audio.map(|index| index as usize),
            subtitles: Vec::new(),
        }),
    }
}

/// xdm-media's engine format for a mirrored IPC format; byte values pass
/// through unchanged because the serde shapes are identical.
pub(crate) fn engine_audio_format(format: MediaAudioFormat) -> xdm_media::AudioFormat {
    match format {
        MediaAudioFormat::Original => xdm_media::AudioFormat::Original,
        MediaAudioFormat::Mp3 { bitrate_kbps } => {
            xdm_media::AudioFormat::Mp3 { bitrate_kbps }
        }
        MediaAudioFormat::M4a { bitrate_kbps } => xdm_media::AudioFormat::M4a { bitrate_kbps },
        MediaAudioFormat::Opus { bitrate_kbps } => {
            xdm_media::AudioFormat::Opus { bitrate_kbps }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::*;
    use xdm_ipc::Command;

    #[test]
    fn live_jobs_validate_containers_and_catch_cap() {
        let (service, _, _) = service(1000);
        let live = |request_id: &str, output: &str, mode: MediaMode| -> bool {
            service
                .command(Command::SubmitMediaJob {
                    request_id: request_id.into(),
                    url: "http://127.0.0.1:1/master.m3u8".into(),
                    output: output.into(),
                    headers: Vec::new(),
                    mode,
                    priority: 1,
                })
                .unwrap()
                .ok
        };
        // A live recording targets the containers xdm-media's recorder muxes.
        assert!(live(
            "live-mp4",
            "/nonexistent-xdm-test/take.mp4",
            MediaMode::Live { video: 0, audio: Some(1), max_seconds: Some(60) }
        ));
        assert!(live(
            "live-mkv",
            "/nonexistent-xdm-test/take.mkv",
            MediaMode::Live { video: 0, audio: None, max_seconds: None }
        ));
        // Sidecars, raw extensions, zeroed or oversized caps, and unbounded
        // track indices stay rejected.
        assert!(!live(
            "live-srt",
            "/nonexistent-xdm-test/take.srt",
            MediaMode::Live { video: 0, audio: None, max_seconds: None }
        ));
        assert!(!live(
            "live-ts",
            "/nonexistent-xdm-test/take.ts",
            MediaMode::Live { video: 0, audio: None, max_seconds: None }
        ));
        assert!(!live(
            "live-zero-cap",
            "/nonexistent-xdm-test/take.mp4",
            MediaMode::Live { video: 0, audio: None, max_seconds: Some(0) }
        ));
        assert!(!live(
            "live-over-cap",
            "/nonexistent-xdm-test/take.mp4",
            MediaMode::Live { video: 0, audio: None, max_seconds: Some(xdm_ipc::MAX_LIVE_SECONDS + 1) }
        ));
        assert!(!live(
            "live-bad-track",
            "/nonexistent-xdm-test/take.mp4",
            MediaMode::Live { video: 20000, audio: None, max_seconds: None }
        ));
    }

    /// Live tasks grid the queued row into the Video category and record the
    /// video and audio selectors in the legacy columns.
    #[test]
    fn live_jobs_persist_mode_and_video_category() {
        let (service, _, _) = service(1000);
        let reply = service
            .command(Command::SubmitMediaJob {
                request_id: "live-cat".into(),
                url: "http://127.0.0.1:1/master.m3u8".into(),
                output: "/nonexistent-xdm-test/live.mkv".into(),
                headers: Vec::new(),
                mode: MediaMode::Live { video: 2, audio: Some(3), max_seconds: Some(30) },
                priority: 1,
            })
            .unwrap();
        assert!(reply.ok, "{:?}", reply.error);
        let inner = service.inner.lock().unwrap();
        let (category, video, audio, mode_json): (String, Option<i64>, Option<i64>, String) = inner
            .db
            .query_row(
                "SELECT category,media_video,media_audio,media_mode_json FROM tasks WHERE id=?1",
                [reply.task_id.unwrap()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        drop(inner);
        assert_eq!(category, "Video");
        assert_eq!((video, audio), (Some(2), Some(3)));
        let mode: MediaMode = serde_json::from_str(&mode_json).unwrap();
        assert_eq!(mode, MediaMode::Live { video: 2, audio: Some(3), max_seconds: Some(30) });
    }

    #[test]
    fn media_job_validates_modes_and_persists_mode_and_headers() {
        let (service, _, _) = service(1000);
        let audio = |request_id: &str, output: &str, format: MediaAudioFormat| {
            service
                .command(Command::SubmitMediaJob {
                    request_id: request_id.into(),
                    url: "http://127.0.0.1:1/master.m3u8".into(),
                    output: output.into(),
                    headers: vec![("Cookie".into(), "xdm-test=1".into())],
                    mode: MediaMode::Audio { track: 1, format },
                    priority: 1,
                })
                .unwrap()
        };
        let accepted = audio(
            "m-audio",
            "/nonexistent-xdm-test/song.mp3",
            MediaAudioFormat::Mp3 { bitrate_kbps: 96 },
        );
        assert!(accepted.ok, "{:?}", accepted.error);
        // Identical replay is idempotent; a different target is rejected.
        assert!(accepted.task_id.unwrap() >= 1);
        assert!(
            audio(
                "m-audio",
                "/nonexistent-xdm-test/song.mp3",
                MediaAudioFormat::Mp3 { bitrate_kbps: 96 }
            )
            .ok
        );
        assert!(
            !audio(
                "m-audio",
                "/nonexistent-xdm-test/song.mp3",
                MediaAudioFormat::Mp3 { bitrate_kbps: 128 }
            )
            .ok
        );
        for (name, output, format) in [
            (
                "bad-ext",
                "/nonexistent-xdm-test/song.m4a",
                MediaAudioFormat::Mp3 { bitrate_kbps: 96 },
            ),
            (
                "bad-bitrate",
                "/nonexistent-xdm-test/song.mp3",
                MediaAudioFormat::Mp3 { bitrate_kbps: 400 },
            ),
            (
                "original-wrong-container",
                "/nonexistent-xdm-test/song.srt",
                MediaAudioFormat::Original,
            ),
        ] {
            assert!(!audio(name, output, format).ok, "{name}");
        }
        // Video modes with subtitles can target MKV; sidecars need .srt/.vtt.
        for (name, output, mode) in [
            (
                "mkv-subs-ok",
                "/nonexistent-xdm-test/film.mkv",
                MediaMode::Video {
                    video: 2,
                    audio: None,
                    subtitles: vec![3, 4],
                },
            ),
            (
                "subs-ok",
                "/nonexistent-xdm-test/film.vtt",
                MediaMode::Subtitles { track: 3 },
            ),
            (
                "video-no-manifest",
                "/nonexistent-xdm-test/film.mp4",
                MediaMode::Video {
                    video: 2,
                    audio: None,
                    subtitles: Vec::new(),
                },
            ),
        ] {
            let reply = service
                .command(Command::SubmitMediaJob {
                    request_id: name.into(),
                    url: if name == "video-no-manifest" {
                        "http://127.0.0.1:1/plain.mp4".into()
                    } else {
                        "http://127.0.0.1:1/master.m3u8".into()
                    },
                    output: output.into(),
                    headers: Vec::new(),
                    mode,
                    priority: 1,
                })
                .unwrap();
            assert_eq!(
                reply.ok,
                name == "mkv-subs-ok" || name == "subs-ok",
                "{name}: {:?}",
                (output, reply.error)
            );
        }
        // Line-breaking headers and a media job whose output is relative stay out.
        assert!(
            !service
                .command(Command::SubmitMediaJob {
                    request_id: "m-evil".into(),
                    url: "http://127.0.0.1:1/master.m3u8".into(),
                    output: "/nonexistent-xdm-test/evil.mp4".into(),
                    headers: vec![("Cookie".into(), "a\r\nEvil: x".into())],
                    mode: MediaMode::Video {
                        video: 0,
                        audio: None,
                        subtitles: Vec::new()
                    },
                    priority: 1,
                })
                .unwrap()
                .ok
        );
        assert!(
            !service
                .command(Command::SubmitMediaJob {
                    request_id: "m-relative".into(),
                    url: "http://127.0.0.1:1/master.m3u8".into(),
                    output: "song.mp3".into(),
                    headers: Vec::new(),
                    mode: MediaMode::Audio {
                        track: 1,
                        format: MediaAudioFormat::Mp3 { bitrate_kbps: 96 }
                    },
                    priority: 1,
                })
                .unwrap()
                .ok
        );
        let row: String = {
            let inner = service.inner.lock().unwrap();
            inner
                .db
                .query_row(
                    "SELECT media_mode_json FROM tasks WHERE id=?1",
                    [accepted.task_id.unwrap()],
                    |row| row.get(0),
                )
                .unwrap()
        };
        assert_eq!(
            row,
            r#"{"kind":"audio","track":1,"format":{"kind":"mp3","bitrate_kbps":96}}"#
        );
    }
}
