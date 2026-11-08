# T7. Terminal state

`scull-term`: the handler for the typed actions. Cursor, SGR, modes, charsets, tab stops, scroll regions, left and right margins, alt screen, saved cursor, device reports. Done when the esctest and vttest subsets chosen in T3 pass.

Execution plan:
1. Pick the conformance subset: T3 has not chosen esctest and vttest cases yet, so T7 picks the subset that a headless core can check (recorded stream in, expected grid, cursor and replies out), lists it in `crates/scull-term/tests/README.md` with sources, and runs it through the `scull-harness` fixture format.
2. `crates/scull-term`: a `Terminal` that implements `scull_parser::Handler` over `scull_grid::Grid`, using `scull_unicode` for widths and grapheme clusters (mode 2027).
3. Split at claim. T7.1: print with wrap and wide characters, C0, cursor movement, ED/EL/ECH/ICH/DCH/IL/DL, SGR (16, 256, truecolour, underline styles and colour), scroll regions (DECSTBM), tab stops, IND/RI/NEL. T7.2: modes (ANSI and DEC private, DECRQM incl. 2027), charsets (G0-G3, DEC special graphics), left and right margins (DECLRMM, DECSLRM), alt screen 47/1047/1049, DECSC/DECRC, device reports (DA1, DA2, DSR, CPR) through a capped reply queue, RIS and DECSTR.
4. Verify with `just check`.
