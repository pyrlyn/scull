//! Terminals with a real child on a real PTY, driven only through the C
//! exports: the wakeup, the frame, the event queue, input and resize.
//! Apart from the unit tests because they need the operating system's
//! shell.

// Helpers outside #[test] functions are still test code; a failure here should abort loudly.
#![allow(clippy::unwrap_used, clippy::panic)]
// The exports are `unsafe extern "C"`; calling them is the point.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::ptr;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use scull_ffi::{
    TT_ABI_VERSION, TT_EVENT_BELL, TT_EVENT_CHILD_EXITED, tt_event, tt_frame, tt_frame_free,
    tt_frame_new, tt_frame_update, tt_frame_view, tt_status, tt_str, tt_term, tt_term_free,
    tt_term_new, tt_term_options, tt_term_poll_event, tt_term_resize, tt_term_resize_begin,
    tt_term_spawn, tt_term_write,
};

/// A child that is slow to start on a loaded machine still finishes well inside this.
const GENEROUS: Duration = Duration::from_secs(30);

const EXIT_CODE: u32 = 3;

fn s(text: &str) -> tt_str {
    tt_str {
        ptr: text.as_ptr(),
        len: text.len(),
    }
}

fn size_of_u32<T>() -> u32 {
    u32::try_from(size_of::<T>()).unwrap()
}

/// Options for `program args` on a 20 x 4 screen, waking `wakeups`.
fn options(program: &tt_str, args: &[tt_str], wakeups: &Sender<()>) -> tt_term_options {
    // SAFETY: `tt_term_options` is plain data; all-zero means "not set".
    let mut options: tt_term_options = unsafe { std::mem::zeroed() };
    options.struct_size = size_of_u32::<tt_term_options>();
    options.abi_version = TT_ABI_VERSION;
    options.cols = 20;
    options.rows = 4;
    options.program = *program;
    options.args = args.as_ptr();
    options.args_len = args.len();
    options.wakeup = Some(wakeup);
    options.userdata = ptr::from_ref(wakeups).cast_mut().cast();
    options
}

unsafe extern "C" fn wakeup(userdata: *mut c_void) {
    // SAFETY: every test passes a `Sender<()>` that outlives its terminal.
    let wakeups = unsafe { &*userdata.cast::<Sender<()>>() };
    let _ = wakeups.send(());
}

/// What a host sees of one terminal.
struct Host {
    term: *mut tt_term,
    frame: *mut tt_frame,
    wakeups: Receiver<()>,
    _sender: Box<Sender<()>>,
    bells: usize,
    exit: Option<tt_event>,
    screen: String,
}

impl Host {
    fn spawn(program: &str, args: &[&str]) -> Self {
        let (tx, wakeups) = channel();
        let tx = Box::new(tx);
        let program = s(program);
        let args: Vec<tt_str> = args.iter().map(|a| s(a)).collect();
        let mut term = ptr::null_mut();
        // SAFETY: the options' strings live through the call.
        let status = unsafe { tt_term_spawn(&options(&program, &args, &tx), &mut term) };
        assert_eq!(status, tt_status::TT_OK);
        Self {
            term,
            frame: tt_frame_new(),
            wakeups,
            _sender: tx,
            bells: 0,
            exit: None,
            screen: String::new(),
        }
    }

    /// What a host does on a wakeup: drain the events, then redraw.
    fn look(&mut self) {
        loop {
            // SAFETY: `tt_event` is plain data.
            let mut event: tt_event = unsafe { std::mem::zeroed() };
            event.struct_size = size_of_u32::<tt_event>();
            // SAFETY: live handle and a whole event.
            match unsafe { tt_term_poll_event(self.term, &mut event) } {
                tt_status::TT_EMPTY => break,
                tt_status::TT_OK if event.kind == TT_EVENT_BELL => self.bells += 1,
                tt_status::TT_OK if event.kind == TT_EVENT_CHILD_EXITED => self.exit = Some(event),
                other => panic!("poll answered {other:?}, kind {}", event.kind),
            }
        }
        // SAFETY: `tt_frame_view` is plain data.
        let mut view: tt_frame_view = unsafe { std::mem::zeroed() };
        view.struct_size = size_of_u32::<tt_frame_view>();
        // SAFETY: live handles and a whole view.
        let status = unsafe { tt_frame_update(self.frame, self.term, &mut view) };
        assert_eq!(status, tt_status::TT_OK);
        // SAFETY: the view's pointers hold until the next update.
        let rows = unsafe { std::slice::from_raw_parts(view.lines, view.lines_len) };
        self.screen = rows
            .iter()
            .map(|row| {
                // SAFETY: as above.
                let text = unsafe { std::slice::from_raw_parts(row.text, row.text_len) };
                String::from_utf8_lossy(text).into_owned()
            })
            .collect::<Vec<_>>()
            .join("\n");
    }

