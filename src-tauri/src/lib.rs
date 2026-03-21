mod audio;
mod encoder;
mod history;
mod permissions;
mod recorder;
mod screen;
mod settings;
mod transcription;
mod transcription_queue;

use crate::audio::AudioCapture;
use crate::encoder::{
    create_preset, delete_preset, duplicate_preset, find_preset,
    get_valid_encoder_presets, load_presets, reset_presets_to_defaults, update_preset, QualityPreset,
};
use crate::history::{HistoryManager, RecordingEntry};
use crate::recorder::Recorder;
use crate::settings::{AppSettings, CameraPosition, SettingsManager};
use crate::transcription_queue::{TranscriptionJob, TranscriptionQueueManager};
use log::info;
use tauri::{
    image::Image,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    Emitter, Manager,
};

// ── Recording commands ─────────────────────────────────────────────

#[tauri::command]
async fn start_recording(
    app: tauri::AppHandle,
    state: tauri::State<'_, Recorder>,
    settings_state: tauri::State<'_, SettingsManager>,
    audio_device: Option<String>,
    quality: Option<String>,
) -> Result<String, String> {
    let s = settings_state.get();
    let device = audio_device.or(s.audio_device);
    let q = quality.unwrap_or(s.selected_preset_id);
    let output_dir = settings_state.output_dir();

    state
        .start(&app, device, Some(q), Some(output_dir))
        .map_err(|e: anyhow::Error| e.to_string())
}

#[tauri::command]
async fn stop_recording(
    app: tauri::AppHandle,
    state: tauri::State<'_, Recorder>,
    settings_state: tauri::State<'_, SettingsManager>,
    history_state: tauri::State<'_, HistoryManager>,
    queue_state: tauri::State<'_, TranscriptionQueueManager>,
) -> Result<String, String> {
    let preset_id = settings_state.get().selected_preset_id;
    let preset = find_preset(&preset_id);
    let (path, duration) = state.stop_with_duration().map_err(|e| e.to_string())?;

    // Add to history
    history_state.add_recording(&path, duration, &preset.id, &preset.name);

    // Run cleanup after recording
    run_cleanup_internal(&settings_state, &history_state);

    // Enqueue transcription if preset has auto_transcribe enabled
    if preset.auto_transcribe {
        queue_state.enqueue(&path, &app);
    }

    Ok(path)
}

#[tauri::command]
fn is_recording(state: tauri::State<'_, Recorder>) -> bool {
    state.is_recording()
}

// ── Audio commands ─────────────────────────────────────────────────

#[tauri::command]
fn list_audio_devices() -> Result<Vec<String>, String> {
    AudioCapture::list_devices().map_err(|e: anyhow::Error| e.to_string())
}

// ── Permission commands ────────────────────────────────────────────

#[tauri::command]
fn check_screen_permission() -> bool {
    permissions::has_screen_recording_permission()
}

#[tauri::command]
fn request_screen_permission() {
    permissions::request_screen_recording_permission();
}

#[tauri::command]
fn open_screen_settings() {
    permissions::open_screen_recording_settings();
}

// ── Settings commands ──────────────────────────────────────────────

#[tauri::command]
fn get_settings(state: tauri::State<'_, SettingsManager>) -> AppSettings {
    state.get()
}

