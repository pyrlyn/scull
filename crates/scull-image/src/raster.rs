//! Decodes image files (PNG, JPEG) to RGBA8 through `rgba-png` (over the
//! maintained `png` crate) and `zune-jpeg`. Each decoder reads the header first so the size
//! caps are checked before the pixel buffer is allocated, and each gets its
//! own allocation limit as a second fence.

use zune_jpeg::JpegDecoder;
use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

use crate::image::rgba_len;
use crate::{Image, ImageError, MAX_IMAGE_BYTES, MAX_IMAGE_SIDE};

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
    let png = rgba_png::read_png_header(data, MAX_IMAGE_BYTES).map_err(corrupt)?;
    let (width, height) = png.size();
    rgba_len(width.into(), height.into())?;
    Image::from_rgba(width, height, png.decode_rgba().map_err(corrupt)?)
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
