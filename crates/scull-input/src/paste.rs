//! Pasted text. In bracketed paste mode (DECSET 2004) it is wrapped in
//! `CSI 200 ~` … `CSI 201 ~`, and any marker inside the text is removed so
//! a crafted clipboard cannot end the paste early and have the rest run
//! as typed commands. Separate from the key encoders because paste is a
//! trust boundary of its own: the text comes from another program.
//! <https://invisible-island.net/xterm/ctlseqs/ctlseqs.html#h2-Bracketed-Paste-Mode>
//!
//! Policy beyond that (confirming control characters, large pastes) is the
//! UI's; kitty's sanitiser is described in `docs/research/kitty.md` §8.

use crate::bytes::CSI;

const START: &[u8] = b"\x1b[200~";
const END: &[u8] = b"\x1b[201~";
/// The same markers with the 8-bit CSI, U+009B in UTF-8, which a program
/// reading 8-bit controls would honour.
const START_C1: &[u8] = "\u{9b}200~".as_bytes();
const END_C1: &[u8] = "\u{9b}201~".as_bytes();
const MARKERS: [&[u8]; 4] = [START, END, START_C1, END_C1];
const START_PARAM: &[u8] = b"200~";
const END_PARAM: &[u8] = b"201~";

const LF: u8 = b'\n';
const CR: u8 = b'\r';

/// Appends `text` as a paste. With `bracketed`, the text is wrapped in the
/// markers and stripped of any marker inside it; without, line feeds
/// become carriage returns, as typing Enter would send.
pub fn encode_paste(text: &str, bracketed: bool, out: &mut Vec<u8>) {
    if !bracketed {
        let mut after_cr = false;
        for &b in text.as_bytes() {
            // CR LF is one line break, already a CR.
            if !(b == LF && after_cr) {
                out.push(if b == LF { CR } else { b });
            }
            after_cr = b == CR;
        }
        return;
    }
    out.extend_from_slice(&CSI);
    out.extend_from_slice(START_PARAM);
    let body = out.len();
    for &b in text.as_bytes() {
        out.push(b);
        // Checking after every byte removes markers that only form once an
        // inner marker is gone (`ESC [ 20 ESC [ 201 ~ 1 ~`), in one pass.
        let tail = out.get(body..).unwrap_or_default();
        if let Some(marker) = MARKERS.iter().find(|m| tail.ends_with(m)) {
            out.truncate(out.len() - marker.len());
        }
    }
    out.extend_from_slice(&CSI);
    out.extend_from_slice(END_PARAM);
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn paste(text: &str, bracketed: bool) -> Vec<u8> {
        let mut out = Vec::new();
        encode_paste(text, bracketed, &mut out);
        out
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn bracketed_paste_wraps_the_text_in_markers() {
        assert_eq!(paste("ls -l\n", true), b"\x1b[200~ls -l\n\x1b[201~");
        assert_eq!(paste("", true), b"\x1b[200~\x1b[201~");
    }

    #[test]
    fn markers_inside_the_text_are_removed() {
        let table: [(&str, &[u8]); 6] = [
            ("a\x1b[201~rm -rf ~\n", b"arm -rf ~\n"),
            ("\x1b[200~x", b"x"),
            ("x\u{9b}201~y", b"xy"),
            ("x\u{9b}200~y", b"xy"),
            // Removing the inner marker must not leave an outer one behind.
            ("\x1b[20\x1b[201~1~z", b"z"),
            ("\x1b[2\x1b[20\x1b[201~1~01~!", b"!"),
        ];
        for (text, body) in table {
            let mut expected = START.to_vec();
            expected.extend_from_slice(body);
            expected.extend_from_slice(END);
            assert_eq!(paste(text, true), expected, "{text:?}");
        }
    }

    #[test]
    fn other_escape_sequences_pass_through_untouched() {
        assert_eq!(paste("\x1b[31mred", true), b"\x1b[200~\x1b[31mred\x1b[201~");
    }

    #[test]
    fn unbracketed_paste_sends_line_feeds_as_carriage_returns() {
        assert_eq!(paste("a\nb\r\nc\r", false), b"a\rb\rc\r");
        assert_eq!(paste("\x1b[201~", false), b"\x1b[201~");
    }

    proptest! {
        #[test]
        fn bracketed_body_never_contains_a_marker(
            parts in prop::collection::vec(
                prop::sample::select(vec!["\x1b", "[", "2", "0", "1", "~", "\u{9b}", "a"]),
                0..64,
            ),
        ) {
            let text = parts.concat();
            let out = paste(&text, true);
            let body = &out[START.len()..out.len() - END.len()];
            for marker in MARKERS {
                prop_assert!(!contains(body, marker), "{text:?}");
            }
        }
    }
}
