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
const PAL_RGB: [[u8; 3]; 14] = [
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
    [0x8A, 0x90, 0x9C], // ledge — grey, so a shelf never reads as confetti
];
const PAL: [u32; 14] = bake(&PAL_RGB);
/// Confetti colours are 1..=12. Pinned, not `PAL.len() - 1`: index 13 is the
/// ledge grey and no piece may be dealt it.
const COLOURS: u32 = 12;
const SHELF_COL: u16 = 13;

/// A ledge: the TOP half of its cell, so the pile above rests on a drawn line
/// instead of floating a half-cell above one.
const LEDGE: Cell = Cell::new(font::UPPER, SHELF_COL);

/// `shelf_of` for a column with no ledge over it.
const NO_SHELF: u16 = u16::MAX;

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

/// A ledge spanning cells `c0..c1` of row `row` (counted from the bottom, like
/// the height map), with a pile of its own on top.
#[derive(Clone, Copy)]
struct Shelf {
    c0: usize,
    c1: usize,
    row: usize,
}

#[derive(Clone, Copy)]
struct Piece {
    x: i32,
    y: i32,
    vy: i32,
    phase: u16,
    sway: i32,
    col: u16,
    /// Grain units this piece will deposit. `deposit` for one that fell out of
    /// the sky; whatever came off the edge, for one a shelf spilled. Carried
    /// rather than assumed so a spill is a MOVE of grains and not a fresh
    /// invention of them.
    load: u32,
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
    shelves: Vec<Shelf>,
    /// Column -> index into `shelves`, or `NO_SHELF`. At most one: shelves are
    /// placed one to a slot, so no column is ever under two of them.
    shelf_of: Vec<u16>,
    /// The pile standing ON each shelf, in grain units, based at `row + 1`.
    /// Zero for a column with no shelf. Its own height map, because a shelf
    /// pile and the floor pile under it lean independently.
    sh: Vec<u32>,
    sdrawn: Vec<u32>,
    /// Ceilings, in grain units: `cap` is where a column's FLOOR pile stops —
    /// the underside of its shelf, or the top of the panel — and `scap` where
    /// its shelf pile does. Toppling honours them, so a transfer into a full
    /// column cannot invent grains the column has nowhere to put.
    cap: Vec<u32>,
    scap: Vec<u32>,
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

/// One neighbouring pair of a height map: leaning steeper than `max_drop`,
/// half the excess slides downhill, clipped to the room the low column has
/// left under its ceiling. Moves grains and never invents one, which is what
/// `relax_conserves_grains` pins.
#[inline]
fn settle(hs: &mut [u32], caps: &[u32], a: usize, b: usize, max_drop: u32) {
    let (lo, hi) = (hs[a], hs[b]);
    let (src, dst, diff) = if lo > hi + max_drop {
        (a, b, lo - hi)
    } else if hi > lo + max_drop {
        (b, a, hi - lo)
    } else {
        return;
    };
    let t = (diff - max_drop).div_ceil(2).min(caps[dst] - hs[dst]);
    hs[src] -= t;
    hs[dst] += t;
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
        //
        // Against `cell`, the SQUARE-pixel cell height, not `grid.cell_h()`,
        // which `SAVER_PIXEL_ASPECT` has stretched: a grain is a slice of the
        // stretched cell, so dividing by the stretched height would hold the
        // pile's incline fixed in framebuffer pixels and land it on pine's
        // panel at 48/1.8 = 27%. The angle is meant to be 48% on the GLASS.
        let max_drop = (slope * grid.cell_w() as u32 * grains / (100 * cell as u32)).max(1);
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

        // Ledges, one to an equal-width slot. The slot IS the non-overlap
        // proof — no pair can touch and no shelf can shadow another — and the
        // two spare columns each keeps on its left leave a gap for confetti to
        // fall past, plus the column on its right that the spill rule needs
        // somewhere to land. A slot under 6 columns has no room for that, so a
        // narrow panel gets fewer shelves rather than crowded ones.
        let want = if rows < 6 {
            0
        } else {
            (3 + next_rand(&mut rng) as usize % 5).min(cols / 6)
        };
        let slot = cols.checked_div(want).unwrap_or(0);
        let mut shelves: Vec<Shelf> = Vec::with_capacity(want);
        for k in 0..want {
            let avail = slot - 4;
            let least = (avail / 2).max(1);
            let w = least + next_rand(&mut rng) as usize % (avail - least + 1);
            let c0 = k * slot + 2 + next_rand(&mut rng) as usize % (slot - 2 - w);
            // Well clear of the floor heap below and the sky above: a ledge on
            // the deck is just a bump, one at the ceiling never gets a pile.
            let row = (rows * 40 / 100 + next_rand(&mut rng) as usize % (rows * 45 / 100).max(1))
                .clamp(1, rows - 2);
            shelves.push(Shelf {
                c0,
                c1: c0 + w,
                row,
            });
        }
        let mut shelf_of = vec![NO_SHELF; cols];
        let mut cap = vec![rows as u32 * grains; cols];
        let mut scap = vec![0u32; cols];
        for (i, sv) in shelves.iter().enumerate() {
            for c in sv.c0..sv.c1 {
                shelf_of[c] = i as u16;
                cap[c] = sv.row as u32 * grains;
                scap[c] = (rows - 1 - sv.row) as u32 * grains;
            }
        }

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
            sh: vec![0; cols],
            sdrawn: vec![0; cols],
            shelves,
            shelf_of,
            cap,
            scap,
            pile_col,
            pieces: Vec::with_capacity(max_pieces),
            // Airtight bound: two cells per piece, plus at most one whole
            // column repaint per column. `render_never_allocates` is only
            // meaningful because this can never be exceeded.
            dirty: Vec::with_capacity(max_pieces * 2 + cols * (rows + 2)),
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
        // Straight into the grid, not through `put`: a ledge is static, and
        // frame 0 blits every cell regardless of `dirty`. The only thing that
        // ever overwrites one is a piece, and `cell_at` puts it back.
        for i in 0..c.shelves.len() {
            let sv = c.shelves[i];
            for col in sv.c0..sv.c1 {
                c.grid.set((rows - 1 - sv.row) * cols + col, LEDGE);
            }
        }
        c
    }

