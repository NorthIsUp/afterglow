//! dvd — the bouncing DVD logo, and the corner hit it exists for.
//!
//! # Why the motion is an exact integer lattice
//!
//! The whole point of this saver is the corner hit, so a corner has to be
//! REACHABLE rather than approached-and-missed forever. Both axes therefore
//! advance by a whole number of pixels per frame and the travel range is
//! trimmed to a multiple of that step, which puts the logo on a finite lattice:
//! the x wall is touched exactly every `span_x / step` frames and the y wall
//! exactly every `span_y / step`, so a corner is a simultaneous solution of two
//! congruences. It exists iff the two phases agree modulo their gcd, which is
//! why `new` starts both axes at the SAME lattice index — that makes the pair
//! solvable by construction, and corners then recur every lcm frames. A
//! floating-point step, or an untrimmed range the step overshoots, turns a
//! guaranteed rare event into one that may never happen at all.
//!
//! Nothing else is fudged toward the corner: the reflection is exact, the
//! phases are whatever the geometry gives, and at 1920x1080 / 15 fps the wait
//! is a few minutes. When it lands the logo stops dead in the corner and
//! strobes through the palette for `DVD_CORNER_MS` — the only time it is ever
//! still, which is what makes it unmistakable.
//!
//! # Per-frame cost
//!
//! Sparse (`flush_sparse`). The scene is one 20x6-cell wordmark on black, 44 of
//! those cells lit, and a frame rewrites only the cells it vacated plus the
//! cells it entered — usually far fewer than both stamps, because a step is
//! under one cell wide and an overlapping cell that keeps its colour is
//! skipped. Nothing here is O(grid) after frame 0.

use crate::font;
use crate::grid::{bake, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, next_rand};

/// Index 0 is the background. The rest is the logo's colour wheel: saturated,
/// roughly equal in brightness, and far enough apart in hue that a bounce is
/// legible from across the room.
#[rustfmt::skip]
const PAL_RGB: [[u8; 3]; 9] = [
    [0x00, 0x00, 0x00],
    [0xFF, 0x3B, 0x30],
    [0xFF, 0x95, 0x00],
    [0xFF, 0xD6, 0x0A],
    [0x30, 0xD1, 0x58],
    [0x40, 0xC8, 0xE0],
    [0x0A, 0x84, 0xFF],
    [0xBF, 0x5A, 0xF2],
    [0xF2, 0xF2, 0xF7],
];
const PAL: [u32; 9] = bake(&PAL_RGB);

/// The wordmark, one char per CELL: `#` is a solid cell of the current colour,
/// anything else is background. Block letters rather than the real disc-shaped
/// mark, because a cell is one glyph and one colour — an oval would have to be
/// drawn in braille at a size where the letters inside it are a single cell.
#[rustfmt::skip]
const ART: [&str; 6] = [
    "####.. #....# ####..",
    "#...#. #....# #...#.",
    "#....# #....# #....#",
    "#....# #....# #....#",
    "#...#. .#..#. #...#.",
    "####.. ..##.. ####..",
];

pub struct Dvd {
    grid: Grid,
    cols: usize,
    rows: usize,
    cell_w: usize,
    cell_h: usize,
    /// `ART` flattened, `aw * ah`, so the "is this cell part of the logo"
    /// question the erase pass asks is one index rather than a search.
    lit: Vec<bool>,
    aw: usize,
    ah: usize,
    dirty: Vec<u32>,
    /// UNFOLDED position in px, modulo `2 * span`: the triangle wave before it
    /// is folded back onto the panel. Reflection is then arithmetic, with no
    /// direction flag that can fall out of step with the position.
    ux: usize,
    uy: usize,
    span_x: usize,
    span_y: usize,
    step: usize,
    cx: usize,
    cy: usize,
    col: u16,
    hold: u32,
    hold_frames: u32,
    corners: u64,
    rng: u32,
}

/// Triangle wave: the unfolded coordinate reflected into `0..=span`.
#[inline]
fn fold(u: usize, span: usize) -> usize {
    if span == 0 {
        return 0;
    }
    let m = u % (2 * span);
    if m <= span {
        m
    } else {
        2 * span - m
    }
}

/// One step along the unfolded axis; true when this frame LANDED on a wall.
/// Exact because `span` is a multiple of `step` — the logo touches the wall, it
/// never straddles it.
#[inline]
fn advance(u: &mut usize, span: usize, step: usize) -> bool {
    if span == 0 {
        return false;
    }
    *u = (*u + step) % (2 * span);
    (*u).is_multiple_of(span)
}

