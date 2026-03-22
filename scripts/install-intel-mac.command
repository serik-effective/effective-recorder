#!/bin/bash
# ============================================================
#  Effective Recorder — Install (Intel Mac)
# ============================================================
#
#  macOS may show a warning that the app is from an
#  unidentified developer. If you see "damaged" error
#  (macOS Ventura+), this script fixes it.
#
#  HOW TO USE:
#  1. Open the DMG and drag "Effective Recorder" to Applications
#  2. Try launching — if macOS blocks it, run this script
#  3. Double-click this script (install-intel-mac.command)
#  4. Launch Effective Recorder from Applications
#
#  NOTE: On older macOS you may not need this script.
#  Just right-click the app → Open → confirm in the dialog.
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
    echo ""
    echo "Or try: right-click the app → Open → confirm"
fi

echo ""
read -p "Press Enter to close..."
