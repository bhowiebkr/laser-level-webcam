#!/bin/sh
# Build a local macOS app bundle. This is ad-hoc signed, not notarized for distribution.
set -eu
cd "$(dirname "$0")/../.."
profile=${1:-release}
case "$profile" in
    release) cargo build --locked --release -p laser-level-webcam ;;
    debug) cargo build --locked -p laser-level-webcam ;;
    *) printf 'Usage: sh packaging/macos/bundle.sh [release|debug]\n' >&2; exit 2 ;;
esac
app="$PWD/dist/Laser Level Webcam.app"
mkdir -p "$app/Contents/MacOS"
cp "${CARGO_TARGET_DIR:-target}/$profile/laser-level-webcam" "$app/Contents/MacOS/laser-level-webcam"
cp packaging/macos/Info.plist "$app/Contents/Info.plist"
/usr/libexec/PlistBuddy -c 'Add :CFBundleExecutable string laser-level-webcam' "$app/Contents/Info.plist"
/usr/libexec/PlistBuddy -c 'Add :CFBundlePackageType string APPL' "$app/Contents/Info.plist"
/usr/bin/codesign --force --sign - "$app"
printf '%s\n' "$app"
