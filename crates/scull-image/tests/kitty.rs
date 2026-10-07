//! Golden tests for the kitty graphics decoder: formats and compression,
//! chunked uploads, image numbers, placements and the cursor, every delete
//! selector, quiet levels and the exact reply bytes, the caps, and a property
//! test that slicing an APC never changes what it does.

// Helpers outside #[test] functions are still test code; a failure should abort loudly.
#![allow(clippy::unwrap_used)]

mod support;

use std::io::Write as _;

use flate2::Compression;
use flate2::write::ZlibEncoder;
use proptest::prelude::*;
use scull_image::{
    DEFAULT_QUOTA_BYTES, ImageId, ImageStore, KittyContext, KittyDecoder, KittyOutcome,
    MAX_IMAGE_SIDE, Placement,
};
use support::{RGBA_2X2, b64, png_2x2};

const CTX: KittyContext = KittyContext {
    row: 5,
    col: 3,
    top: 2,
    screen_rows: 24,
    cell_width: 10,
    cell_height: 20,
};

struct Term {
    dec: KittyDecoder,
    store: ImageStore,
    ctx: KittyContext,
}

impl Term {
    fn new() -> Self {
        Self {
            dec: KittyDecoder::new(),
            store: ImageStore::new(DEFAULT_QUOTA_BYTES),
            ctx: CTX,
        }
    }

    /// Sends one APC body (`G...`) in one slice.
    fn apc(&mut self, body: &[u8]) -> KittyOutcome {
        self.apc_in(body, body.len().max(1), true)
    }

    fn apc_in(&mut self, body: &[u8], cut: usize, complete: bool) -> KittyOutcome {
        self.dec.start();
        for piece in body.chunks(cut) {
            self.dec.put(piece);
        }
        self.dec.finish(complete, &mut self.store, &self.ctx)
    }

    fn cmd(&mut self, control: &str, payload: &[u8]) -> KittyOutcome {
        self.apc(format!("G{control};{}", b64(payload)).as_bytes())
    }

    fn reply(&mut self, control: &str, payload: &[u8]) -> String {
        let out = self.cmd(control, payload);
        String::from_utf8(out.reply.unwrap_or_default()).unwrap()
    }

    fn pixels(&self, id: u32) -> Vec<u8> {
        self.store
            .peek(ImageId::kitty(id))
            .unwrap()
            .pixels()
            .to_vec()
    }

    fn placements(&self) -> Vec<Placement> {
        self.store.placements().to_vec()
    }
}

fn ok(head: &str) -> String {
    format!("\x1b_G{head};OK\x1b\\")
}

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(data).unwrap();
    enc.finish().unwrap()
}

/// `RGBA_2X2` without its alpha channel.
fn rgb_2x2() -> Vec<u8> {
    RGBA_2X2
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|px| &px[..3])
        .copied()
        .collect()
}

#[test]
fn rgba_transmit_stores_exact_pixels_and_answers_ok() {
    let mut t = Term::new();
    assert_eq!(t.reply("a=t,f=32,s=2,v=2,i=1", &RGBA_2X2), ok("i=1"));
    assert_eq!(t.pixels(1), RGBA_2X2);
    // `f=32` and `a=t` are the defaults.
    assert_eq!(t.reply("s=2,v=2,i=2", &RGBA_2X2), ok("i=2"));
    assert!(t.placements().is_empty());
}

