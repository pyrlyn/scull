//! The crate's error enum: one vocabulary for the store and every decoder so
//! the terminal layer can log or answer a failure the same way whatever
//! protocol produced it.

use thiserror::Error;

use crate::ImageId;

/// Why an image was not decoded, stored or placed. Every variant is
/// recoverable: the terminal drops the image and carries on.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ImageError {
    /// The image (width, height) is wider, taller or larger than the caps allow.
    #[error("image of {0}x{1} pixels exceeds the size cap")]
    TooLarge(u64, u64),
    /// The encoded payload grew past this cap in bytes.
    #[error("payload exceeds {0} bytes")]
    PayloadTooLarge(usize),
    /// The image has no pixels.
    #[error("image has no pixels")]
    Empty,
    /// Raw pixel data (expected, actual bytes) does not match the declared size.
    #[error("pixel data of {1} bytes, expected {0}")]
    BadLength(usize, usize),
    /// The payload is not valid base64.
    #[error("invalid base64 payload")]
    Base64,
    /// The file is none of the formats this crate decodes.
    #[error("unrecognised image format")]
    UnknownFormat,
    /// A format decoder rejected the data.
    #[error("corrupt image data: {0}")]
    Decode(String),
    /// The image (needed bytes) does not fit the store's quota even when empty.
    #[error("image of {0} bytes exceeds the quota of {1}")]
    Quota(usize, usize),
    /// No stored image has this id.
    #[error("no image with id {0:?}")]
    NotFound(ImageId),
    /// The command is syntactically wrong.
    #[error("malformed command: {0}")]
    Malformed(&'static str),
    /// The command is valid but this terminal does not implement it.
    #[error("unsupported: {0}")]
    Unsupported(&'static str),
}
