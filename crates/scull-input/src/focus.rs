//! Focus reports (DECSET 1004): `CSI I` when the terminal gains focus and
//! `CSI O` when it loses it. Separate because it belongs to no other
//! input kind; the caller sends it only while the mode is set.
//! <https://invisible-island.net/xterm/ctlseqs/ctlseqs.html#h3-FocusIn_FocusOut>

use crate::bytes::CSI;

const FOCUS_IN: u8 = b'I';
const FOCUS_OUT: u8 = b'O';

/// Appends the report for a focus change.
pub fn encode_focus(focused: bool, out: &mut Vec<u8>) {
    out.extend_from_slice(&CSI);
    out.push(if focused { FOCUS_IN } else { FOCUS_OUT });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_changes_are_csi_i_and_csi_o() {
        let mut out = Vec::new();
        encode_focus(true, &mut out);
        encode_focus(false, &mut out);
        assert_eq!(out, b"\x1b[I\x1b[O");
    }
}
