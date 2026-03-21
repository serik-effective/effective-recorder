use anyhow::{Context, Result};
use chrono::Local;
use crossbeam_channel::bounded;
use log::{info, warn};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

use crate::audio::{AudioCapture, AudioSamples};
use crate::encoder::{find_preset, Encoder, EncoderConfig};
use crate::screen::{ScreenCapture, VideoFrame};

#[derive(Serialize, Clone)]
pub struct RecordingStatus {
    pub elapsed_seconds: u64,
    pub file_size_bytes: u64,
}

struct RecorderState {
    is_recording: Arc<AtomicBool>,
    output_path: PathBuf,
    start_time: Instant,
    video_thread: Option<JoinHandle<()>>,
    encoder_thread: Option<JoinHandle<()>>,
    audio_thread: Option<JoinHandle<()>>,
    status_thread: Option<JoinHandle<()>>,
    /// VideoRecorder handle for streaming capture (must be stopped on drop)
    video_recorder: Option<xcap::VideoRecorder>,
}

pub struct Recorder {
    state: std::sync::Mutex<Option<RecorderState>>,
}

unsafe impl Send for Recorder {}
unsafe impl Sync for Recorder {}

impl Recorder {
    pub fn new() -> Self {
        Self {
            state: std::sync::Mutex::new(None),
        }
    }

    pub fn is_recording(&self) -> bool {
        self.state
            .lock()
            .unwrap()
            .as_ref()
            .map(|s| s.is_recording.load(Ordering::Relaxed))
            .unwrap_or(false)
    }

    pub fn start(
        &self,
        app: &AppHandle,
        audio_device: Option<String>,
        preset_id: Option<String>,
        output_dir_override: Option<PathBuf>,
    ) -> Result<String> {
        let mut state = self.state.lock().unwrap();
        if state.is_some() {
            anyhow::bail!("Recording is already in progress");
        }

        let preset = find_preset(preset_id.as_deref().unwrap_or("daily"));

        let timestamp = Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
        let filename = format!("recording_{}.mp4", timestamp);

        let output_dir = output_dir_override.unwrap_or(Self::get_output_dir()?);
        std::fs::create_dir_all(&output_dir).context("Failed to create output directory")?;
        let output_path = output_dir.join(&filename);

        info!(
            "Starting: {} [{}] preset={:?}",
            output_path.display(),
            preset.name,
            preset.id
        );

        // Screen capture at preset FPS
        let screen =
            ScreenCapture::new(preset.fps).context("Failed to initialize screen capture")?;

        let (out_width, out_height) =
            preset.output_dimensions(screen.width(), screen.height());
        let capture_width = screen.capture_width();
        let capture_height = screen.capture_height();

        info!(
            "Resolution: capture={}x{} → output={}x{} @ {} FPS",
            capture_width, capture_height, out_width, out_height, preset.fps
        );

        // Audio
        let audio_info = match AudioCapture::new(audio_device.as_deref()) {
            Ok(a) => Some((a.sample_rate(), a.channels())),
            Err(e) => {
                warn!("Audio not available: {}. Video only.", e);
                None
            }
        };

        let is_recording = Arc::new(AtomicBool::new(true));
        let file_size = Arc::new(AtomicU64::new(0));
        let start_time = Instant::now();

        let (video_tx, video_rx) = bounded::<VideoFrame>(30);
        let (audio_tx, audio_rx) = bounded::<AudioSamples>(200);

        let (audio_sample_rate, audio_channels) = audio_info.unwrap_or((44100, 1));

        let encoder_config = EncoderConfig {
            width: out_width,
            height: out_height,
            capture_width,
            capture_height,
            fps: preset.fps,
            audio_sample_rate,
            audio_channels,
            output_path: output_path.to_string_lossy().into_owned(),
            preset,
        };

        let encoder_thread = Encoder::start(
            encoder_config,
            is_recording.clone(),
            video_rx,
            audio_rx,
            file_size.clone(),
        );

        let video_thread = screen.start(is_recording.clone(), video_tx, start_time);

        let audio_is_recording = is_recording.clone();
        let audio_device_clone = audio_device.clone();
        let audio_thread = thread::spawn(move || {
            let audio = match AudioCapture::new(audio_device_clone.as_deref()) {
                Ok(a) => a,
                Err(e) => {
                    warn!("Audio thread: {}", e);
                    while audio_is_recording.load(Ordering::Relaxed) {
                        thread::sleep(Duration::from_millis(100));
                    }
                    return;
                }
            };

            match audio.start(audio_is_recording.clone(), audio_tx, start_time) {
                Ok(stream) => {
                    while audio_is_recording.load(Ordering::Relaxed) {
                        thread::sleep(Duration::from_millis(50));
                    }
                    drop(stream);
                }
                Err(e) => {
                    warn!("Audio start failed: {}", e);
                    while audio_is_recording.load(Ordering::Relaxed) {
                        thread::sleep(Duration::from_millis(100));
                    }
                }
            }
        });

        let status_recording = is_recording.clone();
        let status_file_size = file_size.clone();
        let app_handle = app.clone();
        let status_thread = thread::spawn(move || {
            let start = Instant::now();
            while status_recording.load(Ordering::Relaxed) {
                let status = RecordingStatus {
                    elapsed_seconds: start.elapsed().as_secs(),
                    file_size_bytes: status_file_size.load(Ordering::Relaxed),
                };
                let _ = app_handle.emit("recording-status", &status);
                thread::sleep(Duration::from_secs(1));
            }
        });

        *state = Some(RecorderState {
            is_recording,
            output_path,
            start_time,
            video_thread: Some(video_thread),
            encoder_thread: Some(encoder_thread),
            audio_thread: Some(audio_thread),
            status_thread: Some(status_thread),
            video_recorder: None,
        });

        Ok(filename)
    }

    pub fn stop(&self) -> Result<String> {
        let (path, _) = self.stop_with_duration()?;
        Ok(path)
    }

    /// Stop recording and return (path, duration_seconds).
    pub fn stop_with_duration(&self) -> Result<(String, u64)> {
        let mut state_lock = self.state.lock().unwrap();
        let mut state = state_lock.take().context("No recording in progress")?;

        let duration = state.start_time.elapsed().as_secs();
        info!("Stopping recording...");
        state.is_recording.store(false, Ordering::Relaxed);

        // Stop video recorder first (if streaming capture was used)
        if let Some(rec) = state.video_recorder.take() {
            let _ = rec.stop();
        }

        if let Some(t) = state.audio_thread.take() { let _ = t.join(); }
        if let Some(t) = state.video_thread.take() { let _ = t.join(); }
        if let Some(t) = state.encoder_thread.take() { let _ = t.join(); }
        if let Some(t) = state.status_thread.take() { let _ = t.join(); }

        let path = state.output_path.to_string_lossy().into_owned();
        info!("Saved: {} ({}s)", path, duration);

        Ok((path, duration))
    }

    fn get_output_dir() -> Result<PathBuf> {
        if cfg!(target_os = "macos") {
            dirs::home_dir()
                .map(|h| h.join("Movies"))
                .context("Could not determine home directory")
        } else {
            dirs::video_dir().context("Could not determine video directory")
        }
    }
}
