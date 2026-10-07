# Scull

No repository yet.

A terminal emulator with a Rust core and a native UI per platform: SwiftUI on macOS, WinUI on Windows, joined by a C ABI. Rust computes, the platform renders. The design takes the best part of kitty, WezTerm, Alacritty, foot, Contour and Warp at each pipeline stage; see `research.md`.

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T3 | in progress | P0 | 3 | 80% | Cursor / grok 4.7 |
| T7 | in progress | P0 | 5 | 0% | Claude Code / claude-opus-5-5 |
| T9 | todo | P0 | 4 | 0% | |
| T10 | todo | P0 | 4 | 0% | |
| T11 | in progress | P1 | 4 | 0% | Claude Code / claude-opus-5-5 |
| T13 | todo | P1 | 3 | 0% | |
| T14 | todo | P1 | 3 | 0% | |
| T15 | todo | P1 | 5 | 0% | |
| T16 | todo | P1 | 4 | 0% | |
| T17 | todo | P1 | 5 | 0% | |
| T18 | todo | P2 | 5 | 0% | |
| T19 | todo | P2 | 5 | 0% | |
| T20 | todo | P2 | 3 | 0% | |
| T21 | todo | P2 | 3 | 0% | |
| T22 | todo | P2 | 2 | 0% | |
| T23 | todo | P3 | 2 | 0% | |
| T24 | todo | P2 | 2 | 0% | |
| T25 | todo | P2 | 2 | 0% | |
| T26 | todo | P3 | 2 | 0% | |

### T3. Conformance and benchmark harness

Built before the features so every later task lands with tests. Recorded stream → expected grid ref tests, golden frame snapshots, fuzz targets, and a throughput and latency benchmark that runs the same input through Scull and the six reference terminals. Done when the harness runs in CI against a stub core and the benchmark produces a baseline table for the reference terminals. "Better than all of them" is measured here, not claimed.

Execution plan:
1. Conformance is on main: `crates/scull-harness` stub grid, golden fixtures, an in-crate seed corpus and a separate `fuzz/` libFuzzer target. `just check` remains the CI gate.
2. Benchmark is on main: `crates/scull-bench` times `Stub` and writes `docs/benchmarks/baseline.md`. On this machine kitty, WezTerm, Alacritty, foot and Contour were not installed, and Warp has no headless stdin feed, so those rows have no throughput. The task stays open until the same input is timed on the six reference terminals.

### T7. Terminal state

`scull-term`: the handler for the typed actions. Cursor, SGR, modes, charsets, tab stops, scroll regions, left and right margins, alt screen, saved cursor, device reports. Done when the esctest and vttest subsets chosen in T3 pass.


Execution plan:
1. Pick the conformance subset: T3 has not chosen esctest and vttest cases yet, so T7 picks the subset that a headless core can check (recorded stream in, expected grid, cursor and replies out), lists it in `crates/scull-term/tests/README.md` with sources, and runs it through the `scull-harness` fixture format.
2. `crates/scull-term`: a `Terminal` that implements `scull_parser::Handler` over `scull_grid::Grid`, using `scull_unicode` for widths and grapheme clusters (mode 2027).
3. Split at claim. T7.1: print with wrap and wide characters, C0, cursor movement, ED/EL/ECH/ICH/DCH/IL/DL, SGR (16, 256, truecolour, underline styles and colour), scroll regions (DECSTBM), tab stops, IND/RI/NEL. T7.2: modes (ANSI and DEC private, DECRQM incl. 2027), charsets (G0-G3, DEC special graphics), left and right margins (DECLRMM, DECSLRM), alt screen 47/1047/1049, DECSC/DECRC, device reports (DA1, DA2, DSR, CPR) through a capped reply queue, RIS and DECSTR.
4. Verify with `just check`.
### T9. Damage, synchronized output, frame snapshot

