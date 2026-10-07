//! The configuration file, read once and resolved to plain values: a font
//! family and size, colours as RGB, a history length. Nothing here names a
//! toolkit type, so every platform host reads the same [`Settings`].
//!
//! This crate is the only code that touches the TOML. The file is input from
//! outside the program, so it is size-capped, unknown keys are errors, and
//! every value has a range that lives in its type; the committed
//! `docs/config.schema.json` is generated from those same types.

mod edit;
mod error;
mod keybind;
mod live;
mod model;
mod theme;

pub use error::ConfigError;
pub use keybind::{Action, Binding, Chord, MAX_KEYBINDS};
pub use live::{LiveConfig, Snapshot};
pub use model::{
    Font, MAX_ANSI_OVERRIDES, MAX_FAMILY_BYTES, MAX_FILE_BYTES, MAX_FONT_SIZE, MAX_SCROLLBACK,
    MIN_FONT_SIZE, Settings,
};
pub use theme::{Rgb, Scheme, Theme};

#[cfg(test)]
mod live_tests;
#[cfg(test)]
mod tests;
