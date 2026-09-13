//! Flying toasters in BLOCK characters — the same 1989 scene as `toasters`,
//! drawn as chunky pixel art instead of as line art.
//!
//! # What is the same, and why
//!
//! Everything about the BEHAVIOUR, deliberately: down-and-left on one fixed
//! 2.5:1 diagonal, every object on one shared velocity, a four-position wing
//! walked as a six-step ping-pong, toast mixed into the flock at a quarter. All
//! four of those are argued in `toasters.rs` and none of them is a rendering
//! decision, so re-deciding them here would be a second opinion on a settled
//! question. This module is a different BRUSH, not a different homage.
//!
//! # What is different
//!
//! * **Filled regions, not strokes.** `toasters` draws its toaster with `/`,
//!   `|` and `=`, which means the olive chassis can only ever be the colour of
//!   an outline. Here the top face and the whole turned-away side are solid
//!   olive, the slots are punched out of it, and the chrome is a filled panel —
//!   which is what the sprite sheet quantises to and what the line-art version
//!   could not show. See `art::TOASTER_INK`.
//! * **Half the cell, twice the sprite.** 8x16 by default against the line-art
//!   version's 16x32, so a half block is a square 8x8 pixel and the toaster is
//!   30x7 cells rather than 14x4 — the same size on the panel, at twice the
//!   resolution. `TOASTER2_CELL_W` / `_H` are the knob.
//! * **One model.** The original flew one toaster; `toasters` flies four as an
//!   acknowledged departure. Drawing four at this resolution buys variety the
//!   flap and the four slices already supply, so this one is the original's.
//!
//! # Per-frame cost
//!
//! Proportional to the OBJECTS and never to the grid: the scene lives in the
//! grid's own `cur` buffer between frames, each object clears the rectangle it
//! last stamped and stamps a new one, and `flush_sparse` blits exactly the
//! cells named in `dirty`. There is no `fill` walking 16k cells and no scan for
//! changes — which matters more here than it did at 16x32, because halving the
//! cell quadrupled the grid.
//!
//! It is not literally O(objects), and the one place a reader will come looking
//! for that is this paragraph. `dirty` is sorted every frame, which is
//! `k log k` in the cells the objects touched — 4237 `u32` at the default
//! density and 1920x1080, deduping to 2701 blits, and the largest single cost
//! in the renderer. `k` is proportional to the object count and not to the
//! panel, so the shape of the claim holds; the sort is not free, and the
//! comment at the call site says what it buys.
//!
//! The SHADOW-TO-HARDWARE copy is not sparse, and saying otherwise would be a
//! lie a future reader acts on: damage is whole scanlines merged into at most
//! MAX_RUNS runs, so a dozen sprites at a dozen heights smear across a good
//! part of the panel per frame. Still far under `ascii` and `matrix`, which
//! repaint 100% of it every frame.

use crate::font;
use crate::grid::{Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, next_rand};

mod art;

use art::{PAL, TOASTER_CELLS, TOASTER_H, TOASTER_W, TOAST_CELLS, TOAST_H, TOAST_W};

/// Ping-pong over the four wing positions — both halves of the beat, which is
/// what the original's cycled frames looked like in motion. Walking 0,1,2,3 and
/// snapping back draws only the downstroke.
const FLAP: [u8; 6] = [0, 1, 2, 3, 2, 1];

/// The diagonal: 5 across for every 2 down. Taken from the After Dark 4.0
/// binary's (-60, +24) per loop, not from the 45 degrees every web recreation
/// inherited from one CSS demo. `TOASTER2_SPEED` scales both components
/// together, so the angle is a constant of the homage and only the pace is a
/// knob. The long argument is in `toasters.rs`.
const RUN: i32 = 5;
const RISE: i32 = 2;

/// Sub-pixel bits in a position, so the slope survives a step of under six
/// pixels a frame instead of being rounded away.
const SUB: i32 = 8;

