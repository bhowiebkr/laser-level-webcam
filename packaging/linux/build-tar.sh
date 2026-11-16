#!/usr/bin/env bash
# Builds the Linux release: dist/laser-level-webcam-<version>-linux-x64.tar.gz
#
# The archive holds one folder with the window app, the `laser` command-line tool, the icon, a
# desktop entry, the README, the LICENSE and the third-party licences. Build it on the oldest Linux you want it to run on:
# the programs need that glibc or newer.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"

cargo build --release --locked -p laser-level-webcam -p laser

name="laser-level-webcam-$version-linux-x64"
stage="$root/target/linux-tar/$name"
rm -rf "$root/target/linux-tar"
mkdir -p "$stage"
cp target/release/laser-level-webcam target/release/laser "$stage/"
strip "$stage/laser-level-webcam" "$stage/laser"
cp assets/icon.png "$stage/laser-level-webcam.png"
cp packaging/linux/laser-level-webcam.desktop README.md LICENSE "$stage/"
cargo run --release --locked -p pack -- licenses --target x86_64-unknown-linux-gnu --out "$stage/THIRD-PARTY-LICENSES.txt"

mkdir -p dist
tar -C "$root/target/linux-tar" -czf "dist/$name.tar.gz" "$name"
echo "Built dist/$name.tar.gz"