#[test]
fn rgb_png_and_zlib_formats() {
    let mut t = Term::new();
    assert_eq!(t.reply("f=24,s=2,v=2,i=1", &rgb_2x2()), ok("i=1"));
    let opaque: Vec<u8> = RGBA_2X2
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|px| [px[0], px[1], px[2], 255])
        .collect();
    assert_eq!(t.pixels(1), opaque);
    assert_eq!(t.reply("f=100,i=2", &png_2x2()), ok("i=2"));
    assert_eq!(t.pixels(2), RGBA_2X2);
    assert_eq!(t.reply("f=32,o=z,s=2,v=2,i=3", &zlib(&RGBA_2X2)), ok("i=3"));
    assert_eq!(t.pixels(3), RGBA_2X2);
    assert_eq!(
        t.reply("f=24,o=z,s=2,v=2,i=4", &zlib(&rgb_2x2())),
        ok("i=4")
    );
    assert_eq!(t.reply("f=100,o=z,i=5", &zlib(&png_2x2())), ok("i=5"));
    assert_eq!(t.pixels(5), RGBA_2X2);
}

#[test]
fn chunked_upload_joins_across_apcs() {
    let mut t = Term::new();
    let text = b64(&RGBA_2X2);
    let (a, rest) = text.split_at(8);
    let (b, c) = rest.split_at(4);
    assert_eq!(
        t.apc(format!("Gs=2,v=2,i=7,m=1;{a}").as_bytes()),
        KittyOutcome::default()
    );
    assert_eq!(
        t.apc(format!("Gm=1;{b}").as_bytes()),
        KittyOutcome::default()
    );
    assert!(t.store.is_empty());
    let out = t.apc(format!("Gm=0;{c}").as_bytes());
    assert_eq!(out.reply.unwrap(), ok("i=7").into_bytes());
    assert_eq!(t.pixels(7), RGBA_2X2);
}

#[test]
fn a_chunk_may_change_quiet() {
    let mut t = Term::new();
    let text = b64(&RGBA_2X2);
    let (a, b) = text.split_at(8);
    t.apc(format!("Gs=2,v=2,i=7,m=1;{a}").as_bytes());
    assert_eq!(t.apc(format!("Gm=0,q=1;{b}").as_bytes()).reply, None);
    assert_eq!(t.pixels(7), RGBA_2X2);
}

#[test]
fn quiet_levels() {
    let mut t = Term::new();
    assert_eq!(t.reply("s=2,v=2,i=1,q=1", &RGBA_2X2), "");
    assert!(
        t.reply("s=3,v=2,i=1,q=1", &RGBA_2X2)
            .starts_with("\x1b_Gi=1;ENODATA:")
    );
    assert_eq!(t.reply("s=3,v=2,i=1,q=2", &RGBA_2X2), "");
    // Without `i` or `I` nobody is answered.
    assert_eq!(t.reply("s=2,v=2", &RGBA_2X2), "");
    assert_eq!(t.reply("s=3,v=2", &RGBA_2X2), "");
    assert_eq!(t.store.len(), 2);
}

#[test]
fn error_replies_name_the_code() {
    let mut t = Term::new();
    let cases: [(&str, &[u8], &str); 9] = [
        ("s=2,v=2,i=1,I=2", &RGBA_2X2, "i=1,I=2;EINVAL:"),
        ("a=p,i=9", b"", "i=9;ENOENT:"),
        ("f=7,s=2,v=2,i=1", &RGBA_2X2, "i=1;EINVAL:"),
        ("t=f,i=1", b"/etc/passwd", "i=1;EINVAL:"),
        ("a=f,i=1", b"", "i=1;EINVAL:"),
        ("a=t,s=10001,v=1,i=1", b"", "i=1;EFBIG:"),
        ("f=100,i=1", b"not a png", "i=1;EBADMSG:"),
        ("f=32,o=z,s=2,v=2,i=1", b"not zlib", "i=1;EBADMSG:"),
        ("U=1,a=T,s=2,v=2,i=1", &RGBA_2X2, "i=1;EINVAL:"),
    ];
    for (control, payload, want) in cases {
        let got = t.reply(control, payload);
        assert!(
            got.starts_with(&format!("\x1b_G{want}")),
            "{control}: {got:?}"
        );
        assert!(got.ends_with("\x1b\\"), "{control}: {got:?}");
        let msg = &got[got.find(':').unwrap() + 1..got.len() - 2];
        assert!(
            msg.bytes().all(|b| b.is_ascii_graphic() || b == b' '),
            "{msg:?}"
        );
    }
    assert!(t.store.is_empty());
}