/// What a flock member is. Everything moves in lockstep, so an object carries
/// no velocity of its own — only where it is, what it is, and where it is in
/// the wing cycle.
#[derive(Clone, Copy)]
struct Obj {
    /// Top-left of the sprite, in pixels shifted left by `SUB`. Pixels rather
    /// than cells so the diagonal is the panel's and not the cell grid's
    /// aspect ratio.
    x: i32,
    y: i32,
    /// 0 = toaster; 1..=4 = a slice at doneness 0..=3.
    kind: u8,
    /// Offset into `FLAP`, so the flock is not one synchronised wing.
    phase: u8,
    /// Cell coordinates of the rectangle this object last stamped, which is the
    /// rectangle it has to clear next frame.
    drawn: (i32, i32),
}

impl Obj {
    /// A slice is a different SIZE from a toaster, so this is the only answer
    /// to "how big is it" anywhere: the clear pass, the stamp and the despawn
    /// check all go through here. A shared constant would leave the toaster
    /// trailing its right-hand columns across the panel.
    fn size(&self) -> (usize, usize) {
        if self.kind == 0 {
            (TOASTER_W, TOASTER_H)
        } else {
            (TOAST_W, TOAST_H)
        }
    }

    /// Which sprite this object shows right now: a wing frame for a toaster, a
    /// doneness level for a slice. This is the index the RENDERER uses, so a
    /// test that wants the art indexes with it rather than being handed a
    /// second value nothing draws from.
    fn sprite(&self, tick: u32, flap_div: u32) -> usize {
        match self.kind {
            0 => {
                let step = (tick / flap_div) as usize + self.phase as usize;
                FLAP[step % FLAP.len()] as usize
            }
            k => k as usize - 1,
        }
    }

    fn cells(&self, tick: u32, flap_div: u32) -> &'static [Cell] {
        let i = self.sprite(tick, flap_div);
        if self.kind == 0 {
            &TOASTER_CELLS[i]
        } else {
            &TOAST_CELLS[i]
        }
    }
}

pub struct Toasters2 {
    grid: Grid,
    cols: i32,
    rows: i32,
    cell_w: i32,
    cell_h: i32,
    objs: Vec<Obj>,
    /// Cells written this frame, for `flush_sparse`. Reserved once at
    /// construction and only ever cleared — a push past the reserve is an
    /// allocation in the render path, which is what the capacity test guards.
    dirty: Vec<u32>,
    step_x: i32,
    step_y: i32,
    flap_div: u32,
    tick: u32,
    rng: u32,
}

/// Clear or paint one sprite-sized rectangle of the grid, clipped, recording
/// every cell it wrote. `sprite` of `None` clears the whole rectangle; painting
/// SKIPS the blanks rather than clearing them, so sprites are transparent where
/// they overlap. Writing blanks instead punches the other sprite's art out —
/// visible in a dump as toasters eating each other.
///
/// Every write goes into `dirty`. A cell written and left out of it keeps its
/// old pixels on the panel forever, because `flush_sparse` blits that list and
/// nothing else.
fn stamp(
    grid: &mut Grid,
    dirty: &mut Vec<u32>,
    (cols, rows): (i32, i32),
    at: (i32, i32),
    size: (usize, usize),
    sprite: Option<&[Cell]>,
) {
    let (cx, cy) = at;
    for r in 0..size.1 as i32 {
        let y = cy + r;
        if y < 0 || y >= rows {
            continue;
        }
        for c in 0..size.0 as i32 {
            let x = cx + c;
            if x < 0 || x >= cols {
                continue;
            }
            let cell = match sprite {
                Some(cells) => {
                    let cell = cells[r as usize * size.0 + c as usize];
                    // On the GLYPH, not on the whole packed word: the invariant
                    // is "this cell draws nothing", and a cell with no shape but
                    // some colour index draws nothing while still comparing
                    // unequal to `Cell::CLEAR`.
                    if cell.glyph() == font::BLANK as usize {
                        continue;
                    }
                    cell
                }
                None => Cell::CLEAR,
            };
            let i = (y * cols + x) as usize;
            grid.set(i, cell);
            dirty.push(i as u32);
        }
    }
}

