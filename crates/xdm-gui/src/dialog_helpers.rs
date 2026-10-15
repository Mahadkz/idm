use xdm_categories::CategoryRule;
use xdm_grabber::Project;
use xdm_ipc::{ImportEntry, ProxyChoice};

// --- Batch Download Helpers ---

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchPreview {
    pub count: usize,
    pub first: String,
    pub last: String,
    pub valid: bool,
    pub message: String,
}

pub fn preview_batch(template: &str) -> BatchPreview {
    let trimmed = template.trim();
    if trimmed.is_empty() {
        return BatchPreview {
            count: 0,
            first: "First: —".into(),
            last: "Last: —".into(),
            valid: false,
            message: "Enter a URL template with ranges like [1-10]".into(),
        };
    }
    match xdm_transfer_list::expand_batch(trimmed, 10_000) {
        Ok(urls) if !urls.is_empty() => {
            let count = urls.len();
            let first = urls.first().cloned().unwrap_or_default();
            let last = urls.last().cloned().unwrap_or_default();
            BatchPreview {
                count,
                first: format!("First: {first}"),
                last: format!("Last: {last}"),
                valid: true,
                message: format!("Valid template: {count} URL(s) generated"),
            }
        }
        Ok(_) => BatchPreview {
            count: 0,
            first: "First: —".into(),
            last: "Last: —".into(),
            valid: false,
            message: "Template produced 0 URLs".into(),
        },
        Err(err) => BatchPreview {
            count: 0,
            first: "First: —".into(),
            last: "Last: —".into(),
            valid: false,
            message: format!("Invalid template: {err}"),
        },
    }
}

pub fn build_batch_entries(template: &str) -> Result<Vec<ImportEntry>, String> {
    let trimmed = template.trim();
    let urls = xdm_transfer_list::expand_batch(trimmed, 10_000)?;
    if urls.is_empty() {
        return Err("No URLs produced by template".into());
    }
    let list = xdm_transfer_list::parse(&urls.join("\n"))?;
    Ok(list.entries)
}

// --- Site Grabber Wizard Helpers ---

