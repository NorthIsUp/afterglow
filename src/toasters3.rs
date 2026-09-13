//! Flying toasters at braille resolution — the same flock as `toasters`, drawn
//! with U+2800..28FF instead of line art.
//!
//! # What is different, and what is deliberately not
//!
//! The BEHAVIOUR is `toasters`' and is not up for reinterpretation: everything
//! travels down-and-left on the After Dark 4.0 binary's 2.5:1 diagonal, every
//! object moves by one shared vector, the wings walk a four-position ping-pong,
//! and a quarter of the flock is toast. Those are the homage; see `toasters.rs`
//! for the evidence behind each one.
//!
//! What is different is RESOLUTION. A braille cell carries a 2x4 dot matrix, so
//! a 16x6-cell toaster is a 32x24 bitmap rather than 96 characters chosen for
//! their silhouette. That buys a slot that is an actual opening, a chrome edge
//! that bows, a dial, a lever, feet, and wings with barbed trailing edges.
//!
//! It does NOT buy colour. A `Cell` is one glyph and one palette index, so the
//! dots inside a cell are all the same colour and the palette regions are
//! exactly as coarse as `toasters`'. Detail is 8x; colour is 1x. The art is
//! drawn knowing that — every colour region is at least a cell wide.
//!
//! # One model, not four
//!
//! `toasters` flies four models because four line-art silhouettes are cheap and
//! a sky of one 14x4 shape is repetitive. Here the single object already carries
//! more information than all four of those together, and four hand-drawn 32x24
//! bitmaps would be four times the art for variety the detail is already
//! providing. One model, drawn properly.
//!
//! # Per-frame cost
//!
//! The CELL BLIT is O(objects), as in `toasters`: each object clears the
//! rectangle it last stamped and stamps a new one, and nothing walks the grid
//! per object. The sprite is bigger — 96 cells against 50..72 — so the default
//! density is lower to match: the original sized its flock by AREA under
//! sprite (~22% of the screen), which at 96 cells on a 120x33 grid is 9 objects,
//! not 15. Same crowd, bigger objects, and roughly the same cells touched.
//!
//! The SHADOW-TO-HARDWARE copy is not sparse, and the taller sprite is the part
//! that costs: damage is whole scanlines, and a 6-cell object spans 192 px
//! against the classic's 128. Measured medians are in the README table.

use crate::font;
use crate::grid::{Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, next_rand};

mod art;

use art::{FRAMES, PAL, TOASTER_H, TOASTER_W, TOAST_CELLS, TOAST_H, TOAST_W};

/// Ping-pong over the four wing positions — both halves of the beat, not just
/// the downstroke. Same walk as `toasters`, same reason.
const FLAP: [u8; 6] = [0, 1, 2, 3, 2, 1];

/// The diagonal: 5 across for every 2 down. From the After Dark 4.0 binary, not
/// from the 45 degrees the web recreations use. See `toasters.rs`.
const RUN: i32 = 5;
const RISE: i32 = 2;

/// Sub-pixel bits in a position, so the slope survives a step of under six
/// pixels a frame.
const SUB: i32 = 8;

/// A flock member. No velocity and no model: one shared vector, one toaster.
#[derive(Clone, Copy)]
struct Obj {
    /// Top-left of the sprite in pixels, shifted left by `SUB`.
    x: i32,
    y: i32,
    /// 0 = toaster; 1..=4 = a slice at doneness 0..=3.
    kind: u8,
    /// Offset into `FLAP`, so the flock is not one synchronised wing.
    phase: u8,
    /// Cell coordinates of the rectangle this object last stamped into
    /// `scene`, which is the rectangle it has to clear next frame.
    drawn: (i32, i32),
}

impl Obj {
    fn size(&self) -> (usize, usize) {
        if self.kind == 0 {
            (TOASTER_W, TOASTER_H)
        } else {
            (TOAST_W, TOAST_H)
        }
    }

    /// Which sprite this object shows right now. This is the index the RENDERER
    /// uses, so a test that wants the art indexes with it rather than being
    /// handed a second value nothing draws from.
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
            FRAMES[i]
        } else {
            &TOAST_CELLS[i]
        }
    }
}

