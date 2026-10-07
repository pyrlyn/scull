//! The sequence state machine, after Paul Williams' DEC ANSI parser
//! (<https://vt100.net/emu/dec_ansi_parser>), with the ground state and the
//! string payload states replaced by bulk scans. Separate from the scanner
//! and the action types so the transitions read as one table.
//!
//! Deviations from the Williams machine, all deliberate:
//! - Input is UTF-8. C1 controls are recognised in their UTF-8 form
//!   (U+0080..=U+009F) and in the seven-bit `ESC Fe` form; a raw byte
//!   `0x80..=0x9F` is just invalid UTF-8.
//! - APC has its own streamed state, for kitty graphics.
//! - A string (OSC, DCS, APC) ends only on ST; `ESC` followed by anything
//!   else cancels it, so a lost terminator cannot leak a half payload.
//! - OSC also ends on BEL, as xterm accepts.
//! - A byte `0x80` or above inside an escape or a control sequence aborts
//!   the sequence and is read again as text, so no text is swallowed.

use memchr::memchr3;

use crate::handler::{Csi, End, Esc, Handler, Osc};
use crate::params::Params;
use crate::scan::{C1_LEAD, find_c0, find_special, is_c1_tail, utf8_width};

/// Most intermediate bytes kept; no standard sequence uses more than two.
/// A sequence with more is ignored rather than misread.
pub const MAX_INTERMEDIATES: usize = 2;
/// Most OSC payload bytes kept. Enough for an OSC 52 copy of about 750 KiB
/// of text. A longer OSC is dropped whole: a cut clipboard write or link is
/// worse than none.
pub const MAX_OSC_BYTES: usize = 1 << 20;
/// Most DCS payload bytes streamed to the handler; room for a full-screen
/// sixel image. Past it the payload is cut and the end says so.
pub const MAX_DCS_BYTES: usize = 16 << 20;
/// Most APC payload bytes streamed to the handler. kitty graphics clients
/// send 4 KiB chunks; the margin admits clients that do not chunk.
pub const MAX_APC_BYTES: usize = 4 << 20;

const BEL: u8 = 0x07;
const CAN: u8 = 0x18;
const SUB: u8 = 0x1A;
const ESC: u8 = 0x1B;
const DEL: u8 = 0x7F;
// Byte classes of the Williams table.
const C0_LAST: u8 = 0x1F;
const INTERMEDIATE_FIRST: u8 = 0x20;
const INTERMEDIATE_LAST: u8 = 0x2F;
const ESC_FINAL_FIRST: u8 = 0x30;
/// Parameter bytes are `0x30..=0x3F`: digits, `:`, `;` and the private markers.
const PARAM_LAST: u8 = 0x3F;
const CSI_FINAL_FIRST: u8 = 0x40;
const FINAL_LAST: u8 = 0x7E;
/// The ST that ends a string: `ESC \`.
const ST_FINAL: u8 = b'\\';
/// A C1 control `c` is the escape sequence `ESC (c - 0x40)` (ECMA-48 §5.3).
const C1_TO_FE: u8 = 0x40;
const MAX_UTF8_LEN: usize = 4;
const REPLACEMENT: &str = "\u{FFFD}";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Csi,
    Dcs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Entry,
    Param,
    Intermediate,
    Ignore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Str {
    Osc,
    Dcs,
    Apc,
    /// SOS, PM, and a DCS whose header was malformed.
    Ignore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Ground,
    Escape,
    EscapeIntermediate,
    Header(Kind, Phase),
    Str(Str),
    /// Inside a string, right after `ESC`: `\` ends it, anything else cancels it.
    StrEscape(Str),
}

/// A VT parser. Feed it PTY output in chunks of any size; it keeps the state
/// of a sequence or a UTF-8 character that a chunk boundary cut.
#[derive(Debug, Clone)]
pub struct Parser {
    state: State,
    params: Params,
    intermediates: [u8; MAX_INTERMEDIATES],
    intermediate_len: usize,
    intermediate_overflow: bool,
    private: Option<u8>,
    osc: Vec<u8>,
    osc_overflow: bool,
    string_len: usize,
    string_truncated: bool,
    pending: [u8; MAX_UTF8_LEN],
    pending_len: usize,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    /// A parser in the ground state.
    pub fn new() -> Self {
        Self {
            state: State::Ground,
            params: Params::default(),
            intermediates: [0; MAX_INTERMEDIATES],
            intermediate_len: 0,
            intermediate_overflow: false,
            private: None,
            osc: Vec::new(),
            osc_overflow: false,
            string_len: 0,
            string_truncated: false,
            pending: [0; MAX_UTF8_LEN],
            pending_len: 0,
        }
    }