    /// Pixels from the top of the grid down to the top of a pile `high` grain
    /// units tall, measured from the floor.
    #[inline]
    fn top_px(&self, high: u32) -> i32 {
        (self.rows * self.cell_h) as i32 - (high * self.cell_h as u32 / self.grains) as i32
    }

    /// Top of the pile standing on column `c`'s shelf, in grain units from the
    /// FLOOR — the ledge cell itself plus whatever has landed on it.
    #[inline]
    fn shelf_top(&self, c: usize, s: u16) -> u32 {
        (self.shelves[s as usize].row as u32 + 1) * self.grains + self.sh[c]
    }

    /// Floor-pile top of a column, in pixels from the top of the grid.
    #[inline]
    fn surface_px(&self, c: usize) -> i32 {
        self.top_px(self.h[c])
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
        if self.pieces.len() >= self.max_pieces {
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
            load: self.deposit,
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
            let was = p.y + ((cell_h as i32) << FP);
            p.y += p.vy;
            p.x += (p.sway * SIN[(p.phase >> 2) as usize & 31]) >> 7;
            p.x = p.x.clamp(0, (span - 1) << FP);
            let (x, y, col, load) = (p.x, p.y, p.col, p.load);
            let c = (x >> FP) as usize / cell_w;
            let foot = y + ((cell_h as i32) << FP);
            // A shelf catches only what CROSSES its surface this frame. Testing
            // the foot alone would teleport a piece that swayed in under the
            // ledge up onto it, which is the one way a ledge can look fake.
            let s = self.shelf_of[c];
            let caught = s != NO_SHELF && self.sh[c] < self.scap[c] && {
                let top = self.top_px(self.shelf_top(c, s)) << FP;
                was <= top && foot >= top
            };
            if caught {
                self.land_shelf(c, s, col, load);
            } else if foot < (self.surface_px(c) << FP) {
                i += 1;
                continue;
            } else {
                self.land(c, col, load);
            }
            // Erase before the slot is reused: `at` is the only record of
            // what this piece covered.
            let at = self.pieces[i].at;
            if at != u32::MAX {
                let under = self.beneath(at as usize);
                put(&mut self.grid, &mut self.dirty, at as usize, under);
            }
            self.pieces.swap_remove(i);
        }
    }

    fn land(&mut self, c: usize, col: u16, load: u32) {
        let add = load.min(self.cap[c] - self.h[c]);
        self.h[c] += add;
        self.total += add as u64;
        // `add == 0` is a column with no room left. Stamping its top cell
        // anyway changes what `beneath` puts back without changing the height
        // `draw_pile` watches, so the colour would never reach the grid.
        if add > 0 {
            let r = ((self.h[c] - 1) / self.grains) as usize;
            self.pile_col[r * self.cols + c] = col as u8;
        }
    }

