# T6. Grid and scrollback

`scull-grid`: a small fixed cell with interned clusters and styles, rare data out of line, O(1) blank and uniform lines, and one power-of-two ring of lazily allocated rows shared by the screen and scrollback. Rows carry a stable id and a generation. Done when scrolling is an offset change and memory per scrollback row is recorded in the benchmark.

Execution plan:
1. Read the ring storage of Alacritty (`alacritty_terminal` grid/storage) and the cell layouts recorded in `docs/research/`; port from upstream Alacritty or WezTerm only with notices kept.
2. `crates/scull-grid`: a fixed-size `Cell` (cluster id, style id, flags incl. wide/spacer), interning tables for clusters and styles with hard caps, rare data (hyperlink ids, underline colour) out of line, rows with a stable id and a generation, O(1) blank and uniform rows.
3. One power-of-two ring of lazily allocated rows shared by the screen and scrollback with a hard scrollback cap; scrolling is an offset change.
4. Tests for every cap and for scroll as offset change; a divan bench that records memory per scrollback row.
5. Verify with `just check`. Split at claim: T6.1 cells, interning and rows; T6.2 ring and scrollback; the agent reports which landed.
