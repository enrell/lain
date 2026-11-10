#!/bin/sh
# Builds the Rust core for the app (D-251):
#   1. lain-ffi for the host, then the Kotlin bindings from it
#      (uniffi-bindgen in library mode) into app/generated/uniffi;
#   2. lain-ffi for each Android ABI with cargo-ndk into
#      app/build/rustJniLibs/<abi>/ (skipped with --bindings-only).
# Needs cargo; step 2 needs cargo-ndk, the Rust Android targets and an
# NDK (ANDROID_NDK_HOME). PROFILE=debug builds unoptimized libraries.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
ffi="$here/lain-ffi"
app="$here/app"
profile=${PROFILE:-release}
target_dir=${CARGO_TARGET_DIR:-$ffi/target}
export CARGO_TARGET_DIR="$target_dir"

case "$(uname -s)" in
    Darwin) host_lib=liblain_ffi.dylib ;;
    *) host_lib=liblain_ffi.so ;;
esac

cargo build --locked --manifest-path "$ffi/Cargo.toml"
rm -rf "$app/generated/uniffi"
cargo run --locked --manifest-path "$ffi/Cargo.toml" --features bindgen --bin uniffi-bindgen -- \
    generate --library "$target_dir/debug/$host_lib" --language kotlin --no-format \
    --out-dir "$app/generated/uniffi"
echo "host library: $target_dir/debug/$host_lib"

if [ "${1:-}" = "--bindings-only" ]; then
    exit 0
fi

release_flag=""
if [ "$profile" = "release" ]; then
    release_flag="--release"
fi
rm -rf "$app/build/rustJniLibs"
# shellcheck disable=SC2086 # release_flag is empty or one word
cargo ndk --manifest-path "$ffi/Cargo.toml" -t arm64-v8a -t x86_64 --platform 26 \
    -o "$app/build/rustJniLibs" build --locked $release_flag
ls -l "$app/build/rustJniLibs"/*
