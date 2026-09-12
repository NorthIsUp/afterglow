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
//! * **The toaster is not a silver toaster.** Quantising the sprite sheet puts
//!   an olive chassis at a fifth of its pixels, behind a chrome front panel and
//!   white wings — four regions, not one flat metal. See `PAL_RGB`, and the ink
//!   grids that give each stroke of the art its own region.
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

/// Sampled from the original's sprite sheet and quantised to colour families,
/// which is how the one real surprise showed up: the toaster body is NOT
/// chrome. It is an olive chassis with a chrome front panel and white wings,
/// and painting the whole thing one silver is the colour equivalent of
/// scrolling straight left.
///
/// The two olives are the depth cue. #707030 is the lit top face, #303010 the
/// sides turned away from the light, and neither is ever painted against the
/// background: the chassis FILL is where the art has blanks and blanks are
/// transparent, so what a stroke has to be distinguishable from is the stroke
/// beside it. Against the lit olive and the chrome panel, #303010 reads as a
/// turned-away face. Flatten them into one olive and the three-quarter view
/// goes with it, the same way a single wing colour would.
///
/// The sheet's 8.6% of near-black #101010 is the one family with no entry, and
/// that is structural rather than perceptual: it is the outline that separated
/// the sprite from the sheet's background, and here a cell lights only its
/// glyph pixels, so the gaps between glyphs already draw it.
///
/// Index 0 is the background. Nothing paints over it, which is why an idle
/// region costs zero blits.
#[rustfmt::skip]
const PAL_RGB: [[u8; 3]; 16] = [
    [0x00, 0x00, 0x00], //  0        background
    [0x90, 0x90, 0x90], //  1 'C' chrome front panel, the sheet's largest family
    [0xF0, 0xF0, 0xF0], //  2 'W' white — the near wing
    [0xD0, 0xD0, 0xD0], //  3 'c' light chrome — far wing, lever, panel highlight
    [0x30, 0x30, 0x10], //  4 'O' dark olive — the chassis sides, turned away
    [0x70, 0x70, 0x30], //  5 'o' lit olive — the top face
    [0x70, 0x70, 0x70], //  6 's' shadow chrome — the panel's lower lip
    [0xB0, 0xB0, 0xB0], //  7 'm' chrome midtone — the slot rims
    [0xF0, 0xD0, 0x70], //  8 'G' golden crumb, the slice's largest family
    [0xD0, 0x90, 0x30], //  9 'g' mid gold
    [0x90, 0x70, 0x10], // 10 'b' brown crust
    [0x70, 0x30, 0x10], // 11 'd' dark crust edge
    [0xB0, 0x70, 0x10], // 12 'B' mid brown
    [0xF0, 0xF0, 0x90], // 13 'P' pale highlight
    [0xD0, 0xB0, 0x30], // 14 'y' amber
    [0xF0, 0xD0, 0x50], // 15 'Y' deep gold
];
const PAL: [u32; 16] = bake(&PAL_RGB);

/// Ink key -> palette index. Every sprite carries a grid of these parallel to
/// its art, because the art reuses characters across regions: the `/` in column
/// 0 is a white wing and the `/` in column 9 is the olive body's receding edge,
/// and a character-to-colour map could not tell them apart.
///
/// There is no fallback arm. This runs inside `bake_sprites`, in a const, so a
/// key nobody defined is a build failure and not a stroke silently painted the
/// background colour.
const fn ink(k: u8) -> u16 {
    match k {
        b'C' => 1,
        b'W' => 2,
        b'c' => 3,
        b'O' => 4,
        b'o' => 5,
        b's' => 6,
        b'm' => 7,
        b'G' => 8,
        b'g' => 9,
        b'b' => 10,
        b'd' => 11,
        b'B' => 12,
        b'P' => 13,
        b'y' => 14,
        b'Y' => 15,
        _ => panic!("sprite ink uses a key the palette does not define"),
    }
}

/// Resolve a sprite set's art and ink into cells, at COMPILE time — the same
/// bargain `grid::bake` makes for a palette, for the same reason. The frame
/// loop reads a `&'static [Cell]` out of the binary, so there is no allocation,
/// no `Vec` of `Vec`s to chase, and no construction-time work at all.
///
/// Row width comes from `CELLS / H`, and every row of both grids has to match
/// it: a ragged row, an ink grid that is not blank exactly where its art is, and
/// an undefined ink key are all `error[E0080]` rather than a sprite that renders
/// wrong. The one drift this cannot see is an ink row shifted within its own
/// width — same length, no blanks moved — which is what the colour assertions
/// in the tests are for.
const fn bake_sprites<const H: usize, const N: usize, const CELLS: usize>(
    art: &[[&str; H]; N],
    ink_rows: &[[&str; H]; N],
) -> [[Cell; CELLS]; N] {
    let w = CELLS / H;
    assert!(w * H == CELLS, "sprite cell count is not width x height");
    let mut out = [[Cell::CLEAR; CELLS]; N];
    let mut s = 0;
    while s < N {
        let mut r = 0;
        while r < H {
            let (a, k) = (art[s][r].as_bytes(), ink_rows[s][r].as_bytes());
            assert!(a.len() == w, "art row is not the sprite's width");
            assert!(k.len() == w, "ink row is not the sprite's width");
            let mut c = 0;
            while c < w {
                if a[c] == b' ' {
                    assert!(k[c] == b' ', "ink where the art is blank");
                } else {
                    assert!(k[c] != b' ', "art stroke with no ink");
                    out[s][r * w + c] = Cell::new(font::ASCII[(a[c] - 0x20) as usize], ink(k[c]));
                }
                c += 1;
            }
            r += 1;
        }
        s += 1;
    }
    out
}

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