impl Toasters2 {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        // 8x16 is one glyph per cell, which is the point: a half block is then
        // a square 8x8 pixel. A bigger cell magnifies the art, which is
        // legitimate on a different panel and is why this is a knob.
        //
        // The floors are the GLYPH's size and not a taste: below it the blit's
        // `mask`/`rowmap` SKIP source columns and rows rather than scaling
        // them, so the art silently changes instead of shrinking. `▒` is
        // 0xAA/0x55 on alternating rows, and at `cell_w = 4` the mask samples
        // columns 0,2,4,6 — every bit of the 0xAA rows and none of the 0x55
        // ones, so the 50% shade degrades into solid bands, and a scorched
        // slice renders as a plain one.
        let cell_w = env_num(&["TOASTER2_CELL_W"], 8, font::GLYPH_W as i64, 64) as i32;
        let cell_h = env_num(&["TOASTER2_CELL_H"], 16, font::GLYPH_H as i64, 128) as i32;
        let grid = Grid::new(panel, cell_w as usize, cell_h as usize);
        let (cols, rows) = (grid.cols() as i32, grid.rows() as i32);

        // Objects per 1000 cells. 1, not `toasters`' 4, for the same flock:
        // quartering the cell area quadrupled the cell count, and the original
        // sized its crowd by SCREEN area — `screenArea / 480000 * 30`, about a
        // quarter of the screen under sprite. 16 of these on a 240x67 grid is
        // 24% of 1920x1080.
        let density = env_num(&["TOASTER2_DENSITY"], 1, 1, 60);
        let count = ((cols as i64 * rows as i64 * density) / 1000).clamp(1, 256) as usize;
        // Percent of the flock that is toast. The original holds roughly 3
        // toasters per slice.
        let toast_pct = env_num(&["TOASTER2_TOAST_PCT"], 25, 0, 100) as u32;
        // Pixels per second across, matching the 4.0 binary's drift scaled from
        // 640 wide to 1920.
        let speed = env_num(&["TOASTER2_SPEED"], 170, 8, 2000) as i32;
        // Wing frames per second. 15 is a 6-frame ping-pong in 0.4 s.
        let flap_fps = env_num(&["TOASTER2_FLAP_FPS"], 15, 1, 120) as u32;

        // One shared step, in RUN/RISE units, so the slope is exact and every
        // object moves by the identical vector — the lockstep is structural,
        // not a convention the spawner has to keep.
        let unit = ((speed << SUB) / fps.max(1) as i32 / RUN).max(1);

        let mut t = Self {
            grid,
            cols,
            rows,
            cell_w,
            cell_h,
            objs: Vec::with_capacity(count),
            // Worst case is every object clearing a full rectangle and then
            // painting one: two writes per cell of the biggest sprite.
            dirty: Vec::with_capacity(count * TOASTER_W * TOASTER_H * 2),
            step_x: -unit * RUN,
            step_y: unit * RISE,
            flap_div: (fps / flap_fps).max(1),
            tick: 0,
            rng: 0x1357_9bdf,
        };
        // The first `toasts` objects are food, the rest machines. Counted, not
        // rolled per object: a 25% coin flip over a flock of 16 lands on one
        // slice about one seed in twenty, and since nothing re-rolls `kind`,
        // that seed's sky has one slice in it FOREVER. The original counted
        // too — its spawner emitted food whenever the ratio passed 2.0.
        let toasts = (count * toast_pct as usize) / 100;
        for i in 0..count {
            let kind = if i < toasts {
                1 + (next_rand(&mut t.rng) % TOAST_CELLS.len() as u32) as u8
            } else {
                0
            };
            let mut o = Obj {
                x: 0,
                y: 0,
                kind,
                phase: (next_rand(&mut t.rng) % FLAP.len() as u32) as u8,
                drawn: (i32::MIN, i32::MIN),
            };
            // Frame 0 must already be a flock, not an empty screen filling up,
            // so the initial scatter is over the whole panel rather than over
            // the spawn edges.
            o.x = (next_rand(&mut t.rng) as i32).rem_euclid(cols * cell_w) << SUB;
            o.y = (next_rand(&mut t.rng) as i32).rem_euclid(rows * cell_h) << SUB;
            t.objs.push(o);
        }
        t
    }

    /// Put an object back on the leading edges — the top and the right, the two
    /// the flock enters through on a down-and-left path. Entry points snap to a
    /// cell boundary, as the original's fixed diagonal lanes did, and the
    /// choice of edge is weighted by edge length so the arrival rate per unit of
    /// edge is uniform instead of clumping in a corner.
    fn respawn(&mut self, i: usize) {
        let h = self.objs[i].size().1 as i32 * self.cell_h;
        let (pw, ph) = (self.cols * self.cell_w, self.rows * self.cell_h);
        let edge = next_rand(&mut self.rng) as i32;
        let lane = next_rand(&mut self.rng) as i32;
        if edge.rem_euclid(pw + ph) < pw {
            // Top edge, entering downward.
            self.objs[i].x = (lane.rem_euclid(self.cols) * self.cell_w) << SUB;
            self.objs[i].y = (-h) << SUB;
        } else {
            // Right edge, entering leftward.
            self.objs[i].x = pw << SUB;
            self.objs[i].y = (lane.rem_euclid(self.rows) * self.cell_h - h) << SUB;
        }
    }
}

