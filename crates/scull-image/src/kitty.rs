//! The kitty graphics protocol (`APC G <control> ; <base64 payload> ST`),
//! implemented from its specification
//! (<https://sw.kovidgoyal.net/kitty/graphics-protocol/>) as a decoder over an
//! [`ImageStore`]: it parses commands, joins chunked uploads, decodes the
//! pixels, stores and places them, and returns the reply and cursor movement
//! for the terminal to apply. The terminal feeds it the parser's APC callbacks
//! and owns everything about the grid.
//!
//! Only direct transmission (`t=d`) is accepted: reading files or shared
//! memory named by the child would let any program make the terminal open
//! paths on its behalf. Animation, virtual (`U=1`) and relative placements
//! are refused with `EINVAL`.

mod command;

use std::io::Read as _;

use flate2::read::ZlibDecoder;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::b64::Base64Sink;
use crate::image::{RGB_BYTES, rgba_len};
use crate::{
    BYTES_PER_PIXEL, Image, ImageError, ImageId, ImageStore, MAX_IMAGE_SIDE, MAX_IMAGES,
    MAX_PAYLOAD_BYTES, Placement, cells_for, raster,
};
use command::{Command, FORMAT_PNG, FORMAT_RGB, FORMAT_RGBA, MAX_CONTROL_BYTES};

/// Most chunks one upload may span. The payload cap bounds the bytes; this
/// bounds the per-chunk work of a stream of empty chunks.
pub const MAX_KITTY_CHUNKS: u32 = 1 << 16;

/// Widest or tallest placement in cells; bounds the renderer's work for one
/// placement whatever `c` and `r` the client asks for.
const MAX_PLACEMENT_CELLS: u32 = MAX_IMAGE_SIDE;

/// Where the command lands, as the terminal sees it when the APC ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KittyContext {
    /// Absolute line of the cursor.
    pub row: u64,
    /// Column of the cursor.
    pub col: u32,
    /// Absolute line of the top screen row; deletes name cells 1-based from it.
    pub top: u64,
    /// Rows on screen, for deleting the visible placements.
    pub screen_rows: u32,
    /// Cell width in pixels.
    pub cell_width: u32,
    /// Cell height in pixels.
    pub cell_height: u32,
}

/// What the terminal must do after a command.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KittyOutcome {
    /// Bytes to write back to the child, if the command asked for an answer.
    pub reply: Option<Vec<u8>>,
    /// The new placement's size in cells (columns, rows); the spec moves the
    /// cursor past it unless the client sent `C=1`.
    pub cursor: Option<(u32, u32)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Phase {
    #[default]
    Start,
    Control,
    Payload,
    /// Not a graphics command, or one too broken to answer.
    Ignore,
}

type Payload = Result<Vec<u8>, ImageError>;
type Done = Result<Option<(u32, u32)>, ImageError>;

/// An upload spanning one or more APCs (`m=1` on all but the last).
#[derive(Debug, Clone)]
struct Transfer {
    cmd: Command,
    /// The first error sticks: later chunks are swallowed and the final one
    /// reports it, so the client sees one answer per upload.
    sink: Result<Base64Sink, ImageError>,
    chunks: u32,
}

/// Decodes kitty graphics commands for one screen buffer. It lives as long
/// as the buffer, since uploads span APCs and image numbers outlive commands.
#[derive(Debug, Clone, Default)]
pub struct KittyDecoder {
    phase: Phase,
    control: Vec<u8>,
    command: Option<Command>,
    transfer: Option<Transfer>,
    /// Client image numbers (`I`) to the ids the terminal gave them.
    numbers: FxHashMap<u32, u32>,
    /// Where the APC being finished lands.
    ctx: KittyContext,
    /// Ids handed out for numbers, counted down from `u32::MAX` to keep
    /// clear of the small ids clients choose themselves.
    issued: u32,
}

impl KittyDecoder {
    /// A decoder with no upload in progress and no image numbers.
    pub fn new() -> Self {
        Self::default()
    }

