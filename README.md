# Effective Recorder

Loom-like screen recorder for macOS with camera overlay and automatic transcription.

## Features

- Screen + system audio recording with configurable quality presets
- Loom-style camera overlay during recording (circle/rounded, draggable)
- Automatic speech-to-text transcription via Whisper
- Recording history with metadata
- Tray icon with quick controls
- Global keyboard shortcuts (Cmd+Shift+R)
- Storage cleanup policies

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Framework | Tauri 2 |
| Frontend | SvelteKit 2 + Svelte 5 |
| Backend | Rust |
| Video encoding | FFmpeg (statically linked via ffmpeg-next) |
| Screen capture | xcap |
| Audio capture | cpal |
| Transcription | whisper-rs (Whisper.cpp bindings) |
| Permissions | tauri-plugin-macos-permissions |

## Requirements

- macOS (Apple Silicon or Intel)
- Rust / Cargo
- Node.js (18+)

## Getting Started

```bash
npm install
npm run tauri dev
```

On first launch, macOS will request permissions for:
- **Camera** — for the camera overlay
- **Microphone** — for audio recording
- **Screen Recording** — for screen capture

## Project Structure

```
src/                              # Frontend (SvelteKit + Svelte 5)
  routes/
    +page.svelte                  # Main recording UI
    camera-overlay/+page.svelte   # Camera overlay window

src-tauri/                        # Rust backend (Tauri 2)
  src/
    lib.rs                        # Entry point, Tauri commands
    recorder.rs                   # Recording orchestration
    encoder.rs                    # FFmpeg encoding, quality presets
    audio.rs                      # Audio capture (cpal)
    screen.rs                     # Screen capture (xcap)
    transcription.rs              # Whisper speech-to-text
    transcription_queue.rs        # Background transcription jobs
    history.rs                    # Recording history
    settings.rs                   # App settings persistence
    permissions.rs                # macOS permission checks
  capabilities/default.json       # Tauri window permissions
  Info.plist                      # macOS usage descriptions
  Entitlements.plist              # Camera + microphone entitlements
  tauri.conf.json                 # App configuration
```

## Architecture

### Recording Pipeline

The recorder spawns three threads:
1. **Screen capture** (xcap) — captures primary monitor frames at configured FPS
2. **Audio capture** (cpal) — captures system audio from selected input device
3. **FFmpeg encoder** — encodes frames + audio into H.264 MP4

### Transcription

After recording stops, the file is queued for background transcription via Whisper.cpp. The queue persists across app restarts and supports retry/cancel.

### Camera Overlay

A separate Tauri window (`camera-overlay`) opens during recording with `always_on_top`, showing the webcam feed. The overlay is captured as part of the screen recording — no separate video stream compositing needed.

## macOS Permissions

The app requires three macOS permissions, managed through:

- **Info.plist** — usage descriptions (why the app needs access)
- **Entitlements.plist** — camera and microphone entitlements
- **TCC** — macOS Transparency, Consent, and Control system

See [CLAUDE.md](CLAUDE.md) for detailed development rules about permission layers.

## Bundle IDs

| Environment | Identifier |
|-------------|-----------|
| Development | `com.effective-recorder.dev` |
| QA / Pre-prod | `com.effective-recorder.qa` |
| Release | `com.effective-recorder.app` |

Configure in `src-tauri/tauri.conf.json` → `identifier`.