pub fn build_grabber_project(
    start_url: &str,
    depth_str: &str,
    max_pages_str: &str,
    file_types_str: &str,
    include_str: &str,
    exclude_str: &str,
    same_host: bool,
    obey_robots: bool,
) -> Result<Project, String> {
    let url = start_url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("Start URL must begin with http:// or https://".into());
    }
    let max_depth = depth_str
        .trim()
        .parse::<u32>()
        .map_err(|_| "Invalid crawl depth: must be a positive integer")?;
    let max_pages = max_pages_str
        .trim()
        .parse::<usize>()
        .map_err(|_| "Invalid max pages: must be a positive integer")?;
    let file_types = file_types_str
        .split([',', ' ', ';'])
        .map(|s| s.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    let include_patterns = include_str
        .split([',', ';'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let exclude_patterns = exclude_str
        .split([',', ';'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    Ok(Project {
        start_url: url.to_string(),
        max_depth,
        max_pages,
        same_host_only: same_host,
        allowed_hosts: Vec::new(),
        include_patterns,
        exclude_patterns,
        file_types,
        headers: Vec::new(),
        request_delay_ms: 0,
        max_file_count: 1000,
        obey_robots,
        ..Project::default()
    })
}

// --- Categories Editor Helpers ---

pub fn parse_extensions(input: &str) -> Vec<String> {
    input
        .split([',', ' ', ';'])
        .map(|s| s.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn parse_sites(input: &str) -> Vec<String> {
    input
        .split([',', ';'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn format_category_rule(rule: &CategoryRule) -> String {
    let exts = if rule.extensions.is_empty() {
        "—".to_string()
    } else {
        rule.extensions.join(", ")
    };
    let dest = if rule.destination.is_empty() {
        "default".to_string()
    } else {
        rule.destination.clone()
    };
    let sites = if rule.sites.is_empty() {
        "—".to_string()
    } else {
        rule.sites.join(", ")
    };
    format!(
        "{:<12}  ·  ext: [{}]  ·  dest: {}  ·  sites: [{}]",
        rule.name, exts, dest, sites
    )
}

// --- Connection / Proxy Settings Helpers ---

pub fn parse_bypass(input: &str) -> Vec<String> {
    input
        .split([',', ';'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn build_proxy_choice(
    mode: &str,
    http: &str,
    https: &str,
    bypass: &str,
) -> Result<ProxyChoice, String> {
    match mode {
        "system" => Ok(ProxyChoice::System),
        "direct" => Ok(ProxyChoice::Direct),
        "manual" => {
            let http_opt = if http.trim().is_empty() {
                None
            } else {
                Some(http.trim().to_string())
            };
            let https_opt = if https.trim().is_empty() {
                None
            } else {
                Some(https.trim().to_string())
            };
            if http_opt.is_none() && https_opt.is_none() {
                return Err("Specify at least an HTTP or HTTPS proxy URL for manual proxy".into());
            }
            let bypass_vec = parse_bypass(bypass);
            Ok(ProxyChoice::Manual {
                http: http_opt,
                https: https_opt,
                bypass: bypass_vec,
            })
        }
        _ => Err(format!("Unknown proxy mode: {mode}")),
    }
}

// --- URL Validation Helpers ---

pub fn validate_download_url(url: &str) -> Result<String, String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err("URL cannot be empty".into());
    }
    if !(trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("ftp://"))
    {
        return Err("URL must start with http://, https://, or ftp://".into());
    }
    Ok(trimmed.to_string())
}

// --- Media Stream Dialog Helpers ---

use std::path::PathBuf;
use xdm_ipc::{MediaAudioFormat, MediaMode, MAX_LIVE_SECONDS};

pub fn parse_header_lines(input: &str) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            let n = name.trim();
            let v = value.trim();
            if !n.is_empty()
                && n.len() <= 128
                && v.len() <= 8192
                && !n.contains(['\r', '\n', '\0'])
                && !v.contains(['\r', '\n', '\0'])
            {
                headers.push((n.to_string(), v.to_string()));
            }
        }
        if headers.len() >= 32 {
            break;
        }
    }
    headers
}

pub fn parse_track_indices(input: &str) -> Vec<usize> {
    input
        .split([',', ';', ' '])
        .filter_map(|s| s.trim().parse::<usize>().ok())
        .collect()
}

pub fn format_media_track(track: &xdm_media::Track) -> String {
    let kind = track.kind.to_ascii_lowercase();
    let lang = track.language.as_deref().unwrap_or("und");
    let def = if track.default == Some(true) { " [default]" } else { "" };
    match kind.as_str() {
        "video" => {
            let res = match (track.width, track.height) {
                (Some(w), Some(h)) => format!("{w}x{h}"),
                _ => "unknown-res".to_string(),
            };
            let br = track
                .bitrate
                .map(|b| format!("{} kbps", b / 1000))
                .unwrap_or_else(|| "—".into());
            format!(
                "#{} [video] {} · {} · {} · lang: {}{def}",
                track.index, track.codec, res, br, lang
            )
        }
        "audio" => {
            let br = track
                .bitrate
                .map(|b| format!("{} kbps", b / 1000))
                .unwrap_or_else(|| "—".into());
            let sr = track
                .sample_rate
                .map(|s| format!("{s} Hz"))
                .unwrap_or_else(|| "—".into());
            let ch = track
                .channels
                .map(|c| format!("{c}ch"))
                .unwrap_or_else(|| "—".into());
            format!(
                "#{} [audio] {} · {} · {} · {} · lang: {}{def}",
                track.index, track.codec, br, sr, ch, lang
            )
        }
        "subtitle" => {
            format!(
                "#{} [subtitles] {} · lang: {}{def}",
                track.index, track.codec, lang
            )
        }
        _ => format!(
            "#{} [{}] {} · lang: {}{def}",
            track.index, track.kind, track.codec, lang
        ),
    }
}

pub fn build_media_mode(
    mode: &str,
    video_idx: Option<usize>,
    audio_idx: Option<usize>,
    subtitles_str: &str,
    audio_format: &str,
    audio_bitrate_str: &str,
    max_minutes_str: &str,
    is_live: bool,
) -> Result<MediaMode, String> {
    match mode {
        "video" => {
            let video = video_idx.ok_or("Video mode requires a selected video track")?;
            let subtitles = parse_track_indices(subtitles_str);
            Ok(MediaMode::Video {
                video,
                audio: audio_idx,
                subtitles,
            })
        }
        "audio" => {
            let track = audio_idx.ok_or("Audio mode requires a selected audio track")?;
            let format = match audio_format.to_ascii_lowercase().as_str() {
                "original" => MediaAudioFormat::Original,
                "mp3" => {
                    let bitrate = audio_bitrate_str
                        .trim()
                        .parse::<u32>()
                        .map_err(|_| "Invalid MP3 bitrate (must be 8..320)")?;
                    if !(8..=320).contains(&bitrate) {
                        return Err("MP3 bitrate must be between 8 and 320 kbps".into());
                    }
                    MediaAudioFormat::Mp3 {
                        bitrate_kbps: bitrate,
                    }
                }
                "m4a" => {
                    let bitrate = audio_bitrate_str
                        .trim()
                        .parse::<u32>()
                        .map_err(|_| "Invalid M4A bitrate (must be 8..320)")?;
                    if !(8..=320).contains(&bitrate) {
                        return Err("M4A bitrate must be between 8 and 320 kbps".into());
                    }
                    MediaAudioFormat::M4a {
                        bitrate_kbps: bitrate,
                    }
                }
                "opus" => {
                    let bitrate = audio_bitrate_str
                        .trim()
                        .parse::<u32>()
                        .map_err(|_| "Invalid Opus bitrate (must be 8..320)")?;
                    if !(8..=320).contains(&bitrate) {
                        return Err("Opus bitrate must be between 8 and 320 kbps".into());
                    }
                    MediaAudioFormat::Opus {
                        bitrate_kbps: bitrate,
                    }
                }
                other => return Err(format!("Unknown audio format: {other}")),
            };
            Ok(MediaMode::Audio { track, format })
        }
        "subtitles" => {
            let indices = parse_track_indices(subtitles_str);
            let track = indices
                .first()
                .copied()
                .ok_or("Subtitles mode requires a selected subtitle track")?;
            Ok(MediaMode::Subtitles { track })
        }
        "live" => {
            if !is_live {
                return Err("Live recording mode is only available for live streams".into());
            }
            let video = video_idx.ok_or("Live recording requires a video track")?;
            let max_seconds = if max_minutes_str.trim().is_empty() {
                None
            } else {
                let mins = max_minutes_str
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| "Max minutes must be a positive integer")?;
                let secs = mins.checked_mul(60).ok_or("Max minutes overflow")?;
                if secs > MAX_LIVE_SECONDS {
                    return Err(format!(
                        "Max minutes exceeds maximum recording duration ({} minutes)",
                        MAX_LIVE_SECONDS / 60
                    ));
                }
                Some(secs)
            };
            Ok(MediaMode::Live {
                video,
                audio: audio_idx,
                max_seconds,
            })
        }
        _ => Err(format!("Unknown media mode: {mode}")),
    }
}

pub fn suggest_media_filename(url: &str, mode: &MediaMode) -> String {
    let clean_url = url.split('?').next().unwrap_or("stream");
    let base_name = clean_url
        .rsplit('/')
        .next()
        .and_then(|seg| {
            seg.strip_suffix(".m3u8")
                .or_else(|| seg.strip_suffix(".mpd"))
                .or_else(|| seg.strip_suffix(".ts"))
        })
        .unwrap_or("media");
    let stem = if base_name.is_empty() {
        "media"
    } else {
        base_name
    };
    let ext = mode.extensions().first().copied().unwrap_or("mp4");
    format!("{stem}.{ext}")
}

pub fn validate_media_destination(destination: &str, mode: &MediaMode) -> Result<PathBuf, String> {
    let trimmed = destination.trim();
    if trimmed.is_empty() {
        return Err("Destination path cannot be empty".into());
    }
    let path = PathBuf::from(trimmed);
    if !path.is_absolute() {
        return Err("Destination path must be absolute".into());
    }
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    if !mode.valid(ext) {
        let allowed = mode.extensions().join(", .");
        return Err(format!(
            "Output extension '.{ext}' is invalid for this mode. Allowed: .{allowed}"
        ));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_batch_preview_success() {
        let preview = preview_batch("http://example.com/item[1-3].bin");
        assert!(preview.valid);
        assert_eq!(preview.count, 3);
        assert_eq!(preview.first, "First: http://example.com/item1.bin");
        assert_eq!(preview.last, "Last: http://example.com/item3.bin");
    }

    #[test]
    fn test_batch_preview_invalid() {
        let preview = preview_batch("http://example.com/item[1-a].bin");
        assert!(!preview.valid);
        assert_eq!(preview.count, 0);
    }

    #[test]
    fn test_build_batch_entries() {
        let entries = build_batch_entries("http://example.com/part[01-03].zip").unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].url, "http://example.com/part01.zip");
        assert_eq!(entries[2].url, "http://example.com/part03.zip");
    }

    #[test]
    fn test_grabber_project_creation() {
        let proj = build_grabber_project(
            "https://mysite.com",
            "3",
            "50",
            "zip, pdf",
            "*download*",
            "*thumb*",
            true,
            false,
        )
        .unwrap();
        assert_eq!(proj.start_url, "https://mysite.com");
        assert_eq!(proj.max_depth, 3);
        assert_eq!(proj.max_pages, 50);
        assert_eq!(proj.file_types, vec!["zip", "pdf"]);
        assert!(proj.same_host_only);
        assert!(!proj.obey_robots);
    }

    #[test]
    fn test_category_parsing() {
        let exts = parse_extensions(".tar, zip; gz   7z");
        assert_eq!(exts, vec!["tar", "zip", "gz", "7z"]);

        let sites = parse_sites("*.test.com, sub.example.org");
        assert_eq!(sites, vec!["*.test.com", "sub.example.org"]);
    }

    #[test]
    fn test_proxy_choice_modes() {
        assert_eq!(
            build_proxy_choice("system", "", "", "").unwrap(),
            ProxyChoice::System
        );
        assert_eq!(
            build_proxy_choice("direct", "", "", "").unwrap(),
            ProxyChoice::Direct
        );
        let manual = build_proxy_choice(
            "manual",
            "http://127.0.0.1:8080",
            "",
            "localhost, 127.0.0.1",
        )
        .unwrap();
        match manual {
            ProxyChoice::Manual {
                http,
                https,
                bypass,
            } => {
                assert_eq!(http, Some("http://127.0.0.1:8080".into()));
                assert_eq!(https, None);
                assert_eq!(bypass, vec!["localhost", "127.0.0.1"]);
            }
            _ => panic!("Expected manual proxy"),
        }
    }

    #[test]
    fn test_validate_download_url() {
        assert!(validate_download_url("http://example.com/file").is_ok());
        assert!(validate_download_url("https://example.com/file").is_ok());
        assert!(validate_download_url("ftp://example.com/file").is_ok());
        assert!(validate_download_url("ssh://example.com/file").is_err());
        assert!(validate_download_url("").is_err());
    }

    #[test]
    fn test_parse_header_lines() {
        let input = "Cookie: sid=123; user=foo\nReferer: https://example.com/watch\n\nCustom-Header: value";
        let headers = parse_header_lines(input);
        assert_eq!(headers.len(), 3);
        assert_eq!(headers[0], ("Cookie".into(), "sid=123; user=foo".into()));
        assert_eq!(headers[1], ("Referer".into(), "https://example.com/watch".into()));
        assert_eq!(headers[2], ("Custom-Header".into(), "value".into()));

        // Ignore lines without colon
        assert!(parse_header_lines("invalid line").is_empty());
    }

    #[test]
    fn test_parse_track_indices() {
        assert_eq!(parse_track_indices("0, 1, 2"), vec![0, 1, 2]);
        assert_eq!(parse_track_indices("3; 4 5"), vec![3, 4, 5]);
        assert_eq!(parse_track_indices("abc"), Vec::<usize>::new());
    }

    #[test]
    fn test_format_media_track() {
        let v_track = xdm_media::Track {
            index: 0,
            kind: "video".into(),
            codec: "h264".into(),
            width: Some(1920),
            height: Some(1080),
            bitrate: Some(2_500_000),
            language: Some("en".into()),
            sample_rate: None,
            channels: None,
            default: Some(true),
        };
        let formatted_v = format_media_track(&v_track);
        assert!(formatted_v.contains("#0 [video] h264"));
        assert!(formatted_v.contains("1920x1080"));
        assert!(formatted_v.contains("2500 kbps"));
        assert!(formatted_v.contains("lang: en"));
        assert!(formatted_v.contains("[default]"));

        let a_track = xdm_media::Track {
            index: 1,
            kind: "audio".into(),
            codec: "aac".into(),
            width: None,
            height: None,
            bitrate: Some(128_000),
            language: Some("es".into()),
            sample_rate: Some(48000),
            channels: Some(2),
            default: None,
        };
        let formatted_a = format_media_track(&a_track);
        assert!(formatted_a.contains("#1 [audio] aac"));
        assert!(formatted_a.contains("128 kbps"));
        assert!(formatted_a.contains("48000 Hz"));
        assert!(formatted_a.contains("2ch"));
        assert!(formatted_a.contains("lang: es"));
    }

    #[test]
    fn test_build_media_mode_video() {
        let mode = build_media_mode("video", Some(0), Some(1), "2, 3", "original", "", "", false).unwrap();
        assert_eq!(
            mode,
            MediaMode::Video {
                video: 0,
                audio: Some(1),
                subtitles: vec![2, 3],
            }
        );
    }

    #[test]
    fn test_build_media_mode_audio() {
        let orig = build_media_mode("audio", None, Some(1), "", "original", "", "", false).unwrap();
        assert_eq!(
            orig,
            MediaMode::Audio {
                track: 1,
                format: MediaAudioFormat::Original,
            }
        );

        let mp3 = build_media_mode("audio", None, Some(1), "", "mp3", "128", "", false).unwrap();
        assert_eq!(
            mp3,
            MediaMode::Audio {
                track: 1,
                format: MediaAudioFormat::Mp3 { bitrate_kbps: 128 },
            }
        );

        let m4a = build_media_mode("audio", None, Some(2), "", "m4a", "256", "", false).unwrap();
        assert_eq!(
            m4a,
            MediaMode::Audio {
                track: 2,
                format: MediaAudioFormat::M4a { bitrate_kbps: 256 },
            }
        );

        let opus = build_media_mode("audio", None, Some(3), "", "opus", "96", "", false).unwrap();
        assert_eq!(
            opus,
            MediaMode::Audio {
                track: 3,
                format: MediaAudioFormat::Opus { bitrate_kbps: 96 },
            }
        );

        // Invalid bitrate
        assert!(build_media_mode("audio", None, Some(1), "", "mp3", "5", "", false).is_err());
        assert!(build_media_mode("audio", None, Some(1), "", "mp3", "500", "", false).is_err());
    }

    #[test]
    fn test_build_media_mode_subtitles() {
        let sub = build_media_mode("subtitles", None, None, "4", "", "", "", false).unwrap();
        assert_eq!(sub, MediaMode::Subtitles { track: 4 });
    }

    #[test]
    fn test_build_media_mode_live() {
        // Not live should fail
        assert!(build_media_mode("live", Some(0), Some(1), "", "", "", "30", false).is_err());

        // Live stream with 30 min duration
        let live = build_media_mode("live", Some(0), Some(1), "", "", "", "30", true).unwrap();
        assert_eq!(
            live,
            MediaMode::Live {
                video: 0,
                audio: Some(1),
                max_seconds: Some(1800),
            }
        );

        // Unlimited duration
        let live_unlimited = build_media_mode("live", Some(0), None, "", "", "", "", true).unwrap();
        assert_eq!(
            live_unlimited,
            MediaMode::Live {
                video: 0,
                audio: None,
                max_seconds: None,
            }
        );
    }

    #[test]
    fn test_suggest_media_filename() {
        let video_mode = MediaMode::Video {
            video: 0,
            audio: None,
            subtitles: vec![],
        };
        assert_eq!(
            suggest_media_filename("http://example.com/playlist.m3u8", &video_mode),
            "playlist.mp4"
        );

        let audio_mode = MediaMode::Audio {
            track: 1,
            format: MediaAudioFormat::Mp3 { bitrate_kbps: 128 },
        };
        assert_eq!(
            suggest_media_filename("http://example.com/live/master.m3u8?token=xyz", &audio_mode),
            "master.mp3"
        );
    }

    #[test]
    fn test_validate_media_destination() {
        let video_mode = MediaMode::Video {
            video: 0,
            audio: None,
            subtitles: vec![],
        };
        assert!(validate_media_destination("/tmp/video.mp4", &video_mode).is_ok());
        assert!(validate_media_destination("/tmp/video.mkv", &video_mode).is_ok());
        // Wrong extension for video mode
        assert!(validate_media_destination("/tmp/video.mp3", &video_mode).is_err());
        // Relative path
        assert!(validate_media_destination("video.mp4", &video_mode).is_err());
    }
}
