//! The one answer a test sink owes the PTY. ConPTY asks for the cursor
//! position (`ESC [ 6 n`) when it starts and holds the child's output until
//! it hears back; a real host answers through the terminal core.

const ASK: &[u8] = b"\x1b[6n";
const ANSWER: &[u8] = b"\x1b[1;1R";

#[derive(Default)]
pub struct CursorReport {
    /// The last bytes seen, so a request split across reads is still found.
    tail: Vec<u8>,
    asked: bool,
    answered: bool,
}

impl CursorReport {
    pub fn feed(&mut self, bytes: &[u8]) {
        if self.asked {
            return;
        }
        self.tail.extend_from_slice(bytes);
        self.asked = self.tail.windows(ASK.len()).any(|w| w == ASK);
        let keep = self.tail.len().saturating_sub(ASK.len() - 1);
        self.tail.drain(..keep);
    }

    /// Writes the answer once, when it fits in `limit`; returns the bytes written.
    pub fn drain(&mut self, out: &mut Vec<u8>, limit: usize) -> usize {
        if !self.asked || self.answered || ANSWER.len() > limit {
            return 0;
        }
        self.answered = true;
        out.extend_from_slice(ANSWER);
        ANSWER.len()
    }
}
