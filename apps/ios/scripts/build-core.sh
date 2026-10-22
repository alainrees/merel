#!/usr/bin/env bash
# Builds Merel's Rust core for the iPhone and the simulator (Apple silicon and Intel Macs),
# writes the Swift bindings and packs it all into Generated/MerelCore.xcframework.
# Run from anywhere on a Mac with Xcode and rustup:
#   rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
#   apps/ios/scripts/build-core.sh
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
root="$(cd "$here/../.." && pwd)"
out="$here/Generated"
export IPHONEOS_DEPLOYMENT_TARGET=17.0
cd "$root"
for target in aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios; do
  cargo build -p merel-mobile --lib --release --target "$target"
done
cargo build -p merel-mobile --lib
rm -rf "$out" && mkdir -p "$out/headers" "$out/sim"
cargo run -q -p merel-mobile --bin uniffi-bindgen -- generate --library target/debug/libmerel_mobile.dylib \
  --language swift --no-format --out-dir "$out/bindings"
mkdir -p "$out/Swift" && mv "$out/bindings/merel_mobile.swift" "$out/Swift/MerelCore.swift"
cp "$out/bindings/merel_mobileFFI.h" "$out/headers/"
cp "$out/bindings/merel_mobileFFI.modulemap" "$out/headers/module.modulemap"
rm -rf "$out/bindings"
lipo -create target/aarch64-apple-ios-sim/release/libmerel_mobile.a target/x86_64-apple-ios/release/libmerel_mobile.a \
  -output "$out/sim/libmerel_mobile.a"
xcodebuild -create-xcframework \
  -library target/aarch64-apple-ios/release/libmerel_mobile.a -headers "$out/headers" \
  -library "$out/sim/libmerel_mobile.a" -headers "$out/headers" \
  -output "$out/MerelCore.xcframework"
rm -rf "$out/headers" "$out/sim"
echo "Done: $out"
