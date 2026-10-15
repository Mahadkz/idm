use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use xdm_http::{
    DEFAULT_STALL_TIMEOUT, Options, Proxy, TransferControl, download_with_stall_timeout,
};
use xdm_ipc::{Command, CompletionAction, MediaMode, MoveTarget, Paths, ProxyChoice};

fn main() {
    let mut args = std::env::args().skip(1).peekable();
    if args.peek().map(String::as_str) == Some("daemon") {
        args.next();
        daemon_command(args.collect());
        return;
    }
    if args.peek().map(String::as_str) == Some("grab") {
        args.next();
        grab_command(args.collect());
        return;
    }
    if args.peek().map(String::as_str) == Some("media") {
        args.next();
        media_command(args.collect());
        return;
    }
    let mut positional = Vec::new();
    let mut connections = 8usize;
    let mut restart = false;
    let mut headers = Vec::new();
    let mut bytes_per_second = None;
    let mut method = "GET".to_owned();
    let mut body = None;
    let mut proxy = Proxy::System;
    let mut stall_timeout = DEFAULT_STALL_TIMEOUT;
    let mut temp_dir: Option<PathBuf> = None;
    let mut set_file_time = true;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--no-proxy" => proxy = Proxy::Direct,
            "--proxy" => {
                let url = args.next().unwrap_or_else(|| usage());
                proxy = Proxy::Manual {
                    http: Some(url.clone()),
                    https: Some(url),
                    bypass: Vec::new(),
                };
            }
            "--proxy-bypass" => {
                let list = args.next().unwrap_or_else(|| usage());
                let Proxy::Manual { bypass, .. } = &mut proxy else {
                    usage()
                };
                bypass.extend(list.split(',').map(|host| host.trim().to_owned()));
            }
            "--method" => method = args.next().unwrap_or_else(|| usage()),
            "--data-file" => {
                let path = args.next().unwrap_or_else(|| usage());
                body = Some(std::fs::read(&path).unwrap_or_else(|error| {
                    eprintln!("read {path}: {error}");
                    std::process::exit(2)
                }));
                if method == "GET" {
                    method = "POST".to_owned();
                }
            }
            "--connections" => {
                connections = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or_else(|| usage());
            }
            "--restart" => restart = true,
            "--header" => {
                let line = args.next().unwrap_or_else(|| usage());
                let (name, value) = line.split_once(':').unwrap_or_else(|| usage());
                headers.push((name.trim().to_owned(), value.trim().to_owned()));
            }
            "--rate-kib" => {
                let kib = args
                    .next()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or_else(|| usage());
                bytes_per_second = Some(kib.checked_mul(1024).unwrap_or_else(|| usage()));
            }
            "--temp-dir" => {
                let dir = PathBuf::from(args.next().unwrap_or_else(|| usage()));
                if !dir.is_absolute() {
                    usage();
                }
                temp_dir = Some(dir);
            }
            "--no-server-time" => set_file_time = false,
            "--stall-timeout" => {
                let seconds = args
                    .next()
                    .and_then(|v| v.parse::<u64>().ok())
                    .filter(|v| (1..=600).contains(v))
                    .unwrap_or_else(|| usage());
                stall_timeout = Duration::from_secs(seconds);
            }
            "--help" | "-h" => usage(),
            _ if arg.starts_with('-') => usage(),
            _ => positional.push(arg),
        }
    }
    if positional.len() != 2 {
        usage();
    }
    let stop = Arc::new(AtomicBool::new(false));
    let signal = stop.clone();
    if let Err(error) =
        ctrlc::set_handler(move || signal.store(true, std::sync::atomic::Ordering::Relaxed))
    {
        eprintln!("could not install Ctrl-C handler: {error}");
        std::process::exit(2);
    }
    let output = PathBuf::from(&positional[1]);
    if positional[0].starts_with("ftp://") {
        let result = xdm_ftp::download(
            xdm_ftp::FtpOptions {
                url: positional[0].clone(),
                output: output.clone(),
                restart,
                stop,
                temp_dir: None,
                set_file_time: false,
            },
            |_, _| {},
        );
        match result {
            Ok(()) => println!("saved {}", output.display()),
            Err(error) => {
                eprintln!("download: {error}");
                std::process::exit(1);
            }
        }
        return;
    }
    let control = match TransferControl::new(bytes_per_second) {
        Ok(control) => Arc::new(control),
        Err(error) => {
            eprintln!("download: {error}");
            std::process::exit(1);
        }
    };
    match download_with_stall_timeout(
        Options {
            login: None,
            proxy_login: None,
            url: positional[0].clone(),
            output: output.clone(),
            connections,
            restart,
            headers,
            method,
            body,
            proxy,
            bytes_per_second,
            stop,
            temp_dir,
            set_file_time,
        },
        control,
        stall_timeout,
    ) {
        Ok(()) => println!("saved {}", output.display()),
        Err(error) => {
            eprintln!("download: {error}");
            std::process::exit(1);
        }
    }
}

/// Crawls a saved grabber project and prints the discovered files as JSON.
fn grab_command(args: Vec<String>) {
    let [path] = args.as_slice() else { usage() };
    let project: xdm_grabber::Project = std::fs::read(path)
        .map_err(|error| error.to_string())
        .and_then(|data| serde_json::from_slice(&data).map_err(|error| error.to_string()))
        .unwrap_or_else(|error| {
            eprintln!("grab: {path}: {error}");
            std::process::exit(2)
        });
    let stop = Arc::new(AtomicBool::new(false));
    let signal = stop.clone();
    let _ = ctrlc::set_handler(move || signal.store(true, std::sync::atomic::Ordering::Relaxed));
    match xdm_grabber::crawl(&project, stop) {
        Ok(result) => println!("{}", serde_json::to_string_pretty(&result).unwrap_or_default()),
        Err(error) => {
            eprintln!("grab: {error}");
            std::process::exit(1)
        }
    }
}

