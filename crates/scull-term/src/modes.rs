//! Terminal modes: the switches that change how later actions behave, and
//! the mode numbers that name them. Separate from the state so the input
//! encoders (scull-input) and the frame can read them as one plain value.
//!
//! Numbers and power-on values follow xterm's ctlseqs ("Set Mode", "DEC
//! Private Mode Set"); 2026 and 2027 are the synchronized-output and
//! grapheme-cluster specifications shared by Contour, foot and WezTerm.
//! The mouse and kitty keyboard state are scull-input's own types, so the
//! encoders read exactly what the program set.

use scull_input::{KittyKeyboard, MouseEncoding, MouseModes, MouseTracking};

/// Mode switches, in their power-on state by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Modes {
    /// DECAWM (`?7`): printing past the right margin wraps to the next line.
    /// On at power-on, as in xterm.
    pub autowrap: bool,
    /// DECOM (`?6`): cursor addressing is relative to the scroll margins.
    pub origin: bool,
    /// IRM (`4`): printing shifts the rest of the line right.
    pub insert: bool,
    /// LNM (`20`): LF, VT and FF also return the carriage.
    pub newline: bool,
    /// Mode 2027: cluster widths follow the grapheme, not the first code point.
    pub grapheme_clusters: bool,
    /// DECCKM (`?1`): cursor keys send application sequences.
    pub cursor_keys: bool,
    /// DECKPAM / DECKPNM (`ESC =`, `ESC >`): the keypad sends application
    /// sequences.
    pub keypad_app: bool,
    /// DECTCEM (`?25`): the cursor is shown. On at power-on.
    pub cursor_visible: bool,
    /// DECLRMM (`?69`): DECSLRM may set left and right margins.
    pub left_right_margins: bool,
    /// `?1004`: focus changes are reported.
    pub focus_events: bool,
    /// `?2004`: pasted text is bracketed.
    pub bracketed_paste: bool,
    /// Mode 2026: the frame should hold its picture until this is reset.
    pub synchronized_output: bool,
    /// Mouse tracking (`?9`, `?1000`, `?1002`, `?1003`) and its encoding
    /// (`?1005`, `?1006`, `?1015`, `?1016`).
    pub mouse: MouseModes,
    /// `?1007`: on the alternate screen, an untracked wheel sends cursor
    /// keys. On at power-on, as in Ghostty, so `less` and `man` scroll.
    pub alternate_scroll: bool,
    /// The kitty keyboard flag stacks (`CSI > u`, `CSI < u`, `CSI = u`),
    /// one per screen.
    pub kitty: KittyKeyboard,
}

impl Default for Modes {
    fn default() -> Self {
        Self {
            autowrap: true,
            origin: false,
            insert: false,
            newline: false,
            grapheme_clusters: false,
            cursor_keys: false,
            keypad_app: false,
            cursor_visible: true,
            left_right_margins: false,
            focus_events: false,
            bracketed_paste: false,
            synchronized_output: false,
            mouse: MouseModes::default(),
            alternate_scroll: true,
            kitty: KittyKeyboard::default(),
        }
    }
}

/// ANSI modes (`CSI Pm h`) this terminal keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnsiMode {
    /// IRM.
    Insert = 4,
    /// LNM.
    Newline = 20,
}

impl AnsiMode {
    pub(crate) fn from_code(code: u16) -> Option<Self> {
        [Self::Insert, Self::Newline]
            .into_iter()
            .find(|m| *m as u16 == code)
    }
}

/// DEC private modes (`CSI ? Pm h`) this terminal keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DecMode {
    /// DECCKM.
    CursorKeys = 1,
    /// DECOM.
    Origin = 6,
    /// DECAWM.
    Autowrap = 7,
    /// X10 mouse: presses only.
    X10Mouse = 9,
    /// DECTCEM.
    CursorVisible = 25,
    /// The alternate screen, entered and left without clearing.
    AltScreen = 47,
    /// DECLRMM.
    LeftRightMargins = 69,
    /// Normal mouse tracking: presses and releases.
    NormalMouse = 1000,
    /// Button-event tracking: motion while a button is held.
    ButtonMouse = 1002,
    /// Any-event tracking: all motion.
    AnyMouse = 1003,
    /// Focus reporting.
    FocusEvents = 1004,
    /// UTF-8 mouse coordinates.
    Utf8Mouse = 1005,
    /// SGR mouse reports.
    SgrMouse = 1006,
    /// Alternate scroll.
    AlternateScroll = 1007,
    /// urxvt mouse reports.
    UrxvtMouse = 1015,
    /// SGR mouse reports in pixels.
    SgrPixelMouse = 1016,
    /// The alternate screen, cleared on leaving.
    AltScreenClear = 1047,
    /// DECSC on set, DECRC on reset.
    SaveCursor = 1048,
    /// 1048 and 1047 together, with the alternate screen cleared on entry.
    AltScreenSaveCursor = 1049,
    /// Bracketed paste.
    BracketedPaste = 2004,
    /// Synchronized output.
    SynchronizedOutput = 2026,
    /// Grapheme-cluster widths.
    GraphemeClusters = 2027,
}

impl DecMode {
    pub(crate) fn from_code(code: u16) -> Option<Self> {
        [
            Self::CursorKeys,
            Self::Origin,
            Self::Autowrap,
            Self::X10Mouse,
            Self::CursorVisible,
            Self::AltScreen,
            Self::LeftRightMargins,
            Self::NormalMouse,
            Self::ButtonMouse,
            Self::AnyMouse,
            Self::FocusEvents,
            Self::Utf8Mouse,
            Self::SgrMouse,
            Self::AlternateScroll,
            Self::UrxvtMouse,
            Self::SgrPixelMouse,
            Self::AltScreenClear,
            Self::SaveCursor,
            Self::AltScreenSaveCursor,
            Self::BracketedPaste,
            Self::SynchronizedOutput,
            Self::GraphemeClusters,
        ]
        .into_iter()
        .find(|m| *m as u16 == code)
    }

    /// The tracking a mouse tracking mode selects.
    pub(crate) fn mouse_tracking(self) -> Option<MouseTracking> {
        match self {
            Self::X10Mouse => Some(MouseTracking::X10),
            Self::NormalMouse => Some(MouseTracking::Normal),
            Self::ButtonMouse => Some(MouseTracking::ButtonEvent),
            Self::AnyMouse => Some(MouseTracking::AnyEvent),
            _ => None,
        }
    }

    /// The encoding a mouse encoding mode selects.
    pub(crate) fn mouse_encoding(self) -> Option<MouseEncoding> {
        match self {
            Self::Utf8Mouse => Some(MouseEncoding::Utf8),
            Self::SgrMouse => Some(MouseEncoding::Sgr),
            Self::UrxvtMouse => Some(MouseEncoding::Urxvt),
            Self::SgrPixelMouse => Some(MouseEncoding::SgrPixels),
            _ => None,
        }
    }
}

/// A DECRQM answer (`Ps` of DECRPM).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModeStatus {
    NotRecognized = 0,
    Set = 1,
    Reset = 2,
}

impl From<bool> for ModeStatus {
    fn from(on: bool) -> Self {
        if on { Self::Set } else { Self::Reset }
    }
}
