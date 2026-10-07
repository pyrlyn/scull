//! A decoded image: RGBA8 pixels, never mutated once stored, so the frame
//! and the platform can share it through an `Arc` without a lock.

use crate::{BYTES_PER_PIXEL, ImageError, MAX_IMAGE_BYTES, MAX_IMAGE_SIDE};

/// Bytes per packed RGB pixel.
pub(crate) const RGB_BYTES: usize = 3;

/// Identifies a stored image. Kitty's client-chosen ids occupy the `u32`
/// range; ids the store hands out for sixel and iTerm2 images start above
/// it, so the two can never collide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ImageId(pub u64);

impl ImageId {
    /// The first id [`crate::ImageStore::alloc_id`] hands out.
    pub const FIRST_INTERNAL: Self = Self(1 << u32::BITS);

    /// The id of a kitty image.
    pub fn kitty(id: u32) -> Self {
        Self(u64::from(id))
    }

    /// The kitty id, when this is one.
    pub fn as_kitty(self) -> Option<u32> {
        u32::try_from(self.0).ok()
    }
}

/// An RGBA8 image, rows top to bottom, no padding between rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    width: u32,
    height: u32,
    generation: u64,
    pixels: Vec<u8>,
}

/// The byte size of a `width` x `height` RGBA image, or why it is refused.
/// Called before any pixel buffer is allocated.
pub(crate) fn rgba_len(width: u64, height: u64) -> Result<usize, ImageError> {
    if width == 0 || height == 0 {
        return Err(ImageError::Empty);
    }
    let side = u64::from(MAX_IMAGE_SIDE);
    // Within the side cap the product is at most 4e8, far inside `usize`,
    // and `||` keeps it from being computed otherwise.
    let bytes = |w: u64, h: u64| (w * h) as usize * BYTES_PER_PIXEL;
    if width > side || height > side || bytes(width, height) > MAX_IMAGE_BYTES {
        return Err(ImageError::TooLarge(width, height));
    }
    Ok(bytes(width, height))
}

impl Image {
    /// Wraps RGBA8 pixels, checking the size caps and the buffer length.
    pub fn from_rgba(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, ImageError> {
        let expected = rgba_len(width.into(), height.into())?;
        if pixels.len() != expected {
            return Err(ImageError::BadLength(expected, pixels.len()));
        }
        Ok(Self {
            width,
            height,
            generation: 0,
            pixels,
        })
    }

    /// Expands packed RGB8 (kitty `f=24`) to opaque RGBA.
    pub fn from_rgb(width: u32, height: u32, rgb: &[u8]) -> Result<Self, ImageError> {
        let len = rgba_len(width.into(), height.into())?;
        let expected = len / BYTES_PER_PIXEL * RGB_BYTES;
        if rgb.len() != expected {
            return Err(ImageError::BadLength(expected, rgb.len()));
        }
        let mut pixels = Vec::with_capacity(len);
        for px in rgb.as_chunks::<RGB_BYTES>().0 {
            pixels.extend_from_slice(px);
            pixels.push(u8::MAX);
        }
        Self::from_rgba(width, height, pixels)
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Bytes per row.
    pub fn stride(&self) -> usize {
        self.pixels.len() / self.height as usize
    }

    /// The RGBA8 pixels.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Changes whenever the store replaces the pixels behind an id, so a
    /// texture cache keyed by `(pointer, generation)` never shows stale data.
    /// Zero until the image is stored.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn set_generation(&mut self, generation: u64) {
        self.generation = generation;
    }
}
