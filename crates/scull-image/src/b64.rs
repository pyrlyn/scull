//! Streaming base64 for payloads that arrive in pieces (kitty chunks, iTerm2
//! `FilePart`s, parser slices): complete quads are decoded as they come and
//! up to three leftover characters wait for the next piece.

use base64::Engine as _;
use base64::alphabet::STANDARD;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};

use crate::ImageError;

/// Characters per base64 quad.
const QUAD: usize = 4;
/// Bytes a full quad decodes to.
const QUAD_BYTES: usize = 3;
const PAD: u8 = b'=';

/// Clients often drop the final padding; accepting it costs nothing.
const ENGINE: GeneralPurpose = GeneralPurpose::new(
    &STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// Decodes base64 incrementally into a buffer capped at `max` bytes.
#[derive(Debug)]
pub(crate) struct Base64Sink {
    carry: Vec<u8>,
    out: Vec<u8>,
    max: usize,
    /// Padding ends the stream; data after it must fail however the input
    /// was sliced, as it does when decoded in one piece.
    padded: bool,
}

impl Base64Sink {
    pub(crate) fn new(max: usize) -> Self {
        Self {
            carry: Vec::with_capacity(QUAD),
            out: Vec::new(),
            max,
            padded: false,
        }
    }

    /// Decodes the complete quads of `carry + data`.
    pub(crate) fn put(&mut self, mut data: &[u8]) -> Result<(), ImageError> {
        if !self.carry.is_empty() {
            let (head, rest) = data.split_at((QUAD - self.carry.len()).min(data.len()));
            self.carry.extend_from_slice(head);
            data = rest;
            if self.carry.len() == QUAD {
                let quad = std::mem::take(&mut self.carry);
                self.decode(&quad)?;
            }
        }
        let (whole, tail) = data.split_at(data.len() - data.len() % QUAD);
        self.decode(whole)?;
        self.carry.extend_from_slice(tail);
        Ok(())
    }

    /// Decodes the unpadded tail and returns everything decoded.
    pub(crate) fn finish(mut self) -> Result<Vec<u8>, ImageError> {
        let tail = std::mem::take(&mut self.carry);
        self.decode(&tail)?;
        Ok(self.out)
    }

    fn decode(&mut self, input: &[u8]) -> Result<(), ImageError> {
        if input.is_empty() {
            return Ok(());
        }
        if self.padded {
            return Err(ImageError::Base64);
        }
        self.padded = input.last() == Some(&PAD);
        let room = self.max.saturating_sub(self.out.len());
        if input.len().div_ceil(QUAD) * QUAD_BYTES > room.saturating_add(QUAD_BYTES) {
            return Err(ImageError::PayloadTooLarge(self.max));
        }
        ENGINE
            .decode_vec(input, &mut self.out)
            .map_err(|_| ImageError::Base64)?;
        if self.out.len() > self.max {
            return Err(ImageError::PayloadTooLarge(self.max));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_in(pieces: &[&[u8]], max: usize) -> Result<Vec<u8>, ImageError> {
        let mut sink = Base64Sink::new(max);
        for piece in pieces {
            sink.put(piece)?;
        }
        sink.finish()
    }

    #[test]
    fn pieces_split_anywhere_decode_alike() {
        let text: &[u8] = b"aGVsbG8gd29ybGQ="; // "hello world"
        for cut in 0..=text.len() {
            let (a, b) = text.split_at(cut);
            assert_eq!(decode_in(&[a, b], 64).unwrap(), b"hello world", "cut {cut}");
        }
        for cut in 0..text.len() - 1 {
            let (a, rest) = text.split_at(cut);
            let (b, c) = rest.split_at(1);
            assert_eq!(decode_in(&[a, b, c], 64).unwrap(), b"hello world");
        }
    }

    #[test]
    fn missing_padding_is_accepted() {
        assert_eq!(decode_in(&[b"aGk"], 8).unwrap(), b"hi");
    }

    #[test]
    fn data_after_padding_is_an_error_in_any_slicing() {
        assert_eq!(decode_in(&[b"BA==AAAA"], 16), Err(ImageError::Base64));
        assert_eq!(decode_in(&[b"BA==", b"AAAA"], 16), Err(ImageError::Base64));
        assert_eq!(decode_in(&[b"BA==", b"AA"], 16), Err(ImageError::Base64));
    }

    #[test]
    fn garbage_is_an_error() {
        assert_eq!(decode_in(&[b"a*b="], 8), Err(ImageError::Base64));
    }

    #[test]
    fn cap_is_enforced() {
        assert_eq!(
            decode_in(&[b"aGVsbG8gd29ybGQ="], 4),
            Err(ImageError::PayloadTooLarge(4))
        );
        assert_eq!(decode_in(&[b"aGVsbG8gd29ybGQ="], 11).unwrap().len(), 11);
    }
}
