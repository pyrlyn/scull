//! The one answer a test sink owes the PTY. ConPTY asks for the cursor
//! position (`ESC [ 6 n`) when it starts and holds the child's output until
//! it hears back; a real host answers through the terminal core.

const ASK: &[u8] = b"\x1b[6n";
const ANSWER: &[u8] = b"\x1b[1;1R";
/// ConPTY asks before the child writes anything, so only the start of the
/// output is searched: a sink holds the lock while it feeds, and scanning a
/// flood would lengthen every hold.
const LOOK: usize = 64;

#[derive(Default)]
pub struct CursorReport {
    /// The start of the output, so a request split across reads is still found.
    head: Vec<u8>,
    asked: bool,
    answered: bool,
}

impl CursorReport {
    pub fn feed(&mut self, bytes: &[u8]) {
        let room = LOOK - self.head.len();
        if self.asked || room == 0 {
            return;
        }
        self.head.extend_from_slice(&bytes[..bytes.len().min(room)]);
        self.asked = self.head.windows(ASK.len()).any(|w| w == ASK);
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
