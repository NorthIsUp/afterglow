//! Flying toasters — an ASCII homage to After Dark's, Berkeley Systems, 1989.
//!
//! # What this is a copy of
//!
//! The art here is this repo's own, drawn from a description of the original.
//! What is copied is the BEHAVIOUR, and three details are what separate the
//! real thing from the usual imitation:
//!
//! * **Everything travels down-and-left, never straight left.** The original
//!   flock descends across the screen on one fixed diagonal. A horizontal
//!   scroll is the single most common tell.
//! * **Every object moves in lockstep.** One velocity, shared. No per-object
//!   speed jitter, no parallax, no depth layers — those are later screensavers'
//!   ideas, and they dissolve the flock into noise. Objects differ only in
//!   where they entered and where they are in the wing cycle.
//! * **The wings are four frames, not two.** Up, mid, level, down, walked as a
//!   ping-pong so the downstroke and the upstroke are both drawn. A two-frame
//!   flap reads as a flicker.
//!
//! Toast is a quarter of the flock (the original ran about 3:1 toasters to
//! toast). Four doneness sprites, each its own art rather than one sprite
//! tinted — the original put them behind a darkness slider.
//!
//! # Per-frame cost
//!
//! The CELL BLIT is O(objects): each object erases the bounding box it last
//! stamped and stamps a new one into `scene`, and nothing walks the grid per
//! object. ~15 objects x 56 cells against 3960 cells is the win, and it is the
//! part that scales with panel size.
//!
//! The SHADOW-TO-HARDWARE copy is not sparse, and saying otherwise would be a
//! lie a future reader acts on: damage is whole scanlines merged into at most
//! MAX_RUNS runs, so a dozen sprites at a dozen different heights smear across
//! ~60% of the panel per frame (measured: median 640 of 1056 scanlines). That
//! is still well under `ascii` and `matrix`, which repaint 100% every frame.

use crate::env_num;
use crate::font;
use crate::grid::{bake, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};

/// Black, chrome, toast, browned toast. The original's background is solid
/// black and nothing here paints over it, which is also why an idle region
/// costs zero blits.
const PAL_RGB: [[u8; 3]; 6] = [
    [0x00, 0x00, 0x00],
    [0xD8, 0xDE, 0xE6],
    [0xF2, 0xD9, 0x96],
    [0xE0, 0xA8, 0x50],
    [0xB0, 0x72, 0x28],
    [0x6B, 0x3E, 0x14],
];
const PAL: [u32; 6] = bake(&PAL_RGB);

const CHROME: u16 = 1;
/// Palette index of the palest slice; the four doneness levels are contiguous.
const TOAST0: u16 = 2;

const TOASTER_W: usize = 14;
const TOASTER_H: usize = 4;

/// Four wing positions of one two-slot toaster in three-quarter view: the top
/// face with its two slots recedes to the right, the near wing is the larger
/// one on the left. Rows are `TOASTER_W` wide and every frame is the same size,
/// so the stamp is a fixed rectangle and the flap never moves the body.
#[rustfmt::skip]
const TOASTER: [[&str; TOASTER_H]; 4] = [
    // 0 — wings fully up
    [
        r"\    ____    /",
        r"\\  /[][]/| //",
        r" \\|=====|/// ",
        r"   |__o__|    ",
    ],
    // 1 — mid upstroke
    [
        r"     ____     ",
        r"\   /[][]/|  /",
        r"\\_|=====|/_//",
        r"   |__o__|    ",
    ],
    // 2 — level, fully extended
    [
        r"     ____     ",
        r"    /[][]/|   ",
        r"\__|=====|/__/",
        r"   |__o__|    ",
    ],
    // 3 — wings fully down
    [
        r"     ____     ",
        r"    /[][]/|   ",
        r" //|=====|/\\ ",
        r"// |__o__|  \\",
    ],
];

/// Ping-pong over the four wing positions. Walking 0,1,2,3 and snapping back
/// draws only the downstroke; this draws both halves of the beat, which is
/// what the original's cycled frames looked like in motion.
const FLAP: [u8; 6] = [0, 1, 2, 3, 2, 1];

const TOAST_W: usize = 4;
const TOAST_H: usize = 3;

