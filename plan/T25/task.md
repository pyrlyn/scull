# T25. CI: run the fuzz targets and the UCD stale-table check

The `fuzz/` workspace is excluded from the main Cargo workspace, and the stale-table test skips when `target/ucd` is missing, while `just check` never runs `ucd-check`. A hand-edited `tables.rs` would pass CI. Done means: a CI job warms the Unicode cache and runs `ucd-check`, and a short smoke fuzz run executes on every push.

Execution plan:
1. A `ucd` job on the existing CI triggers runs `just ucd-check`. The generator downloads missing pinned Unicode files into `target/ucd` and exits non-zero when `tables.rs` or the grapheme fixture differ. The in-crate test still skips without that cache; this job does not. Not folded into `just check`, so an offline local gate stays offline.
2. A `fuzz` job on Ubuntu follows the coding agent `nightly.yml` (rustup nightly, `cargo install cargo-fuzz --locked`, `cargo fuzz run`) and the token-reduction CLI and the package manager `just fuzz` loop, shortened to `-max_total_time=5` per target via `just fuzz-smoke`. Nightly is `cargo +nightly` only; mise stays on 1.99. `cargo-fuzz` 0.13.2, the version already installed here and pinned by the package manager.
3. Verify locally: warm `target/ucd` with `just ucd-check` (the pinned files are a few megabytes), confirm a drifted table fails, and run `just fuzz-smoke` on the nightly toolchain already on this machine. GitHub Actions itself cannot be proved from here.
