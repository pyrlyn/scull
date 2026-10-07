# Toolchain

## Programs

| Program | How to install | Why here | Source |
| --- | --- | --- | --- |
| rustc, cargo, clippy, rustfmt | mise (`mise.toml`, 1.99.0) | Build, lint and format the core | https://github.com/rust-lang/rust |
| mise | brew, then `mise install` | Pins every tool below for local work and CI | https://github.com/jdx/mise |
| just | mise | Task runner: `just check` is the merge gate | https://github.com/casey/just |
| cargo-fuzz | `cargo install cargo-fuzz`, runs on nightly | `cargo +nightly fuzz run parser` | https://github.com/rust-fuzz/cargo-fuzz |

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
