//! A low-resolution picture (an emulator's or a game engine's) shown on the
//! panel at the glass's shape: scaled straight into the panel's pixels, only
//! the source rows that changed and each over just the columns that changed,
//! with a grid of SOLID cells beside it for the mirror and the terminal.
//!
//! A picture whose every pixel can move every frame makes a per-cell diff and
//! glyph blit the whole cost of a saver; this path skips both.

use crate::font;
use crate::grid::{Cell, Grid};
use crate::surface::{Panel, Surface};

const OFF: u16 = u16::MAX;

/// The source width for a panel, where a source `px_per_ratio` pixels wide
/// per unit of height-relative width fills the glass; and the share of the
/// panel's width and height (per mille) the picture fills without stretching.
pub fn layout(
    panel: &Panel,
    aspect: usize,
    px_per_ratio: f32,
    min_w: usize,
    max_w: usize,
) -> (usize, usize, usize) {
    let glass = panel.w as f32 * aspect as f32 / (100.0 * panel.h as f32);
    let w = ((px_per_ratio * glass).round() as usize).clamp(min_w, max_w);
    let own = w as f32 / px_per_ratio;
    if own >= glass {
        (w, 1000, (glass / own * 1000.0) as usize)
    } else {
        (w, (own / glass * 1000.0) as usize, 1000)
    }
}

/// The mapping from a source `w` by `h` picture to the panel and the grid,
/// and the copy of what the panel shows that lets an unchanged row cost
/// nothing.
pub struct Scaled<T> {
    h: usize,
    w: usize,
    /// The cells the picture covers, `x0..x1` by `y0..y1`; the rest is margin.
    rect: (usize, usize, usize, usize),
    /// The panel pixels the picture covers, the same way, and the panel.
    px_rect: (usize, usize, usize, usize),
    panel: (usize, usize),
    /// Per panel column, the source column; per source column and row, the
    /// first panel column and row it covers, one past the end appended.
    x_src: Vec<u16>,
    x_first: Vec<u16>,
    y_first: Vec<u16>,
    /// Per grid column and row, the source pixel; `OFF` outside the picture.
    col_src: Vec<u16>,
    row_src: Vec<u16>,
    drawn: Vec<T>,
    /// One panel row, gathered once per source row and copied down the rows
    /// it covers.
    line: Vec<u32>,
    /// The panel buffer arrives zeroed: the first frame paints everything.
    first: bool,
}

impl<T: Copy + Default + PartialEq> Scaled<T> {
    /// The grid for `panel` and the mapping of a source `w` (of `layout`)
    /// by `h`, at most `max_w` wide, onto both.
    pub fn new(
        panel: &Panel,
        aspect: usize,
        (w, wide, tall): (usize, usize, usize),
        h: usize,
        max_w: usize,
    ) -> (Grid, Self) {
        // A cell per source pixel across at most: a smaller cell would only
        // repeat pixels.
        let cell_w = (panel.w / w).max(1);
        let cell_h = (panel.h * 100 / aspect / h).max(1);
        let grid = Grid::new(panel, cell_w, cell_h);
        let (cols, rows) = (grid.cols(), grid.rows());
        let (fw, fh) = (cols * wide / 1000, rows * tall / 1000);
        let (x0, y0) = ((cols - fw) / 2, (rows - fh) / 2);
        let (pw, ph) = (panel.w * wide / 1000, panel.h * tall / 1000);
        let (px0, py0) = ((panel.w - pw) / 2, (panel.h - ph) / 2);
        let row_src = (0..rows)
            .map(|r| {
                if (y0..y0 + fh).contains(&r) {
                    ((r - y0) * h / fh.max(1)) as u16
                } else {
                    OFF
                }
            })
            .collect();
        let mut s = Self {
            h,
            w: 0,
            rect: (x0, x0 + fw, y0, y0 + fh),
            px_rect: (px0, px0 + pw, py0, py0 + ph),
            panel: (panel.w, panel.h),
            x_src: vec![0; pw],
            x_first: Vec::with_capacity(max_w + 1),
            y_first: (0..=h).map(|sy| (sy * ph).div_ceil(h) as u16).collect(),
            col_src: vec![OFF; cols],
            row_src,
            drawn: vec![T::default(); max_w * h],
            line: vec![0; pw],
            first: true,
        };
        s.set_width(w);
        (grid, s)
    }

    pub fn width(&self) -> usize {
        self.w
    }

