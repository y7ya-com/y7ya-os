#!/usr/bin/env bash
# Builds the Rust core (crates/notes-ffi) for Android and regenerates its Kotlin
# bindings. Run this before `./gradlew assembleDebug` on a fresh checkout.
set -euo pipefail

ANDROID_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(dirname "$ANDROID_DIR")"
ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
NDK_VERSION="${NDK_VERSION:-30.0.16248370}"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$ANDROID_HOME/ndk/$NDK_VERSION}"
export PATH="$HOME/.cargo/bin:$PATH"

cd "$REPO_DIR"

echo "==> Building libnotes_ffi.so for arm64-v8a and x86_64"
rm -rf "$ANDROID_DIR/app/src/main/jniLibs"
cargo ndk -t arm64-v8a -t x86_64 -P 26 -o "$ANDROID_DIR/app/src/main/jniLibs" \
    build --profile android -p notes-ffi
# cargo-ndk also copies .so files of some dependencies; only ours is loaded.
find "$ANDROID_DIR/app/src/main/jniLibs" -name '*.so' ! -name 'libnotes_ffi.so' -delete

# Release libraries are stripped of UniFFI metadata, so bindings come from the
# host debug build.
echo "==> Generating Kotlin bindings"
cargo build -p notes-ffi
rm -rf "$ANDROID_DIR/app/src/main/java/uniffi"
cargo run -q -p notes-ffi --bin uniffi-bindgen -- generate \
    --library target/debug/libnotes_ffi.so --language kotlin --no-format \
    --out-dir "$ANDROID_DIR/app/src/main/java"

# Point Gradle at the SDK on a fresh checkout.
if [ ! -f "$ANDROID_DIR/local.properties" ]; then
    echo "sdk.dir=$ANDROID_HOME" > "$ANDROID_DIR/local.properties"
fi

echo "==> Done"
