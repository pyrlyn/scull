use std::io::{ErrorKind, Read};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{ConfigError, Rgb, Scheme, Theme};

/// The largest file read; a config is a few hundred bytes, so anything near
/// this is not one.
pub const MAX_FILE_BYTES: u64 = 64 * 1024;
/// The longest font family name, in bytes.
pub const MAX_FAMILY_BYTES: usize = 128;
/// The smallest font size, in points.
pub const MIN_FONT_SIZE: f32 = 4.0;
/// The largest font size, in points.
pub const MAX_FONT_SIZE: f32 = 200.0;
/// The most history rows the file may ask for; the core clamps further to
/// what its ring can hold.
pub const MAX_SCROLLBACK: u32 = 1_000_000;
/// The palette entries a file may override: ANSI colours 0 to 15.
pub const MAX_ANSI_OVERRIDES: usize = 16;

const DEFAULT_FONT_SIZE: f32 = 13.0;
const DEFAULT_SCROLLBACK: u32 = 10_000;

/// A font family name: empty for the platform's monospace font.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[cfg_attr(test, schemars(extend("maxLength" = 128), transform = string_schema))]
struct Family(String);

impl TryFrom<String> for Family {
    type Error = String;

    fn try_from(name: String) -> Result<Self, String> {
        if name.len() > MAX_FAMILY_BYTES || name.chars().any(char::is_control) {
            return Err(format!(
                "a font family is at most {MAX_FAMILY_BYTES} bytes with no control characters"
            ));
        }
        Ok(Self(name))
    }
}

impl From<Family> for String {
    fn from(family: Family) -> Self {
        family.0
    }
}

/// A font size in points.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "f32", into = "f32")]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[cfg_attr(test, schemars(extend("minimum" = 4.0, "maximum" = 200.0), transform = number_schema))]
struct FontSize(f32);

impl TryFrom<f32> for FontSize {
    type Error = String;

    fn try_from(size: f32) -> Result<Self, String> {
        if (MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&size) {
            Ok(Self(size))
        } else {
            Err(format!(
                "a font size is {MIN_FONT_SIZE} to {MAX_FONT_SIZE} points, not {size}"
            ))
        }
    }
}

impl From<FontSize> for f32 {
    fn from(size: FontSize) -> Self {
        size.0
    }
}

/// History rows to keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[cfg_attr(test, schemars(extend("minimum" = 0, "maximum" = 1_000_000), transform = integer_schema))]
struct Scrollback(u32);

impl TryFrom<u32> for Scrollback {
    type Error = String;

    fn try_from(rows: u32) -> Result<Self, String> {
        if rows <= MAX_SCROLLBACK {
            Ok(Self(rows))
        } else {
            Err(format!(
                "scrollback is at most {MAX_SCROLLBACK} rows, not {rows}"
            ))
        }
    }
}

impl From<Scrollback> for u32 {
    fn from(rows: Scrollback) -> Self {
        rows.0
    }
}

/// The first colours of the palette, replacing the scheme's.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Vec<Rgb>", into = "Vec<Rgb>")]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[cfg_attr(test, schemars(extend("maxItems" = 16), transform = array_schema))]
struct AnsiOverrides(Vec<Rgb>);

impl TryFrom<Vec<Rgb>> for AnsiOverrides {
    type Error = String;

    fn try_from(colors: Vec<Rgb>) -> Result<Self, String> {
        if colors.len() > MAX_ANSI_OVERRIDES {
            return Err(format!(
                "ansi has at most {MAX_ANSI_OVERRIDES} colours, not {}",
                colors.len()
            ));
        }
        Ok(Self(colors))
    }
}

impl From<AnsiOverrides> for Vec<Rgb> {
    fn from(colors: AnsiOverrides) -> Self {
        colors.0
    }
}

