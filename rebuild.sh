#!/bin/bash
# Rebuild: kill → deep clean → build → install → launch
set -e

APP_DIR="$(cd "$(dirname "$0")" && pwd)"
BUNDLE_ID="com.effective-recorder.app"
OLD_BUNDLE_ID="com.screenrecorder.app"
BUILD_APP="$APP_DIR/src-tauri/target/release/bundle/macos/Effective Recorder.app"
INSTALL_DIR="$HOME/Applications"
INSTALL_APP="$INSTALL_DIR/Effective Recorder.app"

echo "🔴 Killing..."
pkill -f "Effective Recorder" 2>/dev/null || true
pkill -f "effective-recorder" 2>/dev/null || true
pkill -f "Screen Recorder" 2>/dev/null || true
pkill -f "screen-recorder" 2>/dev/null || true
pkill -f "vite" 2>/dev/null || true
sleep 1

echo "🧹 Deep clean (removing all traces from macOS)..."
rm -rf "/Applications/Screen Recorder.app" 2>/dev/null || true
rm -rf "/Applications/Effective Recorder.app" 2>/dev/null || true
rm -rf "$INSTALL_DIR/Screen Recorder.app" 2>/dev/null || true
rm -rf "$INSTALL_APP" 2>/dev/null || true
for bid in "$BUNDLE_ID" "$OLD_BUNDLE_ID"; do
  tccutil reset ScreenCapture "$bid" 2>/dev/null || true
  tccutil reset Microphone "$bid" 2>/dev/null || true
  /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -u "$INSTALL_APP" 2>/dev/null || true
  rm -rf ~/Library/Saved\ Application\ State/${bid}.savedState 2>/dev/null || true
  rm -rf ~/Library/Caches/${bid} 2>/dev/null || true
  rm -rf ~/Library/WebKit/${bid} 2>/dev/null || true
  rm -rf ~/Library/Containers/${bid} 2>/dev/null || true
  defaults delete "$bid" 2>/dev/null || true
done

echo "🔨 Building..."
cd "$APP_DIR"
npm run tauri build 2>&1 | grep -E "(Compiling effective-recorder|Finished|Bundling|Error|error)" | head -10

echo "📦 Installing to ~/Applications/..."
mkdir -p "$INSTALL_DIR"
cp -R "$BUILD_APP" "$INSTALL_APP"

echo "🚀 Launching..."
open "$INSTALL_APP"

echo ""
echo "✅ Launched from: $INSTALL_APP"
echo ""
echo "📋 First launch after rebuild:"
echo "   1. macOS will show permission dialog → click 'Open System Settings'"
echo "   2. Find 'Effective Recorder' → enable toggle"
echo "   3. Click 'Quit & Reopen'"
