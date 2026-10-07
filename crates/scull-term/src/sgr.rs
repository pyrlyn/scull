//! SGR (Select Graphic Rendition, `CSI ... m`): parameters to a [`Style`].
//! Separate because it is a pure function of the parameters, tested without
//! a grid.
//!
//! Codes follow xterm's ctlseqs ("CSI Pm m"). Extended colours accept both
//! the ITU T.416 colon form (`38:2::r:g:b`, `38:2:r:g:b`, `38:5:n`) and the
//! xterm semicolon form (`38;2;r;g;b`, `38;5;n`). Underline shapes use the
//! colon sub-parameter `4:n` that kitty introduced and xterm, foot, WezTerm
//! and Contour accept. An unknown code or an out-of-range colour is ignored,
//! never clamped into another colour.

use scull_grid::{Attrs, Color, Style, Underline};
use scull_parser::Params;

const RESET: u16 = 0;
const BOLD: u16 = 1;
const DIM: u16 = 2;
const ITALIC: u16 = 3;
const UNDERLINE: u16 = 4;
const BLINK_SLOW: u16 = 5;
const BLINK_FAST: u16 = 6;
const INVERSE: u16 = 7;
const HIDDEN: u16 = 8;
const STRIKE: u16 = 9;
const DOUBLE_UNDERLINE: u16 = 21;
const NORMAL_INTENSITY: u16 = 22;
const NOT_ITALIC: u16 = 23;
const NOT_UNDERLINED: u16 = 24;
const NOT_BLINKING: u16 = 25;
const NOT_INVERSE: u16 = 27;
const NOT_HIDDEN: u16 = 28;
const NOT_STRIKE: u16 = 29;
const FG_FIRST: u16 = 30;
const FG_LAST: u16 = 37;
const FG_EXTENDED: u16 = 38;
const FG_DEFAULT: u16 = 39;
const BG_FIRST: u16 = 40;
const BG_LAST: u16 = 47;
const BG_EXTENDED: u16 = 48;
const BG_DEFAULT: u16 = 49;
const OVERLINE: u16 = 53;
const NOT_OVERLINE: u16 = 55;
const UNDERLINE_COLOR: u16 = 58;
const UNDERLINE_COLOR_DEFAULT: u16 = 59;
const BRIGHT_FG_FIRST: u16 = 90;
const BRIGHT_FG_LAST: u16 = 97;
const BRIGHT_BG_FIRST: u16 = 100;
const BRIGHT_BG_LAST: u16 = 107;

/// Palette index of the first bright colour (`90`/`100` map here).
const BRIGHT_BASE: u16 = 8;
/// Colour space selectors of an extended colour.
const DIRECT: u16 = 2;
const INDEXED: u16 = 5;
/// Components of a direct colour; more sub-parameters than this after the
/// `2` mean a colour space id comes first (`38:2:cs:r:g:b`).
const RGB: usize = 3;
/// `4:n` shapes, indexed by `n`.
const UNDERLINE_SHAPES: [Underline; 6] = [
    Underline::None,
    Underline::Single,
    Underline::Double,
    Underline::Curly,
    Underline::Dotted,
    Underline::Dashed,
];

