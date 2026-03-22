use anyhow::{Context, Result};
use crossbeam_channel::Sender;
use log::{error, info};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use xcap::Monitor;

// ── macOS: fast capture via CoreGraphics ─────────────────────────────
#[cfg(target_os = "macos")]
#[allow(deprecated)]
use objc2_core_graphics::{
    CGDataProviderCopyData, CGDirectDisplayID, CGDisplayCreateImage,
    CGImageGetBytesPerRow, CGImageGetDataProvider, CGImageGetHeight, CGImageGetWidth,
};

/// Fast capture using CGDisplayCreateImage (direct display capture, no window compositing).
/// Returns BGRA data — no RGBA conversion needed since encoder accepts BGRA.
#[cfg(target_os = "macos")]
#[allow(deprecated)]
fn capture_display_fast(display_id: CGDirectDisplayID) -> Result<(Vec<u8>, u32, u32)> {
    let cg_image = CGDisplayCreateImage(display_id)
        .ok_or_else(|| anyhow::anyhow!("CGDisplayCreateImage failed"))?;

    let width = CGImageGetWidth(Some(&cg_image));
    let height = CGImageGetHeight(Some(&cg_image));
    let data_provider = CGImageGetDataProvider(Some(&cg_image));
    let data = CGDataProviderCopyData(data_provider.as_deref())
        .ok_or_else(|| anyhow::anyhow!("Failed to copy display data"))?
        .to_vec();
    let bytes_per_row = CGImageGetBytesPerRow(Some(&cg_image));

    let row_bytes = width * 4;
    if bytes_per_row == row_bytes {
        Ok((data, width as u32, height as u32))
    } else {
        let mut buffer = Vec::with_capacity(width * height * 4);
        for row in data.chunks_exact(bytes_per_row) {
            buffer.extend_from_slice(&row[..row_bytes]);
        }
        Ok((buffer, width as u32, height as u32))
    }
}

/// Windows/Linux fallback: capture via xcap (returns RGBA).
#[cfg(not(target_os = "macos"))]
fn capture_xcap(monitor: &Monitor) -> Result<(Vec<u8>, u32, u32)> {
    let image = monitor.capture_image().context("xcap capture failed")?;
    let width = image.width();
    let height = image.height();
    let data = image.into_raw();
    Ok((data, width, height))
}

/// Wrapper to send Monitor across threads on Windows/Linux.
/// Safety: Monitor handle (HMONITOR) is valid for the lifetime of the recording
/// and is only used for capture (read-only) from a single thread.
#[cfg(not(target_os = "macos"))]
struct SendMonitor(Monitor);
#[cfg(not(target_os = "macos"))]
unsafe impl Send for SendMonitor {}

#[allow(dead_code)]
pub struct VideoFrame {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub timestamp_us: i64,
}

pub struct ScreenCapture {
    monitor: Monitor,
    fps: u32,
    /// Logical dimensions (what user sees, e.g. 1496x967)
    logical_width: u32,
    logical_height: u32,
    /// Real pixel dimensions (Retina, e.g. 2992x1934)
    capture_width: u32,
    capture_height: u32,
}

impl ScreenCapture {
    pub fn new(fps: u32) -> Result<Self> {
        let monitors = Monitor::all().context("Failed to enumerate monitors")?;
        let monitor = monitors
            .into_iter()
            .find(|m| m.is_primary().unwrap_or(false))
            .or_else(|| Monitor::all().ok()?.into_iter().next())
            .context("No monitors found")?;

        let logical_width = monitor.width().unwrap_or(1920);
        let logical_height = monitor.height().unwrap_or(1080);

        // Capture one frame to get real pixel dimensions (important for Retina/HiDPI)
        let test_image = monitor
            .capture_image()
            .context("Failed to capture test frame")?;
        let capture_width = test_image.width();
        let capture_height = test_image.height();

        let scale = if logical_width > 0 {
            capture_width / logical_width
        } else {
            1
        };

        info!(
            "Monitor: logical={}x{}, capture={}x{} (scale={}x)",
            logical_width, logical_height, capture_width, capture_height, scale
        );

        Ok(Self {
            monitor,
            fps,
            logical_width,
            logical_height,
            capture_width,
            capture_height,
        })
    }

