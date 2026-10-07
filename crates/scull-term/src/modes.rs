//! Terminal modes: the switches that change how later actions behave.
//! Separate from the state so the input encoders (scull-input) and the frame
//! can read them as one plain value.

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
}

impl Default for Modes {
    fn default() -> Self {
        Self {
            autowrap: true,
            origin: false,
            insert: false,
            newline: false,
            grapheme_clusters: false,
        }
    }
}
