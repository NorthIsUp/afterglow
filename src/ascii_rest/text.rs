//! Characters for the ascii.rest text pieces, which upstream return as a
//! string and here write cells.

use crate::font;
use crate::grid::Cell;

/// The glyph for `c`: printable ASCII, or one of `font::TEXT`. Anything else
/// is a port bug — `?` in a release build so a dump shows where, a panic in a
/// test so it never ships. `const` so a piece's ramp is a table, not state.
#[inline]
pub const fn glyph(c: char) -> u16 {
    let u = c as u32;
    if u >= 0x20 && u < 0x7F {
        return font::ASCII[(u - 0x20) as usize];
    }
    let (mut lo, mut hi) = (0, font::TEXT.len());
    while lo < hi {
        let mid = (lo + hi) / 2;
        let k = font::TEXT[mid].0 as u32;
        if k == u {
            return font::TEXT[mid].1;
        }
        if k < u {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    debug_assert!(false, "no glyph for a char; add it to genfont's TEXT_CHARS");
    font::ASCII[(b'?' - 0x20) as usize]
}

/// `c` in the piece's one ink.
#[inline]
pub const fn cell(c: char) -> Cell {
    Cell::new(glyph(c), 0)
}

/// A row of chars as cells, for a piece's ramp consts.
pub const fn cells<const N: usize>(cs: [char; N]) -> [Cell; N] {
    let mut out = [Cell::CLEAR; N];
    let mut i = 0;
    while i < N {
        out[i] = cell(cs[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_text_char_resolves() {
        assert_eq!(glyph(' '), font::BLANK);
        assert_eq!(glyph('█'), font::SOLID);
        assert_eq!(glyph('▀'), font::UPPER);
        for (c, g) in font::TEXT {
            assert_eq!(glyph(c), g);
        }
    }
}
