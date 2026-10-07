# Scull

https://github.com/pyrlyn/scull

A terminal emulator with a Rust core and a native UI per platform: SwiftUI on macOS, WinUI on Windows, joined by a C ABI. Rust computes, the platform renders. The design takes the best part of kitty, WezTerm, Alacritty, foot, Contour and Warp at each pipeline stage; see `research.md`.

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T3 | in progress | P0 | 3 | 80% | Cursor / grok 4.7 |
| T13 | todo | P1 | 3 | 0% | |
| T15 | in progress | P1 | 5 | 0% | Claude Code / claude-opus-5-5 |
| T16 | in progress | P1 | 4 | 0% | Claude Code / claude-opus-5-5 |
| T17 | todo | P1 | 5 | 0% | |
| T18 | todo | P2 | 5 | 0% | |
| T20 | in progress | P2 | 3 | 0% | Claude Code / claude-sonnet-5-5 |
| T21 | in progress | P2 | 3 | 0% | Claude Code / claude-sonnet-5-5 |
| T23 | todo | P3 | 2 | 0% | |
| T24 | todo | P2 | 2 | 0% | |
| T25 | todo | P2 | 2 | 0% | |
| T26 | todo | P3 | 2 | 0% | |

### T3. Conformance and benchmark harness

Built before the features so every later task lands with tests. Recorded stream → expected grid ref tests, golden frame snapshots, fuzz targets, and a throughput and latency benchmark that runs the same input through Scull and the six reference terminals. Done when the harness runs in CI against a stub core and the benchmark produces a baseline table for the reference terminals. "Better than all of them" is measured here, not claimed.

Execution plan:
1. Conformance is on main: `crates/scull-harness` stub grid, golden fixtures, an in-crate seed corpus and a separate `fuzz/` libFuzzer target. `just check` remains the CI gate.
2. Benchmark is on main: `crates/scull-bench` times `Stub` and writes `docs/benchmarks/baseline.md`. On this machine kitty, WezTerm, Alacritty, foot and Contour were not installed, and Warp has no headless stdin feed, so those rows have no throughput. The task stays open until the same input is timed on the six reference terminals.

### T13. Selection, search, links, shell integration

Selection and scrollback search in the core. OSC 8 links as per-row ranges. OSC 52 as an event the UI may deny. OSC 7 and OSC 133. Title, bell and notification events. Done when each has a golden test and an event in the queue.

### T15. macOS renderer

A Swift Metal renderer with a CoreText glyph atlas that has eviction, a shaped-run cache, ligatures, colour emoji, geometry-drawn box characters, and dirty-row uploads. Done when input latency and throughput are recorded against the T3 baseline.

Execution plan (split to fit the budget). All new code lives in `macos/Sources/ScullKit/Renderer/`; `TerminalView` and `TerminalSession` change only where the renderer plugs in, because T16 edits the same view.

