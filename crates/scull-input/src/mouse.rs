//! Mouse reports: xterm's tracking modes (X10 9, normal 1000, button-event
//! 1002, any-event 1003) in each coordinate encoding (default, UTF-8 1005,
//! SGR 1006, urxvt 1015, SGR-pixels 1016). Separate from the key encoders
//! because it shares nothing with them but the CSI introducer.
//!
//! Follows xterm's ctlseqs "Mouse Tracking" section and, for the cases it
//! leaves open (clamping, the past-end marker, release codes), xterm's own
//! `button.c` (`EditorButton`, `BtnCode`, `EmitMousePosition`):
//! <https://invisible-island.net/xterm/ctlseqs/ctlseqs.html#h2-Mouse-Tracking>.

use crate::bytes::{CSI, SEMI, push_char, push_decimal};
use crate::key::Modifiers;

/// Mouse buttons. Discriminants are xterm's button indexes, whose low two
/// bits, bit 2 (+64) and bit 3 (+128) make the button code.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum MouseButton {
    Left = 0,
    Middle = 1,
    Right = 2,
    WheelUp = 4,
    WheelDown = 5,
    WheelLeft = 6,
    WheelRight = 7,
    Back = 8,
    Forward = 9,
    Button10 = 10,
    Button11 = 11,
}

impl MouseButton {
    fn is_wheel(self) -> bool {
        matches!(
            self,
            Self::WheelUp | Self::WheelDown | Self::WheelLeft | Self::WheelRight
        )
    }
}

/// What the pointer did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseAction {
    /// A button went down (wheel steps are presses).
    Press,
    /// A button went up.
    Release,
    /// The pointer moved to another cell (or pixel in SGR-pixel mode);
    /// the caller drops moves within the same position, as xterm does.
    Motion,
}

/// One mouse event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MouseEvent {
    /// Press, release or motion.
    pub action: MouseAction,
    /// The button pressed or released; for motion, the lowest button held,
    /// or `None` when none is.
    pub button: Option<MouseButton>,
    /// Shift, Alt and Ctrl are reported; other modifiers are ignored.
    pub mods: Modifiers,
    /// Zero-based cell column.
    pub col: u32,
    /// Zero-based cell row.
    pub row: u32,
    /// Zero-based pixel x inside the text area, for SGR-pixel mode.
    pub x_px: u32,
    /// Zero-based pixel y inside the text area, for SGR-pixel mode.
    pub y_px: u32,
}

/// Which events a program asked for (DECSET 9, 1000, 1002, 1003).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MouseTracking {
    /// No reports.
    #[default]
    Off,
    /// Mode 9: presses of the three main buttons, no modifiers.
    X10,
    /// Mode 1000: presses and releases.
    Normal,
    /// Mode 1002: also motion while a button is held.
    ButtonEvent,
    /// Mode 1003: all motion.
    AnyEvent,
}

/// How a report is spelled (DECSET 1005, 1006, 1015, 1016).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MouseEncoding {
    /// `CSI M Cb Cx Cy`, each value + 32 in one byte.
    #[default]
    Default,
    /// Mode 1005: as default, values past 95 as UTF-8.
    Utf8,
    /// Mode 1006: `CSI < Cb ; Cx ; Cy M|m`.
    Sgr,
    /// Mode 1015: `CSI Cb ; Cx ; Cy M`, Cb + 32 in decimal.
    Urxvt,
    /// Mode 1016: SGR with pixel coordinates.
    SgrPixels,
}

/// The mouse modes of the terminal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct MouseModes {
    /// Which events are reported.
    pub tracking: MouseTracking,
    /// How they are spelled.
    pub encoding: MouseEncoding,
}

/// Offset that makes X10-style values printable.
const PRINTABLE_OFFSET: u32 = 32;
/// Largest zero-based coordinate the default encoding carries; xterm
/// clamps to it and sends it as 0, its past-end marker (255 - 32).
const DEFAULT_LIMIT: u32 = 223;
/// The same for UTF-8 encoding, whose values stop at U+07FF (2047 - 32).
const UTF8_LIMIT: u32 = 2015;
/// Byte sent for a coordinate at or past the limit.
const PAST_END: u8 = 0;
/// Button code of a release when the encoding cannot say which button.
const RELEASE_CODE: u32 = 3;
/// Added for motion events.
const MOTION_FLAG: u32 = 32;
/// Modifier bits of the button code.
const SHIFT_FLAG: u32 = 4;
const META_FLAG: u32 = 8;
const CTRL_FLAG: u32 = 16;
/// Button index bits beyond the low two, and what they add to the code.
const BUTTON_LOW_BITS: u8 = 0b11;
const WHEEL_BIT: u8 = 0b100;
const WHEEL_FLAG: u32 = 64;
const EXTRA_BIT: u8 = 0b1000;
const EXTRA_FLAG: u32 = 128;

