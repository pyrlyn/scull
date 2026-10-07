//! Incremental extended-grapheme-cluster segmentation (UAX #29, Unicode
//! 18.0.0, revision 49). Separate from width because the terminal feeds code
//! points one at a time as they arrive from the PTY: the state here is the
//! only memory of the cluster so far, and nothing looks ahead or back.

use crate::props::{Gcb, InCb, UnicodeVersion, lookup};

/// Where the current cluster stands with respect to rule GB11.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum EmojiRun {
    #[default]
    None,
    /// `\p{Extended_Pictographic} Extend*` just seen.
    Pictographic,
    /// `\p{Extended_Pictographic} Extend* ZWJ` just seen.
    PictographicZwj,
}

/// Grapheme-break state fed one code point at a time.
///
/// `next` answers whether a cluster boundary falls *before* the code point;
/// the first code point always starts a cluster (GB1).
#[derive(Clone, Copy, Debug, Default)]
pub struct GraphemeState {
    version: UnicodeVersion,
    prev: Option<Gcb>,
    /// An odd number of regional indicators ends the text so far (GB12, GB13).
    odd_regional_indicators: bool,
    /// `\p{InCB=Linker} \p{InCB=Extend}*` ends the text so far (GB9c).
    after_linker: bool,
    emoji: EmojiRun,
}

impl GraphemeState {
    /// A state at the start of text, for the given tables.
    pub fn new(version: UnicodeVersion) -> Self {
        Self {
            version,
            ..Self::default()
        }
    }

    /// Feeds `cp`; returns `true` when a cluster boundary falls before it.
    pub fn next(&mut self, cp: char) -> bool {
        let props = lookup(cp, self.version);
        let gcb = props.gcb;
        let boundary = self
            .prev
            .is_none_or(|prev| self.breaks_between(prev, gcb, props.incb, props.ext_pict));

        self.after_linker = match props.incb {
            InCb::Linker => true,
            InCb::Extend => self.after_linker,
            InCb::None | InCb::Consonant => false,
        };
        self.emoji = match (props.ext_pict, gcb, self.emoji) {
            (true, _, _) => EmojiRun::Pictographic,
            (false, Gcb::Extend, EmojiRun::Pictographic) => EmojiRun::Pictographic,
            (false, Gcb::Zwj, EmojiRun::Pictographic) => EmojiRun::PictographicZwj,
            _ => EmojiRun::None,
        };
        self.odd_regional_indicators = gcb == Gcb::RegionalIndicator
            && !(self.prev == Some(Gcb::RegionalIndicator) && self.odd_regional_indicators);
        self.prev = Some(gcb);
        boundary
    }

    /// Rules GB3 to GB999, in the order UAX #29 applies them.
    fn breaks_between(&self, prev: Gcb, next: Gcb, incb: InCb, ext_pict: bool) -> bool {
        use Gcb::{
            Control, Cr, Extend, L, Lf, Lv, Lvt, Prepend, RegionalIndicator, SpacingMark, T, V, Zwj,
        };
        match (prev, next) {
            (Cr, Lf) => false,
            (Control | Cr | Lf, _) | (_, Control | Cr | Lf) => true,
            (L, L | V | Lv | Lvt) | (Lv | V, V | T) | (Lvt | T, T) => false,
            (_, Extend | Zwj | SpacingMark) | (Prepend, _) => false,
            _ if self.after_linker && incb == InCb::Consonant => false,
            _ if self.emoji == EmojiRun::PictographicZwj && ext_pict => false,
            (RegionalIndicator, RegionalIndicator) => !self.odd_regional_indicators,
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clusters(text: &str) -> Vec<String> {
        let mut state = GraphemeState::default();
        let mut out: Vec<String> = Vec::new();
        for cp in text.chars() {
            let boundary = state.next(cp);
            match out.last_mut() {
                Some(last) if !boundary => last.push(cp),
                _ => out.push(cp.to_string()),
            }
        }
        out
    }

    #[test]
    fn crlf_stays_together_and_controls_split() {
        assert_eq!(clusters("a\r\nb"), ["a", "\r\n", "b"]);
    }

    #[test]
    fn devanagari_conjunct_joins_through_virama() {
        // KA VIRAMA SSA: GB9c keeps the conjunct in one cluster.
        assert_eq!(clusters("\u{0915}\u{094D}\u{0937}").len(), 1);
    }

    #[test]
    fn third_regional_indicator_starts_a_new_flag() {
        let ri = "\u{1F1FA}\u{1F1F8}\u{1F1E6}";
        assert_eq!(clusters(ri), ["\u{1F1FA}\u{1F1F8}", "\u{1F1E6}"]);
    }
}
