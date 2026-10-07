//! Golden tests for the sixel decoder: colours (default palette, RGB, HLS),
//! repeats, bands, raster attributes, background modes and the caps, plus a
//! property test that slicing the payload never changes the result.

// Helpers outside #[test] functions are still test code; a failure should abort loudly.
#![allow(clippy::unwrap_used)]

use proptest::prelude::*;
use scull_image::{Image, ImageError, MAX_SIXEL_SIDE, SixelDecoder};

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const CLEAR: [u8; 4] = [0, 0, 0, 0];

fn decode(payload: &[u8]) -> Result<Image, ImageError> {
    decode_on(payload, None)
}

fn decode_on(payload: &[u8], background: Option<[u8; 4]>) -> Result<Image, ImageError> {
    let mut dec = SixelDecoder::new(background);
    dec.put(payload);
    dec.finish()
}

fn pixels(img: &Image) -> Vec<[u8; 4]> {
    img.pixels().as_chunks::<4>().0.to_vec()
}

fn size(img: &Image) -> (u32, u32) {
    (img.width(), img.height())
}

#[test]
fn rgb_colour_fills_a_full_band() {
    let img = decode(b"#1;2;100;0;0#1~~").unwrap();
    assert_eq!(size(&img), (2, 6));
    assert_eq!(pixels(&img), vec![RED; 12]);
}

#[test]
fn defining_a_colour_also_selects_it() {
    let img = decode(b"#3;2;0;0;100~").unwrap();
    assert_eq!(pixels(&img), vec![BLUE; 6]);
}

#[test]
fn default_palette_is_the_vt340_one() {
    // Register 2 is 80% / 13% / 13% red; register 0 is black.
    let img = decode(b"#2@#0@").unwrap();
    assert_eq!(pixels(&img), vec![[204, 33, 33, 255], [0, 0, 0, 255]]);
}

#[test]
fn hls_puts_blue_at_zero_and_red_at_120() {
    for (hue, want) in [(0, BLUE), (120, RED), (240, GREEN), (480, RED)] {
        let payload = format!("#1;1;{hue};50;100@");
        let img = decode(payload.as_bytes()).unwrap();
        assert_eq!(pixels(&img), vec![want], "hue {hue}");
    }
    let grey = decode(b"#1;1;0;50;0@").unwrap();
    assert_eq!(pixels(&grey), vec![[128, 128, 128, 255]]);
}

#[test]
fn repeat_and_single_bit() {
    // `@` sets only the top row of the band.
    let img = decode(b"#1;2;0;0;100!5@").unwrap();
    assert_eq!(size(&img), (5, 1));
    assert_eq!(pixels(&img), vec![BLUE; 5]);
    // A repeat count of zero paints once.
    assert_eq!(size(&decode(b"!0@").unwrap()), (1, 1));
}

#[test]
fn newline_starts_the_next_band() {
    let img = decode(b"#1;2;100;100;100~-~").unwrap();
    assert_eq!(size(&img), (1, 12));
    let img = decode(b"#1;2;100;0;0@-@").unwrap();
    assert_eq!(size(&img), (1, 7));
    assert_eq!(pixels(&img)[1], CLEAR);
}

#[test]
fn carriage_return_overpaints() {
    let img = decode(b"#1;2;100;0;0~~$#2;2;0;100;0@").unwrap();
    let px = pixels(&img);
    assert_eq!((px[0], px[1], px[2]), (GREEN, RED, RED));
}

#[test]
fn unpainted_pixels_take_the_background() {
    let img = decode(b"#1;2;100;0;0?~").unwrap();
    assert_eq!(size(&img), (2, 6));
    assert_eq!(&pixels(&img)[..2], [CLEAR, RED]);
    let bg = [1, 2, 3, 255];
    let img = decode_on(b"#1;2;100;0;0?~", Some(bg)).unwrap();
    assert_eq!(&pixels(&img)[..2], [bg, RED]);
}

#[test]
fn raster_attributes_size_the_image() {
    let bg = [9, 9, 9, 255];
    let img = decode_on(b"\"1;1;4;2#1;2;100;0;0@", Some(bg)).unwrap();
    assert_eq!(size(&img), (4, 2));
    let mut want = vec![bg; 8];
    want[0] = RED;
    assert_eq!(pixels(&img), want);
    // Painting past the raster size grows the image.
    let img = decode(b"\"1;1;1;1~~").unwrap();
    assert_eq!(size(&img), (2, 6));
    // Raster attributes after the first sixel are ignored.
    let img = decode(b"@\"1;1;9;9").unwrap();
    assert_eq!(size(&img), (1, 1));
}

#[test]
fn empty_images_are_errors() {
    assert_eq!(decode(b""), Err(ImageError::Empty));
    assert_eq!(decode(b"#1;2;100;0;0"), Err(ImageError::Empty));
    assert_eq!(decode(b"\"1;1;0;5"), Err(ImageError::Empty));
}

#[test]
fn sides_are_capped() {
    let img = decode(b"!99999@").unwrap();
    assert_eq!(size(&img), (MAX_SIXEL_SIDE, 1));
    // 700 bands is 4200 rows; the ones past the cap are dropped.
    let mut payload = b"@".to_vec();
    payload.extend(std::iter::repeat_n(b'-', 700));
    payload.push(b'@');
    assert_eq!(size(&decode(&payload).unwrap()), (1, 1));
    let tall: Vec<u8> = std::iter::repeat_n(&b"~-"[..], 700)
        .flatten()
        .copied()
        .collect();
    assert_eq!(size(&decode(&tall).unwrap()), (1, MAX_SIXEL_SIDE));
}

#[test]
fn hostile_parameters_do_not_panic() {
    for payload in [
        &b"#99999999999999999999999;2;999;999;999~"[..],
        b"#1;1;4294967295;4294967295;4294967295~",
        b"#1;2;1;2;3;4;5;6;7;8~",
        b"!4294967295~!4294967295~",
        b"\"4294967295;4294967295;0;0~",
        b";;;;##!!\"\"$$--",
        b"\x1b\x00\x7f\xff~",
    ] {
        let _ = decode(payload);
    }
}

#[test]
fn colour_registers_past_the_cap_clamp() {
    let img = decode(b"#5000;2;0;100;0~#9999~").unwrap();
    assert_eq!(pixels(&img), vec![GREEN; 12]);
}

fn payload() -> impl Strategy<Value = Vec<u8>> {
    let alphabet = b"#!\";$-0123456789?@ABCDEFGHO_~".to_vec();
    prop::collection::vec(prop::sample::select(alphabet), 0..300)
}

proptest! {
    #[test]
    fn slicing_never_changes_the_image(data in payload(), cut in 1usize..16) {
        let whole = decode(&data);
        let mut dec = SixelDecoder::new(None);
        for piece in data.chunks(cut) {
            dec.put(piece);
        }
        prop_assert_eq!(dec.finish(), whole);
    }
}
