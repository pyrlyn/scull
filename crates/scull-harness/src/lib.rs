//! Conformance harness: a fixed text grid that a recorded byte stream can be
//! checked against before the real parser and grid exist. Separate from those
//! crates so CI can gate the harness while they are still empty.

/// Stand-in for the terminal core. The grid size is fixed at construction so
/// later tasks can hang the same assertions on the real grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stub {
    cols: u16,
    rows: u16,
    cursor_col: u16,
    /// `rows` means the cursor wrapped off the last row. Printable bytes stop
    /// there so the stub never scrolls; newline still clamps onto the last row.
    cursor_row: u16,
    /// Last column was just written: the next printable wraps first
    /// (xterm DECAWM, DEC STD 070 "last column flag").
    pending_wrap: bool,
    grid: Vec<Vec<char>>,
}

impl Stub {
    /// Empty grid of spaces, cursor at column 0 of row 0.
    ///
    /// A zero axis is raised to 1: a grid with no cells cannot host a cursor,
    /// and every later test assumes at least one.
    #[must_use]
    pub fn new(cols: u16, rows: u16) -> Self {
        let cols = cols.max(1);
        let rows = rows.max(1);
        let grid = vec![vec![' '; usize::from(cols)]; usize::from(rows)];
        Self {
            cols,
            rows,
            cursor_col: 0,
            cursor_row: 0,
            pending_wrap: false,
            grid,
        }
    }

