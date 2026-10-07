//! Numeric parameters of a CSI or DCS header, with colon sub-parameters.
//! Separate because it is the one piece of sequence state a hostile stream can
//! try to grow, so its cap and storage live in one fixed-size place.

/// Most parameter values one sequence keeps, sub-parameters included.
/// `38:2::255:255:255` is six values, and xterm stops at 30 parameters; 32
/// fits both with room and fills a `u32` sub-parameter mask exactly.
pub const MAX_PARAMS: usize = 32;

const DECIMAL_BASE: u16 = 10;

/// Parameters in a fixed array; values saturate at `u16::MAX`, which is far
/// beyond any row, column or mode number a terminal acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Params {
    values: [u16; MAX_PARAMS],
    /// Bit `i` set: value `i` followed a `:` and belongs to the parameter before it.
    sub: u32,
    len: usize,
    /// A value slot is open and still taking digits.
    open: bool,
    /// The next value to open follows a `:`.
    next_is_sub: bool,
    /// A separator was seen, so an empty trailing parameter counts.
    separated: bool,
    truncated: bool,
}

impl Params {
    /// Number of values, sub-parameters included.
    pub fn len(&self) -> usize {
        self.len
    }

    /// No parameters at all, as in `CSI m`.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Each parameter with its sub-parameters: `1;38:5:9` yields `[1]`, `[38, 5, 9]`.
    /// An empty parameter reads as `0`, which every consumer treats as its default.
    pub fn iter(&self) -> impl Iterator<Item = &[u16]> {
        let mut start = 0;
        std::iter::from_fn(move || {
            if start >= self.len {
                return None;
            }
            let mut end = start + 1;
            while end < self.len && self.sub & (1 << end) != 0 {
                end += 1;
            }
            let group = self.values.get(start..end);
            start = end;
            group
        })
    }

    pub(crate) fn truncated(&self) -> bool {
        self.truncated
    }

    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    /// Appends ASCII digits to the open value, opening one if needed.
    pub(crate) fn digits(&mut self, digits: &[u8]) {
        if !self.open && !self.start_value() {
            return;
        }
        if let Some(value) = self.len.checked_sub(1).and_then(|i| self.values.get_mut(i)) {
            *value = digits.iter().fold(*value, |acc, &byte| {
                let digit = u16::from(byte.saturating_sub(b'0'));
                acc.saturating_mul(DECIMAL_BASE).saturating_add(digit)
            });
        }
    }

    /// `;` starts a new parameter, `:` a sub-parameter of the current one.
    pub(crate) fn separator(&mut self, colon: bool) {
        if !self.open {
            self.start_value();
        }
        self.open = false;
        self.separated = true;
        self.next_is_sub = colon;
    }

    /// Closes the sequence: a trailing separator leaves one empty parameter.
    pub(crate) fn finish(&mut self) {
        if !self.open && self.separated {
            self.start_value();
        }
        self.open = false;
    }

    fn start_value(&mut self) -> bool {
        let Some(slot) = self.values.get_mut(self.len) else {
            self.truncated = true;
            return false;
        };
        *slot = 0;
        if self.next_is_sub {
            self.sub |= 1 << self.len;
        }
        self.len += 1;
        self.open = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Params {
        let mut p = Params::default();
        for b in text.bytes() {
            match b {
                b'0'..=b'9' => p.digits(&[b]),
                b';' | b':' => p.separator(b == b':'),
                _ => unreachable!(),
            }
        }
        p.finish();
        p
    }

    fn groups(p: &Params) -> Vec<Vec<u16>> {
        p.iter().map(<[u16]>::to_vec).collect()
    }

    #[test]
    fn no_digits_means_no_parameters() {
        assert!(parse("").is_empty());
    }

    #[test]
    fn empty_parameters_read_as_zero() {
        assert_eq!(groups(&parse(";")), [vec![0], vec![0]]);
        assert_eq!(groups(&parse("1;")), [vec![1], vec![0]]);
    }

    #[test]
    fn colon_groups_sub_parameters_with_their_parameter() {
        assert_eq!(
            groups(&parse("1;38:2::10:20:30;4")),
            [vec![1], vec![38, 2, 0, 10, 20, 30], vec![4]]
        );
    }

    #[test]
    fn huge_values_saturate() {
        assert_eq!(groups(&parse("99999999999")), [vec![u16::MAX]]);
    }

    #[test]
    fn parameters_past_the_cap_are_dropped_and_flagged() {
        let text = vec!["7"; MAX_PARAMS + 10].join(";");
        let p = parse(&text);
        assert_eq!(p.len(), MAX_PARAMS);
        assert!(p.truncated());
        assert!(p.iter().all(|g| g == [7]));
    }

    #[test]
    fn sub_parameters_count_against_the_same_cap() {
        let text = format!("38{}", ":1".repeat(MAX_PARAMS * 2));
        let p = parse(&text);
        assert_eq!(p.len(), MAX_PARAMS);
        assert!(p.truncated());
        assert_eq!(p.iter().count(), 1);
    }
}
