#!/usr/bin/env bash
# Builds libpathforge.so (mobile/) for the app, into app/src/main/jniLibs.
# Needs the Android NDK (ANDROID_NDK_HOME) and cargo-ndk: `cargo install cargo-ndk` and
# `rustup target add aarch64-linux-android`.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo ndk -t arm64-v8a -P 29 -o android/app/src/main/jniLibs build --release -p path_forge_mobile
