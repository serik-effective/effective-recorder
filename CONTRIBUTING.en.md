# Development

> [Русская версия](CONTRIBUTING.md)

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Framework | Tauri 2 |
| Frontend | SvelteKit 2 + Svelte 5 |
| Backend | Rust |
| Video | FFmpeg (statically linked via ffmpeg-next) |
| Screen capture | xcap + CoreGraphics (macOS) |
| Audio capture | cpal |
| Transcription | whisper-rs (Whisper.cpp) |

## Quick Start

```bash
npm install
npm run tauri dev          # development (JS hot-reload)
npm run tauri build --debug  # build .app bundle (for permission testing)
```

> **Important:** `tauri dev` runs a raw binary — macOS won't show permission dialogs (camera, screen). For permission testing, use `tauri build --debug`.

## Project Structure

```
src/                              # Frontend (SvelteKit + Svelte 5)
  routes/
    +page.svelte                  # Main UI
    camera-overlay/+page.svelte   # Camera overlay window

src-tauri/                        # Backend (Rust + Tauri 2)
  src/
    lib.rs                        # Entry point, Tauri commands
    recorder.rs                   # Recording orchestration (threads)
    encoder.rs                    # FFmpeg encoding, quality presets
    audio.rs                      # Audio capture (cpal)
    screen.rs                     # Screen capture (xcap / CoreGraphics)
    transcription.rs              # Whisper speech-to-text
    transcription_queue.rs        # Background transcription queue
    history.rs                    # Recording history
    settings.rs                   # App settings persistence
    permissions.rs                # macOS permission checks
  capabilities/default.json       # Tauri window permissions
  Info.plist                      # macOS usage descriptions
  Entitlements.plist              # Camera + microphone entitlements
  tauri.conf.json                 # App configuration
  tauri.conf.qa.json              # QA bundle configuration
```

## Architecture

### Recording Pipeline

Three threads are spawned on recording start:
1. **Screen capture** — on macOS via CGDisplayCreateImage (~7ms/frame), on other platforms via xcap
2. **Audio capture** — via cpal from selected input device
3. **FFmpeg encoder** — encodes frames + audio into H.264 MP4

Threads are connected via bounded crossbeam channels (30 for video, 200 for audio).

### Transcription

After recording stops, the file is queued for background transcription via Whisper.cpp. The queue is persistent — survives app restarts. Supports retry and cancel.

### Camera Overlay

A separate Tauri window (`camera-overlay`) with `always_on_top`, showing the webcam feed. The overlay is captured as part of the screen recording — no separate video stream compositing needed.

## Bundle IDs

| Environment | Identifier | Purpose |
|-------------|-----------|---------|
| Dev | `com.effective-recorder.dev` | Daily development |
| QA | `com.effective-recorder.qa` | Clean testing |
| Release | `com.effective-recorder.app` | Production |

## Reset macOS Permissions

```bash
tccutil reset Camera com.effective-recorder.dev
tccutil reset Microphone com.effective-recorder.dev
tccutil reset ScreenCapture com.effective-recorder.dev
```

## Static FFmpeg Build

For a standalone bundle with no external dependencies:

```bash
bash build-ffmpeg-static.sh
export FFMPEG_DIR=$PWD/ffmpeg-build/install
export FFMPEG_STATIC=1
export PKG_CONFIG_PATH=$FFMPEG_DIR/lib/pkgconfig
npm run tauri build -- --debug
```

## CI/CD

GitHub Actions builds bundles for 4 platforms:
- macOS ARM64 (Apple Silicon)
- macOS x64 (Intel)
- Windows x64
- Linux x64

Builds are triggered manually or on `v*` tag pushes.