/// Parses `original|mp3:KBPS|m4a:KBPS|opus:KBPS` audio targets.
fn parse_audio_format(spec: &str) -> xdm_ipc::MediaAudioFormat {
    use xdm_ipc::MediaAudioFormat as F;
    if spec == "original" {
        return F::Original;
    }
    let (kind, kbps) = spec.split_once(':').unwrap_or_else(|| usage());
    let bitrate = kbps
        .parse::<u32>()
        .ok()
        .filter(|rate| (8..=320).contains(rate))
        .unwrap_or_else(|| usage());
    match kind {
        "mp3" => F::Mp3 { bitrate_kbps: bitrate },
        "m4a" => F::M4a { bitrate_kbps: bitrate },
        "opus" => F::Opus { bitrate_kbps: bitrate },
        _ => usage(),
    }
}

/// AudioFormat spec for direct (engine) calls.
fn parse_engine_audio_format(spec: &str) -> xdm_media::AudioFormat {
    match parse_audio_format(spec) {
        xdm_ipc::MediaAudioFormat::Original => xdm_media::AudioFormat::Original,
        xdm_ipc::MediaAudioFormat::Mp3 { bitrate_kbps } => {
            xdm_media::AudioFormat::Mp3 { bitrate_kbps }
        }
        xdm_ipc::MediaAudioFormat::M4a { bitrate_kbps } => {
            xdm_media::AudioFormat::M4a { bitrate_kbps }
        }
        xdm_ipc::MediaAudioFormat::Opus { bitrate_kbps } => {
            xdm_media::AudioFormat::Opus { bitrate_kbps }
        }
    }
}

/// Collects the flags `media` subcommands share and the remaining positional
/// arguments in command order.
struct MediaFlags {
    positional: Vec<String>,
    headers: Vec<(String, String)>,
    subtitles: Vec<usize>,
    /// `--max-seconds N` for live recordings; ignored by the other modes.
    max_seconds: Option<u64>,
}

fn media_flags(args: Vec<String>) -> MediaFlags {
    let mut positional = Vec::new();
    let mut headers = Vec::new();
    let mut subtitles = Vec::new();
    let mut max_seconds = None;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--header" => {
                let line = iter.next().unwrap_or_else(|| usage());
                let (name, value) = line.split_once(':').unwrap_or_else(|| usage());
                headers.push((name.trim().to_owned(), value.trim().to_owned()));
            }
            "--subtitles" => {
                let list = iter.next().unwrap_or_else(|| usage());
                for part in list.split(',') {
                    subtitles.push(part.trim().parse::<usize>().unwrap_or_else(|_| usage()));
                }
            }
            "--max-seconds" => {
                let seconds = iter.next().unwrap_or_else(|| usage());
                let seconds = seconds
                    .parse::<u64>()
                    .ok()
                    .filter(|value| (1..=xdm_ipc::MAX_LIVE_SECONDS).contains(value))
                    .unwrap_or_else(|| usage());
                max_seconds = Some(seconds);
            }
            _ => positional.push(arg),
        }
    }
    MediaFlags {
        positional,
        headers,
        subtitles,
        max_seconds,
    }
}

fn media_stop() -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let signal = stop.clone();
    let _ = ctrlc::set_handler(move || signal.store(true, std::sync::atomic::Ordering::Relaxed));
    stop
}

fn media_command(args: Vec<String>) {
    let Some(sub) = args.first().cloned() else { usage() };
    // Queue subcommands talk to the daemon; the rest run the engine directly.
    let queued = sub == "queue";
    let flags = media_flags(args.into_iter().skip(1).collect());
    let positional = flags.positional;
    if queued {
        match positional.as_slice() {
            [url, output, video] => submit_media_job(
                url,
                output,
                MediaMode::Video {
                    video: parse_number(video),
                    audio: None,
                    subtitles: flags.subtitles,
                },
                flags.headers,
            ),
            [url, output, video, audio] => submit_media_job(
                url,
                output,
                MediaMode::Video {
                    video: parse_number(video),
                    audio: Some(parse_number(audio)),
                    subtitles: flags.subtitles,
                },
                flags.headers,
            ),
            _ => usage(),
        }
        return;
    }
    let stop = media_stop();
    let headers = &flags.headers;
    let outcome = match (sub.as_str(), positional.len()) {
        ("inspect", 1) => xdm_media::inspect_report_with_headers(&positional[0], headers)
            .map(|report| {
                // The live verdict rides on stderr, so scripts reading the
                // track array from stdout stay unaffected.
                eprintln!("live: {}", report.live);
                serde_json::to_string_pretty(&report.tracks).unwrap_or_default()
            }),
        ("download", 3) | ("download", 4) => {
            xdm_media::download_with_headers(
                &positional[0],
                &PathBuf::from(&positional[1]),
                parse_number(&positional[2]),
                positional.get(3).map(|v| parse_number(v)),
                &flags.subtitles,
                headers,
                &stop,
                false,
            )
            .map(|_| format!("saved {}", positional[1]))
        }
        ("record", 3) | ("record", 4) => {
            xdm_media::record_live(
                &positional[0],
                &PathBuf::from(&positional[1]),
                parse_number(&positional[2]),
                positional.get(3).map(|v| parse_number(v)),
                headers,
                &stop,
                flags.max_seconds.map(std::time::Duration::from_secs),
            )
            .map(|summary| {
                format!(
                    "saved {} ({} bytes, {:.1}s)",
                    positional[1],
                    summary.bytes,
                    summary.duration_secs
                )
            })
        }
        ("audio", 4) => xdm_media::download_audio(
            &positional[0],
            &PathBuf::from(&positional[1]),
            parse_number(&positional[2]),
            parse_engine_audio_format(&positional[3]),
            headers,
            &stop,
        )
        .map(|_| format!("saved {}", positional[1])),
        ("subtitles", 3) => xdm_media::save_subtitles(
            &positional[0],
            &PathBuf::from(&positional[1]),
            parse_number(&positional[2]),
            headers,
            &stop,
        )
        .map(|_| format!("saved {}", positional[1])),
        _ => usage(),
    };
    match outcome {
        Ok(message) => println!("{message}"),
        Err(error) => {
            eprintln!("media {sub}: {error}");
            std::process::exit(1);
        }
    }
}

