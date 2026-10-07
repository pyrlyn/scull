//! iTerm2 inline images (`OSC 1337 ; File=args:base64`). Specification:
//! <https://iterm2.com/documentation-images.html>. The multipart form
//! (`MultipartFile`, `FilePart`, `FileEnd`) for files past one OSC is not
//! handled yet. Non-inline files are downloads, which a terminal core has no
//! business writing, so they are refused. Turning the requested size into
//! cells is the terminal layer's job: it knows the cell metrics.

use crate::b64::Base64Sink;
use crate::{Image, ImageError, MAX_PAYLOAD_BYTES, raster};

const FILE: &[u8] = b"File=";
/// Longest decoded file name kept; the name is only informational.
const MAX_NAME_BYTES: usize = 1024;

/// A requested width or height.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dimension {
    /// The image's own size, or derived from the other side's aspect ratio.
    #[default]
    Auto,
    /// A number of cells.
    Cells(u32),
    /// A number of pixels.
    Pixels(u32),
    /// A percentage of the screen.
    Percent(u32),
}

impl Dimension {
    fn parse(value: &[u8]) -> Result<Self, ImageError> {
        let number = |digits: &[u8]| {
            std::str::from_utf8(digits)
                .ok()
                .and_then(|s| s.parse::<u32>().ok())
                .ok_or(ImageError::Malformed("iTerm2 dimension"))
        };
        Ok(match value {
            b"auto" => Self::Auto,
            _ if value.ends_with(b"px") => Self::Pixels(number(&value[..value.len() - 2])?),
            [digits @ .., b'%'] => Self::Percent(number(digits)?),
            _ => Self::Cells(number(value)?),
        })
    }
}

/// How the client asked for an image to be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItermArgs {
    /// The file name the client sent, decoded from base64; may be empty.
    pub name: Vec<u8>,
    /// Requested width.
    pub width: Dimension,
    /// Requested height.
    pub height: Dimension,
    /// Scale without distortion when both sides are given.
    pub preserve_aspect: bool,
    /// Move the cursor past the image afterwards (the default).
    pub move_cursor: bool,
}

impl ItermArgs {
    fn parse(text: &[u8]) -> Result<Self, ImageError> {
        let mut args = Self {
            name: Vec::new(),
            width: Dimension::Auto,
            height: Dimension::Auto,
            preserve_aspect: true,
            move_cursor: true,
        };
        let mut inline = false;
        for pair in text.split(|&b| b == b';') {
            let mut kv = pair.splitn(2, |&b| b == b'=');
            let (key, value) = (kv.next().unwrap_or_default(), kv.next().unwrap_or_default());
            match key {
                b"name" => {
                    let mut sink = Base64Sink::new(MAX_NAME_BYTES);
                    sink.put(value)?;
                    args.name = sink.finish()?;
                }
                b"width" => args.width = Dimension::parse(value)?,
                b"height" => args.height = Dimension::parse(value)?,
                b"preserveAspectRatio" => args.preserve_aspect = value != b"0",
                b"inline" => inline = value == b"1",
                // A WezTerm extension iTerm2 also honours.
                b"doNotMoveCursor" => args.move_cursor = value != b"1",
                _ => {}
            }
        }
        if !inline {
            return Err(ImageError::Unsupported("iTerm2 file download"));
        }
        Ok(args)
    }
}

/// A decoded inline image and how it should be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItermImage {
    /// The decoded pixels.
    pub image: Image,
    /// The display arguments.
    pub args: ItermArgs,
}

impl ItermImage {
    /// Whether an `OSC 1337` payload (the part after `1337;`) is an image.
    pub fn matches(osc: &[u8]) -> bool {
        osc.starts_with(FILE)
    }

    /// Decodes an `OSC 1337` payload (the part after `1337;`).
    pub fn decode(osc: &[u8]) -> Result<Self, ImageError> {
        let rest = osc
            .strip_prefix(FILE)
            .ok_or(ImageError::Unsupported("OSC 1337 command"))?;
        let colon = rest.iter().position(|&b| b == b':');
        let colon = colon.ok_or(ImageError::Malformed("iTerm2 File= without payload"))?;
        let args = ItermArgs::parse(&rest[..colon])?;
        let mut sink = Base64Sink::new(MAX_PAYLOAD_BYTES);
        sink.put(&rest[colon + 1..])?;
        let image = raster::decode(&sink.finish()?)?;
        Ok(Self { image, args })
    }
}
