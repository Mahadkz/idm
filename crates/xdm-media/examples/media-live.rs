//! Fixture driver for the live-recording API; exercised by
//! tests/integration/media_live.py and handy for manual checks.
//!
//! `record` sets its stop flag when stdin closes, so callers decide when the
//! recording ends (the equivalent of a player's stop button).

use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use xdm_media::Result;

fn main() {
    if let Err(error) = run(std::env::args().skip(1).collect()) {
        eprintln!("media-live: {error}");
        std::process::exit(1);
    }
}

fn run(args: Vec<String>) -> Result<()> {
    let mut headers = Vec::new();
    let mut max_secs: Option<f64> = None;
    let mut stop_after: Option<f64> = None;
    let mut positional = Vec::new();
    let mut rest = args.into_iter().peekable();
    while let Some(argument) = rest.next() {
        match argument.as_str() {
            "--header" => {
                let line = rest.next().unwrap_or_else(|| usage());
                let (name, value) = line.split_once(':').unwrap_or_else(|| usage());
                headers.push((name.trim().to_owned(), value.trim().to_owned()));
            }
            "--max-secs" => {
                max_secs = Some(rest.next().unwrap_or_else(|| usage()).parse().unwrap_or_else(|_| usage()));
            }
            "--stop-after" => {
                stop_after = Some(rest.next().unwrap_or_else(|| usage()).parse().unwrap_or_else(|_| usage()));
            }
            other if other.starts_with('-') => usage(),
            other => positional.push(other.to_owned()),
        }
    }
    match positional.first().map(String::as_str) {
        Some("inspect") if positional.len() == 2 => {
            println!(
                "{}",
                serde_json::to_string_pretty(&xdm_media::inspect_report_with_headers(
                    &positional[1],
                    &headers
                )?)?
            );
        }
        Some("record") if positional.len() == 4 || positional.len() == 5 => {
            let url = positional[1].clone();
            let output = PathBuf::from(&positional[2]);
            let video = positional[3].parse().unwrap_or_else(|_| usage());
            let audio = positional.get(4).map(|value| value.parse().unwrap_or_else(|_| usage()));
            let stop = Arc::new(AtomicBool::new(false));
            // A closed stdin or a "stop" line ends the recording; the watcher
            // thread dies with the process when a run finishes by itself.
            thread::spawn({
                let stop = stop.clone();
                move || {
                    let mut input = String::new();
                    let _ = std::io::stdin().read_to_string(&mut input);
                    stop.store(true, Ordering::Relaxed);
                }
            });
            if let Some(secs) = stop_after {
                thread::spawn({
                    let stop = stop.clone();
                    move || {
                        thread::sleep(std::time::Duration::from_secs_f64(secs));
                        stop.store(true, Ordering::Relaxed);
                    }
                });
            }
            let max_duration = max_secs.map(std::time::Duration::from_secs_f64);
            let summary = xdm_media::record_live(&url, &output, video, audio, &headers, &stop, max_duration)?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
        }
        _ => usage(),
    }
    Ok(())
}

fn usage() -> ! {
    eprintln!(
        "usage: media-live inspect URL [--header 'Name: value']\n       media-live record URL OUTPUT.mp4|OUTPUT.mkv VIDEO [AUDIO] [--max-secs SECS] [--stop-after SECS] [--header 'Name: value']"
    );
    std::process::exit(2)
}