impl Saver for Toasters2 {
    fn render(&mut self, s: &mut Surface<'_>) {
        self.dirty.clear();
        let bounds = (self.cols, self.rows);

        // Clear every footprint before drawing any of them: an object whose new
        // rectangle overlaps another's old one would otherwise erase a sprite
        // that had already been redrawn.
        for o in &self.objs {
            if o.drawn.0 != i32::MIN {
                stamp(
                    &mut self.grid,
                    &mut self.dirty,
                    bounds,
                    o.drawn,
                    o.size(),
                    None,
                );
            }
        }

        for i in 0..self.objs.len() {
            self.objs[i].x += self.step_x;
            self.objs[i].y += self.step_y;
            let (w, _) = self.objs[i].size();
            let off_left = self.objs[i].x + ((w as i32 * self.cell_w) << SUB) <= 0;
            let off_bottom = self.objs[i].y >= (self.rows * self.cell_h) << SUB;
            if off_left || off_bottom {
                self.respawn(i);
            }
        }

        let (tick, flap_div) = (self.tick, self.flap_div);
        for o in &mut self.objs {
            // div_euclid, not `/`: an object off the left edge has a negative x
            // and truncating division would round its cell towards zero, which
            // makes the sprite jump a column as it crosses x = 0.
            let at = (
                (o.x >> SUB).div_euclid(self.cell_w),
                (o.y >> SUB).div_euclid(self.cell_h),
            );
            stamp(
                &mut self.grid,
                &mut self.dirty,
                bounds,
                at,
                o.size(),
                Some(o.cells(tick, flap_div)),
            );
            o.drawn = at;
        }
        self.tick = self.tick.wrapping_add(1);

        // Sorted and deduped, in place, before the blit. Both halves earn
        // their keep: row-major order lets `Damage` merge each new mark into
        // the last run instead of opening a fresh one — out of order it
        // reported 2832 scanlines of a 1072-line grid, counting the same rows
        // over and over — and the dedup drops the second blit of every cell an
        // object cleared and then repainted, which is most of them.
        self.dirty.sort_unstable();
        self.dirty.dedup();
        self.grid.flush_sparse(s, &PAL, &self.dirty);
    }

    fn name(&self) -> &'static str {
        "toasters2"
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

    fn panel() -> Panel {
        Panel::new(1920, 1080, 1920)
    }

    /// The two details the module doc calls the tells. Motion must be strictly
    /// down AND left at the fixed RISE/RUN slope, and every object must move by
    /// the same vector on the same frame — no per-object speed.
    #[test]
    fn the_flock_moves_down_and_left_in_lockstep() {
        let t = Toasters2::new(&panel(), 30);
        assert!(t.step_x < 0, "must travel left");
        assert!(t.step_y > 0, "must travel down");
        assert_eq!(
            -t.step_x * RISE,
            t.step_y * RUN,
            "the slope is the homage, not a free parameter"
        );

        let mut t = Toasters2::new(&panel(), 30);
        let mut buf = vec![0u32; panel().buf_len()];
        let before: Vec<(i32, i32)> = t.objs.iter().map(|o| (o.x, o.y)).collect();
        saver::frame(&mut t, &mut buf, &panel());
        for (o, &(x0, y0)) in t.objs.iter().zip(before.iter()) {
            // A respawned object jumped, which is the one legitimate exception.
            if o.x > x0 {
                continue;
            }
            assert_eq!((o.x - x0, o.y - y0), (t.step_x, t.step_y));
        }
    }

