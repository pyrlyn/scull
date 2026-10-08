# Scull

https://github.com/pyrlyn/scull

A terminal emulator with a Rust core and a native UI per platform: SwiftUI on macOS, WinUI on Windows, joined by a C ABI. Rust computes, the platform renders. The design takes the best part of kitty, WezTerm, Alacritty, foot, Contour and Warp at each pipeline stage; see `research.md`.

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T3 | in progress | P0 | 3 | 80% | Cursor / grok 4.7 |
| T13 | todo | P1 | 3 | 0% | |
| T15 | in progress | P1 | 5 | 90% | Claude Code / claude-opus-5-5 |
| T16 | in progress | P1 | 4 | 85% | Claude Code / claude-opus-5-5 |
| T17 | in progress | P1 | 5 | 20% | Claude Code / claude-opus-5-5 |
| T18 | todo | P2 | 5 | 0% | |
| T20 | in progress | P2 | 3 | 75% | Claude Code / claude-sonnet-5-5 |
| T21 | in progress | P2 | 3 | 60% | Claude Code / claude-sonnet-5-5 |
| T24 | todo | P2 | 2 | 0% | |
| T25 | todo | P2 | 2 | 0% | |

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

Landed on the branch: T15.1–T15.6, the preedit underline from `tt_frame_view.preedit`, atlas rebuild on a config font change, the crash notice over the Metal layer. Throughput and the key-to-rendered-frame latency are in `docs/benchmarks/macos-renderer.md` (`just macos-bench`). Left: the key-to-screen line, which needs an uncovered window during the probe run; the T3 reference terminals, which are not installed (T3); Powerline and Nerd Font private-use glyphs, which the system font lacks.

### T16. macOS input method and accessibility

`NSTextInputClient` with preedit carried in the frame, and `NSAccessibility` text over the core's read-text calls. Done when CJK input and VoiceOver reading work.

Execution plan (split to fit the budget):

- T16.1 Preedit in the frame and `NSTextInputClient`. The frame (`scull-term`) holds the preedit text and caret the UI gives it, capped in size, and on update lays it over the cursor row: clusters and widths from `scull-unicode` with the terminal's width options, shifted left to fit the row, the cursor moved to the caret, and the row's overlay hash changed so damage repaints it and scroll damage never moves it. So any renderer draws composing text with no code of its own; `tt_frame_view.preedit` (row, col, cols) tells it where to underline. `scull-ffi` exports `tt_frame_preedit`; ABI minor bump, header and C# bindings regenerated. In `macos/`, `TextInput` (marked text, selection, what a key event turned into) and a `TerminalView` extension conforming to `NSTextInputClient` in new files; `keyDown` asks it whether the input method took the key, and committed text goes through `tt_term_text`. The CoreText pass underlines the preedit.
- T16.2 Accessibility. `Terminal::read_text` gives viewport rows as text, one line per row, trailing blanks trimmed, hidden (SGR 8) cells blank; `tt_term_read_text` copies it into a caller buffer, answering the needed length when it is short. A `TerminalView` extension in a new file makes the view an `NSAccessibility` text area over it: value, line for index, range for line, string and frame for range, insertion point at the cursor, `valueChanged` posted on new output while VoiceOver runs.
- Verify: Rust tests for every new export, cap and overlay case; `just check`, `just c-abi-test`, `just macos`, `just macos-test` with Swift tests for marked ranges, `insertText` after preedit, dead-key composition and the accessibility text model; a `-ScullSnapshot` PNG with a CJK preedit set by a debug hook. A real IME candidate window and VoiceOver speech need a human.

### T17. Windows app and renderer

A C# WinUI 3 shell with a `SwapChainPanel` surface, a D3D11 renderer and a DirectWrite atlas. The first step evaluates the interop library that gives C# access to Direct3D and DirectWrite (maintenance, coverage, allocation cost per frame) and records the pick in `toolchain.md`; the research evaluated none. Done when a shell is usable and the T3 benchmark runs on Windows.