#[tauri::command]
fn save_settings(
    state: tauri::State<'_, SettingsManager>,
    settings: AppSettings,
) -> Result<(), String> {
    state.save(settings).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_default_output_dir() -> String {
    SettingsManager::default_output_dir()
        .to_string_lossy()
        .into_owned()
}

// ── Preset commands ────────────────────────────────────────────────

#[tauri::command]
fn list_presets() -> Vec<QualityPreset> {
    load_presets()
}

#[tauri::command]
fn cmd_create_preset(preset: QualityPreset) -> Result<QualityPreset, String> {
    create_preset(preset)
}

#[tauri::command]
fn cmd_update_preset(preset: QualityPreset) -> Result<QualityPreset, String> {
    update_preset(preset)
}

#[tauri::command]
fn cmd_delete_preset(
    id: String,
    settings_state: tauri::State<'_, SettingsManager>,
) -> Result<(), String> {
    let s = settings_state.get();
    delete_preset(&id)?;
    if s.selected_preset_id == id {
        let remaining = load_presets();
        if let Some(first) = remaining.first() {
            let mut new_s = settings_state.get();
            new_s.selected_preset_id = first.id.clone();
            let _ = settings_state.save(new_s);
        }
    }
    Ok(())
}

#[tauri::command]
fn cmd_duplicate_preset(id: String) -> Result<QualityPreset, String> {
    duplicate_preset(&id)
}

#[tauri::command]
fn cmd_reset_presets(
    settings_state: tauri::State<'_, SettingsManager>,
) -> Vec<QualityPreset> {
    let presets = reset_presets_to_defaults();
    // Reset selected preset to first default
    if let Some(first) = presets.first() {
        let mut s = settings_state.get();
        s.selected_preset_id = first.id.clone();
        let _ = settings_state.save(s);
    }
    presets
}

#[tauri::command]
fn cmd_valid_encoder_presets() -> Vec<String> {
    get_valid_encoder_presets()
}

// ── History commands ───────────────────────────────────────────────

#[tauri::command]
fn get_history(state: tauri::State<'_, HistoryManager>) -> Vec<RecordingEntry> {
    state.get_entries()
}

#[tauri::command]
fn delete_recording_entry(
    state: tauri::State<'_, HistoryManager>,
    path: String,
) -> Result<(), String> {
    state.delete_recording(&path).map_err(|e| e.to_string())
}

#[tauri::command]
fn remove_from_history(state: tauri::State<'_, HistoryManager>, path: String) {
    state.remove_entry(&path);
}

#[tauri::command]
fn rescan_history(state: tauri::State<'_, HistoryManager>) -> Vec<RecordingEntry> {
    state.rescan()
}

#[tauri::command]
fn remove_missing_entries(state: tauri::State<'_, HistoryManager>) -> usize {
    state.remove_missing()
}

#[tauri::command]
fn open_file(path: String) -> Result<(), String> {
    opener::open(&path).map_err(|e| e.to_string())
}

#[tauri::command]
fn read_text_file(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| e.to_string())
}

#[tauri::command]
fn reveal_in_folder(path: String) -> Result<(), String> {
    opener::reveal(&path).map_err(|e| e.to_string())
}

// ── Cleanup commands ───────────────────────────────────────────────

#[tauri::command]
fn run_cleanup(
    settings_state: tauri::State<'_, SettingsManager>,
    history_state: tauri::State<'_, HistoryManager>,
) -> usize {
    run_cleanup_internal(&settings_state, &history_state)
}

fn run_cleanup_internal(
    settings_state: &SettingsManager,
    history_state: &HistoryManager,
) -> usize {
    let s = settings_state.get();
    let policy = &s.storage_policy;

    if !policy.cleanup_enabled {
        return 0;
    }

    let recordings_dir = settings_state.output_dir();
    let candidates = history_state.get_cleanup_candidates(
        policy.retention_days,
        policy.delete_large_old_files,
        policy.large_file_threshold_mb,
        policy.max_files_to_delete_per_run,
        &recordings_dir,
    );

    if candidates.is_empty() {
        return 0;
    }

    info!("Cleanup: {} candidates", candidates.len());
    history_state.execute_cleanup(&candidates)
}

// ── Transcription commands ─────────────────────────────────────────

#[tauri::command]
fn is_whisper_model_available() -> bool {
    transcription::is_model_available()
}