impl Dvd {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        // Bigger cells than the other savers: the logo is cell art, so the cell
        // IS the pixel size of the wordmark. 24x24 puts it at 480x144 on a
        // 1080p panel, a quarter of the width, and 24 divides 1080 — the
        // `h % cell_h` strip belongs to no cell, so a cell height that leaves
        // one holds the logo that far off the bottom wall at the corner hit,
        // which is the one place this saver cannot afford to look approximate.
        let cell_w = env_num(&["DVD_CELL_W"], 24, 4, 64) as usize;
        let cell_h = env_num(&["DVD_CELL_H"], 24, 4, 128) as usize;
        let grid = Grid::new(panel, cell_w, cell_h);
        // Back from the grid, not the env: `SAVER_PIXEL_ASPECT` stretches it,
        // and the bounce arithmetic below is in panel pixels.
        let cell_h = grid.cell_h();
        let (cols, rows) = (grid.cols(), grid.rows());

        let ah = ART.len();
        let aw = ART.iter().map(|r| r.len()).max().unwrap_or(1);
        let mut lit = vec![false; aw * ah];
        for (y, row) in ART.iter().enumerate() {
            for (x, ch) in row.bytes().enumerate() {
                lit[y * aw + x] = ch == b'#';
            }
        }

        // px per SECOND, so a slower panel drifts at the same speed. Rounded to
        // a whole number of px per FRAME on purpose: see the module doc, the
        // integer lattice is what makes the corner reachable.
        let speed = env_num(&["DVD_SPEED"], 180, 8, 2000) as usize;
        let fps = fps.max(1) as usize;
        let step = ((speed + fps / 2) / fps).max(1);

        // Travel range, trimmed to a whole number of steps. The trim costs at
        // most `step - 1` px of one wall (zero at the shipped geometry) and buys
        // an exact wall touch.
        let free_x = (cols * cell_w).saturating_sub(aw * cell_w);
        let free_y = (rows * cell_h).saturating_sub(ah * cell_h);
        let span_x = free_x / step * step;
        let span_y = free_y / step * step;

        // The SAME lattice index on both axes, which is what makes the two
        // wall-hit congruences simultaneously solvable. A third of the way
        // along, so the logo does not start in a corner it is meant to earn.
        let k = (span_x / step / 3).max(1);
        let ux = if span_x == 0 {
            0
        } else {
            k * step % (2 * span_x)
        };
        let uy = if span_y == 0 {
            0
        } else {
            k * step % (2 * span_y)
        };

        let hold_ms = env_num(&["DVD_CORNER_MS"], 1500, 0, 10_000) as u32;
        let n_lit = lit.iter().filter(|&&b| b).count();

        Self {
            cx: fold(ux, span_x) / cell_w,
            cy: fold(uy, span_y) / cell_h,
            grid,
            cols,
            rows,
            cell_w,
            cell_h,
            lit,
            aw,
            ah,
            // A frame vacates at most one stamp's worth of cells and enters at
            // most one more; never grown in `render`.
            dirty: Vec::with_capacity(2 * n_lit + 8),
            ux,
            uy,
            span_x,
            span_y,
            step,
            col: 1,
            hold: 0,
            hold_frames: hold_ms * fps as u32 / 1000,
            corners: 0,
            rng: 0x05EE_DD7D,
        }
    }

    /// A different colour than the one showing — the same colour twice reads as
    /// a bounce that did not register.
    #[inline]
    fn recolour(&mut self) {
        let n = PAL.len() as u32 - 1;
        let mut c = 1 + (next_rand(&mut self.rng) % n) as u16;
        if c == self.col {
            c = 1 + c % n as u16;
        }
        self.col = c;
    }

    #[inline]
    fn put(&mut self, cx: usize, cy: usize, c: Cell) {
        if cx >= self.cols || cy >= self.rows {
            return;
        }
        let i = cy * self.cols + cx;
        if c == self.grid.cell(i) {
            return;
        }
        self.grid.set(i, c);
        self.dirty.push(i as u32);
    }

    /// Move the stamp from `(ocx, ocy)` to the current cell. Erase first, but
    /// only what the new stamp does not cover — the background is uniformly
    /// blank, so putting back exactly what was covered is putting back nothing,
    /// and that is why this saver needs no saved-cells buffer.
    fn restamp(&mut self, ocx: usize, ocy: usize) {
        for ay in 0..self.ah {
            for ax in 0..self.aw {
                if !self.lit[ay * self.aw + ax] {
                    continue;
                }
                let (x, y) = (ocx + ax, ocy + ay);
                let (dx, dy) = (x.wrapping_sub(self.cx), y.wrapping_sub(self.cy));
                if !(dx < self.aw && dy < self.ah && self.lit[dy * self.aw + dx]) {
                    self.put(x, y, Cell::CLEAR);
                }
            }
        }
        let c = Cell::new(font::SOLID, self.col);
        for ay in 0..self.ah {
            for ax in 0..self.aw {
                if self.lit[ay * self.aw + ax] {
                    self.put(self.cx + ax, self.cy + ay, c);
                }
            }
        }
    }
}

