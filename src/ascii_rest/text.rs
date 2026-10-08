//! Characters for the ascii.rest text pieces, which upstream return as a
//! string and here write cells.

use crate::glyph;
use crate::grid::Cell;

/// `c` in the piece's one ink.
#[inline]
pub const fn cell(c: char) -> Cell {
    Cell::new(glyph::of(c), 0)
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
