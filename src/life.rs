//! Conway's Life on a torus, with an injector that keeps it from dying.
//!
//! # The problem this saver actually has to solve
//!
//! B3/S23 on a random soup is a beautiful two hundred generations and then a
//! scattering of blocks, beehives and blinkers that never changes again. As a
//! screensaver that is a still image with a twitch in it. Measured on this
//! board: a 240x135 soup falls from 49 changes per thousand cells per
//! generation at generation 100 to 9 by generation 2000, and sits there
//! forever — a torus keeps a few gliders circulating, which is why it does not
//! reach exactly zero and why "is the population non-zero" is not a test of
//! anything. Nothing about the rules fixes that; the rules are the whole point
//! and must stay exact, so the fix has to be outside them.
//!
//! # The fix: churn, patience, meteor
//!
//! `step` already knows how many cells were born or died this generation,
//! because it visits every cell anyway. That ONE number catches all three ways
//! a Life board gets boring, where the usual answers each catch one:
//!
//! * still lifes — churn 0
//! * short oscillators — churn a few cells per blinker
//! * a nearly-empty board with one glider crossing it — churn 10
//!
//! A rolling board hash in a ring buffer (the textbook stagnation detector)
//! sees the first two and is blind to the third: a glider makes every hash
//! distinct forever while the panel is black. Population alone sees the third
//! and is blind to the first two. Churn sees all three and costs an increment.
//!
//! So: when churn stays under `LIFE_QUIET` per mille of the board for
//! `LIFE_PATIENCE` consecutive generations, drop a meteor — one disc of fresh
//! soup at a random spot, the same `meteor` that seeded the board. A meteor
//! landing in a field of still lifes restarts the chaos locally and the front
//! spreads back across the board; a meteor landing in already-busy space is
//! never asked for, because busy space is not quiet. The board therefore never
//! reaches a state it cannot leave, and it is never reset either — a full
//! reseed is a jump cut, and the one thing a screensaver must not do is blink.
//!
//! At the default 30 per mille that settles into an equilibrium: 31 changes
//! per thousand cells per generation and 52 cells per thousand alive, held flat
//! from generation 600 out to 20000, on both panel shapes, at the cost of one
//! meteor every hundred generations or so — ten seconds at the default rate,
//! which is a thing you notice arriving rather than a strobe.
//!
//! Toroidal edges, not a dead border. A dead border is a permanent absorber:
//! gliders that reach it die, and after an hour every glider has, so the
//! interesting long-range traffic is gone and only the middle is alive. On a
//! torus a glider that leaves the right edge comes back on the left and
//! eventually collides with something, which is the interesting event.
//!
//! # Colour
//!
//! A cell's palette index is its AGE: white-hot at birth, cooling through amber
//! and violet to a settled blue after eight generations. Dead cells hold a
//! dark-red ash that fades over `LIFE_FADE` generations. Both are free — the
//! step visits every cell regardless — and they turn the boundary between a
//! growing front and the debris it leaves into a visible gradient, so even a
//! slow generation rate reads as motion rather than as a blinking bitmap.
//!
//! # Damage model: full repaint (Model A)
//!
//! `Grid::fill` + `Grid::flush`, never `flush_sparse`. A generation changes
//! cells all over the board and the age ramp re-colours the surrounding cells
//! too, so a dirty list would be most of the board on a busy frame and a chance
//! to under-report on a quiet one. `flush` derives damage from a u32 compare
//! per cell and structurally cannot under-report.
//!
//! Generations are paced by `LIFE_GPS`, independent of `SAVER_FPS`: Life at 30
//! generations a second is unreadable, and dropping the frame rate to fix that
//! would make every other saver's rotation slot stutter. A frame with no
//! generation in it re-fills the grid from unchanged state, so `flush` reports
//! empty damage and the frame costs one u32 compare per cell.
//!
//! # Environment
//!
//! * `LIFE_CELL_W` / `LIFE_CELL_H` — cell in px, 4..=64 (default 8, 8)
//! * `LIFE_GPS` — generations per SECOND, 1..=60 (default 10)
//! * `LIFE_DENSITY` — percent of cells alive inside fresh soup, 5..=80 (default 38)
//! * `LIFE_SEEDS` — soup discs dropped at startup, 1..=64 (default 12)
//! * `LIFE_QUIET` — churn per MILLE of the board below which a generation
//!   counts as quiet, 1..=500 (default 30)
//! * `LIFE_PATIENCE` — consecutive quiet generations before a meteor,
//!   1..=600 (default 12)
//! * `LIFE_FADE` — generations of ash a dead cell leaves, 0..=6 (default 5)

