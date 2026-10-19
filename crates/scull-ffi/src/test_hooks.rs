//! Exports that exist only for a host's own tests (`--features test-hooks`).
#![allow(unsafe_code, clippy::panic)] // A no_mangle export; the panic is the point.

use crate::guard::{tt_status, with_term};
use crate::term::tt_term;

/// Panics inside the core on `term`, as a core bug would: the call answers
/// `TT_PANIC` and the terminal is poisoned. Lets a host prove one crashed
/// terminal leaves the others running, which no input can trigger.
///
/// # Safety
///
/// `term` is `NULL` or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tt_term_test_panic(term: *const tt_term) -> tt_status {
    // SAFETY: the caller's contract.
    unsafe { with_term(term, |_| panic!("test hook: simulated core bug")) }
}