    /// An APC began.
    pub fn start(&mut self) {
        self.phase = Phase::Start;
        self.control.clear();
        self.command = None;
    }

    /// Feeds a slice of the APC body; slices may split anywhere.
    pub fn put(&mut self, mut data: &[u8]) {
        if self.phase == Phase::Start {
            let Some((&first, rest)) = data.split_first() else {
                return;
            };
            self.phase = if first == b'G' {
                Phase::Control
            } else {
                Phase::Ignore
            };
            data = rest;
        }
        if self.phase == Phase::Control {
            let end = data.iter().position(|&b| b == b';');
            let head = &data[..end.unwrap_or(data.len())];
            if self.control.len() + head.len() > MAX_CONTROL_BYTES {
                return self.abort();
            }
            self.control.extend_from_slice(head);
            let Some(end) = end else {
                return;
            };
            data = &data[end + 1..];
            self.begin();
        }
        if self.phase == Phase::Payload
            && let Some(t) = &mut self.transfer
            && let Ok(sink) = &mut t.sink
            && let Err(err) = sink.put(data)
        {
            t.sink = Err(err);
        }
    }

    /// The APC ended; `complete` is false when the parser cut or cancelled
    /// it, which drops any upload in progress since bytes were lost.
    pub fn finish(
        &mut self,
        complete: bool,
        store: &mut ImageStore,
        ctx: &KittyContext,
    ) -> KittyOutcome {
        let phase = std::mem::take(&mut self.phase);
        self.ctx = *ctx;
        if !complete {
            self.transfer = None;
        }
        if !complete || matches!(phase, Phase::Start | Phase::Ignore) {
            return KittyOutcome::default();
        }
        if phase == Phase::Control {
            self.begin();
        }
        let command = self.command.take();
        if self.transfer.as_ref().is_some_and(|t| t.cmd.more) {
            return KittyOutcome::default();
        }
        if let Some(t) = self.transfer.take() {
            let data = t.sink.and_then(Base64Sink::finish);
            return self.execute(&t.cmd, data, store);
        }
        match command {
            Some(cmd) => self.execute(&cmd, Ok(Vec::new()), store),
            None => KittyOutcome::default(),
        }
    }

    fn abort(&mut self) {
        self.phase = Phase::Ignore;
        self.transfer = None;
    }

    /// The control data is complete: a continuation chunk joins the upload in
    /// progress, anything else starts afresh and abandons it.
    fn begin(&mut self) {
        let Ok(cmd) = command::parse(&self.control) else {
            // Without a parsed id there is nobody to address an error to.
            return self.abort();
        };
        self.phase = Phase::Payload;
        if let Some(t) = &mut self.transfer
            && !cmd.other_keys
        {
            t.cmd.more = cmd.more;
            t.cmd.quiet = t.cmd.quiet.max(cmd.quiet);
            t.chunks += 1;
            if t.chunks > MAX_KITTY_CHUNKS {
                t.sink = Err(ImageError::Malformed("too many chunks"));
            }
            return;
        }
        self.transfer = None;
        if matches!(cmd.action, b't' | b'T' | b'q') {
            let sink = match cmd.medium {
                b'd' => Ok(Base64Sink::new(MAX_PAYLOAD_BYTES)),
                _ => Err(ImageError::Unsupported("file and memory media")),
            };
            let chunks = 1;
            self.transfer = Some(Transfer { cmd, sink, chunks });
        }
        self.command = Some(cmd);
    }

    fn execute(&mut self, cmd: &Command, data: Payload, store: &mut ImageStore) -> KittyOutcome {
        // The id is settled first so even a failure is answered under it.
        let id = match cmd.number {
            _ if cmd.id != 0 => cmd.id,
            0 => 0,
            _ if matches!(cmd.action, b't' | b'T') => self.fresh_id(store),
            n => self.numbers.get(&n).copied().unwrap_or(0),
        };
        let result = self.run(cmd, id, data, store);
        let cursor = result.as_ref().ok().copied().flatten();
        let result = result.map(drop);
        // Deletes are silent unless they fail.
        let silent = cmd.action == b'd' && result.is_ok();
        let reply = command::reply(cmd, id, &result).filter(|_| !silent);
        KittyOutcome { reply, cursor }
    }

