# Toolchain

## Programs

| Program | How to install | Why here | Source |
| --- | --- | --- | --- |
| rustc, cargo, clippy, rustfmt | mise (`mise.toml`, 1.99.0) | Build, lint and format the core | https://github.com/rust-lang/rust |
| mise | brew, then `mise install` | Pins every tool below for local work and CI | https://github.com/jdx/mise |
| just | mise | Task runner: `just check` is the merge gate | https://github.com/casey/just |
| cargo-fuzz | `cargo install cargo-fuzz`, runs on nightly | `cargo +nightly fuzz run parser` | https://github.com/rust-fuzz/cargo-fuzz |
| cc (clang or gcc) | system (Xcode Command Line Tools, distro package) | `just c-abi-test`: the C ABI test under ASan, UBSan and TSan (Unix only) | https://github.com/llvm/llvm-project |
| Xcode (Swift 6.4, SwiftPM, AppKit/SwiftUI SDK) | Mac App Store or developer.apple.com (27.0) | `just macos`, `just macos-test`: build and test the macOS app in `macos/` (arm64 only) | https://developer.apple.com/xcode/ |
| codesign | system (Xcode) | `just macos`: ad-hoc signs `target/macos/Scull.app` so it launches locally | https://developer.apple.com/documentation/security/code-signing-services |

## Bundled files

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| Symbols Nerd Font Mono 3.5.1 (MIT, `NerdFontsSymbolsOnly.tar.xz`) | local (`macos/Resources/`) | https://github.com/ryanoasis/nerd-fonts/releases/tag/v3.5.1 | Powerline separators and Nerd Font icons in shell prompts when the face lacks them |

## cargo

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| thiserror | local | https://github.com/dtolnay/thiserror | Error enum per library crate |
| cargo_metadata | local | https://github.com/oli-obk/cargo_metadata | Crate-graph test (`crates/scull-ffi/tests/deps.rs`) |
| libfuzzer-sys | local | https://github.com/rust-fuzz/libfuzzer | libFuzzer targets for the parser and the stub grid (`fuzz/`; nightly, not the CI gate) |
| cargo-nextest | global (mise) | https://github.com/nextest-rs/nextest | Test runner |
| proptest | local | https://github.com/proptest-rs/proptest | Property tests |
| anyhow | local | https://github.com/dtolnay/anyhow | Error type of the `scull-ucd-gen` binary |
| sha2 | local | https://github.com/RustCrypto/hashes | Verifies the pinned Unicode data files |
| ureq | local | https://github.com/algesten/ureq | Downloads the Unicode data files in `scull-ucd-gen` |
| memchr | local | https://github.com/BurntSushi/memchr | Bulk search for the end of DCS/APC payloads in scull-parser |
| divan | local | https://github.com/nvzqz/divan | Parser throughput bench and the stub baseline (`crates/scull-bench`) |
| vte | local | https://github.com/alacritty/vte | Baseline parser in the throughput bench (dev-dependency) |
| rustc-hash | local | https://github.com/rust-lang/rustc-hash | Fast, deterministic hash maps for the interning tables (clippy.toml bans std HashMap) |
| bitflags | local | https://github.com/bitflags/bitflags | Cell flags and SGR attribute bits in scull-grid; modifier and kitty flag sets in scull-input |
| portable-pty | local | https://github.com/wezterm/wezterm/tree/main/pty | PTY and ConPTY for scull-pty |
| parking_lot | local | https://github.com/Amanieu/parking_lot | Terminal lock with `unlock_fair` so a frame read is not starved |
| cbindgen | local | https://github.com/mozilla/cbindgen | Generates the committed C header `crates/scull-ffi/include/scull.h` (drift test, dev-dependency) |
| csbindgen | local | https://github.com/Cysharp/csbindgen | Generates the committed C# bindings `crates/scull-ffi/bindings/csharp/NativeMethods.g.cs` (drift test, dev-dependency) |
| base64 | local | https://github.com/marshallpierce/rust-base64 | Streaming decode of iTerm2 and kitty image payloads in scull-image |
| png | local | https://github.com/image-rs/image-png | PNG decode for iTerm2 images and kitty `f=100` in scull-image |
| zune-jpeg | local | https://github.com/etemesi254/zune-image | JPEG decode for iTerm2 images in scull-image |
| flate2 | local | https://github.com/rust-lang/flate2-rs | zlib inflate of kitty `o=z` payloads in scull-image |
| serde | local | https://github.com/serde-rs/serde | Derives the config file types in scull-config |
| toml | local | https://github.com/toml-rs/toml | Parses the config file in scull-config (the only module that does) |
| schemars | local | https://github.com/GREsau/schemars | JSON Schema of the config types, committed as `docs/config.schema.json` (dev-dependency) |
| serde_json | local | https://github.com/serde-rs/json | Renders and checks the config schema (dev-dependency) |
| toml_edit | local | https://github.com/toml-rs/toml | Edits a single setting in the config file in place, keeping comments (scull-config) |
| notify | local | https://github.com/notify-rs/notify | Watches the config directory for live reload (scull-config) |

## NuGet

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| Microsoft.Windows.CsWin32 | local from T17.2 (picked in T17.1, not referenced yet) | https://github.com/microsoft/CsWin32 | Direct3D 11, DXGI, DirectWrite, TSF and UIA from C# as blittable structs (`allowMarshaling: false`); the comparison is in `research.md` §6.1 |
