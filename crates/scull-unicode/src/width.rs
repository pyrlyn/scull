//! The one width authority: cells per code point and per grapheme cluster.
//! The bulk scanner and the grid both call into this module, so they can
//! never disagree about where the cursor ends up.
//!
//! Cluster width under DEC private mode 2027 follows the Terminal Unicode
//! Core specification (<https://github.com/contour-terminal/terminal-unicode-core>,
//! `spec/terminal-unicode-core.tex`, commit `64f5385`, checked 2026-10-07):
//! a cluster is one cell group, ZWJ emoji are two cells wide, and VS16 widens
//! the cluster to two cells. VS15 keeps the width, as the specification
//! says, although kitty (`screen.c:1219-1231`), foot (`terminal.c:4397-4422`)
//! and Contour's libunicode (`width.cpp:76-99`) narrow the cluster to one
//! cell: applications that count cells with `wcwidth` give VS15 no width, so
//! narrowing would move the cursor away from where they think it is, while
//! keeping the width costs at most a blank cell beside a narrow glyph. Like foot
//! and Contour, both selectors act only on a base that has an emoji variation
//! sequence in `emoji-variation-sequences.txt`. Sources and line numbers are
//! in `docs/research/kitty.md` and `docs/research/foot-contour.md`.

use crate::props::{UnicodeVersion, WidthClass, lookup};

/// Emoji-presentation selector (VS16).
const VS16: char = '\u{FE0F}';
const ZWJ: char = '\u{200D}';
const NARROW: u8 = 1;
const WIDE: u8 = 2;

/// Width of East Asian Ambiguous characters (UAX #11, class `A`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AmbiguousWidth {
    /// One cell: what most non-CJK fonts and applications assume.
    #[default]
    Narrow,
    /// Two cells: what CJK legacy encodings and fonts assume.
    Wide,
}

/// Settings every width question needs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct WidthOptions {
    /// Cells for East Asian Ambiguous characters.
    pub ambiguous: AmbiguousWidth,
    /// Which Unicode tables answer.
    pub version: UnicodeVersion,
}

/// How a cluster's width is decided; DEC private mode 2027 selects between them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ClusterPolicy {
    /// Mode 2027 reset: the first code point decides, as legacy `wcwidth`
    /// callers expect. Segmentation still groups the cluster into one cell.
    #[default]
    FirstCodepoint,
    /// Mode 2027 set: variation selectors and ZWJ sequences change the width.
    Grapheme,
}

/// Cells taken by `cp` on its own: 0, 1 or 2.
///
/// Zero for nonspacing and enclosing marks, format characters (ZWJ, variation
/// selectors, tags), Hangul medial vowels and final consonants, and C0, DEL
/// and C1 controls. Controls are 0 rather than an error because the parser
/// executes them before anything asks for a width, so the answer is never
/// used for layout and the function stays total. Regional indicators are 2,
/// so a flag does not move the cursor when its second half arrives.
pub fn width(cp: char, opts: WidthOptions) -> u8 {
    class_width(lookup(cp, opts.version).width, opts.ambiguous)
}

fn class_width(class: WidthClass, ambiguous: AmbiguousWidth) -> u8 {
    match (class, ambiguous) {
        (WidthClass::Zero, _) => 0,
        (WidthClass::Narrow, _) | (WidthClass::Ambiguous, AmbiguousWidth::Narrow) => NARROW,
        (WidthClass::Wide, _) | (WidthClass::Ambiguous, AmbiguousWidth::Wide) => WIDE,
    }
}

/// Width of the grapheme cluster being built, fed the same code points as
/// [`crate::GraphemeState`] put into that cluster.
///
/// The base is the first code point with a non-zero width; until one arrives
/// the cluster is zero cells wide and the grid decides where it goes.
#[derive(Clone, Copy, Debug)]
pub struct ClusterWidth {
    opts: WidthOptions,
    policy: ClusterPolicy,
    width: u8,
    has_base: bool,
    base_has_vs: bool,
    after_zwj: bool,
}

impl ClusterWidth {
    /// An empty cluster.
    pub fn new(opts: WidthOptions, policy: ClusterPolicy) -> Self {
        Self {
            opts,
            policy,
            width: 0,
            has_base: false,
            base_has_vs: false,
            after_zwj: false,
        }
    }

    /// Adds `cp` to the cluster and returns the cluster's width after it.
    pub fn push(&mut self, cp: char) -> u8 {
        let props = lookup(cp, self.opts.version);
        let own = class_width(props.width, self.opts.ambiguous);
        if !self.has_base {
            if own > 0 {
                self.has_base = true;
                self.base_has_vs = props.vs_base;
                self.width = own;
            }
        } else if self.policy == ClusterPolicy::Grapheme {
            match cp {
                VS16 if self.base_has_vs => self.width = WIDE,
                _ if self.after_zwj && props.ext_pict => self.width = WIDE,
                _ => {}
            }
        }
        self.after_zwj = cp == ZWJ;
        self.width
    }

    /// Cells the cluster takes so far.
    pub fn width(&self) -> u8 {
        self.width
    }
}

/// Cells taken by a whole cluster, for callers that already hold its text.
pub fn cluster_width(cluster: &str, opts: WidthOptions, policy: ClusterPolicy) -> u8 {
    let mut acc = ClusterWidth::new(opts, policy);
    cluster.chars().for_each(|cp| {
        acc.push(cp);
    });
    acc.width()
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDE_AMBIGUOUS: WidthOptions = WidthOptions {
        ambiguous: AmbiguousWidth::Wide,
        version: UnicodeVersion::LATEST,
    };

    #[test]
    fn controls_are_zero_wide() {
        for cp in ['\0', '\x1b', '\x7f', '\u{85}', '\u{9f}'] {
            assert_eq!(width(cp, WidthOptions::default()), 0, "{cp:?}");
        }
    }

    #[test]
    fn ambiguous_follows_the_setting() {
        assert_eq!(width('\u{00B1}', WidthOptions::default()), NARROW);
        assert_eq!(width('\u{00B1}', WIDE_AMBIGUOUS), WIDE);
    }

    #[test]
    fn first_codepoint_policy_ignores_selectors() {
        let opts = WidthOptions::default();
        assert_eq!(
            cluster_width("\u{2764}\u{FE0F}", opts, ClusterPolicy::FirstCodepoint),
            NARROW
        );
    }
}