    /// The same deposit, onto the pile standing on column `c`'s shelf.
    fn land_shelf(&mut self, c: usize, s: u16, col: u16, load: u32) {
        let add = load.min(self.scap[c] - self.sh[c]);
        self.sh[c] += add;
        self.total += add as u64;
        if add > 0 {
            let r = self.shelves[s as usize].row + 1 + ((self.sh[c] - 1) / self.grains) as usize;
            self.pile_col[r * self.cols + c] = col as u8;
        }
    }

    /// One toppling sweep over the floor and over every shelf. Direction
    /// alternates per sweep so the heap does not drift the way a
    /// single-direction sweep makes it.
    fn topple(&mut self, rev: bool) {
        for k in 0..self.cols.saturating_sub(1) {
            let a = if rev { self.cols - 2 - k } else { k };
            settle(&mut self.h, &self.cap, a, a + 1, self.max_drop);
        }
        for i in 0..self.shelves.len() {
            let sv = self.shelves[i];
            for k in 0..(sv.c1 - sv.c0).saturating_sub(1) {
                let a = if rev { sv.c1 - 2 - k } else { sv.c0 + k };
                settle(&mut self.sh, &self.scap, a, a + 1, self.max_drop);
            }
            // Past the end of a ledge is AIR, not a shorter column, so the
            // neighbour's height is zero and the drop is unbounded: whatever
            // leans over the edge topples off and rejoins the floor beside it.
            // Same half-the-excess rate as any other pair, so it spills as a
            // visible trickle rather than a dump.
            for (edge, out) in [(sv.c0, sv.c0 - 1), (sv.c1 - 1, sv.c1)] {
                if self.sh[edge] > self.max_drop {
                    let t = (self.sh[edge] - self.max_drop).div_ceil(2);
                    // Taken off the ledge only once it has somewhere to go.
                    // Subtracting first and letting `spill` keep what would
                    // not fit destroys grains `total` still believes in.
                    let moved = self.spill(out, sv.row, t);
                    self.sh[edge] -= moved;
                }
            }
        }
    }

    /// Grains that went over a ledge, handed back to the air beside it so they
    /// fall to the next surface in view instead of appearing in the floor map.
    /// Returns how many it took: the sky and the floor beside the shelf can
    /// both be full, and what neither will take stays on the ledge. Dropping
    /// it instead leaves `total` counting grains that no longer exist, and
    /// `total` is the only thing the drain looks at — the panel would erode
    /// and never fill again.
    fn spill(&mut self, out: usize, row: usize, t: u32) -> u32 {
        if self.pieces.len() >= self.max_pieces {
            // Sky full: straight into the floor map, unwatchable but not lost.
            // Both maps, so `total` does not move.
            let t = t.min(self.cap[out] - self.h[out]);
            self.h[out] += t;
            return t;
        }
        self.total -= t as u64;
        let r = next_rand(&mut self.rng);
        self.pieces.push(Piece {
            x: ((out * self.cell_w + self.cell_w / 2) as i32) << FP,
            y: (((self.rows - 1 - row) * self.cell_h) as i32) << FP,
            vy: self.fall,
            phase: (r >> 9) as u16,
            // No sway: it fell off an edge, it did not flutter off one.
            sway: 0,
            col: 1 + r as u16 % COLOURS as u16,
            load: t,
            at: u32::MAX,
        });
        t
    }

    /// Erode grains off random column tops. Removing from the top (rather than
    /// draining the bottom) keeps the heap a heap while it shrinks.
    fn erode(&mut self) {
        for _ in 0..due(&mut self.drain_acc, self.drain_rate) {
            if self.total == 0 {
                return;
            }
            let c = next_rand(&mut self.rng) as usize % self.cols;
            // The shelf pile first: it is what the eye is on, and eroding the
            // floor under a ledge would never show.
            if self.sh[c] > 0 {
                self.sh[c] -= 1;
                self.total -= 1;
            } else if self.h[c] > 0 {
                self.h[c] -= 1;
                self.total -= 1;
            }
        }
    }

    /// What the `k`th cell up from row `base` shows, for a pile `height` grain
    /// units tall standing there. The one place a height map turns into a cell,
    /// and the only thing a shelf pile needs that the floor pile did not: a
    /// base row other than zero.
    #[inline]
    fn stack_cell(&self, height: u32, base: usize, k: usize, c: usize) -> Cell {
        let (full, rem) = ((height / self.grains) as usize, height % self.grains);
        let col = self.pile_col[(base + k) * self.cols + c] as u16;
        if k < full {
            Cell::new(font::SOLID, col)
        } else if k == full && rem * 2 >= self.grains {
            // Half a cell of grain reads as a half block, which is what keeps
            // the crest of the heap from stepping in whole cells.
            Cell::new(font::LOWER, col)
        } else {
            Cell::CLEAR
        }
    }

