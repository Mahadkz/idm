//! Fixture driver for the audio-only and subtitle APIs; exercised by
//! tests/integration/media_audio.py and handy for manual checks.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use xdm_media::{AudioFormat, Result};

fn main() {
    if let Err(error) = run(std::env::args().skip(1).collect()) {
        eprintln!("media-audio: {error}");
        std::process::exit(1);
    }
}

fn run(args: Vec<String>) -> Result<()> {
    let mut headers = Vec::new();
    let mut subtitles = Vec::new();
    let mut positional = Vec::new();
    let mut rest = args.into_iter().peekable();
    while let Some(argument) = rest.next() {
        match argument.as_str() {
            "--header" => {
                let line = rest.next().unwrap_or_else(|| usage());
                let (name, value) = line.split_once(':').unwrap_or_else(|| usage());
                headers.push((name.trim().to_owned(), value.trim().to_owned()));
            }
            "--subtitles" => {
                let list = rest.next().unwrap_or_else(|| usage());
                for part in list.split(',') {
                    subtitles.push(part.trim().parse().unwrap_or_else(|_| usage()));
                }
            }
            other if other.starts_with('-') => usage(),
            other => positional.push(other.to_owned()),
        }
    }
    match positional.first().map(String::as_str) {
        Some("inspect") if positional.len() == 2 => {
            println!(
                "{}",
                serde_json::to_string_pretty(&xdm_media::inspect_with_headers(
                    &positional[1],
                    &headers
                )?)?
            );
        }
        Some("audio") if positional.len() == 5 => {
            let output = &positional[2];
            xdm_media::download_audio(
                &positional[1],
                &PathBuf::from(output),
                positional[3].parse().unwrap_or_else(|_| usage()),
                parse_format(&positional[4])?,
                &headers,
                &AtomicBool::new(false),
            )?;
            println!("saved {output}");
        }
        Some("download") if positional.len() == 4 || positional.len() == 5 => {
            let video = positional[3].parse().unwrap_or_else(|_| usage());
            let audio = match positional.len() {
                5 => Some(positional[4].parse().unwrap_or_else(|_| usage())),
                _ => None,
            };
            let output = &positional[2];
            xdm_media::download_with_headers(
                &positional[1],
                &PathBuf::from(output),
                video,
                audio,
                &subtitles,
                &headers,
                &AtomicBool::new(false),
                false,
            )?;
            println!("saved {output}");
        }
        Some("subtitles") if positional.len() == 4 => {
            let output = &positional[2];
            xdm_media::save_subtitles(
                &positional[1],
                &PathBuf::from(output),
                positional[3].parse().unwrap_or_else(|_| usage()),
                &headers,
                &AtomicBool::new(false),
            )?;
            println!("saved {output}");
        }
        _ => usage(),
    }
    Ok(())
}

fn parse_format(text: &str) -> Result<AudioFormat> {
    match text {
        "original" => Ok(AudioFormat::Original),
        rest => {
            let (kind, bitrate) = rest.split_once(':').ok_or_else(|| xerr(format!("audio format must be original or kind:KBPS, got {text}")))?;
            match kind {
                "mp3" => Ok(AudioFormat::Mp3 { bitrate_kbps: bitrate.parse()? }),
                "m4a" => Ok(AudioFormat::M4a { bitrate_kbps: bitrate.parse()? }),
                "opus" => Ok(AudioFormat::Opus { bitrate_kbps: bitrate.parse()? }),
                other => Err(xerr(format!("unknown audio format {other}"))),
            }
        }
    }
}

fn xerr(message: impl Into<String>) -> Box<dyn std::error::Error + Send + Sync> {
    message.into().into()
}

fn usage() -> ! {
    eprintln!(
        "usage: media-audio inspect URL [--header 'Name: value']\n       media-audio audio URL OUTPUT TRACK original|mp3:KBPS|m4a:KBPS|opus:KBPS [--header 'Name: value']\n       media-audio download URL OUTPUT.mp4|OUTPUT.mkv VIDEO [AUDIO] [--subtitles 1,2] [--header 'Name: value']\n       media-audio subtitles URL OUTPUT.srt|OUTPUT.vtt TRACK [--header 'Name: value']"
    );
    std::process::exit(2)
}