    /// Parses `bytes`, delivering every complete action to `handler`.
    pub fn feed<H: Handler>(&mut self, bytes: &[u8], handler: &mut H) {
        let mut rest = bytes;
        if self.pending_len > 0 {
            rest = self.complete_utf8(rest, handler);
        }
        while !rest.is_empty() {
            let used = match self.state {
                State::Ground => self.ground(rest, handler),
                State::Str(Str::Osc) => self.osc_bytes(rest, handler),
                State::Str(kind) => self.string_bytes(kind, rest, handler),
                _ => self.sequence_bytes(rest, handler),
            };
            rest = rest.get(used..).unwrap_or_default();
        }
    }

    /// Steps through an escape or control sequence until it leaves for a
    /// bulk state. `ESC [`, parameters and the final byte, which make up
    /// nearly all of an SGR-heavy stream, take a short path past the full
    /// transition table in [`Self::byte`].
    fn sequence_bytes<H: Handler>(&mut self, bytes: &[u8], handler: &mut H) -> usize {
        let mut i = 0;
        while let Some(&byte) = bytes.get(i) {
            match (self.state, byte) {
                (State::Ground | State::Str(_), _) => return i,
                (State::Escape, b'[') => self.state = State::Header(Kind::Csi, Phase::Entry),
                (State::Header(kind, Phase::Entry | Phase::Param), b'0'..=b'9') => {
                    let rest = bytes.get(i..).unwrap_or_default();
                    let digits = rest.iter().take_while(|b| b.is_ascii_digit()).count();
                    self.params.digits(rest.get(..digits).unwrap_or_default());
                    self.state = State::Header(kind, Phase::Param);
                    i += digits;
                    continue;
                }
                (State::Header(kind, Phase::Entry | Phase::Param), b':' | b';') => {
                    self.params.separator(byte == b':');
                    self.state = State::Header(kind, Phase::Param);
                }
                (
                    State::Header(kind, Phase::Entry | Phase::Param),
                    CSI_FINAL_FIRST..=FINAL_LAST,
                ) => {
                    self.dispatch(kind, byte, handler);
                }
                _ => {
                    if self.byte(byte, handler) == 0 {
                        return i;
                    }
                }
            }
            i += 1;
        }
        bytes.len()
    }

    fn ground<H: Handler>(&mut self, bytes: &[u8], handler: &mut H) -> usize {
        let mut pos = 0;
        loop {
            pos += find_special(bytes.get(pos..).unwrap_or_default());
            let Some(&byte) = bytes.get(pos) else {
                self.print_run(bytes, true, handler);
                return bytes.len();
            };
            let run = bytes.get(..pos).unwrap_or_default();
            if byte == C1_LEAD {
                match bytes.get(pos + 1) {
                    Some(&tail) if is_c1_tail(tail) => {
                        self.print_run(run, false, handler);
                        self.c1(tail, handler);
                        return pos + 2;
                    }
                    Some(_) => pos += 1,
                    None => {
                        self.print_run(bytes, true, handler);
                        return bytes.len();
                    }
                }
                continue;
            }
            self.print_run(run, false, handler);
            self.byte(byte, handler);
            return pos + 1;
        }
    }

    /// Prints a run of text. At the end of the input an incomplete UTF-8
    /// sequence is kept for the next `feed`; anywhere else it is invalid.
    fn print_run<H: Handler>(&mut self, run: &[u8], at_end: bool, handler: &mut H) {
        if run.is_empty() {
            return;
        }
        if let Ok(text) = std::str::from_utf8(run) {
            handler.print(text);
            return;
        }
        let mut chunks = run.utf8_chunks().peekable();
        while let Some(chunk) = chunks.next() {
            if !chunk.valid().is_empty() {
                handler.print(chunk.valid());
            }
            let bad = chunk.invalid();
            if bad.is_empty() {
                continue;
            }
            let incomplete = bad.first().is_some_and(|&b| utf8_width(b) > bad.len());
            if at_end && incomplete && chunks.peek().is_none() {
                self.pending_len = bad.len();
                if let Some(dst) = self.pending.get_mut(..bad.len()) {
                    dst.copy_from_slice(bad);
                }
            } else {
                handler.print(REPLACEMENT);
            }
        }
    }