const FINAL_PRESS: u8 = b'M';
const FINAL_RELEASE: u8 = b'm';
const SGR_MARKER: u8 = b'<';

/// Appends the report for `event`, or nothing when the modes do not ask
/// for it.
pub fn encode_mouse(event: &MouseEvent, modes: &MouseModes, out: &mut Vec<u8>) {
    let sgr = matches!(
        modes.encoding,
        MouseEncoding::Sgr | MouseEncoding::SgrPixels
    );
    let Some(code) = button_code(event, modes.tracking, sgr) else {
        return;
    };
    let release = event.action == MouseAction::Release;
    let (x, y) = match modes.encoding {
        MouseEncoding::SgrPixels => (event.x_px, event.y_px),
        _ => (event.col, event.row),
    };
    out.extend_from_slice(&CSI);
    match modes.encoding {
        MouseEncoding::Default | MouseEncoding::Utf8 => {
            let utf8 = modes.encoding == MouseEncoding::Utf8;
            let limit = if utf8 { UTF8_LIMIT } else { DEFAULT_LIMIT };
            out.push(FINAL_PRESS);
            push_value(out, code + PRINTABLE_OFFSET, utf8);
            for v in [x, y] {
                if v >= limit {
                    out.push(PAST_END);
                } else {
                    push_value(out, v + 1 + PRINTABLE_OFFSET, utf8);
                }
            }
        }
        MouseEncoding::Sgr | MouseEncoding::SgrPixels | MouseEncoding::Urxvt => {
            if sgr {
                out.push(SGR_MARKER);
                push_decimal(out, code);
            } else {
                push_decimal(out, code + PRINTABLE_OFFSET);
            }
            for v in [x, y] {
                out.push(SEMI);
                push_decimal(out, v.saturating_add(1));
            }
            out.push(if sgr && release {
                FINAL_RELEASE
            } else {
                FINAL_PRESS
            });
        }
    }
}

/// One X10-style value: a raw byte, or in UTF-8 encoding a code point,
/// which takes two bytes past 127. Callers keep values under the limits,
/// so neither fallback is reached.
fn push_value(out: &mut Vec<u8>, value: u32, utf8: bool) {
    if utf8 {
        push_char(out, char::from_u32(value).unwrap_or_default());
    } else {
        out.push(u8::try_from(value).unwrap_or(PAST_END));
    }
}

