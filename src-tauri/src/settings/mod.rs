//! Local preferences contain only display and lifecycle choices. Credentials are read
//! from their original agent stores and never enter this file.

use std::{fs, io, path::{Path, PathBuf}};
use serde::{Deserialize, Serialize};

pub const MIN_OPACITY: u8 = 30;
pub const MAX_OPACITY: u8 = 100;
pub const DEFAULT_OPACITY: u8 = 88;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SavedPosition {
    /// Physical pixels, so restoring across DPI changes is deliberate and testable.
    pub x: i32,
    pub y: i32,
    pub monitor_name: Option<String>,
    pub monitor_scale_factor: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub opacity: u8,
    pub position: Option<SavedPosition>,
    pub refresh_interval_seconds: u64,
    pub activity_timeout_seconds: u64,
    pub autostart_enabled: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { opacity: DEFAULT_OPACITY, position: None, refresh_interval_seconds: 60, activity_timeout_seconds: 600, autostart_enabled: false }
    }
}

impl Settings {
    pub fn normalize(&mut self) {
        self.opacity = self.opacity.clamp(MIN_OPACITY, MAX_OPACITY);
        self.refresh_interval_seconds = self.refresh_interval_seconds.max(60);
        self.activity_timeout_seconds = self.activity_timeout_seconds.max(60);
        if self.position.as_ref().is_some_and(|position| !position.monitor_scale_factor.is_finite() || position.monitor_scale_factor <= 0.0) {
            self.position = None;
        }
    }
}

pub fn load(path: &Path) -> Settings {
    let Ok(bytes) = fs::read(path) else { return Settings::default() };
    let Ok(mut settings) = serde_json::from_slice::<Settings>(&bytes) else { return Settings::default() };
    settings.normalize();
    settings
}

pub fn save(path: &Path, settings: &Settings) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "settings path has no parent"))?;
    fs::create_dir_all(parent)?;
    let content = serde_json::to_vec_pretty(settings).map_err(io::Error::other)?;
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, content)?;
    fs::rename(temporary, path)
}

pub fn app_data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("LLMQuotaOverlay")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn invalid_values_are_bounded_and_bad_dpi_is_removed() {
        let mut settings = Settings { opacity: 0, position: Some(SavedPosition { x: 1, y: 2, monitor_name: None, monitor_scale_factor: f64::NAN }), refresh_interval_seconds: 1, activity_timeout_seconds: 2, autostart_enabled: false };
        settings.normalize();
        assert_eq!(settings.opacity, MIN_OPACITY);
        assert!(settings.position.is_none());
        assert_eq!(settings.refresh_interval_seconds, 60);
        assert_eq!(settings.activity_timeout_seconds, 60);
    }
    #[test] fn save_and_load_preserve_opacity_and_position() { let path = std::env::temp_dir().join("llm-overlay-settings-test.json"); let value = Settings { opacity: 73, position: Some(SavedPosition { x: -1200, y: 42, monitor_name: Some("left".into()), monitor_scale_factor: 1.25 }), ..Settings::default() }; save(&path, &value).unwrap(); assert_eq!(load(&path), value); let _ = fs::remove_file(path); }
}