    /// Completes a UTF-8 sequence the previous `feed` cut, the same way the
    /// whole stream would have decoded it.
    fn complete_utf8<'a, H: Handler>(&mut self, input: &'a [u8], handler: &mut H) -> &'a [u8] {
        let have = self.pending_len;
        let need = utf8_width(self.pending[0]);
        let take = need.saturating_sub(have).min(input.len());
        let mut seq = [0; MAX_UTF8_LEN];
        seq[..have].copy_from_slice(&self.pending[..have]);
        seq[have..have + take].copy_from_slice(&input[..take]);
        let seq = &seq[..have + take];
        self.pending_len = 0;
        let Some(chunk) = seq.utf8_chunks().next() else {
            return input;
        };
        let char_bytes = chunk.valid().as_bytes();
        if let [C1_LEAD, tail] = *char_bytes
            && is_c1_tail(tail)
        {
            self.c1(tail, handler);
            return &input[take..];
        }
        if !char_bytes.is_empty() {
            handler.print(chunk.valid());
            return &input[take..];
        }
        let bad = chunk.invalid();
        if take == input.len() && seq.len() < need && bad.len() == seq.len() {
            self.pending[..seq.len()].copy_from_slice(seq);
            self.pending_len = seq.len();
            return &[];
        }
        handler.print(REPLACEMENT);
        input
            .get(bad.len().saturating_sub(have)..)
            .unwrap_or_default()
    }

    fn c1<H: Handler>(&mut self, tail: u8, handler: &mut H) {
        self.enter_escape();
        self.escape(tail - C1_TO_FE, handler);
    }

    fn enter_escape(&mut self) {
        self.state = State::Escape;
        self.params.clear();
        self.intermediate_len = 0;
        self.intermediate_overflow = false;
        self.private = None;
    }

    /// One byte outside the bulk states. Returns 0 when the byte aborted the
    /// sequence and must be read again in the ground state.
    fn byte<H: Handler>(&mut self, byte: u8, handler: &mut H) -> usize {
        if let State::StrEscape(kind) = self.state {
            if byte == ST_FINAL {
                self.end_string(kind, false, handler);
                self.state = State::Ground;
                return 1;
            }
            self.end_string(kind, true, handler);
            self.enter_escape();
        }
        match byte {
            CAN | SUB => {
                self.state = State::Ground;
                handler.execute(byte);
                return 1;
            }
            ESC => {
                self.enter_escape();
                return 1;
            }
            _ => {}
        }
        match self.state {
            State::Escape => self.escape(byte, handler),
            State::EscapeIntermediate => self.escape_intermediate(byte, handler),
            State::Header(kind, phase) => self.header(kind, phase, byte, handler),
            _ => {
                if byte < b' ' {
                    handler.execute(byte);
                }
                1
            }
        }
    }

    fn escape<H: Handler>(&mut self, byte: u8, handler: &mut H) -> usize {
        self.state = match byte {
            0..=C0_LAST => {
                handler.execute(byte);
                State::Escape
            }
            INTERMEDIATE_FIRST..=INTERMEDIATE_LAST => {
                self.collect(byte);
                State::EscapeIntermediate
            }
            b'[' => State::Header(Kind::Csi, Phase::Entry),
            b'P' => State::Header(Kind::Dcs, Phase::Entry),
            b']' => {
                self.osc.clear();
                self.osc_overflow = false;
                State::Str(Str::Osc)
            }
            b'_' => {
                self.start_string();
                handler.apc_start();
                State::Str(Str::Apc)
            }
            b'X' | b'^' => State::Str(Str::Ignore),
            ESC_FINAL_FIRST..=FINAL_LAST => {
                handler.esc_dispatch(&Esc {
                    intermediates: &[],
                    final_byte: byte,
                });
                State::Ground
            }
            DEL => State::Escape,
            _ => return self.abort(),
        };
        1
    }

    fn escape_intermediate<H: Handler>(&mut self, byte: u8, handler: &mut H) -> usize {
        match byte {
            0..=C0_LAST => handler.execute(byte),
            INTERMEDIATE_FIRST..=INTERMEDIATE_LAST => self.collect(byte),
            ESC_FINAL_FIRST..=FINAL_LAST => {
                self.state = State::Ground;
                if !self.intermediate_overflow {
                    handler.esc_dispatch(&Esc {
                        intermediates: self.intermediates(),
                        final_byte: byte,
                    });
                }
            }
            DEL => {}
            _ => return self.abort(),
        }
        1
    }

    fn header<H: Handler>(&mut self, kind: Kind, phase: Phase, byte: u8, handler: &mut H) -> usize {
        let next = match (byte, phase) {
            (0..=C0_LAST, _) => {
                // Williams: C0 executes inside CSI but is ignored in a DCS header.
                if kind == Kind::Csi {
                    handler.execute(byte);
                }
                phase
            }
            (DEL, _) | (INTERMEDIATE_FIRST..=PARAM_LAST, Phase::Ignore) => phase,
            (INTERMEDIATE_FIRST..=INTERMEDIATE_LAST, _) => {
                self.collect(byte);
                if self.intermediate_overflow {
                    Phase::Ignore
                } else {
                    Phase::Intermediate
                }
            }
            (b'0'..=b';', Phase::Intermediate)
            | (b'<'..=b'?', Phase::Param | Phase::Intermediate) => Phase::Ignore,
            (b'0'..=b'9', _) => {
                self.params.digits(&[byte]);
                Phase::Param
            }
            (b':' | b';', _) => {
                self.params.separator(byte == b':');
                Phase::Param
            }
            (b'<'..=b'?', _) => {
                self.private = Some(byte);
                Phase::Param
            }
            (CSI_FINAL_FIRST..=FINAL_LAST, Phase::Ignore) => {
                self.state = State::Ground;
                return 1;
            }
            (CSI_FINAL_FIRST..=FINAL_LAST, _) => {
                self.dispatch(kind, byte, handler);
                return 1;
            }
            _ => return self.abort(),
        };
        self.state = match (kind, next) {
            (Kind::Dcs, Phase::Ignore) => State::Str(Str::Ignore),
            _ => State::Header(kind, next),
        };
        1
    }

    fn dispatch<H: Handler>(&mut self, kind: Kind, final_byte: u8, handler: &mut H) {
        self.params.finish();
        let csi = Csi {
            params: &self.params,
            intermediates: self
                .intermediates
                .get(..self.intermediate_len)
                .unwrap_or_default(),
            private: self.private,
            final_byte,
            truncated: self.params.truncated(),
        };
        match kind {
            Kind::Csi => {
                handler.csi_dispatch(&csi);
                self.state = State::Ground;
            }
            Kind::Dcs => {
                handler.dcs_hook(&csi);
                self.start_string();
                self.state = State::Str(Str::Dcs);
            }
        }
    }

    fn abort(&mut self) -> usize {
        self.state = State::Ground;
        0
    }

    fn collect(&mut self, byte: u8) {
        match self.intermediates.get_mut(self.intermediate_len) {
            Some(slot) => {
                *slot = byte;
                self.intermediate_len += 1;
            }
            None => self.intermediate_overflow = true,
        }
    }

    fn intermediates(&self) -> &[u8] {
        self.intermediates
            .get(..self.intermediate_len)
            .unwrap_or_default()
    }

    fn osc_bytes<H: Handler>(&mut self, bytes: &[u8], handler: &mut H) -> usize {
        let end = find_c0(bytes);
        let data = bytes.get(..end).unwrap_or_default();
        let room = MAX_OSC_BYTES.saturating_sub(self.osc.len());
        if data.len() > room {
            self.osc_overflow = true;
        }
        self.osc
            .extend_from_slice(data.get(..room.min(data.len())).unwrap_or_default());
        match bytes.get(end) {
            None => return end,
            Some(&BEL) => {
                self.dispatch_osc(true, handler);
                self.state = State::Ground;
            }
            Some(&ESC) => self.state = State::StrEscape(Str::Osc),
            Some(&byte @ (CAN | SUB)) => {
                self.state = State::Ground;
                handler.execute(byte);
            }
            // Other C0 controls are dropped from the payload.
            Some(_) => {}
        }
        end + 1
    }

    fn dispatch_osc<H: Handler>(&mut self, bell_terminated: bool, handler: &mut H) {
        if !self.osc_overflow {
            handler.osc_dispatch(&Osc {
                data: &self.osc,
                bell_terminated,
            });
        }
    }

    fn start_string(&mut self) {
        self.string_len = 0;
        self.string_truncated = false;
    }

    fn string_bytes<H: Handler>(&mut self, kind: Str, bytes: &[u8], handler: &mut H) -> usize {
        let end = memchr3(ESC, CAN, SUB, bytes).unwrap_or(bytes.len());
        let cap = match kind {
            Str::Dcs => MAX_DCS_BYTES,
            Str::Apc => MAX_APC_BYTES,
            Str::Osc | Str::Ignore => 0,
        };
        let room = cap.saturating_sub(self.string_len);
        let data = bytes.get(..end.min(room)).unwrap_or_default();
        if end > room && kind != Str::Ignore {
            self.string_truncated = true;
        }
        self.string_len += data.len();
        if !data.is_empty() {
            match kind {
                Str::Dcs => handler.dcs_put(data),
                Str::Apc => handler.apc_put(data),
                Str::Osc | Str::Ignore => {}
            }
        }
        match bytes.get(end) {
            None => return end,
            Some(&ESC) => self.state = State::StrEscape(kind),
            Some(&byte) => {
                self.end_string(kind, true, handler);
                self.state = State::Ground;
                handler.execute(byte);
            }
        }
        end + 1
    }

    fn end_string<H: Handler>(&mut self, kind: Str, cancelled: bool, handler: &mut H) {
        let end = match (cancelled, self.string_truncated) {
            (true, _) => End::Cancelled,
            (false, true) => End::Truncated,
            (false, false) => End::Complete,
        };
        match kind {
            Str::Osc if !cancelled => self.dispatch_osc(false, handler),
            Str::Dcs => handler.dcs_unhook(end),
            Str::Apc => handler.apc_end(end),
            Str::Osc | Str::Ignore => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::params::MAX_PARAMS;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Action {
        Print(String),
        Execute(u8),
        Esc(Vec<u8>, u8),
        Csi {
            params: Vec<Vec<u16>>,
            intermediates: Vec<u8>,
            private: Option<u8>,
            final_byte: u8,
            truncated: bool,
        },
        Osc(Vec<u8>, bool),
        Hook(Vec<Vec<u16>>, Vec<u8>, u8),
        Put(Vec<u8>),
        Unhook(End),
        ApcStart,
        ApcPut(Vec<u8>),
        ApcEnd(End),
    }

    /// Records actions, merging adjacent text and payload pieces so a stream
    /// split at any point records the same list.
    #[derive(Default)]
    struct Recorder(Vec<Action>);

    impl Handler for Recorder {
        fn print(&mut self, text: &str) {
            assert!(!text.is_empty(), "empty print");
            match self.0.last_mut() {
                Some(Action::Print(s)) => s.push_str(text),
                _ => self.0.push(Action::Print(text.to_owned())),
            }
        }
        fn execute(&mut self, byte: u8) {
            self.0.push(Action::Execute(byte));
        }
        fn esc_dispatch(&mut self, esc: &Esc<'_>) {
            self.0
                .push(Action::Esc(esc.intermediates.to_vec(), esc.final_byte));
        }
        fn csi_dispatch(&mut self, csi: &Csi<'_>) {
            self.0.push(Action::Csi {
                params: csi.params.iter().map(<[u16]>::to_vec).collect(),
                intermediates: csi.intermediates.to_vec(),
                private: csi.private,
                final_byte: csi.final_byte,
                truncated: csi.truncated,
            });
        }
        fn osc_dispatch(&mut self, osc: &Osc<'_>) {
            self.0
                .push(Action::Osc(osc.data.to_vec(), osc.bell_terminated));
        }
        fn dcs_hook(&mut self, h: &Csi<'_>) {
            let params = h.params.iter().map(<[u16]>::to_vec).collect();
            self.0
                .push(Action::Hook(params, h.intermediates.to_vec(), h.final_byte));
        }
        fn dcs_put(&mut self, data: &[u8]) {
            assert!(!data.is_empty(), "empty put");
            match self.0.last_mut() {
                Some(Action::Put(d)) => d.extend_from_slice(data),
                _ => self.0.push(Action::Put(data.to_vec())),
            }
        }
        fn dcs_unhook(&mut self, end: End) {
            self.0.push(Action::Unhook(end));
        }
        fn apc_start(&mut self) {
            self.0.push(Action::ApcStart);
        }
        fn apc_put(&mut self, data: &[u8]) {
            assert!(!data.is_empty(), "empty put");
            match self.0.last_mut() {
                Some(Action::ApcPut(d)) => d.extend_from_slice(data),
                _ => self.0.push(Action::ApcPut(data.to_vec())),
            }
        }
        fn apc_end(&mut self, end: End) {
            self.0.push(Action::ApcEnd(end));
        }
    }

    fn parse(bytes: &[u8]) -> Vec<Action> {
        let mut rec = Recorder::default();
        Parser::new().feed(bytes, &mut rec);
        rec.0
    }

    fn parse_bytewise(bytes: &[u8]) -> Vec<Action> {
        let mut rec = Recorder::default();
        let mut parser = Parser::new();
        for b in bytes {
            parser.feed(std::slice::from_ref(b), &mut rec);
        }
        rec.0
    }

    fn print(s: &str) -> Action {
        Action::Print(s.to_owned())
    }

    fn csi(params: &[&[u16]], private: Option<u8>, inter: &[u8], final_byte: u8) -> Action {
        Action::Csi {
            params: params.iter().map(|p| p.to_vec()).collect(),
            intermediates: inter.to_vec(),
            private,
            final_byte,
            truncated: false,
        }
    }

    /// Counts payload bytes without keeping them, for the cap tests.
    #[derive(Default)]
    struct Counter {
        dcs: usize,
        apc: usize,
        osc: usize,
        end: Option<End>,
    }

    impl Handler for Counter {
        fn osc_dispatch(&mut self, osc: &Osc<'_>) {
            self.osc += 1;
            assert!(osc.data.len() <= MAX_OSC_BYTES);
        }
        fn dcs_put(&mut self, data: &[u8]) {
            self.dcs += data.len();
        }
        fn dcs_unhook(&mut self, end: End) {
            self.end = Some(end);
        }
        fn apc_put(&mut self, data: &[u8]) {
            self.apc += data.len();
        }
        fn apc_end(&mut self, end: End) {
            self.end = Some(end);
        }
    }

    /// Feeds `head`, then `filler` bytes of `a` in large chunks, then `tail`.
    fn feed_long(head: &[u8], filler: usize, tail: &[u8]) -> (Counter, Parser) {
        const CHUNK: usize = 1 << 20;
        let chunk = vec![b'a'; CHUNK];
        let mut parser = Parser::new();
        let mut counter = Counter::default();
        parser.feed(head, &mut counter);
        let mut left = filler;
        while left > 0 {
            let n = left.min(CHUNK);
            parser.feed(&chunk[..n], &mut counter);
            left -= n;
        }
        parser.feed(tail, &mut counter);
        (counter, parser)
    }

    /// How far past a cap the overflow tests go.
    const OVERFLOW: usize = 4096;

    #[test]
    fn text_up_to_a_control_byte_is_one_print() {
        let mut rec = Vec::new();
        struct Raw<'a>(&'a mut Vec<String>);
        impl Handler for Raw<'_> {
            fn print(&mut self, text: &str) {
                self.0.push(text.to_owned());
            }
        }
        Parser::new().feed(b"hello world\r\nnext", &mut Raw(&mut rec));
        assert_eq!(rec, ["hello world", "next"]);
    }

    #[test]
    fn controls_between_runs_are_executed_in_order() {
        assert_eq!(
            parse(b"a\r\nb"),
            [
                print("a"),
                Action::Execute(b'\r'),
                Action::Execute(b'\n'),
                print("b")
            ]
        );
    }

    #[test]
    fn invalid_utf8_becomes_replacement_characters() {
        assert_eq!(parse(b"a\xFFb\xE2\x82c"), [print("a\u{FFFD}b\u{FFFD}c")]);
    }

    #[test]
    fn utf8_split_across_feeds_is_completed() {
        let text = "日本🎉é";
        assert_eq!(parse_bytewise(text.as_bytes()), [print(text)]);
    }

    #[test]
    fn incomplete_utf8_before_a_control_byte_is_invalid() {
        let bytes = b"\xE2\x82\n";
        assert_eq!(parse(bytes), [print("\u{FFFD}"), Action::Execute(b'\n')]);
        assert_eq!(parse_bytewise(bytes), parse(bytes));
    }

    #[test]
    fn del_is_ignored_in_text() {
        assert_eq!(parse(b"a\x7Fb"), [print("ab")]);
    }

    #[test]
    fn esc_dispatch_carries_intermediates() {
        assert_eq!(
            parse(b"\x1b7\x1b(B"),
            [Action::Esc(vec![], b'7'), Action::Esc(vec![b'('], b'B')]
        );
    }

    #[test]
    fn esc_with_too_many_intermediates_is_ignored() {
        let mut seq = vec![ESC];
        seq.extend(std::iter::repeat_n(b'(', MAX_INTERMEDIATES + 1));
        seq.push(b'B');
        assert_eq!(parse(&seq), []);
    }

    #[test]
    fn csi_with_params_private_marker_and_intermediates() {
        assert_eq!(
            parse(b"\x1b[1;38:2::1:2:3m\x1b[?25h\x1b[2 q"),
            [
                csi(&[&[1], &[38, 2, 0, 1, 2, 3]], None, b"", b'm'),
                csi(&[&[25]], Some(b'?'), b"", b'h'),
                csi(&[&[2]], None, b" ", b'q'),
            ]
        );
    }

    #[test]
    fn c0_inside_csi_executes_without_breaking_it() {
        assert_eq!(
            parse(b"\x1b[3\n1m"),
            [Action::Execute(b'\n'), csi(&[&[31]], None, b"", b'm')]
        );
    }

    #[test]
    fn malformed_csi_is_ignored_up_to_its_final_byte() {
        assert_eq!(parse(b"\x1b[1?2mX\x1b[ 1mY"), [print("XY")]);
    }

    #[test]
    fn csi_with_too_many_intermediates_is_ignored() {
        assert_eq!(parse(b"\x1b[1   qok"), [print("ok")]);
    }

    #[test]
    fn csi_with_too_many_params_is_truncated_and_flagged() {
        let mut seq = b"\x1b[".to_vec();
        seq.extend(vec!["1"; MAX_PARAMS + OVERFLOW].join(";").bytes());
        seq.push(b'm');
        let actions = parse(&seq);
        let [
            Action::Csi {
                params, truncated, ..
            },
        ] = actions.as_slice()
        else {
            panic!("{actions:?}");
        };
        assert_eq!(params.len(), MAX_PARAMS);
        assert!(truncated);
    }

    #[test]
    fn can_aborts_a_sequence_and_executes() {
        assert_eq!(parse(b"\x1b[12\x18m"), [Action::Execute(CAN), print("m")]);
    }

    #[test]
    fn high_byte_inside_csi_aborts_it_and_is_printed() {
        assert_eq!(parse("\x1b[1é".as_bytes()), [print("é")]);
    }

    #[test]
    fn utf8_c1_controls_act_as_their_escape_forms() {
        let bytes = "a\u{85}b\u{9B}2J".as_bytes();
        assert_eq!(
            parse(bytes),
            [
                print("a"),
                Action::Esc(vec![], b'E'),
                print("b"),
                csi(&[&[2]], None, b"", b'J'),
            ]
        );
        assert_eq!(parse_bytewise(bytes), parse(bytes));
    }

    #[test]
    fn latin1_supplement_is_text_not_c1() {
        assert_eq!(parse("©«»".as_bytes()), [print("©«»")]);
    }

    #[test]
    fn osc_ends_on_bel_or_st() {
        assert_eq!(
            parse(b"\x1b]0;title\x07\x1b]8;;http://x\x1b\\"),
            [
                Action::Osc(b"0;title".to_vec(), true),
                Action::Osc(b"8;;http://x".to_vec(), false),
            ]
        );
    }

    #[test]
    fn osc_params_split_on_semicolons() {
        let osc = Osc {
            data: b"8;;a;b",
            bell_terminated: false,
        };
        assert_eq!(
            osc.params().collect::<Vec<_>>(),
            [&b"8"[..], b"", b"a", b"b"]
        );
    }

    #[test]
    fn osc_drops_c0_controls_from_its_payload() {
        assert_eq!(
            parse(b"\x1b]2;a\x08\x0db\x07"),
            [Action::Osc(b"2;ab".to_vec(), true)]
        );
    }

    #[test]
    fn esc_without_backslash_cancels_an_osc() {
        assert_eq!(
            parse(b"\x1b]2;lost\x1b[1m"),
            [csi(&[&[1]], None, b"", b'm')]
        );
    }

    #[test]
    fn esc_without_backslash_cancels_a_dcs() {
        assert_eq!(
            parse(b"\x1bPqdata\x1b7"),
            [
                Action::Hook(vec![], vec![], b'q'),
                Action::Put(b"data".to_vec()),
                Action::Unhook(End::Cancelled),
                Action::Esc(vec![], b'7'),
            ]
        );
    }

    #[test]
    fn osc_past_the_cap_is_dropped_and_its_buffer_bounded() {
        let (counter, parser) = feed_long(b"\x1b]52;c;", MAX_OSC_BYTES + OVERFLOW, b"\x07");
        assert_eq!(counter.osc, 0);
        assert!(parser.osc.len() <= MAX_OSC_BYTES);
    }

    #[test]
    fn osc_exactly_at_the_cap_is_dispatched() {
        let (counter, _) = feed_long(b"\x1b]2;", MAX_OSC_BYTES - b"2;".len(), b"\x07");
        assert_eq!(counter.osc, 1);
    }

    #[test]
    fn dcs_hooks_streams_its_payload_and_unhooks() {
        assert_eq!(
            parse(b"\x1bP1$qm\x1b\\"),
            [
                Action::Hook(vec![vec![1]], b"$".to_vec(), b'q'),
                Action::Put(b"m".to_vec()),
                Action::Unhook(End::Complete),
            ]
        );
    }

    #[test]
    fn dcs_past_the_cap_is_cut_and_reported_truncated() {
        let (counter, _) = feed_long(b"\x1bPq", MAX_DCS_BYTES + OVERFLOW, b"\x1b\\");
        assert_eq!(counter.dcs, MAX_DCS_BYTES);
        assert_eq!(counter.end, Some(End::Truncated));
    }

    #[test]
    fn malformed_dcs_header_is_ignored_until_st() {
        assert_eq!(parse(b"\x1bP1?2qpayload\x1b\\ok"), [print("ok")]);
    }

    #[test]
    fn apc_streams_its_payload() {
        assert_eq!(
            parse(b"\x1b_Ga=T;AAAA\x1b\\"),
            [
                Action::ApcStart,
                Action::ApcPut(b"Ga=T;AAAA".to_vec()),
                Action::ApcEnd(End::Complete),
            ]
        );
    }

    #[test]
    fn apc_past_the_cap_is_cut_and_reported_truncated() {
        let (counter, _) = feed_long(b"\x1b_G", MAX_APC_BYTES + OVERFLOW, b"\x1b\\");
        assert_eq!(counter.apc, MAX_APC_BYTES);
        assert_eq!(counter.end, Some(End::Truncated));
    }

    #[test]
    fn apc_cancelled_by_can_says_so() {
        assert_eq!(
            parse(b"\x1b_Gx\x18"),
            [
                Action::ApcStart,
                Action::ApcPut(b"Gx".to_vec()),
                Action::ApcEnd(End::Cancelled),
                Action::Execute(CAN),
            ]
        );
    }

    #[test]
    fn sos_and_pm_are_swallowed() {
        assert_eq!(parse(b"\x1bXsecret\x1b\\\x1b^pm\x1b\\ok"), [print("ok")]);
    }

    /// Fuzz regression: an ignored string after a cancelled APC computed its
    /// room from the APC's byte count and underflowed.
    #[test]
    fn pm_right_after_a_cancelled_apc_is_swallowed() {
        assert_eq!(
            parse(b"\x1b_abc\x1b^x\x1b\\ok"),
            [
                Action::ApcStart,
                Action::ApcPut(b"abc".to_vec()),
                Action::ApcEnd(End::Cancelled),
                print("ok"),
            ]
        );
    }

    /// Bytes weighted towards the ones that change state.
    fn stream() -> impl Strategy<Value = Vec<u8>> {
        let interesting = prop::sample::select(
            b"\x1b\x07\x18\x1a\x7f\n[]P_X^\\;:?019m q$\xc2\x85\x9b\xe2\x82\xac\xf0\x9f\x8e\x89\xff"
                .to_vec(),
        );
        prop::collection::vec(prop_oneof![interesting, any::<u8>(), Just(b'a')], 0..400)
    }

    proptest! {
        #[test]
        fn any_chunking_yields_the_same_actions(
            bytes in stream(),
            cuts in prop::collection::vec(any::<prop::sample::Index>(), 0..8),
        ) {
            let whole = parse(&bytes);
            let mut points: Vec<usize> = cuts.iter().map(|i| i.index(bytes.len() + 1)).collect();
            points.sort_unstable();
            let mut rec = Recorder::default();
            let mut parser = Parser::new();
            let mut from = 0;
            for to in points.into_iter().chain([bytes.len()]) {
                parser.feed(&bytes[from..to], &mut rec);
                from = to;
            }
            prop_assert_eq!(&rec.0, &whole);
            prop_assert_eq!(parse_bytewise(&bytes), whole);
        }

        #[test]
        fn text_without_controls_is_printed_unchanged(text in "[^\\p{Cc}]{0,200}") {
            let expected = if text.is_empty() { vec![] } else { vec![print(&text)] };
            prop_assert_eq!(parse(text.as_bytes()), expected);
        }

        #[test]
        fn arbitrary_bytes_print_their_lossy_decoding_when_free_of_controls(
            mut bytes in prop::collection::vec(prop_oneof![0x20u8..DEL, 0x80u8..=0xFF], 0..400),
        ) {
            let lossy = String::from_utf8_lossy(&bytes).into_owned();
            prop_assume!(!lossy.chars().any(char::is_control));
            // A control byte settles a UTF-8 sequence cut at the end of the input.
            bytes.push(b'\n');
            let printed: String = parse(&bytes)
                .into_iter()
                .filter_map(|a| match a {
                    Action::Print(s) => Some(s),
                    _ => None,
                })
                .collect();
            prop_assert_eq!(printed, lossy);
        }
    }
}
