use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize)]
pub struct WindowState {
    pub width: f32,
    pub height: f32,
}

impl WindowState {
    pub const DEFAULT_WIDTH: f32 = 800.;
    pub const DEFAULT_HEIGHT: f32 = 520.;

    fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("gpui-deb-installer").join("window.json"))
    }

    pub fn load() -> Self {
        Self::config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Self {
                width: Self::DEFAULT_WIDTH,
                height: Self::DEFAULT_HEIGHT,
            })
    }

    pub fn save(width: f32, height: f32) {
        let Some(path) = Self::config_path() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let state = Self { width, height };
        if let Ok(json) = serde_json::to_string(&state) {
            let _ = std::fs::write(path, json);
        }
    }
}