pub struct Toasters3 {
    grid: Grid,
    cols: i32,
    rows: i32,
    cell_w: i32,
    cell_h: i32,
    objs: Vec<Obj>,
    /// The cell the panel is showing, kept between frames so an object can
    /// clear exactly what it drew. `Grid`'s own `cur`/`prev` cannot serve: it
    /// swaps them, and `fill` demands every cell each frame.
    scene: Vec<Cell>,
    step_x: i32,
    step_y: i32,
    flap_div: u32,
    tick: u32,
    rng: u32,
}

/// Clear or paint one sprite-sized rectangle of `scene`, clipped to the grid.
/// `sprite` of `None` clears the whole rectangle; painting SKIPS the cells with
/// no dots rather than clearing them, so sprites are transparent where they
/// overlap — a braille cell's blank pattern interns onto `font::BLANK`, which
/// is what makes that test the same one `toasters` makes.
fn stamp(
    scene: &mut [Cell],
    cols: i32,
    rows: i32,
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
                    if cell.glyph() == font::BLANK as usize {
                        continue;
                    }
                    cell
                }
                None => Cell::CLEAR,
            };
            scene[(y * cols + x) as usize] = cell;
        }
    }
}

impl Toasters3 {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        let cell_w = env_num(&["TOASTER3_CELL_W"], 16, 8, 64) as i32;
        let cell_h = env_num(&["TOASTER3_CELL_H"], 32, 8, 128) as i32;
        let grid = Grid::new(panel, cell_w as usize, cell_h as usize);
        let (cols, rows) = (grid.cols() as i32, grid.rows() as i32);

        // Objects per 1000 cells. 2, not `toasters`' 4, and for the same
        // derivation: the original put ~22% of the screen under sprite, and
        // this sprite is 96 cells where the classic is 56. Half the count,
        // twice the object, same sky.
        let density = env_num(&["TOASTER3_DENSITY"], 2, 1, 60);
        let count = ((cols as i64 * rows as i64 * density) / 1000).clamp(1, 256) as usize;
        let toast_pct = env_num(&["TOASTER3_TOAST_PCT"], 25, 0, 100) as u32;
        let speed = env_num(&["TOASTER3_SPEED"], 170, 8, 2000) as i32;
        let flap_fps = env_num(&["TOASTER3_FLAP_FPS"], 15, 1, 120) as u32;

        // One shared step in RUN/RISE units, so the slope is exact and the
        // lockstep is structural rather than a convention the spawner keeps.
        let unit = ((speed << SUB) / fps.max(1) as i32 / RUN).max(1);

        let mut t = Self {
            grid,
            cols,
            rows,
            cell_w,
            cell_h,
            objs: Vec::with_capacity(count),
            scene: vec![Cell::CLEAR; (cols * rows) as usize],
            step_x: -unit * RUN,
            step_y: unit * RISE,
            flap_div: (fps / flap_fps).max(1),
            tick: 0,
            rng: 0x2468_ace0,
        };
        for _ in 0..count {
            let kind = if next_rand(&mut t.rng) % 100 < toast_pct {
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
            // Frame 0 must already be a flock, not an empty screen filling up.
            o.x = (next_rand(&mut t.rng) as i32).rem_euclid(cols * cell_w) << SUB;
            o.y = (next_rand(&mut t.rng) as i32).rem_euclid(rows * cell_h) << SUB;
            t.objs.push(o);
        }
        t
    }

