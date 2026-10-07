//! Golden tests for iTerm2 inline images: `File=`, the argument grammar, and
//! the formats the file may be in.

// Helpers outside #[test] functions are still test code; a failure should abort loudly.
#![allow(clippy::unwrap_used)]

mod support;

use scull_image::{Dimension, ImageError, ItermImage, MAX_IMAGE_SIDE, decode_file};
use support::{RED_JPEG, RGBA_2X2, b64, png_2x2, png_rgba};

fn file(args: &str, data: &[u8]) -> Vec<u8> {
    format!("File={args}:{}", b64(data)).into_bytes()
}

#[test]
fn inline_png_decodes_to_exact_pixels() {
    let name = b64(b"pic.png");
    let osc = file(&format!("name={name};size=70;inline=1"), &png_2x2());
    let img = ItermImage::decode(&osc).unwrap();
    assert_eq!((img.image.width(), img.image.height()), (2, 2));
    assert_eq!(img.image.pixels(), RGBA_2X2);
    assert_eq!(img.image.stride(), 8);
    assert_eq!(img.args.name, b"pic.png");
    assert_eq!(
        (img.args.width, img.args.height),
        (Dimension::Auto, Dimension::Auto)
    );
    assert!(img.args.preserve_aspect && img.args.move_cursor);
}

#[test]
fn arguments_parse() {
    let osc = file(
        "inline=1;width=10;height=50%;preserveAspectRatio=0;doNotMoveCursor=1",
        &png_2x2(),
    );
    let img = ItermImage::decode(&osc).unwrap();
    assert_eq!(img.args.width, Dimension::Cells(10));
    assert_eq!(img.args.height, Dimension::Percent(50));
    assert!(!img.args.preserve_aspect && !img.args.move_cursor);
    let osc = file("inline=1;width=64px;height=auto;type=image/png", &png_2x2());
    let img = ItermImage::decode(&osc).unwrap();
    assert_eq!(img.args.width, Dimension::Pixels(64));
    assert_eq!(img.args.height, Dimension::Auto);
}

#[test]
fn download_and_garbage_are_refused() {
    let cases: [(&[u8], ImageError); 7] = [
        (
            &file("name=eA==", &png_2x2()),
            ImageError::Unsupported("iTerm2 file download"),
        ),
        (
            &file("inline=1", b"not an image"),
            ImageError::UnknownFormat,
        ),
        (b"File=inline=1:@@@@", ImageError::Base64),
        (
            b"File=inline=1",
            ImageError::Malformed("iTerm2 File= without payload"),
        ),
        (
            &file("inline=1;width=1e9", &png_2x2()),
            ImageError::Malformed("iTerm2 dimension"),
        ),
        (
            &file("inline=1;height=px", &png_2x2()),
            ImageError::Malformed("iTerm2 dimension"),
        ),
        (b"SetMark", ImageError::Unsupported("OSC 1337 command")),
    ];
    for (osc, err) in cases {
        assert_eq!(ItermImage::decode(osc), Err(err));
    }
}

#[test]
fn matches_only_file() {
    assert!(ItermImage::matches(b"File=inline=1:"));
    assert!(!ItermImage::matches(b"CurrentDir=/"));
}

#[test]
fn oversized_png_is_refused_before_decoding() {
    // A valid header claiming one pixel more than the side cap.
    let wide = MAX_IMAGE_SIDE + 1;
    let png = png_rgba(wide, 1, &vec![0; wide as usize * 4]);
    assert_eq!(decode_file(&png), Err(ImageError::TooLarge(wide.into(), 1)));
}

#[test]
fn jpeg_decodes() {
    let jpeg = decode_file(RED_JPEG).unwrap();
    assert_eq!((jpeg.width(), jpeg.height()), (16, 8));
    for px in jpeg.pixels().as_chunks::<4>().0 {
        assert!(
            px[0] > 240 && px[1] < 16 && px[2] < 16 && px[3] == 255,
            "{px:?}"
        );
    }
}

#[test]
fn grey_and_palette_pngs_widen_to_rgba() {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, 2, 1);
    enc.set_color(png::ColorType::GrayscaleAlpha);
    let mut writer = enc.write_header().unwrap();
    writer.write_image_data(&[10, 20, 30, 40]).unwrap();
    writer.finish().unwrap();
    let img = decode_file(&out).unwrap();
    assert_eq!(img.pixels(), [10, 10, 10, 20, 30, 30, 30, 40]);

    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, 1, 1);
    enc.set_color(png::ColorType::Indexed);
    enc.set_palette(vec![1, 2, 3]);
    let mut writer = enc.write_header().unwrap();
    writer.write_image_data(&[0]).unwrap();
    writer.finish().unwrap();
    assert_eq!(decode_file(&out).unwrap().pixels(), [1, 2, 3, 255]);
}

#[test]
fn truncated_files_are_errors_not_panics() {
    for data in [png_2x2(), RED_JPEG.to_vec()] {
        for len in 0..data.len() {
            // Decoders may accept a file missing its trailer; they must not panic.
            let _ = decode_file(&data[..len]);
        }
    }
}