use crate::font;
use crate::grid::{bake, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, next_rand};

/// Age buckets for a live cell. Eight is where the ramp below stops showing a
/// step between adjacent generations at an 8x8 cell; the last bucket is
/// saturating, so a still life settles on one colour instead of cycling.
const AGES: usize = 8;

/// Ash steps a dead cell fades through. Six, because `LIFE_FADE` indexes this
/// ramp directly and a seventh step is already black at this cell size.
const ASHES: usize = 6;

/// Index 0 is dead-and-cold. Then the age ramp, then the ash ramp.
const PAL_LEN: usize = 1 + AGES + ASHES;

/// White-hot at birth cooling to a settled blue, then ash: a dark red that
/// fades out. Two ranges rather than one ramp, so a live cell can never borrow
/// an ash colour by drifting one index.
#[rustfmt::skip]
const RGB: [[u8; 3]; PAL_LEN] = [
    [0x00, 0x00, 0x00],
    [0xFF, 0xFF, 0xF0], [0xFF, 0xEC, 0xAA], [0xFF, 0xC8, 0x64], [0xFF, 0x96, 0x46],
    [0xE6, 0x69, 0x5A], [0xAA, 0x50, 0x8C], [0x6E, 0x4B, 0xB4], [0x46, 0x5A, 0xC8],
    [0x5A, 0x1E, 0x2D], [0x46, 0x18, 0x24], [0x34, 0x12, 0x1C],
    [0x24, 0x0D, 0x14], [0x16, 0x08, 0x0D], [0x0C, 0x05, 0x08],
];

const PAL: [u32; PAL_LEN] = bake(&RGB);

pub struct Life {
    grid: Grid,
    cols: usize,
    rows: usize,
    /// Liveness, one byte per cell, 0 or 1 so a neighbour count is eight adds
    /// with no branch. Double-buffered with `next` and SWAPPED, never rebuilt:
    /// the rules read the whole previous generation, so writing in place would
    /// feed a cell its own successor.
    cur: Vec<u8>,
    next: Vec<u8>,
    /// Generations this cell has been alive, saturating at `AGES - 1`.
    age: Vec<u8>,
    /// Generations of afterglow left on a cell that died. Zero is cold.
    ash: Vec<u8>,
    /// Wrapped neighbour indices: column -1/+1, and row -1/+1 already
    /// multiplied by `cols`. These replace four `%` per cell per generation —
    /// the modulo is the whole cost of a toroidal edge, and it is avoidable.
    xm: Vec<usize>,
    xp: Vec<usize>,
    ym: Vec<usize>,
    yp: Vec<usize>,
    rng: u32,
    /// `acc += gps` per frame, one generation per `fps` accumulated. Integer,
    /// so the generation rate cannot drift over the weeks this runs.
    fps: u32,
    gps: u32,
    acc: u32,
    /// Percent of cells a fresh soup disc lights.
    density: u32,
    /// Radius of a soup disc in cells, from the geometry rather than a knob:
    /// the panel is 1920x1080 today and 1280x400 tomorrow, and a radius that
    /// suits one is a dot or a wall on the other.
    blob: usize,
    fade: u8,
    /// Births + deaths below this in a generation is "quiet".
    quiet: u32,
    patience: u32,
    /// Consecutive quiet generations so far.
    still: u32,
    /// Cells that changed in the last generation. Read by the liveness tests;
    /// nothing in the render path reads it back.
    churn: u32,
}

impl Life {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        let cell_w = env_num(&["LIFE_CELL_W"], 8, 4, 64) as usize;
        let cell_h = env_num(&["LIFE_CELL_H"], 8, 4, 64) as usize;
        let gps = env_num(&["LIFE_GPS"], 10, 1, 60) as u32;
        let density = env_num(&["LIFE_DENSITY"], 38, 5, 80) as u32;
        let seeds = env_num(&["LIFE_SEEDS"], 12, 1, 64) as usize;
        let quiet_permille = env_num(&["LIFE_QUIET"], 30, 1, 500) as u32;
        let patience = env_num(&["LIFE_PATIENCE"], 12, 1, 600) as u32;
        let fade = env_num(&["LIFE_FADE"], 5, 0, ASHES as i64) as u8;

