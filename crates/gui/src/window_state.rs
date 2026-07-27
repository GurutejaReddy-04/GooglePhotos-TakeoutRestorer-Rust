use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WindowState {
    pub width: f32,
    pub height: f32,
    pub x: f32,
    pub y: f32,
    pub is_maximized: bool,
    pub theme: String,
    pub recent_projects: Vec<String>,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            width: 1000.0,
            height: 700.0,
            x: 100.0,
            y: 100.0,
            is_maximized: false,
            theme: "dark".to_string(),
            recent_projects: Vec::new(),
        }
    }
}

impl WindowState {
    fn get_config_path() -> PathBuf {
        let config_dir =
            directories::ProjectDirs::from("", "TakeoutRestorerTeam", "GooglePhotosRestorer")
                .map(|dirs| dirs.config_dir().to_path_buf())
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

        let _ = fs::create_dir_all(&config_dir);

        let mut path = config_dir;
        path.push(".restorer_gui_state.json");
        path
    }

    pub fn load() -> Self {
        let path = Self::get_config_path();
        if path.exists() {
            if let Ok(data) = fs::read_to_string(&path) {
                if let Ok(state) = serde_json::from_str(&data) {
                    return state;
                }
            }
        }
        Self::default()
    }

    pub fn save(&self) {
        let path = Self::get_config_path();
        if let Ok(data) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, data);
        }
    }
}
