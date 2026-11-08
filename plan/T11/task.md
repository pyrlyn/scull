# T11. Resize and reflow

One-pass reflow with tracking points for cursor, saved cursor, viewport and selection. The alt screen is not rewrapped. Interactive resize pauses the PTY and reflows once at the end. Done when no anchor is lost in the reflow golden tests.

Execution plan:
1. Read Alacritty's `grid/resize.rs` and WezTerm's rewrap for tracking-point handling; port only with notices kept.
2. `crates/scull-grid`: `Grid::resize(rows, cols, &mut [TrackPoint])` that rewraps the primary screen and scrollback in one pass, joining wrapped rows into logical lines and splitting them at the new width, keeping wide characters whole, and moving every tracking point (cursor, saved cursor, viewport top, selection ends) with its cell. A flag skips rewrap for the alt screen (truncate or pad only). Row ids survive where a row survives; new rows get new ids.
3. Pausing the PTY during interactive resize belongs to the FFI layer (T10); this task exposes the one-shot resize and documents the contract.
4. Golden tests: shrink and grow round trips, wide characters at the wrap edge, cursor on a wrapped line, points in scrollback, history cap during grow, alt-screen no-rewrap; a proptest that no tracking point is lost or moved outside the grid.
5. Verify with `just check`.
