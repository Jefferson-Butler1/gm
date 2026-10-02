# GM

A co-op roguelite shooter for iPhone: board a derelict ship, fight room to room, get out
through an airlock. Rust core (deterministic sim, wgpu renderer), SwiftUI shell.

## Building from source

Needs a Mac with Xcode, [rustup](https://rustup.rs) (`rust-toolchain.toml` pins the
toolchain and iOS targets), [XcodeGen](https://github.com/yonaskolb/XcodeGen), and an
iPhone in developer mode.

```sh
export GM_DEVICE=<your iPhone's UDID>       # Xcode > Devices and Simulators > Identifier
export GM_TEAM=<your Apple team ID>         # any free Apple account works
export GM_BUNDLE_ID=com.yourname.gm         # unique to you
./run.sh                                    # build, install, launch
```

`cargo test` runs the sim and game tests on the Mac.

## License

Code is MIT or Apache-2.0, at your option (`LICENSE-MIT`, `LICENSE-APACHE`). Third-party assets are CC0; see `CREDITS.md`.
