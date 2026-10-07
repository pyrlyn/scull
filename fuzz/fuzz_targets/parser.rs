//! PTY bytes through `scull_parser::Parser`, fed whole and in chunks.
//! Fails on a panic, on any delivered payload past its cap, or when the
//! chunked feed records different actions from the whole one.
#![no_main]

use libfuzzer_sys::fuzz_target;
use scull_parser::{Csi, End, Esc, Handler, MAX_INTERMEDIATES, MAX_OSC_BYTES, MAX_PARAMS, Osc, Parser};

/// Serialises actions into one byte log. Adjacent text and payload pieces
/// merge, so the log does not depend on where the input was cut.
#[derive(Default)]
struct Log {
    bytes: Vec<u8>,
    last: u8,
}

impl Log {
    fn tag(&mut self, tag: u8) {
        self.bytes.push(tag);
        self.last = tag;
    }

    fn piece(&mut self, tag: u8, data: &[u8]) {
        assert!(!data.is_empty(), "empty piece {tag}");
        if self.last != tag {
            self.tag(tag);
        }
        self.bytes.extend_from_slice(data);
    }

    fn header(&mut self, tag: u8, csi: &Csi<'_>) {
        assert!(csi.params.len() <= MAX_PARAMS);
        assert!(csi.intermediates.len() <= MAX_INTERMEDIATES);
        self.tag(tag);
        for group in csi.params.iter() {
            for value in group {
                self.bytes.extend_from_slice(&value.to_le_bytes());
            }
            self.bytes.push(b'/');
        }
        self.bytes.extend_from_slice(csi.intermediates);
        self.bytes.extend([csi.private.unwrap_or(0), csi.final_byte, u8::from(csi.truncated)]);
    }

    fn end(&mut self, tag: u8, end: End) {
        self.tag(tag);
        self.bytes.push(end as u8);
    }
}

impl Handler for Log {
    fn print(&mut self, text: &str) {
        self.piece(b'p', text.as_bytes());
    }
    fn execute(&mut self, byte: u8) {
        self.tag(b'x');
        self.bytes.push(byte);
    }
    fn esc_dispatch(&mut self, esc: &Esc<'_>) {
        assert!(esc.intermediates.len() <= MAX_INTERMEDIATES);
        self.tag(b'e');
        self.bytes.extend_from_slice(esc.intermediates);
        self.bytes.push(esc.final_byte);
    }
    fn csi_dispatch(&mut self, csi: &Csi<'_>) {
        self.header(b'c', csi);
    }
    fn osc_dispatch(&mut self, osc: &Osc<'_>) {
        assert!(osc.data.len() <= MAX_OSC_BYTES);
        assert!(osc.data.iter().all(|&b| b >= b' '), "C0 in an OSC payload");
        self.tag(b'o');
        self.bytes.extend_from_slice(osc.data);
        self.bytes.push(u8::from(osc.bell_terminated));
    }
    fn dcs_hook(&mut self, header: &Csi<'_>) {
        self.header(b'h', header);
    }
    fn dcs_put(&mut self, data: &[u8]) {
        self.piece(b'd', data);
    }
    fn dcs_unhook(&mut self, end: End) {
        self.end(b'u', end);
    }
    fn apc_start(&mut self) {
        self.tag(b's');
    }
    fn apc_put(&mut self, data: &[u8]) {
        self.piece(b'a', data);
    }
    fn apc_end(&mut self, end: End) {
        self.end(b'z', end);
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&chunk, input)) = data.split_first() else {
        return;
    };
    let mut whole = Log::default();
    Parser::new().feed(input, &mut whole);

    let mut chunked = Log::default();
    let mut parser = Parser::new();
    for piece in input.chunks(usize::from(chunk).max(1)) {
        parser.feed(piece, &mut chunked);
    }
    assert_eq!(whole.bytes, chunked.bytes, "chunk size {chunk}");
});
