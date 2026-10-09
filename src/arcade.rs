//! What the arcade savers share, and chess's counters: a 3x5 pixel font for
//! scores, a decimal formatter that never allocates, and the quadrant glyphs
//! that draw a 2x2-pixel bitmap into one cell.

use crate::font;
use crate::glyph;

/// Rows of a 3x5 glyph, top first, bit 2 the left pixel.
pub type Glyph3 = [u8; 5];

#[rustfmt::skip]
const DIGITS: [Glyph3; 10] = [
    [7, 5, 5, 5, 7], [2, 6, 2, 2, 7], [7, 1, 7, 4, 7], [7, 1, 7, 1, 7], [5, 5, 7, 1, 1],
    [7, 4, 7, 1, 7], [7, 4, 7, 5, 7], [7, 1, 1, 1, 1], [7, 5, 7, 5, 7], [7, 5, 7, 1, 7],
];

#[rustfmt::skip]
const LETTERS: [Glyph3; 26] = [
    [2, 5, 7, 5, 5], [6, 5, 6, 5, 6], [3, 4, 4, 4, 3], [6, 5, 5, 5, 6], [7, 4, 6, 4, 7],
    [7, 4, 6, 4, 4], [3, 4, 5, 5, 3], [5, 5, 7, 5, 5], [7, 2, 2, 2, 7], [1, 1, 1, 5, 2],
    [5, 5, 6, 5, 5], [4, 4, 4, 4, 7], [7, 7, 5, 5, 5], [5, 7, 7, 5, 5], [2, 5, 5, 5, 2],
    [6, 5, 6, 4, 4], [2, 5, 5, 6, 3], [6, 5, 6, 5, 5], [3, 4, 2, 1, 6], [7, 2, 2, 2, 2],
    [5, 5, 5, 5, 7], [5, 5, 5, 5, 2], [5, 5, 7, 7, 5], [5, 5, 2, 5, 5], [5, 5, 2, 2, 2],
    [7, 1, 2, 4, 7],
];

/// The 3x5 glyph for an ASCII digit or capital; anything else is blank.
pub const fn glyph3(c: u8) -> Glyph3 {
    match c {
        b'0'..=b'9' => DIGITS[(c - b'0') as usize],
        b'A'..=b'Z' => LETTERS[(c - b'A') as usize],
        b'-' => [0, 0, 7, 0, 0],
        _ => [0; 5],
    }
}

/// Each lit pixel of `s` in the 3x5 font at scale 1, as `(x, y)`.
#[inline]
pub fn each_pixel(s: &[u8], mut f: impl FnMut(usize, usize)) {
    for (i, &ch) in s.iter().enumerate() {
        for (y, row) in glyph3(ch).iter().enumerate() {
            for c in 0..3 {
                if row & (4 >> c) != 0 {
                    f(i * 4 + c, y);
                }
            }
        }
    }
}

/// Pixels across `len` glyphs at scale 1, one blank column between each.
pub const fn text_w(len: usize) -> usize {
    if len == 0 {
        0
    } else {
        4 * len - 1
    }
}

/// `n` in decimal, into the tail of `buf`, without allocating; capped at the
/// largest value `buf` holds so a score past the field shows nines.
pub fn decimal(n: u64, buf: &mut [u8; 10], max_digits: usize) -> &[u8] {
    let max_digits = max_digits.clamp(1, 10);
    let cap = 10u64.pow(max_digits as u32) - 1;
    let mut v = n.min(cap);
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    &buf[i..]
}

/// The glyph whose lit quadrants are `bits`: 1 upper-left, 2 upper-right,
/// 4 lower-left, 8 lower-right. The font has no `▚` or `▞`, so those two keep
/// one quadrant — they only occur on a sprite's ragged edge.
pub const QUADS: [u16; 16] = [
    font::BLANK,
    glyph::of('▘'),
    glyph::of('▝'),
    font::UPPER,
    glyph::of('▖'),
    glyph::of('▌'),
    glyph::of('▝'),
    font::NO_LR,
    glyph::of('▗'),
    glyph::of('▘'),
    glyph::of('▐'),
    font::NO_LL,
    font::LOWER,
    font::NO_UR,
    font::NO_UL,
    font::SOLID,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_caps_and_never_allocates() {
        let mut b = [0u8; 10];
        assert_eq!(decimal(0, &mut b, 6), b"0");
        assert_eq!(decimal(1234, &mut b, 6), b"1234");
        assert_eq!(decimal(1_234_567, &mut b, 6), b"999999");
        let n = crate::testalloc::allocs_during(|| {
            let _ = decimal(42, &mut b, 3);
        });
        assert_eq!(n, 0);
    }

    #[test]
    fn every_letter_and_digit_lights_something() {
        for c in (b'0'..=b'9').chain(b'A'..=b'Z') {
            let g = glyph3(c);
            assert!(g.iter().all(|r| r >> 3 == 0), "{}", c as char);
            let lit: u32 = g.iter().map(|r| r.count_ones()).sum();
            assert!(lit >= 5, "{}", c as char);
        }
    }

    #[test]
    fn quads_are_the_block_elements_they_name() {
        assert_eq!(QUADS[0], font::BLANK);
        assert_eq!(QUADS[15], font::SOLID);
        assert_eq!(QUADS[3], glyph::of('▀'));
        assert_eq!(QUADS[12], glyph::of('▄'));
        assert_eq!(QUADS[5], glyph::of('▌'));
        assert_eq!(QUADS[10], glyph::of('▐'));
    }
}
