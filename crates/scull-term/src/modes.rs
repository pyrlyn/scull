//! Terminal modes: the switches that change how later actions behave, and
//! the mode numbers that name them. Separate from the state so the input
//! encoders (scull-input) and the frame can read them as one plain value.
//!
//! Numbers and power-on values follow xterm's ctlseqs ("Set Mode", "DEC
//! Private Mode Set"); 2026 and 2027 are the synchronized-output and
//! grapheme-cluster specifications shared by Contour, foot and WezTerm.

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
    /// DECTCEM.
    CursorVisible = 25,
    /// The alternate screen, entered and left without clearing.
    AltScreen = 47,
    /// DECLRMM.
    LeftRightMargins = 69,
    /// Focus reporting.
    FocusEvents = 1004,
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
            Self::CursorVisible,
            Self::AltScreen,
            Self::LeftRightMargins,
            Self::FocusEvents,
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
