//! Decodes image files (PNG, JPEG) to RGBA8 through the maintained
//! `png` and `zune-jpeg` crates. Each decoder reads the header first so the size
//! caps are checked before the pixel buffer is allocated, and each gets its
//! own allocation limit as a second fence.

use std::io::Cursor;

use zune_jpeg::JpegDecoder;
use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

use crate::image::rgba_len;
use crate::{BYTES_PER_PIXEL, Image, ImageError, MAX_IMAGE_BYTES, MAX_IMAGE_SIDE};

const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";
const JPEG_MAGIC: &[u8] = b"\xff\xd8\xff";

/// Decodes a PNG or JPEG file, recognised by its magic bytes.
pub fn decode(data: &[u8]) -> Result<Image, ImageError> {
    if data.starts_with(PNG_MAGIC) {
        decode_png(data)
    } else if data.starts_with(JPEG_MAGIC) {
        decode_jpeg(data)
    } else {
        Err(ImageError::UnknownFormat)
    }
}

fn corrupt(err: impl std::fmt::Display) -> ImageError {
    ImageError::Decode(err.to_string())
}

/// Decodes a PNG file; kitty's `f=100` lands here directly.
pub(crate) fn decode_png(data: &[u8]) -> Result<Image, ImageError> {
    let limits = png::Limits {
        bytes: MAX_IMAGE_BYTES,
    };
    let mut decoder = png::Decoder::new_with_limits(Cursor::new(data), limits);
    // Palette, low bit depth and 16-bit samples all become 8-bit channels.
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(corrupt)?;
    let (width, height) = (reader.info().width, reader.info().height);
    rgba_len(width.into(), height.into())?;
    // Too small a buffer (unknown size) makes `next_frame` fail cleanly.
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let out = reader.next_frame(&mut buf).map_err(corrupt)?;
    let channels = out.color_type.samples();
    buf.truncate(out.line_size * out.height as usize);
    widen(width, height, channels, &buf)
}

/// Spreads 1 to 4 channels per pixel (grey, grey+alpha, RGB, RGBA) to RGBA.
fn widen(width: u32, height: u32, channels: usize, buf: &[u8]) -> Result<Image, ImageError> {
    let pixels = buf.chunks_exact(channels.max(1)).flat_map(|px| match *px {
        [g] => [g, g, g, u8::MAX],
        [g, a] => [g, g, g, a],
        [r, g, b] => [r, g, b, u8::MAX],
        [r, g, b, a, ..] => [r, g, b, a],
        [] => [0; BYTES_PER_PIXEL],
    });
    Image::from_rgba(width, height, pixels.collect())
}

fn decode_jpeg(data: &[u8]) -> Result<Image, ImageError> {
    let side = MAX_IMAGE_SIDE as usize;
    let options = DecoderOptions::default()
        .set_max_width(side)
        .set_max_height(side)
        .jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(data), options);
    decoder.decode_headers().map_err(corrupt)?;
    let (width, height) = decoder.dimensions().ok_or(ImageError::Empty)?;
    // Past `u32` is past the side cap too, which `rgba_len` then reports.
    let side = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    let (width, height) = (side(width), side(height));
    rgba_len(width.into(), height.into())?;
    let pixels = decoder.decode().map_err(corrupt)?;
    Image::from_rgba(width, height, pixels)
}
