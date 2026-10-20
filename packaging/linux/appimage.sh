#!/usr/bin/env bash
# Wraps the release build in an AppImage that runs on most Linux distributions. Run from the
# repository root after `cargo build --release -p mailcraft`; needs appimagetool on PATH.
set -euo pipefail
version="$(packaging/version.sh)"
out=dist
dir="$(mktemp -d)/Mailcraft.AppDir"
mkdir -p "$dir/usr/bin" "$out"
install -m 755 target/release/mailcraft "$dir/usr/bin/mailcraft"
install -m 644 packaging/linux/mailcraft.desktop "$dir/mailcraft.desktop"
install -m 644 packaging/icon-256.png "$dir/mailcraft.png"
ln -s usr/bin/mailcraft "$dir/AppRun"
ARCH=x86_64 appimagetool --appimage-extract-and-run "$dir" "$out/Mailcraft-$version-linux-x86_64.AppImage"
