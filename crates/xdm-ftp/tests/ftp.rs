mod support;

use sha2::{Digest, Sha256};
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use support::{Config, Server};
use xdm_ftp::{FtpOptions, download};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xdm-ftp-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn payload(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

fn options(url: String, output: &Path, restart: bool) -> FtpOptions {
    FtpOptions {
        url,
        output: output.to_path_buf(),
        restart,
        stop: Arc::new(AtomicBool::new(false)),
    }
}

fn part(output: &Path) -> PathBuf {
    PathBuf::from(format!("{}.part", output.display()))
}

fn checkpoint(output: &Path) -> PathBuf {
    PathBuf::from(format!("{}.part.ftp", output.display()))
}

#[test]
fn downloads_and_publishes_full_file() {
    let content = payload(200_000);
    let server = Server::start(Config {
        content: content.clone(),
        path: "/data.bin".to_string(),
        ..Default::default()
    });
    let dir = temp_dir("basic");
    let output = dir.join("data.bin");

    let seen = Cell::new(0u64);
    let total = Cell::new(None);
    download(options(server.url("/data.bin"), &output, false), |done, size| {
        seen.set(done);
        total.set(size);
    })
    .unwrap();

    assert_eq!(sha256(&fs::read(&output).unwrap()), sha256(&content));
    assert_eq!(seen.get(), content.len() as u64);
    assert_eq!(total.get(), Some(content.len() as u64));
    assert!(!part(&output).exists());
    assert!(!checkpoint(&output).exists());
}

#[test]
fn handles_multiline_banner() {
    let content = payload(40_000);
    let server = Server::start(Config {
        content: content.clone(),
        path: "/multi.bin".to_string(),
        multiline_banner: true,
        ..Default::default()
    });
    let dir = temp_dir("banner");
    let output = dir.join("multi.bin");
    download(options(server.url("/multi.bin"), &output, false), |_, _| {}).unwrap();
    assert_eq!(sha256(&fs::read(&output).unwrap()), sha256(&content));
}

#[test]
fn reports_login_failure() {
    let server = Server::start(Config {
        login_ok: false,
        ..Default::default()
    });
    let dir = temp_dir("login");
    let output = dir.join("file.bin");
    let error = download(options(server.url("/file.bin"), &output, false), |_, _| {})
        .unwrap_err()
        .to_string();
    assert!(error.to_lowercase().contains("login"), "{error}");
    assert!(!output.exists());
}

#[test]
fn disconnect_mid_transfer_then_resume() {
    let content = payload(300_000);
    let server = Server::start(Config {
        content: content.clone(),
        path: "/resume.bin".to_string(),
        disconnect_after: Some(65_536),
        ..Default::default()
    });
    let dir = temp_dir("resume");
    let output = dir.join("resume.bin");

    let error = download(options(server.url("/resume.bin"), &output, false), |_, _| {})
        .unwrap_err()
        .to_string();
    assert!(!error.is_empty());
    let partial = fs::metadata(part(&output)).unwrap().len();
    assert_eq!(partial, 65_536);
    assert!(checkpoint(&output).exists());

    download(options(server.url("/resume.bin"), &output, false), |_, _| {}).unwrap();
    assert_eq!(sha256(&fs::read(&output).unwrap()), sha256(&content));
    assert!(!part(&output).exists());
    assert!(!checkpoint(&output).exists());
}

#[test]
fn changed_mdtm_requires_restart() {
    let content = payload(200_000);
    let server = Server::start(Config {
        content: content.clone(),
        path: "/changed.bin".to_string(),
        mdtm: "20240101000000".to_string(),
        disconnect_after: Some(32_768),
        ..Default::default()
    });
    let dir = temp_dir("mdtm");
    let output = dir.join("changed.bin");

    download(options(server.url("/changed.bin"), &output, false), |_, _| {}).unwrap_err();
    assert_eq!(fs::metadata(part(&output)).unwrap().len(), 32_768);

    // Same size, new modification time: resume must be refused.
    server.set_mdtm("20250101000000");
    let error = download(options(server.url("/changed.bin"), &output, false), |_, _| {})
        .unwrap_err()
        .to_string();
    assert!(error.contains("restart"), "{error}");

    download(options(server.url("/changed.bin"), &output, true), |_, _| {}).unwrap();
    assert_eq!(sha256(&fs::read(&output).unwrap()), sha256(&content));
}

#[test]
fn changed_size_requires_restart() {
    let content = payload(150_000);
    let server = Server::start(Config {
        content: content.clone(),
        path: "/sized.bin".to_string(),
        disconnect_after: Some(16_384),
        ..Default::default()
    });
    let dir = temp_dir("size");
    let output = dir.join("sized.bin");

    download(options(server.url("/sized.bin"), &output, false), |_, _| {}).unwrap_err();
    assert_eq!(fs::metadata(part(&output)).unwrap().len(), 16_384);

    // The advertised size no longer matches the checkpoint.
    let server = Server::start(Config {
        content: content.clone(),
        path: "/sized.bin".to_string(),
        size_override: Some(content.len() as u64 + 1),
        ..Default::default()
    });
    let error = download(options(server.url("/sized.bin"), &output, false), |_, _| {})
        .unwrap_err()
        .to_string();
    assert!(error.contains("restart"), "{error}");
}

#[test]
fn rejected_rest_requires_restart() {
    let content = payload(200_000);
    let server = Server::start(Config {
        content: content.clone(),
        path: "/rest.bin".to_string(),
        disconnect_after: Some(48_000),
        ..Default::default()
    });
    let dir = temp_dir("rest");
    let output = dir.join("rest.bin");

    download(options(server.url("/rest.bin"), &output, false), |_, _| {}).unwrap_err();
    assert_eq!(fs::metadata(part(&output)).unwrap().len(), 48_000);

    server.set_rest_supported(false);
    let error = download(options(server.url("/rest.bin"), &output, false), |_, _| {})
        .unwrap_err()
        .to_string();
    assert!(error.contains("restart"), "{error}");

    download(options(server.url("/rest.bin"), &output, true), |_, _| {}).unwrap();
    assert_eq!(sha256(&fs::read(&output).unwrap()), sha256(&content));
}

#[test]
fn pasv_foreign_ip_uses_control_peer() {
    let content = payload(80_000);
    let server = Server::start(Config {
        content: content.clone(),
        path: "/pasv.bin".to_string(),
        epsv_supported: false,
        pasv_foreign_ip: true,
        ..Default::default()
    });
    let dir = temp_dir("pasv");
    let output = dir.join("pasv.bin");
    download(options(server.url("/pasv.bin"), &output, false), |_, _| {}).unwrap();
    assert_eq!(sha256(&fs::read(&output).unwrap()), sha256(&content));
}

#[test]
fn escapes_filenames_with_spaces_and_percent() {
    let content = payload(50_000);
    let server = Server::start(Config {
        content: content.clone(),
        path: "/dir with space/100% done (1).txt".to_string(),
        ..Default::default()
    });
    let dir = temp_dir("escaped");
    let output = dir.join("escaped.txt");
    let url = server.url("/dir%20with%20space/100%25%20done%20(1).txt");
    download(options(url, &output, false), |_, _| {}).unwrap();
    assert_eq!(sha256(&fs::read(&output).unwrap()), sha256(&content));
}

#[test]
fn stop_flag_pauses_and_leaves_resumable_state() {
    let content = payload(400_000);
    let server = Server::start(Config {
        content: content.clone(),
        path: "/pause.bin".to_string(),
        chunk_size: 4096,
        chunk_delay: std::time::Duration::from_millis(2),
        ..Default::default()
    });
    let dir = temp_dir("pause");
    let output = dir.join("pause.bin");

    let stop = Arc::new(AtomicBool::new(false));
    let signal = stop.clone();
    let result = download(
        FtpOptions {
            url: server.url("/pause.bin"),
            output: output.clone(),
            restart: false,
            stop,
        },
        move |done, _| {
            if done >= 20_000 {
                signal.store(true, Ordering::SeqCst);
            }
        },
    );
    let error = result.unwrap_err().to_string();
    assert!(error.contains("paused"), "{error}");
    let partial = fs::metadata(part(&output)).unwrap().len();
    assert!(partial > 0 && partial < content.len() as u64, "{partial}");

    download(options(server.url("/pause.bin"), &output, false), |_, _| {}).unwrap();
    assert_eq!(sha256(&fs::read(&output).unwrap()), sha256(&content));
}
