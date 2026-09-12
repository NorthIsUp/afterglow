//! A character grid over the panel, and the one glyph blitter in the program.
//!
//! Every saver here draws cells, so the change test, the damage report and the
//! frame-0 full paint live once, in `flush`, instead of once per saver.

use crate::font;
use crate::surface::{Panel, Surface};

/// Unlit glyph pixels. A cell always paints its whole rectangle, which is what
/// makes skipping an unchanged cell sound.
const BG: u32 = 0x0000_0000;

/// A cell's entire appearance, packed so the change test is one u32 compare —
/// exactly the compare `heat == prev_heat` was, for a strictly richer cell.
/// Colour is a palette INDEX, not an XRGB value, so the compare is exact.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(transparent)]
pub struct Cell(u32);

impl Cell {
    #[inline]
    pub const fn new(glyph: u16, colour: u16) -> Self {
        Self((glyph as u32) | ((colour as u32) << 16))
    }

    #[inline]
    pub const fn glyph(self) -> usize {
        (self.0 & 0xFFFF) as usize
    }

    #[inline]
    pub const fn colour(self) -> usize {
        (self.0 >> 16) as usize
    }

    /// An unlit, transparent cell: what a cleared cell holds, and what a
    /// sprite's blanks bake down to.
    pub const CLEAR: Self = Self::new(font::BLANK, 0);

    /// The packed word, for the mirror's wire format — which is this exact
    /// u32, so nothing re-encodes what `Cell` already packs.
    #[inline]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

pub struct Grid {
    cols: usize,
    rows: usize,
    cell_w: usize,
    cell_h: usize,
    cur: Vec<Cell>,
    prev: Vec<Cell>,
    /// dest row -> source glyph row; dest column -> source bit mask. These
    /// replace two integer divides PER PIXEL — roughly four million a frame at
    /// 1080p — with two array reads.
    rowmap: Vec<u8>,
    mask: Vec<u8>,
    /// The buffer arrives zeroed and set_crtc has already scanned that black
    /// frame out, so frame 0 must paint every cell. An explicit flag rather
    /// than a sentinel in `prev`, so a saver may use any glyph or colour index.
    first: bool,
}

impl Grid {
    /// `cols = panel.w / cell_w`, `rows = panel.h / cell_h`, both at least 1.
    /// The right/bottom remainder is never written — see the damage contract.
    pub fn new(panel: &Panel, cell_w: usize, cell_h: usize) -> Self {
        let cell_w = cell_w.max(1);
        let cell_h = cell_h.max(1);
        let cols = (panel.w / cell_w).max(1);
        let rows = (panel.h / cell_h).max(1);
        Self {
            cols,
            rows,
            cell_w,
            cell_h,
            cur: vec![Cell::CLEAR; cols * rows],
            prev: vec![Cell::CLEAR; cols * rows],
            rowmap: (0..cell_h)
                .map(|py| (py * font::GLYPH_H / cell_h) as u8)
                .collect(),
            mask: (0..cell_w)
                .map(|px| 0x80u8 >> (px * font::GLYPH_W / cell_w))
                .collect(),
            first: true,
        }
    }

    #[inline]
    pub fn cols(&self) -> usize {
        self.cols
    }

    #[inline]
    pub fn rows(&self) -> usize {
        self.rows
    }

    #[inline]
    pub fn cell_w(&self) -> usize {
        self.cell_w
    }

    #[inline]
    pub fn cell_h(&self) -> usize {
        self.cell_h
    }

    /// The cells of the frame just flushed. `flush` ends by swapping `cur` into
    /// `prev`, so THIS is the drawn frame and `cur` is next frame's scratch —
    /// read it after `flush`, never before.
    #[inline]
    pub fn cells(&self) -> &[Cell] {
        &self.prev
    }

    /// Write this frame's cells. `f(cx, cy)` is called for EVERY cell, so an
    /// unwritten cell is not expressible — which is what lets `flush` swap
    /// `cur`/`prev` instead of copying.
    #[inline]
    pub fn fill<F: FnMut(usize, usize) -> Cell>(&mut self, mut f: F) {
        for cy in 0..self.rows {
            for cx in 0..self.cols {
                self.cur[cy * self.cols + cx] = f(cx, cy);
            }
        }
    }

