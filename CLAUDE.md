# Effective Recorder

macOS screen recorder with camera overlay and auto-transcription.
Stack: Tauri 2 + SvelteKit 2 + Svelte 5 + Rust.

## Quick Start

```bash
npm install
npm run tauri dev          # daily dev (no hot-reload for Rust, but fast JS reload)
npm run tauri build --debug  # when you need .app bundle (for permissions testing)
```

**Important:** `tauri dev` runs a raw binary — macOS won't show permission dialogs (Camera, Screen Recording). For first-time permission setup or testing permission flows, use `tauri build --debug` which creates a proper `.app` bundle at:
```
src-tauri/target/debug/bundle/macos/Effective Recorder.app
```
Launch it with: `open "src-tauri/target/debug/bundle/macos/Effective Recorder.app"`

## 4 Layers of macOS App (NEVER mix)

1. **Tauri capabilities/permissions** — `src-tauri/capabilities/`
2. **macOS TCC permissions** — Camera / Microphone / ScreenCapture (system-level)
3. **App state** — Application Support / Caches / UserDefaults
4. **App identity** — bundle id / signing / entitlements / Info.plist

## Bundle IDs

| Environment | Identifier | Use |
|-------------|-----------|-----|
| **dev** | `com.effective-recorder.dev` | Daily development (default) |
| **qa** | `com.effective-recorder.qa` | Clean pre-prod testing |
| **release** | `com.effective-recorder.app` | Production release |

Change in `src-tauri/tauri.conf.json` → `identifier`.

macOS ties privacy permissions to app identity. Never test "clean first run" on the same bundle id you debug daily.

## Default Dev-Flow

When changing Rust/JS/UI/event handling/recording logic/device code (without touching plist/entitlements/identifier):

- Restart `npm run tauri dev`
- NO `cargo clean`
- NO `tccutil reset`
- NO deleting `~/Library/...`

This is the default flow.

## When to Clean What

### 1. Changed Tauri capabilities (`capabilities/*`)

- Restart app
- Check capability files
- Do NOT touch TCC
- Do NOT `cargo clean`

This is the Tauri layer, not macOS privacy.

### 2. Changed Info.plist (usage descriptions)

- Full relaunch app
- If you want to re-see system prompt: reset only the needed TCC service
- `cargo clean` usually NOT needed

Tauri merges `src-tauri/Info.plist` into the bundle. Apple requires usage descriptions for camera and microphone.

### 3. Changed entitlements / sandbox / signing / bundle id

Heavy artillery:

- Stop app
- Rebuild app bundle
- Often makes sense to delete old built app
- Reset needed TCC permissions
- If necessary, clean app data
- `cargo clean` sometimes justified here

This changes app identity or rights. "System remembers old, but app is already different."

### 4. Changed only local app state (flags, cache, settings)

- Clean app data
- Do NOT touch TCC
- Do NOT `cargo clean`

## TCC Reset Commands

```bash
tccutil reset Camera com.effective-recorder.dev
tccutil reset Microphone com.effective-recorder.dev
tccutil reset ScreenCapture com.effective-recorder.dev
tccutil reset All com.effective-recorder.dev  # full reset
```

Only use when:
- Changed permission flow
- Want to re-test first-run prompt
- One specific privacy permission got stuck

First verify: is it actually TCC, or Tauri capability, or your code logic?

## App Data Paths

```
~/Library/Application Support/com.effective-recorder.dev/
~/Library/Caches/com.effective-recorder.dev/
defaults delete com.effective-recorder.dev
```

For sandboxed builds: `~/Library/Containers/<bundle-id>`

## cargo clean — ONLY when:

- Changed `build.rs` / native dependencies / linker config
- Changed entitlements / signing / bundle config
- Caught clearly stale native build state
- **NOT** for normal Rust/JS/UI changes

`cargo clean` is a build-layer tool, not a permission tool.

## Source of Truth Files

| File | Purpose |
|------|---------|
| `src-tauri/Info.plist` | macOS usage descriptions (camera, mic, screen) |
| `src-tauri/Entitlements.plist` | Camera + microphone entitlements |
| `src-tauri/capabilities/default.json` | Tauri permissions for windows |
| `src-tauri/tauri.conf.json` | App config, bundle id, signing |

Do NOT change these casually. Treat as source of truth.

## Pre-prod Testing

1. Build with bundle id `com.effective-recorder.qa`
2. Use same build path as release
3. Test on clean app identity
4. Check: Privacy & Security → Camera / Microphone / Screen Recording
5. For sandbox violations: Console.app

For local tests Tauri supports ad-hoc signing and `--no-sign`, but for pre-prod use the same signing path as release.

## Project Structure

```
src/                          # Frontend (SvelteKit + Svelte 5)
  routes/
    +page.svelte              # Main recording UI
    camera-overlay/+page.svelte  # Camera overlay window
src-tauri/                    # Rust backend (Tauri 2)
  src/
    lib.rs                    # Entry point, all Tauri commands
    recorder.rs               # Recording orchestration (threads for screen, audio, encoding)
    encoder.rs                # FFmpeg video encoding, quality presets
    audio.rs                  # Audio capture via cpal
    screen.rs                 # Screen capture via xcap
    transcription.rs          # Whisper-rs speech-to-text
    transcription_queue.rs    # Background transcription job queue
    history.rs                # Recording history management
    settings.rs               # App settings, camera position persistence
    permissions.rs            # macOS screen recording permission checks
  capabilities/default.json   # Tauri window permissions
  Info.plist                  # macOS usage descriptions
  Entitlements.plist          # Camera + microphone entitlements
  tauri.conf.json             # App config
```
