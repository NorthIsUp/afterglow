//! `ASCII_REST_TITLE=1`: the piece's name in the panel's bottom-left corner,
//! on a band of ground.
//!
//! Stamped onto the grid after the picture is mapped, so `Piece::frame` — and
//! with it the golden test — never sees it, and built once at construction, so
//! a frame with it off costs one `Option` check.

use crate::font;
use crate::grid::{Cell, Grid};

/// The caption as a block of cells, ground padding included.
pub struct Title {
    w: usize,
    h: usize,
    cells: Vec<Cell>,
}

impl Title {
    /// `night-coast-wide` reads `night coast`. `cell` is the piece's cell
    /// shape: a text piece's cell holds a glyph legibly, so it gets one per
    /// cell. A scene's cell is a few pixels wide, so there each glyph pixel is
    /// a dot instead — rows paired up, because a scene's cell is square on the
    /// glass and the font's pixels are not, and blank columns trimmed so the
    /// caption is spaced like type rather than a monospace grid.
    pub fn new(name: &str, cell: usize, palette: &[u32]) -> Self {
        let text = name.strip_suffix("-wide").unwrap_or(name).replace('-', " ");
        let ink = brightest(palette);
        let glyph = |b: u8| font::ASCII[usize::from(b.clamp(0x20, 0x7E) - 0x20)];
        if cell != 1 {
            let mut t = Self::blank(text.len(), 1);
            for (n, b) in text.bytes().enumerate() {
                t.cells[t.w + 1 + n] = Cell::new(glyph(b), ink);
            }
            return t;
        }
        // Each glyph as its paired rows, and the text as one column list.
        let rows = font::GLYPH_H / 2;
        let mut cols: Vec<u8> = Vec::new();
        for b in text.bytes() {
            let bits = &font::GLYPHS[usize::from(glyph(b))];
            let paired: Vec<u8> = (0..rows).map(|y| bits[y * 2] | bits[y * 2 + 1]).collect();
            let col = |x: usize| (0..rows).fold(0u8, |c, y| c | (paired[y] >> (7 - x) & 1) << y);
            let inked: Vec<usize> = (0..font::GLYPH_W).filter(|&x| col(x) != 0).collect();
            match (inked.first(), inked.last()) {
                (Some(&l), Some(&r)) => cols.extend((l..=r).map(col)),
                _ => cols.extend([0; 3]),
            }
            cols.push(0);
        }
        cols.pop();
        // Rows no glyph here reaches: the font's ascent and descent.
        let used = cols.iter().fold(0u8, |a, c| a | c);
        let (top, bot) = (
            used.trailing_zeros() as usize,
            7 - used.leading_zeros() as usize,
        );
        let mut t = Self::blank(cols.len(), bot + 1 - top);
        for (x, c) in cols.iter().enumerate() {
            for y in top..=bot {
                if c >> y & 1 != 0 {
                    t.cells[(y - top + 1) * t.w + 1 + x] = Cell::new(font::HALFTONE[3], ink);
                }
            }
        }
        t
    }

    /// `w x h` cells of caption inside a one-cell band of ground.
    fn blank(w: usize, h: usize) -> Self {
        let (w, h) = (w + 2, h + 2);
        Self {
            w,
            h,
            cells: vec![Cell::CLEAR; w * h],
        }
    }

    #[cfg(test)]
    pub fn w(&self) -> usize {
        self.w
    }

    #[cfg(test)]
    pub fn h(&self) -> usize {
        self.h
    }

    /// Over `grid`'s bottom-left corner, if the grid is big enough to hold it.
    #[inline]
    pub fn stamp(&self, grid: &mut Grid) {
        let (cols, rows) = (grid.cols(), grid.rows());
        if cols < self.w || rows < self.h {
            return;
        }
        let top = rows - self.h;
        for y in 0..self.h {
            let row = &self.cells[y * self.w..(y + 1) * self.w];
            for (x, &c) in row.iter().enumerate() {
                grid.set((top + y) * cols + x, c);
            }
        }
    }
}

/// The palette entry a caption reads best in: the most luminous.
fn brightest(palette: &[u32]) -> u16 {
    let luma = |c: u32| 299 * (c >> 16 & 0xFF) + 587 * (c >> 8 & 0xFF) + 114 * (c & 0xFF);
    (0..palette.len())
        .max_by_key(|&i| luma(palette[i]))
        .unwrap_or(0) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scene_title_is_dots_in_the_brightest_ink() {
        let t = Title::new("night-coast-wide", 1, &[0x000000, 0xFFFFFF, 0x808080]);
        // Narrower than 11 monospace glyphs, and no taller than paired rows.
        assert!(t.w < 11 * 8 && t.h <= 8 + 2, "{}x{}", t.w, t.h);
        let inked: Vec<_> = t.cells.iter().filter(|c| c.glyph() != 0).collect();
        assert_ne!(inked.len(), 0);
        assert!(inked.iter().all(|c| c.colour() == 1));
        // The border is ground.
        assert!(t.cells[..t.w].iter().all(|c| *c == Cell::CLEAR));
    }

    #[test]
    fn a_text_title_is_a_glyph_per_cell() {
        let t = Title::new("vinyl", 2, &[0xFFFFFF]);
        assert_eq!((t.w, t.h), (7, 3));
        assert_eq!(
            t.cells[t.w + 1],
            Cell::new(font::ASCII[usize::from(b'v' - 0x20)], 0)
        );
    }
}
