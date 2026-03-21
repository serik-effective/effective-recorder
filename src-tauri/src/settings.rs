use anyhow::{Context, Result};
use log::info;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoragePolicy {
    #[serde(default = "default_false")]
    pub cleanup_enabled: bool,
    #[serde(default = "default_30")]
    pub retention_days: u32,
    #[serde(default = "default_false")]
    pub delete_large_old_files: bool,
    #[serde(default = "default_500")]
    pub large_file_threshold_mb: u32,
    #[serde(default = "default_20")]
    pub max_files_to_delete_per_run: u32,
}

fn default_false() -> bool { false }
fn default_30() -> u32 { 30 }
fn default_500() -> u32 { 500 }
fn default_20() -> u32 { 20 }

impl Default for StoragePolicy {
    fn default() -> Self {
        Self {
            cleanup_enabled: false,
            retention_days: 30,
            delete_large_old_files: false,
            large_file_threshold_mb: 500,
            max_files_to_delete_per_run: 20,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default)]
    pub output_dir: Option<String>,
    #[serde(default = "default_preset")]
    pub selected_preset_id: String,
    #[serde(default)]
    pub audio_device: Option<String>,
    #[serde(default)]
    pub storage_policy: StoragePolicy,
    // Legacy field — kept for backwards compat, maps to selected_preset_id
    #[serde(default, skip_serializing)]
    pub quality: Option<String>,
}

fn default_preset() -> String { "daily".to_string() }

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            output_dir: None,
            selected_preset_id: "daily".to_string(),
            audio_device: None,
            storage_policy: StoragePolicy::default(),
            quality: None,
        }
    }
}

/// Persists last camera overlay position between recordings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CameraPosition {
    pub x: f64,
    pub y: f64,
}

impl CameraPosition {
    fn path() -> PathBuf {
        SettingsManager::config_dir().join("camera_position.json")
    }

    pub fn load() -> Option<Self> {
        let content = std::fs::read_to_string(Self::path()).ok()?;
        serde_json::from_str(&content).ok()
    }

    pub fn save(x: f64, y: f64) {
        let pos = Self { x, y };
        if let Ok(json) = serde_json::to_string(&pos) {
            let _ = std::fs::write(Self::path(), json);
        }
    }
}

pub struct SettingsManager {
    settings: Mutex<AppSettings>,
    path: PathBuf,
}

impl SettingsManager {
    pub fn new() -> Self {
        let path = Self::settings_path();
        let mut settings = Self::load_from_file(&path).unwrap_or_default();
        // Migrate legacy "quality" field
        if let Some(q) = settings.quality.take() {
            if settings.selected_preset_id == "daily" {
                settings.selected_preset_id = q;
            }
        }
        info!("Settings loaded: {:?}", settings);
        Self {
            settings: Mutex::new(settings),
            path,
        }
    }

    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".config"))
            .join("effective-recorder")
    }

    fn settings_path() -> PathBuf {
        Self::config_dir().join("settings.json")
    }

    fn load_from_file(path: &PathBuf) -> Result<AppSettings> {
        let content = std::fs::read_to_string(path).context("Cannot read settings file")?;
        serde_json::from_str(&content).context("Cannot parse settings")
    }

    pub fn get(&self) -> AppSettings {
        self.settings.lock().unwrap().clone()
    }

    pub fn save(&self, new_settings: AppSettings) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&new_settings)?;
        std::fs::write(&self.path, json)?;
        info!("Settings saved: preset={}", new_settings.selected_preset_id);
        *self.settings.lock().unwrap() = new_settings;
        Ok(())
    }

    pub fn output_dir(&self) -> PathBuf {
        let s = self.settings.lock().unwrap();
        if let Some(ref dir) = s.output_dir {
            if !dir.is_empty() {
                return PathBuf::from(dir);
            }
        }
        Self::default_output_dir()
    }

    pub fn default_output_dir() -> PathBuf {
        if cfg!(target_os = "macos") {
            dirs::home_dir().unwrap_or_default().join("Movies")
        } else {
            dirs::video_dir().unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join("Videos"))
        }
    }
}
