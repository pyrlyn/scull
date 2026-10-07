//! The public terminal: a parser and the state it drives. Separate from the
//! state so the parser can borrow the state as its handler while callers see
//! one owned value.

use std::time::Instant;

use scull_grid::{Grid, Style};
use scull_parser::Parser;
use scull_unicode::WidthOptions;

use crate::error::TermError;
use crate::modes::Modes;
use crate::state::{Cursor, Margins, State};
use crate::sync::SyncGate;

/// A terminal: feed it PTY output, read its grid and cursor.
#[derive(Debug, Clone)]
pub struct Terminal {
    parser: Parser,
    state: State,
    sync: SyncGate,
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
        // Full-screen programs own the alternate screen; nothing scrolls off
        // it into history (xterm).
        let alt = Grid::new(cols, rows, 0)?;
        Ok(Self {
            parser: Parser::new(),
            state: State::new(grid, alt, width),
            sync: SyncGate::default(),
        })
    }

    /// Parses `bytes` and applies every complete action. Bytes are untrusted
    /// PTY output; nothing in them can make this fail or panic.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.feed(bytes, &mut self.state);
        if self
            .sync
            .fed(self.state.modes.synchronized_output, bytes.len())
        {
            self.end_sync();
        }
    }

    /// Whether synchronized output (mode 2026) holds the picture at `now`.
    /// The hold ends, and the mode is reset, once it has lasted
    /// [`crate::MAX_SYNC_HOLD`] from the first call that saw it or spanned
    /// [`crate::MAX_SYNC_BYTES`] fed bytes. Call it after every `feed` and
    /// wake the UI only when it returns false.
    pub fn sync_held(&mut self, now: Instant) -> bool {
        if self.sync.holds(self.state.modes.synchronized_output, now) {
            return true;
        }
        self.end_sync();
        false
    }

    /// When a hold seen by [`Self::sync_held`] ends by itself: the poll
    /// timeout of the reader thread while the picture is held.
    pub fn sync_deadline(&self) -> Option<Instant> {
        self.sync.deadline()
    }

    fn end_sync(&mut self) {
        self.state.modes.synchronized_output = false;
        self.sync = SyncGate::default();
    }

    /// Moves the viewport `delta` rows back into history (negative: towards
    /// the screen), clamped to what history holds.
    pub fn scroll_display(&mut self, delta: isize) {
        self.state.grid.scroll_display(delta);
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

    /// Whether the alternate screen is shown.
    pub fn is_alt_screen(&self) -> bool {
        self.state.alt_active
    }

    /// Takes the replies queued for the program (device attributes, status
    /// and cursor reports, mode reports), to be written back to the PTY.
    /// The queue holds at most a few kilobytes; replies that arrive while it
    /// is full are dropped whole, so drain it after every `feed`.
    pub fn take_replies(&mut self) -> Vec<u8> {
        self.state.replies.take()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use proptest::prelude::*;
    use scull_grid::{Cell, CellFlags, Content};

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
        b"?",
        b">",
        b"!p",
        b"$p",
        b"h",
        b"l",
        b"n",
        b"c",
        b"s",
        b"u",
        b"6",
        b"7",
        b"8",
        b"47",
        b"69",
        b"1049",
        b"2027",
        b"(0",
        b")0",
        b"\x0e",
        b"\x0f",
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

    /// Random PTY output that reaches deep into the dispatch table.
    pub(crate) fn stream() -> impl Strategy<Value = Vec<u8>> {
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

    #[test]
    fn a_flood_of_requests_never_grows_the_reply_queue_past_its_cap() {
        let mut term = Terminal::new(10, 5, 0).unwrap();
        let request = b"\x1b[6n";
        term.feed(&request.repeat(100_000));
        let replies = term.take_replies();
        assert!(!replies.is_empty());
        assert!(replies.len() <= crate::reply::MAX_REPLY_BYTES);
        assert!(replies.ends_with(b"R"), "only whole replies are kept");
        term.feed(b"\x1b[5n");
        assert_eq!(term.take_replies(), b"\x1b[0n", "draining makes room again");
    }

    #[test]
    fn the_alternate_screen_starts_blank_and_leaves_the_main_one_intact() {
        let mut term = Terminal::new(10, 5, 10).unwrap();
        term.feed(b"main");
        term.feed(b"\x1b[?1049h");
        assert!(term.is_alt_screen());
        assert_eq!(
            term.grid().screen_row(0).unwrap().cell(0),
            Some(Cell::EMPTY)
        );
        term.feed(b"\x1b[?1049l");
        assert!(!term.is_alt_screen());
        let first = term.grid().screen_row(0).unwrap().cell(0).unwrap();
        assert_eq!(first.content(), Content::Char('m'));
    }

    const BSU: &[u8] = b"\x1b[?2026h";
    const ESU: &[u8] = b"\x1b[?2026l";
    const QUERY_SYNC: &[u8] = b"\x1b[?2026$p";
    const SYNC_RESET: &[u8] = b"\x1b[?2026;2$y";

    #[test]
    fn synchronized_output_holds_the_picture_until_esu() {
        let now = Instant::now();
        let mut term = Terminal::new(10, 5, 0).unwrap();
        assert!(!term.sync_held(now));
        term.feed(BSU);
        term.feed(b"half a frame");
        assert!(term.sync_held(now));
        assert_eq!(term.sync_deadline(), now.checked_add(crate::MAX_SYNC_HOLD));
        term.feed(ESU);
        assert!(!term.sync_held(now));
        assert_eq!(term.sync_deadline(), None);
    }

    #[test]
    fn a_hold_past_the_time_cap_is_released_and_reads_as_reset() {
        let start = Instant::now();
        let mut term = Terminal::new(10, 5, 0).unwrap();
        term.feed(BSU);
        assert!(term.sync_held(start));
        assert!(term.sync_held(start + crate::MAX_SYNC_HOLD / 2));
        assert!(!term.sync_held(start + crate::MAX_SYNC_HOLD));
        assert!(!term.modes().synchronized_output);
        term.feed(QUERY_SYNC);
        assert_eq!(term.take_replies(), SYNC_RESET);
    }

    #[test]
    fn a_hold_past_the_byte_cap_is_released() {
        let now = Instant::now();
        let mut term = Terminal::new(10, 5, 0).unwrap();
        term.feed(BSU);
        let rest = crate::MAX_SYNC_BYTES - BSU.len();
        term.feed(&vec![b'x'; rest]);
        assert!(term.sync_held(now), "exactly at the cap still holds");
        term.feed(b"x");
        assert!(!term.sync_held(now));
        assert!(!term.modes().synchronized_output);
    }

    #[test]
    fn a_new_hold_starts_with_fresh_caps() {
        let start = Instant::now();
        let mut term = Terminal::new(10, 5, 0).unwrap();
        term.feed(BSU);
        assert!(term.sync_held(start));
        term.feed(ESU);
        assert!(!term.sync_held(start + crate::MAX_SYNC_HOLD / 2));
        term.feed(BSU);
        let later = start + crate::MAX_SYNC_HOLD;
        assert!(term.sync_held(later), "timed from its own first look");
    }

    #[test]
    fn a_full_reset_ends_the_hold() {
        let now = Instant::now();
        let mut term = Terminal::new(10, 5, 0).unwrap();
        term.feed(BSU);
        term.feed(b"\x1bc");
        assert!(!term.sync_held(now));
    }

    fn stamps(term: &Terminal) -> Vec<crate::RowStamp> {
        let g = term.grid();
        (0..g.screen_rows())
            .map(|r| crate::RowStamp::of(g.visible_row(r).unwrap()))
            .collect()
    }

    fn damage_of(term: &mut Terminal, change: impl FnOnce(&mut Terminal)) -> crate::Damage {
        let old = stamps(term);
        change(term);
        let mut d = crate::Damage::default();
        d.compute(&old, &stamps(term), false);
        d
    }

    fn scroll(start: u16, end: u16, from: u16) -> crate::Scroll {
        crate::Scroll { start, end, from }
    }

    #[test]
    fn a_linefeed_at_the_bottom_is_scroll_damage() {
        let mut term = Terminal::new(10, 4, 10).unwrap();
        term.feed(b"a\r\nb\r\nc\r\nd");
        let d = damage_of(&mut term, |t| t.feed(b"\r\n"));
        assert_eq!(d.scrolls(), [scroll(0, 3, 1)]);
        assert_eq!(d.dirty(), [3]);
    }

    #[test]
    fn a_region_scroll_moves_only_the_region() {
        let mut term = Terminal::new(10, 6, 10).unwrap();
        term.feed(b"\x1b[2;5r");
        let d = damage_of(&mut term, |t| t.feed(b"\x1b[2S"));
        assert_eq!(d.scrolls(), [scroll(1, 3, 3)]);
        assert_eq!(d.dirty(), [3, 4]);
    }

    #[test]
    fn printing_dirties_only_the_rows_written() {
        let mut term = Terminal::new(10, 4, 0).unwrap();
        let d = damage_of(&mut term, |t| t.feed(b"\x1b[3Hx"));
        assert!(d.scrolls().is_empty());
        assert_eq!(d.dirty(), [2]);
    }

    #[test]
    fn scrolling_the_viewport_back_is_scroll_damage_too() {
        let mut term = Terminal::new(10, 3, 10).unwrap();
        term.feed(b"1\r\n2\r\n3\r\n4\r\n5");
        let d = damage_of(&mut term, |t| t.scroll_display(1));
        assert_eq!(d.scrolls(), [scroll(1, 3, 0)]);
        assert_eq!(d.dirty(), [0]);
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
            prop_assert!(term.take_replies().len() <= crate::reply::MAX_REPLY_BYTES);
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
