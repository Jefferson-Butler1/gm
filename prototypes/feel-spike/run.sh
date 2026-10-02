#!/usr/bin/env bash
# One-shot: build Rust (release, device) -> Swift bindings -> xcodegen -> xcodebuild -> install -> launch.
# Usage: ./run.sh [--console]   (--console attaches and streams the app's stdout/stderr)
set -euo pipefail
cd "$(dirname "$0")"

DEVICE=${GM_DEVICE:?set GM_DEVICE to your iPhone UDID}
BUNDLE=com.jeffersonbutler.gm
# Homebrew rustc has no iOS std; force the rustup toolchain.
export PATH="$HOME/.cargo/bin:$PATH"

echo "==> cargo build (aarch64-apple-ios, release)"
(cd rust && cargo build --release --target aarch64-apple-ios -p spike)

echo "==> uniffi bindings"
(cd rust && cargo run -q -p uniffi-bindgen -- generate --library target/aarch64-apple-ios/release/libspike.a --language swift --out-dir ../Generated)
mkdir -p Generated/include
mv Generated/spikeFFI.h Generated/include/spikeFFI.h
mv Generated/spikeFFI.modulemap Generated/include/module.modulemap

echo "==> xcodegen + xcodebuild"
xcodegen generate --quiet
xcodebuild -project FeelSpike.xcodeproj -scheme FeelSpike -configuration Debug \
  -destination "id=$DEVICE" -allowProvisioningUpdates -derivedDataPath build -quiet build

echo "==> install + launch"
xcrun devicectl device install app --device "$DEVICE" build/Build/Products/Debug-iphoneos/FeelSpike.app
if [[ "${1:-}" == "--console" ]]; then
  xcrun devicectl device process launch --terminate-existing --console --device "$DEVICE" "$BUNDLE"
else
  xcrun devicectl device process launch --terminate-existing --device "$DEVICE" "$BUNDLE"
fi