        let grid = Grid::new(panel, cell_w, cell_h);
        let (cols, rows) = (grid.cols(), grid.rows());
        let cells = cols * rows;

        // Geometric mean over ten: 18 cells on a 240x135 board, 9 on a 160x50
        // one. A disc that is a fixed fraction of the WIDTH spans the whole
        // height of the short panel and looks like a bar.
        let blob = ((((cells as f32).sqrt()) / 10.0) as usize).max(2);

        let mut me = Self {
            grid,
            cols,
            rows,
            cur: vec![0; cells],
            next: vec![0; cells],
            age: vec![0; cells],
            ash: vec![0; cells],
            xm: (0..cols).map(|x| (x + cols - 1) % cols).collect(),
            xp: (0..cols).map(|x| (x + 1) % cols).collect(),
            ym: (0..rows).map(|y| ((y + rows - 1) % rows) * cols).collect(),
            yp: (0..rows).map(|y| ((y + 1) % rows) * cols).collect(),
            // Seeded off the clock, the same trick sakura grows its tree from,
            // so a restart is not the same board again.
            rng: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos() ^ d.as_secs() as u32)
                .unwrap_or(0x5EED_11FE),
            fps: fps.max(1),
            gps,
            acc: 0,
            density,
            blob,
            fade,
            quiet: (cells as u32 * quiet_permille / 1000).max(1),
            patience,
            still: 0,
            churn: 0,
        };

        for _ in 0..seeds {
            me.meteor();
        }
        me
    }

    /// A disc of fresh soup at a random spot. Startup seeding and the
    /// stagnation break are the same event, so they are the same code: one
    /// thing to get right, and the panel never shows a transition it was not
    /// already showing at second zero.
    fn meteor(&mut self) {
        let cx = next_rand(&mut self.rng) as usize % self.cols;
        let cy = next_rand(&mut self.rng) as usize % self.rows;
        let r = self.blob as isize;
        for dy in -r..=r {
            let y = (cy as isize + dy).rem_euclid(self.rows as isize) as usize * self.cols;
            for dx in -r..=r {
                if dx * dx + dy * dy > r * r {
                    continue;
                }
                let i = y + (cx as isize + dx).rem_euclid(self.cols as isize) as usize;
                let live = next_rand(&mut self.rng) % 100 < self.density;
                self.cur[i] = live as u8;
                self.age[i] = 0;
                // Not `fade`: a meteor is an arrival, and lighting a ring of
                // ash around every dead cell in the disc would draw the disc's
                // outline on the panel.
                self.ash[i] = 0;
            }
        }
    }

    /// One generation of B3/S23, plus the age/ash bookkeeping and the churn
    /// count, in a single pass. Ages and ash are written here and read by
    /// nothing the rules use, so they are safe to update in place while `next`
    /// is being built from `cur`.
    fn step(&mut self) {
        let (cur, next, age, ash) = (
            &self.cur[..],
            &mut self.next[..],
            &mut self.age[..],
            &mut self.ash[..],
        );
        let (cols, rows) = (self.cols, self.rows);
        let (xm, xp, ym, yp) = (&self.xm[..], &self.xp[..], &self.ym[..], &self.yp[..]);
        let mut churn = 0u32;

        for y in 0..rows {
            let (up, mid, down) = (ym[y], y * cols, yp[y]);
            for x in 0..cols {
                let (l, r) = (xm[x], xp[x]);
                // Eight reads, no branch, no modulo. The classic trick of
                // carrying column sums between cells is faster and is not
                // needed: 32400 cells at 10 generations a second is 2.6M adds
                // a second, well inside the budget, and this version is the
                // rules written out where a reviewer can count them.
                let n = cur[up + l]
                    + cur[up + x]
                    + cur[up + r]
                    + cur[mid + l]
                    + cur[mid + r]
                    + cur[down + l]
                    + cur[down + x]
                    + cur[down + r];
                let i = mid + x;
                let was = cur[i] != 0;
                let now = if was { n == 2 || n == 3 } else { n == 3 };
                next[i] = now as u8;
                match (was, now) {
                    (true, true) => age[i] = (age[i] + 1).min(AGES as u8 - 1),
                    (false, true) => {
                        age[i] = 0;
                        ash[i] = 0;
                        churn += 1;
                    }
                    (true, false) => {
                        ash[i] = self.fade;
                        churn += 1;
                    }
                    (false, false) => ash[i] = ash[i].saturating_sub(1),
                }
            }
        }

        std::mem::swap(&mut self.cur, &mut self.next);
        self.churn = churn;

        // The whole anti-stagnation mechanism, in four lines. See the module
        // doc for why churn and not a board hash.
        self.still = if self.churn < self.quiet {
            self.still + 1
        } else {
            0
        };
        if self.still >= self.patience {
            self.still = 0;
            self.meteor();
        }
    }
}

