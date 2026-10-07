//! libFuzzer target for the kitty graphics decoder. The input is a sequence
//! of APC bodies separated by ESC (the first byte picks the slice length);
//! each runs through two decoders, one fed whole and one in slices, which
//! must agree on every reply, cursor move and placement while the store
//! stays within its quota and every reply is a well-formed APC.

#![no_main]

use libfuzzer_sys::fuzz_target;

use scull_image::{ImageStore, KittyContext, KittyDecoder};

/// Small enough that eviction runs often.
const QUOTA: usize = 1 << 20;
const ESC: u8 = 0x1b;

const CTX: KittyContext = KittyContext {
    row: 3,
    col: 2,
    top: 0,
    screen_rows: 24,
    cell_width: 8,
    cell_height: 16,
};

fuzz_target!(|data: &[u8]| {
    let Some((&chunk, stream)) = data.split_first() else {
        return;
    };
    let (mut whole, mut sliced) = (KittyDecoder::new(), KittyDecoder::new());
    let (mut whole_store, mut sliced_store) = (ImageStore::new(QUOTA), ImageStore::new(QUOTA));
    for body in stream.split(|&b| b == ESC) {
        whole.start();
        whole.put(body);
        let want = whole.finish(true, &mut whole_store, &CTX);
        sliced.start();
        for piece in body.chunks(usize::from(chunk).max(1)) {
            sliced.put(piece);
        }
        let got = sliced.finish(true, &mut sliced_store, &CTX);
        assert_eq!(got, want);
        if let Some(reply) = &want.reply {
            assert!(reply.starts_with(b"\x1b_Gi=") && reply.ends_with(b"\x1b\\"));
        }
        assert!(whole_store.used_bytes() <= QUOTA);
        assert_eq!(whole_store.placements(), sliced_store.placements());
    }
});
