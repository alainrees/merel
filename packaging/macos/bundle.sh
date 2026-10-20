#!/usr/bin/env bash
# Builds Mailcraft.app for Apple Silicon and Intel in one binary, gives it a free ad-hoc
# signature and wraps it in a .dmg with a shortcut to Applications. Run on macOS from the
# repository root after both release builds:
#   cargo build --release -p mailcraft --target aarch64-apple-darwin
#   cargo build --release -p mailcraft --target x86_64-apple-darwin
set -euo pipefail
version="$(packaging/version.sh)"
commit="$(git rev-parse --short=7 HEAD 2>/dev/null || true)"
out=dist
work="$(mktemp -d)"
app="$work/Mailcraft.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$out"

lipo -create -output "$app/Contents/MacOS/mailcraft" \
  target/aarch64-apple-darwin/release/mailcraft \
  target/x86_64-apple-darwin/release/mailcraft

iconset="$work/Mailcraft.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" packaging/icon.png --out "$iconset/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" packaging/icon.png --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/Mailcraft.icns"

sed -e "s/@VERSION@/$version/g" -e "s/@BUILD@/${commit:-$version}/g" packaging/macos/Info.plist > "$app/Contents/Info.plist"

# Ad-hoc: no Apple account, so Gatekeeper asks once, but the bundle is sealed against changes.
codesign --force --deep --sign - "$app"
codesign --verify --deep --strict "$app"

stage="$work/dmg"
mkdir -p "$stage"
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
hdiutil create -volname "Mailcraft" -srcfolder "$stage" -ov -format UDZO "$out/Mailcraft-$version-macos.dmg"
