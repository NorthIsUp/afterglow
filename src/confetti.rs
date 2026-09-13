//! Confetti — pieces flutter down, land, and pile up like sand.
//!
//! # The pile is the saver
//!
//! Everything interesting is in the height map. Each column of the grid holds a
//! height in GRAIN UNITS, `grains_per_cell` to a cell, and a landing piece adds
//! one cell's worth. Sub-cell units exist for one reason: the angle of repose
//! is 0.48 rise/run, and with a grain one cell tall the only legal integer
//! differences between neighbouring columns are 0 and "too steep" — a pile that
//! is either a flat smear or illegal. At 25 units to the cell the limit is 12
//! units, which the relaxation can actually land on.
//!
//! `topple` is the relaxation pass: any pair of neighbours steeper than the
//! limit moves half its excess downhill, a few sweeps a frame, so a fresh spike
//! visibly avalanches out over several frames instead of teleporting flat. It
//! moves grains, never creates or destroys them — `relax_conserves_grains`
//! pins that, because a transfer that loses a unit drains the pile in a way
//! that looks exactly like the drain below.
//!
//! The height map is only half of it: it has to REACH the grid, and every
//! write that is not `draw_pile` has to agree with it. `draw_pile` repaints a
//! column only when its height moved, so a piece erase that put CLEAR back
//! inside the heap left a black hole there until it did — 294 of them at
//! `CONFETTI_DEPOSIT=64`, one lasting 2889 frames. `beneath` is what an erase
//! puts back instead, read against `drawn` rather than the live `h` so it
//! always lands inside the range `draw_pile` is about to repaint.
//!
//! When the heap fills the panel the spawner stops and grains erode off random
//! column tops until it is half empty, then it fills again. A cap would freeze
//! the picture; a reset would blink.
//!
//! # Per-frame cost
//!
//! Sparse (Model B): a frame touches two cells per piece in flight plus the
//! columns whose height moved. What it does NOT buy is cheap damage — the
//! pieces are spread over the whole panel, so the scanline runs merge to nearly
//! full height whatever the cell count is. The win is blit work (hundreds of
//! cells, not 14k), not shadow-copy size.

use crate::font;
use crate::grid::{bake, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, next_rand};

/// Index 0 is the background and is never painted over. 1..=12 are the confetti
/// colours: this is the one saver that is meant to be genuinely multicoloured,
/// so they are spread round the wheel rather than being a ramp, and every one
/// is bright enough to read as paper against black.
#[rustfmt::skip]
const PAL_RGB: [[u8; 3]; 13] = [
    [0x00, 0x00, 0x00],
    [0xFF, 0x3B, 0x30], // red
    [0xFF, 0x7A, 0x1F], // orange
    [0xFF, 0xC1, 0x07], // amber
    [0xF4, 0xEB, 0x4A], // yellow
    [0x9E, 0xDC, 0x28], // lime
    [0x2F, 0xC6, 0x5A], // green
    [0x25, 0xD0, 0xC0], // teal
    [0x35, 0xA6, 0xFF], // sky
    [0x5B, 0x6B, 0xFF], // indigo
    [0xA8, 0x60, 0xFF], // violet
    [0xFF, 0x5F, 0xC4], // magenta
    [0xFF, 0xB8, 0xD0], // blush
];
const PAL: [u32; 13] = bake(&PAL_RGB);
const COLOURS: u32 = PAL.len() as u32 - 1;

/// A tumbling piece cycles these: flat edge-on, small, flat the other way. The
/// atlas already has both half blocks, so the flutter costs a table index.
const TUMBLE: [u16; 4] = [font::LOWER, font::BLOCK, font::UPPER, font::BLOCK];

