#!/usr/bin/env bash
# Build Rust (release, device) -> Swift bindings -> xcodegen -> xcodebuild -> install -> launch on your iPhone.
# Usage: ./run.sh [--console]   (--console attaches and streams the app's stdout/stderr;
# ending the console session SIGTERMs the app, so relaunch without it afterwards)
set -euo pipefail
cd "$(dirname "$0")"

# Your iPhone's UDID (Xcode > Devices and Simulators > Identifier). To sign with your own
# Apple team, also set GM_TEAM (team ID) and GM_BUNDLE_ID (any ID unique to you).
DEVICE=${GM_DEVICE:?set GM_DEVICE to your iPhone UDID}
export GM_TEAM=${GM_TEAM:-39GU9D2P3G}
export GM_BUNDLE_ID=${GM_BUNDLE_ID:-com.jeffersonbutler.gm}
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
(cd ios && xcodegen generate --quiet)
xcodebuild -project ios/GM.xcodeproj -scheme GM -configuration Debug \
  -destination "id=$DEVICE" -allowProvisioningUpdates -derivedDataPath ios/build -quiet build

echo "==> install + launch"
xcrun devicectl device install app --device "$DEVICE" ios/build/Build/Products/Debug-iphoneos/GM.app
if [[ "${1:-}" == "--console" ]]; then
  xcrun devicectl device process launch --terminate-existing --console --device "$DEVICE" "$GM_BUNDLE_ID"
else
  xcrun devicectl device process launch --terminate-existing --device "$DEVICE" "$GM_BUNDLE_ID"
fi
