//! The configuration handle: the file watched by `scull-config`, and the
//! settings it holds handed to the host by polling, like the frame. The core
//! never calls the host except through the wakeup, which fires when a poll
//! would now answer something new.
#![allow(unsafe_code)] // C exports take raw pointers from the host.

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use scull_config::{Action, LiveConfig, Settings, default_path};

use crate::abi_compatible;
use crate::guard::{SizedStruct, can_write, guard, read_sized, tt_status, write_sized};
use crate::input::code_of;
use crate::spawn::{Wake, text, tt_str, tt_wakeup_fn};

/// `tt_keybind.action`: paste the clipboard.
pub const TT_ACTION_PASTE: u8 = 1;
/// Make the font one point larger.
pub const TT_ACTION_FONT_LARGER: u8 = 2;
/// Make the font one point smaller.
pub const TT_ACTION_FONT_SMALLER: u8 = 3;
/// Back to the configured font size.
pub const TT_ACTION_FONT_RESET: u8 = 4;
/// Scroll the history one screen up.
pub const TT_ACTION_SCROLL_PAGE_UP: u8 = 5;
/// Scroll the history one screen down.
pub const TT_ACTION_SCROLL_PAGE_DOWN: u8 = 6;
/// Scroll to the oldest history row.
pub const TT_ACTION_SCROLL_TO_TOP: u8 = 7;
/// Scroll back to the screen.
pub const TT_ACTION_SCROLL_TO_BOTTOM: u8 = 8;

const _: () = assert!(
    TT_ACTION_PASTE == Action::Paste as u8
        && TT_ACTION_FONT_LARGER == Action::FontLarger as u8
        && TT_ACTION_FONT_SMALLER == Action::FontSmaller as u8
        && TT_ACTION_FONT_RESET == Action::FontReset as u8
        && TT_ACTION_SCROLL_PAGE_UP == Action::ScrollPageUp as u8
        && TT_ACTION_SCROLL_PAGE_DOWN == Action::ScrollPageDown as u8
        && TT_ACTION_SCROLL_TO_TOP == Action::ScrollToTop as u8
        && TT_ACTION_SCROLL_TO_BOTTOM == Action::ScrollToBottom as u8
);

/// How to open the configuration.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct tt_config_options {
    /// `sizeof(tt_config_options)` as the host knows it.
    pub struct_size: u32,
    /// `TT_ABI_VERSION` as the host was built with it.
    pub abi_version: u32,
    /// The config file; empty for the default place
    /// (`$XDG_CONFIG_HOME/scull/config.toml`, `~/.config/scull/config.toml`,
    /// `%APPDATA%\scull\config.toml`).
    pub path: tt_str,
    /// Called from a core thread when `tt_config_poll` would answer
    /// something new; `NULL` for none. Same contract as `tt_wakeup_fn`:
    /// it must not block or call `tt_*`, it is not called again until the
    /// host polls, and never after `tt_config_free` returns.
    pub wakeup: tt_wakeup_fn,
    /// Passed to `wakeup` as is.
    pub userdata: *mut c_void,
}

// SAFETY: repr(C), `struct_size` first, then integers, raw pointers and an
// optional function pointer.
unsafe impl SizedStruct for tt_config_options {}

/// A key binding: when `key` is pressed with exactly `mods` among Shift,
/// Alt, Control and Super held, do `action`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct tt_keybind {
    /// As `tt_key_event.key`.
    pub key: u32,
    /// `TT_MOD_*` bits.
    pub mods: u8,
    /// `TT_ACTION_*`.
    pub action: u8,
}