/// sin scaled by 127, 32 steps. Per-piece sway at cell granularity, so a table
/// rather than `f32::sin` is taste; per PIXEL it would be the rule.
#[rustfmt::skip]
const SIN: [i32; 32] = [
    0, 25, 49, 71, 90, 106, 117, 125, 127, 125, 117, 106, 90, 71, 49, 25,
    0, -25, -49, -71, -90, -106, -117, -125, -127, -125, -117, -106, -90, -71, -49, -25,
];

/// Positions and velocities are 8.8 fixed-point pixels.
const FP: i32 = 8;

#[derive(Clone, Copy)]
struct Piece {
    x: i32,
    y: i32,
    vy: i32,
    phase: u16,
    sway: i32,
    col: u16,
    /// The cell this piece was last DRAWN into, so the erase puts back exactly
    /// what was covered instead of a recomputed guess. `u32::MAX` = off-grid.
    at: u32,
}

pub struct Confetti {
    grid: Grid,
    cols: usize,
    rows: usize,
    cell_w: usize,
    cell_h: usize,
    /// Column heights, in grain units.
    h: Vec<u32>,
    /// What `h` was when the column was last drawn. One O(cols) compare a frame
    /// replaces every piece of per-column dirty bookkeeping.
    drawn: Vec<u32>,
    /// Per pile cell, its colour index; `r * cols + c`, r from the bottom.
    /// Pre-seeded random so the heap is speckled, and overwritten at the top
    /// cell by whatever piece landed there.
    pile_col: Vec<u8>,
    pieces: Vec<Piece>,
    dirty: Vec<u32>,
    grains: u32,
    deposit: u32,
    max_drop: u32,
    relax: u32,
    total: u64,
    drain_hi: u64,
    drain_lo: u64,
    draining: bool,
    spawn_rate: u32,
    spawn_acc: u32,
    drain_rate: u32,
    drain_acc: u32,
    fall: i32,
    sway: i32,
    /// Half-width of the gust band, in 8.8 px, and its sweep through `SIN`.
    gust: i32,
    gust_phase: u32,
    gust_step: u32,
    max_pieces: usize,
    frame: u32,
    rng: u32,
}

/// Whole units owed this frame, carrying the fraction (8.8).
#[inline]
fn due(acc: &mut u32, rate: u32) -> u32 {
    *acc += rate;
    let n = *acc >> 8;
    *acc &= 0xFF;
    n
}

/// The only way a cell is written. Every `set` needs its index in `dirty` or
/// that region of the panel is frozen for the life of the pod.
#[inline]
fn put(grid: &mut Grid, dirty: &mut Vec<u32>, i: usize, c: Cell) {
    if grid.cell(i) == c {
        return;
    }
    grid.set(i, c);
    dirty.push(i as u32);
}

impl Confetti {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        let fps = fps.max(1);
        let cell = env_num(&["CONFETTI_CELL"], 12, 4, 64) as usize;
        let grid = Grid::new(panel, cell, cell);
        let (cols, rows) = (grid.cols(), grid.rows());
        let grains = env_num(&["CONFETTI_GRAINS"], 25, 4, 64) as u32;
        let slope = env_num(&["CONFETTI_SLOPE"], 48, 5, 200) as u32;
        // rise/run in pixels, converted to grain units per column. `.max(1)`
        // because a zero drop is a pile that cannot lean at all.
        let max_drop =
            (slope * grid.cell_w() as u32 * grains / (100 * grid.cell_h() as u32)).max(1);
        // Grain units a landed piece adds. Below `grains` (a whole cell) on
        // purpose: it is the one knob that trades airborne density against how
        // fast the heap swallows the panel.
        let deposit = env_num(&["CONFETTI_DEPOSIT"], 12, 1, 64) as u32;
        let max_pieces = env_num(&["CONFETTI_MAX"], 600, 1, 4000) as usize;
        let rate = env_num(&["CONFETTI_RATE"], 90, 1, 400) as u32;
        let fall = env_num(&["CONFETTI_FALL"], 260, 10, 2000) as i32;
        let sway = env_num(&["CONFETTI_SWAY"], 110, 0, 400) as i32;
        let full_pct = env_num(&["CONFETTI_FULL_PCT"], 35, 5, 95) as u64;
        let drain = env_num(&["CONFETTI_DRAIN"], 4000, 10, 100_000) as u32;
        let relax = env_num(&["CONFETTI_RELAX"], 8, 1, 64) as u32;
        // Uniform rain lays a uniform blanket — correct, and no pile at all.
        // Confetti falls in gusts instead: a band sweeping the panel, so a heap
        // builds under it and topples outward once the gust has moved on. 100
        // is rain again.
        let gust_pct = env_num(&["CONFETTI_GUST_PCT"], 40, 1, 100) as i32;
        let gust_secs = env_num(&["CONFETTI_GUST_SECS"], 37, 2, 600) as u32;

