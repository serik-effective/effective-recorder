use log::info;

/// Check if screen recording ACTUALLY works by doing a test capture
/// and verifying the result contains real content (not just our own window).
/// This is more reliable than CGPreflightScreenCaptureAccess() which
/// returns stale results after app rebuilds.
#[cfg(target_os = "macos")]
pub fn has_screen_recording_permission() -> bool {
    // First try the API check
    extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
    }
    let api_result = unsafe { CGPreflightScreenCaptureAccess() };
    info!("CGPreflightScreenCaptureAccess: {}", api_result);

    if api_result {
        return true;
    }

    // API said no — but it might be wrong after rebuild.
    // Try actual capture to verify.
    match xcap::Monitor::all() {
        Ok(monitors) => {
            if let Some(monitor) = monitors.into_iter().find(|m| m.is_primary().unwrap_or(false)) {
                match monitor.capture_image() {
                    Ok(img) => {
                        // Check if captured image has variety (not all same color)
                        // If no permission, macOS returns a blank/solid image
                        let data = img.as_raw();
                        if data.len() < 100 {
                            info!("Capture too small — no permission");
                            return false;
                        }

                        // Sample pixels at different positions to check variety
                        let w = img.width() as usize;
                        let h = img.height() as usize;
                        let stride = w * 4;
                        let mut unique_colors = std::collections::HashSet::new();

                        for &(x_frac, y_frac) in &[
                            (0.1, 0.1), (0.5, 0.5), (0.9, 0.9),
                            (0.1, 0.9), (0.9, 0.1), (0.3, 0.7),
                            (0.7, 0.3), (0.5, 0.1), (0.5, 0.9),
                        ] {
                            let x = (w as f64 * x_frac) as usize;
                            let y = (h as f64 * y_frac) as usize;
                            let idx = y * stride + x * 4;
                            if idx + 3 < data.len() {
                                let r = data[idx];
                                let g = data[idx + 1];
                                let b = data[idx + 2];
                                // Quantize to reduce noise
                                unique_colors.insert((r / 16, g / 16, b / 16));
                            }
                        }

                        let has_variety = unique_colors.len() >= 3;
                        info!(
                            "Test capture: {}x{}, {} unique colors → permission={}",
                            w, h, unique_colors.len(), has_variety
                        );
                        has_variety
                    }
                    Err(e) => {
                        info!("Test capture failed: {} — no permission", e);
                        false
                    }
                }
            } else {
                info!("No primary monitor found");
                false
            }
        }
        Err(e) => {
            info!("Monitor enumeration failed: {}", e);
            false
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn has_screen_recording_permission() -> bool {
    true
}

/// Request screen recording permission on macOS.
#[cfg(target_os = "macos")]
pub fn request_screen_recording_permission() {
    extern "C" {
        fn CGRequestScreenCaptureAccess() -> bool;
    }
    let result = unsafe { CGRequestScreenCaptureAccess() };
    info!("CGRequestScreenCaptureAccess: {}", result);
}

#[cfg(not(target_os = "macos"))]
pub fn request_screen_recording_permission() {}

/// Open System Settings to Screen Recording pane.
#[cfg(target_os = "macos")]
pub fn open_screen_recording_settings() {
    let _ = std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")
        .spawn();
}

#[cfg(not(target_os = "macos"))]
pub fn open_screen_recording_settings() {}
