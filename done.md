# Done

### T1. Settle the open decisions

The creator answered the seven questions in `research.md` §8: "Wrapp" is Warp; the Windows host is all C#; the VT core is our own Rust code, not libghostty-vt; the PTY lives in the core; the minimum macOS is 26; the licence is `GPL-3.0-or-later`; Windows uses one swap chain per window. Each answer is recorded in `AGENTS.md`.

### T2. Workspace scaffold

Cargo workspace with the seven crates of `research.md` §7, a pinned toolchain, clippy with `-D warnings`, a disallowed-types list, rustfmt, CI, and `toolchain.md` filled in. Done when an empty workspace builds and lints clean on macOS and Windows.

Delivered: seven `publish = false` crates under `crates/`, Rust 1.99.0 pinned in `mise.toml`, workspace lints (no unwrap/expect/panic/print outside tests), `clippy.toml` disallowed types and methods, `rustfmt.toml`, `just check` as the merge gate, a CI matrix on macOS arm64, Windows and Linux, and a crate-graph test in `crates/scull-ffi/tests/deps.rs`.