/// The colour of every stroke above, cell for cell. Four regions, and they are
/// what the palette buys: the wings are white (the near one) and light chrome
/// (the far one, one stop down so the flap reads as depth), columns 3..=9 of
/// each row are the body — olive edges around a chrome front panel that runs
/// light-to-shadow left to right, between side edges in the turned-away olive —
/// and the top face is the lit olive with chrome slot rims. A space here must line up with a space in the art; the glyph test
/// checks it.
#[rustfmt::skip]
const TOASTER_INK: [[&str; TOASTER_H]; 4] = [
    [
        "W    oooo    c",
        "WW  ommmmOO cc",
        " WWOcCCCsOccc ",
        "   OsscssO    ",
    ],
    [
        "     oooo     ",
        "W   ommmmOO  c",
        "WWWOcCCCsOcccc",
        "   OsscssO    ",
    ],
    [
        "     oooo     ",
        "    ommmmOO   ",
        "WWWOcCCCsOcccc",
        "   OsscssO    ",
    ],
    [
        "     oooo     ",
        "    ommmmOO   ",
        " WWOcCCCsOccc ",
        "WW OsscssO  cc",
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

/// The doneness ramp, drawn in colour as well as in strokes: each level starts
/// one stop further down the gold -> brown ladder than the last and darkens
/// again from the crumb top to the crust edge, so all eight of the slice's
/// sampled colours are on screen at once across the flock.
#[rustfmt::skip]
const TOAST_INK: [[&str; TOAST_H]; 4] = [
    [
        " PP ",
        "G  G",
        "yYYy",
    ],
    [
        " GG ",
        "GggG",
        "gBBg",
    ],
    [
        " gg ",
        "gBBg",
        "BbbB",
    ],
    [
        " bb ",
        "bddb",
        "dddd",
    ],
];

/// The art, resolved. `render` stamps out of these and nothing else.
const TOASTER_CELLS: [[Cell; TOASTER_W * TOASTER_H]; 4] = bake_sprites(&TOASTER, &TOASTER_INK);
const TOAST_CELLS: [[Cell; TOAST_W * TOAST_H]; 4] = bake_sprites(&TOAST_SPRITE, &TOAST_INK);

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

    /// Which sprite this object shows right now: a wing frame for a toaster, a
    /// doneness level for a slice. This is the index the RENDERER uses, so a
    /// test that wants the art indexes `TOASTER` with it rather than being
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
            &TOASTER_CELLS[i]
        } else {
            &TOAST_CELLS[i]
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
/// `sprite` of `None` clears the whole rectangle; painting SKIPS the blanks
/// rather than clearing them, so sprites are transparent where they overlap.
/// Writing blanks instead punches the other sprite's strokes out — visible in a
/// dump as toasters eating each other.
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
                    // On the GLYPH, not on the whole packed word: the
                    // invariant is "this cell draws nothing", and a cell with
                    // no strokes but some colour index draws nothing while
                    // still comparing unequal to `Cell::CLEAR`.
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
            scene: vec![Cell::CLEAR; (cols * rows) as usize],
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

    /// The subject of the whole palette: four regions, each its own colour, and
    /// two olives that have to stay two. Frame 2 is level flight, the frame the
    /// flock spends most of its time in; row 2 is the body row. Asserting named
    /// cells is also the only thing that sees an ink row shifted within its own
    /// width — same length, blanks in the same places, every stroke a valid key,
    /// and the whole body wrongly coloured.
    #[test]
    fn each_region_of_the_toaster_is_its_own_colour() {
        let f = &TOASTER_CELLS[2];
        let at = |r: usize, c: usize| f[r * TOASTER_W + c].colour() as u16;

        assert_eq!(at(2, 0), ink(b'W'), "the near wing is white");
        assert_eq!(at(2, 13), ink(b'c'), "the far wing is one stop down");
        assert_eq!(at(2, 5), ink(b'C'), "the front panel is chrome");
        assert_eq!(at(2, 4), ink(b'c'), "lit at its left edge");
        assert_eq!(at(2, 8), ink(b's'), "and in shadow at its right");
        assert_eq!(at(2, 3), ink(b'O'), "the body's side edges are turned away");
        assert_eq!(at(2, 9), ink(b'O'));
        assert_eq!(at(0, 5), ink(b'o'), "the top face is the lit olive");
        assert_eq!(at(1, 5), ink(b'm'), "the slot rims are chrome");

        // The depth cue. One olive and the three-quarter view is a flat box,
        // exactly as one wing colour would make the flap a flat flicker.
        assert_ne!(
            ink(b'O'),
            ink(b'o'),
            "two olives, or there is no near and far"
        );
        assert_ne!(ink(b'W'), ink(b'c'), "two whites, same reason");

        // Doneness has to darken. Every slice's crumb top, against the next.
        let top = |d: usize| PAL[TOAST_CELLS[d][1].colour()];
        for d in 1..4 {
            assert!(
                top(d) < top(d - 1),
                "slice {d} is not darker than {}",
                d - 1
            );
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
            .map(|t| TOASTER[o.sprite(t, 1)][2])
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