#[tauri::command]
async fn download_whisper_model(app: tauri::AppHandle) -> Result<(), String> {
    transcription::download_model(app)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_transcription_queue(
    state: tauri::State<'_, TranscriptionQueueManager>,
) -> Vec<TranscriptionJob> {
    state.get_jobs()
}

#[tauri::command]
#[tauri::command]
fn start_transcription(
    app: tauri::AppHandle,
    state: tauri::State<'_, TranscriptionQueueManager>,
    path: String,
) -> Result<(), String> {
    state.enqueue(&path, &app);
    Ok(())
}

#[tauri::command]
fn retry_transcription(
    app: tauri::AppHandle,
    state: tauri::State<'_, TranscriptionQueueManager>,
    path: String,
) -> Result<(), String> {
    state.retry_job(&path, &app)
}

#[tauri::command]
fn cancel_transcription(
    app: tauri::AppHandle,
    state: tauri::State<'_, TranscriptionQueueManager>,
    path: String,
) -> Result<(), String> {
    state.cancel_job(&path, &app)
}

#[tauri::command]
fn force_close_app(window: tauri::Window) {
    window.destroy().unwrap_or_default();
}

// ── Camera overlay commands ───────────────────────────────────────

#[tauri::command]
async fn open_camera_overlay(
    app: tauri::AppHandle,
    device_id: Option<String>,
    size: Option<String>,
    shape: Option<String>,
) -> Result<(), String> {
    // Close existing overlay if any
    if let Some(w) = app.get_webview_window("camera-overlay") {
        let _ = w.close();
    }

    let size_str = size.unwrap_or_else(|| "medium".into());
    let shape_str = shape.unwrap_or_else(|| "circle".into());

    let dimension: f64 = match size_str.as_str() {
        "small" => 150.0,
        "large" => 300.0,
        _ => 200.0,
    };

    // Get saved position or calculate default (80% of screen)
    let saved_pos = CameraPosition::load();
    let (pos_x, pos_y) = if let Some(pos) = saved_pos {
        (pos.x, pos.y)
    } else {
        // Default: ~80% from top-left corner of the primary monitor
        if let Some(monitor) = app.primary_monitor().map_err(|e| e.to_string())?.as_ref() {
            let screen = monitor.size();
            let scale = monitor.scale_factor();
            let logical_w = screen.width as f64 / scale;
            let logical_h = screen.height as f64 / scale;
            (logical_w * 0.8 - dimension / 2.0, logical_h * 0.8 - dimension / 2.0)
        } else {
            (800.0, 500.0)
        }
    };

    // Build URL with query params
    let mut params = vec![
        format!("size={}", size_str),
        format!("shape={}", shape_str),
    ];
    if let Some(ref id) = device_id {
        params.push(format!("deviceId={}", id));
    }
    let url = format!("/camera-overlay?{}", params.join("&"));

    tauri::WebviewWindowBuilder::new(
        &app,
        "camera-overlay",
        tauri::WebviewUrl::App(url.into()),
    )
    .title("Camera")
    .inner_size(dimension + 4.0, dimension + 4.0)
    .position(pos_x, pos_y)
    .always_on_top(true)
    .decorations(false)
    .skip_taskbar(true)
    .resizable(false)
    .shadow(false)
    .transparent(true)
    .build()
    .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
async fn close_camera_overlay(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("camera-overlay") {
        // Save position before closing
        if let Ok(pos) = w.outer_position() {
            let scale = w.scale_factor().unwrap_or(1.0);
            CameraPosition::save(pos.x as f64 / scale, pos.y as f64 / scale);
        }
        w.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn save_camera_position(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("camera-overlay") {
        if let Ok(pos) = w.outer_position() {
            let scale = w.scale_factor().unwrap_or(1.0);
            CameraPosition::save(pos.x as f64 / scale, pos.y as f64 / scale);
        }
    }
    Ok(())
}

// ── Tray & shortcuts ───────────────────────────────────────────────

fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let start_item = MenuItemBuilder::with_id("start", "Start Recording").build(app)?;
    let stop_item = MenuItemBuilder::with_id("stop", "Stop Recording").build(app)?;
    let quit_item = MenuItemBuilder::with_id("quit", "Quit").build(app)?;

    let menu = MenuBuilder::new(app)
        .item(&start_item)
        .item(&stop_item)
        .separator()
        .item(&quit_item)
        .build()?;

    let _tray = TrayIconBuilder::new()
        .menu(&menu)
        .tooltip("Effective Recorder")
        .icon(Image::from_bytes(include_bytes!("../icons/32x32.png"))?)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "start" => { let _ = app.emit("tray-start-recording", ()); }
            "stop" => { let _ = app.emit("tray-stop-recording", ()); }
            "quit" => { app.exit(0); }
            _ => {}
        })
        .build(app)?;

    Ok(())
}

fn setup_global_shortcuts(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
        let shortcut: Shortcut = "CommandOrControl+Shift+R".parse()?;
        app.global_shortcut().on_shortcut(shortcut, move |app, _scut, event| {
            if event.state == ShortcutState::Pressed {
                let _ = app.emit("global-toggle-recording", ());
            }
        })?;
    }
    Ok(())
}