#[test]
fn bad_base64_is_ebadmsg() {
    let mut t = Term::new();
    let out = t.apc(b"Gs=2,v=2,i=1;@@@@");
    assert!(
        String::from_utf8(out.reply.unwrap())
            .unwrap()
            .starts_with("\x1b_Gi=1;EBADMSG:")
    );
}

#[test]
fn image_numbers_get_fresh_ids() {
    let mut t = Term::new();
    let first = t.reply("s=2,v=2,I=7", &RGBA_2X2);
    assert_eq!(first, ok(&format!("i={},I=7", u32::MAX)));
    // A second upload under the same number gets a new id; `I` then means it.
    let second = t.reply("s=2,v=2,I=7", &RGBA_2X2);
    assert_eq!(second, ok(&format!("i={},I=7", u32::MAX - 1)));
    assert_eq!(t.store.len(), 2);
    let put = t.reply("a=p,I=7,p=3", b"");
    assert_eq!(put, ok(&format!("i={},I=7,p=3", u32::MAX - 1)));
    assert_eq!(t.placements()[0].image, ImageId::kitty(u32::MAX - 1));
    assert!(t.reply("a=p,I=8", b"").starts_with("\x1b_Gi=0,I=8;ENOENT:"));
}

#[test]
fn query_decodes_without_storing() {
    let mut t = Term::new();
    assert_eq!(t.reply("a=q,s=2,v=2,i=31", &RGBA_2X2), ok("i=31"));
    assert!(t.store.is_empty());
    assert!(t.reply("a=q,s=2,v=1,i=31", &RGBA_2X2).contains("ENODATA"));
}

#[test]
fn transmit_and_place_sizes_by_cells_and_moves_the_cursor() {
    let mut t = Term::new();
    // 25x45 px in 10x20 cells is 3x3 cells.
    let rgba = vec![9; 25 * 45 * 4];
    let out = t.cmd("a=T,s=25,v=45,i=1,z=-1", &rgba);
    assert_eq!(out.reply.unwrap(), ok("i=1").into_bytes());
    assert_eq!(out.cursor, Some((3, 3)));
    let p = t.placements()[0];
    assert_eq!((p.row, p.col, p.cols, p.rows, p.z), (5, 3, 3, 3, -1));
    assert_eq!((p.crop.width, p.crop.height), (25, 45));
    // `C=1` keeps the cursor; an anonymous image places too, silently.
    let out = t.cmd("a=T,s=25,v=45,C=1", &rgba);
    assert_eq!(out, KittyOutcome::default());
    assert_eq!(t.placements().len(), 2);
}

#[test]
fn placement_geometry() {
    let mut t = Term::new();
    t.cmd("s=40,v=20,i=1", &vec![0; 40 * 20 * 4]);
    // Only `c`: rows follow the aspect ratio (4 cols = 40 px wide, 20 px tall).
    assert_eq!(t.cmd("a=p,i=1,c=4", b"").cursor, Some((4, 1)));
    assert_eq!(t.cmd("a=p,i=1,c=8", b"").cursor, Some((8, 2)));
    assert_eq!(t.cmd("a=p,i=1,r=2", b"").cursor, Some((8, 2)));
    assert_eq!(t.cmd("a=p,i=1,c=5,r=7", b"").cursor, Some((5, 7)));
    // The crop is clamped to the image; the offset to the cell.
    t.cmd("a=p,i=1,x=30,y=5,w=50,X=99,Y=3", b"");
    let p = *t.placements().last().unwrap();
    assert_eq!(
        (p.crop.x, p.crop.y, p.crop.width, p.crop.height),
        (30, 5, 10, 15)
    );
    assert_eq!((p.offset_x, p.offset_y, p.cols, p.rows), (9, 3, 2, 1));
    assert!(t.reply("a=p,i=1,x=40", b"").contains("EINVAL"));
    // Hostile sizes are clamped.
    assert_eq!(
        t.cmd("a=p,i=1,c=4294967295", b"").cursor,
        Some((MAX_IMAGE_SIDE, MAX_IMAGE_SIDE))
    );
}

