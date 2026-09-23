#!/usr/bin/env bash
# One-shot: build Rust (release, device) -> Swift bindings -> xcodegen -> xcodebuild -> install -> launch.
# Usage: ./run.sh [--console]   (--console attaches and streams the app's stdout/stderr)
set -euo pipefail
cd "$(dirname "$0")"

DEVICE_XCB=00008120-001C7D480C80201E
DEVICE_CTL=CD58EBDC-2EB9-582A-843C-3E7443A69089
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
/opt/homebrew/bin/xcodegen generate --quiet
xcodebuild -project FeelSpike.xcodeproj -scheme FeelSpike -configuration Debug \
  -destination "id=$DEVICE_XCB" -allowProvisioningUpdates -derivedDataPath build -quiet build

echo "==> install + launch"
xcrun devicectl device install app --device "$DEVICE_CTL" build/Build/Products/Debug-iphoneos/FeelSpike.app
if [[ "${1:-}" == "--console" ]]; then
  xcrun devicectl device process launch --terminate-existing --console --device "$DEVICE_CTL" "$BUNDLE"
else
  xcrun devicectl device process launch --terminate-existing --device "$DEVICE_CTL" "$BUNDLE"
fi