        let grid_cell_w = grid.cell_w();
        let capacity = (cols * rows) as u64 * grains as u64;
        let mut rng = 0x5EED_C0DE;
        let pile_col = (0..cols * rows)
            .map(|_| (1 + next_rand(&mut rng) % COLOURS) as u8)
            .collect();

        let mut c = Self {
            cols,
            rows,
            cell_w: grid.cell_w(),
            cell_h: grid.cell_h(),
            grid,
            h: vec![0; cols],
            drawn: vec![0; cols],
            pile_col,
            pieces: Vec::with_capacity(max_pieces),
            // Airtight bound: two cells per piece, plus at most one whole
            // column repaint per column. `render_never_allocates` is only
            // meaningful because this can never be exceeded.
            dirty: Vec::with_capacity(max_pieces * 2 + cols * (rows + 1)),
            grains,
            deposit,
            max_drop,
            relax,
            total: 0,
            drain_hi: capacity * full_pct / 100,
            drain_lo: capacity * full_pct / 200,
            draining: false,
            spawn_rate: (rate << FP) / fps,
            spawn_acc: 0,
            drain_rate: (drain << FP) / fps,
            drain_acc: 0,
            fall: (fall << FP) / fps as i32,
            sway: (sway << FP) / fps as i32,
            gust: (((cols * grid_cell_w) as i32) << FP) * gust_pct / 200,
            gust_phase: 0,
            // A whole sweep is 32 SIN steps, held in 8.8 so a slow sweep does
            // not round to a standstill.
            gust_step: ((32 << FP) as u32 / (gust_secs * fps)).max(1),
            max_pieces,
            frame: 0,
            rng,
        };
        // Start mid-storm. An empty sky means the first seconds after a switch
        // are a black panel, and frame 0 would paint nothing.
        let air = (rate as usize * rows * c.cell_h / fall.max(1) as usize).min(max_pieces);
        for i in 0..air {
            // Walk the gust through a whole sweep while seeding, or every
            // seeded piece lands in the band the sweep happens to start in and
            // the opening frame is one narrow column of confetti.
            c.gust_phase = (i * (32 << FP) / air.max(1)) as u32;
            let y = (next_rand(&mut c.rng) as usize % (rows * c.cell_h)) as i32;
            c.add(y << FP);
        }
        c.gust_phase = 0;
        c
    }

    /// Pile top of a column, in pixels from the top of the grid.
    #[inline]
    fn surface_px(&self, c: usize) -> i32 {
        (self.rows * self.cell_h) as i32 - (self.h[c] * self.cell_h as u32 / self.grains) as i32
    }

    fn spawn(&mut self) {
        for _ in 0..due(&mut self.spawn_acc, self.spawn_rate) {
            let y = -((self.cell_h as i32) << FP);
            self.add(y);
        }
    }

    /// One piece at height `y`, inside the gust band. The band is a sine sweep
    /// rather than a random walk so the heap it leaves is a smooth dune instead
    /// of a run of unrelated spikes.
    fn add(&mut self, y: i32) {
        if self.pieces.len() == self.max_pieces {
            return;
        }
        let r = next_rand(&mut self.rng);
        let s = next_rand(&mut self.rng);
        let span = ((self.cols * self.cell_w) as i32) << FP;
        let centre = span / 2 + (span / 2) * SIN[(self.gust_phase >> FP) as usize & 31] / 127;
        let x = centre - self.gust + (r % (2 * self.gust.max(1)) as u32) as i32;
        self.pieces.push(Piece {
            x: x.clamp(0, span - (1 << FP)),
            y,
            // +-25% so the flock never falls in ranks.
            vy: self.fall * (75 + (s % 51) as i32) / 100,
            phase: (r >> 9) as u16,
            sway: self.sway * (30 + ((s >> 8) % 71) as i32) / 100,
            col: 1 + (s >> 16) as u16 % COLOURS as u16,
            at: u32::MAX,
        });
    }

    /// Move every piece, landing the ones that reach their column's surface.
    fn fall_step(&mut self) {
        let (cell_w, cell_h) = (self.cell_w, self.cell_h);
        let span = (self.cols * cell_w) as i32;
        let mut i = 0;
        while i < self.pieces.len() {
            let p = &mut self.pieces[i];
            p.phase = p.phase.wrapping_add(1);
            p.y += p.vy;
            p.x += (p.sway * SIN[(p.phase >> 2) as usize & 31]) >> 7;
            p.x = p.x.clamp(0, (span - 1) << FP);
            let (x, y, col) = (p.x, p.y, p.col);
            let c = (x >> FP) as usize / cell_w;
            if y + ((cell_h as i32) << FP) >= (self.surface_px(c) << FP) {
                self.land(c, col);
                // Erase before the slot is reused: `at` is the only record of
                // what this piece covered.
                let at = self.pieces[i].at;
                if at != u32::MAX {
                    let under = self.beneath(at as usize);
                    put(&mut self.grid, &mut self.dirty, at as usize, under);
                }
                self.pieces.swap_remove(i);
                continue;
            }
            i += 1;
        }
    }

    fn land(&mut self, c: usize, col: u16) {
        let cap = (self.rows as u32) * self.grains;
        let add = self.deposit.min(cap - self.h[c]);
        self.h[c] += add;
        self.total += add as u64;
        if self.h[c] > 0 {
            let r = ((self.h[c] - 1) / self.grains) as usize;
            self.pile_col[r * self.cols + c] = col as u8;
        }
    }

    /// One toppling sweep: a pair steeper than `max_drop` moves half its excess
    /// downhill. Direction alternates per sweep so the heap does not drift the
    /// way a single-direction sweep makes it.
    fn topple(&mut self, rev: bool) {
        for k in 0..self.cols.saturating_sub(1) {
            let a = if rev { self.cols - 2 - k } else { k };
            let (lo, hi) = (self.h[a], self.h[a + 1]);
            let (src, dst, diff) = if lo > hi + self.max_drop {
                (a, a + 1, lo - hi)
            } else if hi > lo + self.max_drop {
                (a + 1, a, hi - lo)
            } else {
                continue;
            };
            let t = (diff - self.max_drop).div_ceil(2);
            self.h[src] -= t;
            self.h[dst] += t;
        }
    }

    /// Erode grains off random column tops. Removing from the top (rather than
    /// draining the bottom) keeps the heap a heap while it shrinks.
    fn erode(&mut self) {
        for _ in 0..due(&mut self.drain_acc, self.drain_rate) {
            if self.total == 0 {
                return;
            }
            let c = next_rand(&mut self.rng) as usize % self.cols;
            if self.h[c] > 0 {
                self.h[c] -= 1;
                self.total -= 1;
            }
        }
    }

    /// What row `r` of column `c` shows with no piece on it, at column height
    /// `height`. The one place the height map turns into a cell.
    #[inline]
    fn pile_cell(&self, height: u32, r: usize, c: usize) -> Cell {
        let (full, rem) = ((height / self.grains) as usize, height % self.grains);
        let col = self.pile_col[r * self.cols + c] as u16;
        if r < full {
            Cell::new(font::SOLID, col)
        } else if r == full && rem * 2 >= self.grains {
            // Half a cell of grain reads as a half block, which is what keeps
            // the crest of the heap from stepping in whole cells.
            Cell::new(font::LOWER, col)
        } else {
            Cell::CLEAR
        }
    }

    /// What a piece is covering, by grid index — against `drawn`, the height
    /// the grid was last painted from, NOT the live `h`. `draw_pile` repaints
    /// exactly the rows between `drawn` and `h`, so a write made from `drawn`
    /// is either already right or inside the range about to be repainted.
    /// Reading `h` here instead leaves a wrong cell behind whenever the landing
    /// that triggered the erase, or the topple after it, moved the column
    /// past what that range covers.
    #[inline]
    fn beneath(&self, i: usize) -> Cell {
        let c = i % self.cols;
        self.pile_cell(self.drawn[c], self.rows - 1 - i / self.cols, c)
    }

    /// Repaint the cells of every column whose height moved.
    fn draw_pile(&mut self) {
        for c in 0..self.cols {
            let (now, was) = (self.h[c], self.drawn[c]);
            if now == was {
                continue;
            }
            let lo = (now.min(was) / self.grains) as usize;
            let hi = ((now.max(was) / self.grains) as usize).min(self.rows - 1);
            for r in lo..=hi {
                let cell = self.pile_cell(now, r, c);
                let i = (self.rows - 1 - r) * self.cols + c;
                put(&mut self.grid, &mut self.dirty, i, cell);
            }
            self.drawn[c] = now;
        }
    }

    fn draw_pieces(&mut self) {
        for i in 0..self.pieces.len() {
            let p = self.pieces[i];
            let py = p.y >> FP;
            let at = if py < 0 {
                u32::MAX
            } else {
                let cy = py as usize / self.cell_h;
                if cy >= self.rows {
                    u32::MAX
                } else {
                    (cy * self.cols + (p.x >> FP) as usize / self.cell_w) as u32
                }
            };
            self.pieces[i].at = at;
            if at != u32::MAX {
                let g = TUMBLE[(p.phase >> 3) as usize & 3];
                put(
                    &mut self.grid,
                    &mut self.dirty,
                    at as usize,
                    Cell::new(g, p.col),
                );
            }
        }
    }
}

