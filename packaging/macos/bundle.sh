#!/usr/bin/env bash
# Builds Merel.app for Apple Silicon and Intel in one binary, signs it for free (see below)
# and wraps it in a .dmg with a shortcut to Applications. Run on macOS from the
# repository root after both release builds:
#   cargo build --release -p merel --target aarch64-apple-darwin
#   cargo build --release -p merel --target x86_64-apple-darwin
set -euo pipefail
version="$(packaging/version.sh)"
commit="$(git rev-parse --short=7 HEAD 2>/dev/null || true)"
out=dist
work="$(mktemp -d)"
app="$work/Merel.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$out"

lipo -create -output "$app/Contents/MacOS/merel" \
  target/aarch64-apple-darwin/release/merel \
  target/x86_64-apple-darwin/release/merel

iconset="$work/Merel.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" packaging/icon.png --out "$iconset/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" packaging/icon.png --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/Merel.icns"

sed -e "s/@VERSION@/$version/g" -e "s/@BUILD@/${commit:-$version}/g" packaging/macos/Info.plist > "$app/Contents/Info.plist"

# No Apple account, so Gatekeeper asks once either way, but the bundle is sealed against
# changes. With MACOS_CERT_P12 (made by packaging/macos/make-signing-cert.sh) every release
# carries the same self-made identity, so the keychain keeps trusting Merel after an update;
# an ad-hoc signature differs per build and makes macOS ask for the keychain password again.
if [ -n "${MACOS_CERT_P12:-}" ]; then
  keychain="$work/sign.keychain-db"
  keychain_password="$(uuidgen)"
  security create-keychain -p "$keychain_password" "$keychain"
  security unlock-keychain -p "$keychain_password" "$keychain"
  printf '%s' "$MACOS_CERT_P12" | base64 --decode > "$work/cert.p12"
  echo "Importing the signing certificate"
  security import "$work/cert.p12" -k "$keychain" -P "${MACOS_CERT_PASSWORD:-}" -T /usr/bin/codesign
  rm "$work/cert.p12"
  # What arrived (names and kinds only), for when the next step finds nothing.
  security dump-keychain "$keychain" | grep -E '^class|"labl"' || true
  echo "Letting codesign use the key"
  security set-key-partition-list -S apple-tool:,apple: -s -k "$keychain_password" "$keychain" >/dev/null
  # Without -v: a self-made certificate is not trusted by the system, and need not be.
  security find-identity -p codesigning "$keychain"
  identity="$(security find-identity -p codesigning "$keychain" | awk '/"Merel"/ { print $2; exit }')"
  [ -n "$identity" ] || { echo "No Merel signing identity in MACOS_CERT_P12" >&2; exit 1; }
  codesign --force --deep --keychain "$keychain" --sign "$identity" "$app"
  security delete-keychain "$keychain"
else
  codesign --force --deep --sign - "$app"
fi
codesign --verify --deep --strict "$app"
# The requirement the keychain checks; the same on every release when signed with the certificate.
codesign --display --requirements - "$app"

stage="$work/dmg"
mkdir -p "$stage"
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
# hdiutil's own size guess runs short for a fresh bundle ("No space left on device"), so give
# it the content size plus room for the file system.
size_mb=$(($(du -sm "$stage" | cut -f1) * 12 / 10 + 20))
hdiutil create -volname "Merel" -srcfolder "$stage" -size "${size_mb}m" -fs HFS+ -ov -format UDZO "$out/Merel-$version-macos.dmg"
