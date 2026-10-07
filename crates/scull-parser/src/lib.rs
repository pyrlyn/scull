//! VT parser: a bulk scanner that hands printable runs over in one piece,
//! in front of a table state machine for CSI, OSC, DCS and APC that emits
//! typed actions. Separate from the terminal state so it can be fuzzed and
//! benchmarked on its own.
