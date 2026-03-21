use anyhow::{Context, Result};
use crossbeam_channel::Receiver;
use ffmpeg_next as ffmpeg;
use ffmpeg_next::codec::Id;
use ffmpeg_next::util::frame::audio::Audio as FfmpegAudioFrame;
use ffmpeg_next::util::frame::video::Video as FfmpegVideoFrame;
use ffmpeg_next::{Dictionary, Rational};
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::audio::AudioSamples;
use crate::screen;

// ── Preset system ──────────────────────────────────────────────────

const VALID_X264_PRESETS: &[&str] = &[
    "ultrafast", "superfast", "veryfast", "faster", "fast",
    "medium", "slow", "slower", "veryslow",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityPreset {
    pub id: String,
    pub name: String,
    pub description: String,
    pub target_height: u32,
    pub fps: u32,
    pub crf: u32,
    pub preset: String, // x264 preset
    pub audio_bitrate: usize,
    pub audio_channels: u32,
    #[serde(default)]
    pub is_system: bool,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub auto_transcribe: bool,
    #[serde(default)]
    pub camera_overlay_enabled: bool,
    #[serde(default)]
    pub camera_device_id: Option<String>,
    #[serde(default = "default_camera_size")]
    pub camera_overlay_size: String,
    #[serde(default = "default_camera_shape")]
    pub camera_overlay_shape: String,
}

fn default_camera_size() -> String { "medium".into() }
fn default_camera_shape() -> String { "circle".into() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresetsFile {
    pub presets: Vec<QualityPreset>,
}

impl QualityPreset {
    pub fn output_dimensions(&self, logical_w: u32, logical_h: u32) -> (u32, u32) {
        let aspect = logical_w as f64 / logical_h as f64;
        let out_h = self.target_height.min(logical_h);
        let out_w = (out_h as f64 * aspect).round() as u32;
        (out_w & !1, out_h & !1)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty() { return Err("ID is required".into()); }
        if self.name.is_empty() { return Err("Name is required".into()); }
        if self.fps < 5 || self.fps > 24 { return Err("FPS must be 5-24".into()); }
        if self.target_height < 240 || self.target_height > 1080 {
            return Err("Height must be 240-1080".into());
        }
        if self.crf > 51 { return Err("CRF must be 0-51".into()); }
        if self.audio_bitrate < 16_000 || self.audio_bitrate > 320_000 {
            return Err("Audio bitrate must be 16000-320000".into());
        }
        if self.audio_channels != 1 && self.audio_channels != 2 {
            return Err("Audio channels must be 1 or 2".into());
        }
        if !VALID_X264_PRESETS.contains(&self.preset.as_str()) {
            return Err(format!("Invalid encoder preset '{}'. Valid: {:?}", self.preset, VALID_X264_PRESETS));
        }
        Ok(())
    }
}

fn default_presets() -> Vec<QualityPreset> {
    let now = chrono::Utc::now().to_rfc3339();
    vec![
        QualityPreset {
            id: "voice".into(), name: "Voice-first".into(),
            description: "~60-90 MB/h".into(), target_height: 360, fps: 5,
            crf: 38, preset: "veryfast".into(), audio_bitrate: 32_000,
            audio_channels: 1, is_system: true,
            created_at: Some(now.clone()), updated_at: Some(now.clone()),
            auto_transcribe: false,
            camera_overlay_enabled: false,
            camera_device_id: None,
            camera_overlay_size: default_camera_size(),
            camera_overlay_shape: default_camera_shape(),
        },
        QualityPreset {
            id: "daily".into(), name: "Daily Lite".into(),
            description: "~140-190 MB/h".into(), target_height: 720, fps: 8,
            crf: 32, preset: "veryfast".into(), audio_bitrate: 48_000,
            audio_channels: 1, is_system: true,
            created_at: Some(now.clone()), updated_at: Some(now.clone()),
            auto_transcribe: false,
            camera_overlay_enabled: false,
            camera_device_id: None,
            camera_overlay_size: default_camera_size(),
            camera_overlay_shape: default_camera_shape(),
        },
        QualityPreset {
            id: "meeting".into(), name: "Meeting".into(),
            description: "~220-300 MB/h".into(), target_height: 720, fps: 12,
            crf: 29, preset: "fast".into(), audio_bitrate: 64_000,
            audio_channels: 1, is_system: true,
            created_at: Some(now.clone()), updated_at: Some(now.clone()),
            auto_transcribe: false,
            camera_overlay_enabled: false,
            camera_device_id: None,
            camera_overlay_size: default_camera_size(),
            camera_overlay_shape: default_camera_shape(),
        },
        QualityPreset {
            id: "presentation".into(), name: "Presentation".into(),
            description: "~260-380 MB/h".into(), target_height: 900, fps: 10,
            crf: 26, preset: "medium".into(), audio_bitrate: 64_000,
            audio_channels: 1, is_system: true,
            created_at: Some(now.clone()), updated_at: Some(now.clone()),
            auto_transcribe: false,
            camera_overlay_enabled: false,
            camera_device_id: None,
            camera_overlay_size: default_camera_size(),
            camera_overlay_shape: default_camera_shape(),
        },
        QualityPreset {
            id: "loom".into(), name: "Loom HD".into(),
            description: "~500-800 MB/h".into(), target_height: 1080, fps: 24,
            crf: 23, preset: "veryfast".into(), audio_bitrate: 128_000,
            audio_channels: 2, is_system: true,
            created_at: Some(now.clone()), updated_at: Some(now),
            auto_transcribe: true,
            camera_overlay_enabled: true,
            camera_device_id: None,
            camera_overlay_size: "large".into(),
            camera_overlay_shape: "circle".into(),
        },
    ]
}

fn presets_path() -> std::path::PathBuf {
    crate::settings::SettingsManager::config_dir().join("presets.json")
}

pub fn load_presets() -> Vec<QualityPreset> {
    let path = presets_path();
    match std::fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str::<PresetsFile>(&content) {
            Ok(f) if !f.presets.is_empty() => f.presets,
            _ => {
                let defaults = default_presets();
                save_presets_file(&defaults);
                defaults
            }
        },
        Err(_) => {
            info!("No presets file, creating defaults");
            let defaults = default_presets();
            save_presets_file(&defaults);
            defaults
        }
    }
}

pub fn save_presets_file(presets: &[QualityPreset]) {
    let path = presets_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = PresetsFile { presets: presets.to_vec() };
    if let Ok(json) = serde_json::to_string_pretty(&file) {
        let _ = std::fs::write(&path, json);
    }
}

pub fn find_preset(id: &str) -> QualityPreset {
    let presets = load_presets();
    presets.iter().find(|p| p.id == id).cloned()
        .unwrap_or_else(|| presets.into_iter().next()
            .unwrap_or_else(|| default_presets()[1].clone()))
}

pub fn create_preset(mut preset: QualityPreset) -> Result<QualityPreset, String> {
    preset.validate()?;
    let mut presets = load_presets();
    if presets.iter().any(|p| p.id == preset.id) {
        return Err(format!("Preset with id '{}' already exists", preset.id));
    }
    let now = chrono::Utc::now().to_rfc3339();
    preset.is_system = false;
    preset.created_at = Some(now.clone());
    preset.updated_at = Some(now);
    presets.push(preset.clone());
    save_presets_file(&presets);
    Ok(preset)
}

pub fn update_preset(mut preset: QualityPreset) -> Result<QualityPreset, String> {
    preset.validate()?;
    let mut presets = load_presets();
    let idx = presets.iter().position(|p| p.id == preset.id)
        .ok_or_else(|| format!("Preset '{}' not found", preset.id))?;
    preset.is_system = presets[idx].is_system;
    preset.created_at = presets[idx].created_at.clone();
    preset.updated_at = Some(chrono::Utc::now().to_rfc3339());
    presets[idx] = preset.clone();
    save_presets_file(&presets);
    Ok(preset)
}

pub fn delete_preset(id: &str) -> Result<(), String> {
    let mut presets = load_presets();
    let idx = presets.iter().position(|p| p.id == id)
        .ok_or_else(|| format!("Preset '{}' not found", id))?;
    if presets[idx].is_system {
        return Err("Cannot delete system preset. Duplicate it first.".into());
    }
    if presets.len() <= 1 {
        return Err("Cannot delete the last preset".into());
    }
    presets.remove(idx);
    save_presets_file(&presets);
    Ok(())
}

pub fn duplicate_preset(id: &str) -> Result<QualityPreset, String> {
    let source = load_presets().iter().find(|p| p.id == id).cloned()
        .ok_or_else(|| format!("Preset '{}' not found", id))?;
    let now = chrono::Utc::now().to_rfc3339();
    let new_id = format!("{}-copy-{}", source.id, &now[17..19]);
    let mut copy = source.clone();
    copy.id = new_id;
    copy.name = format!("{} (copy)", source.name);
    copy.is_system = false;
    copy.created_at = Some(now.clone());
    copy.updated_at = Some(now);
    create_preset(copy)
}

pub fn reset_presets_to_defaults() -> Vec<QualityPreset> {
    let defaults = default_presets();
    save_presets_file(&defaults);
    defaults
}

pub fn get_valid_encoder_presets() -> Vec<String> {
    VALID_X264_PRESETS.iter().map(|s| s.to_string()).collect()
}

// ── Encoder ────────────────────────────────────────────────────────

pub struct EncoderConfig {
    pub width: u32,
    pub height: u32,
    pub capture_width: u32,
    pub capture_height: u32,
    pub fps: u32,
    pub audio_sample_rate: u32,
    pub audio_channels: u16,
    pub output_path: String,
    pub preset: QualityPreset,
}

pub struct Encoder;

impl Encoder {
    pub fn start(
        config: EncoderConfig,
        is_recording: Arc<AtomicBool>,
        video_rx: Receiver<screen::VideoFrame>,
        audio_rx: Receiver<AudioSamples>,
        file_size: Arc<std::sync::atomic::AtomicU64>,
    ) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            if let Err(e) =
                Self::run_encoding(config, is_recording, video_rx, audio_rx, file_size)
            {
                error!("Encoder error: {:?}", e);
            }
        })
    }

    fn run_encoding(
        config: EncoderConfig,
        is_recording: Arc<AtomicBool>,
        video_rx: Receiver<screen::VideoFrame>,
        audio_rx: Receiver<AudioSamples>,
        file_size: Arc<std::sync::atomic::AtomicU64>,
    ) -> Result<()> {
        ffmpeg::init().context("Failed to initialize FFmpeg")?;

        let mut output = ffmpeg::format::output(&config.output_path)
            .context("Failed to create output context")?;

        let global_header = output
            .format()
            .flags()
            .contains(ffmpeg::format::Flags::GLOBAL_HEADER);

        let out_width = config.width;
        let out_height = config.height;
        let cap_width = config.capture_width;
        let cap_height = config.capture_height;
        let preset = &config.preset;

        info!(
            "Encoder: [{}] capture={}x{} → output={}x{} @ {} FPS, CRF={}, preset={}",
            preset.name, cap_width, cap_height, out_width, out_height,
            config.fps, preset.crf, preset.preset
        );

        // === VIDEO ENCODER ===
        let video_codec = ffmpeg::encoder::find_by_name("libx264")
            .or_else(|| ffmpeg::encoder::find(Id::H264))
            .context("H264 codec not found")?;

        let mut video_stream = output
            .add_stream(video_codec)
            .context("Failed to add video stream")?;
        let video_stream_index = video_stream.index();

        let video_ctx = ffmpeg::codec::context::Context::new_with_codec(video_codec);
        let mut video_enc = video_ctx.encoder().video()?;

        // Use MPEG standard time_base (1/90000) for high-resolution PTS without integer truncation
        let video_encoder_tb = Rational(1, 90000);

        video_enc.set_width(out_width);
        video_enc.set_height(out_height);
        video_enc.set_format(ffmpeg::format::Pixel::YUV420P);
        video_enc.set_time_base(video_encoder_tb);
        video_enc.set_frame_rate(Some(Rational(config.fps as i32, 1)));
        video_enc.set_gop(config.fps * 2); // keyframe every 2 seconds
        video_enc.set_max_b_frames(3);     // B-frames for better compression

        if global_header {
            unsafe {
                (*video_enc.as_mut_ptr()).flags |=
                    ffmpeg::codec::flag::Flags::GLOBAL_HEADER.bits() as i32;
            }
        }

        let mut video_opts = Dictionary::new();
        video_opts.set("preset", &preset.preset);
        video_opts.set("crf", &preset.crf.to_string());
        video_opts.set("tune", "zerolatency");

        let mut video_encoder = video_enc
            .open_as_with(video_codec, video_opts)
            .context("Failed to open video encoder")?;

        video_stream.set_parameters(&video_encoder);

        // === AUDIO ENCODER ===
        let audio_codec = ffmpeg::encoder::find(Id::AAC).context("AAC codec not found")?;

        let mut audio_stream = output
            .add_stream(audio_codec)
            .context("Failed to add audio stream")?;
        let audio_stream_index = audio_stream.index();

        let audio_ctx = ffmpeg::codec::context::Context::new_with_codec(audio_codec);
        let mut audio_enc = audio_ctx.encoder().audio()?;

        let encoder_audio_channels = preset.audio_channels as i32;
        let encoder_channel_layout =
            ffmpeg::channel_layout::ChannelLayout::default(encoder_audio_channels);

        let audio_encoder_tb = Rational(1, config.audio_sample_rate as i32);

        audio_enc.set_rate(config.audio_sample_rate as i32);
        audio_enc.set_channel_layout(encoder_channel_layout);
        audio_enc.set_format(ffmpeg::format::Sample::F32(
            ffmpeg::format::sample::Type::Planar,
        ));
        audio_enc.set_time_base(audio_encoder_tb);
        audio_enc.set_bit_rate(preset.audio_bitrate);

        if global_header {
            unsafe {
                (*audio_enc.as_mut_ptr()).flags |=
                    ffmpeg::codec::flag::Flags::GLOBAL_HEADER.bits() as i32;
            }
        }

        let mut audio_encoder = audio_enc
            .open_as(audio_codec)
            .context("Failed to open audio encoder")?;

        audio_stream.set_parameters(&audio_encoder);

        output
            .write_header()
            .context("Failed to write output header")?;

        let video_stream_tb = output.stream(video_stream_index).unwrap().time_base();
        let audio_stream_tb = output.stream(audio_stream_index).unwrap().time_base();
        let actual_video_enc_tb = video_encoder.time_base();
        let actual_audio_enc_tb = audio_encoder.time_base();

        let encoder_format = audio_encoder.format();
        let encoder_rate = audio_encoder.rate();
        let frame_size = {
            let fs = audio_encoder.frame_size() as usize;
            if fs == 0 { 1024 } else { fs }
        };

        info!(
            "Audio: input {} Hz {} ch → encoder {} ch, {}kbps, frame_size={}",
            config.audio_sample_rate, config.audio_channels,
            encoder_audio_channels, preset.audio_bitrate / 1000, frame_size
        );

        // SWS: macOS captures BGRA, Windows/Linux capture RGBA → YUV420P
        let sws_flags = ffmpeg::software::scaling::Flags::BICUBIC;
        let input_pixel_format = if cfg!(target_os = "macos") {
            ffmpeg::format::Pixel::BGRA
        } else {
            ffmpeg::format::Pixel::RGBA
        };
        let mut sws_context = ffmpeg::software::scaling::Context::get(
            input_pixel_format,
            cap_width,
            cap_height,
            ffmpeg::format::Pixel::YUV420P,
            out_width,
            out_height,
            sws_flags,
        )
        .context("Failed to create SWS context")?;

        // Audio resampler (input channels → preset channels, packed → planar)
        let input_channel_layout =
            ffmpeg::channel_layout::ChannelLayout::default(config.audio_channels as i32);

        let mut audio_resampler = ffmpeg::software::resampling::Context::get(
            ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Packed),
            input_channel_layout,
            config.audio_sample_rate,
            encoder_format,
            encoder_channel_layout,
            encoder_rate,
        )
        .context("Failed to create audio resampler")?;

        let mut audio_pts: i64 = 0;
        let mut audio_buffer: Vec<f32> = Vec::new();
        let input_channels = config.audio_channels as usize;

        let timeout = Duration::from_millis(10);
        let mut video_frame_count: u64 = 0;
        let mut audio_frame_count: u64 = 0;

        loop {
            let still_recording = is_recording.load(Ordering::Relaxed);

            // === VIDEO ===
            match video_rx.recv_timeout(timeout) {
                Ok(frame) => {
                    let t_copy = std::time::Instant::now();
                    let mut src_frame =
                        FfmpegVideoFrame::new(input_pixel_format, cap_width, cap_height);

                    let src_stride = src_frame.stride(0);
                    let row_bytes = (frame.width * 4) as usize;

                    // Fast path: if stride matches, single memcpy
                    if src_stride == row_bytes && frame.data.len() >= row_bytes * cap_height as usize {
                        let total = row_bytes * cap_height as usize;
                        src_frame.data_mut(0)[..total]
                            .copy_from_slice(&frame.data[..total]);
                    } else {
                        for y in 0..cap_height.min(frame.height) as usize {
                            let src_offset = y * frame.width as usize * 4;
                            let dst_offset = y * src_stride;
                            if src_offset + row_bytes <= frame.data.len()
                                && dst_offset + row_bytes <= src_frame.data_mut(0).len()
                            {
                                src_frame.data_mut(0)[dst_offset..dst_offset + row_bytes]
                                    .copy_from_slice(&frame.data[src_offset..src_offset + row_bytes]);
                            }
                        }
                    }
                    let copy_ms = t_copy.elapsed().as_millis();

                    let t_sws = std::time::Instant::now();
                    let mut dst_frame =
                        FfmpegVideoFrame::new(ffmpeg::format::Pixel::YUV420P, out_width, out_height);
                    sws_context.run(&src_frame, &mut dst_frame)?;
                    let sws_ms = t_sws.elapsed().as_millis();

                    let t_enc = std::time::Instant::now();
                    // Convert real capture timestamp to PTS using actual encoder time_base with rounding
                    let tb_den = actual_video_enc_tb.1 as i64;
                    let pts = (frame.timestamp_us * tb_den + 500_000) / 1_000_000;
                    dst_frame.set_pts(Some(pts));

                    video_encoder.send_frame(&dst_frame)?;

                    let mut pkt = ffmpeg::Packet::empty();
                    while video_encoder.receive_packet(&mut pkt).is_ok() {
                        pkt.set_stream(video_stream_index);
                        pkt.rescale_ts(actual_video_enc_tb, video_stream_tb);
                        let _ = pkt.write_interleaved(&mut output);
                    }

                    let enc_ms = t_enc.elapsed().as_millis();

                    video_frame_count += 1;

                    // Log encoder timing every 60 frames
                    if video_frame_count % 60 == 0 {
                        info!("ENCODER DIAG: copy={}ms sws={}ms encode={}ms (frame {})",
                            copy_ms, sws_ms, enc_ms, video_frame_count);
                    }
                    if video_frame_count % 30 == 0 {
                        if let Ok(meta) = std::fs::metadata(&config.output_path) {
                            file_size.store(meta.len(), Ordering::Relaxed);
                        }
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) if !still_recording => break,
                Err(_) => {}
            }

            // === AUDIO ===
            loop {
                match audio_rx.try_recv() {
                    Ok(samples) => audio_buffer.extend_from_slice(&samples.data),
                    Err(_) => break,
                }
            }

            let samples_per_frame = frame_size * input_channels;
            while audio_buffer.len() >= samples_per_frame {
                let chunk: Vec<f32> = audio_buffer.drain(..samples_per_frame).collect();

                let mut src_audio = FfmpegAudioFrame::new(
                    ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Packed),
                    frame_size,
                    input_channel_layout,
                );
                src_audio.set_rate(config.audio_sample_rate);
                src_audio.set_samples(frame_size);

                let plane = src_audio.data_mut(0);
                let bytes = f32_slice_as_bytes(&chunk);
                let copy_len = plane.len().min(bytes.len());
                plane[..copy_len].copy_from_slice(&bytes[..copy_len]);

                let mut dst_audio = FfmpegAudioFrame::new(
                    encoder_format,
                    frame_size,
                    encoder_channel_layout,
                );
                dst_audio.set_rate(encoder_rate);

                match audio_resampler.run(&src_audio, &mut dst_audio) {
                    Ok(_) => {
                        if dst_audio.samples() == 0 {
                            continue;
                        }
                        dst_audio.set_pts(Some(audio_pts));
                        audio_pts += dst_audio.samples() as i64;

                        if let Err(e) = audio_encoder.send_frame(&dst_audio) {
                            warn!("Audio send_frame: {}", e);
                            continue;
                        }

                        let mut pkt = ffmpeg::Packet::empty();
                        while audio_encoder.receive_packet(&mut pkt).is_ok() {
                            pkt.set_stream(audio_stream_index);
                            pkt.rescale_ts(actual_audio_enc_tb, audio_stream_tb);
                            let _ = pkt.write_interleaved(&mut output);
                        }
                        audio_frame_count += 1;
                    }
                    Err(e) => warn!("Resample: {}", e),
                }
            }

            if !still_recording && video_rx.is_empty() && audio_rx.is_empty() {
                break;
            }
        }

        info!("Flushing: {} video, {} audio frames", video_frame_count, audio_frame_count);

        // Flush video
        video_encoder.send_eof()?;
        {
            let mut pkt = ffmpeg::Packet::empty();
            while video_encoder.receive_packet(&mut pkt).is_ok() {
                pkt.set_stream(video_stream_index);
                pkt.rescale_ts(actual_video_enc_tb, video_stream_tb);
                let _ = pkt.write_interleaved(&mut output);
            }
        }

        // Flush audio resampler
        let mut flush_frame = FfmpegAudioFrame::new(encoder_format, frame_size, encoder_channel_layout);
        flush_frame.set_rate(encoder_rate);
        if audio_resampler.flush(&mut flush_frame).is_ok() && flush_frame.samples() > 0 {
            flush_frame.set_pts(Some(audio_pts));
            let _ = audio_encoder.send_frame(&flush_frame);
        }

        // Flush audio encoder
        audio_encoder.send_eof()?;
        {
            let mut pkt = ffmpeg::Packet::empty();
            while audio_encoder.receive_packet(&mut pkt).is_ok() {
                pkt.set_stream(audio_stream_index);
                pkt.rescale_ts(actual_audio_enc_tb, audio_stream_tb);
                let _ = pkt.write_interleaved(&mut output);
            }
        }

        output.write_trailer().context("Failed to write trailer")?;

        if let Ok(meta) = std::fs::metadata(&config.output_path) {
            let size = meta.len();
            file_size.store(size, Ordering::Relaxed);
            info!("Done: {} ({:.1} MB)", config.output_path, size as f64 / 1_048_576.0);
        }

        Ok(())
    }
}

fn f32_slice_as_bytes(data: &[f32]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 4) }
}