    fn run(&mut self, cmd: &Command, id: u32, data: Payload, store: &mut ImageStore) -> Done {
        if cmd.id != 0 && cmd.number != 0 {
            return Err(ImageError::Malformed("i and I together"));
        }
        // Refused before decoding so a failed placement stores nothing.
        if cmd.unsupported_placement && matches!(cmd.action, b'T' | b'p') {
            return Err(ImageError::Unsupported("virtual and relative placements"));
        }
        let key = ImageId::kitty(id);
        match cmd.action {
            b't' | b'T' | b'q' => {
                let image = decode(cmd, data?)?;
                if cmd.action == b'q' {
                    return Ok(None);
                }
                let key = if id == 0 { store.alloc_id() } else { key };
                store.insert(key, image)?;
                if cmd.number != 0 {
                    self.remember(cmd.number, id, store);
                }
                if cmd.action == b'T' {
                    return self.place(cmd, key, store);
                }
                Ok(None)
            }
            b'p' => self.place(cmd, key, store),
            b'd' => {
                self.delete(cmd, store);
                Ok(None)
            }
            b'f' | b'a' | b'c' => Err(ImageError::Unsupported("animation")),
            _ => Err(ImageError::Malformed("unknown action")),
        }
    }

    /// An unused kitty id for an image known only by its number.
    fn fresh_id(&mut self, store: &ImageStore) -> u32 {
        // Terminates: the store holds far fewer than `u32::MAX` images.
        loop {
            // Skips 0 and 1: 0 means "no id".
            let id = u32::MAX - self.issued % (u32::MAX - 1);
            self.issued = self.issued.wrapping_add(1);
            if store.peek(ImageId::kitty(id)).is_none() {
                return id;
            }
        }
    }

    fn remember(&mut self, number: u32, id: u32, store: &ImageStore) {
        if self.numbers.len() >= MAX_IMAGES {
            self.numbers
                .retain(|_, id| store.peek(ImageId::kitty(*id)).is_some());
        }
        self.numbers.insert(number, id);
    }

    /// Places a stored image at the cursor; returns the cursor movement.
    fn place(&self, cmd: &Command, image: ImageId, store: &mut ImageStore) -> Done {
        let ctx = &self.ctx;
        let stored = store.peek(image).ok_or(ImageError::NotFound(image))?;
        let crop = cmd.crop.clamp(stored.width(), stored.height());
        if crop.width == 0 || crop.height == 0 {
            return Err(ImageError::Malformed("source rectangle outside the image"));
        }
        let (cell_w, cell_h) = (ctx.cell_width.max(1), ctx.cell_height.max(1));
        // The offset must stay inside the first cell.
        let (offset_x, offset_y) = (cmd.offset_x.min(cell_w - 1), cmd.offset_y.min(cell_h - 1));
        // The other side follows the crop's aspect ratio when only one is given.
        let scaled = |cells: u32, cell: u32, num: u32, den: u32| {
            let px = u64::from(cells) * u64::from(cell) * u64::from(num) / u64::from(den);
            u32::try_from(px).unwrap_or(u32::MAX)
        };
        let (cols, rows) = match (cmd.cols, cmd.rows) {
            (0, 0) => (
                cells_for(crop.width.saturating_add(offset_x), cell_w),
                cells_for(crop.height.saturating_add(offset_y), cell_h),
            ),
            (c, 0) => (
                c,
                cells_for(scaled(c, cell_w, crop.height, crop.width), cell_h),
            ),
            (0, r) => (
                cells_for(scaled(r, cell_h, crop.width, crop.height), cell_w),
                r,
            ),
            (c, r) => (c, r),
        };
        let (cols, rows) = (cols.min(MAX_PLACEMENT_CELLS), rows.min(MAX_PLACEMENT_CELLS));
        // The spec ignores placement ids of anonymous images.
        let named = cmd.id != 0 || cmd.number != 0;
        let id = if named { cmd.placement } else { 0 };
        let (row, col, z) = (ctx.row, ctx.col, cmd.z);
        store.place(Placement {
            image,
            id,
            row,
            col,
            cols,
            rows,
            offset_x,
            offset_y,
            crop,
            z,
        })?;
        Ok((!cmd.no_move).then_some((cols, rows)))
    }