/// Four doneness levels, pale to scorched — the original shipped `toast0`
/// through `toast3` as four separate 64x64 sprites behind a darkness slider,
/// not one slice tinted, so the scorching is drawn as well as coloured.
#[rustfmt::skip]
const TOAST_SPRITE: [[&str; TOAST_H]; 4] = [
    [
        r" __ ",
        r"|  |",
        r"|__|",
    ],
    [
        r" __ ",
        r"|..|",
        r"|__|",
    ],
    [
        r" __ ",
        r"|::|",
        r"|##|",
    ],
    [
        r" __ ",
        r"|##|",
        r"|##|",
    ],
];

/// The diagonal: 5 across for every 2 down, about 22 degrees below horizontal.
///
/// This is the one number worth arguing about. The 45 degrees every web
/// recreation uses comes from Bryan Braun's CSS version (a `translate(-1600px,
/// 1600px)`), not from the original. The only slope taken from shipped code is
/// the After Dark 4.0 binary, where the flock drifts (-60, +24) per loop and
/// (-20, +8) per hold — 2.5 across per 1 down, which is what this is. Nobody
/// appears to have disassembled the 1990 Mac module, so the 2.0-era slope is
/// undocumented and 2.5:1 is the best evidence there is.
///
/// `TOASTER_SPEED` scales both components together, so the angle is a constant
/// of the homage and only the pace is a knob.
const RUN: i32 = 5;
const RISE: i32 = 2;

/// Sub-pixel bits in a position. 5 across per 2 down at a speed the eye reads
/// as the original is under six pixels a frame, so whole-pixel steps would have
/// to round the slope away; the fraction is carried instead.
const SUB: i32 = 8;

/// What a flock member is. Everything moves in lockstep, so an object carries
/// no velocity of its own — only where it is, what it is, and where it is in
/// the wing cycle.
#[derive(Clone, Copy)]
struct Obj {
    /// Top-left of the sprite, in pixels shifted left by `SUB`. Kept in pixels
    /// rather than cells so the diagonal is the panel's diagonal and not the
    /// cell grid's aspect ratio, which is 16x32 by default.
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

    fn sprite(&self, tick: u32, flap_div: u32) -> (&'static [&'static str], u16) {
        match self.kind {
            0 => {
                let step = (tick / flap_div) as usize + self.phase as usize;
                (&TOASTER[FLAP[step % FLAP.len()] as usize], CHROME)
            }
            k => (&TOAST_SPRITE[k as usize - 1], TOAST0 + k as u16 - 1),
        }
    }
}

pub struct Toasters {
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

/// splitmix32, not the LCG the other savers use. Fire and matrix draw one
/// number per cell, where the LCG's correlation between successive outputs is
/// invisible; this draws an (x, y) PAIR from consecutive outputs, and there the
/// correlation is a flock that clumps along a diagonal band and leaves a third
/// of the panel empty. That was visible in a dump, which is why this is here.
#[inline]
fn next_rand(rng: &mut u32) -> u32 {
    *rng = rng.wrapping_add(0x9E37_79B9);
    let mut z = *rng;
    z = (z ^ (z >> 16)).wrapping_mul(0x85EB_CA6B);
    z = (z ^ (z >> 13)).wrapping_mul(0xC2B2_AE35);
    (z ^ (z >> 16)) >> 1
}

/// Clear or paint one sprite-sized rectangle of `scene`, clipped to the grid.
/// `sprite` of `None` clears the whole rectangle; painting SKIPS the spaces
/// rather than clearing them, so sprites are transparent where they overlap.
/// Writing blanks instead punches the other sprite's strokes out — visible in a
/// dump as toasters eating each other.
fn stamp(
    scene: &mut [Cell],
    cols: i32,
    rows: i32,
    at: (i32, i32),
    size: (usize, usize),
    sprite: Option<(&[&str], u16)>,
) {
    let (cx, cy) = at;
    for r in 0..size.1 as i32 {
        let y = cy + r;
        if y < 0 || y >= rows {
            continue;
        }
        let row = sprite.map(|(s, _)| s[r as usize].as_bytes());
        for c in 0..size.0 as i32 {
            let x = cx + c;
            if x < 0 || x >= cols {
                continue;
            }
            let cell = match (row, sprite) {
                (Some(bytes), Some((_, colour))) => {
                    let ch = bytes[c as usize];
                    if ch == b' ' {
                        continue;
                    }
                    Cell::new(font::ASCII[(ch - 0x20) as usize], colour)
                }
                _ => Cell::new(font::BLANK, 0),
            };
            scene[(y * cols + x) as usize] = cell;
        }
    }
}

impl Toasters {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        let cell_w = env_num(&["TOASTER_CELL_W"], 16, 8, 64) as i32;
        let cell_h = env_num(&["TOASTER_CELL_H"], 32, 8, 128) as i32;
        let grid = Grid::new(panel, cell_w as usize, cell_h as usize);
        let (cols, rows) = (grid.cols() as i32, grid.rows() as i32);

