//! Sixel graphics (DEC VT330/VT340, `DCS P1;P2;P3 q ... ST`): a streaming
//! decoder that paints into an RGBA canvas as payload slices arrive, so a
//! large image never has to be buffered as text. Written from the VT330/340
//! programmer reference rather than reused: `icy_sixel`'s stream borrows its
//! decoder (it cannot live across parser callbacks), its 64 Mpx limit is a
//! private constant, and it always pulls an encoder with its quantiser.
//!
//! Pixels are 1:1: like xterm and foot, the aspect ratio of `P1` and of the
//! raster attributes is ignored.

use crate::{BYTES_PER_PIXEL, Image, ImageError};

/// Longest side of a sixel image; 4096 x 4096 RGBA is 64 MiB.
pub const MAX_SIXEL_SIDE: u32 = 4096;
/// Colour registers; xterm and foot also offer 1024.
pub const MAX_SIXEL_COLORS: usize = 1024;
/// Pixel writes allowed per image, as overdraws of the largest canvas: a
/// hostile stream of `!4096~$` would otherwise burn CPU without growing.
const MAX_PAINTS: u64 = (MAX_SIXEL_SIDE as u64) * (MAX_SIXEL_SIDE as u64) * OVERDRAW;
const OVERDRAW: u64 = 8;
/// Rows painted by one sixel character.
const BAND: u32 = 6;
/// Sixel characters are `?` (no pixels) to `~` (all six).
const SIXEL_FIRST: u8 = b'?';
const SIXEL_LAST: u8 = b'~';
/// The most numeric parameters any command takes (`#Pc;Pu;Px;Py;Pz`).
const MAX_PARAMS: usize = 5;
/// Colour coordinate system selectors of `#Pc;Pu;...`.
const COORD_HLS: u32 = 1;
const COORD_RGB: u32 = 2;
const PERCENT: u32 = 100;
const DEGREES: u32 = 360;
const HUE_SECTOR: f32 = 60.0;
/// DEC puts blue at hue 0 and red at 120; HSL puts red at 0.
const DEC_HUE_SHIFT: u32 = 240;

/// The VT340 default colour map, in percent RGB (VT330/340 programmer reference).
const VT340: [[u32; 3]; 16] = [
    [0, 0, 0],
    [20, 20, 80],
    [80, 13, 13],
    [20, 80, 20],
    [80, 20, 80],
    [20, 80, 80],
    [80, 80, 20],
    [53, 53, 53],
    [26, 26, 26],
    [33, 33, 60],
    [60, 26, 26],
    [33, 60, 33],
    [60, 33, 60],
    [33, 60, 60],
    [60, 60, 33],
    [80, 80, 80],
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Ground,
    Color,
    Repeat,
    Raster,
}

/// Decodes one sixel image from its DCS payload (the bytes after `q`).
#[derive(Debug, Clone)]
pub struct SixelDecoder {
    state: State,
    params: [u32; MAX_PARAMS],
    nparams: usize,
    palette: Vec<[u8; BYTES_PER_PIXEL]>,
    color: usize,
    x: u32,
    band: u32,
    /// Painted extent (or the raster attributes' size, if larger).
    width: u32,
    height: u32,
    /// Allocated canvas size in pixels; rows are `cap_w` wide.
    cap_w: u32,
    cap_h: u32,
    canvas: Vec<u8>,
    background: [u8; BYTES_PER_PIXEL],
    painted: bool,
    paints: u64,
}

fn percent(v: u32) -> u8 {
    let v = v.min(PERCENT) * u32::from(u8::MAX);
    u8::try_from((v + PERCENT / 2) / PERCENT).unwrap_or(u8::MAX)
}

fn rgb(r: u32, g: u32, b: u32) -> [u8; BYTES_PER_PIXEL] {
    [percent(r), percent(g), percent(b), u8::MAX]
}

/// DEC HLS (hue in degrees, lightness and saturation in percent) to RGBA.
fn hls(h: u32, l: u32, s: u32) -> [u8; BYTES_PER_PIXEL] {
    let unit = |v: u32| v.min(PERCENT) as f32 / PERCENT as f32;
    let (l, s) = (unit(l), unit(s));
    let h = ((h % DEGREES + DEC_HUE_SHIFT) % DEGREES) as f32 / HUE_SECTOR;
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let byte = |v: f32| ((v + m) * f32::from(u8::MAX)).round() as u8;
    [byte(r), byte(g), byte(b), u8::MAX]
}

impl SixelDecoder {
    /// A decoder for one image. `background` fills pixels no sixel paints:
    /// `None` (DCS `P2` = 1) leaves them transparent, otherwise the terminal
    /// passes its background colour.
    pub fn new(background: Option<[u8; BYTES_PER_PIXEL]>) -> Self {
        let mut palette = vec![[0, 0, 0, u8::MAX]; MAX_SIXEL_COLORS];
        for (slot, [r, g, b]) in palette.iter_mut().zip(VT340) {
            *slot = rgb(r, g, b);
        }
        Self {
            state: State::Ground,
            params: [0; MAX_PARAMS],
            nparams: 0,
            palette,
            color: 0,
            x: 0,
            band: 0,
            width: 0,
            height: 0,
            cap_w: 0,
            cap_h: 0,
            canvas: Vec::new(),
            background: background.unwrap_or_default(),
            painted: false,
            paints: 0,
        }
    }

