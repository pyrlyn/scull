# Scull

No repository yet.

A terminal emulator with a Rust core and a native UI per platform: SwiftUI on macOS, WinUI on Windows, joined by a C ABI. Rust computes, the platform renders. The design takes the best part of kitty, WezTerm, Alacritty, foot, Contour and Warp at each pipeline stage; see `research.md`.

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T3 | in progress | P0 | 3 | 50% | Cursor / grok 4.7 |
| T4 | in progress | P0 | 4 | 0% | Claude Code / claude-opus-5-5 |
| T6 | todo | P0 | 5 | 0% | |
| T7 | todo | P0 | 5 | 0% | |
| T8 | todo | P0 | 3 | 0% | |
| T9 | todo | P0 | 4 | 0% | |
| T10 | todo | P0 | 4 | 0% | |
| T11 | todo | P1 | 4 | 0% | |
| T12 | todo | P1 | 4 | 0% | |
| T13 | todo | P1 | 3 | 0% | |
| T14 | todo | P1 | 3 | 0% | |
| T15 | todo | P1 | 5 | 0% | |
| T16 | todo | P1 | 4 | 0% | |
| T17 | todo | P1 | 5 | 0% | |
| T18 | todo | P2 | 5 | 0% | |
| T19 | todo | P2 | 5 | 0% | |
| T20 | todo | P2 | 3 | 0% | |
| T21 | todo | P2 | 3 | 0% | |

### T3. Conformance and benchmark harness

Built before the features so every later task lands with tests. Recorded stream → expected grid ref tests, golden frame snapshots, fuzz targets, and a throughput and latency benchmark that runs the same input through Scull and the six reference terminals. Done when the harness runs in CI against a stub core and the benchmark produces a baseline table for the reference terminals. "Better than all of them" is measured here, not claimed.

Execution plan:
1. Conformance, on `t3-conformance`: `crates/scull-harness` is a stub grid so a recorded stream, golden fixtures, an in-crate seed corpus and a separate `fuzz/` libFuzzer target can run before the parser and grid exist. The existing `just check` workflow stays the CI gate.
2. Benchmark, on the other branch: `crates/scull-bench` and `docs/benchmarks/`. Not this branch.

### T4. Parser

`scull-parser`: a bulk scanner that delivers text up to the next control byte as one run, in front of a table state machine for CSI, OSC, DCS and APC, emitting typed actions. Payloads are dispatched in place with hard length caps. Done when the fuzz target runs clean and throughput on plain text beats `vte` in the T3 benchmark.


Execution plan:
1. Pick the state machine: `vtparse` behind our scanner (research.md §6) or our own table after Paul Williams' DEC parser; record why in the commit.
2. `crates/scull-parser`: bulk scanner (printable UTF-8 runs up to the next control byte as one `&str`, split UTF-8 kept across `feed` calls), CSI with colon sub-parameters, ESC, OSC, DCS and APC delivered in place to a handler trait; hard caps on parameters, intermediates and payload lengths, each with a test.
3. `fuzz/` workspace (cargo-fuzz, nightly) with a parser target; run it clean locally.
4. A divan bench in `crates/scull-parser/benches` against `vte` on plain text and mixed SGR output.
5. Verify with `just check`. Over the 500-line budget, stop at a coherent boundary and split the rest into T4.x.
### T6. Grid and scrollback

`scull-grid`: a small fixed cell with interned clusters and styles, rare data out of line, O(1) blank and uniform lines, and one power-of-two ring of lazily allocated rows shared by the screen and scrollback. Rows carry a stable id and a generation. Done when scrolling is an offset change and memory per scrollback row is recorded in the benchmark.

### T7. Terminal state

`scull-term`: the handler for the typed actions. Cursor, SGR, modes, charsets, tab stops, scroll regions, left and right margins, alt screen, saved cursor, device reports. Done when the esctest and vttest subsets chosen in T3 pass.

### T8. PTY and I/O thread

`scull-pty`: a reader thread per terminal on top of a PTY crate, bounded work per lock hold, `try_lock` with back-pressure, replies written from the same thread, child exit through the same loop. ConPTY on Windows. Done when a flooding child cannot starve a frame read and a shell runs on both platforms.

### T9. Damage, synchronized output, frame snapshot

Row dirty bits plus scroll damage; mode 2026 with byte and time caps; a UI-owned frame that copies only changed rows into flat buffers under a short lock, with text runs for platform shaping. Done when the property test holds: repainting dirty rows over the previous frame equals a full repaint.

### T10. C ABI

`scull-ffi`: opaque handles, the frame API, one wakeup callback, a polled event queue, status codes, `catch_unwind` on every export with per-terminal poisoning, an ABI version check and `struct_size` in every struct. The header and C# bindings are generated, committed and diff-checked in CI. Done when a C test program runs create, feed, update and free under the sanitizers from two threads.

### T11. Resize and reflow

One-pass reflow with tracking points for cursor, saved cursor, viewport and selection. The alt screen is not rewrapped. Interactive resize pauses the PTY and reflows once at the end. Done when no anchor is lost in the reflow golden tests.

### T12. Input encoding

`scull-input`: pure encoders for legacy keys, the kitty keyboard protocol with a flag stack per screen, win32-input-mode, all mouse modes including SGR-pixel, bracketed paste with end-marker stripping, and focus events. Done when the table-driven tests match xterm and the kitty specification.

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