    /// Returns the logical (output) width — this is what the encoder uses
    pub fn width(&self) -> u32 {
        self.logical_width
    }

    /// Returns the logical (output) height — this is what the encoder uses
    pub fn height(&self) -> u32 {
        self.logical_height
    }

    /// Returns the actual captured pixel width (may be 2x on Retina)
    pub fn capture_width(&self) -> u32 {
        self.capture_width
    }

    /// Returns the actual captured pixel height (may be 2x on Retina)
    pub fn capture_height(&self) -> u32 {
        self.capture_height
    }

    /// Returns true if running on macOS (BGRA output) vs Windows/Linux (RGBA output).
    #[allow(dead_code)]
    pub fn is_bgra_output(&self) -> bool {
        cfg!(target_os = "macos")
    }

    /// Start capture loop. macOS uses CGDisplayCreateImage (fast, BGRA), others use xcap (RGBA).
    pub fn start(
        &self,
        is_recording: Arc<AtomicBool>,
        sender: Sender<VideoFrame>,
        start_time: Instant,
    ) -> thread::JoinHandle<()> {
        let fps = self.fps;

        #[cfg(target_os = "macos")]
        let display_id = self.monitor.id().unwrap_or(0);
        #[cfg(not(target_os = "macos"))]
        let monitor = SendMonitor(self.monitor.clone());

        thread::spawn(move || {
            let frame_duration = Duration::from_nanos(1_000_000_000 / fps as u64);
            let mut frame_count: u64 = 0;
            let mut last_log = Instant::now();
            let mut capture_time_total = Duration::ZERO;
            let mut send_time_total = Duration::ZERO;
            let mut sleep_time_total = Duration::ZERO;

            #[cfg(target_os = "macos")]
            info!("Fast display capture started (CGDisplayCreateImage, target {} FPS)", fps);
            #[cfg(not(target_os = "macos"))]
            info!("Screen capture started (xcap, target {} FPS)", fps);

            while is_recording.load(Ordering::Relaxed) {
                let loop_start = Instant::now();

                let t0 = Instant::now();

                #[cfg(target_os = "macos")]
                let capture_result = capture_display_fast(display_id);
                #[cfg(not(target_os = "macos"))]
                let capture_result = capture_xcap(&monitor.0);

                match capture_result {
                    Ok((data, width, height)) => {
                        let capture_dur = t0.elapsed();
                        capture_time_total += capture_dur;

                        let timestamp_us = start_time.elapsed().as_micros() as i64;

                        let frame = VideoFrame {
                            data,
                            width,
                            height,
                            timestamp_us,
                        };

                        let t_send = Instant::now();
                        if sender.send(frame).is_err() {
                            break;
                        }
                        send_time_total += t_send.elapsed();

                        frame_count += 1;

                        if last_log.elapsed() >= Duration::from_secs(2) {
                            let total_elapsed = start_time.elapsed().as_secs_f64();
                            info!(
                                "CAPTURE DIAG: {:.1} fps | capture={:.0}ms send={:.0}ms sleep={:.0}ms ({}x{}, {:.1}s)",
                                frame_count as f64 / total_elapsed,
                                capture_time_total.as_millis() as f64 / frame_count as f64,
                                send_time_total.as_millis() as f64 / frame_count as f64,
                                sleep_time_total.as_millis() as f64 / frame_count as f64,
                                width, height,
                                total_elapsed
                            );
                            last_log = Instant::now();
                        }
                    }
                    Err(e) => {
                        error!("Capture failed: {}", e);
                    }
                }

                // Precise frame pacing: sleep for bulk of wait, then spin-wait for accuracy
                let remaining = frame_duration.saturating_sub(loop_start.elapsed());
                if remaining > Duration::from_millis(2) {
                    let t_sleep = Instant::now();
                    thread::sleep(remaining - Duration::from_millis(2));
                    sleep_time_total += t_sleep.elapsed();
                }
                // Spin-wait for the last ~2ms for precise timing
                while loop_start.elapsed() < frame_duration {
                    std::hint::spin_loop();
                }
            }

            info!("Capture stopped. Total frames: {}", frame_count);
        })
    }
}
