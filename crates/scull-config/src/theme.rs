use std::fmt;

use serde::{Deserialize, Serialize};

/// A colour as `0xRRGGBB`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[cfg_attr(test, schemars(extend("pattern" = "^#[0-9a-fA-F]{6}$"), transform = crate::model::string_schema))]
pub struct Rgb(u32);

impl Rgb {
    /// The colour `0xRRGGBB`; the bits above 24 are dropped.
    pub const fn new(rgb: u32) -> Self {
        Self(rgb & 0xFF_FFFF)
    }

    /// `0xRRGGBB`.
    pub const fn value(self) -> u32 {
        self.0
    }
}

impl TryFrom<String> for Rgb {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        let digits = text.strip_prefix('#').unwrap_or_default();
        // `from_str_radix` would also take a sign, so the digits are checked first.
        if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("`{text}` is not a colour like #1a2b3c"));
        }
        u32::from_str_radix(digits, 16)
            .map(Self)
            .map_err(|e| e.to_string())
    }
}

impl From<Rgb> for String {
    fn from(rgb: Rgb) -> Self {
        format!("#{:06x}", rgb.0)
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:06x}", self.0)
    }
}

/// The built-in colour schemes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case")]
pub enum Scheme {
    /// Light text on near-black; the xterm palette.
    #[default]
    ScullDark,
    /// Dark text on near-white.
    ScullLight,
    /// Ethan Schoonover's Solarized, dark variant.
    SolarizedDark,
}

/// The colours a host paints with: default text, background, cursor and the
/// 16 palette entries that indexed colours 0 to 15 select.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    /// Text with the default foreground.
    pub foreground: Rgb,
    /// Cells with the default background.
    pub background: Rgb,
    /// The cursor block.
    pub cursor: Rgb,
    /// Black, red, green, yellow, blue, magenta, cyan, white, then the bright eight.
    pub ansi: [Rgb; 16],
}

impl Scheme {
    /// The name the file spells it with.
    pub fn name(self) -> &'static str {
        match self {
            Self::ScullDark => "scull-dark",
            Self::ScullLight => "scull-light",
            Self::SolarizedDark => "solarized-dark",
        }
    }

    /// The colours of the scheme.
    pub fn theme(self) -> Theme {
        let (foreground, background, cursor, ansi) = match self {
            Self::ScullDark => (0xE5E5E5, 0x141414, 0xE5E5E5, XTERM),
            Self::ScullLight => (0x2B2B2B, 0xFAFAFA, 0x2B2B2B, LIGHT),
            Self::SolarizedDark => (0x839496, 0x002B36, 0x93A1A1, SOLARIZED),
        };
        Theme {
            foreground: Rgb::new(foreground),
            background: Rgb::new(background),
            cursor: Rgb::new(cursor),
            ansi: ansi.map(Rgb::new),
        }
    }
}

// xterm's own values, which programs tuned their indexed colours against.
const XTERM: [u32; 16] = [
    0x000000, 0xCD0000, 0x00CD00, 0xCDCD00, 0x0000EE, 0xCD00CD, 0x00CDCD, 0xE5E5E5, 0x7F7F7F,
    0xFF0000, 0x00FF00, 0xFFFF00, 0x5C5CFF, 0xFF00FF, 0x00FFFF, 0xFFFFFF,
];

// Darker than xterm's: yellow and white would vanish on a near-white ground.
const LIGHT: [u32; 16] = [
    0x1F1F1F, 0xB3261E, 0x1E7B34, 0x8A6100, 0x1D4ED8, 0x9333EA, 0x0E7490, 0xBDBDBD, 0x5C5C5C,
    0xD92D20, 0x2E9E4A, 0xA67C00, 0x3B6EF0, 0xA855F7, 0x1593B0, 0xF5F5F5,
];

const SOLARIZED: [u32; 16] = [
    0x073642, 0xDC322F, 0x859900, 0xB58900, 0x268BD2, 0xD33682, 0x2AA198, 0xEEE8D5, 0x002B36,
    0xCB4B16, 0x586E75, 0x657B83, 0x839496, 0x6C71C4, 0x93A1A1, 0xFDF6E3,
];