    /// Write `bytes` into the grid.
    ///
    /// Printable ASCII (`0x20..=0x7E`) lands at the cursor. Filling the last
    /// column arms a wrap and leaves the cursor there; only the next
    /// printable wraps, to column 0 of the next row (xterm DECAWM, the same
    /// rule as `scull-term`). Past the last row those bytes are dropped: the
    /// stub does not scroll. `\n` and `\r` clear a pending wrap instead of
    /// wrapping. `\n` goes to column 0 of the next row and stays on the last
    /// row. `\r` returns to column 0 of the same row. Any other byte is
    /// ignored so a capture can contain controls the stub does not understand.
    pub fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            match byte {
                b'\n' => {
                    self.pending_wrap = false;
                    self.cursor_col = 0;
                    let last = self.rows.saturating_sub(1);
                    self.cursor_row = self.cursor_row.saturating_add(1).min(last);
                }
                b'\r' => {
                    self.pending_wrap = false;
                    self.cursor_col = 0;
                }
                0x20..=0x7E => self.write_printable(char::from(byte)),
                _ => {}
            }
        }
    }

    /// Rows of the grid, each `cols` characters long. Borrowed so a test can
    /// compare a frame without cloning it.
    #[must_use]
    pub fn grid(&self) -> &[Vec<char>] {
        &self.grid
    }

    /// Width in columns. Unchanged by [`Self::feed`].
    #[must_use]
    pub const fn cols(&self) -> u16 {
        self.cols
    }

    /// Height in rows. Unchanged by [`Self::feed`].
    #[must_use]
    pub const fn rows(&self) -> u16 {
        self.rows
    }

    fn write_printable(&mut self, ch: char) {
        if self.pending_wrap {
            self.wrap_line();
        }
        if self.cursor_row >= self.rows {
            return;
        }
        let row = usize::from(self.cursor_row);
        let col = usize::from(self.cursor_col);
        if let Some(cell) = self.grid.get_mut(row).and_then(|line| line.get_mut(col)) {
            *cell = ch;
        }
        if self.cursor_col < self.cols.saturating_sub(1) {
            self.cursor_col += 1;
        } else {
            self.pending_wrap = true;
        }
    }

    /// The deferred wrap: column 0 of the next row, or off the grid when
    /// the cursor was already on the last row.
    fn wrap_line(&mut self) {
        self.pending_wrap = false;
        self.cursor_col = 0;
        self.cursor_row = self.cursor_row.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::Stub;

    fn render(stub: &Stub) -> String {
        let mut out = format!("{} {}\n", stub.cols(), stub.rows());
        for row in stub.grid() {
            for ch in row {
                out.push(*ch);
            }
            out.push('\n');
        }
        out
    }

    /// The fixture is the expected frame: `cols rows`, then one line per row.
    fn assert_golden(bytes: &[u8], snapshot: &str) {
        let mut header = snapshot.lines();
        let mut parts = header.next().unwrap_or("").split_whitespace();
        let cols: u16 = parts.next().unwrap_or("0").parse().unwrap_or(0);
        let rows: u16 = parts.next().unwrap_or("0").parse().unwrap_or(0);
        assert!(cols >= 1, "fixture columns");
        assert!(rows >= 1, "fixture rows");
        let mut stub = Stub::new(cols, rows);
        stub.feed(bytes);
        assert_eq!(render(&stub), snapshot);
    }

    #[test]
    fn recorded_stream_matches_the_expected_grid() {
        let mut stub = Stub::new(3, 2);
        stub.feed(b"xy\nz");
        assert_eq!(stub.grid(), &[vec!['x', 'y', ' '], vec!['z', ' ', ' ']]);
    }

    /// xterm DECAWM (ctlseqs; esctest2 `test_DECSET_DECAWM` and
    /// `test_DECSET_DECAWM_CursorAtRightMargin`): the last column arms a
    /// wrap and the cursor stays there. CR returns to column 0 of that
    /// same line; only the next printable wraps.
    #[test]
    fn xterm_decawm_defers_wrap_until_the_next_printable() {
        let mut same_line = Stub::new(4, 2);
        same_line.feed(b"abcd\rX");
        assert_eq!(
            same_line.grid(),
            &[vec!['X', 'b', 'c', 'd'], vec![' ', ' ', ' ', ' ']]
        );

        let mut next = Stub::new(4, 2);
        next.feed(b"abcdW");
        assert_eq!(
            next.grid(),
            &[vec!['a', 'b', 'c', 'd'], vec!['W', ' ', ' ', ' ']]
        );

        // A control clears the pending wrap, so newline starts the next row.
        let mut linefeed = Stub::new(4, 3);
        linefeed.feed(b"abcd\nZ");
        assert_eq!(
            linefeed.grid(),
            &[
                vec!['a', 'b', 'c', 'd'],
                vec!['Z', ' ', ' ', ' '],
                vec![' ', ' ', ' ', ' '],
            ]
        );

        assert_golden(b"abcd\rX\nabcdY", include_str!("../fixtures/wrap.txt"));
    }

    #[test]
    fn newline_carriage_return_and_ignored_controls_match_their_golden_snapshot() {
        assert_golden(
            b"ab\ncd\rX\x00\x1bY",
            include_str!("../fixtures/controls.txt"),
        );
    }

    #[test]
    fn feeding_bytes_never_changes_dimensions() {
        let mut stub = Stub::new(0, 0);
        assert_eq!((stub.cols(), stub.rows()), (1, 1));
        stub.feed(b"hello\n\r\x00\x7f");
        assert_eq!((stub.cols(), stub.rows()), (1, 1));
        assert_eq!(stub.grid().len(), 1);
        assert_eq!(stub.grid().first().map(Vec::len), Some(1));

        let mut wide = Stub::new(5, 2);
        wide.feed(&[0x00, b'\n', b'A', 0x7f, 0xff]);
        assert_eq!((wide.cols(), wide.rows()), (5, 2));
        assert!(wide.grid().iter().all(|row| row.len() == 5));
    }

    #[test]
    fn seeded_corpus_keeps_the_grid_size_on_edge_slices() {
        let mut past = Stub::new(2, 1);
        past.feed(b"abcd");
        assert_eq!(past.grid(), &[vec!['a', 'b']]);
        assert_eq!((past.cols(), past.rows()), (2, 1));

        let mut stub = Stub::new(4, 2);
        let (cols, rows) = (stub.cols(), stub.rows());
        let edges: &[&[u8]] = &[b"", b"\x00\x01\x07\x1b\x7f", b"abcdefghijklmnopqrstuvwxyz"];
        for slice in edges {
            stub.feed(slice);
            assert_eq!((stub.cols(), stub.rows()), (cols, rows));
            assert_eq!(stub.grid().len(), usize::from(rows));
            assert!(stub.grid().iter().all(|row| row.len() == usize::from(cols)));
        }

        let mut state: u32 = 0x00c0_ffee;
        for _ in 0..64 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let len = (state & 0x1f) as usize;
            let mut buf = [0_u8; 32];
            for cell in buf.iter_mut().take(len) {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                *cell = (state >> 16) as u8;
            }
            stub.feed(&buf[..len]);
            assert_eq!((stub.cols(), stub.rows()), (cols, rows));
            assert_eq!(stub.grid().len(), usize::from(rows));
            assert!(stub.grid().iter().all(|row| row.len() == usize::from(cols)));
        }
    }
}