impl Saver for Confetti {
    fn render(&mut self, s: &mut Surface<'_>) {
        self.dirty.clear();
        self.frame = self.frame.wrapping_add(1);
        self.gust_phase = self.gust_phase.wrapping_add(self.gust_step) % (32 << FP);

        // Erase every piece where it was drawn BEFORE anything moves, so a
        // piece moving onto another's old cell cannot be erased by it.
        for i in 0..self.pieces.len() {
            let at = self.pieces[i].at;
            if at != u32::MAX {
                // Put back what the piece covered, not black: inside the heap,
                // CLEAR is a hole `draw_pile` will not repaint until that
                // column's height happens to move.
                let under = self.beneath(at as usize);
                put(&mut self.grid, &mut self.dirty, at as usize, under);
            }
        }

        // A gust can bury one end of the panel long before the heap as a whole
        // is full, so the ceiling is a trigger too.
        let peak = self.h.iter().copied().max().unwrap_or(0);
        if self.total >= self.drain_hi || peak + self.grains >= self.rows as u32 * self.grains {
            self.draining = true;
        } else if self.draining && self.total <= self.drain_lo {
            self.draining = false;
        }
        if self.draining {
            self.erode();
        } else {
            self.spawn();
        }

        self.fall_step();
        for k in 0..self.relax {
            self.topple((self.frame + k) & 1 == 1);
        }
        // Pile after the pieces' erases, so a column that grew over a piece's
        // old cell repaints it in the same frame.
        self.draw_pile();
        self.draw_pieces();

        // Row-major, so cells sharing a scanline collapse into one damage run —
        // `Damage::mark` merges only into the LAST run.
        self.dirty.sort_unstable();
        self.grid.flush_sparse(s, &PAL, &self.dirty);
    }