    /// What row `r` of column `c` shows with no piece on it: its ledge, the
    /// pile on that ledge, or the floor pile under it.
    #[inline]
    fn cell_at(&self, c: usize, r: usize) -> Cell {
        let s = self.shelf_of[c];
        if s != NO_SHELF {
            let row = self.shelves[s as usize].row;
            if r == row {
                return LEDGE;
            }
            if r > row {
                return self.stack_cell(self.sdrawn[c], row + 1, r - row - 1, c);
            }
        }
        self.stack_cell(self.drawn[c], 0, r, c)
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
        self.cell_at(i % self.cols, self.rows - 1 - i / self.cols)
    }

    /// Repaint one pile's changed cells: the rows between where it was drawn
    /// and where it is now, clamped to `cells` so a floor pile filled to the
    /// underside of its shelf never paints over the ledge.
    fn draw_span(&mut self, c: usize, base: usize, cells: usize, now: u32, was: u32) {
        let lo = (now.min(was) / self.grains) as usize;
        let hi = ((now.max(was) / self.grains) as usize).min(cells - 1);
        for k in lo..=hi {
            let cell = self.stack_cell(now, base, k, c);
            let i = (self.rows - 1 - (base + k)) * self.cols + c;
            put(&mut self.grid, &mut self.dirty, i, cell);
        }
    }