    /// Put an object back on the leading edges — the top and the right, the two
    /// the flock enters through on a down-and-left path — stepped along fixed
    /// diagonal lanes rather than scattered, and weighted by edge length so the
    /// arrival rate per unit of edge is uniform instead of clumping in a corner.
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

impl Saver for Toasters3 {
    fn render(&mut self, s: &mut Surface<'_>) {
        // Clear every footprint before drawing any of them: an object whose new
        // rectangle overlaps another's old one would otherwise erase a sprite
        // that had already been redrawn.
        for o in &self.objs {
            if o.drawn.0 != i32::MIN {
                stamp(
                    &mut self.scene,
                    self.cols,
                    self.rows,
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
                &mut self.scene,
                self.cols,
                self.rows,
                at,
                o.size(),
                Some(o.cells(tick, flap_div)),
            );
            o.drawn = at;
        }
        self.tick = self.tick.wrapping_add(1);

        // Split borrow: `fill` takes `&mut self.grid` while the closure reads
        // the scene, so the two cannot both go through `self`.
        let (grid, scene, cols) = (&mut self.grid, &self.scene[..], self.cols as usize);
        grid.fill(|cx, cy| scene[cy * cols + cx]);
        grid.flush(s, &PAL);
    }

    fn name(&self) -> &'static str {
        "toasters3"
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

    /// The two details `toasters`' module doc calls the tells, and they have to
    /// survive the change of renderer: motion strictly down AND left on the
    /// fixed RISE/RUN slope, and every object moving by the same vector.
    #[test]
    fn the_flock_moves_down_and_left_in_lockstep() {
        let t = Toasters3::new(&panel(), 30);
        assert!(t.step_x < 0, "must travel left");
        assert!(t.step_y > 0, "must travel down");
        assert_eq!(
            -t.step_x * RISE,
            t.step_y * RUN,
            "the slope is the homage, not a free parameter"
        );

        let mut t = Toasters3::new(&panel(), 30);
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

    /// The wings have to beat, and beat in both directions. A flap stuck on one
    /// frame, or walking 0,1,2,3 and snapping back, both pass a "does it
    /// animate" check and neither is the original.
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
        assert_eq!(walk, vec![0, 1, 2, 3, 2, 1]);

        // And it must be the WING that moves, not the body: the middle columns
        // of every row — inside the body, outside both wings — are identical
        // across all four frames.
        let band = |f: &'static [Cell]| {
            (0..TOASTER_H)
                .flat_map(|r| f[r * TOASTER_W + 5..r * TOASTER_W + 11].to_vec())
                .collect::<Vec<_>>()
        };
        for (i, f) in FRAMES.iter().enumerate() {
            assert_eq!(band(f), band(FRAMES[0]), "frame {i} moved the body");
        }

        // The divisor is what ties the flap to wall-clock time rather than to
        // the frame rate, so a slow panel must not flap slowly.
        assert_eq!(Toasters3::new(&panel(), 30).flap_div, 30 / 15);
    }

    /// The "screen went blank" bug class: pixels written but never reported.
    #[test]
    fn damage_covers_every_changed_scanline() {
        let p = panel();
        let mut t = Toasters3::new(&p, 30);
        let mut buf = vec![0u32; p.buf_len()];
        let mut prev = buf.clone();

        let d = saver::frame(&mut t, &mut buf, &p);
        assert_eq!(
            d.rows(),
            t.grid.rows() * t.cell_h as usize,
            "frame 0 must paint every cell"
        );

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
        assert!(moved > 30, "a flying flock must change pixels ({moved}/39)");

        // Against the GRID's height, not the panel's: the grid never owns the
        // bottom `h % cell_h` strip, so `worst < p.h` was true for any
        // implementation — including one repainting everything every frame.
        let all = t.grid.rows() * t.cell_h as usize;
        assert!(worst < all, "damaged every scanline ({worst} of {all})");
    }

