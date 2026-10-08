//! char -> glyph index, for anything that draws text or a character ramp.
//! Hand-written beside the generated `font` so genfont's output stays a table.

use crate::font;

/// The glyph for `c`: printable ASCII, or one of `font::TEXT`. Anything else
/// is a bug — `?` in a release build so a dump shows where, a panic in a
/// test so it never ships. `const` so a ramp is a table, not state.
#[inline]
pub const fn of(c: char) -> u16 {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_text_char_resolves() {
        assert_eq!(of(' '), font::BLANK);
        assert_eq!(of('█'), font::SOLID);
        assert_eq!(of('▀'), font::UPPER);
        for (c, g) in font::TEXT {
            assert_eq!(of(c), g);
        }
    }
}
