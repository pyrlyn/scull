//! The public terminal: a parser and the state it drives. Separate from the
//! state so the parser can borrow the state as its handler while callers see
//! one owned value.

use scull_grid::{Grid, Style};
use scull_parser::Parser;
use scull_unicode::WidthOptions;

use crate::error::TermError;
use crate::modes::Modes;
use crate::state::{Cursor, Margins, State};

/// A terminal: feed it PTY output, read its grid and cursor.
#[derive(Debug, Clone)]
pub struct Terminal {
    parser: Parser,
    state: State,
}

impl Terminal {
    /// A blank `cols` x `rows` terminal keeping up to `scrollback` history
    /// rows, with default width options.
    pub fn new(cols: u16, rows: u16, scrollback: usize) -> Result<Self, TermError> {
        Self::with_width(cols, rows, scrollback, WidthOptions::default())
    }

    /// The same with explicit width options (ambiguous width, Unicode version).
    pub fn with_width(
        cols: u16,
        rows: u16,
        scrollback: usize,
        width: WidthOptions,
    ) -> Result<Self, TermError> {
        let grid = Grid::new(cols, rows, scrollback)?;
        Ok(Self {
            parser: Parser::new(),
            state: State::new(grid, width),
        })
    }

    /// Parses `bytes` and applies every complete action. Bytes are untrusted
    /// PTY output; nothing in them can make this fail or panic.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.feed(bytes, &mut self.state);
    }

    /// The grid being drawn.
    pub fn grid(&self) -> &Grid {
        &self.state.grid
    }

    /// The cursor.
    pub fn cursor(&self) -> Cursor {
        self.state.cursor
    }

    /// The style printed text gets now.
    pub fn pen(&self) -> &Style {
        self.state.pen.style()
    }

    /// The mode switches.
    pub fn modes(&self) -> &Modes {
        &self.state.modes
    }

    /// The scroll margins.
    pub fn margins(&self) -> Margins {
        self.state.margins
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use scull_grid::CellFlags;

    use super::*;

    /// Pieces of sequences, so random streams reach deep into the dispatch
    /// table instead of printing bytes almost every time.
    const FRAGMENTS: &[&[u8]] = &[
        b"\x1b[",
        b"\x1b",
        b";",
        b":",
        b"0",
        b"1",
        b"2",
        b"9",
        b"65535",
        b"99999",
        b"@",
        b"A",
        b"B",
        b"C",
        b"D",
        b"E",
        b"F",
        b"G",
        b"H",
        b"I",
        b"J",
        b"K",
        b"L",
        b"M",
        b"P",
        b"S",
        b"T",
        b"X",
        b"Z",
        b"a",
        b"b",
        b"d",
        b"e",
        b"g",
        b"m",
        b"r",
        b"#8",
        b"\r",
        b"\n",
        b"\t",
        b"\x08",
        "\u{4e2d}".as_bytes(),
        "\u{301}".as_bytes(),
        "\u{200d}".as_bytes(),
        "\u{1f468}".as_bytes(),
        "\u{fe0f}".as_bytes(),
    ];

    fn stream() -> impl Strategy<Value = Vec<u8>> {
        let piece = prop_oneof![
            any::<u8>().prop_map(|b| vec![b]),
            proptest::sample::select(FRAGMENTS).prop_map(<[u8]>::to_vec),
        ];
        proptest::collection::vec(piece, 0..400).prop_map(|p| p.concat())
    }

    #[test]
    fn a_zero_sized_terminal_is_an_error() {
        assert!(Terminal::new(0, 5, 0).is_err());
        assert!(Terminal::new(5, 0, 0).is_err());
    }

    #[test]
    fn a_new_terminal_starts_home_with_autowrap() {
        let term = Terminal::new(10, 5, 0).unwrap();
        assert_eq!(term.cursor(), Cursor::default());
        assert!(term.modes().autowrap);
        assert_eq!(term.margins(), Margins::full(10, 5));
        assert_eq!(*term.pen(), Style::default());
    }

    #[test]
    fn a_sequence_split_across_feeds_still_applies() {
        let mut term = Terminal::new(10, 5, 0).unwrap();
        term.feed(b"\x1b[3");
        term.feed(b";4H");
        assert_eq!((term.cursor().row, term.cursor().col), (2, 3));
    }

    proptest! {
        #[test]
        fn no_byte_stream_panics_or_leaves_the_screen(
            cols in 1u16..24,
            rows in 1u16..8,
            bytes in stream(),
        ) {
            let mut term = Terminal::new(cols, rows, 4).unwrap();
            term.feed(&bytes);
            let cursor = term.cursor();
            prop_assert!(cursor.row < rows && cursor.col < cols);
            let m = term.margins();
            prop_assert!(m.top <= m.bottom && m.bottom < rows);
            prop_assert!(m.left <= m.right && m.right < cols);
        }

        #[test]
        fn no_byte_stream_leaves_half_a_wide_character(
            cols in 1u16..24,
            rows in 1u16..8,
            bytes in stream(),
        ) {
            let mut term = Terminal::new(cols, rows, 4).unwrap();
            term.feed(&bytes);
            for r in 0..rows {
                let row = term.grid().screen_row(r).unwrap();
                for c in 0..cols {
                    let flags = row.cell(c).unwrap().flags();
                    if flags.contains(CellFlags::WIDE) {
                        let next = row.cell(c + 1).map(|n| n.flags());
                        prop_assert!(next.is_some_and(|f| f.contains(CellFlags::SPACER)));
                    }
                    if flags.contains(CellFlags::SPACER) {
                        let prev = c.checked_sub(1).and_then(|p| row.cell(p)).map(|p| p.flags());
                        prop_assert!(prev.is_some_and(|f| f.contains(CellFlags::WIDE)));
                    }
                }
            }
        }
    }
}
