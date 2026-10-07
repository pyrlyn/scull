//! SGR styles and their interning table. Separate from the cell so a cell
//! carries a 16-bit id while colours, underline colour and attributes live
//! once per distinct style: underline colour is the rare data that would
//! otherwise widen every cell.

use bitflags::bitflags;

use crate::GridError;
use crate::intern::{Interner, Marks};

/// Live styles at most, the default included: every id fits the cell's `u16`.
pub const MAX_STYLES: usize = u16::MAX as usize + 1;

/// A colour as the application named it; the palette resolves it later, so
/// a palette change (OSC 4) repaints without touching the grid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Color {
    /// The terminal's default foreground, background or underline colour.
    #[default]
    Default,
    /// One of the 256 palette entries.
    Indexed(u8),
    /// A direct colour (SGR 38/48/58 with `2`).
    Rgb(u8, u8, u8),
}

/// Underline shape (SGR 4 with a sub-parameter, SGR 21).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Underline {
    /// No underline.
    #[default]
    None,
    /// Single line.
    Single,
    /// Double line.
    Double,
    /// Curly line.
    Curly,
    /// Dotted line.
    Dotted,
    /// Dashed line.
    Dashed,
}

bitflags! {
    /// SGR on/off attributes.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct Attrs: u16 {
        /// SGR 1.
        const BOLD = 1;
        /// SGR 2.
        const DIM = 1 << 1;
        /// SGR 3.
        const ITALIC = 1 << 2;
        /// SGR 5 and 6.
        const BLINK = 1 << 3;
        /// SGR 7.
        const INVERSE = 1 << 4;
        /// SGR 8.
        const HIDDEN = 1 << 5;
        /// SGR 9.
        const STRIKE = 1 << 6;
        /// SGR 53.
        const OVERLINE = 1 << 7;
    }
}

/// Everything SGR can say about a cell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Style {
    /// Foreground.
    pub fg: Color,
    /// Background.
    pub bg: Color,
    /// Underline colour (SGR 58); `Default` follows the foreground.
    pub underline_color: Color,
    /// Underline shape.
    pub underline: Underline,
    /// On/off attributes.
    pub attrs: Attrs,
}

/// Index of a [`Style`] in a [`StyleTable`]. Id 0 is the default style.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StyleId(pub u16);

impl StyleId {
    /// The default style; always present, never reclaimed.
    pub const DEFAULT: Self = Self(0);
}

/// Distinct styles, capped at [`MAX_STYLES`].
#[derive(Debug, Clone)]
pub struct StyleTable(Interner<Style>);

impl Default for StyleTable {
    fn default() -> Self {
        Self::with_cap(MAX_STYLES)
    }
}

impl StyleTable {
    /// A table holding at most `cap` live styles (clamped to [`MAX_STYLES`]).
    pub fn with_cap(cap: usize) -> Self {
        Self(Interner::new(cap.min(MAX_STYLES), [Style::default()]))
    }

    /// The id of `style`, adding it when new.
    pub fn intern(&mut self, style: &Style) -> Result<StyleId, GridError> {
        self.0
            .intern(style, |s| *s)
            .and_then(|id| u16::try_from(id).ok())
            .map(StyleId)
            .ok_or(GridError::StyleTableFull { cap: self.0.cap() })
    }

    /// The style behind `id`; `None` for an id that was reclaimed.
    pub fn get(&self, id: StyleId) -> Option<&Style> {
        self.0.get(u32::from(id.0))
    }

    /// Live styles, the default included.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Never true: the default style is always present.
    pub fn is_empty(&self) -> bool {
        self.0.len() == 0
    }

    /// The hard cap of live styles.
    pub fn cap(&self) -> usize {
        self.0.cap()
    }

    /// Styles added since the last sweep; the grid sweeps only
    /// when enough were added for the walk over every row to pay off.
    pub fn inserts_since_sweep(&self) -> usize {
        self.0.inserts_since_sweep()
    }

    /// A blank mark set for [`Self::sweep`].
    pub fn marks(&self) -> Marks {
        self.0.marks()
    }

    /// Reclaims every id `live` does not mark; returns how many. Ids held
    /// outside the marked rows are invalid afterwards.
    pub fn sweep(&mut self, live: &Marks) -> usize {
        self.0.sweep(live)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn red(n: u8) -> Style {
        Style {
            fg: Color::Rgb(n, 0, 0),
            ..Style::default()
        }
    }

    #[test]
    fn default_style_is_id_zero() {
        let mut t = StyleTable::default();
        assert_eq!(t.intern(&Style::default()), Ok(StyleId::DEFAULT));
        assert_eq!(t.get(StyleId::DEFAULT), Some(&Style::default()));
    }

    #[test]
    fn style_table_refuses_styles_past_its_cap() {
        const CAP: usize = 3;
        let mut t = StyleTable::with_cap(CAP);
        assert!(t.intern(&red(1)).is_ok());
        assert!(t.intern(&red(2)).is_ok());
        assert_eq!(
            t.intern(&red(3)),
            Err(GridError::StyleTableFull { cap: CAP })
        );
        assert_eq!(t.len(), CAP);
    }

    #[test]
    fn full_size_table_never_hands_out_an_id_past_u16() {
        let mut t = StyleTable::default();
        let mut last = StyleId::DEFAULT;
        for n in 0..MAX_STYLES {
            let [hi, lo] = u16::try_from(n).unwrap().to_be_bytes();
            match t.intern(&Style {
                bg: Color::Rgb(hi, lo, 1),
                ..Style::default()
            }) {
                Ok(id) => last = id,
                Err(e) => {
                    assert_eq!(e, GridError::StyleTableFull { cap: MAX_STYLES });
                    break;
                }
            }
        }
        assert_eq!(last, StyleId(u16::MAX));
        assert_eq!(t.len(), MAX_STYLES);
    }
}