// serde's `try_from` hides the inner type from schemars, so the schema is
// pinned to the JSON type each newtype really is.
#[cfg(test)]
fn pinned(schema: &mut schemars::Schema, kind: &str) {
    schema.insert("type".into(), kind.into());
}
#[cfg(test)]
pub(crate) fn string_schema(schema: &mut schemars::Schema) {
    pinned(schema, "string");
}
#[cfg(test)]
fn number_schema(schema: &mut schemars::Schema) {
    pinned(schema, "number");
}
#[cfg(test)]
fn integer_schema(schema: &mut schemars::Schema) {
    pinned(schema, "integer");
}
#[cfg(test)]
fn array_schema(schema: &mut schemars::Schema) {
    pinned(schema, "array");
}

/// `[font]`.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
struct FontSection {
    /// Family name; empty for the platform's monospace font.
    family: Family,
    /// Size in points.
    size: FontSize,
}

impl Default for FontSection {
    fn default() -> Self {
        Self {
            family: Family(String::new()),
            size: FontSize(DEFAULT_FONT_SIZE),
        }
    }
}

/// `[colors]`.
#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
struct ColorsSection {
    /// The built-in scheme everything below overrides.
    scheme: Scheme,
    /// Default text colour.
    foreground: Option<Rgb>,
    /// Default background colour.
    background: Option<Rgb>,
    /// Cursor colour.
    cursor: Option<Rgb>,
    /// Palette entries 0 to 15, from the first; fewer than 16 keeps the rest.
    ansi: AnsiOverrides,
}

/// The file as written; anything it leaves out is the default.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Config {
    font: FontSection,
    colors: ColorsSection,
    /// History rows kept per terminal; applies to terminals opened afterwards.
    scrollback: Scrollback,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font: FontSection::default(),
            colors: ColorsSection::default(),
            scrollback: Scrollback(DEFAULT_SCROLLBACK),
        }
    }
}

/// The font a host asks the platform for.
#[derive(Clone, Debug, PartialEq)]
pub struct Font {
    /// Family name; empty for the platform's monospace font.
    pub family: String,
    /// Size in points.
    pub size: f32,
}

/// Everything the file decides, with defaults filled in.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// The terminal font.
    pub font: Font,
    /// The colours.
    pub colors: Theme,
    /// History rows for terminals opened from now on.
    pub scrollback: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Config::default().resolve()
    }
}

impl Config {
    fn resolve(self) -> Settings {
        let c = self.colors;
        let mut colors = c.scheme.theme();
        colors.foreground = c.foreground.unwrap_or(colors.foreground);
        colors.background = c.background.unwrap_or(colors.background);
        colors.cursor = c.cursor.unwrap_or(colors.cursor);
        for (slot, color) in colors.ansi.iter_mut().zip(c.ansi.0) {
            *slot = color;
        }
        Settings {
            font: Font {
                family: self.font.family.0,
                size: self.font.size.0,
            },
            colors,
            scrollback: self.scrollback.0,
        }
    }
}

impl Settings {
    /// Parses and validates `text`; `origin` only names the file in errors.
    pub fn parse(text: &str, origin: &Path) -> Result<Self, ConfigError> {
        let config: Config = toml::from_str(text).map_err(|e| ConfigError::Invalid {
            path: origin.to_owned(),
            message: e.to_string(),
        })?;
        Ok(config.resolve())
    }

    /// Reads `path`. A file that does not exist is not an error: it is the
    /// defaults, which is what a fresh install has.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        match read_capped(path) {
            Ok(text) => Self::parse(&text, path),
            Err(ConfigError::Io { source, .. }) if source.kind() == ErrorKind::NotFound => {
                Ok(Self::default())
            }
            Err(e) => Err(e),
        }
    }
}

/// The file's text, never more than [`MAX_FILE_BYTES`] of it in memory.
fn read_capped(path: &Path) -> Result<String, ConfigError> {
    let io = |source| ConfigError::Io {
        path: path.to_owned(),
        source,
    };
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(io)?
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(ConfigError::TooLarge {
            path: path.to_owned(),
        });
    }
    String::from_utf8(bytes).map_err(|_| ConfigError::Invalid {
        path: path.to_owned(),
        message: "not valid UTF-8".to_owned(),
    })
}
