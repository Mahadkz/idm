use crate::settings::GuiSettings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundEvent {
    DownloadComplete,
    DownloadFailed,
    QueueComplete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundCommand {
    pub program: String,
    pub args: Vec<String>,
}

pub const DEFAULT_LINUX_COMPLETE: &str = "/usr/share/sounds/freedesktop/stereo/complete.oga";
pub const DEFAULT_LINUX_FAILED: &str = "/usr/share/sounds/freedesktop/stereo/dialog-warning.oga";
pub const DEFAULT_LINUX_QUEUE: &str = "/usr/share/sounds/freedesktop/stereo/complete.oga";

pub const DEFAULT_WINDOWS_COMPLETE: &str = "C:\\Windows\\Media\\tada.wav";
pub const DEFAULT_WINDOWS_FAILED: &str = "C:\\Windows\\Media\\Windows Ding.wav";
pub const DEFAULT_WINDOWS_QUEUE: &str = "C:\\Windows\\Media\\chimes.wav";

pub fn resolve_sound_path(custom: &str, event: SoundEvent, is_windows: bool) -> String {
    let trimmed = custom.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    if is_windows {
        match event {
            SoundEvent::DownloadComplete => DEFAULT_WINDOWS_COMPLETE.to_string(),
            SoundEvent::DownloadFailed => DEFAULT_WINDOWS_FAILED.to_string(),
            SoundEvent::QueueComplete => DEFAULT_WINDOWS_QUEUE.to_string(),
        }
    } else {
        match event {
            SoundEvent::DownloadComplete => DEFAULT_LINUX_COMPLETE.to_string(),
            SoundEvent::DownloadFailed => DEFAULT_LINUX_FAILED.to_string(),
            SoundEvent::QueueComplete => DEFAULT_LINUX_QUEUE.to_string(),
        }
    }
}

pub fn build_sound_command_args(
    player_binary: &str,
    sound_path: &str,
    is_windows: bool,
) -> SoundCommand {
    if is_windows {
        SoundCommand {
            program: "powershell".to_string(),
            args: vec![
                "-NoProfile".to_string(),
                "-NonInteractive".to_string(),
                "-Command".to_string(),
                format!("(New-Object Media.SoundPlayer '{}').PlaySync()", sound_path),
            ],
        }
    } else {
        SoundCommand {
            program: player_binary.to_string(),
            args: vec![sound_path.to_string()],
        }
    }
}

pub fn which_in_path(cmd: &str) -> bool {
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(cmd);
            if candidate.is_file() {
                return true;
            }
        }
    }
    false
}

pub fn detect_linux_player() -> Option<&'static str> {
    if which_in_path("paplay") {
        Some("paplay")
    } else if which_in_path("aplay") {
        Some("aplay")
    } else {
        None
    }
}

pub fn play_sound(command: SoundCommand) {
    if std::env::var("XDM_FAKE_OS_ACTIONS").as_deref() == Ok("1") {
        return;
    }
    std::thread::spawn(move || {
        let _ = std::process::Command::new(&command.program)
            .args(&command.args)
            .output();
    });
}

pub fn trigger_event_sound(settings: &GuiSettings, event: SoundEvent) {
    let (enabled, custom_path) = match event {
        SoundEvent::DownloadComplete => (
            settings.sound_download_complete,
            &settings.sound_download_complete_path,
        ),
        SoundEvent::DownloadFailed => (
            settings.sound_download_failed,
            &settings.sound_download_failed_path,
        ),
        SoundEvent::QueueComplete => (
            settings.sound_queue_complete,
            &settings.sound_queue_complete_path,
        ),
    };

    if !enabled {
        return;
    }

    #[cfg(windows)]
    let is_windows = true;
    #[cfg(not(windows))]
    let is_windows = false;

    let path = resolve_sound_path(custom_path, event, is_windows);
    let player = if is_windows {
        "powershell"
    } else {
        detect_linux_player().unwrap_or("paplay")
    };

    let command = build_sound_command_args(player, &path, is_windows);
    play_sound(command);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_sound_path_custom() {
        assert_eq!(
            resolve_sound_path("/custom/sound.wav", SoundEvent::DownloadComplete, false),
            "/custom/sound.wav"
        );
        assert_eq!(
            resolve_sound_path("C:\\custom.wav", SoundEvent::DownloadFailed, true),
            "C:\\custom.wav"
        );
    }

    #[test]
    fn test_resolve_sound_path_defaults() {
        assert_eq!(
            resolve_sound_path("", SoundEvent::DownloadComplete, false),
            DEFAULT_LINUX_COMPLETE
        );
        assert_eq!(
            resolve_sound_path("", SoundEvent::DownloadFailed, false),
            DEFAULT_LINUX_FAILED
        );
        assert_eq!(
            resolve_sound_path("", SoundEvent::QueueComplete, false),
            DEFAULT_LINUX_QUEUE
        );
        assert_eq!(
            resolve_sound_path("", SoundEvent::DownloadComplete, true),
            DEFAULT_WINDOWS_COMPLETE
        );
        assert_eq!(
            resolve_sound_path("", SoundEvent::DownloadFailed, true),
            DEFAULT_WINDOWS_FAILED
        );
        assert_eq!(
            resolve_sound_path("", SoundEvent::QueueComplete, true),
            DEFAULT_WINDOWS_QUEUE
        );
    }

    #[test]
    fn test_sound_command_construction_linux_paplay() {
        let cmd = build_sound_command_args("paplay", "/path/to/complete.oga", false);
        assert_eq!(cmd.program, "paplay");
        assert_eq!(cmd.args, vec!["/path/to/complete.oga"]);
    }

    #[test]
    fn test_sound_command_construction_linux_aplay() {
        let cmd = build_sound_command_args("aplay", "/path/to/alert.wav", false);
        assert_eq!(cmd.program, "aplay");
        assert_eq!(cmd.args, vec!["/path/to/alert.wav"]);
    }

    #[test]
    fn test_sound_command_construction_windows() {
        let cmd = build_sound_command_args("powershell", "C:\\Windows\\Media\\tada.wav", true);
        assert_eq!(cmd.program, "powershell");
        assert_eq!(
            cmd.args,
            vec![
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "(New-Object Media.SoundPlayer 'C:\\Windows\\Media\\tada.wav').PlaySync()"
            ]
        );
    }
}
