//! Fixture builders shared by the protocol tests: tiny PNG files
//! encoded on the spot, so the expected pixels are known exactly.

#![allow(dead_code)]

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

/// A 2x2 RGBA image: red, green / blue, half-transparent white.
pub const RGBA_2X2: [u8; 16] = [
    255, 0, 0, 255, 0, 255, 0, 255, //
    0, 0, 255, 255, 255, 255, 255, 128,
];

/// `RGBA_2X2` as a PNG file.
pub fn png_2x2() -> Vec<u8> {
    png_rgba(2, 2, &RGBA_2X2)
}

/// An RGBA PNG file.
pub fn png_rgba(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(rgba).unwrap();
    writer.finish().unwrap();
    out
}

pub fn b64(data: &[u8]) -> String {
    STANDARD.encode(data)
}

/// The JPEG fixture: 16x8 solid red, made with `sips` at quality 90.
pub const RED_JPEG: &[u8] = include_bytes!("../fixtures/red-16x8.jpg");
