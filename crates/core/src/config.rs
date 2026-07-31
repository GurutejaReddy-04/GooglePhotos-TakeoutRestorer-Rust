use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::error::AppError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub exiftool_path: Option<PathBuf>,
    pub supported_image_extensions: Vec<String>,
    pub supported_video_extensions: Vec<String>,
    pub live_photo_pairs: LivePhotoPairs,
    pub processing: ProcessingConfig,
    pub matching: MatchingConfig,
    pub ui: UiConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            exiftool_path: None,
            supported_image_extensions: vec![
                ".jpg".to_string(),
                ".jpeg".to_string(),
                ".heic".to_string(),
                ".png".to_string(),
                ".webp".to_string(),
                ".gif".to_string(),
                ".tiff".to_string(),
                ".tif".to_string(),
                ".bmp".to_string(),
                ".dng".to_string(),
                ".cr2".to_string(),
                ".nef".to_string(),
                ".arw".to_string(),
                ".orf".to_string(),
                ".rw2".to_string(),
            ],
            supported_video_extensions: vec![
                ".mp4".to_string(),
                ".mov".to_string(),
                ".mkv".to_string(),
                ".webm".to_string(),
                ".avi".to_string(),
                ".wmv".to_string(),
                ".flv".to_string(),
            ],
            live_photo_pairs: LivePhotoPairs::default(),
            processing: ProcessingConfig::default(),
            matching: MatchingConfig::default(),
            ui: UiConfig::default(),
        }
    }
}

impl Config {
    /// Attempt to load configuration from the default application data directory.
    pub fn load() -> Result<Self, AppError> {
        let config_dir =
            directories::ProjectDirs::from("", "TakeoutRestorerTeam", "GooglePhotosRestorer")
                .map(|dirs| dirs.config_dir().to_path_buf())
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

        let config_path = config_dir.join("config.toml");

        if !config_path.exists() {
            let default_config = Self::default();
            default_config.save()?; // Generate default on disk
            return Ok(default_config);
        }

        Self::import(&config_path)
    }

    /// Import configuration from a specific path. Applies environment overrides.
    pub fn import(path: &Path) -> Result<Self, AppError> {
        let contents = std::fs::read_to_string(path).map_err(AppError::Io)?;
        let mut config: Config = toml::from_str(&contents).map_err(|e| {
            AppError::Config(format!("Invalid config at {}: {}", path.display(), e))
        })?;

        // Schema Migration (Forward compatibility stub)
        if config.version > 1 {
            return Err(AppError::Config(format!(
                "Unsupported config version: {}. Please update the application.",
                config.version
            )));
        } else if config.version < 1 {
            config.version = 1;
        }

        config.apply_environment_overrides();
        config.validate()?;
        Ok(config)
    }

    /// Apply overrides from the environment (e.g. RESTORER_MAX_WORKERS=8)
    fn apply_environment_overrides(&mut self) {
        if let Ok(val) = std::env::var("RESTORER_MAX_WORKERS") {
            if let Ok(workers) = val.parse::<usize>() {
                self.processing.max_workers = workers;
            }
        }
        if let Ok(val) = std::env::var("RESTORER_LEVENSHTEIN_THRESHOLD") {
            if let Ok(threshold) = val.parse::<u32>() {
                self.matching.levenshtein_threshold = threshold;
            }
        }
    }

    /// Export configuration to a specific path.
    pub fn export(&self, path: &Path) -> Result<(), AppError> {
        let contents = toml::to_string_pretty(self).map_err(|e| AppError::Config(e.to_string()))?;
        std::fs::write(path, contents).map_err(AppError::Io)?;
        Ok(())
    }

    /// Save configuration to the default application data directory.
    pub fn save(&self) -> Result<(), AppError> {
        let config_dir =
            directories::ProjectDirs::from("", "TakeoutRestorerTeam", "GooglePhotosRestorer")
                .map(|dirs| dirs.config_dir().to_path_buf())
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

        if !config_dir.exists() {
            std::fs::create_dir_all(&config_dir)?;
        }

        let config_path = config_dir.join("config.toml");
        self.export(&config_path)
    }