fn parse_number(text: &str) -> usize {
    text.parse::<usize>()
        .ok()
        .filter(|value| *value <= 10000)
        .unwrap_or_else(|| usage())
}

/// Queues a full media job with the daemon through `SubmitMediaJob`.
fn submit_media_job(url: &str, output: &str, mode: MediaMode, headers: Vec<(String, String)>) {
    send_daemon(Command::SubmitMediaJob {
        request_id: new_request_id("cli-media-job"),
        url: url.to_owned(),
        output: output.to_owned(),
        headers,
        mode,
        priority: 1,
    });
}

fn new_request_id(prefix: &str) -> String {
    format!(
        "{prefix}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}

fn daemon_command(args: Vec<String>) {
    match args.first().map(String::as_str) {
        // daemon media-job URL OUT video VIDEO [AUDIO] [--subtitles 1,2] [--header 'N: v']
        // daemon media-job URL OUT audio TRACK original|mp3:KBPS|m4a:KBPS|opus:KBPS [--header 'N: v']
        // daemon media-job URL OUT subtitles TRACK [--header 'N: v']
        // daemon media-job URL OUT live VIDEO [AUDIO] [--max-seconds N] [--header 'N: v']
        Some("media-job") if args.len() >= 5 => {
            let media = media_flags(args[3..].to_vec());
            let mode = match media.positional.as_slice() {
                [kind, video] if kind == "video" => MediaMode::Video {
                    video: parse_number(video),
                    audio: None,
                    subtitles: media.subtitles,
                },
                [kind, video, audio] if kind == "video" => MediaMode::Video {
                    video: parse_number(video),
                    audio: Some(parse_number(audio)),
                    subtitles: media.subtitles,
                },
                [kind, track, format] if kind == "audio" => MediaMode::Audio {
                    track: parse_number(track),
                    format: parse_audio_format(format),
                },
                [kind, track] if kind == "subtitles" => MediaMode::Subtitles {
                    track: parse_number(track),
                },
                [kind, video] if kind == "live" => MediaMode::Live {
                    video: parse_number(video),
                    audio: None,
                    max_seconds: media.max_seconds,
                },
                [kind, video, audio] if kind == "live" => MediaMode::Live {
                    video: parse_number(video),
                    audio: Some(parse_number(audio)),
                    max_seconds: media.max_seconds,
                },
                _ => usage(),
            };
            submit_media_job(&args[1], &args[2], mode, media.headers);
            return;
        }
        Some("preview") if args.len() == 2 => {
            match xdm_transfer_list::read(&PathBuf::from(&args[1])) {
                Ok(list) => println!("{}", serde_json::to_string_pretty(&list).unwrap()),
                Err(error) => {
                    eprintln!("preview: {error}");
                    std::process::exit(1);
                }
            }
            return;
        }
        Some("import") if (3..=5).contains(&args.len()) && args[3..].iter().all(|f| f == "--start" || f == "--allow-duplicates") => {
            let list = xdm_transfer_list::read(&PathBuf::from(&args[1])).unwrap_or_else(|error| {
                eprintln!("import: {error}");
                std::process::exit(1);
            });
            send_daemon(Command::ImportListEx {
                request_id: new_request_id("cli-import"),
                directory: args[2].clone(),
                entries: list.entries,
                enqueue: args[3..].iter().any(|f| f == "--start"),
                allow_duplicate: args[3..].iter().any(|f| f == "--allow-duplicates"),
            });
            return;
        }
        Some("playlist") if (3..=5).contains(&args.len()) && args[3..].iter().all(|f| f == "--start" || f == "--allow-duplicates") => {
            // M3U/M3U8/PLS playlist of plain files; HLS playlists are refused.
            let source = &args[1];
            let parsed = if source.starts_with("http://") || source.starts_with("https://") {
                xdm_grabber::fetch_text(source).and_then(|text| {
                    let base = url::Url::parse(source).map_err(|e| e.to_string())?;
                    xdm_transfer_list::parse_with_base(&text, Some(&base)).map_err(Into::into)
                })
            } else {
                xdm_transfer_list::read(&PathBuf::from(source)).map_err(Into::into)
            };
            let list = parsed.unwrap_or_else(|error| {
                eprintln!("playlist: {error}");
                std::process::exit(1);
            });
            send_daemon(Command::ImportListEx {
                request_id: new_request_id("cli-playlist"),
                directory: args[2].clone(),
                entries: list.entries,
                enqueue: args[3..].iter().any(|f| f == "--start"),
                allow_duplicate: args[3..].iter().any(|f| f == "--allow-duplicates"),
            });
            return;
        }
        Some("batch") if (3..=5).contains(&args.len()) && args[3..].iter().all(|f| f == "--start" || f == "--allow-duplicates") => {
            // IDM-style batch download: expand `[001-100]`-style groups into a URL list.
            let list = xdm_transfer_list::expand_batch(&args[1], 10_000)
                .and_then(|urls| xdm_transfer_list::parse(&urls.join("\n")))
                .unwrap_or_else(|error| {
                    eprintln!("batch: {error}");
                    std::process::exit(1);
                });
            send_daemon(Command::ImportListEx {
                request_id: new_request_id("cli-batch"),
                directory: args[2].clone(),
                entries: list.entries,
                enqueue: args[3..].iter().any(|f| f == "--start"),
                allow_duplicate: args[3..].iter().any(|f| f == "--allow-duplicates"),
            });
            return;
        }
        Some("export") if args.len() == 3 => {
            let result = Paths::open()
                .map_err(|error| error.to_string())
                .and_then(|paths| {
                    xdm_transfer_list::export(&paths, &PathBuf::from(&args[1]), &args[2])
                });
            match result {
                Ok(count) => println!("exported {count} tasks"),
                Err(error) => {
                    eprintln!("export: {error}");
                    std::process::exit(1);
                }
            }
            return;
        }
        Some("grab") if args.len() >= 3 => {
            let mut queue = &args[3..];
            let mut enqueue = false;
            if queue.first().map(String::as_str) == Some("--start") {
                enqueue = true;
                queue = &queue[1..];
            }
            let mut queue_id = None;
            if queue.first().map(String::as_str) == Some("--queue") {
                queue_id = Some(
                    queue
                        .get(1)
                        .and_then(|id| id.parse().ok())
                        .unwrap_or_else(|| usage()),
                );
                queue = &queue[2..];
            }
            if !queue.is_empty() {
                usage();
            }
            send_daemon(Command::GrabSite {
                request_id: new_request_id("cli-grab"),
                project: read_project(&args[1]),
                directory: args[2].clone(),
                queue_id,
                enqueue,
            });
            return;
        }
        Some("grab-status") if args.len() == 2 => {
            send_daemon(Command::GrabStatus { request_id: args[1].clone() });
            return;
        }
        Some("grab-save") if args.len() == 3 => {
            send_daemon(Command::GrabSave {
                name: args[1].clone(),
                project: read_project(&args[2]),
            });
            return;
        }
        Some("grab-projects") if args.len() == 1 => {
            send_daemon(Command::GrabProjects);
            return;
        }
        Some("grab-remove") if args.len() == 2 => {
            send_daemon(Command::GrabRemove { name: args[1].clone() });
            return;
        }
        Some("category") if args.len() == 2 && args[1] == "list" => {
            send_daemon(Command::CategoryList);
            return;
        }
        Some("category")
        if args.len() >= 5 && args.len() <= 6 && args[1] == "save" =>
        {
            send_daemon(Command::CategorySave {
                name: args[2].clone(),
                extensions: split_list(&args[3]),
                destination: args[4].clone(),
                sites: args.get(5).map(|list| split_list(list)).unwrap_or_default(),
            });
            return;
        }
        Some("category") if args.len() == 3 && args[1] == "remove" => {
            send_daemon(Command::CategoryRemove { name: args[2].clone() });
            return;
        }
        // daemon login list
        // daemon login save PATTERN USER PASSWORD_FILE [--disabled]
        // daemon login remove ID
        // The password is read from a file ("-" for stdin), never argv.
        Some("login") if args.len() == 2 && args[1] == "list" => {
            send_daemon(Command::LoginList);
            return;
        }
        Some("login") if args.len() >= 5 && args[1] == "save" => {
            let mut rest: Vec<String> = args[2..].to_vec();
            let mut enabled = true;
            if rest.last().map(String::as_str) == Some("--disabled") {
                enabled = false;
                rest.pop();
            }
            let [pattern, user, password_file] = rest.as_slice() else { usage() };
            send_daemon(Command::LoginSave {
                pattern: pattern.clone(),
                username: user.clone(),
                password: read_secret(password_file),
                enabled,
            });
            return;
        }
        Some("login") if args.len() == 3 && args[1] == "remove" => {
            send_daemon(Command::LoginRemove {
                id: args[2].parse().unwrap_or_else(|_| usage()),
            });
            return;
        }
        // daemon proxy-login USER PASSWORD_FILE|-   (stored by the daemon, never in the proxy URL)
        Some("proxy-login") if args.len() == 3 => {
            send_daemon(Command::ProxyLogin {
                username: args[1].clone(),
                password: read_secret(&args[2]),
            });
            return;
        }
        Some("proxy-login-clear") if args.len() == 1 => {
            send_daemon(Command::ProxyLoginClear);
            return;
        }
        // daemon auth TASK_ID USER PASSWORD_FILE [--remember]
        Some("auth") if args.len() == 4 || (args.len() == 5 && args[4] == "--remember") => {
            send_daemon(Command::TaskProvideLogin {
                task_id: args[1].parse().unwrap_or_else(|_| usage()),
                username: args[2].clone(),
                password: read_secret(&args[3]),
                remember: args.len() == 5,
            });
            return;
        }
        // queue completion ID none|notify|sleep|hibernate|shutdown|restart|
        // exit|program PATH [ARGS...] [--confirm-destructive] [--countdown N]
        Some("queue") if args.len() >= 4 && args[1] == "completion" => {
            let queue_id: i64 = args[2].parse().unwrap_or_else(|_| usage());
            let mut rest: Vec<String> = args[3..].to_vec();
            let mut confirm = false;
            let mut countdown = None;
            // Options are parsed from the end; the last `--countdown` value
            // precedes its flag, as in `--countdown 30`.
            loop {
                if rest.last().map(String::as_str) == Some("--confirm-destructive") {
                    confirm = true;
                    rest.pop();
                } else if rest.len() >= 2 && rest[rest.len() - 2] == "--countdown" {
                    countdown = Some(rest.last().unwrap().parse::<u64>().unwrap_or_else(|_| usage()));
                    rest.truncate(rest.len() - 2);
                } else {
                    break;
                }
            }
            let command = match rest.first().map(String::as_str) {
                Some("program") if rest.len() >= 2 => Command::QueueSetCompletionAction {
                    queue_id,
                    action: CompletionAction::Program {
                        program: rest[1].clone(),
                        args: rest[2..].to_vec(),
                    },
                    confirm,
                    countdown_seconds: countdown,
                },
                Some(name) if rest.len() == 1 => {
                    let action = match name {
                        "none" => Some(CompletionAction::None),
                        "notify" => Some(CompletionAction::Notify),
                        "sleep" => Some(CompletionAction::Sleep),
                        "hibernate" => Some(CompletionAction::Hibernate),
                        "shutdown" => Some(CompletionAction::Shutdown),
                        "restart" => Some(CompletionAction::Restart),
                        "exit" => Some(CompletionAction::Exit),
                        _ => None,
                    };
                    match action {
                        // Without opt-in flags the four legacy recorded-only
                        // actions stay on the old IPC form, so existing
                        // clients (GUI, scripts) keep their semantics.
                        Some(action)
                            if !confirm
                                && countdown.is_none()
                                && matches!(
                                    action,
                                    CompletionAction::None
                                        | CompletionAction::Notify
                                        | CompletionAction::Sleep
                                        | CompletionAction::Shutdown
                                ) =>
                        {
                            Command::QueueSetCompletion { queue_id, action: name.to_string() }
                        }
                        Some(action) => Command::QueueSetCompletionAction {
                            queue_id,
                            action,
                            confirm,
                            countdown_seconds: countdown,
                        },
                        None => usage(),
                    }
                }
                _ => usage(),
            };
            send_daemon(command);
            return;
        }
        _ => {}
    }
    let command = match args.first().map(String::as_str) {

        Some("list") if args.len() == 1 => Command::List,
        // daemon list --offset N --limit N [--sort COL] [--status S] [--category C] [--queue ID] [--search TEXT]
        Some("list") if args.len() > 1 => {
            let mut offset = 0u64;
            let mut limit = 100u64;
            let mut sort = None;
            let mut filter = xdm_ipc::TaskFilter::default();
            let mut rest = args[1..].iter();
            while let Some(flag) = rest.next() {
                let value = rest.next().unwrap_or_else(|| usage());
                match flag.as_str() {
                    "--offset" => offset = value.parse().unwrap_or_else(|_| usage()),
                    "--limit" => limit = value.parse().unwrap_or_else(|_| usage()),
                    "--sort" => sort = Some(value.clone()),
                    "--status" => filter.status = Some(value.clone()),
                    "--category" => filter.category = Some(value.clone()),
                    "--queue" => filter.queue_id = Some(value.parse().unwrap_or_else(|_| usage())),
                    "--search" => filter.search = Some(value.clone()),
                    _ => usage(),
                }
            }
            Command::ListPage { offset, limit, sort, filter: Some(filter) }
        }
        Some("move-file") if args.len() == 3 => Command::TaskMoveFile {
            task_id: args[1].parse().unwrap_or_else(|_| usage()),
            new_path: args[2].clone(),
        },
        Some("remove") if args.len() == 3 && args[2] == "--delete-partial" => Command::TaskRemoveEx {
            task_id: args[1].parse().unwrap_or_else(|_| usage()),
            delete_partial: true,
        },
        Some("partials") if args.len() == 2 => Command::PartialScan { directory: args[1].clone() },
        Some("partials-delete") if args.len() >= 3 => Command::PartialDelete {
            directory: args[1].clone(),
            paths: args[2..].to_vec(),
        },
        Some("info") if args.len() == 2 => Command::GetTask {
            task_id: args[1].parse().unwrap_or_else(|_| usage()),
        },
        Some("remove") if args.len() == 2 => Command::Remove {
            task_id: args[1].parse().unwrap_or_else(|_| usage()),
        },
        Some("redownload") if args.len() == 4 && args[3] == "--allow-duplicate" => {
            Command::RedownloadEx {
                task_id: args[1].parse().unwrap_or_else(|_| usage()),
                request_id: new_request_id("cli-redownload"),
                output: args[2].clone(),
                allow_duplicate: true,
            }
        }
        Some("redownload") if args.len() == 3 => Command::Redownload {
            task_id: args[1].parse().unwrap_or_else(|_| usage()),
            request_id: new_request_id("cli-redownload"),
            output: args[2].clone(),
        },
        Some("pause") if args.len() == 2 => Command::Pause {
            task_id: args[1].parse().unwrap_or_else(|_| usage()),
        },
        Some("resume") if args.len() == 2 => Command::Resume {
            task_id: args[1].parse().unwrap_or_else(|_| usage()),
        },
        Some("priority") if args.len() == 3 => Command::SetPriority {
            task_id: args[1].parse().unwrap_or_else(|_| usage()),
            priority: match args[2].as_str() {
                "low" => 0,
                "normal" => 1,
                "immediate" => 2,
                _ => usage(),
            },
        },
        Some("limit") if args.len() == 2 => Command::SetConcurrency {
            limit: args[1].parse().unwrap_or_else(|_| usage()),
        },
        Some("rate") if args.len() == 3 => Command::SetTaskRate {
            task_id: args[1].parse().unwrap_or_else(|_| usage()),
            bytes_per_second: if args[2] == "unlimited" {
                None
            } else {
                Some(
                    args[2]
                        .parse::<u64>()
                        .ok()
                        .and_then(|rate| rate.checked_mul(1024))
                        .unwrap_or_else(|| usage()),
                )
            },
        },
        Some("queue") if args.len() == 2 && args[1] == "start" => Command::QueueStart,
        Some("queue") if args.len() == 2 && args[1] == "stop" => Command::QueueStop,
        Some("queue")
            if (4..=5).contains(&args.len())
                && args[1] == "create"
                && args.last().is_some_and(|flag| flag == "--sync") =>
        {
            Command::QueueCreateSync {
                name: args[2].clone(),
                limit: if args.len() == 5 {
                    args[3].parse().unwrap_or_else(|_| usage())
                } else {
                    2
                },
            }
        }
        Some("queue") if args.len() == 4 && args[1] == "sync-interval" && args[3] == "off" => {
            Command::QueueSetSync {
                queue_id: args[2].parse().unwrap_or_else(|_| usage()),
                interval_seconds: None,
            }
        }
        Some("queue") if args.len() == 5 && args[1] == "sync-interval" => {
            let queue_id = args[2].parse().unwrap_or_else(|_| usage());
            let (a, b): (u64, u64) = (
                args[3].parse().unwrap_or_else(|_| usage()),
                args[4].parse().unwrap_or_else(|_| usage()),
            );
            Command::QueueSetSync { queue_id, interval_seconds: Some(a * 3600 + b * 60) }
        }
        Some("queue") if args.len() == 4 && args[1] == "sync-interval-seconds" => {
            Command::QueueSetSync {
                queue_id: args[2].parse().unwrap_or_else(|_| usage()),
                interval_seconds: Some(args[3].parse().unwrap_or_else(|_| usage())),
            }
        }
        Some("queue") if args.len() == 3 && args[1] == "check" => Command::QueueCheck {
            queue_id: args[2].parse().unwrap_or_else(|_| usage()),
        },
        Some("queue") if (args.len() == 3 || args.len() == 4) && args[1] == "create" => {
            Command::QueueCreate {
                name: args[2].clone(),
                limit: args
                    .get(3)
                    .map_or(2, |limit| limit.parse().unwrap_or_else(|_| usage())),
            }
        }
        Some("queue") if args.len() == 3 && ["enable", "disable"].contains(&args[1].as_str()) => {
            Command::QueueSetRunning {
                queue_id: args[2].parse().unwrap_or_else(|_| usage()),
                running: args[1] == "enable",
            }
        }
        Some("queue") if args.len() == 4 && args[1] == "limit" => Command::QueueSetConcurrency {
            queue_id: args[2].parse().unwrap_or_else(|_| usage()),
            limit: args[3].parse().unwrap_or_else(|_| usage()),
        },
        Some("queue") if args.len() == 4 && args[1] == "rename" => Command::QueueRename {
            queue_id: args[2].parse().unwrap_or_else(|_| usage()),
            name: args[3].clone(),
        },
        Some("queue") if args.len() == 3 && args[1] == "remove" => Command::QueueRemove {
            queue_id: args[2].parse().unwrap_or_else(|_| usage()),
        },
        Some("queue") if args.len() >= 4 && args[1] == "assign" => Command::AssignQueue {
            queue_id: args[2].parse().unwrap_or_else(|_| usage()),
            task_ids: args[3..]
                .iter()
                .map(|id| id.parse().unwrap_or_else(|_| usage()))
                .collect(),
        },
        Some("queue") if args.len() == 3 && args[1] == "at" => Command::QueueScheduleStart {
            at_unix: args[2].parse().unwrap_or_else(|_| usage()),
        },
        Some("queue") if args.len() == 3 && args[1] == "stop-at" => Command::QueueScheduleStop {
            at_unix: args[2].parse().unwrap_or_else(|_| usage()),
        },
        Some("queue") if args.len() == 3 && args[1] == "start-one" => Command::QueueStartOne {
            queue_id: args[2].parse().unwrap_or_else(|_| usage()),
        },
        Some("queue") if args.len() == 3 && args[1] == "stop-one" => Command::QueueStopOne {
            queue_id: args[2].parse().unwrap_or_else(|_| usage()),
        },
        Some("queue") if args.len() == 3 && args[1] == "events" => Command::QueueEvents {
            queue_id: args[2].parse().unwrap_or_else(|_| usage()),
        },
        Some("queue") if args.len() == 5 && args[1] == "retry" => Command::QueueSetRetry {
            queue_id: args[2].parse().unwrap_or_else(|_| usage()),
            max_retries: if args[3] == "infinite" {
                -1
            } else {
                args[3].parse().unwrap_or_else(|_| usage())
            },
            delay_seconds: args[4].parse().unwrap_or_else(|_| usage()),
        },
        Some("completion") if args.len() == 2 && args[1] == "status" => Command::CompletionStatus,
        Some("completion") if args.len() == 3 && args[1] == "cancel" => Command::CompletionCancel {
            queue_id: args[2].parse().unwrap_or_else(|_| usage()),
        },
        Some("queue") if args.len() == 4 && args[1] == "schedule" && args[3] == "none" => {
            Command::QueueSetSchedule {
                queue_id: args[2].parse().unwrap_or_else(|_| usage()),
                kind: "none".into(),
                start: 0,
                stop: None,
                days: 0,
            }
        }
        Some("queue") if args.len() == 6 && args[1] == "schedule" && args[3] == "once" => {
            Command::QueueSetSchedule {
                queue_id: args[2].parse().unwrap_or_else(|_| usage()),
                kind: "once".into(),
                start: args[4].parse().unwrap_or_else(|_| usage()),
                stop: (args[5] != "-").then(|| args[5].parse().unwrap_or_else(|_| usage())),
                days: 0,
            }
        }
        Some("queue") if args.len() == 7 && args[1] == "schedule" && args[3] == "daily" => {
            Command::QueueSetSchedule {
                queue_id: args[2].parse().unwrap_or_else(|_| usage()),
                kind: "daily".into(),
                start: parse_clock(&args[4]),
                stop: (args[5] != "-").then(|| parse_clock(&args[5])),
                days: parse_days(&args[6]),
            }
        }
        Some("move") if args.len() == 3 => Command::TaskMove {
            task_id: args[1].parse().unwrap_or_else(|_| usage()),
            target: match args[2].as_str() {
                "up" => MoveTarget::Up,
                "down" => MoveTarget::Down,
                "top" => MoveTarget::Top,
                "bottom" => MoveTarget::Bottom,
                index => MoveTarget::Index(index.parse().unwrap_or_else(|_| usage())),
            },
        },
        Some("submit") if args.len() == 3 => Command::Submit {
            request_id: new_request_id("cli"),
            url: args[1].clone(),
            output: args[2].clone(),
            headers: Vec::new(),
            priority: 1,
            connections: 8,
        },
        // The extended submit carries the duplicate policy, so a repeat of a
        // URL another task already owns needs --allow-duplicate to pass.
        Some("submit") if args.len() == 4 && args[3] == "--allow-duplicate" => {
            Command::SubmitEx {
                request_id: new_request_id("cli"),
                url: args[1].clone(),
                output: args[2].clone(),
                headers: Vec::new(),
                method: "GET".into(),
                body: None,
                priority: 1,
                connections: 8,
                name_from_server: false,
                ignore_modification_date: false,
                allow_duplicate: true,
                preserve_server_time: true,
            }
        }
        Some("submit-post") if args.len() == 4 => {
            let body = std::fs::read(&args[3]).unwrap_or_else(|error| {
                eprintln!("read {}: {error}", args[3]);
                std::process::exit(2)
            });
            Command::SubmitRequest {
                request_id: new_request_id("cli-post"),
                url: args[1].clone(),
                output: args[2].clone(),
                headers: Vec::new(),
                method: "POST".into(),
                body: Some(body),
                priority: 1,
                connections: 8,
            }
        }
        Some("proxy") if args.len() == 2 && args[1] == "get" => Command::GetProxy,
        Some("proxy") if args.len() == 2 && args[1] == "system" => {
            Command::SetProxy { proxy: ProxyChoice::System }
        }
        Some("proxy") if args.len() == 2 && args[1] == "direct" => {
            Command::SetProxy { proxy: ProxyChoice::Direct }
        }
        Some("proxy") if (args.len() == 3 || args.len() == 4) && args[1] == "manual" => {
            Command::SetProxy {
                proxy: ProxyChoice::Manual {
                    http: Some(args[2].clone()),
                    https: Some(args[2].clone()),
                    bypass: match args.get(3) {
                        Some(list) => list.split(',').map(|part| part.trim().to_owned()).collect(),
                        None => Vec::new(),
                    },
                },
            }
        }
        Some("limit-global") if args.len() == 2 => Command::SetGlobalLimit {
            kib: parse_kib(&args[1]),
        },
        Some("limit-host") if args.len() == 3 => Command::SetHostLimit {
            pattern: args[1].clone(),
            kib: match args[2].as_str() {
                "remove" => None,
                "unlimited" => Some(0),
                value => Some(parse_kib(value).unwrap_or_else(|| usage())),
            },
        },
        Some("host-connections") if args.len() == 3 => Command::SetHostConnections {
            pattern: args[1].clone(),
            count: if args[2] == "remove" {
                None
            } else {
                Some(args[2].parse().unwrap_or_else(|_| usage()))
            },
        },
        Some("quota") if args.len() == 2 && args[1] == "off" => Command::SetQuota {
            mib: 0,
            window_seconds: 0,
        },
        Some("quota") if args.len() == 3 => Command::SetQuota {
            mib: args[1].parse().unwrap_or_else(|_| usage()),
            window_seconds: args[2]
                .parse::<f64>()
                .ok()
                .filter(|hours| hours.is_finite() && *hours > 0.0)
                .map(|hours| (hours * 3600.0).round().max(1.0) as u64)
                .unwrap_or_else(|| usage()),
        },
        Some("bandwidth") if args.len() == 1 => Command::BandwidthStatus,
        Some("temp-dir") if args.len() == 2 && args[1] == "get" => Command::GetTempDir,
        Some("temp-dir") if args.len() == 2 => Command::SetTempDir {
            path: (args[1] != "off").then(|| args[1].clone()),
        },
        Some("stall-timeout") if args.len() == 2 && args[1] == "get" => Command::GetStallTimeout,
        Some("stall-timeout") if args.len() == 2 => Command::SetStallTimeout {
            seconds: args[1]
                .parse::<u64>()
                .ok()
                .filter(|seconds| (1..=600).contains(seconds))
                .unwrap_or_else(|| usage()),
        },
        Some("restart") if args.len() == 2 => Command::TaskRestart {
            task_id: args[1].parse().unwrap_or_else(|_| usage()),
        },
        _ => usage(),
    };
    send_daemon(command);
}

/// Reads a grabber project file verbatim as JSON for the daemon to validate.
fn read_project(path: &str) -> serde_json::Value {
    let bytes = std::fs::read(path).unwrap_or_else(|error| {
        eprintln!("read {path}: {error}");
        std::process::exit(2)
    });
    serde_json::from_slice(&bytes).unwrap_or_else(|error| {
        eprintln!("parse {path}: {error}");
        std::process::exit(2)
    })
}

/// Reads a password from a file, or stdin for "-"; never accepted from argv.
/// Only trailing line breaks are trimmed, so spaces stay part of the secret.
fn read_secret(path: &str) -> String {
    let text = if path == "-" {
        let mut buffer = String::new();
        std::io::stdin()
            .read_to_string(&mut buffer)
            .unwrap_or_else(|error| {
                eprintln!("read stdin: {error}");
                std::process::exit(2)
            });
        buffer
    } else {
        std::fs::read_to_string(path).unwrap_or_else(|error| {
            eprintln!("read {path}: {error}");
            std::process::exit(2)
        })
    };
    text.trim_end_matches(['\n', '\r']).to_owned()
}

/// KiB/s argument; `unlimited` is `None`.
fn parse_kib(text: &str) -> Option<u64> {
    if text == "unlimited" {
        return None;
    }
    Some(
        text.parse::<u64>()
            .ok()
            .filter(|kib| *kib > 0)
            .unwrap_or_else(|| usage()),
    )
}

/// Comma-separated CLI list; "-" means empty.
fn split_list(text: &str) -> Vec<String> {
    if text == "-" {
        Vec::new()
    } else {
        text.split(',')
            .map(|part| part.trim().to_owned())
            .filter(|part| !part.is_empty())
            .collect()
    }
}

/// Parses HH:MM into minutes since local midnight.
fn parse_clock(text: &str) -> i64 {
    let (hour, minute) = text.split_once(':').unwrap_or_else(|| usage());
    let hour: i64 = hour.parse().unwrap_or_else(|_| usage());
    let minute: i64 = minute.parse().unwrap_or_else(|_| usage());
    if !(0..24).contains(&hour) || !(0..60).contains(&minute) {
        usage();
    }
    hour * 60 + minute
}

/// Parses "all", "weekdays" or a comma list such as mon,wed,sun into a mask.
fn parse_days(text: &str) -> u8 {
    match text {
        "all" => 0x7f,
        "weekdays" => 0x1f,
        list => list.split(',').fold(0u8, |mask, day| {
            let bit = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"]
                .iter()
                .position(|name| *name == day)
                .unwrap_or_else(|| usage());
            mask | 1 << bit
        }),
    }
}

fn send_daemon(command: Command) {
    let paths = Paths::open().unwrap_or_else(|e| {
        eprintln!("daemon paths: {e}");
        std::process::exit(1)
    });
    let reply = paths.request(command).unwrap_or_else(|e| {
        eprintln!("daemon unavailable: {e}");
        std::process::exit(1)
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&reply).unwrap_or_default()
    );
    if !reply.ok {
        std::process::exit(1);
    }
}

fn usage() -> ! {
    eprintln!(
        "usage: xdm-cli URL OUTPUT_PATH [--connections 1..16] [--header 'Name: value'] [--rate-kib N] [--restart] [--temp-dir ABSOLUTE_DIR] [--no-server-time]\n       xdm-cli URL OUTPUT_PATH [--method POST] [--data-file BODY_FILE]\n       xdm-cli URL OUTPUT_PATH [--stall-timeout 1..600] [--no-proxy | --proxy http|https|socks5|socks5h://[USER:PASS@]HOST:PORT [--proxy-bypass HOST,...]]\n       xdm-cli daemon list|submit URL OUTPUT_PATH|submit URL OUTPUT_PATH --allow-duplicate|submit-post URL OUTPUT_PATH BODY_FILE|pause ID|resume ID|limit 1..16\n       xdm-cli daemon list [--offset N] [--limit 1..2000] [--sort COL] [--status S] [--category C] [--queue ID] [--search TEXT]\n       xdm-cli daemon move-file ID NEW_PATH|partials DIR|partials-delete DIR PATH...|remove ID --delete-partial\n       xdm-cli daemon info ID|remove ID|redownload ID OUTPUT_PATH [--allow-duplicate]|restart ID\n       xdm-cli daemon preview LIST_FILE|import LIST_FILE ABSOLUTE_DIRECTORY [--start] [--allow-duplicates]|batch 'URL[001-100]' ABSOLUTE_DIRECTORY [--start] [--allow-duplicates]|export NEW_FILE json|urls\n       xdm-cli daemon proxy get|system|direct|manual http|https|socks4|socks5|socks5h://[USER:PASS@]HOST:PORT [BYPASS,...]\n       xdm-cli daemon proxy-login USER PASSWORD_FILE|-   (store proxy credentials; requeues tasks waiting on a 407)\n       xdm-cli daemon proxy-login-clear\n       xdm-cli daemon stall-timeout get|SECONDS\n       xdm-cli daemon temp-dir get|ABSOLUTE_DIR|off\n       xdm-cli daemon limit-global KIB|unlimited\n       xdm-cli daemon limit-host PATTERN KIB|unlimited|remove\n       xdm-cli daemon host-connections PATTERN N|remove\n       xdm-cli daemon quota MIB HOURS|off\n       xdm-cli daemon bandwidth\n       xdm-cli daemon priority ID immediate|normal|low\n       xdm-cli daemon rate ID KIB_PER_SECOND|unlimited\n       xdm-cli daemon queue start|stop|at UNIX_SECONDS|stop-at UNIX_SECONDS\n       xdm-cli daemon queue create NAME [LIMIT] [--sync]|sync-interval ID HOURS MINUTES|off|check ID|enable ID|disable ID|limit ID N|rename ID NAME|remove ID\n       xdm-cli daemon queue assign QUEUE_ID TASK_ID [TASK_ID...]\n       xdm-cli daemon queue start-one ID|stop-one ID|events ID\n       xdm-cli daemon queue completion ID none|notify|sleep|hibernate|shutdown|restart|exit|program PATH [ARGS...] [--confirm-destructive] [--countdown SECONDS]\n       xdm-cli daemon completion status|cancel QUEUE_ID\n       xdm-cli daemon queue retry ID COUNT|infinite DELAY_SECONDS\n       xdm-cli daemon queue schedule ID none|once START_UNIX STOP_UNIX|-|daily HH:MM HH:MM|- all|weekdays|mon,tue,...\n       xdm-cli daemon preview LIST_FILE|import LIST_FILE ABSOLUTE_DIRECTORY [--start] [--allow-duplicates]|playlist FILE_OR_URL ABSOLUTE_DIRECTORY [--start] [--allow-duplicates]|batch 'URL[001-100]' ABSOLUTE_DIRECTORY [--start] [--allow-duplicates]|export NEW_FILE json|urls
       xdm-cli daemon grab PROJECT.json ABSOLUTE_DIRECTORY [--start] [--queue ID]
       xdm-cli daemon grab-status REQUEST_ID|grab-save NAME PROJECT.json|grab-projects|grab-remove NAME
       xdm-cli daemon category list|save NAME EXT,EXT|- ABSOLUTE_DIRECTORY [*.site,...]|remove NAME
       xdm-cli daemon login list|save PATTERN USER PASSWORD_FILE|- [--disabled]|remove ID
       xdm-cli daemon auth TASK_ID USER PASSWORD_FILE|- [--remember]
       xdm-cli daemon move ID up|down|top|bottom|INDEX
       xdm-cli grab PROJECT.json
       xdm-cli media inspect MANIFEST_URL [--header 'Name: value']
       xdm-cli media download MANIFEST_URL OUTPUT.mp4|OUTPUT.mkv VIDEO_INDEX [AUDIO_INDEX] [--subtitles 1,2] [--header 'Name: value']
       xdm-cli media record MANIFEST_URL OUTPUT.mp4|OUTPUT.mkv VIDEO_INDEX [AUDIO_INDEX] [--max-seconds N] [--header 'Name: value']
       xdm-cli media audio MANIFEST_URL OUTPUT TRACK original|mp3:KBPS|m4a:KBPS|opus:KBPS [--header 'Name: value']
       xdm-cli media subtitles MANIFEST_URL OUTPUT.srt|.vtt TRACK [--header 'Name: value']
       xdm-cli media queue MANIFEST_URL OUTPUT VIDEO_INDEX [AUDIO_INDEX] [--subtitles 1,2] [--header 'Name: value']
       xdm-cli daemon media-job MANIFEST_URL OUTPUT video|audio|subtitles|live SELECTION [SELECTION...] [--subtitles 1,2] [--max-seconds N] [--header 'Name: value']"
    );
    std::process::exit(2)
}