impl Saver for Life {
    fn render(&mut self, s: &mut Surface<'_>) {
        // A `while`, not an `if`: LIFE_GPS may exceed SAVER_FPS, and a
        // generation rate that silently clamps to the frame rate is a knob
        // that lies.
        self.acc += self.gps;
        while self.acc >= self.fps {
            self.acc -= self.fps;
            self.step();
        }

        let (grid, cur, age, ash, cols, fade) = (
            &mut self.grid,
            &self.cur[..],
            &self.age[..],
            &self.ash[..],
            self.cols,
            self.fade,
        );
        grid.fill(|cx, cy| {
            let i = cy * cols + cx;
            if cur[i] != 0 {
                return Cell::new(font::SOLID, 1 + age[i] as u16);
            }
            match ash[i] {
                0 => Cell::CLEAR,
                // `fade - a` counts UP the ash ramp as the cell cools, so the
                // freshest ash is always the brightest entry whatever LIFE_FADE
                // is set to.
                a => Cell::new(font::SOLID, (1 + AGES) as u16 + (fade - a) as u16),
            }
        });
        grid.flush(s, &PAL);
    }

    fn name(&self) -> &'static str {
        "life"
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

    /// 1080 is not a multiple of 8 at every cell size, but it is at 8 — so the
    /// margin test needs a height that is not. 1050 leaves two scanlines below
    /// the last cell row, which is the strip `paint_margins` exists for.
    fn panel() -> Panel {
        Panel::new(1920, 1080, 1920)
    }

    /// A board with exactly the cells named alive, nothing else, and an
    /// injector that will never fire during the test.
    fn board(cols: usize, rows: usize, live: &[(usize, usize)]) -> Life {
        let mut l = Life::new(&Panel::new(cols * 8, rows * 8, cols * 8), 15);
        assert_eq!((l.cols, l.rows), (cols, rows));
        l.cur.fill(0);
        l.age.fill(0);
        l.ash.fill(0);
        // Patience past any generation count a rules test runs, so a meteor
        // can never be the reason a shape moved.
        l.patience = u32::MAX;
        for &(x, y) in live {
            l.cur[y * cols + x] = 1;
        }
        l
    }

    fn live_cells(l: &Life) -> Vec<(usize, usize)> {
        (0..l.cur.len())
            .filter(|&i| l.cur[i] != 0)
            .map(|i| (i % l.cols, i / l.cols))
            .collect()
    }

    /// T1. B3/S23 is exact and a wrong neighbour count is a plausible-looking
    /// bug, so the three canonical shapes are asserted directly rather than
    /// through anything the panel shows.
    ///
    /// A blinker is period 2: horizontal, vertical, horizontal.
    #[test]
    fn a_blinker_oscillates_with_period_two() {
        let mut l = board(16, 16, &[(4, 5), (5, 5), (6, 5)]);
        l.step();
        assert_eq!(live_cells(&l), [(5, 4), (5, 5), (5, 6)], "gen 1 vertical");
        // Two ends died and two cells were born: churn counts BOTH, which is
        // what makes it a stagnation signal rather than a growth signal.
        assert_eq!(l.churn, 4, "gen 1 churn");
        l.step();
        assert_eq!(live_cells(&l), [(4, 5), (5, 5), (6, 5)], "gen 2 back");
    }