    fn name(&self) -> &'static str {
        "confetti"
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
    use crate::saver;
    use crate::surface::Damage;

    /// 1070 is deliberately not a multiple of the default 12px cell: the
    /// remainder strip is exactly what frame 0 is asked to prove it covers.
    fn panel() -> Panel {
        Panel::new(1920, 1070, 1920)
    }

    /// A lean in grain units per column as a rise/run percentage — what
    /// `CONFETTI_SLOPE` is set in. A grain is `cell_h / grains` pixels tall and
    /// a column is `cell_w` wide.
    fn slope_pct(c: &Confetti, lean: u32) -> u32 {
        lean * 100 * c.cell_h as u32 / (c.cell_w as u32 * c.grains)
    }

    /// The steepest lean anywhere in the height map, in grain units per column.
    fn steepest(c: &Confetti) -> u32 {
        (0..c.cols - 1)
            .map(|i| c.h[i].abs_diff(c.h[i + 1]))
            .max()
            .unwrap()
    }

    fn small() -> Panel {
        Panel::new(480, 274, 480)
    }

    /// Every scanline the frame actually changed must be inside a damage run.
    /// Blind to a dirty-list omission by construction — an unreported write is
    /// never blitted, so the pixels never move — which is why
    /// `damage_and_grid_agree_every_frame` carries the grid assert too.
    fn changed_rows_are_reported(before: &[u32], after: &[u32], p: &Panel, d: &Damage, n: usize) {
        for y in 0..p.h {
            let row = y * p.w..y * p.w + p.w;
            if before[row.clone()] == after[row] {
                continue;
            }
            assert!(
                d.runs()
                    .iter()
                    .any(|&(a, b)| y >= a as usize && y < b as usize),
                "frame {n}: scanline {y} changed but was not reported"
            );
        }
    }

