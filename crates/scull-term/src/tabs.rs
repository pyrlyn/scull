//! Horizontal tab stops (HTS, TBC, HT, CHT, CBT). Separate because the stop
//! set is its own small table, sized to the screen and reset by RIS.

/// Power-on tab stops fall every eight columns (VT100 and xterm).
const DEFAULT_TAB_WIDTH: usize = 8;

/// One flag per column: a tab stop is set there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TabStops(Vec<bool>);

impl TabStops {
    /// Stops every eight columns, the first at column 8.
    pub(crate) fn new(cols: u16) -> Self {
        Self(
            (0..usize::from(cols))
                .map(|c| c > 0 && c % DEFAULT_TAB_WIDTH == 0)
                .collect(),
        )
    }

    /// HTS: a stop at `col`.
    pub(crate) fn set(&mut self, col: u16) {
        if let Some(stop) = self.0.get_mut(usize::from(col)) {
            *stop = true;
        }
    }

    /// TBC 0: no stop at `col`.
    pub(crate) fn clear(&mut self, col: u16) {
        if let Some(stop) = self.0.get_mut(usize::from(col)) {
            *stop = false;
        }
    }

    /// TBC 3: no stops at all.
    pub(crate) fn clear_all(&mut self) {
        self.0.fill(false);
    }

    /// The `n`th stop right of `col`, or `limit` when fewer stops lie before it.
    pub(crate) fn next(&self, col: u16, n: u16, limit: u16) -> u16 {
        let mut at = col;
        for _ in 0..n {
            match (at + 1..limit).find(|&c| self.is_set(c)) {
                Some(stop) => at = stop,
                None => return limit.max(col),
            }
        }
        at
    }

    /// The `n`th stop left of `col`, or column 0 when fewer stops lie there.
    pub(crate) fn prev(&self, col: u16, n: u16) -> u16 {
        let mut at = col;
        for _ in 0..n {
            match (0..at).rev().find(|&c| self.is_set(c)) {
                Some(stop) => at = stop,
                None => return 0,
            }
        }
        at
    }

    fn is_set(&self, col: u16) -> bool {
        self.0.get(usize::from(col)).copied().unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLS: u16 = 30;
    const LAST: u16 = COLS - 1;

    #[test]
    fn default_stops_fall_every_eight_columns() {
        let tabs = TabStops::new(COLS);
        assert_eq!(tabs.next(0, 1, LAST), 8);
        assert_eq!(tabs.next(8, 1, LAST), 16);
        assert_eq!(tabs.next(0, 3, LAST), 24);
    }

    #[test]
    fn next_stops_at_the_limit_when_stops_run_out() {
        let tabs = TabStops::new(COLS);
        assert_eq!(tabs.next(25, 1, LAST), LAST);
        assert_eq!(tabs.next(3, 9, LAST), LAST);
        assert_eq!(tabs.next(LAST, 1, LAST), LAST);
    }

    #[test]
    fn prev_stops_at_column_zero() {
        let tabs = TabStops::new(COLS);
        assert_eq!(tabs.prev(20, 1), 16);
        assert_eq!(tabs.prev(16, 2), 0);
        assert_eq!(tabs.prev(0, 1), 0);
    }

    #[test]
    fn set_and_clear_change_single_columns() {
        let mut tabs = TabStops::new(COLS);
        tabs.set(3);
        tabs.clear(8);
        assert_eq!(tabs.next(0, 1, LAST), 3);
        assert_eq!(tabs.next(3, 1, LAST), 16);
        tabs.clear_all();
        assert_eq!(tabs.next(0, 1, LAST), LAST);
        tabs.set(COLS + 5);
        assert_eq!(tabs.next(0, 1, LAST), LAST, "out of range is ignored");
    }
}
