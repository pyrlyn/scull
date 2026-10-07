//! The fixed 8-byte cell. Its own module because its size is the grid's
//! memory budget: everything rare (multi-code-point clusters, colours,
//! underline colour, hyperlinks) lives elsewhere and the cell holds ids.
//!
//! Wide characters follow foot and Alacritty: the head cell holds the text
//! and the `WIDE` flag, the cell to its right is a `SPACER` with no text and
//! the head's style (so its background paints). A wide character that does
//! not fit at the end of a row leaves a `LEADING_SPACER` there and wraps;
//! reflow (T11) drops that cell when the row is joined again.

use bitflags::bitflags;

use crate::cluster::ClusterId;
use crate::style::StyleId;

/// What a cell shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Content {
    /// Nothing written (or erased). Distinct from a space so copying can trim
    /// erased tails but keep typed spaces.
    Empty,
    /// One code point, stored inline.
    Char(char),
    /// Two or more code points, interned in the grid's cluster table.
    Cluster(ClusterId),
}

bitflags! {
    /// Layout flags of a cell. SGR attributes live in the style, not here.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct CellFlags: u16 {
        /// Head of a two-cell character.
        const WIDE = 1;
        /// Right half of a two-cell character; no text of its own.
        const SPACER = 1 << 1;
        /// Placeholder at the end of a row where a wide character did not fit.
        const LEADING_SPACER = 1 << 2;
        /// Protected from selective erase (DECSCA).
        const PROTECTED = 1 << 3;
    }
}

/// Tag bit beside the public flags: `content` is a cluster id, not a code
/// point. Only the constructors set it, so `content` can never be misread.
const CLUSTER: u16 = 1 << 15;

/// One grid cell: content, style id, layout flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Cell {
    content: u32,
    style: StyleId,
    flags: u16,
}

/// The size the scrollback memory figures in the bench are built on.
pub const CELL_BYTES: usize = 8;
const _: () = assert!(size_of::<Cell>() == CELL_BYTES);

impl Cell {
    /// Never written, default style.
    pub const EMPTY: Self = Self::blank(StyleId::DEFAULT);

    /// Erased with `style`: what ED/EL leave behind with background colour erase.
    pub const fn blank(style: StyleId) -> Self {
        Self::raw(0, style, 0)
    }

    /// One code point. `'\0'` reads back as [`Content::Empty`].
    pub const fn char(ch: char, style: StyleId) -> Self {
        Self::raw(ch as u32, style, 0)
    }

    /// An interned cluster.
    pub const fn cluster(id: ClusterId, style: StyleId) -> Self {
        Self::raw(id.0, style, CLUSTER)
    }

    /// The right half of a wide character drawn with `style`.
    pub const fn spacer(style: StyleId) -> Self {
        Self::raw(0, style, CellFlags::SPACER.bits())
    }

    const fn raw(content: u32, style: StyleId, flags: u16) -> Self {
        Self {
            content,
            style,
            flags,
        }
    }

    /// This cell with `flags` added.
    #[must_use]
    pub const fn with_flags(self, flags: CellFlags) -> Self {
        let public = flags.bits() & CellFlags::all().bits();
        Self {
            flags: self.flags | public,
            ..self
        }
    }

    /// This cell with `flags` cleared.
    #[must_use]
    pub const fn without_flags(self, flags: CellFlags) -> Self {
        let public = flags.bits() & CellFlags::all().bits();
        Self {
            flags: self.flags & !public,
            ..self
        }
    }

    /// This cell drawn with another style.
    #[must_use]
    pub const fn with_style(self, style: StyleId) -> Self {
        Self { style, ..self }
    }

    /// What the cell shows.
    pub fn content(self) -> Content {
        if let Some(id) = self.cluster_id() {
            return Content::Cluster(id);
        }
        match char::from_u32(self.content) {
            Some('\0') | None => Content::Empty,
            Some(ch) => Content::Char(ch),
        }
    }

    /// The cluster id, when the cell holds one; the sweep marks these.
    pub const fn cluster_id(self) -> Option<ClusterId> {
        if self.flags & CLUSTER != 0 {
            Some(ClusterId(self.content))
        } else {
            None
        }
    }

    /// The style id.
    pub const fn style(self) -> StyleId {
        self.style
    }

    /// The layout flags.
    pub const fn flags(self) -> CellFlags {
        CellFlags::from_bits_truncate(self.flags)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_cell_is_all_zero_and_reads_empty() {
        assert_eq!(Cell::EMPTY, Cell::default());
        assert_eq!(Cell::EMPTY.content(), Content::Empty);
        assert_eq!(Cell::char('\0', StyleId::DEFAULT).content(), Content::Empty);
    }

    #[test]
    fn char_and_cluster_never_alias() {
        let id = ClusterId('a' as u32);
        let ch = Cell::char('a', StyleId::DEFAULT);
        let cl = Cell::cluster(id, StyleId::DEFAULT);
        assert_ne!(ch, cl);
        assert_eq!(ch.content(), Content::Char('a'));
        assert_eq!(cl.content(), Content::Cluster(id));
        assert_eq!(ch.cluster_id(), None);
    }

    #[test]
    fn public_flag_setters_cannot_forge_a_cluster() {
        let all = CellFlags::from_bits_retain(u16::MAX);
        let cell = Cell::char('x', StyleId(3)).with_flags(all);
        assert_eq!(cell.content(), Content::Char('x'));
        assert_eq!(cell.flags(), CellFlags::all());
        let cl = Cell::cluster(ClusterId(1), StyleId(3)).without_flags(all);
        assert_eq!(cl.content(), Content::Cluster(ClusterId(1)));
    }

    #[test]
    fn spacer_keeps_the_head_style_and_has_no_text() {
        let s = Cell::spacer(StyleId(9));
        assert_eq!(s.style(), StyleId(9));
        assert_eq!(s.content(), Content::Empty);
        assert!(s.flags().contains(CellFlags::SPACER));
    }
}
