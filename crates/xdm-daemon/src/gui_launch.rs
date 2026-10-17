//! Starts the GUI on demand so a browser offer always reaches a Download File Info dialog.
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

static LAUNCHED: Mutex<Option<Instant>> = Mutex::new(None);

/// The GUI's "show download dialog" preference (default on), read from its settings file.
pub(crate) fn show_file_info(dir: &Path) -> bool {
    std::fs::read(dir.join("gui-settings.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|v| v.get("show_file_info").and_then(|b| b.as_bool()))
        .unwrap_or(true)
}

fn gui_program() -> PathBuf {
    let name = if cfg!(windows) { "xdm-gui.exe" } else { "xdm-gui" };
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(name)))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from(name))
}

/// Spawns a detached GUI (same environment and data dir) unless one was started moments ago.
/// Returns false when it could not be started.
pub(crate) fn launch() -> bool {
    if std::env::var_os("XDM_NO_GUI_LAUNCH").is_some() {
        return false;
    }
    let Ok(mut last) = LAUNCHED.lock() else { return false };
    if last.is_some_and(|at| at.elapsed() < Duration::from_secs(20)) {
        return true;
    }
    let mut command = std::process::Command::new(gui_program());
    command
        .arg("--dialog-only")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0000_0008 | 0x0800_0000);
    }
    match command.spawn() {
        Ok(mut child) => {
            *last = Some(Instant::now());
            // Reap it when it exits; a GUI that dies early lets the next offer retry.
            std::thread::spawn(move || {
                let _ = child.wait();
                if let Ok(mut last) = LAUNCHED.lock() {
                    *last = None;
                }
            });
            true
        }
        Err(_) => false,
    }
}
