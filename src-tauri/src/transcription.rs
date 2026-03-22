use anyhow::{Context, Result};
use ffmpeg_next as ffmpeg;
use log::{info, warn, error};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

const MODEL_FILENAME: &str = "ggml-large-v3-turbo.bin";
const MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin";
const MODEL_MIN_SIZE: u64 = 100_000_000; // 100 MB sanity check

// ── Model management ────────────────────────────────────────────────

pub fn models_dir() -> PathBuf {
    crate::settings::SettingsManager::config_dir().join("models")
}

pub fn model_path() -> PathBuf {
    models_dir().join(MODEL_FILENAME)
}

pub fn is_model_available() -> bool {
    let p = model_path();
    p.exists()
        && std::fs::metadata(&p)
            .map(|m| m.len() > MODEL_MIN_SIZE)
            .unwrap_or(false)
}

#[derive(Serialize, Clone)]
pub struct ModelDownloadProgress {
    pub percent: u8,
    pub bytes_downloaded: u64,
    pub total_bytes: u64,
}

pub async fn download_model(app: AppHandle) -> Result<()> {
    use futures_util::StreamExt;

    let dir = models_dir();
    std::fs::create_dir_all(&dir).context("Failed to create models directory")?;

    let dest = model_path();
    let partial = dest.with_extension("bin.downloading");

    info!("Downloading Whisper model from {}", MODEL_URL);

    let client = reqwest::Client::new();
    let response = client.get(MODEL_URL).send().await.context("Download request failed")?;

    let total = response.content_length().unwrap_or(0);
    let mut downloaded: u64 = 0;
    let mut last_percent: u8 = 0;

    let mut file = std::fs::File::create(&partial).context("Failed to create download file")?;
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("Failed to read download chunk")?;
        std::io::Write::write_all(&mut file, &chunk)
            .context("Failed to write download chunk")?;

        downloaded += chunk.len() as u64;
        let percent = if total > 0 {
            ((downloaded as f64 / total as f64) * 100.0) as u8
        } else {
            0
        };

        // Emit progress every 1%
        if percent != last_percent {
            last_percent = percent;
            let _ = app.emit(
                "model-download-progress",
                ModelDownloadProgress {
                    percent,
                    bytes_downloaded: downloaded,
                    total_bytes: total,
                },
            );
        }
    }

    drop(file);

    // Verify size
    let size = std::fs::metadata(&partial)
        .map(|m| m.len())
        .unwrap_or(0);

    if size < MODEL_MIN_SIZE {
        let _ = std::fs::remove_file(&partial);
        anyhow::bail!("Downloaded model too small ({} bytes), likely corrupted", size);
    }

    // Atomic rename
    std::fs::rename(&partial, &dest).context("Failed to finalize model file")?;

    info!("Whisper model downloaded: {} ({:.1} MB)", dest.display(), size as f64 / 1_048_576.0);
    Ok(())
}

// ── Audio extraction ────────────────────────────────────────────────

