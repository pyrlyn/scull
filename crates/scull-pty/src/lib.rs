//! The pseudo-terminal and its reader thread. The only crate that spawns
//! child processes, so back-pressure and child exit live in one place.
