# Toolchain

## Programs

| Program | How to install | Why here | Source |
| --- | --- | --- | --- |
| rustc, cargo, clippy, rustfmt | mise (`mise.toml`, 1.99.0) | Build, lint and format the core | https://github.com/rust-lang/rust |
| mise | brew, then `mise install` | Pins every tool below for local work and CI | https://github.com/jdx/mise |
| just | mise | Task runner: `just check` is the merge gate | https://github.com/casey/just |

## cargo

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| thiserror | local | https://github.com/dtolnay/thiserror | Error enum per library crate |
| cargo_metadata | local | https://github.com/oli-obk/cargo_metadata | Crate-graph test (`crates/scull-ffi/tests/deps.rs`) |
| libfuzzer-sys | local | https://github.com/rust-fuzz/libfuzzer | libFuzzer target for the stub grid (`fuzz/`; nightly, not the CI gate) |
| cargo-nextest | global (mise) | https://github.com/nextest-rs/nextest | Test runner |
| divan | local | https://github.com/nvzqz/divan | Default benchmark harness for the stub feed (`crates/scull-bench`) |
| proptest | local | https://github.com/proptest-rs/proptest | Property tests |
| anyhow | local | https://github.com/dtolnay/anyhow | Error type of the `scull-ucd-gen` binary |
| sha2 | local | https://github.com/RustCrypto/hashes | Verifies the pinned Unicode data files |
| ureq | local | https://github.com/algesten/ureq | Downloads the Unicode data files in `scull-ucd-gen` |