    /// Looks after every wakeup, not on a clock, until `done` holds.
    fn wait_for(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + GENEROUS;
        self.look();
        while !done(self) {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                self.wakeups.recv_timeout(left).is_ok(),
                "timed out waiting for {what}; screen so far: {:?}",
                self.screen
            );
            self.look();
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        // SAFETY: live handles, not used again; the terminal goes first so
        // no wakeup reaches the sender after it is dropped.
        unsafe {
            tt_term_free(self.term);
            tt_frame_free(self.frame);
        }
    }
}

#[cfg(unix)]
#[test]
fn output_bell_and_exit_reach_the_host_in_order() {
    let mut host = Host::spawn("/bin/sh", &["-c", "printf hi; printf '\\a'; exit 3"]);
    host.wait_for("the exit", |h| h.exit.is_some());
    assert!(host.screen.contains("hi"), "{:?}", host.screen);
    assert_eq!(host.bells, 1);
    let exit = host.exit.unwrap();
    assert_eq!((exit.exit_code, exit.signaled), (EXIT_CODE, 0));
}

#[cfg(windows)]
#[test]
fn output_and_exit_reach_the_host() {
    let mut host = Host::spawn("cmd", &["/C", "echo hi & exit 3"]);
    host.wait_for("the exit", |h| h.exit.is_some());
    assert!(host.screen.contains("hi"), "{:?}", host.screen);
    assert_eq!(host.exit.unwrap().exit_code, EXIT_CODE);
}

#[cfg(unix)]
#[test]
fn input_reaches_the_child_and_a_resize_is_seen_by_it() {
    // `read` holds the child until the resize and the line are in, so the
    // size it prints cannot be the initial one.
    let mut host = Host::spawn("/bin/sh", &["-c", "read x; stty size; echo got $x"]);
    // SAFETY: live handle.
    unsafe {
        assert_eq!(tt_term_resize_begin(host.term), tt_status::TT_OK);
        assert_eq!(tt_term_resize(host.term, 30, 5, 300, 100), tt_status::TT_OK);
    }
    let line = b"ping\r";
    let mut written = 0;
    // SAFETY: live handle, `line` readable, `written` writable.
    let status = unsafe { tt_term_write(host.term, line.as_ptr(), line.len(), &mut written) };
    assert_eq!((status, written), (tt_status::TT_OK, line.len()));
    host.wait_for("the reply", |h| h.screen.contains("got ping"));
    assert!(host.screen.contains("5 30"), "{:?}", host.screen);
}

#[test]
fn a_missing_program_is_an_io_error_and_bad_strings_are_invalid() {
    let (tx, _rx) = channel();
    let spawn = |options: &tt_term_options| {
        let mut term = ptr::null_mut();
        // SAFETY: the options' strings live through the call.
        let status = unsafe { tt_term_spawn(options, &mut term) };
        // SAFETY: NULL or the handle just made.
        unsafe { tt_term_free(term) };
        status
    };
    let missing = s("scull-no-such-program-anywhere");
    assert_eq!(spawn(&options(&missing, &[], &tx)), tt_status::TT_IO);
    let not_utf8 = [0xFF_u8];
    let bad = tt_str {
        ptr: not_utf8.as_ptr(),
        len: 1,
    };
    assert_eq!(spawn(&options(&bad, &[], &tx)), tt_status::TT_INVALID);
    let dangling = tt_str {
        ptr: ptr::null(),
        len: 1,
    };
    assert_eq!(
        spawn(&options(&missing, &[dangling], &tx)),
        tt_status::TT_INVALID
    );
    for entry in ["NO_EQUALS", "=value"] {
        let env = [s(entry)];
        let mut with_env = options(&missing, &[], &tx);
        with_env.env = env.as_ptr();
        with_env.env_len = env.len();
        assert_eq!(spawn(&with_env), tt_status::TT_INVALID, "{entry}");
    }
}

#[test]
fn a_terminal_without_a_child_has_no_input_to_take() {
    let (tx, _rx) = channel();
    let mut term = ptr::null_mut();
    // SAFETY: valid options and out pointer; the spawn fields are ignored.
    let status = unsafe { tt_term_new(&options(&s(""), &[], &tx), &mut term) };
    assert_eq!(status, tt_status::TT_OK);
    let mut written = 1;
    // SAFETY: live handle; NULL bytes with no length are allowed.
    unsafe {
        assert_eq!(
            tt_term_write(term, ptr::null(), 0, &mut written),
            tt_status::TT_CLOSED
        );
        assert_eq!(written, 0);
        assert_eq!(
            tt_term_write(term, ptr::null(), 0, ptr::null_mut()),
            tt_status::TT_INVALID
        );
        assert_eq!(tt_term_resize_begin(term), tt_status::TT_OK);
        assert_eq!(tt_term_resize(term, 10, 2, 0, 0), tt_status::TT_OK);
        tt_term_free(term);
    }
}
