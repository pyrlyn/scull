# T9. Damage, synchronized output, frame snapshot

Row dirty bits plus scroll damage; mode 2026 with byte and time caps; a UI-owned frame that copies only changed rows into flat buffers under a short lock, with text runs for platform shaping. Done when the property test holds: repainting dirty rows over the previous frame equals a full repaint.

Execution plan (split to fit the 500-line budget):

- T9.1 Damage and synchronized output in `scull-term`. Dirty rows come from the existing per-row generation counters in `scull-grid` (no new dirty bits unless a counter cannot express it); scrolls are recorded as scroll damage (region, count) so the UI can blit instead of repainting. Mode 2026 (BSU/ESU via `CSI ? 2026 h/l` and DECRQM) holds damage back, released by ESU, a byte cap or a time cap; caps are named constants, the clock is injected so tests are deterministic.
- T9.2 Frame snapshot in a new `frame` module of `scull-term`. A UI-owned `Frame` with flat per-row buffers (codepoints or cluster ids, style ids, widths) and text runs split on style and width for platform shaping. `Terminal::update_frame(&mut Frame)` copies only rows whose generation changed, under one short borrow, and applies scroll damage by moving rows inside the frame.
- Verify: unit tests per module, and a proptest that feeds random byte streams in chunks and checks that repainting dirty rows over the previous frame equals a full repaint after every chunk. Gate: `just check` exit 0. Grid changes stay minimal because T11 is editing `scull-grid` in parallel.
