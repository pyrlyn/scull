//! The bulk text scanner: finds the next byte that can end a printable run,
//! eight bytes per step. Separate because it is the hot loop for plain output
//! and is benchmarked and tested on its own.
//!
//! The word tests are the "determine if a word has a byte less than n" and
//! "has a zero byte" tricks from Sean Eron Anderson's Bit Twiddling Hacks
//! (public domain). Each answers exactly whether such a byte exists, so a
//! word that passes needs no further look.

/// Lead byte of the two-byte UTF-8 forms of U+0080..=U+00BF, which include
/// the C1 controls U+0080..=U+009F.
pub(crate) const C1_LEAD: u8 = 0xC2;
/// Last continuation byte that, after [`C1_LEAD`], encodes a C1 control.
pub(crate) const C1_LAST: u8 = 0x9F;
const C1_FIRST: u8 = 0x80;
/// First byte that is not a C0 control.
const SPACE: u8 = 0x20;
const DEL: u8 = 0x7F;

const LANES: usize = size_of::<u64>();
const ONES: u64 = u64::from_ne_bytes([1; LANES]);
const HIGH_BITS: u64 = ONES << (u8::BITS - 1);

const fn splat(byte: u8) -> u64 {
    u64::from_ne_bytes([byte; LANES])
}

const fn has_less_than(word: u64, bound: u8) -> bool {
    word.wrapping_sub(splat(bound)) & !word & HIGH_BITS != 0
}

const fn has_byte(word: u64, byte: u8) -> bool {
    let x = word ^ splat(byte);
    x.wrapping_sub(ONES) & !x & HIGH_BITS != 0
}

/// A byte that stops a printable run: C0, DEL, or a possible C1 lead.
pub(crate) const fn is_special(byte: u8) -> bool {
    byte < SPACE || byte == DEL || byte == C1_LEAD
}

/// Whether the byte after [`C1_LEAD`] makes the pair a C1 control.
pub(crate) const fn is_c1_tail(byte: u8) -> bool {
    byte >= C1_FIRST && byte <= C1_LAST
}

/// Index of the first [`is_special`] byte, or `bytes.len()`.
pub(crate) fn find_special(bytes: &[u8]) -> usize {
    find(bytes, is_special, |w| {
        has_less_than(w, SPACE) || has_byte(w, DEL) || has_byte(w, C1_LEAD)
    })
}

/// Index of the first C0 control, or `bytes.len()`.
pub(crate) fn find_c0(bytes: &[u8]) -> usize {
    find(bytes, |b| b < SPACE, |w| has_less_than(w, SPACE))
}

#[inline(always)]
fn find(bytes: &[u8], byte_hit: impl Fn(u8) -> bool, word_hit: impl Fn(u64) -> bool) -> usize {
    let (words, _) = bytes.as_chunks::<LANES>();
    let mut skipped = 0;
    for word in words {
        if word_hit(u64::from_ne_bytes(*word)) {
            break;
        }
        skipped += LANES;
    }
    bytes
        .get(skipped..)
        .and_then(|rest| rest.iter().position(|&b| byte_hit(b)))
        .map_or(bytes.len(), |i| skipped + i)
}

/// Length of the UTF-8 sequence a lead byte starts; 0 if it cannot start one.
pub(crate) const fn utf8_width(lead: u8) -> usize {
    match lead {
        0x00..=0x7F => 1,
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn naive(bytes: &[u8]) -> usize {
        bytes
            .iter()
            .position(|&b| is_special(b))
            .unwrap_or(bytes.len())
    }

    #[test]
    fn plain_ascii_has_no_special_byte() {
        let text = b"the quick brown fox jumps over the lazy dog";
        assert_eq!(find_special(text), text.len());
    }

    #[test]
    fn every_special_byte_is_found_at_every_offset() {
        let specials = (0..SPACE).chain([DEL, C1_LEAD]);
        for special in specials {
            for at in 0..LANES * 3 {
                let mut bytes = vec![b'a'; LANES * 3];
                bytes[at] = special;
                assert_eq!(find_special(&bytes), at, "byte {special:#x} at {at}");
            }
        }
    }

    #[test]
    fn high_bytes_other_than_c1_lead_do_not_stop_a_run() {
        let text = "héllo wörld — ünïcode 日本語 🎉".as_bytes();
        let expected = naive(text);
        assert_eq!(find_special(text), expected);
    }

    #[test]
    fn c0_search_ignores_del_and_c1_lead() {
        let bytes = [b'a', DEL, C1_LEAD, b'b', b'c', b'd', b'e', b'f', b'g', 0x07];
        assert_eq!(find_c0(&bytes), bytes.len() - 1);
    }

    #[test]
    fn word_test_agrees_with_the_byte_test_on_every_byte_pair() {
        for a in 0..=u8::MAX {
            for b in 0..=u8::MAX {
                let bytes = [b'x', b'x', b'x', a, b, b'x', b'x', b'x'];
                assert_eq!(find_special(&bytes), naive(&bytes), "{a:#x} {b:#x}");
            }
        }
    }
}