    /// Deletes placements per `d`; the upper-case forms also free the images
    /// that are left without one.
    fn delete(&self, cmd: &Command, store: &mut ImageStore) {
        let ctx = &self.ctx;
        let key = cmd.delete.to_ascii_lowercase();
        let (x, y) = (cmd.crop.x, cmd.crop.y);
        let target = match key {
            b'i' => cmd.id,
            b'n' => self.numbers.get(&cmd.number).copied().unwrap_or(0),
            _ => 0,
        };
        let by_id = |image: ImageId| match key {
            b'i' | b'n' => target != 0 && image == ImageId::kitty(target),
            b'r' => image.as_kitty().is_some_and(|k| (x..=y).contains(&k)),
            _ => false,
        };
        // `x` and `y` name a cell 1-based from the top-left of the screen.
        let row = ctx.top.saturating_add(u64::from(y.saturating_sub(1)));
        let col = x.saturating_sub(1);
        let bottom = ctx.top.saturating_add(u64::from(ctx.screen_rows));
        let doomed = |p: &Placement| match key {
            b'a' => p.row < bottom && p.row.saturating_add(u64::from(p.rows)) > ctx.top,
            b'i' | b'n' => by_id(p.image) && (cmd.placement == 0 || p.id == cmd.placement),
            b'r' => by_id(p.image),
            b'c' => p.covers(ctx.row, ctx.col),
            b'p' => p.covers(row, col),
            b'q' => p.covers(row, col) && p.z == cmd.z,
            b'x' => p.covers_col(col),
            b'y' => p.covers_row(row),
            b'z' => p.z == cmd.z,
            // `f` deletes animation frames, which are not supported.
            _ => false,
        };
        let mut touched = FxHashSet::default();
        store.retain_placements(|p| {
            let gone = doomed(p);
            if gone {
                touched.insert(p.image);
            }
            !gone
        });
        if cmd.delete.is_ascii_uppercase() {
            store.prune_unplaced(|image| touched.contains(&image) || by_id(image));
        }
    }
}

/// Decodes an upload's payload per `f` and `o`.
fn decode(cmd: &Command, data: Vec<u8>) -> Result<Image, ImageError> {
    let (width, height) = (cmd.width, cmd.height);
    let raw = |channels: usize| {
        rgba_len(width.into(), height.into()).map(|n| n / BYTES_PER_PIXEL * channels)
    };
    // Raw pixels have a known size, so inflating can stop right past it.
    let limit = match cmd.format {
        FORMAT_RGB => raw(RGB_BYTES)?,
        FORMAT_RGBA => raw(BYTES_PER_PIXEL)?,
        FORMAT_PNG => MAX_PAYLOAD_BYTES,
        _ => return Err(ImageError::Malformed("unknown format")),
    };
    let data = if cmd.compressed {
        inflate(&data, limit)?
    } else {
        data
    };
    match cmd.format {
        FORMAT_RGB => Image::from_rgb(width, height, &data),
        FORMAT_RGBA => Image::from_rgba(width, height, data),
        _ => raster::decode_png(&data),
    }
}

/// Inflates zlib data, refusing output past `limit` bytes: a few kilobytes
/// of zeros would otherwise expand to gigabytes.
fn inflate(data: &[u8], limit: usize) -> Result<Vec<u8>, ImageError> {
    let mut out = Vec::new();
    let cap = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    let read = ZlibDecoder::new(data).take(cap).read_to_end(&mut out);
    read.map_err(|e| ImageError::Decode(e.to_string()))?;
    if out.len() > limit {
        return Err(ImageError::PayloadTooLarge(limit));
    }
    Ok(out)
}
