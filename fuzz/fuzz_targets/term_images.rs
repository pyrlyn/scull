//! libFuzzer target for images on the terminal: PTY bytes salted with
//! sixel, iTerm2 and kitty sequences, resizes, viewport scrolls and frame
//! updates. Whatever arrives, replies stay within the queue's cap, the
//! cursor stays on the screen, every placement's image is in the store,
//! and an updated frame shows the placements a fresh one would.

#![no_main]

use std::time::Instant;

use libfuzzer_sys::fuzz_target;

use scull_term::{Frame, Terminal};

/// scull-term's reply queue cap (`MAX_REPLY_BYTES`, crate-private).
const REPLY_CAP: usize = 4096;
/// Largest screen side the input picks; small screens scroll and wrap often.
const MAX_SIDE: u8 = 40;
const SCROLLBACK: usize = 16;
/// Marks a control byte: the next byte picks an operation.
const OP: u8 = 0xFF;
/// Operations other than inserting a fragment.
const OP_RESIZE: u8 = 0;
const OP_FRAME: u8 = 1;
const OP_VIEW: u8 = 2;
const OP_CELL: u8 = 3;
const OPS: u8 = 4;

/// Sequence heads and whole images the raw bytes would rarely assemble.
const FRAGMENTS: &[&[u8]] = &[
    b"\x1bPq",
    b"\x1bP0;1q",
    b"#0;2;100;0;0#0",
    b"~~-~~",
    b"\x1b\\",
    b"\x1b]1337;File=inline=1",
    b";width=3;height=2",
    b";doNotMoveCursor=1",
    b":iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==\x07",
    b"\x1b_Gi=1,f=24,s=2,v=2,a=T;AAAAAAAAAAAAAAAA\x1b\\",
    b"\x1b_Gi=2,f=24,s=2,v=2,a=T,C=1,c=3,r=2,z=-1;AAAAAAAAAAAAAAAA\x1b\\",
    b"\x1b_Ga=p,i=1,X=3,Y=5,x=1,w=1\x1b\\",
    b"\x1b_Ga=d\x1b\\",
    b"\x1b_Ga=q,i=9,s=1,v=1,f=24;AAAA\x1b\\",
    b"\x1b[?1049h",
    b"\x1b[?1049l",
    b"\x1b[2J",
    b"\x1b[3J",
    b"\x1b[2;3r",
    b"\x1b[S",
    b"\x1b[T",
    b"\x1bc",
    b"\r\n",
];

fn side(b: u8) -> u16 {
    u16::from(b % MAX_SIDE + 1)
}

fn check(t: &mut Terminal, frame: &mut Frame, now: Instant) {
    assert!(t.take_replies().len() <= REPLY_CAP);
    let (cursor, grid) = (t.cursor(), t.grid());
    assert!(cursor.row < grid.screen_rows() && cursor.col < grid.cols());
    let store = t.images();
    for p in store.placements() {
        assert!(store.peek(p.image).is_some(), "placement without image");
    }
    if t.update_frame(frame, now) {
        let mut fresh = Frame::new();
        assert!(t.update_frame(&mut fresh, now));
        assert_eq!(frame.placements(), fresh.placements());
        let rows = i64::from(frame.rows());
        for p in frame.placements() {
            assert!(p.row < rows && p.row + i64::from(p.rows) > 0);
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let [cols, rows, rest @ ..] = data else {
        return;
    };
    let Ok(mut t) = Terminal::new(side(*cols), side(*rows), SCROLLBACK) else {
        return;
    };
    let (mut frame, now) = (Frame::new(), Instant::now());
    let mut bytes = rest.iter().copied();
    let mut pending = Vec::new();
    while let Some(b) = bytes.next() {
        if b != OP {
            pending.push(b);
            continue;
        }
        let (Some(op), arg) = (bytes.next(), bytes.next().unwrap_or_default()) else {
            break;
        };
        t.feed(&std::mem::take(&mut pending));
        match op % (OPS + 1) {
            OP_RESIZE => {
                let _ = t.resize(side(arg), side(arg.rotate_left(4)));
            }
            OP_FRAME => check(&mut t, &mut frame, now),
            OP_VIEW => t.scroll_display(isize::from(i8::from_ne_bytes([arg]))),
            OP_CELL => t.set_cell_size(u32::from(arg % 16), u32::from(arg / 8)),
            _ => pending.extend_from_slice(FRAGMENTS[usize::from(arg) % FRAGMENTS.len()]),
        }
    }
    t.feed(&pending);
    check(&mut t, &mut frame, now);
});
