#!/usr/bin/env bash
# Builds Merel's Rust core for Android phones (arm64) and the emulator (x86_64) into
# app/src/main/jniLibs, and writes the Kotlin bindings into app/src/main/java/uniffi.
# Needs the Android NDK (ANDROID_NDK_HOME), and once:
#   rustup target add aarch64-linux-android x86_64-linux-android && cargo install cargo-ndk
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
root="$(cd "$here/../.." && pwd)"
cd "$root"
cargo ndk -t arm64-v8a -t x86_64 -o "$here/app/src/main/jniLibs" build -p merel-mobile --lib --release
cargo build -p merel-mobile --lib
lib=target/debug/libmerel_mobile.so
[ -f "$lib" ] || lib=target/debug/libmerel_mobile.dylib
rm -rf "$here/app/src/main/java/uniffi"
cargo run -q -p merel-mobile --bin uniffi-bindgen -- generate --library "$lib" --language kotlin --no-format \
  --out-dir "$here/app/src/main/java"
echo "Done: $here/app/src/main"