    /// The wings have to beat, and beat in BOTH directions. A flap stuck on one
    /// frame, or walking 0,1,2,3 and snapping back, both pass a "does it
    /// animate" check and neither is the original.
    ///
    /// The walk is what is under test, not the art: whether the four positions
    /// are distinct pictures is `art::tests`.
    #[test]
    fn the_flap_runs_its_full_ping_pong() {
        let o = Obj {
            x: 0,
            y: 0,
            kind: 0,
            phase: 0,
            drawn: (i32::MIN, i32::MIN),
        };
        let walk: Vec<usize> = (0..FLAP.len() as u32).map(|t| o.sprite(t, 1)).collect();
        let mut positions = walk.clone();
        positions.sort_unstable();
        positions.dedup();
        assert_eq!(positions, vec![0, 1, 2, 3], "four positions, got {walk:?}");
        assert_eq!(walk[1], walk[FLAP.len() - 1], "the beat must ping-pong");
        // It has to come back to where it started, or the "ping-pong" is a
        // six-step walk that happens to visit four frames.
        assert_eq!(o.sprite(FLAP.len() as u32, 1), walk[0], "and it must close");

        // And it must be the WING that moves, not the body: the base row's
        // middle columns — inside the body, outside every wing — are identical
        // across all four frames.
        let body = |f: usize| {
            TOASTER_CELLS[f][(TOASTER_H - 1) * TOASTER_W + TOASTER_W / 3..][..TOASTER_W / 3]
                .to_vec()
        };
        for f in 1..4 {
            assert_eq!(body(f), body(0), "frame {f} moved the body");
        }

        // The divisor is what ties the flap to wall-clock time rather than to
        // the frame rate, so a slow panel must not flap slowly.
        assert_eq!(Toasters2::new(&panel(), 30).flap_div, 30 / 15);
    }

    /// The "screen went blank" bug class: pixels written but never reported.
    /// `Surface` makes that unrepresentable, so what this really guards is the
    /// inverse — that the saver draws through `Surface` at all, that frame 0
    /// covers the panel, and that a moving flock's damage tracks it.
    #[test]
    fn damage_covers_every_changed_scanline() {
        let p = panel();
        let mut t = Toasters2::new(&p, 30);
        let mut buf = vec![0u32; p.buf_len()];
        let mut prev = buf.clone();

        let d = saver::frame(&mut t, &mut buf, &p);
        // The WHOLE panel, not just the whole cells: the strip below the last
        // cell row is painted and reported too, or it shows whatever simpledrm's
        // shadow buffer held (that was a visible line on the real panel).
        assert_eq!(d.rows(), p.h, "frame 0 must paint the whole panel");

        let (mut worst, mut moved) = (0, 0);
        for n in 1..40 {
            prev.copy_from_slice(&buf);
            let d = saver::frame(&mut t, &mut buf, &p);
            let mut changed = 0;
            for y in 0..p.h {
                if prev[y * p.w..][..p.w] == buf[y * p.w..][..p.w] {
                    continue;
                }
                changed += 1;
                assert!(
                    d.runs()
                        .iter()
                        .any(|&(a, b)| y as u16 >= a && (y as u16) < b),
                    "frame {n}: scanline {y} changed but was not reported"
                );
            }
            moved += usize::from(changed > 0);
            worst = worst.max(d.rows());
        }
        // Over the window, not per frame: the sub-pixel step means cell
        // positions do not advance every frame, so a per-frame assert is flaky
        // on correct code at a different speed or density.
        assert!(moved > 30, "a flying flock must change pixels ({moved}/39)");

        // Against the GRID's height, not the panel's: the grid never owns the
        // bottom `h % cell_h` strip, so `worst < p.h` was true for any
        // implementation — including one repainting everything every frame.
        let all = t.grid.rows() * t.cell_h as usize;
        assert!(worst < all, "damaged every scanline ({worst} of {all})");
    }

