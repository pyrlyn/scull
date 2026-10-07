//! libFuzzer target for the sixel decoder. Arbitrary payloads must decode to
//! an image within the side cap or an error, and slicing the payload (the
//! first byte picks the slice length) must not change the result.

#![no_main]

use libfuzzer_sys::fuzz_target;

use scull_image::{MAX_SIXEL_SIDE, SixelDecoder};

fuzz_target!(|data: &[u8]| {
    let Some((&chunk, payload)) = data.split_first() else {
        return;
    };
    let mut whole = SixelDecoder::new(None);
    whole.put(payload);
    let whole = whole.finish();
    if let Ok(image) = &whole {
        assert!(image.width() <= MAX_SIXEL_SIDE && image.height() <= MAX_SIXEL_SIDE);
    }
    let mut sliced = SixelDecoder::new(None);
    for piece in payload.chunks(usize::from(chunk).max(1)) {
        sliced.put(piece);
    }
    assert_eq!(sliced.finish(), whole);
});