    /// Feeds a slice of the payload; slices may split anywhere.
    pub fn put(&mut self, data: &[u8]) {
        for &byte in data {
            if self.state != State::Ground {
                match byte {
                    b'0'..=b'9' => {
                        self.nparams = self.nparams.max(1);
                        // Parameters past the fifth are ignored.
                        if let Some(p) = self.params.get_mut(self.nparams - 1) {
                            *p = p.saturating_mul(10).saturating_add(u32::from(byte - b'0'));
                        }
                        continue;
                    }
                    b';' => {
                        self.nparams = (self.nparams.max(1) + 1).min(MAX_PARAMS + 1);
                        continue;
                    }
                    _ if self.end_command(byte) => continue,
                    _ => {}
                }
            }
            self.ground(byte);
        }
    }

    /// Ends the image; errors when nothing was painted or sized.
    pub fn finish(mut self) -> Result<Image, ImageError> {
        self.end_command(0);
        if self.width == 0 || self.height == 0 {
            return Err(ImageError::Empty);
        }
        self.grow(self.width, self.height);
        let row = self.width as usize * BYTES_PER_PIXEL;
        let stride = self.cap_w as usize * BYTES_PER_PIXEL;
        let mut pixels = Vec::with_capacity(row * self.height as usize);
        for y in 0..self.height as usize {
            pixels.extend_from_slice(&self.canvas[y * stride..y * stride + row]);
        }
        Image::from_rgba(self.width, self.height, pixels)
    }

    fn ground(&mut self, byte: u8) {
        let state = match byte {
            b'#' => State::Color,
            b'!' => State::Repeat,
            b'"' => State::Raster,
            b'$' => {
                self.x = 0;
                return;
            }
            b'-' => {
                self.x = 0;
                self.band = self.band.saturating_add(BAND);
                return;
            }
            SIXEL_FIRST..=SIXEL_LAST => {
                self.paint(byte - SIXEL_FIRST, 1);
                return;
            }
            // Everything else, including stray controls, is ignored.
            _ => return,
        };
        self.state = state;
        self.params = [0; MAX_PARAMS];
        self.nparams = 0;
    }

    /// Applies the pending command. True when it consumed `byte`: a repeat
    /// paints the sixel that ends it.
    fn end_command(&mut self, byte: u8) -> bool {
        let (state, p) = (self.state, self.params);
        self.state = State::Ground;
        match state {
            State::Ground => {}
            State::Color => {
                self.color = (p[0] as usize).min(MAX_SIXEL_COLORS - 1);
                let defined = match p[1] {
                    _ if self.nparams < MAX_PARAMS => None,
                    COORD_HLS => Some(hls(p[2], p[3], p[4])),
                    COORD_RGB => Some(rgb(p[2], p[3], p[4])),
                    _ => None,
                };
                if let Some(color) = defined {
                    self.palette[self.color] = color;
                }
            }
            State::Repeat if (SIXEL_FIRST..=SIXEL_LAST).contains(&byte) => {
                self.paint(byte - SIXEL_FIRST, p[0].max(1));
                return true;
            }
            State::Repeat => {}
            // Raster attributes count only before the first sixel.
            State::Raster if !self.painted => {
                self.width = p[2].min(MAX_SIXEL_SIDE);
                self.height = p[3].min(MAX_SIXEL_SIDE);
            }
            State::Raster => {}
        }
        false
    }

    /// Paints `bits` (bit 0 at the top of the band) `count` times from x.
    fn paint(&mut self, bits: u8, count: u32) {
        let x0 = self.x;
        self.x = self.x.saturating_add(count);
        let x1 = self.x.min(MAX_SIXEL_SIDE);
        if x0 >= x1 {
            return;
        }
        self.painted = true;
        self.width = self.width.max(x1);
        if bits == 0 {
            return;
        }
        let last = u8::BITS - 1 - bits.leading_zeros();
        let rows = (0..=last).filter(|i| bits & (1 << i) != 0);
        let y_end = self.band.saturating_add(last + 1).min(MAX_SIXEL_SIDE);
        let cost = u64::from(x1 - x0) * u64::from(bits.count_ones());
        if y_end <= self.band || self.paints.saturating_add(cost) > MAX_PAINTS {
            return;
        }
        self.paints += cost;
        self.height = self.height.max(y_end);
        self.grow(x1, y_end);
        let color = self.palette[self.color];
        let stride = self.cap_w as usize * BYTES_PER_PIXEL;
        let (start, end) = (x0 as usize * BYTES_PER_PIXEL, x1 as usize * BYTES_PER_PIXEL);
        for y in rows.map(|i| self.band + i).filter(|&y| y < y_end) {
            let line = y as usize * stride;
            for px in self.canvas[line + start..line + end].as_chunks_mut().0 {
                *px = color;
            }
        }
    }

    /// Makes the canvas at least `w` x `h`, doubling to amortise regrowth.
    fn grow(&mut self, w: u32, h: u32) {
        let (old_w, old_h) = (self.cap_w as usize, self.cap_h as usize);
        if w > self.cap_w {
            self.cap_w = w.max(self.cap_w.saturating_mul(2)).min(MAX_SIXEL_SIDE);
        }
        if h > self.cap_h {
            self.cap_h = h.max(self.cap_h.saturating_mul(2)).min(MAX_SIXEL_SIDE);
        }
        let (new_w, new_h) = (self.cap_w as usize, self.cap_h as usize);
        if (new_w, new_h) == (old_w, old_h) {
            return;
        }
        let mut canvas = vec![0; new_w * new_h * BYTES_PER_PIXEL];
        for px in canvas.as_chunks_mut().0 {
            *px = self.background;
        }
        let (old_row, new_row) = (old_w * BYTES_PER_PIXEL, new_w * BYTES_PER_PIXEL);
        for y in 0..old_h {
            canvas[y * new_row..y * new_row + old_row]
                .copy_from_slice(&self.canvas[y * old_row..(y + 1) * old_row]);
        }
        self.canvas = canvas;
    }
}