/// Applies `CSI params m` to `style`. No parameters means reset.
pub(crate) fn apply(style: &mut Style, params: &Params) {
    if params.is_empty() {
        *style = Style::default();
        return;
    }
    let mut groups = params.iter();
    while let Some(group) = groups.next() {
        let Some((&code, sub)) = group.split_first() else {
            continue;
        };
        match code {
            RESET => *style = Style::default(),
            BOLD => style.attrs.insert(Attrs::BOLD),
            DIM => style.attrs.insert(Attrs::DIM),
            ITALIC => style.attrs.insert(Attrs::ITALIC),
            UNDERLINE => style.underline = underline(sub.first().copied(), style.underline),
            BLINK_SLOW | BLINK_FAST => style.attrs.insert(Attrs::BLINK),
            INVERSE => style.attrs.insert(Attrs::INVERSE),
            HIDDEN => style.attrs.insert(Attrs::HIDDEN),
            STRIKE => style.attrs.insert(Attrs::STRIKE),
            DOUBLE_UNDERLINE => style.underline = Underline::Double,
            NORMAL_INTENSITY => style.attrs.remove(Attrs::BOLD | Attrs::DIM),
            NOT_ITALIC => style.attrs.remove(Attrs::ITALIC),
            NOT_UNDERLINED => style.underline = Underline::None,
            NOT_BLINKING => style.attrs.remove(Attrs::BLINK),
            NOT_INVERSE => style.attrs.remove(Attrs::INVERSE),
            NOT_HIDDEN => style.attrs.remove(Attrs::HIDDEN),
            NOT_STRIKE => style.attrs.remove(Attrs::STRIKE),
            FG_FIRST..=FG_LAST => style.fg = indexed(code - FG_FIRST),
            BG_FIRST..=BG_LAST => style.bg = indexed(code - BG_FIRST),
            BRIGHT_FG_FIRST..=BRIGHT_FG_LAST => {
                style.fg = indexed(code - BRIGHT_FG_FIRST + BRIGHT_BASE);
            }
            BRIGHT_BG_FIRST..=BRIGHT_BG_LAST => {
                style.bg = indexed(code - BRIGHT_BG_FIRST + BRIGHT_BASE);
            }
            FG_DEFAULT => style.fg = Color::Default,
            BG_DEFAULT => style.bg = Color::Default,
            UNDERLINE_COLOR_DEFAULT => style.underline_color = Color::Default,
            OVERLINE => style.attrs.insert(Attrs::OVERLINE),
            NOT_OVERLINE => style.attrs.remove(Attrs::OVERLINE),
            FG_EXTENDED | BG_EXTENDED | UNDERLINE_COLOR => {
                let color = if sub.is_empty() {
                    semicolon_color(&mut groups)
                } else {
                    colon_color(sub)
                };
                let slot = match code {
                    FG_EXTENDED => &mut style.fg,
                    BG_EXTENDED => &mut style.bg,
                    _ => &mut style.underline_color,
                };
                if let Some(color) = color {
                    *slot = color;
                }
            }
            _ => {}
        }
    }
}

/// `4` alone is a single underline; `4:n` picks the shape, and an unknown
/// shape leaves the current one.
fn underline(shape: Option<u16>, current: Underline) -> Underline {
    match shape {
        None => Underline::Single,
        Some(n) => UNDERLINE_SHAPES
            .get(usize::from(n))
            .copied()
            .unwrap_or(current),
    }
}

fn indexed(n: u16) -> Color {
    u8::try_from(n).map_or(Color::Default, Color::Indexed)
}

fn byte(n: u16) -> Option<u8> {
    u8::try_from(n).ok()
}

/// `38:5:n`, `38:2:r:g:b` or `38:2:cs:r:g:b`.
fn colon_color(sub: &[u16]) -> Option<Color> {
    match *sub {
        [INDEXED, n, ..] => byte(n).map(Color::Indexed),
        [DIRECT, ref rest @ ..] if rest.len() > RGB => rgb_of(rest.get(1..=RGB)?),
        [DIRECT, ref rest @ ..] => rgb_of(rest),
        _ => None,
    }
}

/// `38;5;n` or `38;2;r;g;b`: the colour takes the following parameters.
fn semicolon_color<'a>(groups: &mut impl Iterator<Item = &'a [u16]>) -> Option<Color> {
    let mut next = || groups.next().and_then(|g| g.first().copied());
    match next()? {
        INDEXED => byte(next()?).map(Color::Indexed),
        DIRECT => {
            let rgb = [next()?, next()?, next()?];
            rgb_of(&rgb)
        }
        _ => None,
    }
}