    /// Point the columns, cells and panel pixels at a source `w` wide, in
    /// place: a width change after a knob change never allocates.
    pub fn set_width(&mut self, w: usize) {
        let (x0, x1, ..) = self.rect;
        let span = (x1 - x0).max(1);
        for (c, src) in self.col_src.iter_mut().enumerate() {
            *src = if (x0..x1).contains(&c) {
                ((c - x0) * w / span) as u16
            } else {
                OFF
            };
        }
        let pw = self.x_src.len();
        for (x, src) in self.x_src.iter_mut().enumerate() {
            *src = (x * w / pw) as u16;
        }
        // First x with x * w / pw >= sx.
        self.x_first.clear();
        self.x_first
            .extend((0..=w).map(|sx| (sx * pw).div_ceil(w.max(1)) as u16));
        self.w = w;
    }

    /// Show `pix`, `width()` by `h`: the rows that differ from what the panel
    /// shows (all of them when `full`, or on the first frame) through
    /// `panel_colour`, and every cell through `cell_colour`.
    pub fn draw(
        &mut self,
        s: &mut Surface<'_>,
        grid: &mut Grid,
        pix: &[T],
        full: bool,
        panel_colour: impl Fn(T) -> u32,
        cell_colour: impl Fn(T) -> u16,
    ) {
        self.blit(s, pix, full || self.first, panel_colour);
        if self.first {
            self.margins(s);
            self.first = false;
        }
        let (x0, x1, ..) = self.rect;
        let w = self.w;
        let (cols, row_src) = (&self.col_src[x0..x1], &self.row_src);
        grid.fill_rows(|cy, row| {
            let sy = row_src[cy];
            if sy == OFF {
                return row.fill(Cell::CLEAR);
            }
            let src = &pix[sy as usize * w..][..w];
            row[..x0].fill(Cell::CLEAR);
            row[x1..].fill(Cell::CLEAR);
            for (c, &sx) in row[x0..x1].iter_mut().zip(cols) {
                *c = Cell::new(font::SOLID, cell_colour(src[sx as usize]));
            }
        });
        grid.settle();
    }

    fn blit(&mut self, s: &mut Surface<'_>, pix: &[T], full: bool, colour: impl Fn(T) -> u32) {
        let w = self.w;
        let (px0, _, py0, _) = self.px_rect;
        for sy in 0..self.h {
            let (ya, yb) = (self.y_first[sy] as usize, self.y_first[sy + 1] as usize);
            let row = &pix[sy * w..][..w];
            let old = &mut self.drawn[sy * w..][..w];
            let (lo, hi) = if full {
                (0, w)
            } else if row == old || ya == yb {
                continue;
            } else {
                let diff = |(a, b): (&T, &T)| a != b;
                let z = || row.iter().zip(old.iter());
                (
                    z().position(diff).unwrap_or(0),
                    w - z().rev().position(diff).unwrap_or(0),
                )
            };
            old[lo..hi].copy_from_slice(&row[lo..hi]);
            let (xa, xb) = (self.x_first[lo] as usize, self.x_first[hi] as usize);
            let line = &mut self.line[xa..xb];
            for (out, &sx) in line.iter_mut().zip(&self.x_src[xa..xb]) {
                *out = colour(row[sx as usize]);
            }
            for out in s.cell_rows(px0 + xa, py0 + ya, xb - xa, yb - ya) {
                out.copy_from_slice(line);
            }
        }
    }

    /// The panel outside the picture, black.
    fn margins(&self, s: &mut Surface<'_>) {
        let (x0, x1, y0, y1) = self.px_rect;
        let (pw, ph) = self.panel;
        for (x, y, w, h) in [
            (0, 0, pw, y0),
            (0, y1, pw, ph - y1),
            (0, y0, x0, y1 - y0),
            (x1, y0, pw - x1, y1 - y0),
        ] {
            for row in s.cell_rows(x, y, w, h) {
                row.fill(0);
            }
        }
    }
}

#[cfg(test)]
impl<T> Scaled<T> {
    pub fn px_rect(&self) -> (usize, usize, usize, usize) {
        self.px_rect
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::with_test_aspect;

    /// Pine's 768 Doom columns spread over 960 cells: every source column
    /// and row lands on a cell, in order, none skipped.
    #[test]
    fn every_source_pixel_lands_on_a_cell() {
        let p = Panel::new(1920, 1080, 1920);
        let (grid, s) = with_test_aspect(180, || {
            Scaled::<u8>::new(&p, 180, layout(&p, 180, 240.0, 320, 768), 200, 768)
        });
        assert_eq!((grid.cols(), s.width()), (960, 768));
        let (x0, x1, y0, y1) = s.rect;
        let cols = &s.col_src[x0..x1];
        let used: std::collections::BTreeSet<_> = cols.iter().copied().collect();
        assert_eq!(used.len(), 768);
        assert!(cols.windows(2).all(|p| p[0] <= p[1]));
        let rows: std::collections::BTreeSet<_> = s.row_src[y0..y1].iter().copied().collect();
        assert_eq!(rows.len(), 200);
    }
}