// ── Entry point ────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();

    let settings_mgr = SettingsManager::new();
    let history_mgr = HistoryManager::new();

    // First-run preset logic
    {
        let s = settings_mgr.get();
        let presets = load_presets();
        let selected_exists = presets.iter().any(|p| p.id == s.selected_preset_id);
        if !selected_exists {
            if let Some(first) = presets.first() {
                info!("Selected preset '{}' not found, switching to '{}'", s.selected_preset_id, first.id);
                let mut new_s = s.clone();
                new_s.selected_preset_id = first.id.clone();
                let _ = settings_mgr.save(new_s);
            }
        }
    }

    // Run cleanup on startup
    run_cleanup_internal(&settings_mgr, &history_mgr);

    // Transcription queue: recover interrupted jobs
    let queue_mgr = TranscriptionQueueManager::new();
    queue_mgr.recover_interrupted();

    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init());

    #[cfg(target_os = "macos")]
    {
        builder = builder.plugin(tauri_plugin_macos_permissions::init());
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        builder = builder.plugin(tauri_plugin_global_shortcut::Builder::new().build());
    }

    builder
        .manage(Recorder::new())
        .manage(settings_mgr)
        .manage(history_mgr)
        .manage(queue_mgr)
        .setup(|app| {
            let has_perm = permissions::has_screen_recording_permission();
            log::info!("Startup: screen recording permission = {}", has_perm);
            setup_tray(app)?;
            setup_global_shortcuts(app)?;

            // Start transcription worker for any pending jobs
            let queue = app.state::<TranscriptionQueueManager>();
            queue.start_worker(app.handle().clone());

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let queue = window.state::<TranscriptionQueueManager>();
                if queue.has_active_jobs() {
                    api.prevent_close();
                    let _ = window.emit("transcription-close-warning", ());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            start_recording,
            stop_recording,
            is_recording,
            list_audio_devices,
            check_screen_permission,
            request_screen_permission,
            open_screen_settings,
            get_settings,
            save_settings,
            get_default_output_dir,
            list_presets,
            cmd_create_preset,
            cmd_update_preset,
            cmd_delete_preset,
            cmd_duplicate_preset,
            cmd_valid_encoder_presets,
            cmd_reset_presets,
            get_history,
            delete_recording_entry,
            remove_from_history,
            rescan_history,
            remove_missing_entries,
            open_file,
            read_text_file,
            reveal_in_folder,
            run_cleanup,
            is_whisper_model_available,
            download_whisper_model,
            get_transcription_queue,
            start_transcription,
            retry_transcription,
            cancel_transcription,
            force_close_app,
            open_camera_overlay,
            close_camera_overlay,
            save_camera_position,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