#[test]
fn same_placement_id_moves_the_placement() {
    let mut t = Term::new();
    t.cmd("s=2,v=2,i=1", &RGBA_2X2);
    t.cmd("a=p,i=1,p=4", b"");
    t.ctx.row = 9;
    t.cmd("a=p,i=1,p=4", b"");
    assert_eq!(t.placements().len(), 1);
    assert_eq!(t.placements()[0].row, 9);
    t.cmd("a=p,i=1", b"");
    t.cmd("a=p,i=1", b"");
    assert_eq!(t.placements().len(), 3);
}

#[test]
fn retransmit_replaces_the_image_and_its_placements() {
    let mut t = Term::new();
    t.cmd("a=T,s=2,v=2,i=1", &RGBA_2X2);
    let generation = t.store.peek(ImageId::kitty(1)).unwrap().generation();
    t.cmd("f=24,s=2,v=2,i=1", &rgb_2x2());
    assert!(t.placements().is_empty());
    assert_ne!(
        t.store.peek(ImageId::kitty(1)).unwrap().generation(),
        generation
    );
}

/// Three 1x1 images placed at (row 5, col 3), (row 3, col 0) and (row 40,
/// col 7) with z = 0, 1 and 2.
fn three_placed() -> Term {
    let mut t = Term::new();
    for (i, (row, col)) in [(5, 3), (3, 0), (40, 7)].into_iter().enumerate() {
        t.ctx.row = row;
        t.ctx.col = col;
        t.cmd(&format!("a=T,s=1,v=1,i={},p=1,z={i}", i + 1), &[1, 2, 3, 4]);
    }
    t.ctx = CTX;
    t
}

fn ids(t: &Term) -> Vec<u64> {
    t.placements().iter().map(|p| p.image.0).collect()
}

#[test]
fn delete_selectors() {
    // Screen top is line 2 and 24 rows tall, so cell (x, y) is line y + 1,
    // column x - 1; the third image is below the screen.
    let cases: [(&str, &[u64]); 12] = [
        ("a=d", &[3]),
        ("a=d,d=a", &[3]),
        ("a=d,d=i,i=2", &[1, 3]),
        ("a=d,d=i,i=2,p=9", &[1, 2, 3]),
        ("a=d,d=c", &[2, 3]),
        ("a=d,d=p,x=4,y=4", &[2, 3]),
        ("a=d,d=q,x=4,y=4,z=1", &[1, 2, 3]),
        ("a=d,d=x,x=1", &[1, 3]),
        ("a=d,d=y,y=2", &[1, 3]),
        ("a=d,d=z,z=2", &[1, 2]),
        ("a=d,d=r,x=2,y=3", &[1]),
        ("a=d,d=f", &[1, 2, 3]),
    ];
    for (control, left) in cases {
        let mut t = three_placed();
        assert_eq!(t.cmd(control, b""), KittyOutcome::default(), "{control}");
        assert_eq!(ids(&t), left, "{control}");
        assert_eq!(t.store.len(), 3, "lower case keeps images: {control}");
    }
}

