#!/bin/sh
# Builds the release exe and assembles dist/: exe, libmpv-2.dll, fonts/, skins/, LICENSES.
set -e
cd "$(dirname "$0")/.."
/mnt/c/Users/enrell/.cargo/bin/cargo.exe build --release -j 6
rm -rf dist && mkdir -p dist/fonts dist/skins
cp /mnt/c/lain-proto-target/slint/release/lain-slint.exe dist/
cp /mnt/d/lain-proto/libmpv/libmpv-2.dll dist/
cp fonts/*.ttf fonts/OFL-*.txt dist/fonts/
cp skins/*.slint dist/skins/
cp README.md dist/README.md 2>/dev/null || true
ls -la dist
