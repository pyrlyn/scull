//! Pure encoders from key, mouse, paste and focus events to the bytes a
//! program expects. No state of its own beyond what the caller passes in,
//! so every encoding is a table-driven test.
//!
//! Encoders append to a caller-owned `Vec<u8>`, so a reused buffer makes
//! encoding allocation-free. The one piece of state a program can grow, the
//! kitty keyboard flag stack, is fixed-size ([`KITTY_FLAG_STACK_DEPTH`]).

mod bytes;
mod key;
mod keyboard;
mod kitty;
mod legacy;
mod win32;

pub use key::{Key, KeyAction, KeyEvent, KeypadKey, MediaKey, ModifierKey, Modifiers, SystemKey};
pub use keyboard::{KeyModes, ModifyOtherKeys, encode_key};
pub use kitty::{
    KITTY_FLAG_STACK_DEPTH, KittyFlagStack, KittyFlags, KittyKeyboard, Screen, SetMode,
};
