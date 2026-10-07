//! The C ABI that the Swift and C# hosts link: opaque handles, the frame
//! snapshot, the wakeup callback and the polled event queue. Every export is
//! guarded so no panic crosses the boundary.
//!
//! The design is `docs/research/ffi-native-ui.md` Part 4: hand-written
//! `extern "C"` functions over `#[repr(C)]` structs, so cbindgen (C header)
//! and csbindgen (C#) only describe them and the frame is read in place.
//!
//! Contract for every caller, stated once here and in the header:
//!
//! - A `tt_term *` or `tt_frame *` came from its `_new` function and is not
//!   yet freed. `NULL` is answered with `TT_INVALID`, never dereferenced.
//! - Every struct starts with `struct_size`, which the caller sets to
//!   `sizeof` of its own copy. The core reads and writes only that many
//!   bytes; fields a smaller (older) struct lacks read as zero. Fields are
//!   only ever appended.
//! - A panic inside the core never unwinds into the host: the call answers
//!   `TT_PANIC` and the terminal is poisoned, answering `TT_POISONED` to
//!   every later call until it is freed. Other terminals are unaffected.
//! - Pointers in a `tt_frame_view` stay valid until the next
//!   `tt_frame_update` or `tt_frame_free` on that frame. An image in it
//!   lives on while the host holds a `tt_image_retain` on it.

// The exported names are the C names; the header is the API.
#![allow(non_camel_case_types)]
// Exported constants are plain literals, never shifts or other constants:
// csbindgen drops a constant it cannot read as a literal from the C#
// bindings. Each one is checked against its meaning where it is defined.

mod event;
mod frame;
mod guard;
mod input;
mod spawn;
mod term;

pub use event::{
    TT_EVENT_BELL, TT_EVENT_CHILD_EXITED, TT_EXIT_CODE_UNKNOWN, tt_event, tt_term_poll_event,
};

pub use frame::{
    TT_ATTR_BLINK, TT_ATTR_BOLD, TT_ATTR_DIM, TT_ATTR_HIDDEN, TT_ATTR_INVERSE, TT_ATTR_ITALIC,
    TT_ATTR_OVERLINE, TT_ATTR_STRIKE, TT_CELL_CLUSTER, TT_COLOR_DEFAULT, TT_COLOR_INDEXED,
    TT_COLOR_KIND_MASK, TT_COLOR_RGB, TT_MAX_PREEDIT_BYTES, TT_UNDERLINE_CURLY,
    TT_UNDERLINE_DASHED, TT_UNDERLINE_DOTTED, TT_UNDERLINE_DOUBLE, TT_UNDERLINE_NONE,
    TT_UNDERLINE_SINGLE, tt_cell, tt_cursor, tt_frame, tt_frame_free, tt_frame_new,
    tt_frame_preedit, tt_frame_update, tt_frame_view, tt_image, tt_image_pixels, tt_image_release,
    tt_image_retain, tt_placement, tt_preedit, tt_row, tt_run, tt_scroll, tt_style,
};
pub use guard::tt_status;
pub use input::{
    TT_KEY_CAPS_LOCK, TT_KEY_END, TT_KEY_ESCAPE, TT_KEY_F1, TT_KEY_KP_0, TT_KEY_LEFT_SHIFT,
    TT_KEY_MEDIA_PLAY, TT_KEY_PRESS, TT_KEY_RELEASE, TT_KEY_REPEAT, TT_MOD_ALT, TT_MOD_CAPS_LOCK,
    TT_MOD_CTRL, TT_MOD_HYPER, TT_MOD_META, TT_MOD_NUM_LOCK, TT_MOD_SHIFT, TT_MOD_SUPER,
    TT_MOUSE_LEFT, TT_MOUSE_MIDDLE, TT_MOUSE_MOTION, TT_MOUSE_NONE, TT_MOUSE_PRESS,
    TT_MOUSE_RELEASE, TT_MOUSE_RIGHT, TT_MOUSE_WHEEL_DOWN, TT_MOUSE_WHEEL_LEFT,
    TT_MOUSE_WHEEL_RIGHT, TT_MOUSE_WHEEL_UP, tt_key_event, tt_mouse_event, tt_term_focus,
    tt_term_key, tt_term_mouse, tt_term_paste, tt_term_scroll_display, tt_term_text,
};
pub use spawn::{tt_str, tt_term_resize_begin, tt_term_spawn, tt_term_write, tt_wakeup_fn};
pub use term::{tt_term, tt_term_feed, tt_term_free, tt_term_new, tt_term_options, tt_term_resize};

/// Breaking changes bump the major version; a host refuses to run on
/// another major.
pub const TT_ABI_VERSION_MAJOR: u32 = 0;

/// Additions (new functions, fields appended to a struct) bump the minor.
/// While the major is 0 every minor may break, so the minor must match too.
pub const TT_ABI_VERSION_MINOR: u32 = 4;

/// The version a host was built against, `major << 16 | minor`; pass it
/// in `tt_term_options.abi_version`.
pub const TT_ABI_VERSION: u32 = 0x0000_0004;

const _: () = assert!(TT_ABI_VERSION == TT_ABI_VERSION_MAJOR << MINOR_BITS | TT_ABI_VERSION_MINOR);

/// Bits of the minor version in [`TT_ABI_VERSION`].
const MINOR_BITS: u32 = 16;

/// Whether a host built against ABI `version` can use this library.
fn abi_compatible(version: u32) -> bool {
    let major = version >> MINOR_BITS;
    major == TT_ABI_VERSION_MAJOR && (major != 0 || version == TT_ABI_VERSION)
}

/// The ABI version of this library, `major << 16 | minor`.
#[allow(unsafe_code)] // `no_mangle` is how a symbol gets its C name.
#[unsafe(no_mangle)]
pub extern "C" fn tt_abi_version() -> u32 {
    TT_ABI_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_exact_version_is_compatible_while_the_major_is_zero() {
        assert!(abi_compatible(TT_ABI_VERSION));
        assert!(abi_compatible(tt_abi_version()));
        assert!(!abi_compatible(TT_ABI_VERSION + 1));
        assert!(!abi_compatible(TT_ABI_VERSION - 1), "a host without input");
        assert!(!abi_compatible(1 << MINOR_BITS | TT_ABI_VERSION_MINOR));
        assert!(!abi_compatible(0));
    }
}
