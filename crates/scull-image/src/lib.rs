//! Inline images for the terminal core: decoded RGBA images behind an `Arc`,
//! placements kept apart from them, and a store with a byte quota and LRU
//! eviction. The protocol decoders (iTerm2, sixel, kitty) live here too so
//! they can be tested and fuzzed without the terminal state; a leaf crate,
//! because nothing about pixels needs the grid or the parser.
//!
//! Every size that untrusted bytes can choose (pixels, payload bytes, image
//! and placement counts) is checked against a named cap before allocating.

mod b64;
mod error;
mod image;
mod iterm;
mod kitty;
mod placement;
mod raster;
mod sixel;
mod store;

pub use error::ImageError;
pub use image::{Image, ImageId};
pub use iterm::{Dimension, ItermArgs, ItermImage};
pub use kitty::{KittyContext, KittyDecoder, KittyOutcome, MAX_KITTY_CHUNKS};
pub use placement::{Crop, Placement, cells_for};
pub use raster::decode as decode_file;
pub use sixel::{MAX_SIXEL_COLORS, MAX_SIXEL_SIDE, SixelDecoder};
pub use store::ImageStore;

/// Bytes per decoded pixel: every image is RGBA8.
pub const BYTES_PER_PIXEL: usize = 4;

/// Longest side of a decoded image in pixels; kitty uses the same limit.
pub const MAX_IMAGE_SIDE: u32 = 10_000;

/// Largest decoded image in bytes (about 33 million RGBA pixels, enough for
/// an 8K screen) so one image cannot take the whole quota's worth of memory
/// before the quota is even consulted.
pub const MAX_IMAGE_BYTES: usize = 128 << 20;

/// Largest encoded payload (a file or raw pixel data after base64) any
/// protocol may accumulate. Raw RGBA is the largest legitimate encoding.
pub const MAX_PAYLOAD_BYTES: usize = MAX_IMAGE_BYTES;

/// Default image quota of one store, matching kitty's 320 MB per buffer.
pub const DEFAULT_QUOTA_BYTES: usize = 320 << 20;

/// Most images one store keeps, however small they are.
pub const MAX_IMAGES: usize = 4096;

/// Most placements one store keeps; the oldest goes first when full.
pub const MAX_PLACEMENTS: usize = 16_384;
