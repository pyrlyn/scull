---
status: in progress
priority: P1
complexity: 5
readiness: 90%
agent: Claude Code / claude-opus-5-5
---

# T15. macOS renderer

A Swift Metal renderer with a CoreText glyph atlas that has eviction, a shaped-run cache, ligatures, colour emoji, geometry-drawn box characters, and dirty-row uploads. Done when input latency and throughput are recorded against the T3 baseline.

Execution plan (split to fit the budget). All new code lives in `macos/Sources/ScullKit/Renderer/`; `TerminalView` and `TerminalSession` change only where the renderer plugs in, because T16 edits the same view.

- T15.1 Glyph atlas (`GlyphAtlas.swift`): one BGRA8 `MTLTexture` packed in shelves; CoreText rasterises a glyph of any font at the backing scale, mask glyphs as coverage and colour glyphs (emoji) as premultiplied colour, and only the new glyph's region is uploaded. Eviction drops the least recently used shelf that no frame still in flight has drawn; when nothing can be evicted the atlas doubles up to a cap and bumps its generation so the renderer rebuilds every row. Tests: packing, lookup reuse, eviction order, the in-flight guard, growth, colour detection.
- T15.2 Shaping (`RunShaper.swift`): a run's text shaped by CoreText with standard ligatures and system font fallback, so emoji come from the colour emoji font; every glyph is placed at the column of its cluster plus its offset inside the cluster, so fallback advances never drift off the core's grid. A bounded shaped-run cache keyed by text, face and width. Tests: a ligature font collapses `->`, emoji shape with a colour font, combining marks stay on their base cell, wide runs land two columns apart, the cache stays within its cap.
- T15.3 Sprites (`Sprites.swift`): box drawing (U+2500–U+257F), block elements and shades (U+2580–U+259F) and the underline kinds (single, double, curly, dotted, dashed) drawn from geometry into cell-sized atlas bitmaps, so lines meet across cells whatever the font. Tests: every code point in the ranges has a sprite, line arms reach the cell edges, shades have the right coverage.
- T15.4 Renderer (`MetalRenderer.swift`, `Shaders.swift`; the shader source is compiled at run time because SwiftPM does not build `.metal` files): instanced quads for backgrounds, decorations, glyphs and the cursor; per-row instance slots behind an indirection table, so a scroll from the frame's damage only remaps slots; dirty rows are rebuilt once and copied into each of the three in-flight buffers when that buffer is next used. `TerminalSession` accumulates the damage of every `tt_frame_update` (`FrameDamage.swift`) so no update between two draws is lost. `TerminalView` hosts a `CAMetalLayer`, and the debug snapshot renders offscreen through the same renderer. Tests: damage composition across updates and scrolls, an offscreen render of a fed frame checked at known pixels.
- T15.5 Image placements: textures cached by image and generation, drawn below or above the text by `z`.
- T15.6 Measurements: throughput (the T3 fixed 1,000,000-byte input fed through the real core, with frames updated and rendered offscreen) and input latency (key event to presented drawable through the shell's echo, `-ScullLatencyProbe`), written to `docs/benchmarks/macos-renderer.md` beside the T3 baseline.
- Verify: `just check`, `just macos`, `just macos-test`; a `-ScullSnapshot` PNG showing text, colours, wide characters, emoji, box drawing and the cursor drawn by Metal.

Landed on the branch: T15.1–T15.6, the preedit underline from `tt_frame_view.preedit`, atlas rebuild on a config font change, the crash notice over the Metal layer. Throughput and the key-to-rendered-frame latency are in `docs/benchmarks/macos-renderer.md` (`just macos-bench`). Left: the key-to-screen line, which needs an uncovered window during the probe run; the T3 reference terminals, which are not installed (T3). Powerline and Nerd Font private-use glyphs are no longer left: the separators U+E0B0–U+E0BF are drawn from cell geometry (T28 in `done.md`) and the other icons come from the bundled Symbols Nerd Font Mono fallback (T27 in `done.md`).