/// Extract audio from video file and convert to 16kHz mono f32 PCM.
pub fn extract_audio_pcm(video_path: &str) -> Result<Vec<f32>> {
    ffmpeg::init().context("Failed to initialize FFmpeg")?;

    let mut input = ffmpeg::format::input(video_path)
        .context("Failed to open video file")?;

    let audio_stream = input
        .streams()
        .best(ffmpeg::media::Type::Audio)
        .context("No audio stream found in video")?;

    let audio_stream_index = audio_stream.index();
    let audio_params = audio_stream.parameters();

    let context = ffmpeg::codec::context::Context::from_parameters(audio_params)
        .context("Failed to create decoder context")?;
    let mut decoder = context.decoder().audio()
        .context("Failed to create audio decoder")?;

    let target_rate = 16000u32;
    let target_channel_layout = ffmpeg::channel_layout::ChannelLayout::MONO;

    // Set up resampler: input format → 16kHz mono f32
    let mut resampler = ffmpeg::software::resampling::Context::get(
        decoder.format(),
        decoder.channel_layout(),
        decoder.rate(),
        ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Packed),
        target_channel_layout,
        target_rate,
    )
    .context("Failed to create audio resampler")?;

    let mut pcm_data: Vec<f32> = Vec::new();

    for (stream, packet) in input.packets() {
        if stream.index() != audio_stream_index {
            continue;
        }

        decoder.send_packet(&packet)?;

        let mut decoded_frame = ffmpeg::frame::Audio::empty();
        while decoder.receive_frame(&mut decoded_frame).is_ok() {
            let mut resampled = ffmpeg::frame::Audio::empty();
            resampler.run(&decoded_frame, &mut resampled)?;

            if resampled.samples() > 0 {
                let data = resampled.data(0);
                let floats: &[f32] = unsafe {
                    std::slice::from_raw_parts(
                        data.as_ptr() as *const f32,
                        resampled.samples(),
                    )
                };
                pcm_data.extend_from_slice(floats);
            }
        }
    }

    // Flush decoder
    decoder.send_eof()?;
    let mut decoded_frame = ffmpeg::frame::Audio::empty();
    while decoder.receive_frame(&mut decoded_frame).is_ok() {
        let mut resampled = ffmpeg::frame::Audio::empty();
        if resampler.run(&decoded_frame, &mut resampled).is_ok() && resampled.samples() > 0 {
            let data = resampled.data(0);
            let floats: &[f32] = unsafe {
                std::slice::from_raw_parts(
                    data.as_ptr() as *const f32,
                    resampled.samples(),
                )
            };
            pcm_data.extend_from_slice(floats);
        }
    }

    // Flush resampler
    let mut flush_frame = ffmpeg::frame::Audio::empty();
    if resampler.flush(&mut flush_frame).is_ok() && flush_frame.samples() > 0 {
        let data = flush_frame.data(0);
        let floats: &[f32] = unsafe {
            std::slice::from_raw_parts(
                data.as_ptr() as *const f32,
                flush_frame.samples(),
            )
        };
        pcm_data.extend_from_slice(floats);
    }

    info!(
        "Extracted {:.1}s of audio ({} samples at {}Hz)",
        pcm_data.len() as f64 / target_rate as f64,
        pcm_data.len(),
        target_rate
    );

    Ok(pcm_data)
}

// ── Whisper inference ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct Segment {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

