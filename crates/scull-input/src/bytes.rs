//! Byte-level building blocks every encoder shares: the 7-bit control
//! introducers and allocation-free number and character output. Kept apart
//! so the key, mouse, paste and focus encoders spell a sequence one way.

/// ESC: starts every 7-bit sequence and is xterm's Alt prefix.
pub(crate) const ESC: u8 = 0x1b;
/// CSI in 7-bit form; programs expect 7-bit controls from the keyboard.
pub(crate) const CSI: [u8; 2] = [ESC, b'['];
/// SS3 in 7-bit form, used by application cursor and keypad modes.
pub(crate) const SS3: [u8; 2] = [ESC, b'O'];
/// Parameter separator inside a CSI sequence.
pub(crate) const SEMI: u8 = b';';
/// Sub-parameter separator inside a CSI parameter.
pub(crate) const COLON: u8 = b':';

/// Digits in `u32::MAX`, the widest number any encoder writes.
const MAX_U32_DIGITS: usize = 10;
pub(crate) const DECIMAL_RADIX: u32 = 10;

/// Appends `n` in decimal without going through `fmt`, which would need a
/// `Result` that cannot fail on a `Vec`.
pub(crate) fn push_decimal(out: &mut Vec<u8>, mut n: u32) {
    let mut digits = [0_u8; MAX_U32_DIGITS];
    let mut start = digits.len();
    loop {
        start -= 1;
        // `n % 10` is below 10, so the cast cannot truncate.
        digits[start] = b'0' + (n % DECIMAL_RADIX) as u8;
        n /= DECIMAL_RADIX;
        if n == 0 {
            break;
        }
    }
    out.extend_from_slice(&digits[start..]);
}

/// Appends `c` as UTF-8.
pub(crate) fn push_char(out: &mut Vec<u8>, c: char) {
    let mut buf = [0_u8; char::MAX_LEN_UTF8];
    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_output_matches_display_at_the_edges() {
        for n in [0, 1, 9, 10, 255, 57_441, u32::MAX] {
            let mut out = Vec::new();
            push_decimal(&mut out, n);
            assert_eq!(out, n.to_string().into_bytes());
        }
    }

    #[test]
    fn char_output_is_utf8() {
        let mut out = Vec::new();
        push_char(&mut out, 'ж');
        push_char(&mut out, '😀');
        assert_eq!(out, "ж😀".as_bytes());
    }
}
