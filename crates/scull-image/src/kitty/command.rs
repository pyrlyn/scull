//! The control data of a kitty graphics command (`key=value,...` before the
//! `;`), parsed into one flat struct with the spec's defaults. Written from
//! the protocol specification
//! (<https://sw.kovidgoyal.net/kitty/graphics-protocol/>), not from kitty's
//! GPL code.

use crate::{Crop, ImageError};

/// Longest control data accepted; every key with a 32-bit value fits in a
/// third of it.
pub(crate) const MAX_CONTROL_BYTES: usize = 1024;

/// `f` values.
pub(crate) const FORMAT_RGB: u32 = 24;
pub(crate) const FORMAT_RGBA: u32 = 32;
pub(crate) const FORMAT_PNG: u32 = 100;

/// One parsed command. Field names follow the spec's key letters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Command {
    /// `a`: t, T, q, p, d (f, a, c are animation and unsupported).
    pub action: u8,
    /// `q`: 1 hides OK replies, 2 hides errors too.
    pub quiet: u32,
    /// `f`: 24 (RGB), 32 (RGBA) or 100 (PNG).
    pub format: u32,
    /// `t`: only `d` (direct) is supported.
    pub medium: u8,
    /// `o=z`: zlib-compressed payload.
    pub compressed: bool,
    /// `m=1`: more chunks follow.
    pub more: bool,
    /// `s`, `v`: raw pixel width and height.
    pub width: u32,
    pub height: u32,
    /// `i`, `I`, `p`: image id, image number, placement id.
    pub id: u32,
    pub number: u32,
    pub placement: u32,
    /// `x`, `y`, `w`, `h`: source rectangle; for deletes, `x`, `y` name a
    /// cell or an id range.
    pub crop: Crop,
    /// `X`, `Y`: pixel offset inside the first cell.
    pub offset_x: u32,
    pub offset_y: u32,
    /// `c`, `r`: placement size in cells.
    pub cols: u32,
    pub rows: u32,
    /// `C=1`: leave the cursor where it is.
    pub no_move: bool,
    /// `U=1`, or `P` (relative placement): not supported.
    pub unsupported_placement: bool,
    /// `z`: stacking order.
    pub z: i32,
    /// `d`: what to delete.
    pub delete: u8,
    /// Any key besides `m` and `q`, which tells a new command from the next
    /// chunk of a transfer.
    pub other_keys: bool,
}

fn number<T: std::str::FromStr>(value: &[u8]) -> Result<T, ImageError> {
    std::str::from_utf8(value)
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or(ImageError::Malformed("kitty value"))
}

fn letter(value: &[u8]) -> Result<u8, ImageError> {
    match *value {
        [c] => Ok(c),
        _ => Err(ImageError::Malformed("kitty value")),
    }
}

/// Parses the control data. Unknown keys are ignored so newer clients keep
/// working; malformed values are errors.
pub(crate) fn parse(control: &[u8]) -> Result<Command, ImageError> {
    let mut cmd = Command {
        action: b't',
        format: FORMAT_RGBA,
        medium: b'd',
        delete: b'a',
        ..Command::default()
    };
    // Leading and trailing commas are "undefined"; tolerate them.
    for pair in control.split(|&b| b == b',').filter(|p| !p.is_empty()) {
        let [key, b'=', value @ ..] = pair else {
            return Err(ImageError::Malformed("kitty key"));
        };
        cmd.other_keys |= !matches!(key, b'm' | b'q');
        match key {
            b'a' => cmd.action = letter(value)?,
            b'q' => cmd.quiet = number(value)?,
            b'f' => cmd.format = number(value)?,
            b't' => cmd.medium = letter(value)?,
            b'o' => cmd.compressed = letter(value)? == b'z',
            b'm' => cmd.more = number::<u32>(value)? == 1,
            b's' => cmd.width = number(value)?,
            b'v' => cmd.height = number(value)?,
            b'i' => cmd.id = number(value)?,
            b'I' => cmd.number = number(value)?,
            b'p' => cmd.placement = number(value)?,
            b'x' => cmd.crop.x = number(value)?,
            b'y' => cmd.crop.y = number(value)?,
            b'w' => cmd.crop.width = number(value)?,
            b'h' => cmd.crop.height = number(value)?,
            b'X' => cmd.offset_x = number(value)?,
            b'Y' => cmd.offset_y = number(value)?,
            b'c' => cmd.cols = number(value)?,
            b'r' => cmd.rows = number(value)?,
            b'C' => cmd.no_move = number::<u32>(value)? == 1,
            b'U' | b'P' => cmd.unsupported_placement |= number::<u32>(value)? != 0,
            b'z' => cmd.z = number(value)?,
            b'd' => cmd.delete = letter(value)?,
            _ => {}
        }
    }
    Ok(cmd)
}

/// The spec's reply: `ESC _G i=<id>[,I=<n>][,p=<p>];<OK or ECODE:msg> ESC \`.
/// None when the command named no image or `q` silences it.
pub(crate) fn reply(cmd: &Command, id: u32, result: &Result<(), ImageError>) -> Option<Vec<u8>> {
    const QUIET_OK: u32 = 1;
    const QUIET_ALL: u32 = 2;
    let silenced = match result {
        Ok(()) => cmd.quiet >= QUIET_OK,
        Err(_) => cmd.quiet >= QUIET_ALL,
    };
    if silenced || (id == 0 && cmd.number == 0) {
        return None;
    }
    let mut out = format!("\x1b_Gi={id}");
    if cmd.number != 0 {
        out += &format!(",I={}", cmd.number);
    }
    if cmd.placement != 0 {
        out += &format!(",p={}", cmd.placement);
    }
    out.push(';');
    match result {
        Ok(()) => out += "OK",
        Err(err) => {
            let code = match err {
                ImageError::NotFound(_) => "ENOENT",
                ImageError::TooLarge(..) | ImageError::PayloadTooLarge(_) => "EFBIG",
                ImageError::Quota(..) => "ENOSPC",
                ImageError::Empty | ImageError::BadLength(..) => "ENODATA",
                ImageError::Malformed(_) | ImageError::Unsupported(_) => "EINVAL",
                ImageError::Base64 | ImageError::UnknownFormat | ImageError::Decode(_) => "EBADMSG",
            };
            // The spec allows only printable ASCII and spaces in the message.
            let msg = err
                .to_string()
                .replace(|c: char| !c.is_ascii_graphic(), " ");
            out += &format!("{code}:{msg}");
        }
    }
    out += "\x1b\\";
    Some(out.into_bytes())
}
