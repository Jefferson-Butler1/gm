#!/usr/bin/env bash
# Build Rust (release, device) -> Swift bindings -> xcodegen -> xcodebuild -> install -> launch on Garold.
# Usage: ./run.sh [--console]   (--console attaches and streams the app's stdout/stderr;
# ending the console session SIGTERMs the app, so relaunch without it afterwards)
set -euo pipefail
cd "$(dirname "$0")"

DEVICE_XCB=00008120-001C7D480C80201E
DEVICE_CTL=CD58EBDC-2EB9-582A-843C-3E7443A69089
BUNDLE=com.jeffersonbutler.gm
# Homebrew rustc has no iOS std; use rustup's proxies so rust-toolchain.toml applies.
export PATH="$HOME/.cargo/bin:$PATH"

echo "==> cargo build (aarch64-apple-ios, release)"
cargo build --release --target aarch64-apple-ios -p game

echo "==> uniffi bindings"
rm -rf ios/Generated
cargo run -q -p uniffi-bindgen -- generate --library target/aarch64-apple-ios/release/libgame.a \
  --language swift --out-dir ios/Generated
mkdir -p ios/Generated/include
mv ios/Generated/gameFFI.h ios/Generated/include/gameFFI.h
mv ios/Generated/gameFFI.modulemap ios/Generated/include/module.modulemap

echo "==> xcodegen + xcodebuild"
(cd ios && /opt/homebrew/bin/xcodegen generate --quiet)
xcodebuild -project ios/GM.xcodeproj -scheme GM -configuration Debug \
  -destination "id=$DEVICE_XCB" -allowProvisioningUpdates -derivedDataPath ios/build -quiet build

echo "==> install + launch"
xcrun devicectl device install app --device "$DEVICE_CTL" ios/build/Build/Products/Debug-iphoneos/GM.app
if [[ "${1:-}" == "--console" ]]; then
  xcrun devicectl device process launch --terminate-existing --console --device "$DEVICE_CTL" "$BUNDLE"
else
  xcrun devicectl device process launch --terminate-existing --device "$DEVICE_CTL" "$BUNDLE"
fi
