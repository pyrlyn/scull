//! Per-code-point properties and the one lookup into the generated tables.
//! Separate from the width and grapheme modules so both read the same record
//! from a single three-stage lookup and the generated file has one set of
//! types to name.

use crate::tables;

/// The Unicode Character Database version the tables come from.
///
/// Only the latest version ships today. Older versions are added as further
/// variants with their own generated tables, so an application can keep the
/// widths its users' other tools still assume.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum UnicodeVersion {
    /// Unicode 18.0.0.
    #[default]
    V18,
}

impl UnicodeVersion {
    /// The newest version this crate knows.
    pub const LATEST: Self = Self::V18;

    /// The version as `(major, minor, update)`, as the data files name it.
    pub fn number(self) -> (u8, u8, u8) {
        match self {
            Self::V18 => tables::UNICODE_VERSION,
        }
    }
}

/// Width class before the ambiguous-width setting is applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WidthClass {
    Zero,
    Narrow,
    Wide,
    /// East Asian Width `A`: narrow or wide depending on the setting.
    Ambiguous,
}

/// `Grapheme_Cluster_Break` property values (UAX #29, table 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Gcb {
    Other,
    Cr,
    Lf,
    Control,
    Extend,
    Zwj,
    RegionalIndicator,
    Prepend,
    SpacingMark,
    L,
    V,
    T,
    Lv,
    Lvt,
}

/// `Indic_Conjunct_Break` property values, used by rule GB9c.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InCb {
    None,
    Linker,
    Consonant,
    Extend,
}

/// Everything the width and grapheme code needs to know about one code point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Props {
    pub(crate) width: WidthClass,
    pub(crate) gcb: Gcb,
    pub(crate) incb: InCb,
    /// `Extended_Pictographic`, used by rule GB11 and ZWJ cluster width.
    pub(crate) ext_pict: bool,
    /// Has an emoji variation sequence (`emoji-variation-sequences.txt`), so
    /// VS15 and VS16 after it change its presentation.
    pub(crate) vs_base: bool,
}

/// Properties of `cp` in the given version's tables.
pub(crate) fn lookup(cp: char, version: UnicodeVersion) -> &'static Props {
    match version {
        UnicodeVersion::V18 => lookup_v18(cp),
    }
}

fn lookup_v18(cp: char) -> &'static Props {
    use tables::{LEAF, LOW_BITS, MID, MID_BITS, PROPS, TOP};
    let cp = cp as usize;
    let low_mask = (1 << LOW_BITS) - 1;
    let mid_mask = (1 << MID_BITS) - 1;
    // Indexing cannot go out of bounds: the generator sizes TOP for every
    // scalar value, and `every_scalar_value_has_props` walks all of them.
    let mid = usize::from(TOP[cp >> (LOW_BITS + MID_BITS)]);
    let leaf = usize::from(MID[(mid << MID_BITS) | ((cp >> LOW_BITS) & mid_mask)]);
    &PROPS[usize::from(LEAF[(leaf << LOW_BITS) | (cp & low_mask)])]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scalar_value_has_props() {
        let mut all = (0..=u32::from(char::MAX)).filter_map(char::from_u32);
        assert!(all.all(|c| tables::PROPS.contains(lookup(c, UnicodeVersion::LATEST))));
    }

    #[test]
    fn latest_version_names_the_generated_tables() {
        assert_eq!(UnicodeVersion::LATEST.number(), (18, 0, 0));
    }

    #[test]
    fn lookup_reads_known_properties() {
        let v = UnicodeVersion::LATEST;
        assert_eq!(lookup('\u{094D}', v).incb, InCb::Linker);
        assert_eq!(lookup('\u{0915}', v).incb, InCb::Consonant);
        assert_eq!(lookup('\u{200D}', v).gcb, Gcb::Zwj);
        assert_eq!(lookup('\u{1F1E6}', v).gcb, Gcb::RegionalIndicator);
        assert!(lookup('\u{1F600}', v).ext_pict);
        assert!(lookup('\u{2764}', v).vs_base);
        assert!(!lookup('a', v).vs_base);
    }
}