Row dirty bits plus scroll damage; mode 2026 with byte and time caps; a UI-owned frame that copies only changed rows into flat buffers under a short lock, with text runs for platform shaping. Done when the property test holds: repainting dirty rows over the previous frame equals a full repaint.

### T10. C ABI

`scull-ffi`: opaque handles, the frame API, one wakeup callback, a polled event queue, status codes, `catch_unwind` on every export with per-terminal poisoning, an ABI version check and `struct_size` in every struct. The header and C# bindings are generated, committed and diff-checked in CI. Done when a C test program runs create, feed, update and free under the sanitizers from two threads.

### T11. Resize and reflow

One-pass reflow with tracking points for cursor, saved cursor, viewport and selection. The alt screen is not rewrapped. Interactive resize pauses the PTY and reflows once at the end. Done when no anchor is lost in the reflow golden tests.

Execution plan:
1. Read Alacritty's `grid/resize.rs` and WezTerm's rewrap for tracking-point handling; port only with notices kept.
2. `crates/scull-grid`: `Grid::resize(rows, cols, &mut [TrackPoint])` that rewraps the primary screen and scrollback in one pass, joining wrapped rows into logical lines and splitting them at the new width, keeping wide characters whole, and moving every tracking point (cursor, saved cursor, viewport top, selection ends) with its cell. A flag skips rewrap for the alt screen (truncate or pad only). Row ids survive where a row survives; new rows get new ids.
3. Pausing the PTY during interactive resize belongs to the FFI layer (T10); this task exposes the one-shot resize and documents the contract.
4. Golden tests: shrink and grow round trips, wide characters at the wrap edge, cursor on a wrapped line, points in scrollback, history cap during grow, alt-screen no-rewrap; a proptest that no tracking point is lost or moved outside the grid.
5. Verify with `just check`.

### T13. Selection, search, links, shell integration

Selection and scrollback search in the core. OSC 8 links as per-row ranges. OSC 52 as an event the UI may deny. OSC 7 and OSC 133. Title, bell and notification events. Done when each has a golden test and an event in the queue.

### T14. macOS app: first light

A SwiftUI shell with an `NSView` surface that draws frames with CoreText, sends key and mouse events, and resizes. Done when a shell is usable in one window.

### T15. macOS renderer

A Swift Metal renderer with a CoreText glyph atlas that has eviction, a shaped-run cache, ligatures, colour emoji, geometry-drawn box characters, and dirty-row uploads. Done when input latency and throughput are recorded against the T3 baseline.

### T16. macOS input method and accessibility

`NSTextInputClient` with preedit carried in the frame, and `NSAccessibility` text over the core's read-text calls. Done when CJK input and VoiceOver reading work.

### T17. Windows app and renderer

A C# WinUI 3 shell with a `SwapChainPanel` surface, a D3D11 renderer and a DirectWrite atlas. The first step evaluates the interop library that gives C# access to Direct3D and DirectWrite (maintenance, coverage, allocation cost per frame) and records the pick in `toolchain.md`; the research evaluated none. Done when a shell is usable and the T3 benchmark runs on Windows.

### T18. Windows input method and accessibility

TSF text input and a UIA text provider, written in C#. Windows Terminal's C++ implementation is the reference to read, not code to link. Done when CJK input and Narrator reading work.

### T19. Images

Sixel, kitty graphics and iTerm2 inline images decoded in the core to RGBA, kept as a positioned list with an image and placement split and a quota with LRU eviction. The frame carries placements; the platform uploads textures. Done when each protocol has golden tests and the decoders have fuzz targets.

### T20. Configuration, fonts and themes

A config file loaded and validated in the core, with font, colour scheme, scrollback and key binding settings, live reload, and a settings view on each platform. Done when a change applies without restart.

### T21. Tabs, splits and windows

Native tabs, split panes and multiple windows on both platforms over the same core handles. Done when a crashed terminal poisons one pane and the rest keep running.