    /// T2. A block is a still life: S23 keeps all four (each has exactly 3
    /// neighbours) and B3 must not light any of the twelve cells around it,
    /// which each see exactly 2.
    #[test]
    fn a_block_is_still() {
        let want = [(4, 4), (5, 4), (4, 5), (5, 5)];
        let mut l = board(16, 16, &want);
        for g in 0..50 {
            l.step();
            assert_eq!(live_cells(&l), want, "gen {g}");
            assert_eq!(l.churn, 0, "gen {g}: a still life churned");
        }
    }

    /// T3. A glider is the shape that catches an asymmetric neighbour bug: it
    /// must come back to ITS OWN cells translated by exactly (1, 1) after four
    /// generations, and the three intermediate phases must not be it.
    #[test]
    fn a_glider_walks_one_cell_diagonally_every_four_generations() {
        let g0 = [(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)];
        let mut l = board(32, 32, &g0);
        for period in 1..=6 {
            for phase in 1..4 {
                l.step();
                assert_ne!(
                    live_cells(&l),
                    g0.iter()
                        .map(|&(x, y)| (x + period, y + period))
                        .collect::<Vec<_>>(),
                    "period {period} phase {phase}: arrived early"
                );
            }
            l.step();
            let want: Vec<_> = g0.iter().map(|&(x, y)| (x + period, y + period)).collect();
            assert_eq!(live_cells(&l), want, "after {} generations", period * 4);
        }
    }

    /// T4. Toroidal edges, asserted on both axes and on the corner at once: the
    /// four corner cells of the board are a 2x2 block ON A TORUS and must
    /// therefore be a still life. With a dead border they are four isolated
    /// cells and all die in one generation, so this test has exactly one
    /// passing implementation.
    #[test]
    fn the_board_wraps_at_every_edge() {
        let (w, h) = (24usize, 18usize);
        let corners = [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)];
        let mut l = board(w, h, &corners);
        for g in 0..20 {
            l.step();
            let mut got = live_cells(&l);
            got.sort_unstable();
            let mut want = corners;
            want.sort_unstable();
            assert_eq!(got, want, "gen {g}: the corner block did not wrap");
        }