    /// Enforce runtime invariants. Returns error if irrecoverable.
    pub fn validate(&mut self) -> Result<(), AppError> {
        if self.processing.max_workers == 0 {
            self.processing.max_workers = 1;
        }
        if self.matching.levenshtein_threshold == 0 {
            self.matching.levenshtein_threshold = 1;
        }

        // Enforce Phase 4 sidebar boundaries
        self.ui.sidebar_width = self.ui.sidebar_width.clamp(240, 460);

        let normalize_exts = |exts: &mut Vec<String>| {
            for ext in exts.iter_mut() {
                let lower = ext.to_lowercase();
                *ext = if lower.starts_with('.') {
                    lower
                } else {
                    format!(".{}", lower)
                };
            }
        };
        normalize_exts(&mut self.supported_image_extensions);
        normalize_exts(&mut self.supported_video_extensions);

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LivePhotoPairs {
    pub default_image_extension: String,
    pub default_video_extension: String,
}

impl Default for LivePhotoPairs {
    fn default() -> Self {
        Self {
            default_image_extension: ".jpg".to_string(),
            default_video_extension: ".mov".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum OutputMode {
    #[default]
    Copy,
    InPlace,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ProcessingConfig {
    pub max_workers: usize,
    pub gps_enabled: bool,
    pub timezone_enabled: bool,
    pub unmatched_enabled: bool,
    pub anonymous_logging: bool,
    pub output_mode: OutputMode,
    pub high_performance: bool,
}

impl Default for ProcessingConfig {
    fn default() -> Self {
        Self {
            max_workers: 4,
            gps_enabled: true,
            timezone_enabled: true,
            unmatched_enabled: true,
            anonymous_logging: false,
            output_mode: OutputMode::default(),
            high_performance: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiConfig {
    pub theme: String,
    pub window_width: u32,
    pub window_height: u32,
    pub window_maximized: bool,
    pub window_x: Option<i32>,
    pub window_y: Option<i32>,
    pub sidebar_width: u32,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: "System".to_string(),
            window_width: 1000,
            window_height: 750,
            window_maximized: false,
            window_x: None,
            window_y: None,
            sidebar_width: 340,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MatchingConfig {
    pub levenshtein_threshold: u32,
    pub min_truncation_length: usize,
}

impl Default for MatchingConfig {
    fn default() -> Self {
        Self {
            levenshtein_threshold: 3,
            min_truncation_length: 8,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_config_default_and_validate() {
        let mut config = Config::default();
        assert_eq!(config.version, 1);
        config.validate().unwrap();
        assert!(config.processing.max_workers >= 1);
    }

    #[test]
    fn test_config_validation_fixes_zero_workers() {
        let mut config = Config::default();
        config.processing.max_workers = 0;
        config.validate().unwrap();
        assert_eq!(config.processing.max_workers, 1);
    }

    #[test]
    fn test_config_validation_normalizes_extensions() {
        let mut config = Config {
            supported_image_extensions: vec![
                "JPG".to_string(),
                ".PNG".to_string(),
                "heic".to_string(),
            ],
            ..Config::default()
        };
        config.validate().unwrap();
        assert_eq!(
            config.supported_image_extensions,
            vec![".jpg".to_string(), ".png".to_string(), ".heic".to_string()]
        );
    }

    #[test]
    fn test_config_import_export() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let config = Config::default();
        config.export(&path).unwrap();

        let imported = Config::import(&path).unwrap();
        assert_eq!(config, imported);
    }

    #[test]
    fn test_deny_unknown_fields() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("invalid.toml");

        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"version = 1\nunknown_field = true\n")
            .unwrap();

        let result = Config::import(&path);
        assert!(result.is_err());
        if let Err(crate::error::AppError::Config(msg)) = result {
            assert!(msg.contains("unknown field `unknown_field`"));
        } else {
            panic!("Expected Config error");
        }
    }

    #[test]
    fn test_p0_005_preserves_user_max_workers_below_12() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let mut config = Config::default();
        config.processing.max_workers = 4; // Value <= 12
        config.export(&path).unwrap();

        let imported = Config::import(&path).unwrap();
        assert_eq!(
            imported.processing.max_workers, 4,
            "Config import must preserve user max_workers setting even if <= 12"
        );
    }
}
