//! libFuzzer target for the iTerm2 image decoder and the PNG/JPEG decoders
//! behind it. Arbitrary bytes must produce an image within the caps or an
//! error, never a panic or an allocation past the caps.

#![no_main]

use libfuzzer_sys::fuzz_target;

use scull_image::{ItermImage, MAX_IMAGE_BYTES, decode_file};

fuzz_target!(|data: &[u8]| {
    // Raw bytes reach the format decoders directly; base64 would hide them.
    if let Ok(image) = decode_file(data) {
        assert!(image.pixels().len() <= MAX_IMAGE_BYTES);
    }
    let _ = ItermImage::decode(data);
});