        // And a glider crossing the right edge reappears on the left with its
        // shape intact, which the corner block alone does not prove.
        let start = (w - 3, 4);
        let g0 = [(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)];
        let mut l = board(
            w,
            h,
            &g0.map(|(x, y)| ((x + start.0) % w, (y + start.1) % h)),
        );
        for _ in 0..(4 * w) {
            l.step();
        }
        // w periods of four generations move it (w, w) — back to the same
        // column, w rows down.
        let want: Vec<_> = {
            let mut v: Vec<_> = g0
                .iter()
                .map(|&(x, y)| ((x + start.0 + w) % w, (y + start.1 + w) % h))
                .collect();
            v.sort_unstable_by_key(|&(x, y)| (y, x));
            v
        };
        assert_eq!(live_cells(&l), want, "the glider did not survive the seam");
    }

    /// T5. Frame 0 must cover the panel, INCLUDING the strip below the last
    /// cell row, and must cover it with the board rather than with a reported
    /// black rectangle.
    #[test]
    fn frame_zero_paints_the_whole_panel() {
        let p = Panel::new(1920, 1050, 1920);
        let mut l = Life::new(&p, 15);
        assert!(
            !p.h.is_multiple_of(l.grid.cell_h()),
            "test panel divides evenly"
        );
        let mut buf = vec![0u32; p.buf_len()];
        let d = saver::frame(&mut l, &mut buf, &p);
        assert_eq!(d.rows(), p.h, "frame 0 must paint the whole panel");
        let lit = buf.iter().filter(|&&px| px != 0).count();
        assert!(lit > 10_000, "frame 0 painted nothing ({lit} lit)");
    }

    /// T6. Every scanline whose pixels changed is inside a damage run —
    /// otherwise simpledrm scans out the previous frame there forever. The two
    /// companion assertions keep it from being a bound nothing approaches.
    #[test]
    fn damage_covers_every_changed_scanline() {
        let p = panel();
        let mut l = Life::new(&p, 15);
        let mut buf = vec![0u32; p.buf_len()];
        let mut prev = vec![0u32; p.buf_len()];
        let stride = p.buf_len() / p.h;
        saver::frame(&mut l, &mut buf, &p);

        let mut total = 0usize;
        const FRAMES: usize = 300;
        for n in 1..FRAMES {
            prev.copy_from_slice(&buf);
            let d = saver::frame(&mut l, &mut buf, &p);
            for y in 0..p.h {
                let row = y * stride..y * stride + p.w;
                if buf[row.clone()] != prev[row] {
                    assert!(
                        d.runs()
                            .iter()
                            .any(|&(a, b)| (a as usize..b as usize).contains(&y)),
                        "frame {n}: scanline {y} changed but was not reported"
                    );
                }
            }
            total += d.rows();
        }
        assert!(
            total > 0,
            "nothing moved: the coverage check proves nothing"
        );
    }

    /// T7. `render` owns no per-frame collection: every buffer is sized in
    /// `new`, `meteor` writes through by index, and `Grid::fill` writes in
    /// place. Length, capacity AND address, because a same-size `self.cur =
    /// vec![...]` in the frame path keeps the first two and is the exact bug.
    ///
    /// The shape check alone cannot see a scratch `Vec` allocated and dropped
    /// inside `render` — it is gone again by the time the buffers are compared
    /// — so the run is also bracketed by `testalloc`, the crate's one counting
    /// allocator. Both, because neither subsumes the other: the counter misses
    /// a same-size reallocation that reuses the block, the shapes miss a
    /// temporary.
    #[test]
    fn render_never_allocates() {
        let p = Panel::new(320, 200, 320);
        let mut l = Life::new(&p, 15);
        let mut buf = vec![0u32; p.buf_len()];
        let shape = |l: &Life| {
            let v = |b: &Vec<u8>| (b.len(), b.capacity(), b.as_ptr() as usize);
            let u = |b: &Vec<usize>| (b.len(), b.capacity(), b.as_ptr() as usize);
            // The board pair is compared as a SET: `step` ends in a
            // `mem::swap`, so the two addresses trade places every generation
            // and are the same two addresses forever. A `self.cur = vec![..]`
            // introduces a third, which is what this catches.
            let mut pair = [v(&l.cur), v(&l.next)];
            pair.sort_unstable();
            [
                pair[0],
                pair[1],
                v(&l.age),
                v(&l.ash),
                u(&l.xm),
                u(&l.xp),
                u(&l.ym),
                u(&l.yp),
            ]
        };
        let reserved = shape(&l);
        assert!(reserved[0].0 > 0, "nothing was reserved for the frame loop");
        let cells = l.grid.cells().len();
        // Meteors land during this run, so the injector's writes are covered
        // too — it is in the frame path and it is the only thing there that
        // touches a whole region at once.
        l.quiet = u32::MAX;
        l.patience = 1;
        let n = crate::testalloc::allocs_during(|| {
            for _ in 0..20_000 {
                saver::frame(&mut l, &mut buf, &p);
                assert_eq!(shape(&l), reserved, "the render path allocated");
                assert_eq!(l.grid.cells().len(), cells, "the grid was reallocated");
            }
        });
        assert_eq!(n, 0, "the render path made {n} allocations");
        assert!(
            l.cur.iter().any(|&c| c != 0),
            "20k frames drew nothing into the reserved buffers"
        );
    }

    /// T8. The stagnation break, on the worst case there is: a single block,
    /// which is a fixed point of the rules and churns zero forever. Nothing in
    /// B3/S23 can move it, so anything that happens is the injector.
    #[test]
    fn a_dead_board_is_brought_back() {
        let mut l = board(64, 64, &[(4, 4), (5, 4), (4, 5), (5, 5)]);
        l.patience = 5;
        l.quiet = 8;
        for g in 0..4 {
            l.step();
            assert_eq!(l.cur.iter().filter(|&&c| c != 0).count(), 4, "gen {g}");
        }
        // The fifth quiet generation trips it.
        l.step();
        assert!(
            l.cur.iter().filter(|&&c| c != 0).count() > 20,
            "the board stayed dead past LIFE_PATIENCE"
        );
    }

    /// T9. What this saver IS, and the reason it is not plain Life: the board
    /// must still be visibly moving after a long run, at BOTH panel shapes.
    ///
    /// Stated as churn per generation rather than as population, because a
    /// board can hold two thousand cells and be a car park of blocks. The
    /// floor is 15 changes per thousand cells, and it is not a bound nothing
    /// approaches in either direction: over eight seeded boards at each shape,
    /// the quietest hundred generations late in the run measure 29 and 27 per
    /// mille WITH the injector and 6 and 3 per mille with it disabled. Comment
    /// out the `meteor()` call in `step` and this test fails on both panels.
    #[test]
    fn it_never_dies_down() {
        for (w, h) in [(1920usize, 1080usize), (1280, 400)] {
            let p = Panel::new(w, h, w);
            let mut l = Life::new(&p, 15);
            let cells = (l.cols * l.rows) as u32;
            for _ in 0..3_000 {
                l.step();
            }
            // Late in the run, over a window long enough that one quiet
            // generation between two meteors cannot fail it.
            let mut worst = u32::MAX;
            let mut window = 0u32;
            for g in 0..1_000 {
                l.step();
                window += l.churn;
                if g % 100 == 99 {
                    worst = worst.min(window);
                    window = 0;
                }
            }
            let permille = worst * 1000 / 100 / cells;
            assert!(
                permille >= 15,
                "{w}x{h}: the quietest hundred generations averaged {permille} \
                 changes per thousand of {cells} cells — the board died down"
            );
        }
    }

    /// T11. The age and ash bookkeeping, which the rules tests cannot see
    /// because it changes only colour. Both halves are a mutation that
    /// survived everything above: leaving `age` alone on rebirth paints a
    /// reborn cell in the settled colour its previous life ended on, and never
    /// decrementing `ash` freezes the afterglow so every cell that has ever
    /// died stays lit forever.
    #[test]
    fn age_saturates_ash_cools_and_a_rebirth_starts_over() {
        // A lone cell dies with nothing to revive it, so its ash is the only
        // thing that can move.
        let mut l = board(16, 16, &[(2, 2)]);
        let i = 2 * 16 + 2;
        l.step();
        assert_eq!(l.ash[i], l.fade, "a fresh corpse holds a full fade");
        l.step();
        assert_eq!(l.ash[i], l.fade - 1, "the ash did not cool");

        // A blinker, with the centre cell aged to the top of the ramp and one
        // end aged part-way. The centre survives and must SATURATE; the end
        // dies and comes back two generations later and must start over.
        let mut l = board(16, 16, &[(4, 5), (5, 5), (6, 5)]);
        let (mid, end) = (5 * 16 + 5, 5 * 16 + 4);
        l.age[mid] = AGES as u8 - 1;
        l.age[end] = 6;
        l.step();
        assert_eq!(
            l.age[mid],
            AGES as u8 - 1,
            "age ran off the end of the ramp"
        );
        assert_eq!(l.ash[end], l.fade);
        l.step();
        assert_eq!(l.cur[end], 1, "the blinker did not come back");
        assert_eq!(l.age[end], 0, "a reborn cell kept its old age");
        assert_eq!(l.ash[end], 0, "a reborn cell kept its ash");
    }

    /// T10. The generation rate is a knob, not a synonym for the frame rate.
    /// `acc` back at zero after a second of frames is the exact accounting;
    /// a blinker's orientation is the independent witness that the generations
    /// actually happened, and it is the only observable the render path leaves
    /// behind without a counter nothing in production would read.
    #[test]
    fn generations_are_paced_independently_of_frames() {
        for (fps, gps) in [(30u32, 10u32), (15, 15), (15, 60), (60, 1)] {
            let mut l = board(16, 16, &[(4, 5), (5, 5), (6, 5)]);
            l.fps = fps;
            l.gps = gps;
            l.acc = 0;
            let p = Panel::new(128, 128, 128);
            let mut buf = vec![0u32; p.buf_len()];
            for _ in 0..fps {
                saver::frame(&mut l, &mut buf, &p);
            }
            assert_eq!(l.acc, 0, "fps={fps} gps={gps}: the accumulator drifted");
            // gps generations happened, so the blinker is vertical iff gps is
            // odd. Nothing but the pacing decides that.
            let want: Vec<(usize, usize)> = if gps % 2 == 1 {
                vec![(5, 4), (5, 5), (5, 6)]
            } else {
                vec![(4, 5), (5, 5), (6, 5)]
            };
            assert_eq!(live_cells(&l), want, "fps={fps} gps={gps}");
        }
    }
}