/// The settings in force. Every pointer in it stays valid until the next
/// `tt_config_poll` or `tt_config_free` on the same handle.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct tt_config_view {
    /// `sizeof(tt_config_view)` as the host knows it.
    pub struct_size: u32,
    /// 1 when anything changed since the previous poll (always on the first).
    pub updated: u8,
    /// 1 when file changes are watched; 0 means only `tt_config_set` and a
    /// new handle see the file.
    pub watching: u8,
    /// Grows with every change of the settings or of `error`.
    pub generation: u64,
    /// The font family; empty for the platform's monospace font.
    pub font_family: tt_str,
    /// The font size in points.
    pub font_size: f32,
    /// History rows for terminals opened from now on.
    pub scrollback: u32,
    /// Default text colour, `0xRRGGBB`.
    pub foreground: u32,
    /// Default background colour, `0xRRGGBB`.
    pub background: u32,
    /// Cursor colour, `0xRRGGBB`.
    pub cursor: u32,
    /// Palette entries 0 to 15, `0xRRGGBB`.
    pub ansi: [u32; 16],
    /// The key bindings in force.
    pub keybinds: *const tt_keybind,
    /// Number of `keybinds`.
    pub keybinds_len: usize,
    /// Why the file could not be used the last time it was read, with its
    /// name; empty when it was fine. The settings above are then the last
    /// good ones.
    pub error: tt_str,
    /// The config file's path.
    pub path: tt_str,
}

// SAFETY: repr(C), `struct_size` first, then integers, floats, arrays of
// integers, raw pointers and `tt_str`s.
unsafe impl SizedStruct for tt_config_view {}

/// What the last poll handed out, kept alive for the host to read.
struct Polled {
    generation: u64,
    settings: Arc<Settings>,
    error: String,
    keybinds: Vec<tt_keybind>,
}

/// The configuration. Opaque to the host.
pub struct tt_config {
    live: LiveConfig,
    polled: Mutex<Polled>,
    wake: Option<Arc<Wake>>,
    path: String,
}

/// Opens the configuration and starts watching it. A missing file is the
/// defaults and a broken one is an `error` in the view, so this fails only
/// for bad arguments or a host with no home directory to put the file in.
/// On `TT_OK` `*out` holds the handle; free it with `tt_config_free`.
///
/// # Safety
///
/// `options` is `NULL` or points to `struct_size` readable bytes, its
/// strings valid for their lengths during this call; `out` is `NULL` or
/// writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_config_new(
    options: *const tt_config_options,
    out: *mut *mut tt_config,
) -> tt_status {
    guard(|| {
        if out.is_null() {
            return tt_status::TT_INVALID;
        }
        // SAFETY: the caller's contract.
        let Some(options) = (unsafe { read_sized(options) }) else {
            return tt_status::TT_INVALID;
        };
        if !abi_compatible(options.abi_version) {
            return tt_status::TT_ABI_MISMATCH;
        }
        // SAFETY: the caller's contract.
        let Some(given) = (unsafe { text(options.path) }) else {
            return tt_status::TT_INVALID;
        };
        let Some(path) = (if given.is_empty() {
            default_path()
        } else {
            Some(PathBuf::from(given))
        }) else {
            return tt_status::TT_INVALID;
        };
        let wake = options
            .wakeup
            .map(|func| Arc::new(Wake::new(func, options.userdata)));
        let fire = wake.clone();
        let live = LiveConfig::open(path.clone(), move || {
            if let Some(wake) = &fire {
                wake.fire();
            }
        });
        let config = tt_config {
            live,
            polled: Mutex::new(Polled {
                generation: 0,
                settings: Arc::new(Settings::default()),
                error: String::new(),
                keybinds: Vec::new(),
            }),
            wake,
            path: path.to_string_lossy().into_owned(),
        };
        // SAFETY: `out` is non-null and writable by the caller's contract.
        unsafe { out.write(Box::into_raw(Box::new(config))) };
        tt_status::TT_OK
    })
}

fn str_of(s: &str) -> tt_str {
    tt_str {
        ptr: s.as_ptr(),
        len: s.len(),
    }
}

