//! Terminal state: cursor, modes, charsets, margins and screens, driven by
//! the parser's actions and writing into the grid. Pure state with no I/O, so
//! conformance suites replay byte streams against it headlessly.