    /// Repaint the cells of every pile whose height moved — the floor pile of
    /// each column, and the shelf pile where there is one.
    fn draw_pile(&mut self) {
        for c in 0..self.cols {
            let (now, was) = (self.h[c], self.drawn[c]);
            if now != was {
                let cells = (self.cap[c] / self.grains) as usize;
                self.draw_span(c, 0, cells, now, was);
                self.drawn[c] = now;
            }
            let s = self.shelf_of[c];
            if s == NO_SHELF {
                continue;
            }
            let (now, was) = (self.sh[c], self.sdrawn[c]);
            if now != was {
                let base = self.shelves[s as usize].row + 1;
                self.draw_span(c, base, self.rows - base, now, was);
                self.sdrawn[c] = now;
            }
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
        // is full, so the ceiling is a trigger too. Measured against each
        // pile's OWN ceiling: a floor column stopped under a ledge is a full
        // bin, not a full panel, and draining on that would never let a shelf
        // load up.
        let mut roof = false;
        for c in 0..self.cols {
            roof |= if self.shelf_of[c] == NO_SHELF {
                self.h[c] + self.grains >= self.cap[c]
            } else {
                self.sh[c] + self.grains >= self.scap[c]
            };
        }
        if self.total >= self.drain_hi || roof {
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
    use crate::dump;
    use crate::saver;
    use crate::surface::Damage;

    /// 1070 is deliberately not a multiple of the default 12px cell: the
    /// remainder strip is exactly what frame 0 is asked to prove it covers.
    fn panel() -> Panel {
        Panel::new(1920, 1070, 1920)
    }

    /// `CONFETTI_SLOPE` is 48% on the GLASS, not in the framebuffer. Pine's
    /// panel squashes by 1.8, so the framebuffer incline has to be that much
    /// steeper — which it is only because `max_drop` is derived from the
    /// SQUARE-pixel cell height rather than from the stretched one.
    #[test]
    fn the_pile_leans_at_its_angle_on_the_glass() {
        let at = |a| crate::grid::with_test_aspect(a, || Confetti::new(&panel(), 30));
        let (sq, pine) = (at(100), at(180));
        assert_eq!(
            sq.max_drop, pine.max_drop,
            "the lean is in grains either way"
        );
        // A grain is a slice of the stretched cell, so the same grain count is
        // 1.8x the framebuffer rise — 48% becomes 86%, and 86/1.8 is 48 again.
        let (a, b) = (slope_pct(&sq, sq.max_drop), slope_pct(&pine, pine.max_drop));
        assert_eq!(a, 48);
        assert!(
            (80..92).contains(&b),
            "framebuffer slope {b}% is not ~1.8 x {a}%"
        );
    }

    /// A lean in grain units per column as a rise/run percentage — what
    /// `CONFETTI_SLOPE` is set in. A grain is `cell_h / grains` pixels tall and
    /// a column is `cell_w` wide.
    fn slope_pct(c: &Confetti, lean: u32) -> u32 {
        lean * 100 * c.cell_h as u32 / (c.cell_w as u32 * c.grains)
    }

    /// The steepest lean anywhere in the floor height map, in grain units per
    /// column. A pair whose LOW column is filled to the underside of a shelf is
    /// not a slope — it is a full bin that cannot take the grains that would
    /// flatten it — so it is skipped rather than counted as a violation.
    fn steepest(c: &Confetti) -> u32 {
        let mut worst = 0;
        for i in 0..c.cols - 1 {
            let low = i + (c.h[i + 1] < c.h[i]) as usize;
            if c.h[low] < c.cap[low] {
                worst = worst.max(c.h[i].abs_diff(c.h[i + 1]));
            }
        }
        worst
    }

    /// The steepest lean on any shelf, over the pairs a shelf's own relaxation
    /// owns — its interior. The edge pairs belong to the spill rule instead.
    fn steepest_shelf(c: &Confetti) -> u32 {
        c.shelves
            .iter()
            .flat_map(|sv| sv.c0..sv.c1.saturating_sub(1))
            .map(|i| c.sh[i].abs_diff(c.sh[i + 1]))
            .max()
            .unwrap_or(0)
    }

    /// Grains in the two height maps — what `total` claims to be. In-flight
    /// loads are deliberately NOT here: `total` is what the drain measures and
    /// the drain is about how much of the panel the heap has eaten.
    fn on_the_ground(c: &Confetti) -> u64 {
        c.h.iter().chain(c.sh.iter()).map(|&v| v as u64).sum()
    }

    /// Every grain the saver is holding: the ground, plus the loads in flight —
    /// a spill leaves the maps and comes back as a piece, so counting only the
    /// maps would call a lost grain conserved.
    fn grains_held(c: &Confetti) -> u64 {
        on_the_ground(c) + c.pieces.iter().map(|p| p.load as u64).sum::<u64>()
    }

    /// Load every ledge past what it can hold and fill the floor columns beside
    /// them to the brim — the state where both the spill and the ceiling clips
    /// have to work, and the one a shelf at rest never reaches on its own.
    fn overload(c: &mut Confetti, full_sky: bool) {
        if full_sky {
            // Nothing can be airborne, so every spill has to take the
            // map-to-map fallback. Setting it to the current length does not
            // work: pieces land and the list drains below the limit again.
            c.max_pieces = 0;
        }
        for i in 0..c.shelves.len() {
            let sv = c.shelves[i];
            for col in sv.c0..sv.c1 {
                c.sh[col] = c.scap[col];
            }
            c.h[sv.c0 - 1] = c.cap[sv.c0 - 1];
            c.h[sv.c1] = c.cap[sv.c1];
        }
        c.total = on_the_ground(c);
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
                dump::row_reported(&before[y * p.w..][..p.w], &after[y * p.w..][..p.w], y, d),
                "frame {n}: scanline {y} changed outside every reported rect"
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
                    // Both piles of the column: the floor one, and the one on
                    // its ledge. A shelf pile that never reached the grid is
                    // the same bug one storey up.
                    let s = c.shelf_of[col];
                    let spans = if s == NO_SHELF {
                        [(0usize, c.h[col], c.rows), (0, 0, 0)]
                    } else {
                        let sv = c.shelves[s as usize];
                        // The ledge is what the pile visibly rests on. Undrawn,
                        // the pile floats.
                        let i = (c.rows - 1 - sv.row) * c.cols + col;
                        if !covered[i] {
                            let cell = c.grid.cell(i);
                            assert_eq!(
                                (cell.glyph(), cell.colour()),
                                (font::UPPER as usize, SHELF_COL as usize),
                                "deposit {deposit}, frame {n}: column {col} row {} \
                                 is a ledge but is not drawn as one",
                                sv.row
                            );
                            checked += 1;
                        }
                        [(0, c.h[col], sv.row), (sv.row + 1, c.sh[col], c.rows)]
                    };
                    for (base, height, top) in spans {
                        // The crest cell too, where the map says half a cell or
                        // more of grain: a piece erase that blanked it leaves a
                        // notch in the skyline the same way a buried one leaves
                        // a hole.
                        let full = (height / c.grains) as usize;
                        let crest = (height % c.grains) * 2 >= c.grains && base + full < top;
                        for k in 0..full + crest as usize {
                            let r = base + k;
                            let i = (c.rows - 1 - r) * c.cols + col;
                            if covered[i] {
                                continue;
                            }
                            let want = if k < full { font::SOLID } else { font::LOWER };
                            let cell = c.grid.cell(i);
                            // Glyph and lit-ness, not the exact speckle: which
                            // colour a buried cell wears is bookkeeping, whether
                            // it is drawn at all is the saver.
                            assert!(
                                cell.glyph() == want as usize
                                    && (1..=COLOURS as usize).contains(&cell.colour()),
                                "deposit {deposit}, frame {n}: column {col} is {height} \
                                 grains deep over row {base} but row {r} is not heap \
                                 (glyph {}, colour {})",
                                cell.glyph(),
                                cell.colour()
                            );
                            checked += 1;
                        }
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

        // Every shelf obeys the same angle. Not pinned to a floor of 44% the
        // way the deck is — a short shelf pile settles flatter than the limit
        // and that is legal — so the ceiling is the assertion, plus relief, so
        // a shelf that never collected anything cannot pass by being empty.
        let pct = slope_pct(&c, steepest_shelf(&c));
        assert!(
            pct <= 52,
            "a settled shelf pile leans at {pct}%, the angle of repose is 48%"
        );
        let peak = *c.sh.iter().max().unwrap();
        assert!(
            peak > 2 * c.grains,
            "no shelf ever built a pile (tallest {peak} grain units)"
        );
    }

    /// A ledge is 3..=7 of them, none over another, each with air on both sides
    /// for confetti to fall past and air above for a pile to grow into.
    #[test]
    fn shelves_are_placed_apart_with_room_to_fall_past() {
        // A spread of widths, not one panel: the layout is drawn from the seed
        // and the column count is the only thing that moves it, so a single
        // size can satisfy the gap rule by luck.
        let mut sizes = vec![(1920, 1070), (1280, 720), (480, 274)];
        sizes.extend((480..=1920).step_by(96).map(|w| (w, 1080)));
        let mut rows_seen = Vec::new();
        for (w, h) in sizes {
            let p = Panel::new(w, h, w);
            let c = Confetti::new(&p, 15);
            assert!(
                (3..=7).contains(&c.shelves.len()),
                "{w}x{h}: {} shelves, wanted 3..=7",
                c.shelves.len()
            );
            let mut last_end = 0;
            for sv in &c.shelves {
                assert!(sv.c0 < sv.c1, "{w}x{h}: empty shelf");
                assert!(sv.row >= 1 && sv.row + 1 < c.rows, "{w}x{h}: no room above");
                // Two clear columns between shelves and at both panel edges:
                // the gap IS where confetti reaches the floor, and the reason
                // no shelf can be stranded under another.
                assert!(
                    sv.c0 >= last_end + 2,
                    "{w}x{h}: shelf at {} starts {} columns after the last",
                    sv.c0,
                    sv.c0 as i64 - last_end as i64
                );
                last_end = sv.c1;
            }
            // One clear column at the panel edge is the minimum: the spill
            // rule reads the floor column just past a ledge's end.
            assert!(last_end < c.cols, "{w}x{h}: a shelf runs off the panel");
            rows_seen.push(c.shelves.iter().map(|sv| sv.row).collect::<Vec<_>>());
            // Deterministic from the seed, or none of the above is testable.
            let again = Confetti::new(&p, 15);
            let same = c
                .shelves
                .iter()
                .zip(&again.shelves)
                .all(|(a, b)| (a.c0, a.c1, a.row) == (b.c0, b.c1, b.row));
            assert!(same, "{w}x{h}: the layout is not reproducible");
        }
        // Random HEIGHTS, not just random positions: ledges all on one line is
        // a shelving unit, not confetti weather.
        assert!(
            rows_seen.iter().any(|rs| rs.iter().any(|r| *r != rs[0])),
            "every shelf sits at the same height on every panel"
        );
    }

    /// `beneath` is the answer an erase gives and `draw_pile` is the answer the
    /// repaint gives; they have to be the SAME answer for every cell, on every
    /// storey. They were not, once — `beneath` read the live `h` and left a
    /// stale half block — and a shelf pile doubles the number of ways to
    /// disagree: the ledge cell, and the pile standing on it.
    #[test]
    fn what_an_erase_puts_back_is_what_the_pile_paints() {
        let p = small();
        let mut c = Confetti::new(&p, 15);
        let mut buf = vec![0u32; p.buf_len()];
        let mut rng = 0xA11C_E123u32;
        let mut covered = vec![false; c.cols * c.rows];
        let mut ledges = 0usize;
        for _ in 0..200 {
            // Shove both maps somewhere new, so the repaint has real ground to
            // cover rather than the frame's own drift.
            for col in 0..c.cols {
                c.h[col] = next_rand(&mut rng) % (c.cap[col] + 1);
                c.sh[col] = next_rand(&mut rng) % (c.scap[col] + 1);
            }
            c.total = on_the_ground(&c);
            saver::frame(&mut c, &mut buf, &p);
            covered.fill(false);
            for piece in &c.pieces {
                if piece.at != u32::MAX {
                    covered[piece.at as usize] = true;
                }
            }
            for (i, &hidden) in covered.iter().enumerate() {
                if hidden {
                    continue;
                }
                let (col, r) = (i % c.cols, c.rows - 1 - i / c.cols);
                // Glyph, not the whole cell: which speckle colour a buried cell
                // wears is bookkeeping, the same thing the census says. Solid /
                // half / blank / ledge is the part an erase can get wrong in a
                // way anyone sees.
                assert_eq!(
                    c.beneath(i).glyph(),
                    c.grid.cell(i).glyph(),
                    "column {col} row {r}: an erase would put back something \
                     other than what the pile is drawn as"
                );
                ledges += (c.grid.cell(i).glyph() == font::UPPER as usize) as usize;
            }
        }
        // Non-vacuous over the part that is new: ledges were in the sweep.
        assert!(ledges > 1000, "only {ledges} ledge cells were compared");
    }

    /// A pile on a shelf grows until it reaches the edge, and then goes over
    /// it: past the end there is only air, so the excess topples off and
    /// carries on down to the floor beside the shelf.
    #[test]
    fn a_shelf_pile_spills_off_its_edges() {
        let p = panel();
        let mut c = Confetti::new(&p, 15);
        let sv = c.shelves[0];
        let mid = (sv.c0 + sv.c1) / 2;
        // Loaded past what the ledge can hold at the angle of repose — which is
        // the only state the edge rule is about. A pile that still fits settles
        // into a cone with its feet short of both ends and spills nothing.
        for col in sv.c0..sv.c1 {
            c.sh[col] = c.scap[col];
        }
        c.total = on_the_ground(&c);
        let loaded = grains_held(&c);
        for k in 0..4000 {
            c.topple(k & 1 == 1);
        }
        assert_eq!(grains_held(&c), loaded, "grains went missing over the edge");
        // What went over the edge is falling again, beside the shelf and no
        // longer on it — not teleported into the floor map, which is invisible.
        let (mut left, mut right) = (0u64, 0u64);
        for p in &c.pieces {
            let col = (p.x >> FP) as usize / c.cell_w;
            left += (col < sv.c0) as u64;
            right += (col >= sv.c1) as u64;
        }
        assert!(
            left > 0 && right > 0,
            "nothing spilled off either end ({left} left, {right} right)"
        );
        assert!(
            c.sh[mid] < c.scap[mid],
            "the spike on the shelf never came down"
        );
        let edges = (c.sh[sv.c0], c.sh[sv.c1 - 1]);
        assert!(
            edges.0 <= c.max_drop && edges.1 <= c.max_drop,
            "the ends of the ledge are still overloaded: {edges:?}"
        );
    }

    /// Every pile has a ceiling — the underside of its shelf for a floor pile,
    /// the top of the panel for a shelf pile. Nothing may push a column past
    /// it: a floor pile that grows through its own ledge paints over the drawn
    /// line, and every `cap - h` in the file underflows from there. Two writers
    /// have to clip, `settle` and the spill's full-sky fallback, and only a
    /// loaded shelf over a full floor column exercises both.
    #[test]
    fn no_pile_grows_through_its_ceiling() {
        let p = panel();
        let mut buf = vec![0u32; p.buf_len()];
        // Both regimes: a spill becomes a falling piece while there is room in
        // the sky, and a straight map-to-map transfer when there is not. Only
        // the second one writes a height map without going through `land`.
        for full_sky in [false, true] {
            let mut c = Confetti::new(&p, 15);
            overload(&mut c, full_sky);
            // Non-vacuous by construction: the columns beside every ledge start
            // AT their ceiling, so the clips are load-bearing from frame 0.
            assert!(c.h.iter().zip(&c.cap).any(|(h, cap)| h == cap));
            for n in 0..2000 {
                saver::frame(&mut c, &mut buf, &p);
                for col in 0..c.cols {
                    assert!(
                        c.h[col] <= c.cap[col],
                        "frame {n} (full sky {full_sky}): floor column {col} is \
                         {} grains deep, ceiling is {}",
                        c.h[col],
                        c.cap[col]
                    );
                    assert!(
                        c.sh[col] <= c.scap[col],
                        "frame {n} (full sky {full_sky}): shelf column {col} is \
                         {} grains deep, ceiling is {}",
                        c.sh[col],
                        c.scap[col]
                    );
                }
            }
        }
    }

    /// The spill's last resort, driven directly. With the sky full the grains
    /// go map-to-map, and the transfer takes only what the floor column has
    /// room for — whatever it refuses stays on the ledge. Dropping that instead
    /// loses grains `total` still counts, and `total` is all the drain looks
    /// at: the panel would erode and never fill again.
    ///
    /// Driven rather than waited for. The column beside a ledge is never itself
    /// shelved, so its ceiling is the top of the panel, and no run of the saver
    /// reaches a refusal before the roof trigger drains it — which is exactly
    /// why the branch needs writing down rather than watching for.
    #[test]
    fn a_spill_that_is_refused_leaves_the_grains_on_the_ledge() {
        let p = panel();
        let mut c = Confetti::new(&p, 15);
        let sv = c.shelves[0];
        let (edge, out) = (sv.c1 - 1, sv.c1);
        // Sky full, so `spill` cannot hand the grains to a falling piece.
        c.max_pieces = 0;
        c.sh[edge] = c.scap[edge];
        // Five grains of room beside the ledge, and no more.
        c.cap[out] = c.h[out] + 5;
        c.total = on_the_ground(&c);
        let before = grains_held(&c);

        let moved = c.spill(out, sv.row, 40);
        // What the caller does with the answer, and the whole reason there is
        // an answer rather than a `()`.
        c.sh[edge] -= moved;
        assert_eq!(moved, 5, "the spill took more than the column had room for");
        assert_eq!(c.h[out], c.cap[out], "and did not fill the room it had");
        assert_eq!(
            grains_held(&c),
            before,
            "a map-to-map transfer changed the grain count"
        );
        assert_eq!(
            c.total,
            on_the_ground(&c),
            "a map-to-map transfer moved `total`"
        );

        // Now with no room at all, and through the caller: a whole sweep must
        // leave the count alone, because the ledge keeps what is refused.
        c.cap[out] = c.h[out];
        let before_sweep = grains_held(&c);
        c.topple(false);
        assert_eq!(
            grains_held(&c),
            before_sweep,
            "grains went over the edge and vanished"
        );
        assert_eq!(c.total, on_the_ground(&c), "`total` outlived its grains");
    }

    /// `total` is exactly what is on the ground, and it is the only thing the
    /// drain looks at. A spill moves grains out of the maps and into the air,
    /// so it has to say so — and what the air and the floor both refuse has to
    /// stay on the ledge, not evaporate: grains dropped without telling `total`
    /// read as a heap fuller than it is, and the spawner stops for good.
    #[test]
    fn total_is_exactly_the_grains_on_the_ground() {
        let p = panel();
        for full_sky in [false, true] {
            let mut c = Confetti::new(&p, 15);
            let mut buf = vec![0u32; p.buf_len()];
            overload(&mut c, full_sky);
            let shed = on_the_ground(&c);
            for n in 0..2000 {
                saver::frame(&mut c, &mut buf, &p);
                assert_eq!(
                    c.total,
                    on_the_ground(&c),
                    "frame {n} (full sky {full_sky}): `total` and the maps disagree"
                );
            }
            // Non-vacuous: the ledges really did shed over these frames, so
            // the spill path ran rather than the invariant holding at rest.
            assert!(
                on_the_ground(&c) != shed,
                "full sky {full_sky}: nothing ever spilled"
            );
        }
    }

    /// A shelf catches what falls ONTO it, not what is already under it. The
    /// cheap test — is the piece's foot below the shelf surface — is true for
    /// everything below the ledge as well, and teleports a piece that drifted
    /// underneath up onto the top.
    #[test]
    fn a_piece_under_a_ledge_falls_past_it() {
        let p = panel();
        let mut c = Confetti::new(&p, 15);
        let sv = c.shelves[0];
        let mid = (sv.c0 + sv.c1) / 2;
        c.pieces.clear();
        c.pieces.push(Piece {
            x: ((mid * c.cell_w) as i32) << FP,
            // One row under the ledge, falling straight down.
            y: (((c.rows - sv.row) * c.cell_h) as i32) << FP,
            vy: c.fall,
            phase: 0,
            sway: 0,
            col: 1,
            load: c.deposit,
            at: u32::MAX,
        });
        for _ in 0..400 {
            c.fall_step();
        }
        assert_eq!(c.sh[mid], 0, "a piece under the ledge landed on top of it");
        assert!(c.h[mid] > 0, "the piece never reached the floor");
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
            c.h[i] = next_rand(&mut rng) % (c.cap[i] + 1);
            c.sh[i] = next_rand(&mut rng) % (c.scap[i] + 1);
        }
        // Writing the maps behind the saver's back breaks the one invariant it
        // keeps for itself — `total` is what is on the ground — and a spill
        // subtracts from it.
        c.total = on_the_ground(&c);
        // Spilling off a ledge moves grains BETWEEN the two maps, so counting
        // only the floor would call a lost shelf grain conserved.
        let before = grains_held(&c);
        assert!(c.sh.iter().any(|&v| v > 0), "no shelf pile to conserve");
        for k in 0..200 {
            c.topple(k & 1 == 1);
            assert_eq!(grains_held(&c), before, "sweep {k} changed the grain count");
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