fn rgb_of(rgb: &[u16]) -> Option<Color> {
    match *rgb {
        [r, g, b] => Some(Color::Rgb(byte(r)?, byte(g)?, byte(b)?)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scull_parser::{Csi, Handler, Parser};

    struct Capture(Style);

    impl Handler for Capture {
        fn csi_dispatch(&mut self, csi: &Csi<'_>) {
            apply(&mut self.0, csi.params);
        }
    }

    fn sgr_from(start: Style, params: &str) -> Style {
        let mut capture = Capture(start);
        Parser::new().feed(format!("\x1b[{params}m").as_bytes(), &mut capture);
        capture.0
    }

    fn sgr(params: &str) -> Style {
        sgr_from(Style::default(), params)
    }

    #[test]
    fn empty_and_zero_reset_everything() {
        let styled = sgr("1;3;31;44");
        assert_eq!(sgr_from(styled, ""), Style::default());
        assert_eq!(sgr_from(styled, "0"), Style::default());
    }

    #[test]
    fn attributes_set_and_clear_in_pairs() {
        let on = sgr("1;2;3;5;7;8;9;53");
        let all = Attrs::BOLD
            | Attrs::DIM
            | Attrs::ITALIC
            | Attrs::BLINK
            | Attrs::INVERSE
            | Attrs::HIDDEN
            | Attrs::STRIKE
            | Attrs::OVERLINE;
        assert_eq!(on.attrs, all);
        assert_eq!(sgr_from(on, "22;23;25;27;28;29;55").attrs, Attrs::empty());
    }

    #[test]
    fn sixteen_colours_map_to_palette_indices() {
        let s = sgr("31;42");
        assert_eq!((s.fg, s.bg), (Color::Indexed(1), Color::Indexed(2)));
        let s = sgr("97;100");
        assert_eq!((s.fg, s.bg), (Color::Indexed(15), Color::Indexed(8)));
        let s = sgr_from(s, "39;49");
        assert_eq!((s.fg, s.bg), (Color::Default, Color::Default));
    }

    #[test]
    fn semicolon_and_colon_forms_give_the_same_colours() {
        let expected = (Color::Indexed(200), Color::Rgb(1, 2, 3));
        for params in [
            "38;5;200;48;2;1;2;3",
            "38:5:200;48:2::1:2:3",
            "38:5:200;48:2:1:2:3",
            "38:5:200;48:2:0:1:2:3",
        ] {
            let s = sgr(params);
            assert_eq!((s.fg, s.bg), expected, "{params}");
        }
    }

    #[test]
    fn parameters_after_a_semicolon_colour_still_apply() {
        let s = sgr("38;2;10;20;30;1;4");
        assert_eq!(s.fg, Color::Rgb(10, 20, 30));
        assert!(s.attrs.contains(Attrs::BOLD));
        assert_eq!(s.underline, Underline::Single);
    }

    #[test]
    fn out_of_range_colours_are_ignored() {
        assert_eq!(sgr("31;38;5;256").fg, Color::Indexed(1));
        assert_eq!(sgr("31;38:2::1:2:300").fg, Color::Indexed(1));
        assert_eq!(sgr("31;38;5").fg, Color::Indexed(1), "truncated colour");
    }

    #[test]
    fn underline_shapes_and_colour() {
        assert_eq!(sgr("4:3").underline, Underline::Curly);
        assert_eq!(sgr("4:5").underline, Underline::Dashed);
        assert_eq!(sgr("21").underline, Underline::Double);
        assert_eq!(sgr("4:3;4:0").underline, Underline::None);
        assert_eq!(sgr("4:4;4:9").underline, Underline::Dotted, "unknown shape");
        assert_eq!(sgr("4;24").underline, Underline::None);
        let s = sgr("58:2::255:0:0");
        assert_eq!(s.underline_color, Color::Rgb(255, 0, 0));
        assert_eq!(sgr_from(s, "59").underline_color, Color::Default);
        assert_eq!(sgr("58;5;9").underline_color, Color::Indexed(9));
    }

    #[test]
    fn unknown_codes_are_ignored() {
        assert_eq!(sgr("1;12345;3").attrs, Attrs::BOLD | Attrs::ITALIC);
    }
}
