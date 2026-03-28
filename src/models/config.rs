use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "AppConfig::default_width")]
    pub width: f32,
    #[serde(default = "AppConfig::default_height")]
    pub height: f32,
    #[serde(default)]
    pub auto_close: bool,
}

impl AppConfig {
    fn default_width() -> f32 { 800. }
    fn default_height() -> f32 { 520. }

    fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("gpui-deb-installer").join("config.json"))
    }

    pub fn load() -> Self {
        // Also try the old window.json path for migration
        let path = Self::config_path();
        path.as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = Self::config_path() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string(self) {
            let _ = std::fs::write(path, json);
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            width: Self::default_width(),
            height: Self::default_height(),
            auto_close: false,
        }
    }
}