        // Objects per 1000 cells, and the calibration knob: the right crowd
        // depends on the panel, on the cell size, and on taste, so this is a
        // dial rather than a constant that looked right once.
        //
        // 4 is derived, not guessed. The original sized its flock as
        // `screenArea / 480000 * 30`, i.e. 19 objects of 64x64 on a 640x480
        // screen — 25% of the screen under sprite. A 14x4-cell toaster on a
        // 120x33 grid is 224x128 px, so 16 objects is 22% of 1920x1080. Same
        // crowd, different sprite size.
        let density = env_num(&["TOASTER_DENSITY"], 4, 1, 60);
        let count = ((cols as i64 * rows as i64 * density) / 1000).clamp(1, 256) as usize;
        // Percent of the flock that is toast rather than a toaster. The
        // original holds roughly 3 toasters per slice — its spawner explicitly
        // emits food whenever the ratio passes 2.0 — and a count of Braun's
        // recreation gives 37:12.
        let toast_pct = env_num(&["TOASTER_TOAST_PCT"], 25, 0, 100) as u32;
        // Pixels per second across. 170 puts a 1920-wide panel inside the
        // 94..226 px/s bracket the recreations use, and matches the ~50 px/s
        // the 4.0 binary drifts scaled from 640 wide to 1920.
        let speed = env_num(&["TOASTER_SPEED"], 170, 8, 2000) as i32;
        // Wing frames per second. 15 is a 6-frame ping-pong in 0.4 s, which is
        // the 2.5 flaps/sec the recreations settle on. Below one wing frame per
        // render frame the divisor floors to 1 and the flap runs at the frame
        // rate instead.
        let flap_fps = env_num(&["TOASTER_FLAP_FPS"], 15, 1, 120) as u32;

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
            scene: vec![Cell::new(font::BLANK, 0); (cols * rows) as usize],
            step_x: -unit * RUN,
            step_y: unit * RISE,
            flap_div: (fps / flap_fps).max(1),
            tick: 0,
            rng: 0x1357_9bdf,
        };
        for _ in 0..count {
            let kind = if next_rand(&mut t.rng) % 100 < toast_pct {
                1 + (next_rand(&mut t.rng) % TOAST_SPRITE.len() as u32) as u8
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

    /// Put an object back on the leading edges — the top and the right, the
    /// two the flock enters through on a down-and-left path. The original
    /// called this its "reverse L" batch and stepped the entry points along
    /// fixed diagonal LANES rather than scattering them, so these snap to a
    /// cell boundary; weighting the choice by edge length is what keeps the
    /// arrival rate per unit of edge uniform instead of clumping in a corner.
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

impl Saver for Toasters {
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
                Some(o.sprite(tick, flap_div)),
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
        "toasters"
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
        Panel::new(640, 480, 640)
    }

    /// Every character the art uses has to exist in the atlas and be a
    /// non-blank glyph, or a sprite silently loses strokes. The space is the
    /// one legitimate blank.
    #[test]
    fn every_sprite_character_has_a_glyph() {
        let rows = TOASTER
            .iter()
            .flatten()
            .chain(TOAST_SPRITE.iter().flatten());
        for row in rows {
            for &b in row.as_bytes() {
                assert!((0x20..0x7F).contains(&b), "{b:#x} is not printable ascii");
                let g = font::ASCII[(b - 0x20) as usize];
                assert_eq!(
                    g == font::BLANK,
                    b == b' ',
                    "{:?} maps to glyph {g}",
                    b as char
                );
            }
        }
        // Uniform rows are what makes the stamp a fixed rectangle.
        for f in &TOASTER {
            for row in f {
                assert_eq!(row.len(), TOASTER_W);
            }
        }
        for f in &TOAST_SPRITE {
            for row in f {
                assert_eq!(row.len(), TOAST_W);
            }
        }
    }

    /// The two details the module doc calls the tells. Motion must be strictly
    /// down AND left at the fixed RISE/RUN slope, and every object must move by
    /// the same vector on the same frame — no per-object speed.
    #[test]
    fn the_flock_moves_down_and_left_in_lockstep() {
        let t = Toasters::new(&panel(), 30);
        assert!(t.step_x < 0, "must travel left");
        assert!(t.step_y > 0, "must travel down");
        assert_eq!(
            -t.step_x * RISE,
            t.step_y * RUN,
            "the slope is the homage, not a free parameter"
        );

        let mut t = Toasters::new(&panel(), 30);
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

    /// The wings have to actually beat, and beat in both directions. A flap
    /// stuck on one frame, or walking 0,1,2,3 and snapping back, both pass a
    /// "does it animate" check and neither is the original.
    #[test]
    fn the_wings_flap_through_all_four_positions_and_back() {
        let o = Obj {
            x: 0,
            y: 0,
            kind: 0,
            phase: 0,
            drawn: (i32::MIN, i32::MIN),
        };
        let seen: Vec<&str> = (0..FLAP.len() as u32)
            .map(|t| o.sprite(t, 1).0[2])
            .collect();
        let distinct: std::collections::BTreeSet<&&str> = seen.iter().collect();
        assert_eq!(distinct.len(), 4, "four wing positions, got {seen:?}");
        assert_eq!(seen[1], seen[FLAP.len() - 1], "the beat must ping-pong");
        // And it must be the wing that moves, not the body: the body columns
        // are identical across every frame.
        for f in &TOASTER {
            assert_eq!(&f[3][3..10], &TOASTER[0][3][3..10]);
        }
        // The divisor is what ties the flap to wall-clock time rather than to
        // the frame rate, so a slow panel must not flap slowly.
        assert_eq!(Toasters::new(&panel(), 30).flap_div, 30 / 15);
    }

    /// The "screen went blank" bug class: pixels written but never reported.
    /// `Surface` makes that unrepresentable, so what this really guards is the
    /// inverse — that the saver draws through `Surface` at all, that frame 0
    /// covers the panel, and that a moving flock's damage tracks it.
    #[test]
    fn damage_covers_every_changed_scanline() {
        // The real panel, not the small one: at 640x480 the flock is two
        // objects and a frame where both are still off the spawn edge changes
        // nothing, which says nothing either way about damage.
        let p = Panel::new(1920, 1080, 1920);
        let mut t = Toasters::new(&p, 30);
        let mut buf = vec![0u32; p.buf_len()];
        let mut prev = buf.clone();

        let d = saver::frame(&mut t, &mut buf, &p);
        // Every cell, which is the panel bar the bottom `h % cell_h` strip
        // the grid never owns — see the damage contract in surface.rs.
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
        // Over the window, not per frame: an idle frame is legitimate — the
        // sub-pixel step means cell positions do not advance every frame (4 in
        // 3000 measured) — so a per-frame assert is flaky on correct code at a
        // different seed, speed or density.
        assert!(moved > 30, "a flying flock must change pixels ({moved}/39)");

        // Against the GRID's height, not the panel's: the grid never owns the
        // bottom `h % cell_h` strip, so `worst < p.h` was true for any
        // implementation — including one repainting everything every frame.
        let all = t.grid.rows() * t.cell_h as usize;
        assert!(worst < all, "damaged every scanline ({worst} of {all})");
    }

    /// The failure mode this hand-rolled double-buffer actually has: a sprite
    /// that is stamped but not erased leaves a trail, and every other test
    /// here passes while it happens — damage is still reported, pixels still
    /// change. Lit cells would climb without bound; they must stay level.
    #[test]
    fn a_sprite_leaves_no_trail() {
        let p = Panel::new(1920, 1080, 1920);
        let mut t = Toasters::new(&p, 30);
        let mut buf = vec![0u32; p.buf_len()];

        let lit = |t: &mut Toasters, buf: &mut Vec<u32>| {
            saver::frame(t, buf, &p);
            buf.iter().filter(|&&px| px != 0).count()
        };
        let early = lit(&mut t, &mut buf);
        for _ in 0..400 {
            lit(&mut t, &mut buf);
        }
        let late = lit(&mut t, &mut buf);
        // Generous: the flock's on-screen count varies as objects respawn. A
        // trail is unbounded growth, not a wobble.
        assert!(
            late < early * 3,
            "lit pixels grew {early} -> {late}: sprites are not being erased"
        );
    }
}
