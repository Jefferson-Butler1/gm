# Which Rust 2D rendering stack for iOS?

Researched 2026-09-22 against crates.io, upstream source, issue trackers, and Apple docs.

## TL;DR

- **Recommendation: a thin custom renderer on raw `wgpu` 30, drawing into a `CAMetalLayer` that a Swift `UIView` owns.** Swift owns the app (SwiftUI + UIScene), the `CADisplayLink`, and touch input. Rust owns the sim and all game rendering. The boundary is a small UniFFI API.
- Every stack that makes Rust own the whole app (Bevy through winit, macroquad/miniquad) currently has two blocking problems on iPhone. It is **capped at 60 Hz**, and it does **not adopt the UIScene lifecycle**. Apps built with the iOS 27 SDK (Xcode 27, shipped Sept 2026) fail to launch without UIScene, and that SDK becomes mandatory for App Store uploads in April 2027.
- **Fallback: Swift renders with SpriteKit or Metal from sim snapshots.** The host shell, display link, and input path stay the same; only the draw call changes owner. Bevy embedded via the `bevy-in-app` pattern is a distant second choice if we later want an engine.

## Comparison

| Criterion | Bevy 0.19.1 (0.20-rc.1) | macroquad 0.4.16 / miniquad 0.4.11 | wgpu 30.0.1, custom renderer, Swift-owned layer |
|---|---|---|---|
| iOS maturity | Works, but the iOS path goes through winit 0.30. Many open iOS issues: 60 fps cap, CPU and frame-pacing problems, a memory leak with 2D sprites | Works, but the iOS backend is thin. Defaults to GL (OpenGL ES, deprecated since iOS 12); Metal is opt-in | Metal backend is the primary Apple path. `SurfaceTargetUnsafe::CoreAnimationLayer` exists specifically for this. Simulator has known Metal validation issues |
| Touch | Through winit touch events. `FingerId` may be unstable (winit #3970). Touch floods drop FPS (#4204) | Touch IDs unstable (miniquad #589) | Native `UITouch` in Swift (timestamps, coalesced and predicted touches), forwarded to Rust. We control this completely |
| 120 Hz / pacing | **Capped at 60** (bevy #19358 → winit #4358, open). No display-link API in winit (#2412, open) | **Hard-coded 60**: `setPreferredFramesPerSecond:60` in `ios.rs` | Swift `CADisplayLink.preferredFrameRateRange` + `CADisableMinimumFrameDurationOnPhone`. wgpu on iOS offers `Fifo` only, with `maximumDrawableCount` set from `desired_maximum_frame_latency` (1..=2) |
| Embedding in SwiftUI | No (the default plugins call `UIApplicationMain`). Possible only by dropping `WinitPlugin`, as in `jinleili/bevy-in-app`, which is stuck on Bevy 0.18 | No. Calls `UIApplicationMain` and owns the `UIWindow` | Yes, by design. `UIViewRepresentable` wraps the layer-backed view, and SwiftUI overlays sit on top |
| UIScene (iOS 27 SDK) | winit has not adopted it (#4224, open). No UIScene code in 0.30.13 or 0.31.0-beta.3 | No UIScene code | Not applicable: Swift/SwiftUI owns the lifecycle |
| macOS iteration | Good (`cargo run`); clean build is heavy | Excellent | Good: desktop harness with winit + wgpu on macOS (winit's macOS path does not have these iOS problems) |
| Build pipeline | Staticlib + Xcode project; Bevy's own mobile example uses a hand-maintained xcodeproj | Staticlib + manual Xcode project (no iOS tooling found) | Staticlib → xcframework → local SwiftPM package with UniFFI bindings |
| Clean release build* | 141 s | 5 s | 19 s |
| Stripped binary* | 52.9 MB | 0.9 MB | 2.9 MB |
| Incremental debug rebuild* | 2.0 s | 0.3 s | 0.4 s |

\*Measured locally on an M1 Max with rustc 1.97.1 for aarch64-apple-darwin, using toy crates (`opt-level="s"`, thin LTO, strip). Bevy used `default-features=false, features=["2d"]`; wgpu used `metal`+`wgsl` only. Treat these as relative numbers only; real game code will dominate incremental times. Clean iOS staticlib builds took about the same time: wgpu 18 s, macroquad 5 s. Bevy 0.19 needs rustc ≥ 1.95.

## Per-option notes

### Bevy
- Versions: stable 0.19.1, and 0.20.0-rc.1 (2026-09-15), which moves to wgpu 30 but **still depends on winit ^0.30** ([crates.io](https://crates.io/crates/bevy_winit)).
- 60 fps cap on iOS: [bevy#19358](https://github.com/bevyengine/bevy/issues/19358) traces to [winit#4358](https://github.com/rust-windowing/winit/issues/4358), still open as of 2026-09-18. A winit maintainer suggested the fix is a CADisplayLink API ([winit#2412](https://github.com/rust-windowing/winit/issues/2412)) that has not been built. Until then, the workaround is to "use CADisplayLink directly with winit".
- Other open iOS issues: high CPU and low FPS on an empty app ([#16734](https://github.com/bevyengine/bevy/issues/16734)), a 2D sprite memory leak on 0.18.1 ([#23453](https://github.com/bevyengine/bevy/issues/23453)), priority inversion ([#16762](https://github.com/bevyengine/bevy/issues/16762)), and no safe-area insets ([#23003](https://github.com/bevyengine/bevy/issues/23003)).
- Embedding: the default `WinitPlugin` takes over the app. [`jinleili/bevy-in-app`](https://github.com/jinleili/bevy-in-app) removes it, injects a `CAMetalLayer` handle, and drives frames from the host. Its latest version supports only Bevy 0.18, and the last commit was 2026-02-01, so it lags Bevy by two releases.
- Cost: 53 MB binary and 141 s clean build for an empty 2D app. We would get an ECS and renderer we don't need, because the sim is already a separate headless crate.

### macroquad / miniquad
- macroquad 0.4.16 (2026-07-30) pins `miniquad =0.4.11`. The GitHub repo publishes no releases, only crates.io versions.
- iOS backend source (`miniquad-0.4.11/src/native/ios.rs`) calls `UIApplicationMain` and builds its own `UIWindow`/`UIViewController`. It creates `MTKView` with `setPreferredFramesPerSecond:60` hard-coded, and it contains no UIScene code. The default graphics API is GL ("for legacy reasons", `conf.rs`). [OpenGL ES was deprecated in iOS 12](https://developer.apple.com/documentation/opengles).
- Open issues: [unstable iOS touch IDs #589](https://github.com/not-fl3/miniquad/issues/589) and [macOS-only API called on iOS #561](https://github.com/not-fl3/miniquad/issues/561).
- It is the fastest to iterate with and the smallest, but it has no embedding story and would need forking for both 120 Hz and UIScene.

### wgpu + thin custom renderer (recommended)
- wgpu 30.0.1 (2026-08-22). The source defines `SurfaceTargetUnsafe::CoreAnimationLayer(*mut c_void)` (behind `cfg(metal)`), which creates a surface directly from a `CAMetalLayer`, with no winit or raw-window-handle involved. docs.rs does not show this variant because of the cfg gate.
- The Metal HAL (`wgpu-hal-30.0.1/src/metal/`) on iOS exposes only `PresentMode::Fifo`. `Immediate` requires `displaySyncEnabled`, which is macOS-only. It sets `maximumDrawableCount = desired_maximum_frame_latency + 1`, with latency in the range 1..=2. There is no `presentAfterMinimumDuration` or frame-rate control, so the host display link sets the pace. There is also no per-frame presentation timestamp yet ([wgpu#9856](https://github.com/gfx-rs/wgpu/issues/9856), proposed).
- Prior art: [`jinleili/wgpu-in-app`](https://github.com/jinleili/wgpu-in-app) embeds wgpu into existing iOS/Android apps without winit, and was updated to wgpu 30 on 2026-07-14.
- Simulator: open issue [wgpu#7057](https://github.com/gfx-rs/wgpu/issues/7057) (Metal validation and alignment failures in the iOS Simulator). Plan to test feel on a physical device.
- Renderer scope for pixel art is small: a sprite-batch pipeline, one texture atlas, nearest sampling, an integer-scaled offscreen target blitted to the drawable, and a few WGSL post effects. [`pixels`](https://crates.io/crates/pixels) 0.17.2 (wgpu framebuffer) could be used for the blit stage, but it isn't required.

### Other contenders (rejected)
- **Fyrox 1.0**: renders on OpenGL today; its 1.0 export CLI lists `pc/wasm/android` but not iOS ([Fyrox 1.0 post](https://fyrox.rs/blog/post/fyrox-game-engine-1-0-0/)).
- **Godot + gdext**: Godot, not Rust, would own rendering, and gdext calls its iOS support "experimental" ([gdext README](https://github.com/godot-rust/gdext), [#498](https://github.com/godot-rust/gdext/issues/498)).
- **ggez 0.10 / notan 0.14**: not evaluated (no iOS focus found; unverified).

## Platform facts that drive the decision

- **UIScene is now mandatory.** From TN3187: "In the next major release following iOS 26, UIScene lifecycle will be required when building with the latest SDK; otherwise, your app won't launch" ([TN3187](https://developer.apple.com/documentation/technotes/tn3187-migrating-to-the-uikit-scene-based-life-cycle)). Xcode 27 and the iOS 27 SDK shipped in Sept 2026. [Apple news](https://developer.apple.com/news/?id=k1mtkt1k) and [coverage](https://www.macobserver.com/news/april-2027-sdk-requirement-five-platforms/) say the iOS 27 SDK is required for uploads from April 2027. Neither winit (0.30.13, 0.31.0-beta.3) nor miniquad has any UIScene code (checked in source). A SwiftUI host gets UIScene for free.
- **120 Hz on iPhone** requires the `CADisableMinimumFrameDurationOnPhone` Info.plist key, plus a `CADisplayLink` with a `preferredFrameRateRange` ([Apple: ProMotion](https://developer.apple.com/documentation/quartzcore/optimizing-iphone-and-ipad-apps-to-support-promotion-displays)).

## Recommended integration approach

```
GameApp (SwiftUI, UIScene)
└─ ZStack
   ├─ GameView: UIViewRepresentable → GameUIView (layerClass = CAMetalLayer)
   │    - CADisplayLink(preferredFrameRateRange 80...120, preferred 120) on main runloop
   │    - touchesBegan/Moved/Ended/Cancelled → coalesced touches → Rust input buffer
   └─ SwiftUI HUD/menus (fed by a small snapshot struct returned from Rust each frame)
```

1. **Crates.** Keep `sim` headless. Add a `render` crate (wgpu 30, `default-features=false`, `metal` + `wgsl`) and an `ffi` crate (`crate-type = ["staticlib", "lib"]`, UniFFI 0.32) that owns a `Game { sim, renderer, input }`.
2. **FFI surface (UniFFI object).** `Game::new(layer_ptr: u64, width_px, height_px, scale)`, `resize(...)`, `push_touches(Vec<TouchEvent>)`, `frame(timestamp, target_timestamp) -> HudSnapshot`, `pause()`, `resume()`. Pass the layer pointer as `u64`, cast it back to `*mut c_void` in Rust, and keep the view alive on the Swift side for the surface's lifetime. The loop is `frame()`: fixed-step sim updates with an accumulator, then render.
3. **Surface config.** `PresentMode::Fifo`, `desired_maximum_frame_latency: 2` (3 drawables). *Measured in the feel spike:* latency 1 (2 drawables) blocks `nextDrawable()` for about one vsync, which halves an iPhone 14 Pro to 60 fps. Latency 2 holds 119.9 fps with a ~0.7 ms acquire. Set `contentsScale` on the layer and render pixel art to a low-res offscreen target, then integer-scale.
4. **Build.** A script (or `cargo-swift`) builds `aarch64-apple-ios`, `aarch64-apple-ios-sim`, and `aarch64-apple-darwin`, runs `uniffi-bindgen` for Swift, and runs `xcodebuild -create-xcframework`. It outputs a local SwiftPM package that the Xcode app (generated by XcodeGen) depends on. Note that [cargo-swift](https://github.com/antoniusnaumann/cargo-swift) 0.11.1 supports UniFFI up to 0.31.1, while [UniFFI](https://crates.io/crates/uniffi) is at 0.32.1. Either pin UniFFI 0.31 to use cargo-swift, or write a ~30-line build script. Prefer the script.
5. **Desktop loop.** Add a `desktop` bin: winit window + the same `render` + `sim`, with mouse/keyboard mapped to the same `TouchEvent`s. Run it with `cargo run`, no Xcode needed. winit's macOS backend has none of the iOS problems above.
6. **Fallback path.** If Rust rendering hurts feel, swap `GameUIView` for an `SKView`/`MTKView` drawing from `frame()`'s snapshot. Nothing else in the host or the FFI changes except that `render` is no longer called.

## Open risks for the feel spike

1. **Touch-to-photon latency** at 120 Hz with `desired_maximum_frame_latency` 1 vs 2. Measure with a high-speed camera, or with `CADisplayLink.targetTimestamp` vs `UITouch.timestamp`. wgpu has no present-timestamp API yet (#9856).
2. **Pacing stability.** Check whether `get_current_texture()` (a blocking `nextDrawable`) inside the display-link callback ever misses the deadline or drops to 60 Hz, and whether ProMotion ramps down during idle menus. Try `preferredFrameRateRange` min 80 vs 120.
3. **Main-thread budget.** Sim + render + SwiftUI overlay compositing on the main thread in one callback. Watch the frame-time headroom at 8.3 ms. A render thread is a later option but not a day-one one.
4. **SwiftUI overlay cost.** Check that HUD updates at 120 Hz don't cause SwiftUI re-render hitches. The HUD may only need 10–30 Hz updates.
5. **Simulator.** wgpu#7057 validation failures. Check whether the simulator is usable for layout work or device-only.
6. **FFI overhead** per frame (UniFFI `Vec<TouchEvent>` lowering). Expected to be negligible but not measured. If it shows up, switch to a hand-written `extern "C"` hot path.
7. **Lifecycle.** Background/foreground and scene disconnect: pause the display link, drop or reconfigure the surface, and handle the layer resizing on rotation or Dynamic Island insets.