    /// The failure mode a hand-rolled double-buffer actually has: a sprite
    /// stamped but not erased leaves a trail, and every other test here passes
    /// while it happens — damage is still reported, pixels still change, and
    /// the count of lit cells SATURATES rather than climbing, so a ratio bound
    /// waves it straight through.
    ///
    /// What holds exactly, every frame, is that the scene is the flock and
    /// nothing else: the union of what the objects stamped, no cell more and no
    /// cell less. A single trailing column fails it.
    ///
    /// It also checks the other half of the `flush_sparse` bargain — that
    /// `dirty` named every cell the saver wrote. A cell written and left out
    /// keeps its old pixels forever, and the scene comparison cannot see that
    /// because the scene is right; only `cur` against `prev` can.
    #[test]
    fn a_sprite_leaves_no_trail() {
        let p = panel();
        let mut t = Toasters2::new(&p, 30);
        let mut buf = vec![0u32; p.buf_len()];

        for n in 0..400 {
            saver::frame(&mut t, &mut buf, &p);
            // `render` has already bumped the tick, so the sprites on screen
            // are the ones the PREVIOUS tick chose.
            let tick = t.tick.wrapping_sub(1);
            let mut want = vec![false; (t.cols * t.rows) as usize];
            for o in &t.objs {
                let (w, h) = o.size();
                let cells = o.cells(tick, t.flap_div);
                for r in 0..h as i32 {
                    for c in 0..w as i32 {
                        let (x, y) = (o.drawn.0 + c, o.drawn.1 + r);
                        if x < 0 || x >= t.cols || y < 0 || y >= t.rows {
                            continue;
                        }
                        if cells[r as usize * w + c as usize].glyph() != font::BLANK as usize {
                            want[(y * t.cols + x) as usize] = true;
                        }
                    }
                }
            }
            let stale = (0..want.len())
                .find(|&i| want[i] != (t.grid.cell(i).glyph() != font::BLANK as usize));
            if let Some(i) = stale {
                let (x, y) = (i as i32 % t.cols, i as i32 / t.cols);
                panic!(
                    "frame {n}: cell ({x},{y}) is {} but the flock says {}",
                    if want[i] { "blank" } else { "lit" },
                    if want[i] { "lit" } else { "blank" }
                );
            }
            assert_eq!(
                t.grid.cells(),
                (0..want.len()).map(|i| t.grid.cell(i)).collect::<Vec<_>>(),
                "frame {n}: a cell was written but left out of `dirty`"
            );
        }
    }

    /// The flock's mix, which is counted and not rolled: a per-object coin flip
    /// put ONE slice in a sixteen-object sky at this seed, and since nothing
    /// re-rolls `kind` it stayed that way for the life of the process.
    #[test]
    fn a_quarter_of_the_flock_is_toast() {
        let t = Toasters2::new(&panel(), 30);
        let toast = t.objs.iter().filter(|o| o.kind != 0).count();
        let n = t.objs.len();
        assert_eq!(toast * 100 / n, 25, "{toast} slices in {n}");
        assert!(t.objs.iter().any(|o| o.kind == 0), "and some toasters");
    }

    /// `dirty` is reserved once and pushed to per cell; a push past the reserve
    /// is an allocation in the render path, which is the one cost this design
    /// exists to avoid.
    #[test]
    fn the_render_path_never_allocates() {
        let p = panel();
        let mut t = Toasters2::new(&p, 30);
        let mut buf = vec![0u32; p.buf_len()];
        let reserved = t.dirty.capacity();
        let mut worst = 0;
        for _ in 0..200 {
            saver::frame(&mut t, &mut buf, &p);
            worst = worst.max(t.dirty.len());
            assert_eq!(
                t.dirty.capacity(),
                reserved,
                "`dirty` grew past its reserve"
            );
            // Sorted AND unique, pinned together because they are one property
            // in practice: strictly ascending is what lets `Damage` merge each
            // mark into the last run instead of opening a fresh one. Duplicates
            // are legal for `flush_sparse` and merely blit twice, so nothing
            // else here notices the `dedup` going missing.
            assert!(
                t.dirty.windows(2).all(|w| w[0] < w[1]),
                "`dirty` is not strictly ascending"
            );
        }
        assert!(worst > 0, "`dirty` was never used");
        // And the whole point of the sparse path: a frame touches a small
        // fraction of the grid, not all of it.
        let cells = (t.cols * t.rows) as usize;
        assert!(worst < cells / 2, "{worst} writes against {cells} cells");
    }
}
