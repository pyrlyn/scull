//! The cell grid: fixed-size cells, rows with stable ids, and the ring shared
//! by the screen and scrollback. Knows nothing about escape sequences so the
//! storage can be tested and measured without a parser.
