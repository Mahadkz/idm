use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

pub const DEFAULT_CONNECTIONS: usize = 8;
pub const DEFAULT_THEME: &str = "system";

pub fn default_clipboard_extensions_value() -> Vec<String> {
    vec![
        "zip".into(),
        "rar".into(),
        "7z".into(),
        "exe".into(),
        "msi".into(),
        "iso".into(),
        "mp3".into(),
        "mp4".into(),
        "mkv".into(),
        "pdf".into(),
    ]
}

fn default_connections_value() -> usize {
    DEFAULT_CONNECTIONS
}

fn default_theme_value() -> String {
    DEFAULT_THEME.to_string()
}

fn default_confirm_remove_value() -> bool {
    true
}

fn default_start_minimized_value() -> bool {
    false
}

fn default_sound_value() -> bool {
    false
}

fn default_empty_string() -> String {
    String::new()
}

fn default_monitor_clipboard_value() -> bool {
    false
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuiSettings {
    #[serde(default = "default_connections_value")]
    pub default_connections: usize,
    #[serde(default = "default_theme_value")]
    pub theme: String,
    #[serde(default = "default_confirm_remove_value")]
    pub confirm_remove: bool,
    #[serde(default = "default_start_minimized_value")]
    pub start_minimized: bool,
    #[serde(default = "default_sound_value")]
    pub sound_download_complete: bool,
    #[serde(default = "default_empty_string")]
    pub sound_download_complete_path: String,
    #[serde(default = "default_sound_value")]
    pub sound_download_failed: bool,
    #[serde(default = "default_empty_string")]
    pub sound_download_failed_path: String,
    #[serde(default = "default_sound_value")]
    pub sound_queue_complete: bool,
    #[serde(default = "default_empty_string")]
    pub sound_queue_complete_path: String,
    #[serde(default = "default_monitor_clipboard_value")]
    pub monitor_clipboard: bool,
    #[serde(default = "default_clipboard_extensions_value")]
    pub clipboard_extensions: Vec<String>,
}

impl GuiSettings {
    pub fn clipboard_extensions_str(&self) -> String {
        self.clipboard_extensions.join(", ")
    }

    pub fn set_clipboard_extensions_from_str(&mut self, s: &str) {
        self.clipboard_extensions = s
            .split([',', ' ', ';'])
            .map(|ext| ext.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|ext| !ext.is_empty())
            .collect();
    }
}

impl Default for GuiSettings {
    fn default() -> Self {
        Self {
            default_connections: DEFAULT_CONNECTIONS,
            theme: DEFAULT_THEME.to_string(),
            confirm_remove: true,
            start_minimized: false,
            sound_download_complete: false,
            sound_download_complete_path: String::new(),
            sound_download_failed: false,
            sound_download_failed_path: String::new(),
            sound_queue_complete: false,
            sound_queue_complete_path: String::new(),
            monitor_clipboard: false,
            clipboard_extensions: default_clipboard_extensions_value(),
        }
    }
}

pub fn load_settings(data_dir: &Path) -> GuiSettings {
    let path = data_dir.join("gui-settings.json");
    if let Ok(bytes) = fs::read(&path) {
        if let Ok(mut settings) = serde_json::from_slice::<GuiSettings>(&bytes) {
            settings.default_connections = settings.default_connections.clamp(1, 16);
            if settings.theme != "light" && settings.theme != "dark" && settings.theme != "system" {
                settings.theme = "system".to_string();
            }
            if settings.clipboard_extensions.is_empty() {
                settings.clipboard_extensions = default_clipboard_extensions_value();
            }
            return settings;
        }
    }
    GuiSettings::default()
}

pub fn save_settings(data_dir: &Path, settings: &GuiSettings) -> Result<(), String> {
    let path = data_dir.join("gui-settings.json");
    let mut valid = settings.clone();
    valid.default_connections = valid.default_connections.clamp(1, 16);
    if valid.theme != "light" && valid.theme != "dark" && valid.theme != "system" {
        valid.theme = "system".to_string();
    }
    if valid.clipboard_extensions.is_empty() {
        valid.clipboard_extensions = default_clipboard_extensions_value();
    }
    let json = serde_json::to_vec_pretty(&valid).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_settings() {
        let s = GuiSettings::default();
        assert_eq!(s.default_connections, 8);
        assert_eq!(s.theme, "system");
        assert!(s.confirm_remove);
        assert!(!s.start_minimized);
        assert!(!s.monitor_clipboard);
        assert!(!s.sound_download_complete);
        assert_eq!(s.sound_download_complete_path, "");
        assert_eq!(s.clipboard_extensions.len(), 10);
        assert!(s.clipboard_extensions.contains(&"zip".to_string()));
        assert!(s.clipboard_extensions.contains(&"pdf".to_string()));
    }

    #[test]
    fn test_save_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("xdm-test-save-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let s = GuiSettings {
            default_connections: 12,
            theme: "dark".into(),
            confirm_remove: false,
            start_minimized: true,
            sound_download_complete: true,
            sound_download_complete_path: "/sounds/done.wav".into(),
            sound_download_failed: true,
            sound_download_failed_path: "/sounds/fail.wav".into(),
            sound_queue_complete: true,
            sound_queue_complete_path: "/sounds/queue.wav".into(),
            monitor_clipboard: true,
            clipboard_extensions: vec!["iso".into(), "dmg".into()],
        };
        save_settings(&dir, &s).unwrap();
        let loaded = load_settings(&dir);
        assert_eq!(loaded, s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_clamp_connections() {
        let dir = std::env::temp_dir().join(format!("xdm-test-clamp-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let s = GuiSettings {
            default_connections: 99,
            ..GuiSettings::default()
        };
        save_settings(&dir, &s).unwrap();
        let loaded = load_settings(&dir);
        assert_eq!(loaded.default_connections, 16);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_backward_compatibility_load_old_json() {
        let dir = std::env::temp_dir().join(format!("xdm-test-compat-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let old_json = r#"{"default_connections": 4}"#;
        fs::write(dir.join("gui-settings.json"), old_json).unwrap();
        let loaded = load_settings(&dir);
        assert_eq!(loaded.default_connections, 4);
        assert_eq!(loaded.theme, "system");
        assert!(loaded.confirm_remove);
        assert!(!loaded.start_minimized);
        assert!(!loaded.monitor_clipboard);
        assert_eq!(loaded.clipboard_extensions.len(), 10);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_set_clipboard_extensions_from_str() {
        let mut s = GuiSettings::default();
        s.set_clipboard_extensions_from_str(".zip, TAR; gz   7z");
        assert_eq!(s.clipboard_extensions, vec!["zip", "tar", "gz", "7z"]);
        assert_eq!(s.clipboard_extensions_str(), "zip, tar, gz, 7z");
    }
}