/// Fills `*view` with the settings in force and sets `updated` when they
/// are not what the previous poll returned. Polling lets the next wakeup
/// fire.
///
/// # Safety
///
/// `config` is `NULL` or live; `view` is `NULL` or points to `struct_size`
/// writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_config_poll(
    config: *const tt_config,
    view: *mut tt_config_view,
) -> tt_status {
    guard(|| {
        // SAFETY: the caller's contract.
        let Some(config) = (unsafe { config.as_ref() }) else {
            return tt_status::TT_INVALID;
        };
        // SAFETY: the caller's contract.
        if !unsafe { can_write(view) } {
            return tt_status::TT_INVALID;
        }
        if let Some(wake) = &config.wake {
            wake.rearm();
        }
        let snapshot = config.live.snapshot();
        let mut polled = config.polled.lock().unwrap_or_else(PoisonError::into_inner);
        let updated = polled.generation != snapshot.generation;
        if updated {
            polled.generation = snapshot.generation;
            polled.error = snapshot.error.unwrap_or_default();
            polled.keybinds = snapshot
                .settings
                .bindings
                .iter()
                .filter_map(|b| {
                    Some(tt_keybind {
                        key: code_of(b.chord.key)?,
                        mods: b.chord.mods.bits(),
                        action: b.action as u8,
                    })
                })
                .collect();
            polled.settings = snapshot.settings;
        }
        let s = &polled.settings;
        let out = tt_config_view {
            struct_size: 0,
            updated: u8::from(updated),
            watching: u8::from(snapshot.watching),
            generation: polled.generation,
            font_family: str_of(&s.font.family),
            font_size: s.font.size,
            scrollback: s.scrollback,
            foreground: s.colors.foreground.value(),
            background: s.colors.background.value(),
            cursor: s.colors.cursor.value(),
            ansi: s.colors.ansi.map(scull_config::Rgb::value),
            keybinds: polled.keybinds.as_ptr(),
            keybinds_len: polled.keybinds.len(),
            error: str_of(&polled.error),
            path: str_of(&config.path),
        };
        // SAFETY: `can_write` checked the caller's struct above.
        unsafe { write_sized(view, &out) };
        tt_status::TT_OK
    })
}

/// Changes one setting in the config file, keeping the rest of the file as
/// it is, and applies it at once. `key` is one of `font.family`,
/// `font.size`, `colors.scheme`, `colors.foreground`, `colors.background`,
/// `colors.cursor`, `scrollback`; `value` is its text (`13.5`,
/// `solarized-dark`, `#1a2b3c`), and an empty `value` removes the key so its
/// default applies. `TT_INVALID` for an unknown key or a value the config
/// does not accept, with the file untouched; `TT_IO` when it cannot be
/// written.
///
/// # Safety
///
/// `config` is `NULL` or live; the strings are valid for their lengths.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_config_set(
    config: *const tt_config,
    key: tt_str,
    value: tt_str,
) -> tt_status {
    guard(|| {
        // SAFETY: the caller's contract.
        let Some(config) = (unsafe { config.as_ref() }) else {
            return tt_status::TT_INVALID;
        };
        // SAFETY: the caller's contract.
        let (Some(key), Some(value)) = (unsafe { (text(key), text(value)) }) else {
            return tt_status::TT_INVALID;
        };
        match config.live.set(key, (!value.is_empty()).then_some(value)) {
            Ok(()) => tt_status::TT_OK,
            Err(scull_config::ConfigError::Io { .. }) => tt_status::TT_IO,
            Err(_) => tt_status::TT_INVALID,
        }
    })
}

