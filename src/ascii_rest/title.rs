//! `ASCII_REST_TITLE=1`: the piece's name in the panel's bottom-left corner,
//! on a band of ground.
//!
//! Stamped onto the grid after the picture is mapped, so `Piece::frame` — and
//! with it the golden test — never sees it, and built once at construction, so
//! a frame with it off costs one `Option` check.

use crate::font;
use crate::grid::{Cell, Grid};

/// The caption in two sizes, picked per grid by what fits: the tour's
/// close-ups make the cells several times wider, and a caption counted in
/// cells grows with them.
pub struct Title {
    /// Each glyph pixel a dot, for a scene's few-pixel cells; None for a text
    /// piece, whose cell already holds a glyph.
    dots: Option<Block>,
    /// One glyph per cell.
    glyphs: Block,
}

/// Caption cells inside a one-cell band of ground.
struct Block {
    w: usize,
    h: usize,
    cells: Vec<Cell>,
}

impl Title {
    /// `night-coast` reads `night coast`. `cell` is the piece's cell
    /// shape, 1 for a scene.
    pub fn new(name: &str, cell: usize, palette: &[u32]) -> Self {
        let text = name.replace('-', " ");
        let ink = brightest(palette);
        let mut glyphs = Block::blank(text.len(), 1);
        for (n, b) in text.bytes().enumerate() {
            glyphs.cells[glyphs.w + 1 + n] = Cell::new(glyph(b), ink);
        }
        Self {
            dots: (cell == 1).then(|| Block::dots(&text, ink)),
            glyphs,
        }
    }

    /// The block `stamp` uses on a `cols x rows` grid: dots while they take
    /// at most half its width, which they do untoured, then glyphs —
    /// a close-up's wider cells would otherwise blow the dots up across half
    /// the panel — then nothing.
    fn pick(&self, cols: usize, rows: usize) -> Option<&Block> {
        let dots = self
            .dots
            .as_ref()
            .filter(|b| b.w * 2 <= cols && b.h <= rows);
        dots.or(Some(&self.glyphs).filter(|b| b.w <= cols && b.h <= rows))
    }

    /// The `(w, h)` in cells `stamp` covers on a `cols x rows` grid.
    #[cfg(test)]
    pub fn size(&self, cols: usize, rows: usize) -> Option<(usize, usize)> {
        self.pick(cols, rows).map(|b| (b.w, b.h))
    }

    /// Over the bottom-left corner of the cells wholly on the panel, so a
    /// shifted grid's caption is never cut by the glass's edge.
    #[inline]
    pub fn stamp(&self, grid: &mut Grid) {
        let (xs, ys) = grid.inside();
        let Some(b) = self.pick(xs.len(), ys.len()) else {
            return;
        };
        let (cols, top) = (grid.cols(), ys.end - b.h);
        for y in 0..b.h {
            let row = &b.cells[y * b.w..(y + 1) * b.w];
            for (x, &c) in row.iter().enumerate() {
                grid.set((top + y) * cols + xs.start + x, c);
            }
        }
    }
}

impl Block {
    fn blank(w: usize, h: usize) -> Self {
        let (w, h) = (w + 2, h + 2);
        Self {
            w,
            h,
            cells: vec![Cell::CLEAR; w * h],
        }
    }

    /// Each glyph pixel a dot — rows paired up, because a scene's cell is
    /// square on the glass and the font's pixels are not, and blank columns
    /// trimmed so the caption is spaced like type rather than a monospace
    /// grid.
    fn dots(text: &str, ink: u16) -> Self {
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
        let mut b = Self::blank(cols.len(), bot + 1 - top);
        for (x, c) in cols.iter().enumerate() {
            for y in top..=bot {
                if c >> y & 1 != 0 {
                    b.cells[(y - top + 1) * b.w + 1 + x] = Cell::new(font::HALFTONE[3], ink);
                }
            }
        }
        b
    }
}

fn glyph(b: u8) -> u16 {
    font::ASCII[usize::from(b.clamp(0x20, 0x7E) - 0x20)]
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
        let t = Title::new("night-coast", 1, &[0x000000, 0xFFFFFF, 0x808080]);
        let d = t.dots.as_ref().unwrap();
        // Narrower than 11 monospace glyphs, and no taller than paired rows.
        assert!(d.w < 11 * 8 && d.h <= 8 + 2, "{}x{}", d.w, d.h);
        let inked: Vec<_> = d.cells.iter().filter(|c| c.glyph() != 0).collect();
        assert_ne!(inked.len(), 0);
        assert!(inked.iter().all(|c| c.colour() == 1));
        // The border is ground.
        assert!(d.cells[..d.w].iter().all(|c| *c == Cell::CLEAR));
    }

    /// A close-up's grid is narrower, in wider cells; the glyphs take over
    /// before the dots fill the panel, rather than the caption ballooning or
    /// vanishing.
    #[test]
    fn a_close_up_falls_back_to_glyphs() {
        let t = Title::new("night-coast", 1, &[0xFFFFFF]);
        let (dw, _) = t.size(320, 100).unwrap();
        assert!(dw > 13 && dw * 2 <= 320, "{dw}");
        assert_eq!(t.size(dw * 2 - 1, 100), Some((13, 3)));
        assert_eq!(t.size(12, 100), None);
    }

    #[test]
    fn a_text_title_is_a_glyph_per_cell() {
        let t = Title::new("vinyl", 2, &[0xFFFFFF]);
        assert!(t.dots.is_none());
        assert_eq!(t.size(64, 30), Some((7, 3)));
        assert_eq!(
            t.glyphs.cells[t.glyphs.w + 1],
            Cell::new(font::ASCII[usize::from(b'v' - 0x20)], 0)
        );
    }
}
