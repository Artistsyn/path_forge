#!/usr/bin/env bash
# Builds libpathforge.a (mobile/) for iPhones into ios/build/native, for the Xcode project to link.
# Needs a Mac with Xcode and Rust: `rustup target add aarch64-apple-ios`.
set -euo pipefail
cd "$(dirname "$0")/.."
export IPHONEOS_DEPLOYMENT_TARGET=16.0
cargo rustc --release -p path_forge_mobile --lib --crate-type staticlib --target aarch64-apple-ios -- --print native-static-libs
mkdir -p ios/build/native
cp target/aarch64-apple-ios/release/libpathforge.a ios/build/native/
