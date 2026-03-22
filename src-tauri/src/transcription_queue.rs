use anyhow::Result;
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

use crate::history::HistoryManager;
use crate::transcription;

const MAX_RETRIES: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionJob {
    pub recording_path: String,
    pub srt_path: String,
    pub status: String, // "pending" | "in_progress" | "completed" | "failed"
    pub created_at: String,
    pub progress_percent: u8,
    pub retry_count: u32,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct QueueFile {
    jobs: Vec<TranscriptionJob>,
}

#[derive(Serialize, Clone)]
struct TranscriptionProgressEvent {
    recording_path: String,
    percent: u8,
}

#[derive(Serialize, Clone)]
struct TranscriptionCompletedEvent {
    recording_path: String,
    srt_path: String,
}

#[derive(Serialize, Clone)]
struct TranscriptionFailedEvent {
    recording_path: String,
    error: String,
}

pub struct TranscriptionQueueManager {
    jobs: Mutex<Vec<TranscriptionJob>>,
    queue_path: PathBuf,
    is_processing: Arc<AtomicBool>,
}

impl TranscriptionQueueManager {
    pub fn new() -> Self {
        let queue_path = crate::settings::SettingsManager::config_dir()
            .join("transcription_queue.json");
        let jobs = Self::load_from_file(&queue_path).unwrap_or_default();
        info!("Transcription queue loaded: {} jobs", jobs.len());
        Self {
            jobs: Mutex::new(jobs),
            queue_path,
            is_processing: Arc::new(AtomicBool::new(false)),
        }
    }

    fn load_from_file(path: &PathBuf) -> Result<Vec<TranscriptionJob>> {
        let content = std::fs::read_to_string(path)?;
        let file: QueueFile = serde_json::from_str(&content)?;
        Ok(file.jobs)
    }

    #[allow(dead_code)]
    fn save(&self) {
        let jobs = self.jobs.lock().unwrap();
        self.save_jobs(&jobs);
    }

    fn save_jobs(&self, jobs: &[TranscriptionJob]) {
        if let Some(parent) = self.queue_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = QueueFile {
            jobs: jobs.to_vec(),
        };
        if let Ok(json) = serde_json::to_string_pretty(&file) {
            if let Err(e) = std::fs::write(&self.queue_path, json) {
                warn!("Failed to save transcription queue: {}", e);
            }
        }
    }

    /// Recover interrupted jobs on startup (in_progress → pending).
    pub fn recover_interrupted(&self) {
        let mut jobs = self.jobs.lock().unwrap();
        let mut recovered = 0;
        for job in jobs.iter_mut() {
            if job.status == "in_progress" {
                job.status = "pending".to_string();
                job.progress_percent = 0;
                recovered += 1;
            }
        }
        if recovered > 0 {
            info!("Recovered {} interrupted transcription jobs", recovered);
            self.save_jobs(&jobs);
        }
    }

    /// Add a new transcription job to the queue.
    pub fn enqueue(&self, recording_path: &str, app: &AppHandle) {
        let srt_path = Path::new(recording_path)
            .with_extension("srt")
            .to_string_lossy()
            .into_owned();

        let job = TranscriptionJob {
            recording_path: recording_path.to_string(),
            srt_path,
            status: "pending".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            progress_percent: 0,
            retry_count: 0,
            error: None,
        };

        {
            let mut jobs = self.jobs.lock().unwrap();
            // Don't add duplicate
            if jobs.iter().any(|j| j.recording_path == recording_path && j.status != "completed" && j.status != "failed") {
                info!("Transcription already queued for: {}", recording_path);
                return;
            }
            jobs.push(job);
            self.save_jobs(&jobs);
        }

        info!("Enqueued transcription for: {}", recording_path);

        // Update history status
        let history = app.state::<HistoryManager>();
        history.update_transcription_status(recording_path, "pending", None);

        // Start worker if not running
        self.ensure_worker_running(app.clone());
    }

    /// Check if there are active (pending or in_progress) jobs.
    pub fn has_active_jobs(&self) -> bool {
        let jobs = Self::load_from_file(&self.queue_path)
            .unwrap_or_else(|_| self.jobs.lock().unwrap().clone());
        jobs.iter().any(|j| j.status == "pending" || j.status == "in_progress")
    }

    /// Get all jobs for UI display (reads from disk to get worker updates).
    pub fn get_jobs(&self) -> Vec<TranscriptionJob> {
        Self::load_from_file(&self.queue_path).unwrap_or_else(|_| {
            self.jobs.lock().unwrap().clone()
        })
    }

    /// Retry a failed transcription.
    pub fn retry_job(&self, recording_path: &str, app: &AppHandle) -> Result<(), String> {
        {
            let mut jobs = self.jobs.lock().unwrap();
            if let Some(job) = jobs.iter_mut().find(|j| j.recording_path == recording_path) {
                if job.status != "failed" {
                    return Err("Can only retry failed jobs".into());
                }
                job.status = "pending".to_string();
                job.progress_percent = 0;
                job.error = None;
                job.retry_count = 0;
                self.save_jobs(&jobs);
            } else {
                return Err("Job not found".into());
            }
        }

        let history = app.state::<HistoryManager>();
        history.update_transcription_status(recording_path, "pending", None);

        self.ensure_worker_running(app.clone());
        Ok(())
    }

    /// Cancel/remove a job from the queue.
    pub fn cancel_job(&self, recording_path: &str, app: &AppHandle) -> Result<(), String> {
        {
            let mut jobs = self.jobs.lock().unwrap();
            let before = jobs.len();
            jobs.retain(|j| j.recording_path != recording_path);
            if jobs.len() == before {
                return Err("Job not found".into());
            }
            self.save_jobs(&jobs);
        }

        let history = app.state::<HistoryManager>();
        history.update_transcription_status(recording_path, "cancelled", None);

        Ok(())
    }

    /// Start the background worker if not already running.
    pub fn start_worker(&self, app: AppHandle) {
        self.ensure_worker_running(app);
    }

    fn ensure_worker_running(&self, app: AppHandle) {
        if self.is_processing.compare_exchange(
            false, true, Ordering::SeqCst, Ordering::SeqCst
        ).is_err() {
            return; // Already running
        }

        let is_processing = self.is_processing.clone();
        let queue_path = self.queue_path.clone();

        thread::spawn(move || {
            info!("Transcription worker started");

            loop {
                // Get next pending job
                let job = {
                    let content = std::fs::read_to_string(&queue_path).unwrap_or_default();
                    let mut queue: QueueFile = serde_json::from_str(&content)
                        .unwrap_or(QueueFile { jobs: vec![] });

                    if let Some(job) = queue.jobs.iter_mut().find(|j| j.status == "pending") {
                        job.status = "in_progress".to_string();
                        job.progress_percent = 0;
                        let job_clone = job.clone();
                        // Save updated status
                        if let Ok(json) = serde_json::to_string_pretty(&queue) {
                            let _ = std::fs::write(&queue_path, json);
                        }
                        Some(job_clone)
                    } else {
                        None
                    }
                };

                let job = match job {
                    Some(j) => j,
                    None => {
                        info!("Transcription worker: no more jobs, stopping");
                        break;
                    }
                };

                info!("Transcribing: {}", job.recording_path);

                // Update history
                let history = app.state::<HistoryManager>();
                history.update_transcription_status(&job.recording_path, "in_progress", None);
                let _ = app.emit("transcription-started", &job.recording_path);

                // Check model availability
                if !transcription::is_model_available() {
                    warn!("Whisper model not available, marking job as failed");
                    Self::update_job_on_disk(
                        &queue_path, &job.recording_path,
                        "failed", 0, Some("Whisper model not downloaded"), job.retry_count,
                    );
                    history.update_transcription_status(&job.recording_path, "failed", None);
                    let _ = app.emit("transcription-failed", TranscriptionFailedEvent {
                        recording_path: job.recording_path.clone(),
                        error: "Whisper model not downloaded".into(),
                    });
                    continue;
                }

                // Check recording file exists
                if !Path::new(&job.recording_path).exists() {
                    warn!("Recording file missing: {}", job.recording_path);
                    Self::update_job_on_disk(
                        &queue_path, &job.recording_path,
                        "failed", 0, Some("Recording file not found"), job.retry_count,
                    );
                    history.update_transcription_status(&job.recording_path, "failed", None);
                    let _ = app.emit("transcription-failed", TranscriptionFailedEvent {
                        recording_path: job.recording_path.clone(),
                        error: "Recording file not found".into(),
                    });
                    continue;
                }

                // Run transcription with time-based progress estimation
                let progress = Arc::new(AtomicU8::new(0));

                // Progress reporter thread: emits progress to frontend
                let progress_reporter = progress.clone();
                let reporter_app = app.clone();
                let reporter_path = job.recording_path.clone();
                let reporter_queue_path = queue_path.clone();
                let reporter_running = Arc::new(AtomicBool::new(true));
                let reporter_running_clone = reporter_running.clone();

                let reporter_thread = thread::spawn(move || {
                    let mut last_percent: u8 = 0;
                    while reporter_running_clone.load(Ordering::Relaxed) {
                        let current = progress_reporter.load(Ordering::Relaxed);
                        if current != last_percent {
                            last_percent = current;
                            Self::update_job_on_disk(
                                &reporter_queue_path, &reporter_path,
                                "in_progress", current, None, 0,
                            );
                            let _ = reporter_app.emit("transcription-progress", TranscriptionProgressEvent {
                                recording_path: reporter_path.clone(),
                                percent: current,
                            });
                        }
                        thread::sleep(Duration::from_millis(500));
                    }
                });

                let progress_for_cb = progress.clone();
                let recording_path = job.recording_path.clone();
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    transcription::transcribe(
                        &recording_path,
                        move |percent| {
                            progress_for_cb.store(percent, Ordering::Relaxed);
                        },
                    )
                }));
                let result = match result {
                    Ok(r) => r,
                    Err(panic_info) => {
                        let msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                            format!("Whisper crashed: {}", s)
                        } else if let Some(s) = panic_info.downcast_ref::<String>() {
                            format!("Whisper crashed: {}", s)
                        } else {
                            "Whisper crashed (unknown panic)".to_string()
                        };
                        error!("{}", msg);
                        Err(anyhow::anyhow!("{}", msg))
                    }
                };

                // Stop reporter thread
                reporter_running.store(false, Ordering::Relaxed);
                let _ = reporter_thread.join();

                match result {
                    Ok(segments) => {
                        // Write SRT file
                        let srt_path = Path::new(&job.recording_path).with_extension("srt");
                        match transcription::write_srt(&segments, &srt_path) {
                            Ok(()) => {
                                let srt_str = srt_path.to_string_lossy().into_owned();
                                info!("Transcription completed: {}", srt_str);
                                Self::update_job_on_disk(
                                    &queue_path, &job.recording_path,
                                    "completed", 100, None, 0,
                                );
                                history.update_transcription_status(
                                    &job.recording_path, "completed", Some(&srt_str),
                                );
                                let _ = app.emit("transcription-completed", TranscriptionCompletedEvent {
                                    recording_path: job.recording_path.clone(),
                                    srt_path: srt_str,
                                });
                            }
                            Err(e) => {
                                let err_msg = format!("Failed to write SRT: {}", e);
                                Self::handle_job_error(&queue_path, &job, &err_msg, &app, &history);
                            }
                        }
                    }
                    Err(e) => {
                        let err_msg = format!("Transcription failed: {}", e);
                        Self::handle_job_error(&queue_path, &job, &err_msg, &app, &history);
                    }
                }

                // Small delay between jobs
                thread::sleep(Duration::from_secs(1));
            }

            is_processing.store(false, Ordering::SeqCst);
            info!("Transcription worker stopped");
        });
    }

    fn handle_job_error(
        queue_path: &Path,
        job: &TranscriptionJob,
        error: &str,
        app: &AppHandle,
        history: &HistoryManager,
    ) {
        let new_retry = job.retry_count + 1;
        let new_status = if new_retry < MAX_RETRIES { "pending" } else { "failed" };
        error!("Transcription error (attempt {}/{}): {}", new_retry, MAX_RETRIES, error);

        Self::update_job_on_disk(
            queue_path, &job.recording_path,
            new_status, 0, Some(error), new_retry,
        );
        history.update_transcription_status(&job.recording_path, new_status, None);

        if new_status == "failed" {
            let _ = app.emit("transcription-failed", TranscriptionFailedEvent {
                recording_path: job.recording_path.clone(),
                error: error.to_string(),
            });
        }
    }

    fn update_job_on_disk(
        queue_path: &Path,
        recording_path: &str,
        status: &str,
        progress: u8,
        error: Option<&str>,
        retry_count: u32,
    ) {
        let content = match std::fs::read_to_string(queue_path) {
            Ok(c) => c,
            Err(_) => return,
        };
        let mut queue: QueueFile = match serde_json::from_str(&content) {
            Ok(q) => q,
            Err(_) => return,
        };

        if let Some(job) = queue.jobs.iter_mut().find(|j| j.recording_path == recording_path) {
            job.status = status.to_string();
            job.progress_percent = progress;
            if let Some(err) = error {
                job.error = Some(err.to_string());
            }
            if retry_count > 0 {
                job.retry_count = retry_count;
            }
        }

        if let Ok(json) = serde_json::to_string_pretty(&queue) {
            let _ = std::fs::write(queue_path, json);
        }
    }
}
