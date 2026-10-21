#!/usr/bin/env bash
# Wraps the release build in an AppImage that runs on most Linux distributions. Run from the
# repository root after `cargo build --release -p merel`; needs appimagetool on PATH.
set -euo pipefail
version="$(packaging/version.sh)"
out=dist
dir="$(mktemp -d)/Merel.AppDir"
mkdir -p "$dir/usr/bin" "$out"
install -m 755 target/release/merel "$dir/usr/bin/merel"
install -m 644 packaging/linux/merel.desktop "$dir/merel.desktop"
install -m 644 packaging/icon-256.png "$dir/merel.png"
ln -s usr/bin/merel "$dir/AppRun"
ARCH=x86_64 appimagetool --appimage-extract-and-run "$dir" "$out/Merel-$version-linux-x86_64.AppImage"
