//! Golden segmentation and width results for the sequences a terminal meets
//! most: emoji ZWJ sequences, flags, skin tones, variation selectors,
//! combining marks, Hangul jamo, CJK and ambiguous width. Driven the way the
//! grid will drive the crate: one code point at a time.

use proptest::prelude::*;
use scull_unicode::{
    AmbiguousWidth, ClusterPolicy, ClusterWidth, GraphemeState, UnicodeVersion, WidthOptions,
    cluster_width, width,
};

const NARROW: WidthOptions = WidthOptions {
    ambiguous: AmbiguousWidth::Narrow,
    version: UnicodeVersion::LATEST,
};
const WIDE: WidthOptions = WidthOptions {
    ambiguous: AmbiguousWidth::Wide,
    version: UnicodeVersion::LATEST,
};
const MAX_WIDTH: u8 = 2;

/// Clusters of `text` with their widths under mode 2027.
fn layout(text: &str, opts: WidthOptions) -> Vec<(String, u8)> {
    let mut state = GraphemeState::new(opts.version);
    let mut out: Vec<(String, ClusterWidth)> = Vec::new();
    for cp in text.chars() {
        let boundary = state.next(cp);
        match out.last_mut() {
            Some((cluster, acc)) if !boundary => {
                cluster.push(cp);
                acc.push(cp);
            }
            _ => {
                let mut acc = ClusterWidth::new(opts, ClusterPolicy::Grapheme);
                acc.push(cp);
                out.push((cp.to_string(), acc));
            }
        }
    }
    out.into_iter().map(|(c, acc)| (c, acc.width())).collect()
}

fn one(cluster: &str, cells: u8) -> Vec<(String, u8)> {
    vec![(cluster.to_owned(), cells)]
}

#[test]
fn zwj_family_is_one_wide_cluster() {
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    assert_eq!(layout(family, NARROW), one(family, 2));
}

#[test]
fn zwj_joins_a_text_default_base_into_a_wide_emoji() {
    // HEART ON FIRE with and without the VS16 its RGI form carries.
    for seq in [
        "\u{2764}\u{FE0F}\u{200D}\u{1F525}",
        "\u{2764}\u{200D}\u{1F525}",
    ] {
        assert_eq!(layout(seq, NARROW), one(seq, 2), "{seq:?}");
    }
}

#[test]
fn flags_pair_regional_indicators() {
    let us_de = "\u{1F1FA}\u{1F1F8}\u{1F1E9}\u{1F1EA}";
    let want = vec![
        ("\u{1F1FA}\u{1F1F8}".to_owned(), 2),
        ("\u{1F1E9}\u{1F1EA}".to_owned(), 2),
    ];
    assert_eq!(layout(us_de, NARROW), want);
    assert_eq!(layout("\u{1F1FA}", NARROW), one("\u{1F1FA}", 2));
}

#[test]
fn skin_tone_modifier_joins_its_base() {
    let thumbs = "\u{1F44D}\u{1F3FD}";
    assert_eq!(layout(thumbs, NARROW), one(thumbs, 2));
    assert_eq!(width('\u{1F3FD}', NARROW), 2);
}

#[test]
fn vs16_widens_a_text_default_emoji() {
    assert_eq!(layout("\u{2764}", NARROW), one("\u{2764}", 1));
    assert_eq!(
        layout("\u{2764}\u{FE0F}", NARROW),
        one("\u{2764}\u{FE0F}", 2)
    );
    let keycap = "1\u{FE0F}\u{20E3}";
    assert_eq!(layout(keycap, NARROW), one(keycap, 2));
}

#[test]
fn vs15_narrows_an_emoji_default_base() {
    assert_eq!(layout("\u{231A}", NARROW), one("\u{231A}", 2));
    assert_eq!(
        layout("\u{231A}\u{FE0E}", NARROW),
        one("\u{231A}\u{FE0E}", 1)
    );
}

#[test]
fn selectors_leave_bases_without_variation_sequences_alone() {
    assert_eq!(layout("a\u{FE0F}", NARROW), one("a\u{FE0F}", 1));
    assert_eq!(
        layout("\u{4E2D}\u{FE0E}", NARROW),
        one("\u{4E2D}\u{FE0E}", 2)
    );
}

#[test]
fn mode_2027_reset_keeps_the_first_codepoint_width() {
    let heart = "\u{2764}\u{FE0F}";
    assert_eq!(
        cluster_width(heart, NARROW, ClusterPolicy::FirstCodepoint),
        1
    );
    assert_eq!(cluster_width(heart, NARROW, ClusterPolicy::Grapheme), 2);
}

#[test]
fn combining_marks_add_no_width() {
    let e = "e\u{0301}\u{0302}";
    assert_eq!(layout(e, NARROW), one(e, 1));
    assert_eq!(width('\u{0301}', NARROW), 0);
    // A mark with no base is its own zero-width cluster; the grid places it.
    assert_eq!(layout("\u{0301}", NARROW), one("\u{0301}", 0));
}

#[test]
fn indic_conjunct_is_one_cluster() {
    let ksha = "\u{0915}\u{094D}\u{0937}";
    assert_eq!(layout(ksha, NARROW), one(ksha, 1));
}

#[test]
fn hangul_jamo_compose_into_one_wide_cluster() {
    let han = "\u{1112}\u{1161}\u{11AB}";
    assert_eq!(layout(han, NARROW), one(han, 2));
    assert_eq!(layout("\u{D55C}", NARROW), one("\u{D55C}", 2));
    assert_eq!(width('\u{1161}', NARROW), 0);
    assert_eq!(width('\u{11AB}', NARROW), 0);
}

#[test]
fn cjk_and_fullwidth_are_wide_halfwidth_is_narrow() {
    for cp in ['\u{4E2D}', '\u{3042}', '\u{FF21}', '\u{20000}', '\u{3FFFD}'] {
        assert_eq!(width(cp, NARROW), 2, "{cp:?}");
    }
    let voiced_ka = "\u{FF76}\u{FF9E}";
    assert_eq!(layout(voiced_ka, NARROW), one(voiced_ka, 1));
}

#[test]
fn ambiguous_characters_follow_the_setting() {
    for cp in ['\u{00B1}', '\u{03B1}', '\u{2192}', '\u{E000}'] {
        assert_eq!(width(cp, NARROW), 1, "{cp:?}");
        assert_eq!(width(cp, WIDE), 2, "{cp:?}");
    }
    assert_eq!(width('a', WIDE), 1);
    assert_eq!(width('\u{4E2D}', WIDE), 2);
}

proptest! {
    #[test]
    fn width_never_exceeds_two(cp in any::<char>()) {
        prop_assert!(width(cp, NARROW) <= MAX_WIDTH);
        prop_assert!(width(cp, WIDE) <= MAX_WIDTH);
    }

    #[test]
    fn segmentation_and_cluster_width_accept_any_input(text in any::<Vec<char>>()) {
        let text: String = text.into_iter().collect();
        for (_, cells) in layout(&text, WIDE) {
            prop_assert!(cells <= MAX_WIDTH);
        }
    }
}
