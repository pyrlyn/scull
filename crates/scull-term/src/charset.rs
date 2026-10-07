//! Character sets: the G0-G3 designations (SCS), the locking shifts that
//! pick one for GL (SI, SO, LS2, LS3) and the single shifts (SS2, SS3).
//! Separate because it is a small translation table with its own state that
//! DECSC saves and restores whole.
//!
//! Only the sets programs still use are kept: ASCII, DEC Special Graphics
//! (line drawing, VT100 User Guide table 3-9 on vt100.net) and the UK
//! national set. Other final bytes are ignored, as xterm ignores sets it
//! does not support.

/// One 94-character set.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Charset {
    #[default]
    Ascii,
    DecSpecial,
    Uk,
}

/// The first code point DEC Special Graphics replaces.
const DEC_SPECIAL_FIRST: char = '_';

/// DEC Special Graphics for `_` through `~`; xterm shows `_` as a blank.
const DEC_SPECIAL: [char; 32] = [
    ' ', '◆', '▒', '␉', '␌', '␍', '␊', '°', '±', '␤', '␋', '┘', '┐', '┌', '└', '┼', '⎺', '⎻', '─',
    '⎼', '⎽', '├', '┤', '┴', '┬', '│', '≤', '≥', 'π', '≠', '£', '·',
];

/// The only code point the UK set changes.
const UK_POUND_SOURCE: char = '#';
const POUND: char = '£';

impl Charset {
    /// The set an SCS final byte names.
    pub(crate) fn from_final(byte: u8) -> Option<Self> {
        match byte {
            b'B' => Some(Self::Ascii),
            b'0' => Some(Self::DecSpecial),
            b'A' => Some(Self::Uk),
            _ => None,
        }
    }

    fn map(self, ch: char) -> char {
        match self {
            Self::Ascii => ch,
            Self::DecSpecial => (u32::from(ch))
                .checked_sub(u32::from(DEC_SPECIAL_FIRST))
                .and_then(|i| DEC_SPECIAL.get(usize::try_from(i).ok()?))
                .copied()
                .unwrap_or(ch),
            Self::Uk if ch == UK_POUND_SOURCE => POUND,
            Self::Uk => ch,
        }
    }
}

/// The SCS intermediates for G0 through G3, in slot order.
const SLOT_INTERMEDIATES: [u8; 4] = *b"()*+";

/// The four designations and the shift state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Charsets {
    slots: [Charset; 4],
    /// The slot GL maps to.
    gl: usize,
    /// A slot for the next printed character only (SS2, SS3).
    single: Option<usize>,
}

impl Charsets {
    /// The slot an SCS intermediate designates.
    pub(crate) fn slot_for(intermediate: u8) -> Option<usize> {
        SLOT_INTERMEDIATES.iter().position(|&b| b == intermediate)
    }

    pub(crate) fn designate(&mut self, slot: usize, set: Charset) {
        if let Some(s) = self.slots.get_mut(slot) {
            *s = set;
        }
    }

    pub(crate) fn lock_shift(&mut self, slot: usize) {
        if slot < self.slots.len() {
            self.gl = slot;
        }
    }

    pub(crate) fn single_shift(&mut self, slot: usize) {
        if slot < self.slots.len() {
            self.single = Some(slot);
        }
    }

    /// Translates one printed character, using up a pending single shift.
    pub(crate) fn map(&mut self, ch: char) -> char {
        let slot = self.single.take().unwrap_or(self.gl);
        if !ch.is_ascii() {
            return ch;
        }
        self.slots.get(slot).copied().unwrap_or_default().map(ch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dec_special_graphics_draws_lines_and_leaves_letters_below_it() {
        let mut c = Charsets::default();
        c.designate(0, Charset::DecSpecial);
        assert_eq!(c.map('q'), '─');
        assert_eq!(c.map('x'), '│');
        assert_eq!(c.map('l'), '┌');
        assert_eq!(c.map('~'), '·');
        assert_eq!(c.map('A'), 'A');
        assert_eq!(c.map('中'), '中');
    }

    #[test]
    fn shift_out_selects_g1_and_shift_in_returns_to_g0() {
        let mut c = Charsets::default();
        c.designate(1, Charset::DecSpecial);
        assert_eq!(c.map('q'), 'q');
        c.lock_shift(1);
        assert_eq!(c.map('q'), '─');
        c.lock_shift(0);
        assert_eq!(c.map('q'), 'q');
    }

    #[test]
    fn a_single_shift_lasts_one_character() {
        let mut c = Charsets::default();
        c.designate(2, Charset::Uk);
        c.single_shift(2);
        assert_eq!(c.map('#'), '£');
        assert_eq!(c.map('#'), '#');
    }

    #[test]
    fn out_of_range_slots_are_ignored() {
        let mut c = Charsets::default();
        c.designate(9, Charset::DecSpecial);
        c.lock_shift(9);
        c.single_shift(9);
        assert_eq!(c, Charsets::default());
        assert_eq!(Charsets::slot_for(b'+'), Some(3));
        assert_eq!(Charsets::slot_for(b'-'), None);
        assert_eq!(Charset::from_final(b'Z'), None);
    }
}