/// Frees the configuration and stops its watcher. `NULL` is a no-op. No
/// other call may use `config` during or after this one.
///
/// # Safety
///
/// `config` is `NULL` or live, and not used again.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_config_free(config: *mut tt_config) {
    if config.is_null() {
        return;
    }
    // A panic while dropping leaks what is left rather than abort the host.
    let _ = guard(|| {
        // SAFETY: from `Box::into_raw` in `tt_config_new`, freed only here.
        drop(unsafe { Box::from_raw(config) });
        tt_status::TT_OK
    });
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::ptr;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use super::*;
    use crate::TT_ABI_VERSION;
    use crate::guard::zeroed;

    /// Counts wakeups in the `AtomicUsize` the userdata points to.
    unsafe extern "C" fn wake(counter: *mut c_void) {
        // SAFETY: the tests pass a leaked counter, which lives forever.
        unsafe { &*counter.cast::<AtomicUsize>() }.fetch_add(1, Ordering::SeqCst);
    }

    fn counter() -> &'static AtomicUsize {
        Box::leak(Box::new(AtomicUsize::new(0)))
    }

    /// A scratch config path per test, removed on drop.
    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("scull-ffi-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn file(&self) -> PathBuf {
            self.0.join("config.toml")
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn s(text: &str) -> tt_str {
        str_of(text)
    }

    fn open(path: &str, wakeups: Option<&'static AtomicUsize>) -> *mut tt_config {
        let options = tt_config_options {
            struct_size: u32::try_from(size_of::<tt_config_options>()).unwrap(),
            abi_version: TT_ABI_VERSION,
            path: s(path),
            wakeup: wakeups.map(|_| wake as unsafe extern "C" fn(*mut c_void)),
            userdata: wakeups.map_or(ptr::null_mut(), |c| ptr::from_ref(c).cast_mut().cast()),
        };
        let mut config = ptr::null_mut();
        // SAFETY: valid options and out pointer; `path` outlives the call.
        let status = unsafe { tt_config_new(&options, &mut config) };
        assert_eq!(status, tt_status::TT_OK);
        config
    }

    fn poll(config: *const tt_config) -> tt_config_view {
        let mut view: tt_config_view = zeroed();
        view.struct_size = u32::try_from(size_of::<tt_config_view>()).unwrap();
        // SAFETY: live handle, writable view.
        assert_eq!(
            unsafe { tt_config_poll(config, &mut view) },
            tt_status::TT_OK
        );
        view
    }

    fn text_of(s: tt_str) -> String {
        // SAFETY: a view string is valid until the next poll, which has not happened.
        unsafe { text(s) }.unwrap().to_owned()
    }

    fn poll_until(config: *const tt_config, what: &str, done: impl Fn(&tt_config_view) -> bool) {
        let start = Instant::now();
        while !done(&poll(config)) {
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "timed out: {what}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn the_view_carries_the_defaults_and_the_path() {
        let dir = Dir::new("defaults");
        let path = dir.file();
        let config = open(path.to_str().unwrap(), None);
        let view = poll(config);
        let defaults = Settings::default();
        assert_eq!((view.updated, view.watching), (1, 1));
        assert_eq!(view.font_size, defaults.font.size);
        assert_eq!(text_of(view.font_family), "");
        assert_eq!(text_of(view.path), path.to_str().unwrap());
        assert_eq!(text_of(view.error), "");
        assert_eq!(view.background, defaults.colors.background.value());
        assert_eq!(view.ansi[1], defaults.colors.ansi[1].value());
        assert_eq!(view.keybinds_len, defaults.bindings.len());
        // SAFETY: `keybinds_len` entries at `keybinds`, valid until the next poll.
        let binds = unsafe { std::slice::from_raw_parts(view.keybinds, view.keybinds_len) };
        let paste = binds.iter().find(|b| b.action == TT_ACTION_PASTE).unwrap();
        assert_eq!(
            (paste.key, paste.mods),
            (u32::from('v'), crate::TT_MOD_SUPER)
        );
        assert!(
            binds.iter().any(|b| b.key == crate::TT_KEY_ESCAPE + 10),
            "page up by code"
        );
        assert_eq!(
            poll(config).updated,
            0,
            "nothing changed since the last poll"
        );
        // SAFETY: live handle, not used again.
        unsafe { tt_config_free(config) };
    }

    #[test]
    fn a_change_to_the_file_wakes_the_host_and_shows_in_the_next_poll() {
        let dir = Dir::new("wake");
        let wakeups = counter();
        let config = open(dir.file().to_str().unwrap(), Some(wakeups));
        poll(config);
        assert_eq!(wakeups.load(Ordering::SeqCst), 0);
        fs::write(dir.file(), "[font]\nfamily = \"Menlo\"\nsize = 20\n").unwrap();
        poll_until(config, "the new font", |v| v.font_size == 20.0);
        assert!(wakeups.load(Ordering::SeqCst) >= 1);
        let view = poll(config);
        assert_eq!(view.updated, 0);
        fs::write(dir.file(), "scrollback = 12\n").unwrap();
        poll_until(config, "the new scrollback", |v| v.scrollback == 12);
        // SAFETY: live handle, not used again.
        unsafe { tt_config_free(config) };
    }

    #[test]
    fn a_broken_file_is_an_error_beside_the_last_good_settings() {
        let dir = Dir::new("broken");
        fs::write(dir.file(), "scrollback = 3\n").unwrap();
        let config = open(dir.file().to_str().unwrap(), None);
        assert_eq!(poll(config).scrollback, 3);
        fs::write(dir.file(), "scrollback = -3\n").unwrap();
        poll_until(config, "the error", |v| v.error.len > 0);
        let view = poll(config);
        assert_eq!(view.scrollback, 3);
        assert!(text_of(view.error).contains("config.toml"));
        // SAFETY: live handle, not used again.
        unsafe { tt_config_free(config) };
    }

    #[test]
    fn set_writes_the_file_and_the_next_poll_sees_it() {
        let dir = Dir::new("set");
        let config = open(dir.file().to_str().unwrap(), None);
        poll(config);
        // SAFETY: live handle; strings outlive the calls.
        unsafe {
            assert_eq!(
                tt_config_set(config, s("font.size"), s("17")),
                tt_status::TT_OK
            );
            assert_eq!(
                tt_config_set(config, s("colors.scheme"), s("scull-light")),
                tt_status::TT_OK
            );
            assert_eq!(
                tt_config_set(config, s("font.size"), s("500")),
                tt_status::TT_INVALID
            );
            assert_eq!(
                tt_config_set(config, s("bogus"), s("1")),
                tt_status::TT_INVALID
            );
        }
        let view = poll(config);
        assert_eq!((view.updated, view.font_size), (1, 17.0));
        assert_eq!(
            view.background,
            scull_config::Scheme::ScullLight.theme().background.value()
        );
        // SAFETY: live handle; strings outlive the call.
        unsafe {
            assert_eq!(
                tt_config_set(config, s("font.size"), s("")),
                tt_status::TT_OK
            );
            tt_config_free(config);
        }
        assert!(!fs::read_to_string(dir.file()).unwrap().contains("size"));
    }

    #[test]
    fn arguments_are_checked() {
        let dir = Dir::new("args");
        let mut out = ptr::null_mut();
        let good = tt_config_options {
            struct_size: u32::try_from(size_of::<tt_config_options>()).unwrap(),
            abi_version: TT_ABI_VERSION,
            path: s(dir.file().to_str().unwrap()),
            ..zeroed()
        };
        let other_abi = tt_config_options {
            abi_version: TT_ABI_VERSION + 1,
            ..good
        };
        let bad_utf8 = [0xFF_u8, 0xFE];
        let not_text = tt_config_options {
            path: tt_str {
                ptr: bad_utf8.as_ptr(),
                len: 2,
            },
            ..good
        };
        // SAFETY: NULL and invalid values are exactly what is being passed.
        unsafe {
            assert_eq!(tt_config_new(ptr::null(), &mut out), tt_status::TT_INVALID);
            assert_eq!(tt_config_new(&good, ptr::null_mut()), tt_status::TT_INVALID);
            assert_eq!(
                tt_config_new(&other_abi, &mut out),
                tt_status::TT_ABI_MISMATCH
            );
            assert_eq!(tt_config_new(&not_text, &mut out), tt_status::TT_INVALID);
            assert_eq!(
                tt_config_poll(ptr::null(), &mut zeroed()),
                tt_status::TT_INVALID
            );
            assert_eq!(
                tt_config_set(ptr::null(), s("a"), s("b")),
                tt_status::TT_INVALID
            );
            tt_config_free(ptr::null_mut());
        }
        let config = open(dir.file().to_str().unwrap(), None);
        // SAFETY: live handle; a NULL view is refused, a view of 4 bytes gets only its size back.
        unsafe {
            assert_eq!(
                tt_config_poll(config, ptr::null_mut()),
                tt_status::TT_INVALID
            );
            let mut tiny = [4_u32, 0xAAAA_AAAA];
            let status = tt_config_poll(config, tiny.as_mut_ptr().cast());
            assert_eq!((status, tiny), (tt_status::TT_OK, [4, 0xAAAA_AAAA]));
            assert_eq!(
                tt_config_set(
                    config,
                    tt_str {
                        ptr: ptr::null(),
                        len: 3
                    },
                    s("")
                ),
                tt_status::TT_INVALID
            );
            tt_config_free(config);
        }
    }

    #[test]
    fn no_wakeup_arrives_after_free() {
        let dir = Dir::new("free");
        let wakeups = counter();
        let config = open(dir.file().to_str().unwrap(), Some(wakeups));
        poll(config);
        // SAFETY: live handle, not used again.
        unsafe { tt_config_free(config) };
        fs::write(dir.file(), "scrollback = 1\n").unwrap();
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(wakeups.load(Ordering::SeqCst), 0);
    }
}