Execution plan (split to fit the budget). C# lives in `windows/`; the core is reached only through the committed csbindgen bindings, compiled in by link.

- T17.1 Interop pick and the managed core: compare Vortice.Windows, TerraFX.Interop.Windows, Microsoft.Windows.CsWin32 and Silk.NET for Direct3D 11, DXGI, DirectWrite, TSF and UIA (maintenance from NuGet and GitHub releases, coverage, per-frame allocations, AOT and trimming, licence), recorded in `research.md` with sources and the pick in `toolchain.md`. `windows/Scull.Core`, a platform-neutral class library over `NativeMethods.g.cs`: `Terminal` (create, feed, resize, events and bell, frame update and view, read text), `SafeHandle` ownership so each handle is freed once, the ABI version checked at start-up, status codes mapped to exceptions, poisoning tracked as `TerminalSession.swift` does. `windows/Scull.Core.Tests` loads the real Rust library and covers each of those. `just windows-core-test`, and a CI job on Windows and macOS. Verify: `just check`, `just windows-core-test`, `dotnet build` with warnings as errors.
- T17.2 WinUI 3 shell: an unpackaged app with a `SwapChainPanel`, a D3D11 renderer (instanced quads for backgrounds, glyphs, decorations and the cursor, per-row slots driven by the frame's damage) and a DirectWrite glyph atlas with eviction, through the T17.1 interop pick; `tt_term_spawn` with the wakeup posted to the dispatcher queue.
- T17.3 Input, resize and measurements: keys, text, mouse, wheel, paste and focus mapped to the exports; `resize_begin`/`resize` on a drag; the T3 benchmark input timed through the renderer on Windows, written beside the macOS numbers.

Landed: T17.1. The pick is Microsoft.Windows.CsWin32 with `allowMarshaling: false` (`research.md` §6.1, `toolchain.md`); it is not referenced yet. `windows/Scull.Core` wraps the linked bindings in `Terminal` (SafeHandle ownership that frees once and never during a call, the ABI check before the first handle, status codes as `ScullException`s, poisoning tracked, the frame as zero-copy spans), and `windows/Scull.Core.Tests` runs 16 MSTest tests against the real library with `just windows-core-test`, also a CI step on all three runners. Left for later slices: `tt_term_spawn` with the wakeup (T17.2), the input exports (T17.3), image placements in the view (T17.2) and preedit (T18).

### T18. Windows input method and accessibility

TSF text input and a UIA text provider, written in C#. Windows Terminal's C++ implementation is the reference to read, not code to link. Done when CJK input and Narrator reading work.

### T20. Configuration, fonts and themes

A config file loaded and validated in the core, with font, colour scheme, scrollback and key binding settings, live reload, and a settings view on each platform. Done when a change applies without restart.

Execution plan (split to fit the budget):
- T20.1. `crates/scull-config` model: `Config` TOML types (font family and size, colour scheme with overrides, scrollback), defaults, built-in schemes, load with a size cap and `deny_unknown_fields`, errors that name the file, JSON Schema committed to `docs/config.schema.json` with a stale-schema test. Verify: unit tests for parsing, defaults, every validation error and caps.
- T20.2. Key bindings in `scull-config`: `[[keybind]]` chord parsing over `scull_input::{Key, Modifiers}`, an action enum, default bindings merged with the user's, caps and duplicate checks. Verify: parsing and rejection tests.
- T20.3. Editing and live reload in `scull-config`: `toml_edit` setter that validates before it writes and replaces the file atomically; `notify` watcher on the parent directory feeding a store with a generation counter and the last error (a bad edit keeps the last good settings). Verify: tests that edit a temp file and see the generation and values change, and a broken edit keep the old settings.
- T20.4. C ABI in `scull-ffi`: `tt_config` handle (`new`, `poll` into a sized `tt_config_view`, `set`, `free`) behind the wakeup callback only; bump the ABI minor; `just bindings`; extend the C test under ASan and TSan. Verify: Rust tests, `just c-abi-test`, `just bindings-check`.
- T20.5. macOS: new `ScullKit/Config.swift` wrapper and `Palette`/`TerminalView` reading it (font, colours, key bindings, scrollback for new terminals); live reload through the wakeup. Verify: `just macos`, `just macos-test`, and the `-ScullSnapshot` PNG before and after editing the file while the app runs.
- T20.6. macOS settings view: new SwiftUI `Settings` scene in new files (family, size, scheme, scrollback) that writes through `tt_config_set`. Verify: `just macos`, a snapshot after a change made through the view.
- T20.7. Windows settings view. Waiting for T17; it reuses T20.1 to T20.4 unchanged.
- Scrollback note: the grid ring is sized when a terminal is created, so a changed scrollback applies to terminals opened afterwards, not to running ones.

### T21. Tabs, splits and windows

Native tabs, split panes and multiple windows on both platforms over the same core handles. Done when a crashed terminal poisons one pane and the rest keep running.

Execution plan (split to fit the budget):

- T21.1 Pane tree and poison state (macOS, `macos/`): a pure value type `PaneTree` in `Sources/ScullKit/PaneTree.swift` (leaf or split of two subtrees, an axis, a focused pane; split, close, focus next and previous, no UI types). `TerminalSession` records `TT_POISONED` and `TT_PANIC` from any call as `isPoisoned`, and `TerminalView` draws a "this terminal crashed" notice instead of the grid for a poisoned session. The pane tree is the same shape the Windows app will lay out as viewports. Verify: Swift tests for split, close and focus order; a test that a poisoned handle leaves sibling sessions answering `TT_OK`, using a `test-hooks` Cargo feature on `scull-ffi` that exports `tt_term_test_panic` (not in the header, not in the shipped library) and a `just macos-test` that builds the library with it.
- T21.2 Panes, tabs and windows on macOS: `Sources/ScullKit/PaneHostView.swift` (an `NSView` that lays the tree out with nested `NSSplitView`s and keeps one `TerminalView` per pane, so a split never restarts a shell; a child exit closes its pane, the last pane closes the window) and a `PaneSurface` representable; the app becomes a `WindowGroup` (native window tabs, one pane tree and one set of sessions per window or tab) with File commands New Window, New Tab, Split Right, Split Down, Close Pane and Next or Previous Pane, routed through the responder chain. `TerminalView` gets only a child-exit callback, per-pane focus (first responder, not key window) and the poison notice; its draw path is untouched because T15 replaces it. Verify: `just check`, `just test`, `just c-abi-test`, `just macos`, `just macos-test`; the debug snapshot (`-ScullSplit` with `-ScullHostSnapshot`) shows two panes side by side; a second window and a tab open from the menu.
- T21.3 Windows tabs, splits and windows (waits for T17, which creates the WinUI app): the same `PaneTree` semantics in C# over `Scull.Core` handles, WinUI `TabView` for tabs and one window per `AppWindow`. One swap chain per window: panes are viewports of that swap chain, not separate swap chains, per AGENTS.md. A poisoned handle poisons one viewport. Verify: the same pane-tree tests in C#, and a poisoned handle leaving sibling panes running.

### T24. Reconcile harness eager-wrap with xterm's deferred DECAWM

`crates/scull-harness/src/lib.rs:87-92` wraps eagerly at the last column while xterm DECAWM defers the wrap to the next printable; the golden fixtures encode eager-wrap semantics and will diverge when T7 replays the same streams against the real core. Coordinate with the T7 owner before touching fixtures. Done means: the wrap semantics the fixtures assert are the ones the real core will have, or the divergence is documented as intentional for the stub.

### T25. CI: run the fuzz targets and the UCD stale-table check

The `fuzz/` workspace is excluded from CI (`Cargo.toml:4`) with a gitignored corpus, and the stale-table check silently skips without the `target/ucd` cache (`tools/scull-ucd-gen/src/main.rs:384-397`) while `just check` — the only CI gate — never runs `ucd-check`; a hand-edited `tables.rs` would go unnoticed. Done means: a CI job warms the cache and runs `ucd-check`, and a short smoke fuzz run executes on every push.