pub fn transcribe(
    video_path: &str,
    progress_callback: impl Fn(u8) + Send + Sync + 'static,
) -> Result<Vec<Segment>> {
    info!("Starting transcription: {}", video_path);

    let cb = std::sync::Arc::new(progress_callback);

    // Step 1: Extract audio (report 0-30% for extraction)
    cb(0);
    let pcm = extract_audio_pcm(video_path)
        .context("Failed to extract audio from video")?;
    cb(30);

    if pcm.is_empty() {
        anyhow::bail!("No audio data extracted from video");
    }

    // Step 2: Load Whisper model
    let model = model_path();
    let model_str = model.to_str().context("Invalid model path")?;

    // Check model file
    let model_meta = std::fs::metadata(&model);
    match &model_meta {
        Ok(m) => info!("Whisper model file: {} ({:.1} MB)", model_str, m.len() as f64 / 1_048_576.0),
        Err(e) => {
            error!("Whisper model file not accessible: {} - {}", model_str, e);
            anyhow::bail!("Whisper model not accessible: {}", e);
        }
    }

    // Check available memory
    #[cfg(target_os = "macos")]
    {
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64;
        let total_pages = unsafe { libc::sysconf(libc::_SC_PHYS_PAGES) } as u64;
        let total_mem_mb = (page_size * total_pages) / 1_048_576;
        info!("System memory: {} MB total", total_mem_mb);
        if total_mem_mb < 4096 {
            warn!("Low system memory ({} MB) - Whisper turbo model needs ~2GB RAM", total_mem_mb);
        }
    }

    let n_threads = num_cpus();
    info!("Loading Whisper model with {} threads...", n_threads);

    let ctx = match WhisperContext::new_with_params(model_str, WhisperContextParameters::default()) {
        Ok(ctx) => {
            info!("Whisper model loaded successfully");
            ctx
        }
        Err(e) => {
            error!("Failed to load Whisper model: {}", e);
            anyhow::bail!("Failed to load Whisper model: {}", e);
        }
    };

    info!("Creating Whisper state...");
    let mut state = match ctx.create_state() {
        Ok(s) => {
            info!("Whisper state created successfully");
            s
        }
        Err(e) => {
            error!("Failed to create Whisper state: {}", e);
            anyhow::bail!("Failed to create Whisper state: {}", e);
        }
    };

    // Step 3: Configure and run inference (30-95%)
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some("auto"));
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_n_threads(n_threads);

    // NOTE: Not using set_progress_callback_safe due to SIGSEGV crash in
    // whisper-rs 0.13 callback trampoline. Progress is estimated externally
    // by the worker thread instead.

    info!("Running Whisper inference on {} samples ({:.1}s audio), {} threads...",
          pcm.len(), pcm.len() as f64 / 16000.0, n_threads);
    cb(35); // Signal that inference is starting

    match state.full(params, &pcm) {
        Ok(rc) => info!("Whisper inference completed successfully (rc={})", rc),
        Err(e) => {
            error!("Whisper inference FAILED: {}", e);
            anyhow::bail!("Whisper inference failed: {}", e);
        }
    };

    // Step 4: Extract segments
    let n_segments = state
        .full_n_segments()
        .map_err(|e| anyhow::anyhow!("Failed to get segments: {}", e))?;

    let mut segments = Vec::with_capacity(n_segments as usize);
    for i in 0..n_segments {
        let start = state
            .full_get_segment_t0(i)
            .map_err(|e| anyhow::anyhow!("Failed to get segment start: {}", e))?;
        let end = state
            .full_get_segment_t1(i)
            .map_err(|e| anyhow::anyhow!("Failed to get segment end: {}", e))?;
        let text = state
            .full_get_segment_text(i)
            .map_err(|e| anyhow::anyhow!("Failed to get segment text: {}", e))?;

        segments.push(Segment {
            start_ms: start as i64 * 10, // whisper returns centiseconds
            end_ms: end as i64 * 10,
            text: text.trim().to_string(),
        });
    }

    cb(100);
    info!("Transcription complete: {} segments", segments.len());
    Ok(segments)
}

// ── SRT output ──────────────────────────────────────────────────────

pub fn write_srt(segments: &[Segment], path: &Path) -> Result<()> {
    let mut srt = String::new();

    for (i, seg) in segments.iter().enumerate() {
        if seg.text.is_empty() {
            continue;
        }

        let start = format_srt_time(seg.start_ms);
        let end = format_srt_time(seg.end_ms);

        srt.push_str(&format!("{}\n{} --> {}\n{}\n\n", i + 1, start, end, seg.text));
    }

    std::fs::write(path, srt).context("Failed to write SRT file")?;
    info!("SRT written: {} ({} segments)", path.display(), segments.len());
    Ok(())
}

fn format_srt_time(ms: i64) -> String {
    let ms = ms.max(0) as u64;
    let hours = ms / 3_600_000;
    let minutes = (ms % 3_600_000) / 60_000;
    let seconds = (ms % 60_000) / 1_000;
    let millis = ms % 1_000;
    format!("{:02}:{:02}:{:02},{:03}", hours, minutes, seconds, millis)
}

fn num_cpus() -> i32 {
    std::thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(4)
        .min(8) // Cap at 8 threads to leave headroom
}