    #[test]
    fn frame_zero_paints_the_whole_panel() {
        let p = panel();
        let mut c = Confetti::new(&p, 15);
        let mut buf = vec![0u32; p.buf_len()];
        let d = saver::frame(&mut c, &mut buf, &p);
        assert_eq!(d.rows(), p.h, "frame 0 must paint the whole panel");
        // A reported black rectangle would satisfy the line above, so: the sky
        // is seeded in `new` and frame 0 has to show it.
        let lit = buf.iter().filter(|&&px| px != 0).count();
        assert!(lit > 10_000, "frame 0 painted nothing ({lit} lit pixels)");
    }

    /// The Model B invariant. A `set` left out of `dirty` is a region of the
    /// panel frozen for the life of the pod, and no framebuffer diff can see
    /// it; `cur` vs `prev` can.
    #[test]
    fn damage_and_grid_agree_every_frame() {
        let p = small();
        let mut c = Confetti::new(&p, 15);
        let mut buf = vec![0u32; p.buf_len()];
        let mut prev = vec![0u32; p.buf_len()];
        let cells = c.grid.cols() * c.grid.rows();
        for n in 0..600 {
            prev.copy_from_slice(&buf);
            let d = saver::frame(&mut c, &mut buf, &p);
            for i in 0..cells {
                assert_eq!(
                    c.grid.cell(i),
                    c.grid.cells()[i],
                    "frame {n}: cell {i} was written but not reported"
                );
            }
            if n > 0 {
                changed_rows_are_reported(&prev, &buf, &p, &d, n);
            }
        }
    }