- T15.1 Glyph atlas (`GlyphAtlas.swift`): one BGRA8 `MTLTexture` packed in shelves; CoreText rasterises a glyph of any font at the backing scale, mask glyphs as coverage and colour glyphs (emoji) as premultiplied colour, and only the new glyph's region is uploaded. Eviction drops the least recently used shelf that no frame still in flight has drawn; when nothing can be evicted the atlas doubles up to a cap and bumps its generation so the renderer rebuilds every row. Tests: packing, lookup reuse, eviction order, the in-flight guard, growth, colour detection.
- T15.2 Shaping (`RunShaper.swift`): a run's text shaped by CoreText with standard ligatures and system font fallback, so emoji come from the colour emoji font; every glyph is placed at the column of its cluster plus its offset inside the cluster, so fallback advances never drift off the core's grid. A bounded shaped-run cache keyed by text, face and width. Tests: a ligature font collapses `->`, emoji shape with a colour font, combining marks stay on their base cell, wide runs land two columns apart, the cache stays within its cap.
- T15.3 Sprites (`Sprites.swift`): box drawing (U+2500–U+257F), block elements and shades (U+2580–U+259F) and the underline kinds (single, double, curly, dotted, dashed) drawn from geometry into cell-sized atlas bitmaps, so lines meet across cells whatever the font. Tests: every code point in the ranges has a sprite, line arms reach the cell edges, shades have the right coverage.
- T15.4 Renderer (`MetalRenderer.swift`, `Shaders.swift`; the shader source is compiled at run time because SwiftPM does not build `.metal` files): instanced quads for backgrounds, decorations, glyphs and the cursor; per-row instance slots behind an indirection table, so a scroll from the frame's damage only remaps slots; dirty rows are rebuilt once and copied into each of the three in-flight buffers when that buffer is next used. `TerminalSession` accumulates the damage of every `tt_frame_update` (`FrameDamage.swift`) so no update between two draws is lost. `TerminalView` hosts a `CAMetalLayer`, and the debug snapshot renders offscreen through the same renderer. Tests: damage composition across updates and scrolls, an offscreen render of a fed frame checked at known pixels.
- T15.5 Image placements: textures cached by image and generation, drawn below or above the text by `z`.
- T15.6 Measurements: throughput (the T3 fixed 1,000,000-byte input fed through the real core, with frames updated and rendered offscreen) and input latency (key event to presented drawable through the shell's echo, `-ScullLatencyProbe`), written to `docs/benchmarks/macos-renderer.md` beside the T3 baseline.
- Verify: `just check`, `just macos`, `just macos-test`; a `-ScullSnapshot` PNG showing text, colours, wide characters, emoji, box drawing and the cursor drawn by Metal.

### T16. macOS input method and accessibility

`NSTextInputClient` with preedit carried in the frame, and `NSAccessibility` text over the core's read-text calls. Done when CJK input and VoiceOver reading work.

### T17. Windows app and renderer

A C# WinUI 3 shell with a `SwapChainPanel` surface, a D3D11 renderer and a DirectWrite atlas. The first step evaluates the interop library that gives C# access to Direct3D and DirectWrite (maintenance, coverage, allocation cost per frame) and records the pick in `toolchain.md`; the research evaluated none. Done when a shell is usable and the T3 benchmark runs on Windows.

### T18. Windows input method and accessibility

TSF text input and a UIA text provider, written in C#. Windows Terminal's C++ implementation is the reference to read, not code to link. Done when CJK input and Narrator reading work.

### T20. Configuration, fonts and themes

A config file loaded and validated in the core, with font, colour scheme, scrollback and key binding settings, live reload, and a settings view on each platform. Done when a change applies without restart.

### T21. Tabs, splits and windows

Native tabs, split panes and multiple windows on both platforms over the same core handles. Done when a crashed terminal poisons one pane and the rest keep running.

### T23. Guard the interner Marks/sweep contract against interleaved interns

`crates/scull-grid/src/intern.rs:27-32` silently ignores an out-of-range mark, and `sweep()` at `intern.rs:128-142` frees every unmarked slot — a caller that interns between `marks()` and `sweep()` gets live ids reclaimed (wrong styles/text, not UB). The only current call path (`grid.rs:345-360`) is safe today. Done means: `sweep` skips ids at or beyond the marks' length (or the lengths are asserted), so the hazard cannot resurface.

### T24. Reconcile harness eager-wrap with xterm's deferred DECAWM

`crates/scull-harness/src/lib.rs:87-92` wraps eagerly at the last column while xterm DECAWM defers the wrap to the next printable; the golden fixtures encode eager-wrap semantics and will diverge when T7 replays the same streams against the real core. Coordinate with the T7 owner before touching fixtures. Done means: the wrap semantics the fixtures assert are the ones the real core will have, or the divergence is documented as intentional for the stub.

### T25. CI: run the fuzz targets and the UCD stale-table check

The `fuzz/` workspace is excluded from CI (`Cargo.toml:4`) with a gitignored corpus, and the stale-table check silently skips without the `target/ucd` cache (`tools/scull-ucd-gen/src/main.rs:384-397`) while `just check` — the only CI gate — never runs `ucd-check`; a hand-edited `tables.rs` would go unnoticed. Done means: a CI job warms the cache and runs `ucd-check`, and a short smoke fuzz run executes on every push.

### T26. Close grid-layer test gaps

`LEADING_SPACER` (`crates/scull-grid/src/cell.rs:36-38`) has no test for the end-of-row wide-wrap path; `scull-bench` measures a single pass (`src/lib.rs:56-62`) so baseline numbers are noisy. Done means: both covered or averaged.