    /// Blit every changed cell (every cell on frame 0), report exactly the rows
    /// blitted, then swap `cur` into `prev`. The only code in the program that
    /// writes pixels. Snapshotting is folded in: there is nothing to misorder.
    pub fn flush(&mut self, s: &mut Surface<'_>, pal: &[u32]) {
        for cy in 0..self.rows {
            for cx in 0..self.cols {
                let i = cy * self.cols + cx;
                let c = self.cur[i];
                if !self.first && c == self.prev[i] {
                    continue;
                }
                let bits = &font::GLYPHS[c.glyph()];
                let fg = pal[c.colour()];
                for (row, &sr) in s
                    .cell_rows(cx * self.cell_w, cy * self.cell_h, self.cell_w, self.cell_h)
                    .zip(self.rowmap.iter())
                {
                    // & 15 is free and lets the bounds check fold away.
                    let line = bits[(sr & (font::GLYPH_H as u8 - 1)) as usize];
                    for (out, &m) in row.iter_mut().zip(self.mask.iter()) {
                        *out = if line & m != 0 { fg } else { BG };
                    }
                }
            }
        }
        self.first = false;
        std::mem::swap(&mut self.cur, &mut self.prev);
    }
}

/// RGB -> XRGB8888 (`0x00RRGGBB`) at compile time, so no saver bakes a palette
/// at runtime.
pub const fn bake<const N: usize>(rgb: &[[u8; 3]; N]) -> [u32; N] {
    let mut out = [0u32; N];
    let mut i = 0;
    while i < N {
        out[i] = ((rgb[i][0] as u32) << 16) | ((rgb[i][1] as u32) << 8) | rgb[i][2] as u32;
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_packs_both_fields() {
        let c = Cell::new(1234, 4321);
        assert_eq!((c.glyph(), c.colour()), (1234, 4321));
        assert_ne!(c, Cell::new(1234, 4322));
    }

    /// The LUTs must reproduce the arithmetic the pre-refactor blit did inline,
    /// scaled from an 8x8 source to an 8x16 one: flooring an already-floored
    /// index by two is the same as flooring the original by twice the cell.
    #[test]
    fn lookup_tables_match_the_divides_they_replace() {
        let panel = Panel::new(1920, 1080, 1920);
        for cell in [8usize, 16, 24, 32, 64] {
            let g = Grid::new(&panel, cell, cell);
            for py in 0..cell {
                assert_eq!(g.rowmap[py] as usize, py * font::GLYPH_H / cell);
                assert_eq!(g.rowmap[py] as usize / 2, py * 8 / cell, "cell={cell}");
            }
            for px in 0..cell {
                assert_eq!(g.mask[px], 0x80u8 >> (px * 8 / cell));
            }
        }
    }

    #[test]
    fn frame_zero_paints_every_cell_then_only_changes() {
        let panel = Panel::new(4, 4, 4);
        let mut g = Grid::new(&panel, 2, 2);
        let mut buf = vec![0u32; panel.buf_len()];

        g.fill(|_, _| Cell::new(font::BLANK, 0));
        let mut s = Surface::new(&mut buf, &panel);
        g.flush(&mut s, &[0x11]);
        // Every cell is blank, but frame 0 still has to cover the panel.
        assert_eq!(s.finish().rows(), 4);

        let mut s = Surface::new(&mut buf, &panel);
        g.fill(|_, _| Cell::new(font::BLANK, 0));
        g.flush(&mut s, &[0x11]);
        assert!(
            s.finish().is_empty(),
            "an unchanged frame must dirty nothing"
        );

        let mut s = Surface::new(&mut buf, &panel);
        g.fill(|cx, cy| Cell::new(font::SOLID, (cx == 1 && cy == 1) as u16));
        g.flush(&mut s, &[0x11, 0x22]);
        assert_eq!(s.finish().rows(), 4);
        assert_eq!(buf[3 * 4 + 3], 0x22);
        assert_eq!(buf[0], 0x11);
    }
}
