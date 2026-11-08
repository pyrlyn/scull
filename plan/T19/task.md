# T19. Images

Sixel, kitty graphics and iTerm2 inline images decoded in the core to RGBA, kept as a positioned list with an image and placement split and a quota with LRU eviction. The frame carries placements; the platform uploads textures. Done when each protocol has golden tests and the decoders have fuzz targets.

Execution plan (split to fit the 500-line budget; this run does T19.1–T19.3, T19.4 waits for T9):

- T19.1 New leaf crate `scull-image` (no workspace dependencies): `Image` (RGBA8, width, height, generation) behind an `Arc`, `Placement` (image id, cell anchor, cell size, pixel crop, z-index) kept apart from images, and an `ImageStore` with a byte quota and LRU eviction of images that no placement uses. iTerm2 inline images (OSC 1337 `File=` with base64 payload) decoded through maintained crates (`base64`, and `png`/`zune-jpeg`/`image` or similar, whichever is best maintained and lightest), with size caps checked before allocation.
- T19.2 Sixel decoder: streaming `put` of DCS bytes into an RGBA buffer, palette and raster attributes, with width, height and colour caps. Reuse a maintained crate if one fits; otherwise write it and say why in the commit.
- T19.3 Kitty graphics protocol (APC `G`): key parsing, chunked transmission (`m=1`), formats 24/32/100, zlib via `flate2`, transmit/put/delete actions, quiet levels, and the reply strings, as a pure decoder over the store. kitty is GPL-3.0-only: implement from the protocol spec, never port its code.
- T19.4 (after T9) Wire `scull-image` into `scull-term` (DCS/APC/OSC routing, placement on cursor, scroll and reflow re-anchoring, alt-screen clearing) and into the frame as placements.
- Verify: golden tests per protocol, proptests on the store's quota and LRU, a fuzz target per decoder in `fuzz/`, crate graph test updated, `just check` exit 0.