impl Saver for Dvd {
    fn render(&mut self, s: &mut Surface<'_>) {
        self.dirty.clear();
        let (ocx, ocy) = (self.cx, self.cy);

        if self.hold > 0 {
            // Dead still in the corner, strobing: the only frames where the
            // logo does not move.
            self.hold -= 1;
            self.recolour();
        } else {
            let bx = advance(&mut self.ux, self.span_x, self.step);
            let by = advance(&mut self.uy, self.span_y, self.step);
            if bx || by {
                self.recolour();
            }
            if bx && by {
                self.corners += 1;
                self.hold = self.hold_frames;
            }
            self.cx = fold(self.ux, self.span_x) / self.cell_w;
            self.cy = fold(self.uy, self.span_y) / self.cell_h;
        }

        self.restamp(ocx, ocy);
        self.dirty.sort_unstable();
        self.grid.flush_sparse(s, &PAL, &self.dirty);
    }

    fn name(&self) -> &'static str {
        "dvd"
    }

    fn grid(&self) -> &Grid {
        &self.grid
    }

    fn palette(&self) -> &[u32] {
        &PAL
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dump;
    use crate::saver;

    /// 1070, not the 1080 the panel actually is: 1080 divides by the default
    /// 24px cell exactly and would prove nothing about the strip below the last
    /// cell row. 1070 leaves 14 scanlines belonging to no cell, which is the
    /// strip frame 0 has to paint.
    fn panel() -> Panel {
        Panel::new(1920, 1070, 1920)
    }

    fn dvd() -> Dvd {
        Dvd::new(&panel(), 15)
    }

    /// Cells the scene is actually showing, as (cx, cy).
    fn drawn(d: &Dvd) -> Vec<(usize, usize)> {
        let mut v = Vec::new();
        for cy in 0..d.rows {
            for cx in 0..d.cols {
                if d.grid.cells()[cy * d.cols + cx] != Cell::CLEAR {
                    v.push((cy, cx));
                }
            }
        }
        v
    }

    /// Where the logo claims to be, as (cx, cy).
    fn stamp(d: &Dvd) -> Vec<(usize, usize)> {
        let mut v = Vec::new();
        for ay in 0..d.ah {
            for ax in 0..d.aw {
                if d.lit[ay * d.aw + ax] {
                    v.push((d.cy + ay, d.cx + ax));
                }
            }
        }
        v.sort_unstable();
        v
    }

    /// T1 + T2: the whole panel on frame 0, and after that every scanline that
    /// changed is reported AND every cell written is in `dirty`. The second is
    /// the one that matters — an unreported write never reaches the panel, so
    /// the framebuffer diff above it is structurally blind to that bug.
    #[test]
    fn damage_covers_every_changed_scanline() {
        let p = panel();
        let mut c = dvd();
        let mut buf = vec![0u32; p.buf_len()];
        let mut prev = buf.clone();

        let d = saver::frame(&mut c, &mut buf, &p);
        assert_eq!(d.rows(), p.h, "frame 0 must paint the whole panel");
        assert!(
            buf.iter().filter(|&&px| px != 0).count() > 10_000,
            "frame 0 painted nothing"
        );

        let cells = c.cols * c.rows;
        let mut rows = Vec::new();
        let mut moved = 0;
        for n in 1..3000 {
            prev.copy_from_slice(&buf);
            let d = saver::frame(&mut c, &mut buf, &p);
            let mut changed = 0;
            for y in 0..p.h {
                if prev[y * p.w..][..p.w] == buf[y * p.w..][..p.w] {
                    continue;
                }
                changed += 1;
                assert!(
                    dump::row_reported(&prev[y * p.w..][..p.w], &buf[y * p.w..][..p.w], y, &d),
                    "frame {n}: scanline {y} changed outside every reported rect"
                );
            }
            moved += usize::from(changed > 0);
            rows.push(d.rows());
            for i in 0..cells {
                assert_eq!(
                    c.grid.cell(i),
                    c.grid.cells()[i],
                    "frame {n}: cell {i} was written but not reported"
                );
            }
        }
        // Not every frame: a step is 12px against a 24x32 cell, so the stamp
        // lands on a new cell about two frames in three and an in-between frame
        // is legitimately still. A STALLED logo is the bug, and only the window
        // tells the two apart.
        assert!(moved > 1800, "the logo stalled ({moved}/2999 frames moved)");

        // Sparse, and it has to stay sparse. Measured: median 120 rows/frame,
        // max 168, against 1070 on the panel: the logo is five cell rows, so a
        // frame reporting a large fraction of the panel is a regression to a
        // full repaint. Median, so the rare corner hold cannot hide one.
        rows.sort_unstable();
        let median = rows[rows.len() / 2];
        let logo_rows = c.ah * c.cell_h;
        assert!(
            median <= 2 * logo_rows,
            "median {median} rows/frame is not sparse for a {logo_rows}-row logo"
        );
        // Non-vacuous: the bound is not passing because nothing is ever drawn.
        // The busiest frame repaints a whole logo band and then some.
        assert!(
            *rows.last().unwrap() >= logo_rows,
            "the busiest frame reported {} rows, less than the logo's {logo_rows}",
            rows.last().unwrap()
        );
        eprintln!(
            "dvd damage rows/frame: median {median}, max {}",
            rows.last().unwrap()
        );
    }

    /// T3. Capacity, not length: a Vec that never grows past its reserve never
    /// reallocates and `clear` keeps capacity, so unchanged capacity IS "the
    /// render path did not allocate". `lit` is the only other buffer and it is
    /// read-only after `new`.
    #[test]
    fn render_never_allocates() {
        let p = panel();
        let mut c = dvd();
        let mut buf = vec![0u32; p.buf_len()];
        let reserved = c.dirty.capacity();
        let lit_len = c.lit.len();
        assert!(reserved > 0, "nothing was reserved for the frame loop");

        let mut worst = 0;
        for _ in 0..50_000 {
            saver::frame(&mut c, &mut buf, &p);
            worst = worst.max(c.dirty.len());
            assert_eq!(
                c.dirty.capacity(),
                reserved,
                "`dirty` grew past its reserve: the render path allocated"
            );
            assert_eq!(c.lit.len(), lit_len, "the art buffer was rewritten");
        }
        // Non-vacuous: a reserve nothing ever fills proves nothing. A diagonal
        // step vacates and enters more cells than one stamp holds.
        let n_lit = c.lit.iter().filter(|&&b| b).count();
        assert!(
            worst > n_lit,
            "`dirty` never held more than one stamp ({worst} <= {n_lit})"
        );
        assert!(
            worst <= reserved,
            "{worst} entries in a reserve of {reserved}"
        );
    }

    /// T4a — the saver's entire reason to exist. A corner hit must be genuinely
    /// reachable, and rare. Break the shared lattice index in `new` (give one
    /// axis a different phase) and the two wall congruences stop being
    /// simultaneously solvable: the logo bounces forever and never corners.
    #[test]
    fn the_corner_is_reachable_and_rare() {
        let p = panel();
        let mut c = dvd();
        let mut buf = vec![0u32; p.buf_len()];
        const FRAMES: usize = 60_000;

        let mut seen = Vec::new();
        for n in 0..FRAMES {
            let before = c.corners;
            saver::frame(&mut c, &mut buf, &p);
            if c.corners > before {
                // Both walls, exactly — not merely near them.
                let (x, y) = (fold(c.ux, c.span_x), fold(c.uy, c.span_y));
                assert!(
                    (x == 0 || x == c.span_x) && (y == 0 || y == c.span_y),
                    "frame {n}: counted a corner at ({x}, {y}), walls at ({}, {})",
                    c.span_x,
                    c.span_y
                );
                seen.push(n);
            }
        }
        assert!(!seen.is_empty(), "no corner hit in {FRAMES} frames");
        // Rare: minutes apart at 15 fps, not a metronome.
        assert!(
            seen.len() < FRAMES / 1000,
            "{} corner hits in {FRAMES} frames is not rare",
            seen.len()
        );
    }

    /// T4b — no trail. The exact invariant, cell for cell: the scene shows the
    /// logo's cells, no cell more and no cell less. A ratio would not do it, a
    /// trail saturates once the logo starts sweeping up its own leftovers.
    #[test]
    fn the_logo_leaves_no_trail() {
        let p = panel();
        let mut c = dvd();
        let mut buf = vec![0u32; p.buf_len()];
        for n in 0..5_000 {
            saver::frame(&mut c, &mut buf, &p);
            assert_eq!(drawn(&c), stamp(&c), "frame {n}: the scene is not the logo");
        }
    }

    /// Frame 0 already shows the wordmark, and it IS the wordmark: a logo that
    /// renders as a solid block passes every damage test here.
    #[test]
    fn the_scene_is_the_wordmark() {
        let p = panel();
        let mut c = dvd();
        let mut buf = vec![0u32; p.buf_len()];
        saver::frame(&mut c, &mut buf, &p);
        let n_lit = c.lit.iter().filter(|&&b| b).count();
        assert_eq!(drawn(&c).len(), n_lit);
        assert!(n_lit < c.aw * c.ah, "the art is a solid rectangle");
        assert_eq!(drawn(&c), stamp(&c));
    }
}
