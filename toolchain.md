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
| cargo-nextest | global (mise) | https://github.com/nextest-rs/nextest | Test runner |