    /// The failure mode this hand-rolled double-buffer actually has: a sprite
    /// stamped but not erased leaves a trail, and every other test here passes
    /// while it happens. What holds EXACTLY, every frame, is that `scene` shows
    /// the flock and nothing else — the union of what the objects stamped, no
    /// cell more and no cell less. A single trailing column fails it. A
    /// ratio-of-lit-pixels version of this test does not: stale columns get
    /// swept by the flock's own clear rectangles, so a real trail saturates.
    #[test]
    fn a_sprite_leaves_no_trail() {
        let p = panel();
        let mut t = Toasters3::new(&p, 30);
        let mut buf = vec![0u32; p.buf_len()];

        for n in 0..400 {
            saver::frame(&mut t, &mut buf, &p);
            // `render` has already bumped the tick, so the sprites on screen
            // are the ones the PREVIOUS tick chose.
            let tick = t.tick.wrapping_sub(1);
            let mut want = vec![Cell::CLEAR; t.scene.len()];
            for o in &t.objs {
                let (w, h) = o.size();
                let cells = o.cells(tick, t.flap_div);
                for r in 0..h as i32 {
                    for c in 0..w as i32 {
                        let (x, y) = (o.drawn.0 + c, o.drawn.1 + r);
                        if x < 0 || x >= t.cols || y < 0 || y >= t.rows {
                            continue;
                        }
                        let cell = cells[r as usize * w + c as usize];
                        if cell.glyph() != font::BLANK as usize {
                            want[(y * t.cols + x) as usize] = cell;
                        }
                    }
                }
            }
            // Cell for cell, not lit-or-not: a stale cell that happens to carry
            // a lit pattern under a different colour is still a trail.
            if let Some(i) = (0..want.len()).find(|&i| want[i] != t.scene[i]) {
                let (x, y) = (i as i32 % t.cols, i as i32 / t.cols);
                panic!(
                    "frame {n}: cell ({x},{y}) is {:?} but the flock says {:?}",
                    t.scene[i], want[i]
                );
            }
        }
    }

    /// A cell with ink but no dots is a sprite quietly losing a stroke: the
    /// colour is there and nothing draws. `bake_braille` cannot see it —
    /// `font::BRAILLE[0]` is a perfectly valid index that happens to be BLANK.
    #[test]
    fn every_sprite_cell_has_a_non_blank_glyph() {
        let all = FRAMES
            .iter()
            .enumerate()
            .map(|(i, f)| (format!("wing {i}"), *f))
            .chain(
                TOAST_CELLS
                    .iter()
                    .enumerate()
                    .map(|(i, c)| (format!("toast {i}"), &c[..])),
            );
        for (what, cells) in all {
            for (i, cell) in cells.iter().enumerate() {
                assert!(
                    *cell == Cell::CLEAR || cell.glyph() != font::BLANK as usize,
                    "{what} cell {i}: ink with no dots"
                );
                assert!(
                    cell.glyph() < font::GLYPHS.len(),
                    "{what} cell {i}: glyph past the atlas"
                );
                assert!(
                    font::GLYPHS[cell.glyph()].iter().any(|&r| r != 0) || *cell == Cell::CLEAR,
                    "{what} cell {i}: glyph is blank in the atlas"
                );
            }
        }
    }

    /// The whole reason for this saver: the art has to USE the 2x4 matrix. A
    /// sprite that only ever reached for the full cell and the blank one would
    /// render exactly as `blocks` does and every other test here would pass.
    #[test]
    fn the_art_uses_the_dot_matrix_and_not_just_solid_cells() {
        let patterns: std::collections::HashSet<usize> = FRAMES
            .iter()
            .flat_map(|f| f.iter())
            .chain(TOAST_CELLS.iter().flat_map(|c| c.iter()))
            .map(|c| c.glyph())
            .collect();
        assert!(
            patterns.len() > 24,
            "only {} distinct glyphs — this is block art, not braille",
            patterns.len()
        );
        assert!(
            patterns.contains(&(font::SOLID as usize)),
            "no fully-lit cell, so the body has no solid interior"
        );
        // Every glyph the art uses must be one the braille table names, or the
        // art is reaching into the atlas past the set it claims to draw with.
        for g in &patterns {
            assert!(
                font::BRAILLE.contains(&(*g as u16)),
                "glyph {g} is not a braille pattern"
            );
        }
    }

    /// Doneness has to darken. Every slice's crumb, against the next.
    #[test]
    fn every_slice_is_darker_than_the_last() {
        let mid = |d: usize| PAL[TOAST_CELLS[d][TOAST_W + 3].colour()];
        for d in 1..4 {
            assert!(
                mid(d) < mid(d - 1),
                "slice {d} is not darker than {}",
                d - 1
            );
        }
    }
}