    /// CLAUDE.md makes the frame loop the top constraint in this repo. A Vec
    /// that never grows past its reserve never reallocates, and `clear` keeps
    /// capacity, so unchanged capacity IS "did not allocate".
    #[test]
    fn render_never_allocates() {
        let p = Panel::new(960, 544, 960);
        let mut c = Confetti::new(&p, 15);
        let mut buf = vec![0u32; p.buf_len()];
        let (dirty_cap, piece_cap) = (c.dirty.capacity(), c.pieces.capacity());
        let (h, drawn, pile) = (c.h.capacity(), c.drawn.capacity(), c.pile_col.capacity());
        assert!(dirty_cap > 0, "nothing was reserved for the frame loop");
        let mut worst = 0;
        for _ in 0..50_000 {
            saver::frame(&mut c, &mut buf, &p);
            worst = worst.max(c.dirty.len());
            assert_eq!(
                c.dirty.capacity(),
                dirty_cap,
                "`dirty` grew past its reserve: the render path allocated"
            );
            assert_eq!(c.pieces.capacity(), piece_cap, "`pieces` reallocated");
        }
        assert_eq!(
            (c.h.capacity(), c.drawn.capacity(), c.pile_col.capacity()),
            (h, drawn, pile)
        );
        // Non-vacuous: the list really does carry hundreds of cells a frame, so
        // a reserve of zero would have had to grow.
        assert!(worst > 300, "`dirty` never held a real frame ({worst})");
    }

    /// The pile is the saver, so the HEIGHT MAP has to reach the GRID: every
    /// cell the map says is buried is drawn as heap, unless a piece covers it
    /// this frame. Everything else here exercises the map and never looks at
    /// the panel — with `draw_pile` deleted outright the other six stayed
    /// green, and the picture was confetti falling onto a bare black floor.
    ///
    /// The same census catches the erase holes: an erase that put CLEAR inside
    /// the heap left a black cell there until that column's height moved.
    #[test]
    fn the_height_map_reaches_the_grid() {
        let p = small();
        // The shipped deposit, and the top of the documented range (1..=64),
        // where the holes were worst: 294 of them at 1920x1080, the
        // longest-lived lasting 2889 frames — over three minutes at 15fps.
        for deposit in [12u32, 64] {
            let mut c = Confetti::new(&p, 15);
            c.deposit = deposit;
            let mut buf = vec![0u32; p.buf_len()];
            let mut covered = vec![false; c.cols * c.rows];
            let mut checked = 0usize;
            for n in 0..2000 {
                saver::frame(&mut c, &mut buf, &p);
                covered.fill(false);
                for piece in &c.pieces {
                    if piece.at != u32::MAX {
                        covered[piece.at as usize] = true;
                    }
                }
                for col in 0..c.cols {
                    // The crest cell too, where `h` says half a cell or more of
                    // grain: a piece erase that blanked it leaves a notch in
                    // the skyline the same way a buried one leaves a hole.
                    let full = (c.h[col] / c.grains) as usize;
                    let crest = (c.h[col] % c.grains) * 2 >= c.grains && full < c.rows;
                    for r in 0..full + crest as usize {
                        let i = (c.rows - 1 - r) * c.cols + col;
                        if covered[i] {
                            continue;
                        }
                        let want = if r < full { font::SOLID } else { font::LOWER };
                        let cell = c.grid.cell(i);
                        // Glyph and lit-ness, not the exact speckle: which
                        // colour a buried cell wears is bookkeeping, whether
                        // it is drawn at all is the saver.
                        assert!(
                            cell.glyph() == want as usize
                                && (1..=COLOURS as usize).contains(&cell.colour()),
                            "deposit {deposit}, frame {n}: column {col} is {} grains deep \
                             but row {r} is not heap (glyph {}, colour {})",
                            c.h[col],
                            cell.glyph(),
                            cell.colour()
                        );
                        checked += 1;
                    }
                }
            }
            // Non-vacuous: a heap that never formed would assert nothing.
            assert!(checked > 100_000, "only {checked} buried cells checked");
        }
    }

