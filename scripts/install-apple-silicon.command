#!/bin/bash
# ============================================================
#  Effective Recorder — Install (Apple Silicon)
# ============================================================
#
#  macOS Sequoia blocks unsigned apps with "damaged" error.
#  This script removes the quarantine flag so you can run it.
#
#  HOW TO USE:
#  1. Open the DMG and drag "Effective Recorder" to Applications
#  2. Double-click this script (install-apple-silicon.command)
#  3. Enter your password if asked
#  4. Launch Effective Recorder from Applications
#
# ============================================================

APP_NAME="Effective Recorder"

echo ""
echo "=== Effective Recorder — Installer ==="
echo ""

# Check if app is in /Applications
if [ -d "/Applications/$APP_NAME.app" ]; then
    echo "Found: /Applications/$APP_NAME.app"
    xattr -cr "/Applications/$APP_NAME.app"
    echo "Quarantine removed. You can now launch the app!"
elif [ -d "$HOME/Applications/$APP_NAME.app" ]; then
    echo "Found: ~/Applications/$APP_NAME.app"
    xattr -cr "$HOME/Applications/$APP_NAME.app"
    echo "Quarantine removed. You can now launch the app!"
else
    echo "App not found in /Applications or ~/Applications."
    echo ""
    echo "Please drag '$APP_NAME' from DMG to Applications first,"
    echo "then run this script again."
    echo ""
    echo "Or run manually:"
    echo "  xattr -cr /Applications/Effective\ Recorder.app"
fi

echo ""
read -p "Press Enter to close..."