#[test]
fn upper_case_deletes_free_unplaced_images() {
    let mut t = three_placed();
    t.cmd("a=d,d=A", b"");
    assert_eq!(ids(&t), [3]);
    assert_eq!(t.store.len(), 1);
    // An image that was never placed is freed by id too.
    let mut t = Term::new();
    t.cmd("s=2,v=2,i=5", &RGBA_2X2);
    t.cmd("a=d,d=I,i=5", b"");
    assert!(t.store.is_empty());
    // Freeing one placement keeps an image another placement still shows.
    let mut t = Term::new();
    t.cmd("a=T,s=2,v=2,i=5,p=1", &RGBA_2X2);
    t.cmd("a=p,i=5,p=2", b"");
    t.cmd("a=d,d=I,i=5,p=1", b"");
    assert_eq!(t.store.len(), 1);
    t.cmd("a=d,d=N,I=3", b"");
    assert_eq!(t.store.len(), 1);
    t.cmd("s=2,v=2,I=3", &RGBA_2X2);
    t.cmd("a=d,d=N,I=3", b"");
    assert_eq!(t.store.len(), 1);
}

#[test]
fn a_new_command_or_a_cut_apc_aborts_an_upload() {
    let mut t = Term::new();
    let text = b64(&RGBA_2X2);
    let (a, b) = text.split_at(8);
    t.apc(format!("Gs=2,v=2,i=1,m=1;{a}").as_bytes());
    t.cmd("a=d", b"");
    // The tail now reads as a fresh, size-less upload: nothing is stored.
    t.apc(format!("Gm=0;{b}").as_bytes());
    assert!(t.store.is_empty());

    t.apc(format!("Gs=2,v=2,i=1,m=1;{a}").as_bytes());
    t.apc_in(format!("Gm=1;{b}").as_bytes(), 3, false);
    assert_eq!(t.apc(b"Gm=0;"), KittyOutcome::default());
    assert!(t.store.is_empty());
}

#[test]
fn a_failed_upload_answers_once_at_the_end() {
    let mut t = Term::new();
    t.apc(b"Gs=2,v=2,i=1,m=1;@@@@");
    assert_eq!(t.apc(b"Gm=1;AAAA"), KittyOutcome::default());
    let out = t.apc(b"Gm=0;AAAA");
    assert!(
        String::from_utf8(out.reply.unwrap())
            .unwrap()
            .contains("EBADMSG")
    );
}

#[test]
fn non_graphics_and_broken_apcs_are_ignored() {
    let mut t = Term::new();
    for body in [
        &b""[..],
        b"X;hello",
        b"G",
        b"Gi=;AAAA",
        b"Gi=1,s=x;AAAA",
        b"Gnokey;AAAA",
        b"Gi=99999999999;AAAA",
    ] {
        assert_eq!(t.apc(body), KittyOutcome::default(), "{body:?}");
    }
    let long = format!("Gi=1,{};AAAA", "x=1,".repeat(300));
    assert_eq!(t.apc(long.as_bytes()), KittyOutcome::default());
    assert!(t.store.is_empty());
}

#[test]
fn zlib_bombs_stop_at_the_declared_size() {
    let mut t = Term::new();
    let bomb = zlib(&vec![0; 1 << 20]);
    assert!(
        t.reply("o=z,s=2,v=2,i=1", &bomb)
            .starts_with("\x1b_Gi=1;EFBIG:")
    );
    // A short stream is a length error, not a panic.
    assert!(
        t.reply("o=z,s=2,v=2,i=1", &zlib(&[0; 3]))
            .contains("ENODATA")
    );
}

#[test]
fn unknown_keys_are_ignored() {
    let mut t = Term::new();
    assert_eq!(t.reply("s=2,v=2,i=1,K=5,", &RGBA_2X2), ok("i=1"));
}

proptest! {
    #[test]
    fn slicing_never_changes_the_outcome(cut in 1usize..40, more in any::<bool>()) {
        let body = format!("Ga=T,s=2,v=2,i=3,p=2,m={};{}", u8::from(more), b64(&RGBA_2X2));
        let mut whole = Term::new();
        let mut sliced = Term::new();
        let want = whole.apc(body.as_bytes());
        prop_assert_eq!(sliced.apc_in(body.as_bytes(), cut, true), want);
        prop_assert_eq!(sliced.placements(), whole.placements());
        prop_assert_eq!(sliced.store.len(), whole.store.len());
    }
}
