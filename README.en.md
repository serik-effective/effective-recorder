# Effective Recorder

> [Русская версия](README.md)

Screen recording with camera overlay and automatic transcription. A local-first Loom alternative.

## Features

- 🖥 **Screen recording** — full screen with system audio
- 📹 **Camera overlay** — Loom-style circle/rounded overlay on top of recording
- 🎙 **Transcription** — automatic speech recognition (Whisper AI, runs locally)
- ⚙️ **Quality presets** — from minimal to HD, with resolution, FPS and compression settings
- ⌨️ **Hotkeys** — Cmd+Shift+R to start/stop
- 📂 **Recording history** — all recordings with metadata and quick access

## Download

Go to [Releases](../../releases) and download:
- **macOS** — `.dmg` file (Apple Silicon and Intel)
- **Windows** — `.msi` installer

> ⚠️ The app is not code-signed yet. On first launch:
> - **macOS**: Right-click → Open → Open
> - **Windows**: "More info" → "Run anyway"

## How to use

1. Launch the app
2. On first launch, grant access to camera, microphone and screen recording
3. Choose a quality preset (or create your own)
4. Click **Start Recording** (or Cmd+Shift+R)
5. Click **Stop** to finish
6. Recording is saved to Movies (macOS) / Videos (Windows) folder

## Presets

| Preset | Resolution | FPS | Use case |
|--------|-----------|-----|----------|
| Voice-first | 360p | 5 | Calls where audio matters most |
| Daily Lite | 720p | 8 | Daily standups |
| Meeting | 720p | 12 | Meeting recordings |
| Presentation | 900p | 10 | Presentations and demos |
| Loom HD | 1080p | 24 | Maximum quality |

You can create custom presets with any settings.

## Transcription

The app can automatically transcribe speech in recordings and generate subtitles (.srt).

- Runs **entirely locally** via Whisper AI — no data sent to the internet
- Downloads the model (~1.5 GB) on first use
- Enable auto-transcription in preset settings
- Or click the **Transcribe** button on any recording in history

## System requirements

- **macOS** 12+ (Apple Silicon or Intel)
- **Windows** 10+ (64-bit)
- ~2 GB free space (including transcription model)

## Feedback

Found a bug or have an idea? Create an [Issue](../../issues).

---

📖 [Developer documentation](CONTRIBUTING.md)