    /// What this saver IS. Settled, no pair of neighbouring columns may lean
    /// steeper than the angle of repose — and the heap must actually LEAN,
    /// because a flat smear satisfies the limit trivially.
    #[test]
    fn the_pile_settles_to_the_angle_of_repose() {
        let p = panel();
        let mut c = Confetti::new(&p, 15);
        let mut buf = vec![0u32; p.buf_len()];
        let mut lean = Vec::new();
        for _ in 0..1500 {
            saver::frame(&mut c, &mut buf, &p);
            lean.push(steepest(&c));
        }
        // The LIVE picture, not just the settled one: toppling has to happen in
        // the frame loop. A flank under the gust is legitimately steep for a
        // few frames, so this is the median rather than the worst frame.
        lean.sort_unstable();
        let median = lean[lean.len() / 2];
        // A piece that landed this frame has not been toppled yet, so one
        // fresh deposit above the limit is the honest bound.
        let live = c.max_drop + c.deposit;
        assert!(
            median <= live,
            "the typical frame leans at {median} units/column, limit is {live}"
        );
        // Let the avalanche finish: toppling is rate-limited per frame on
        // purpose, so a flank under an active gust is legitimately steep.
        for k in 0..4000 {
            c.topple(k & 1 == 1);
        }
        // Pinned to the NUMBER, not to `max_drop` — `max_drop` is derived from
        // `CONFETTI_SLOPE`, so comparing against it passes for any slope at
        // all: 200 (a near-vertical wall) and 5 (a flat smear) were both green
        // before this. Measured on the height map, where the physics is: the
        // heap renders at half-cell resolution, so a 48% slope draws as 6px of
        // a 12px column and no per-adjacent-column PIXEL rule can express it.
        let pct = slope_pct(&c, steepest(&c));
        assert!(
            (44..=52).contains(&pct),
            "a settled pile leans at {pct}%, the angle of repose is 48%"
        );
        let (hi, lo) = (*c.h.iter().max().unwrap(), *c.h.iter().min().unwrap());
        assert!(
            hi - lo > 4 * c.grains,
            "no relief: {lo} to {hi} grain units"
        );
    }

    /// Toppling MOVES grains. A transfer that loses one drains the heap in a
    /// way that looks exactly like the intended drain, and nothing on the panel
    /// would tell you which one you were watching.
    #[test]
    fn relax_conserves_grains() {
        let p = panel();
        let mut c = Confetti::new(&p, 15);
        let mut rng = 0x1234_5678u32;
        for i in 0..c.cols {
            c.h[i] = next_rand(&mut rng) % (c.rows as u32 * c.grains);
        }
        let before: u64 = c.h.iter().map(|&v| v as u64).sum();
        for k in 0..200 {
            c.topple(k & 1 == 1);
            let now: u64 = c.h.iter().map(|&v| v as u64).sum();
            assert_eq!(now, before, "sweep {k} changed the grain count");
        }
    }

    /// The heap must not simply eat the panel: past `CONFETTI_FULL_PCT` the
    /// spawner stops and grains erode until it is half empty again.
    #[test]
    fn the_heap_drains_instead_of_burying_the_panel() {
        let p = small();
        let mut c = Confetti::new(&p, 15);
        let mut buf = vec![0u32; p.buf_len()];
        let mut peak = 0;
        let (mut filled, mut drained) = (false, false);
        for _ in 0..4000 {
            saver::frame(&mut c, &mut buf, &p);
            peak = peak.max(c.total);
            filled |= c.draining;
            drained |= filled && c.total <= c.drain_lo;
        }
        assert!(
            filled,
            "the heap never reached the drain threshold (peak {peak}, trigger {})",
            c.drain_hi
        );
        assert!(
            drained,
            "the heap filled to {peak} and never drained back below {}",
            c.drain_lo
        );
    }
}
