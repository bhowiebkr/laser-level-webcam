#!/usr/bin/env bash
# Builds the macOS release: dist/laser-level-webcam-<version>-macos-universal.zip
#
# The zip holds "Laser Level Webcam.app", the `laser` command-line tool, the README, the
# LICENSE and the third-party licences. Both programs are universal: one file with the Apple silicon and the Intel code.
# Runs on a Mac with Xcode or its command-line tools. Needs both Rust targets, once:
#   rustup target add aarch64-apple-darwin x86_64-apple-darwin
#
# The app is signed ad hoc (no Apple developer ID), so macOS asks before the first start.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"
short="${version%%-*}"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}"

for target in aarch64-apple-darwin x86_64-apple-darwin; do
    cargo build --release --locked --target "$target" -p laser-level-webcam -p laser
done

stage="$root/target/macos-app"
app="$stage/Laser Level Webcam.app"
rm -rf "$stage"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
for bin in laser-level-webcam laser; do
    lipo -create -output "$stage/$bin" \
        "target/aarch64-apple-darwin/release/$bin" "target/x86_64-apple-darwin/release/$bin"
done
mv "$stage/laser-level-webcam" "$app/Contents/MacOS/laser-level-webcam"
sed -e "s/@VERSION@/$version/g" -e "s/@SHORT@/$short/g" packaging/macos/Info.plist > "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist"
printf 'APPL????' > "$app/Contents/PkgInfo"
cp assets/icon.icns "$app/Contents/Resources/AppIcon.icns"
cp README.md LICENSE "$stage/"
cargo run --release --locked -p pack -- licenses --target aarch64-apple-darwin --target x86_64-apple-darwin \
    --out "$stage/THIRD-PARTY-LICENSES.txt"

codesign --force --sign - "$stage/laser"
codesign --force --deep --sign - "$app"
codesign --verify --deep --strict "$app"
lipo -info "$app/Contents/MacOS/laser-level-webcam" "$stage/laser"

mkdir -p dist
zip="$root/dist/laser-level-webcam-$version-macos-universal.zip"
rm -f "$zip"
# ditto keeps the app's symbolic links and extended attributes, which the signature covers.
ditto -c -k --sequesterRsrc "$stage" "$zip"
echo "Built $zip"