/// The button code before encoding, or `None` when the tracking mode does
/// not report the event.
fn button_code(event: &MouseEvent, tracking: MouseTracking, sgr: bool) -> Option<u32> {
    let button = event.button;
    let reported = match (tracking, event.action) {
        (MouseTracking::Off, _) => false,
        (MouseTracking::X10, MouseAction::Press) => {
            matches!(
                button,
                Some(MouseButton::Left | MouseButton::Middle | MouseButton::Right)
            )
        }
        (MouseTracking::X10, _) => false,
        // Wheel steps have no release.
        (_, MouseAction::Press) => button.is_some(),
        (_, MouseAction::Release) => button.is_some_and(|b| !b.is_wheel()),
        (MouseTracking::Normal, MouseAction::Motion) => false,
        (MouseTracking::ButtonEvent, MouseAction::Motion) => button.is_some(),
        (MouseTracking::AnyEvent, MouseAction::Motion) => true,
    };
    if !reported {
        return None;
    }
    // SGR keeps the released button and marks the release with `m`; the
    // older encodings can only say "a button was released".
    let button = button.filter(|_| sgr || event.action != MouseAction::Release);
    let mut code = button.map_or(RELEASE_CODE, |b| {
        let index = b as u8;
        let mut code = u32::from(index & BUTTON_LOW_BITS);
        if index & WHEEL_BIT != 0 {
            code += WHEEL_FLAG;
        }
        if index & EXTRA_BIT != 0 {
            code += EXTRA_FLAG;
        }
        code
    });
    if event.action == MouseAction::Motion {
        code += MOTION_FLAG;
    }
    // X10 mode predates modifier reporting.
    if tracking != MouseTracking::X10 {
        for (m, flag) in [
            (Modifiers::SHIFT, SHIFT_FLAG),
            (Modifiers::ALT, META_FLAG),
            (Modifiers::CTRL, CTRL_FLAG),
        ] {
            if event.mods.contains(m) {
                code += flag;
            }
        }
    }
    Some(code)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    const COL: u32 = 9;
    const ROW: u32 = 4;
    /// `COL` and `ROW` as X10 bytes: one-based plus 32.
    const CX: u8 = b'*';
    const CY: u8 = b'%';

    fn ev(action: MouseAction, button: Option<MouseButton>, mods: Modifiers) -> MouseEvent {
        MouseEvent {
            action,
            button,
            mods,
            col: COL,
            row: ROW,
            x_px: 100,
            y_px: 50,
        }
    }

    fn press(button: MouseButton) -> MouseEvent {
        ev(MouseAction::Press, Some(button), Modifiers::empty())
    }

    fn release(button: MouseButton) -> MouseEvent {
        ev(MouseAction::Release, Some(button), Modifiers::empty())
    }

    fn motion(button: Option<MouseButton>) -> MouseEvent {
        ev(MouseAction::Motion, button, Modifiers::empty())
    }

    fn enc(event: &MouseEvent, tracking: MouseTracking, encoding: MouseEncoding) -> Vec<u8> {
        let mut out = Vec::new();
        encode_mouse(event, &MouseModes { tracking, encoding }, &mut out);
        out
    }

    fn x10_report(cb: u8) -> Vec<u8> {
        vec![0x1b, b'[', b'M', cb, CX, CY]
    }

    #[test]
    fn button_codes_match_xterm_normal_tracking() {
        let ctrl = Modifiers::CTRL;
        let table = [
            (press(MouseButton::Left), b' '),
            (press(MouseButton::Middle), b'!'),
            (press(MouseButton::Right), b'"'),
            (release(MouseButton::Right), b'#'),
            (ev(MouseAction::Press, Some(MouseButton::Right), ctrl), b'2'),
            (
                ev(
                    MouseAction::Press,
                    Some(MouseButton::Left),
                    Modifiers::SHIFT,
                ),
                b'$',
            ),
            (
                ev(MouseAction::Press, Some(MouseButton::Left), Modifiers::ALT),
                b'(',
            ),
            (press(MouseButton::WheelUp), b'`'),
            (press(MouseButton::WheelDown), b'a'),
            (press(MouseButton::WheelLeft), b'b'),
            (press(MouseButton::WheelRight), b'c'),
            (press(MouseButton::Back), 160),
            (press(MouseButton::Button11), 163),
            (release(MouseButton::Back), b'#'),
        ];
        for (event, cb) in table {
            let got = enc(&event, MouseTracking::Normal, MouseEncoding::Default);
            assert_eq!(got, x10_report(cb), "{event:?}");
        }
    }

    #[test]
    fn tracking_modes_choose_which_events_are_reported() {
        use MouseTracking as T;
        let none: &[u8] = &[];
        let table: [(MouseEvent, [&[u8]; 5]); 7] = [
            // Off, X10, Normal, ButtonEvent, AnyEvent
            (press(MouseButton::Left), [none, b" ", b" ", b" ", b" "]),
            (release(MouseButton::Left), [none, none, b"#", b"#", b"#"]),
            (press(MouseButton::WheelUp), [none, none, b"`", b"`", b"`"]),
            (
                release(MouseButton::WheelUp),
                [none, none, none, none, none],
            ),
            (
                motion(Some(MouseButton::Left)),
                [none, none, none, b"@", b"@"],
            ),
            (
                motion(Some(MouseButton::Right)),
                [none, none, none, b"B", b"B"],
            ),
            (motion(None), [none, none, none, none, b"C"]),
        ];
        let modes = [T::Off, T::X10, T::Normal, T::ButtonEvent, T::AnyEvent];
        for (event, row) in table {
            for (tracking, cb) in modes.into_iter().zip(row) {
                let expected = cb.first().map(|&cb| x10_report(cb)).unwrap_or_default();
                let got = enc(&event, tracking, MouseEncoding::Default);
                assert_eq!(got, expected, "{event:?} {tracking:?}");
            }
        }
    }

    #[test]
    fn x10_mode_reports_no_modifiers_and_only_three_buttons() {
        let ctrl_left = ev(MouseAction::Press, Some(MouseButton::Left), Modifiers::CTRL);
        let x10 = MouseTracking::X10;
        assert_eq!(
            enc(&ctrl_left, x10, MouseEncoding::Default),
            x10_report(b' ')
        );
        assert!(enc(&press(MouseButton::Back), x10, MouseEncoding::Default).is_empty());
    }

    #[test]
    fn default_encoding_sends_the_past_end_marker_beyond_223() {
        let at = |col: u32| {
            let mut e = press(MouseButton::Left);
            e.col = col;
            e.row = 0;
            enc(&e, MouseTracking::Normal, MouseEncoding::Default)
        };
        assert_eq!(at(0), b"\x1b[M !!");
        assert_eq!(at(222), b"\x1b[M \xff!");
        assert_eq!(at(223), b"\x1b[M \x00!");
        assert_eq!(at(u32::MAX), b"\x1b[M \x00!");
    }

    #[test]
    fn utf8_encoding_extends_coordinates_to_2015() {
        let at = |col: u32| {
            let mut e = press(MouseButton::Left);
            e.col = col;
            e.row = 0;
            enc(&e, MouseTracking::Normal, MouseEncoding::Utf8)
        };
        assert_eq!(at(94), b"\x1b[M \x7f!");
        assert_eq!(at(95), b"\x1b[M \xc2\x80!");
        assert_eq!(at(2014), b"\x1b[M \xdf\xbf!");
        assert_eq!(at(2015), b"\x1b[M \x00!");
        let back = enc(
            &press(MouseButton::Back),
            MouseTracking::Normal,
            MouseEncoding::Utf8,
        );
        assert_eq!(back, [0x1b, b'[', b'M', 0xc2, 0xa0, CX, CY]);
    }

    #[test]
    fn sgr_reports_the_released_button_with_a_lowercase_final() {
        let sgr = |e: &MouseEvent| {
            String::from_utf8(enc(e, MouseTracking::AnyEvent, MouseEncoding::Sgr)).unwrap()
        };
        let table = [
            (press(MouseButton::Left), "0;10;5M"),
            (release(MouseButton::Left), "0;10;5m"),
            (release(MouseButton::Right), "2;10;5m"),
            (release(MouseButton::Back), "128;10;5m"),
            (press(MouseButton::WheelDown), "65;10;5M"),
            (motion(None), "35;10;5M"),
            (motion(Some(MouseButton::Middle)), "33;10;5M"),
            (
                ev(
                    MouseAction::Press,
                    Some(MouseButton::Left),
                    Modifiers::CTRL | Modifiers::SHIFT,
                ),
                "20;10;5M",
            ),
        ];
        for (event, expected) in table {
            assert_eq!(sgr(&event), format!("\x1b[<{expected}"), "{event:?}");
        }
        let mut far = press(MouseButton::Left);
        far.col = u32::MAX;
        far.row = 5000;
        assert_eq!(sgr(&far), format!("\x1b[<0;{};5001M", u32::MAX));
    }

    #[test]
    fn urxvt_reports_x10_codes_in_decimal() {
        let urxvt = |e: &MouseEvent| enc(e, MouseTracking::Normal, MouseEncoding::Urxvt);
        assert_eq!(urxvt(&press(MouseButton::Left)), b"\x1b[32;10;5M");
        assert_eq!(urxvt(&release(MouseButton::Left)), b"\x1b[35;10;5M");
        assert_eq!(urxvt(&press(MouseButton::WheelUp)), b"\x1b[96;10;5M");
    }

    #[test]
    fn sgr_pixels_reports_one_based_pixel_positions() {
        let px = |e: &MouseEvent| enc(e, MouseTracking::Normal, MouseEncoding::SgrPixels);
        assert_eq!(px(&press(MouseButton::Left)), b"\x1b[<0;101;51M");
        assert_eq!(px(&release(MouseButton::Left)), b"\x1b[<0;101;51m");
    }

    proptest! {
        #[test]
        fn no_mouse_event_panics(
            action in prop::sample::select(vec![
                MouseAction::Press, MouseAction::Release, MouseAction::Motion,
            ]),
            button in prop::option::of(prop::sample::select(vec![
                MouseButton::Left, MouseButton::WheelRight, MouseButton::Button11,
            ])),
            mods in any::<u8>(),
            col in any::<u32>(),
            row in any::<u32>(),
            tracking in prop::sample::select(vec![
                MouseTracking::X10, MouseTracking::Normal, MouseTracking::AnyEvent,
            ]),
            encoding in prop::sample::select(vec![
                MouseEncoding::Default, MouseEncoding::Utf8, MouseEncoding::Sgr,
                MouseEncoding::Urxvt, MouseEncoding::SgrPixels,
            ]),
        ) {
            let event = MouseEvent {
                action,
                button,
                mods: Modifiers::from_bits_retain(mods),
                col,
                row,
                x_px: col,
                y_px: row,
            };
            let mut out = Vec::new();
            encode_mouse(&event, &MouseModes { tracking, encoding }, &mut out);
        }
    }
}
