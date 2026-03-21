use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use log::{info, warn};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::settings::SettingsManager;

const MAX_HISTORY: usize = 500;
const DISPLAY_LIMIT: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingEntry {
    pub filename: String,
    pub path: String,
    pub size_bytes: u64,
    pub duration_seconds: u64,
    pub recorded_at: String,    // ISO 8601
    pub preset_id: String,
    pub preset_name: String,
    pub status: String,         // "exists" or "missing"
    #[serde(default)]
    pub transcript_path: Option<String>,
    #[serde(default)]
    pub transcription_status: Option<String>, // None | "pending" | "in_progress" | "completed" | "failed"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HistoryFile {
    recordings: Vec<RecordingEntry>,
}

pub struct HistoryManager {
    entries: Mutex<Vec<RecordingEntry>>,
    path: PathBuf,
}

impl HistoryManager {
    pub fn new() -> Self {
        let path = SettingsManager::config_dir().join("recordings_history.json");
        let entries = Self::load_from_file(&path).unwrap_or_default();
        info!("History loaded: {} recordings", entries.len());
        Self {
            entries: Mutex::new(entries),
            path,
        }
    }

    fn load_from_file(path: &PathBuf) -> Result<Vec<RecordingEntry>> {
        let content = std::fs::read_to_string(path).context("Cannot read history")?;
        let file: HistoryFile = serde_json::from_str(&content).context("Cannot parse history")?;
        Ok(file.recordings)
    }

    fn save_to_file(&self, entries: &[RecordingEntry]) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = HistoryFile {
            recordings: entries.to_vec(),
        };
        let json = serde_json::to_string_pretty(&file)?;
        std::fs::write(&self.path, json)?;
        Ok(())
    }

    /// Add a new recording entry (called after recording stops).
    pub fn add_recording(
        &self,
        path: &str,
        duration_seconds: u64,
        preset_id: &str,
        preset_name: &str,
    ) {
        let file_path = Path::new(path);
        let size_bytes = std::fs::metadata(file_path)
            .map(|m| m.len())
            .unwrap_or(0);

        let entry = RecordingEntry {
            filename: file_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            path: path.to_string(),
            size_bytes,
            duration_seconds,
            recorded_at: Utc::now().to_rfc3339(),
            preset_id: preset_id.to_string(),
            preset_name: preset_name.to_string(),
            status: if file_path.exists() { "exists" } else { "missing" }.to_string(),
            transcript_path: None,
            transcription_status: None,
        };

        let mut entries = self.entries.lock().unwrap();
        entries.insert(0, entry);

        // Trim to MAX_HISTORY
        if entries.len() > MAX_HISTORY {
            entries.truncate(MAX_HISTORY);
        }

        if let Err(e) = self.save_to_file(&entries) {
            warn!("Failed to save history: {}", e);
        }
    }

    /// Get history entries (limited to DISPLAY_LIMIT), sorted newest first.
    /// Syncs file status (exists/missing).
    pub fn get_entries(&self) -> Vec<RecordingEntry> {
        let mut entries = self.entries.lock().unwrap();
        // Sync statuses
        for entry in entries.iter_mut() {
            entry.status = if Path::new(&entry.path).exists() {
                // Also update size in case file was modified
                if let Ok(meta) = std::fs::metadata(&entry.path) {
                    entry.size_bytes = meta.len();
                }
                "exists".to_string()
            } else {
                "missing".to_string()
            };
        }
        let _ = self.save_to_file(&entries);
        entries.iter().take(DISPLAY_LIMIT).cloned().collect()
    }

    /// Remove an entry from history by path.
    pub fn remove_entry(&self, path: &str) {
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|e| e.path != path);
        let _ = self.save_to_file(&entries);
    }

    /// Delete a recording file and remove from history.
    pub fn delete_recording(&self, path: &str) -> Result<()> {
        let file_path = Path::new(path);
        if file_path.exists() {
            std::fs::remove_file(file_path).context("Failed to delete file")?;
            info!("Deleted recording: {}", path);
        }
        self.remove_entry(path);
        Ok(())
    }

    /// Update transcription status for a recording.
    pub fn update_transcription_status(
        &self,
        recording_path: &str,
        status: &str,
        transcript_path: Option<&str>,
    ) {
        let mut entries = self.entries.lock().unwrap();
        if let Some(entry) = entries.iter_mut().find(|e| e.path == recording_path) {
            entry.transcription_status = Some(status.to_string());
            if let Some(tp) = transcript_path {
                entry.transcript_path = Some(tp.to_string());
            }
            let _ = self.save_to_file(&entries);
        }
    }

    /// Rescan: check all entries for file existence.
    pub fn rescan(&self) -> Vec<RecordingEntry> {
        self.get_entries()
    }

    /// Remove entries where file is missing.
    pub fn remove_missing(&self) -> usize {
        let mut entries = self.entries.lock().unwrap();
        let before = entries.len();
        entries.retain(|e| Path::new(&e.path).exists());
        let removed = before - entries.len();
        if removed > 0 {
            let _ = self.save_to_file(&entries);
        }
        removed
    }

    /// Get mutable access to entries for cleanup operations.
    /// Returns paths of entries that should be cleaned up based on policy.
    pub fn get_cleanup_candidates(
        &self,
        retention_days: u32,
        delete_large: bool,
        large_threshold_mb: u32,
        max_delete: u32,
        recordings_dir: &Path,
    ) -> Vec<String> {
        let entries = self.entries.lock().unwrap();
        let now = Utc::now();
        let mut candidates: Vec<String> = Vec::new();

        for entry in entries.iter() {
            if candidates.len() >= max_delete as usize {
                break;
            }

            let path = Path::new(&entry.path);

            // Safety: only delete files within the recordings directory
            if !path.starts_with(recordings_dir) {
                continue;
            }

            if !path.exists() {
                continue;
            }

            let recorded_at = match DateTime::parse_from_rfc3339(&entry.recorded_at) {
                Ok(dt) => dt.with_timezone(&Utc),
                Err(_) => continue,
            };

            let age_days = (now - recorded_at).num_days();

            // Rule 1: older than retention_days
            if age_days >= retention_days as i64 {
                candidates.push(entry.path.clone());
                continue;
            }

            // Rule 2: large old files (older than 1 day)
            if delete_large && age_days >= 1 {
                let size_mb = entry.size_bytes / (1024 * 1024);
                if size_mb >= large_threshold_mb as u64 {
                    candidates.push(entry.path.clone());
                }
            }
        }

        candidates
    }

    /// Execute cleanup: delete files and remove from history.
    pub fn execute_cleanup(&self, paths: &[String]) -> usize {
        let mut deleted = 0;
        for path in paths {
            let file_path = Path::new(path);
            if file_path.exists() {
                if let Err(e) = std::fs::remove_file(file_path) {
                    warn!("Cleanup: failed to delete {}: {}", path, e);
                    continue;
                }
            }
            self.remove_entry(path);
            deleted += 1;
            info!("Cleanup: deleted {}", path);
        }
        deleted
    }
}
