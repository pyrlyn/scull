//! The parser's actions mapped onto state changes: C0 controls, escape
//! sequences and control sequences. Separate so the whole vocabulary reads
//! as one table, with the semantics in the modules it calls.
//!
//! Sequence names and defaults are xterm's ctlseqs. A parameter of 0 means
//! the default (1 for counts and positions), as in ECMA-48; out-of-range
//! values are clamped by the callee, never rejected. Sequences outside this
//! table, OSC included (T13), are ignored.

use scull_parser::{Csi, Esc, Handler, Params};

use crate::edit::Erase;
use crate::sgr;
use crate::state::State;

const BS: u8 = 0x08;
const HT: u8 = 0x09;
const LF: u8 = 0x0A;
const VT: u8 = 0x0B;
const FF: u8 = 0x0C;
const CR: u8 = 0x0D;

/// The intermediate of the DEC line-attribute and test sequences (`ESC #`).
const HASH: u8 = b'#';

/// TBC parameters.
const TBC_AT_CURSOR: u16 = 0;
const TBC_ALL: u16 = 3;

/// Parameter `i`, with 0 or a missing value read as `default`.
fn arg(params: &Params, i: usize, default: u16) -> u16 {
    match params.iter().nth(i).and_then(|g| g.first().copied()) {
        None | Some(0) => default,
        Some(v) => v,
    }
}

/// A count: parameter `i`, at least 1.
fn count(params: &Params, i: usize) -> u16 {
    arg(params, i, 1)
}

/// A 1-based position as a 0-based index.
fn position(params: &Params, i: usize) -> u16 {
    count(params, i) - 1
}

impl Handler for State {
    fn print(&mut self, text: &str) {
        self.print_text(text);
    }

    fn execute(&mut self, byte: u8) {
        self.end_cluster();
        match byte {
            BS => self.back(1),
            HT => self.tab(1),
            LF | VT | FF => self.linefeed(),
            CR => self.carriage_return(),
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, esc: &Esc<'_>) {
        self.end_cluster();
        match (esc.intermediates, esc.final_byte) {
            ([], b'D') => self.index(),
            ([], b'E') => {
                self.index();
                self.carriage_return();
            }
            ([], b'H') => self.tabs.set(self.cursor.col),
            ([], b'M') => self.reverse_index(),
            ([HASH], b'8') => self.alignment_test(),
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, csi: &Csi<'_>) {
        self.end_cluster();
        if csi.private.is_some() || !csi.intermediates.is_empty() {
            return;
        }
        let p = csi.params;
        match csi.final_byte {
            b'@' => self.insert_chars(count(p, 0)),
            b'A' => self.up(count(p, 0)),
            b'B' => self.down(count(p, 0)),
            b'C' => self.forward(count(p, 0)),
            b'D' => self.back(count(p, 0)),
            b'E' => {
                self.down(count(p, 0));
                self.carriage_return();
            }
            b'F' => {
                self.up(count(p, 0));
                self.carriage_return();
            }
            b'G' | b'`' => self.goto_col(position(p, 0)),
            b'H' | b'f' => self.goto_origin(position(p, 0), position(p, 1)),
            b'I' => self.tab(count(p, 0)),
            b'J' => {
                if let Some(which) = Erase::from_param(arg(p, 0, 0)) {
                    self.erase_display(which);
                }
            }
            b'K' => {
                if let Some(which) = Erase::from_param(arg(p, 0, 0)) {
                    self.erase_line(which);
                }
            }
            b'L' => self.insert_lines(count(p, 0)),
            b'M' => self.delete_lines(count(p, 0)),
            b'P' => self.delete_chars(count(p, 0)),
            b'S' => self.scroll_up(count(p, 0)),
            // With more parameters `CSI T` is xterm's mouse highlight tracking.
            b'T' if p.len() <= 1 => self.scroll_down(count(p, 0)),
            b'X' => self.erase_chars(count(p, 0)),
            b'Z' => self.back_tab(count(p, 0)),
            b'a' => self.relative(0, count(p, 0)),
            b'b' => self.repeat(count(p, 0)),
            b'd' => self.goto_row(position(p, 0)),
            b'e' => self.relative(count(p, 0), 0),
            b'g' => match arg(p, 0, TBC_AT_CURSOR) {
                TBC_AT_CURSOR => self.tabs.clear(self.cursor.col),
                TBC_ALL => self.tabs.clear_all(),
                _ => {}
            },
            b'm' => {
                let mut style = *self.pen.style();
                sgr::apply(&mut style, p);
                self.pen.set(style);
            }
            b'r' => self.set_top_bottom(p),
            _ => {}
        }
    }
}

impl State {
    /// DECSTBM: top and bottom margins, 1-based, defaulting to the screen.
    /// A region of fewer than two rows is refused (xterm); a valid one
    /// homes the cursor to the origin.
    fn set_top_bottom(&mut self, p: &Params) {
        let rows = self.rows();
        let top = position(p, 0);
        let bottom = arg(p, 1, rows).min(rows) - 1;
        if top < bottom {
            self.margins.top = top;
            self.margins.bottom = bottom;
            self.goto_origin(0, 0);
        }
    }
}
