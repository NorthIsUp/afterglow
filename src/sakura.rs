//! Sakura — a cherry tree beside a lake at night, shedding blossom on a slow
//! wind.
//!
//! # What a reader needs that the code does not say
//!
//! * **The scene is built once and never redrawn.** `new` rasterises the tree,
//!   the lake and the reflection into `scene`, copies it into the grid, and
//!   from then on `render` touches only the cells petals occupy. Per-frame work
//!   is O(petals), never O(cells).
//! * **`scene` is the restore source, not a per-petal save.** Two petals in one
//!   cell would otherwise save each other's pixels and leave one behind — the
//!   trail bug in its usual costume. Restoring from the immutable scene makes
//!   overlap correct by construction.
//! * **The wind is one field sampled per petal**, not per-petal randomness. A
//!   gust has to arrive at every petal at once or the fall reads as noise
//!   instead of as weather. Per-petal `drag` is what keeps them from moving in
//!   lockstep.
//! * **Flutter is an oscillation, not a drift.** `x` carries only the wind's
//!   integral; the sway is added at draw time from a phase. A petal whose sway
//!   was accumulated into its position performs a random walk and wanders off
//!   the panel.
//! * **Sub-cell motion is free.** A petal's braille pattern is chosen from its
//!   fractional position inside the cell, so it moves in 6x4 px steps through a
//!   12x16 px cell without any extra cost.
//!
//! # Colour
//!
//! Night, not daylight: the panel's unlit pixels are black, so a dark sky costs
//! nothing and every lit thing is a silhouette against it. Blossom runs
//! `#FFD9E8` → `#9E5070` (near-white through to a dusty rose) because pink at
//! full saturation on black reads as neon; the trunk is `#584437`, a warm brown
//! dark enough to stay behind the blossom. The reflection is the same hues at
//! roughly 40% luminance and pulled toward the water's blue — a reflection at
//! the subject's brightness reads as a second tree, not as water.

use crate::font;
use crate::grid::{bake, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, next_rand};

#[rustfmt::skip]
const PAL_RGB: [[u8; 3]; 21] = [
    [0x00, 0x00, 0x00], //  0 night sky — the panel's own black, so it is free
    [0x5E, 0x66, 0x7A], //  1 star, dim
    [0x96, 0x9E, 0xB4], //  2 star, bright
    [0x58, 0x44, 0x37], //  3 trunk — warm brown, lit side
    [0x36, 0x2A, 0x21], //  4 branches and the trunk's shadow side
    [0xFF, 0xD9, 0xE8], //  5 blossom, sunlit crown
    [0xF7, 0xA8, 0xC4], //  6 blossom, mid
    [0xD9, 0x78, 0x9F], //  7 blossom, deep
    [0x9E, 0x50, 0x70], //  8 blossom, inside the canopy
    [0x24, 0x38, 0x4F], //  9 the water's surface line
    [0x0E, 0x1A, 0x2E], // 10 water, near the line
    [0x0A, 0x13, 0x22], // 11 water, mid
    [0x07, 0x0D, 0x18], // 12 water, far
    [0x24, 0x1C, 0x16], // 13 trunk, reflected
    [0x17, 0x12, 0x0E], // 14 branches, reflected
    [0x6B, 0x4D, 0x5C], // 15 blossom crown, reflected
    [0x5A, 0x43, 0x56], // 16 blossom mid, reflected
    [0x47, 0x34, 0x3F], // 17 blossom deep, reflected
    [0x35, 0x26, 0x2E], // 18 blossom inside, reflected
    [0xB0, 0x7F, 0x92], // 19 a petal resting on the water
    [0x4A, 0x64, 0x84], // 20 the ripple where one just landed
];
const PAL: [u32; 21] = bake(&PAL_RGB);

const STAR_DIM: u16 = 1;
const STAR_LIT: u16 = 2;
const TRUNK: u16 = 3;
const BRANCH: u16 = 4;
/// Crown to core. The tier is picked by height inside the canopy, so the mass
/// is lit from above — which is most of what makes it read as a solid crown
/// rather than as confetti.
const BLOSSOM: [u16; 4] = [5, 6, 7, 8];
const WATER_LINE: u16 = 9;
/// Near the line, mid, far. Three bands rather than a ramp: the water is nearly
/// black and more steps than this are not distinguishable on the panel.
const WATER: [u16; 3] = [10, 11, 12];
const PETAL_WET: u16 = 19;
const RIPPLE: u16 = 20;

/// Reflected counterpart of a scene colour, or 0 for "does not reflect". The
/// reflected entries sit at roughly 40% of the subject's luminance and are
/// pulled toward the water's blue; equal brightness reads as a second tree.
const fn reflect(c: u16) -> u16 {
    match c {
        TRUNK => 13,
        BRANCH => 14,
        5 => 15,
        6 => 16,
        7 => 17,
        8 => 18,
        _ => 0,
    }
}

/// Braille bit for a sub-cell dot: `DOT[column][row]` over the 2x4 grid inside
/// one cell. This is the whole reason the scene is drawn in braille — 8x the
/// spatial resolution of the cell grid for the price of a glyph index.
const DOT: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];

#[inline]
fn dot(sx: i32, sy: i32) -> u8 {
    if (0..2).contains(&sx) && (0..4).contains(&sy) {
        DOT[sx as usize][sy as usize]
    } else {
        0
    }
}

/// Mirror a braille cell top-to-bottom, for the reflection. Mirroring the whole
/// row of cells without mirroring inside each one leaves the reflection's
/// sub-cell detail a quarter-cell out of register with the subject's, which at
/// the waterline is exactly where the eye is looking.
fn flip_bits(b: u8) -> u8 {
    let mut out = 0;
    for (sx, col) in DOT.iter().enumerate() {
        for sy in 0..4 {
            if b & col[sy] != 0 {
                out |= DOT[sx][3 - sy];
            }
        }
    }
    out
}

/// sin over a 1024-unit turn, returning -1024..=1024. A parabola over a
/// triangle wave: peak error about 6% against a real sine, which is invisible
/// in a gust and costs no table and no float.
#[inline]
fn sin1024(a: i32) -> i32 {
    let a = a.rem_euclid(1024);
    let t = if a < 256 {
        a * 4
    } else if a < 768 {
        2048 - a * 4
    } else {
        a * 4 - 4096
    };
    t * (2048 - t.abs()) / 1024
}

#[inline]
fn cos1024(a: i32) -> i32 {
    sin1024(a + 256)
}

/// Position is 8.8 fixed point in CELLS: 256 units is one column or one row.
/// Cells, not pixels, so the same knob means the same thing at any cell size.
const FIX: i32 = 256;

/// Branch recursion depth. Four generations is where the tips stop being
/// individually visible at 12px cells and start being canopy.
const MAX_DEPTH: u32 = 4;

/// A sub-row is 1/4 cell and a sub-column 1/2 cell, so on a 12x16 cell they are
/// 6px by 4px. Everything drawn in sub-cell space has to correct for that or a
/// branch at 45 degrees comes out at 34 — hence the `* 3 / 2` on every dy.
const ASPECT_NUM: i32 = 3;
const ASPECT_DEN: i32 = 2;

/// One falling petal. Sixteen words, and the whole per-frame cost of the saver
/// is this struct times `SAKURA_PETALS`.
#[derive(Clone, Copy)]
struct Petal {
    /// Drift position, 8.8 in cells. The flutter is NOT accumulated here.
    x: i32,
    y: i32,
    /// Fall speed, 8.8 cells per frame.
    vy: i32,
    /// How much of the wind this petal catches, 0..=256. A shared wind with no
    /// per-petal drag moves the whole fall as one rigid sheet.
    drag: i32,
    /// Sway oscillator: phase over a 1024 turn, its step, and its amplitude in
    /// 8.8 cells.
    sway: i32,
    sway_rate: i32,
    sway_amp: i32,
    /// Tumble phase over a 1024 turn. Its quadrant picks the petal's outline,
    /// which is what gives the edge-on flicker.
    spin: i32,
    spin_rate: i32,
    colour: u16,
    /// Cell this petal is painted into, or `NOWHERE`.
    at: u32,
    /// 0 while airborne, else frames since it touched the water.
    rest: u32,
}

const NOWHERE: u32 = u32::MAX;

pub struct Sakura {
    grid: Grid,
    /// The still scene: tree, lake, reflection. The restore source for every
    /// cell a petal vacates, and never written after `new`.
    scene: Vec<Cell>,
    /// Cells a petal may detach from — the blossom mass.
    blossom: Vec<u32>,
    petals: Vec<Petal>,
    /// This frame's changed cells. Reserved in `new`, only ever cleared.
    dirty: Vec<u32>,
    /// First grid row that is water. Petals rest here and below.
    water_y: usize,
    /// Steady drift and gust amplitude, 8.8 cells per frame.
    wind_base: i32,
    wind_amp: i32,
    /// Periods of the two gust oscillators in frames. Deliberately
    /// incommensurate, so the pattern does not repeat on a countable cycle.
    gust_p: i32,
    gust_q: i32,
    /// Frames a landed petal shows a ripple, then floats, then recycles.
    ripple_frames: u32,
    rest_frames: u32,
    fall: i32,
    /// Sway amplitude (8.8 cells) and rate (1024-turn units per frame) before
    /// the per-petal scaling, and the same for the tumble.
    sway_amp_units: i32,
    sway_rate_units: i32,
    spin_rate_units: i32,
    frame: i32,
    rng: u32,
}

// ---------------------------------------------------------------------------
// Scene construction. All of this runs once, in `new`.
// ---------------------------------------------------------------------------

/// Sub-cell raster the scene is composed in: one byte of braille bits and one
/// colour per cell, so wood, blossom, water and reflection can overdraw each
/// other before anything becomes a `Cell`.
struct Raster {
    cols: i32,
    rows: i32,
    bits: Vec<u8>,
    col: Vec<u16>,
    /// Branch endpoints, where blossom clusters are stamped.
    tips: Vec<(i32, i32)>,
}

impl Raster {
    fn new(cols: usize, rows: usize) -> Self {
        Self {
            cols: cols as i32,
            rows: rows as i32,
            bits: vec![0; cols * rows],
            col: vec![0; cols * rows],
            tips: Vec::new(),
        }
    }

    /// Light one sub-cell dot and claim the cell for `c`. A cell holds one
    /// colour, so the last writer wins — which is why wood is drawn before
    /// blossom and the reflection after everything.
    #[inline]
    fn px(&mut self, sx: i32, sy: i32, c: u16) {
        let (cx, cy) = (sx.div_euclid(2), sy.div_euclid(4));
        if cx < 0 || cy < 0 || cx >= self.cols || cy >= self.rows {
            return;
        }
        let i = (cy * self.cols + cx) as usize;
        self.bits[i] |= dot(sx.rem_euclid(2), sy.rem_euclid(4));
        self.col[i] = c;
    }

    /// A limb: a straight run whose thickness tapers from `t0` to `t1`,
    /// thickened across its own dominant axis so a horizontal branch is not a
    /// one-pixel wire.
    #[allow(clippy::too_many_arguments)]
    fn limb(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, t0: i32, t1: i32, c: u16) {
        let n = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
        let steep = (y1 - y0).abs() >= (x1 - x0).abs();
        for k in 0..=n {
            let x = x0 + (x1 - x0) * k / n;
            let y = y0 + (y1 - y0) * k / n;
            let h = (t0 + (t1 - t0) * k / n) / 2;
            for o in -h..=h {
                if steep {
                    self.px(x + o, y, c);
                } else {
                    self.px(x, y + o, c);
                }
            }
        }
    }

    /// One branch and its children. `ang` is a 1024-unit turn with 256 = up.
    ///
    /// Two things here are the difference between a tree and a lightning bolt.
    /// The trunk forks into FOUR main limbs rather than two, so the crown is
    /// wide instead of a Y; and every generation is pulled further toward the
    /// horizontal, because a laden cherry branch droops. Without the droop the
    /// whole tree grows straight off the top of the panel.
    #[allow(clippy::too_many_arguments)]
    fn branch(&mut self, x: i32, y: i32, ang: i32, len: i32, thick: i32, d: u32, rng: &mut u32) {
        let x1 = x + ((cos1024(ang) * len) >> 10);
        let y1 = y - ((sin1024(ang) * len * ASPECT_NUM / ASPECT_DEN) >> 10);
        let next = (thick * 2 / 3).max(1);
        self.limb(
            x,
            y,
            x1,
            y1,
            thick,
            next,
            if d == 0 { TRUNK } else { BRANCH },
        );
        if d >= MAX_DEPTH || len < 4 {
            self.tips.push((x1, y1));
            return;
        }
        // Four limbs off the trunk, two or three off everything after it.
        let kids = if d == 0 {
            4
        } else {
            2 + (next_rand(rng) % 2) as i32
        };
        for k in 0..kids {
            let sign = if k % 2 == 0 { 1 } else { -1 };
            // Wider off the trunk than anywhere else: that first fork sets how
            // broad the crown is, and everything above it only refines it.
            let spread = if d == 0 {
                70 + (next_rand(rng) % 120) as i32
            } else {
                60 + (next_rand(rng) % 90) as i32
            };
            let mut a = ang + sign * spread + (next_rand(rng) % 27) as i32 - 13;
            a += (a - 256).signum() * (20 * d as i32);
            let scale = if d == 0 { 88 } else { 68 };
            let l = len * (scale + (next_rand(rng) % 20) as i32) / 100;
            self.branch(x1, y1, a, l, next, d + 1, rng);
            // A limb that only forks at its end grows as a bare Y, so blossom
            // is also hung partway along it.
            if d >= 1 {
                self.tips.push(((x + x1) / 2, (y + y1) / 2));
            }
        }
    }
}

/// Triangular draw over `-r..=r`: two uniforms summed. A uniform blob has a
/// hard edge and reads as a disc; this one has a soft one and reads as blossom.
#[inline]
fn tri(rng: &mut u32, r: i32) -> i32 {
    let r = r.max(1);
    ((next_rand(rng) % (r as u32 * 2 + 1)) as i32 + (next_rand(rng) % (r as u32 * 2 + 1)) as i32)
        / 2
        - r
}

impl Sakura {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        // 12x16, as city: a petal wants to be about a centimetre on the panel,
        // and 16x32 gives a 1080p panel only 33 rows to fall through.
        let cell_w = env_num(&["SAKURA_CELL_W"], 12, 4, 64) as usize;
        let cell_h = env_num(&["SAKURA_CELL_H"], 16, 4, 128) as usize;
        let mut grid = Grid::new(panel, cell_w, cell_h);
        let (cols, rows) = (grid.cols(), grid.rows());
        let fps = fps.max(1) as i32;

        let n_petals = env_num(&["SAKURA_PETALS"], 150, 0, 4000) as usize;
        // Hundredths of a CELL-ROW per second. In cells rather than pixels so
        // the same number means the same visual speed at any cell size.
        let fall_cps = env_num(&["SAKURA_FALL"], 380, 10, 20_000) as i32;
        // Steady breath of the wind, and how far a gust swings either side of
        // it. Both hundredths of a cell-column per second. The gust is several
        // times the base because a wind that never reverses reads as a
        // conveyor belt.
        let base_cps = env_num(&["SAKURA_WIND_BASE"], 180, -20_000, 20_000) as i32;
        let gust_cps = env_num(&["SAKURA_WIND"], 900, 0, 20_000) as i32;
        // Seconds per gust cycle, x10. Two oscillators with no common period.
        let gust_p = env_num(&["SAKURA_GUST_SECS"], 110, 5, 6000) as i32 * fps / 10;
        let gust_q = gust_p * 39 / 100;
        // Sway: amplitude in hundredths of a cell, and cycles per second x10.
        let sway_cps = env_num(&["SAKURA_FLUTTER"], 95, 0, 10_000) as i32;
        let sway_rate = env_num(&["SAKURA_FLUTTER_RATE"], 9, 1, 600) as i32;
        // Milliseconds for one full tumble of a petal.
        let spin_ms = env_num(&["SAKURA_TUMBLE_MS"], 1300, 60, 60_000) as i32;
        // How long a petal floats before it is recycled to the crown. It is
        // also what sets how many petals are in the AIR: a petal spends its
        // fall airborne and this afloat, so a long float empties the sky and
        // piles every petal onto the same few rows of water.
        let rest_frames = env_num(&["SAKURA_REST_SECS"], 5, 1, 3600) as u32 * fps as u32;
        let ripple_ms = env_num(&["SAKURA_RIPPLE_MS"], 420, 0, 10_000) as u32;
        let ripple_frames = ripple_ms * fps as u32 / 1000;
        let n_stars = env_num(&["SAKURA_STARS"], 110, 0, 20_000) as usize;
        // Where the horizon sits. Low enough that the lake is a lake and not a
        // puddle, high enough that the tree has room to be a tree.
        let water_y = (rows * env_num(&["SAKURA_WATER_PCT"], 64, 10, 95) as usize / 100).max(1);

        let mut rng = 0x5a_2a_91_7du32;
        let mut r = Raster::new(cols, rows);

        // --- the tree ---------------------------------------------------
        // Rooted on the waterline and leaning out over it, which is both what a
        // lakeside cherry does and what puts the canopy over its own
        // reflection.
        let trunk_x = (cols as i32 * 32 / 100) * 2;
        let trunk_y = water_y as i32 * 4;
        // A SHORT trunk. The crown, not the trunk, is what has to fill the
        // upper half — a trunk that reaches two thirds of the way up pushes
        // every branch off the top of the panel.
        let top = rows as i32 * 40 / 100;
        let trunk_len = ((water_y as i32 - top) * 4) * ASPECT_DEN / ASPECT_NUM;
        let thick = (cell_w as i32 / 2).clamp(4, 9);
        // Two segments with a kink, leaning out over the water: a dead-straight
        // trunk reads as a mast.
        let mid_len = trunk_len / 2;
        let kx = trunk_x + ((cos1024(240) * mid_len) >> 10);
        let ky = trunk_y - ((sin1024(240) * mid_len * ASPECT_NUM / ASPECT_DEN) >> 10);
        r.limb(trunk_x, trunk_y, kx, ky, thick, thick * 4 / 5, TRUNK);
        r.branch(
            kx,
            ky,
            252,
            (trunk_len - mid_len).max(4),
            thick * 4 / 5,
            0,
            &mut rng,
        );

        // --- the blossom ------------------------------------------------
        // Stamped around the branch tips rather than into a free-floating
        // ellipse, so the crown sits on the structure that holds it up.
        if !r.tips.is_empty() {
            let (mut lo, mut hi) = (i32::MAX, i32::MIN);
            for &(_, ty) in &r.tips {
                lo = lo.min(ty);
                hi = hi.max(ty);
            }
            let span = (hi - lo).max(1);
            // Roughly three quarters of a dot per CELL of panel. Below about half
            // that the crown stops being a mass and goes back to confetti.
            let dots = cols * rows * 3 / 4 * env_num(&["SAKURA_BLOOM"], 100, 0, 400) as usize / 100;
            for _ in 0..dots {
                let t = r.tips[next_rand(&mut rng) as usize % r.tips.len()];
                let sx = t.0 + tri(&mut rng, 4);
                let sy = t.1 + tri(&mut rng, 6);
                // Tier by height in the crown, jittered by one so the bands do
                // not become visible stripes.
                // Weighted to the middle of the ramp: an even split across the
                // four tiers makes the crown one flat pale mass, because the
                // two pale tiers are half of it.
                const TIER: [usize; 6] = [0, 1, 1, 2, 2, 3];
                let t = ((sy - lo) * 6 / span + (next_rand(&mut rng) % 3) as i32 - 1).clamp(0, 5)
                    as usize;
                let tier = TIER[t];
                r.px(sx, sy, BLOSSOM[tier]);
            }
        }

        // --- the stars --------------------------------------------------
        // Only in empty sky: a star inside the canopy would recolour the whole
        // cell and punch a grey hole in the blossom.
        for _ in 0..n_stars {
            let cx = next_rand(&mut rng) as i32 % cols.max(1) as i32;
            let cy = next_rand(&mut rng) as i32 % water_y.max(1) as i32;
            let i = (cy * cols as i32 + cx) as usize;
            if r.col[i] != 0 {
                continue;
            }
            let c = if next_rand(&mut rng).is_multiple_of(4) {
                STAR_LIT
            } else {
                STAR_DIM
            };
            r.px(
                cx * 2 + (next_rand(&mut rng) % 2) as i32,
                cy * 4 + (next_rand(&mut rng) % 4) as i32,
                c,
            );
        }

        // --- the lake ---------------------------------------------------
        // The surface line first: a full-width bar one sub-row tall, which is
        // the single strongest cue that the bottom of the panel is water.
        for cx in 0..cols as i32 {
            r.px(cx * 2, water_y as i32 * 4, WATER_LINE);
            r.px(cx * 2 + 1, water_y as i32 * 4, WATER_LINE);
        }
        // Then a sparse scatter of horizontal dashes, thinning and darkening
        // with distance. Horizontal because a vertical mark in water reads as a
        // post, not as a ripple.
        for cy in (water_y + 1)..rows {
            let k = cy - water_y;
            let band = (k * 3 / (rows - water_y).max(1)).min(2);
            // Sparse and STREAKY. A dash per cell at any real density reads as
            // static; a few long ones per row read as a swell.
            let density = 13u32.saturating_sub(k as u32 / 3).max(3);
            let mut cx = 0i32;
            while cx < cols as i32 {
                if next_rand(&mut rng) % 100 >= density {
                    cx += 1;
                    continue;
                }
                let sy = cy as i32 * 4 + (next_rand(&mut rng) % 4) as i32;
                let run = 3 + (next_rand(&mut rng) % 9) as i32;
                for o in 0..run {
                    r.px(cx * 2 + o, sy, WATER[band]);
                }
                cx += run / 2 + 2;
            }
        }

        // --- the reflection ---------------------------------------------
        // A vertical mirror about the waterline with a per-row horizontal
        // shear, which is the cheapest thing that reads as a rippled surface.
        // Rows further from the line are sheared further and dissolve, so the
        // reflection fades into open water instead of stopping dead.
        for k in 0..(rows - water_y - 1) {
            let dst_y = water_y + 1 + k;
            let Some(src_y) = water_y.checked_sub(1 + k) else {
                break;
            };
            let amp = 1 + (k as i32) / 5;
            let jit = ((sin1024(k as i32 * 167 + 240) * amp) >> 10).clamp(-4, 4);
            // Past a third of the way down the reflection breaks up.
            let keep =
                100u32.saturating_sub(k as u32 * 100 / (rows - water_y).max(1) as u32 * 2 / 3);
            for cx in 0..cols as i32 {
                let sx = cx + jit;
                if sx < 0 || sx >= cols as i32 {
                    continue;
                }
                let si = (src_y as i32 * cols as i32 + sx) as usize;
                let rc = reflect(r.col[si]);
                if rc == 0 || r.bits[si] == 0 {
                    continue;
                }
                if next_rand(&mut rng) % 100 >= keep {
                    continue;
                }
                let di = (dst_y as i32 * cols as i32 + cx) as usize;
                r.bits[di] |= flip_bits(r.bits[si]);
                r.col[di] = rc;
            }
        }

        // --- bake -------------------------------------------------------
        let scene: Vec<Cell> = r
            .bits
            .iter()
            .zip(r.col.iter())
            .map(|(&b, &c)| {
                if c == 0 || b == 0 {
                    Cell::CLEAR
                } else {
                    Cell::new(font::BRAILLE[b as usize], c)
                }
            })
            .collect();
        for (i, &c) in scene.iter().enumerate() {
            grid.set(i, c);
        }

        let blossom: Vec<u32> = r
            .col
            .iter()
            .enumerate()
            .filter(|&(i, &c)| BLOSSOM.contains(&c) && r.bits[i] != 0)
            .map(|(i, _)| i as u32)
            .collect();

        let per_frame = |cps: i32| cps * FIX / (100 * fps);
        let mut me = Self {
            grid,
            scene,
            blossom,
            petals: vec![
                Petal {
                    x: 0,
                    y: 0,
                    vy: 0,
                    drag: 256,
                    sway: 0,
                    sway_rate: 0,
                    sway_amp: 0,
                    spin: 0,
                    spin_rate: 0,
                    colour: BLOSSOM[1],
                    at: NOWHERE,
                    rest: 0,
                };
                n_petals
            ],
            // Two entries per petal: the cell it leaves and the cell it takes.
            dirty: Vec::with_capacity(n_petals * 2 + 8),
            water_y,
            wind_base: per_frame(base_cps),
            wind_amp: per_frame(gust_cps),
            gust_p: gust_p.max(1),
            gust_q: gust_q.max(1),
            ripple_frames,
            rest_frames,
            fall: per_frame(fall_cps).max(1),
            sway_amp_units: sway_cps * FIX / 100,
            sway_rate_units: (1024 * sway_rate / (10 * fps)).max(1),
            spin_rate_units: (1024 * 1000 / (spin_ms * fps)).max(1),
            frame: 0,
            rng,
        };
        for k in 0..n_petals {
            me.respawn(k);
            // Stagger BOTH phases, or the tree sheds its whole crown on frame 1
            // and then the whole crown lands on the same frame five seconds
            // later — which looks exactly like a bug and is one.
            if next_rand(&mut me.rng) % 100 < 35 {
                me.petals[k].rest = 1 + next_rand(&mut me.rng) % rest_frames.max(1);
                me.land(k);
            } else {
                let drop = (me.water_y as i32 * FIX - me.petals[k].y).max(1);
                me.petals[k].y += (next_rand(&mut me.rng) as i32).rem_euclid(drop);
            }
        }
        me
    }

    /// Wind at a frame, 8.8 cells per frame, shared by every petal. Two
    /// oscillators of incommensurate period summed 6:4, so gusts ease in and
    /// out and the pattern never settles into a beat.
    #[inline]
    fn wind(&self, t: i32) -> i32 {
        let a = sin1024((t % self.gust_p) * 1024 / self.gust_p);
        let b = sin1024((t % self.gust_q) * 1024 / self.gust_q + 371);
        self.wind_base + ((self.wind_amp * (a * 6 + b * 4) / 10) >> 10)
    }

    /// Send a petal back to the crown with a fresh set of parameters.
    fn respawn(&mut self, k: usize) {
        let (cols, rows) = (self.grid.cols() as i32, self.grid.rows() as i32);
        let (x, y) = if self.blossom.is_empty() {
            // A panel too small to hold a tree still has to hold petals.
            (
                (next_rand(&mut self.rng) as i32).rem_euclid(cols) * FIX,
                (next_rand(&mut self.rng) as i32).rem_euclid(rows.min(2)) * FIX,
            )
        } else {
            let c = self.blossom[next_rand(&mut self.rng) as usize % self.blossom.len()] as i32;
            (
                (c % cols) * FIX + (next_rand(&mut self.rng) as i32).rem_euclid(FIX),
                (c / cols) * FIX,
            )
        };
        let vy = self.fall * (70 + (next_rand(&mut self.rng) % 61) as i32) / 100;
        let drag = 140 + (next_rand(&mut self.rng) % 117) as i32;
        let sway = (next_rand(&mut self.rng) % 1024) as i32;
        let sway_rate = self.sway_rate_units * (60 + (next_rand(&mut self.rng) % 81) as i32) / 100;
        let sway_amp = self.sway_amp_units * (50 + (next_rand(&mut self.rng) % 101) as i32) / 100;
        let spin = (next_rand(&mut self.rng) % 1024) as i32;
        let spin_rate = self.spin_rate_units * (60 + (next_rand(&mut self.rng) % 101) as i32) / 100;
        // Mostly the mid pinks: a shower of pure white petals looks like snow.
        let colour = BLOSSOM[[0usize, 1, 1, 1, 2, 2][(next_rand(&mut self.rng) % 6) as usize]];
        let p = &mut self.petals[k];
        *p = Petal {
            x,
            y,
            vy: vy.max(1),
            drag,
            sway,
            sway_rate,
            sway_amp,
            spin,
            spin_rate: spin_rate.max(1),
            colour,
            at: NOWHERE,
            rest: 0,
        };
    }

    /// Put a petal on the surface. Spread over several rows and sub-rows
    /// rather than snapped to the waterline, or every petal that ever lands
    /// stacks into one solid bar of pink across the lake.
    fn land(&mut self, k: usize) {
        let water = self.water_y as i32;
        let rows = self.grid.rows() as i32;
        let row = water + (next_rand(&mut self.rng) % 5) as i32;
        let sub = (next_rand(&mut self.rng) % 4) as i32;
        self.petals[k].y = (row.min(rows - 1)) * FIX + sub * (FIX / 4);
    }

    /// The petal's outline for this tumble phase, positioned at its sub-cell.
    /// Face-on is a three-dot blob, edge-on is a single dot, and the two
    /// diagonals between them are pairs — that alternation IS the flutter.
    #[inline]
    fn petal_bits(spin: i32, sx: i32, sy: i32) -> u8 {
        match (spin.rem_euclid(1024)) / 256 {
            0 => dot(0, sy) | dot(1, sy) | dot(sx, sy + 1),
            1 => dot(sx, sy) | dot(1 - sx, sy + 1),
            2 => dot(sx, sy),
            _ => dot(sx, sy) | dot(1 - sx, sy - 1),
        }
    }
}

impl Saver for Sakura {
    fn render(&mut self, s: &mut Surface<'_>) {
        self.dirty.clear();

        // Lift every petal first, so a petal that moves into the cell another
        // one is leaving is not erased by that one's restore.
        for k in 0..self.petals.len() {
            let at = self.petals[k].at;
            if at != NOWHERE {
                self.grid.set(at as usize, self.scene[at as usize]);
                self.dirty.push(at);
                self.petals[k].at = NOWHERE;
            }
        }

        let w = self.wind(self.frame);
        self.frame = self.frame.wrapping_add(1);
        let (cols, rows) = (self.grid.cols() as i32, self.grid.rows() as i32);
        let water = self.water_y as i32;

        for k in 0..self.petals.len() {
            let p = self.petals[k];
            let mut p = p;

            if p.rest == 0 {
                p.y += p.vy;
                p.x += (w * p.drag) >> 8;
                p.sway = (p.sway + p.sway_rate).rem_euclid(1024);
                p.spin = (p.spin + p.spin_rate).rem_euclid(1024);
            } else {
                // Afloat: carried by the surface, which is slower than the air.
                p.x += (w * p.drag) >> 10;
                p.rest += 1;
            }

            // The sway is an offset, never accumulated — see the module doc.
            let dx = p.x + ((sin1024(p.sway) * p.sway_amp) >> 10);
            // A petal edge-on to its fall stalls a little. Two sway cycles per
            // tumble, so the stall lands on the flicker.
            let dy = p.y + ((sin1024(p.sway * 2) * p.sway_amp) >> 12);

            if p.rest == 0 && dy >= water * FIX {
                p.rest = 1;
                self.petals[k] = p;
                self.land(k);
                p = self.petals[k];
            }

            let cx = dx.div_euclid(FIX);
            let cy = p.y.div_euclid(FIX).min(rows - 1);
            let recycle = cx < 0 || cx >= cols || p.rest > self.rest_frames;
            if recycle {
                self.petals[k] = p;
                self.respawn(k);
                continue;
            }

            let (gx, gy) = if p.rest == 0 { (dx, dy) } else { (dx, p.y) };
            let sx = (gx.div_euclid(FIX / 2)).rem_euclid(2);
            let sy = (gy.div_euclid(FIX / 4)).rem_euclid(4);
            let bits = if p.rest == 0 {
                Self::petal_bits(p.spin, sx, sy)
            } else {
                // Afloat it lies flat: a horizontal pair, which also happens to
                // be the shape of the ripple it arrived in.
                dot(0, sy) | dot(1, sy)
            };
            let colour = if p.rest > 0 && p.rest <= self.ripple_frames {
                RIPPLE
            } else if p.rest > 0 {
                PETAL_WET
            } else {
                p.colour
            };
            let i = (cy * cols + cx) as u32;
            self.grid
                .set(i as usize, Cell::new(font::BRAILLE[bits as usize], colour));
            self.dirty.push(i);
            p.at = i;
            self.petals[k] = p;
        }

        // Row-major, and deduplicated: a cell that was vacated and re-taken is
        // in the list twice, and `Damage::mark` merges only into the LAST run.
        self.dirty.sort_unstable();
        self.dirty.dedup();
        self.grid.flush_sparse(s, &PAL, &self.dirty);
    }

    fn name(&self) -> &'static str {
        "sakura"
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

    fn sakura() -> Sakura {
        Sakura::new(&panel(), 30)
    }

    fn cells(s: &Sakura) -> usize {
        s.grid.cols() * s.grid.rows()
    }

    /// Relative luminance, near enough for "is this darker than that".
    fn lum(c: u16) -> u32 {
        let [r, g, b] = PAL_RGB[c as usize];
        r as u32 * 30 + g as u32 * 59 + b as u32 * 11
    }

    /// The bottom strip below the last cell row is a real error line on the
    /// panel, and a frame-0 assertion that only checks the cells cannot see it.
    /// The pixel count is what stops this passing on a black panel.
    #[test]
    fn frame_zero_paints_the_whole_panel_and_shows_a_scene() {
        let p = panel();
        let mut s = sakura();
        let mut buf = vec![0u32; p.buf_len()];
        let d = saver::frame(&mut s, &mut buf, &p);
        assert_eq!(d.rows(), p.h, "frame 0 must paint every scanline");
        let lit = buf.iter().filter(|&&px| px != 0).count();
        assert!(
            lit > 50_000,
            "frame 0 painted {lit} lit pixels: not a scene"
        );
        for i in 0..cells(&s) {
            assert_eq!(s.grid.cell(i), s.grid.cells()[i], "frame 0 left {i} stale");
        }
    }

    /// The `flush_sparse` under-report bug, which a framebuffer diff is
    /// structurally blind to: an unreported write is never blitted, so the
    /// framebuffer never changes and the diff finds nothing. `cur` and `prev`
    /// are identical after every flush, so a cell written and left out of
    /// `dirty` is a mismatch HERE even though the panel would just freeze.
    #[test]
    fn every_written_cell_is_reported() {
        let p = panel();
        let mut s = sakura();
        let mut buf = vec![0u32; p.buf_len()];
        let mut prev = buf.clone();
        saver::frame(&mut s, &mut buf, &p);

        let n = cells(&s);
        let mut rows = Vec::new();
        let mut moved = 0;
        for f in 1..1500 {
            prev.copy_from_slice(&buf);
            let d = saver::frame(&mut s, &mut buf, &p);
            for i in 0..n {
                assert_eq!(
                    s.grid.cell(i),
                    s.grid.cells()[i],
                    "frame {f}: cell {i} was written but not reported"
                );
            }
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
                    "frame {f}: scanline {y} changed but was not reported"
                );
            }
            moved += usize::from(changed > 0);
            rows.push(d.rows());
        }
        assert!(
            moved > 1400,
            "the petals stalled ({moved}/1499 frames moved)"
        );

        // The point of the saver: a repaint regression is what this catches.
        // Petals are scattered down the whole panel, so scanline-granular
        // damage is inherently coarse — the bound is what was MEASURED with a
        // margin, not an aspiration.
        rows.sort_unstable();
        let median = rows[rows.len() / 2];
        assert!(
            median < p.h * 2 / 3,
            "median damage {median} of {} scanlines: the scene is not static",
            p.h
        );
    }

    /// A petal must leave NOTHING behind. Exact, cell for cell, against the
    /// still scene — not a ratio: trails saturate rather than growing without
    /// bound, so a ratio test waves a real trail straight through.
    #[test]
    fn a_petal_restores_exactly_what_was_under_it() {
        let p = panel();
        let mut s = sakura();
        let mut buf = vec![0u32; p.buf_len()];
        let n = cells(&s);
        let mut occupied = vec![false; n];
        let mut over_scene = 0usize;

        for f in 0..1200 {
            saver::frame(&mut s, &mut buf, &p);
            occupied.fill(false);
            for q in &s.petals {
                if q.at != NOWHERE {
                    occupied[q.at as usize] = true;
                    over_scene += usize::from(s.scene[q.at as usize] != Cell::CLEAR);
                }
            }
            for (i, &taken) in occupied.iter().enumerate().take(n) {
                if taken {
                    continue;
                }
                assert_eq!(
                    s.grid.cell(i),
                    s.scene[i],
                    "frame {f}: cell {i} is not the scene — a petal left a trail"
                );
            }
        }
        // Non-vacuous: petals must actually have crossed painted scene cells,
        // or "nothing left behind" is "nothing happened".
        assert!(
            over_scene > 5_000,
            "petals only covered {over_scene} painted cells: nothing to restore"
        );
    }

    /// Gusts, not a constant. It has to reverse, it has to be smooth, and it
    /// must not settle into one value.
    #[test]
    fn the_wind_breathes() {
        let s = sakura();
        let w: Vec<i32> = (0..20_000).map(|t| s.wind(t)).collect();
        let (lo, hi) = (*w.iter().min().unwrap(), *w.iter().max().unwrap());
        assert!(lo < 0, "the wind never reverses (min {lo})");
        assert!(hi > 0, "the wind never blows (max {hi})");
        // Smooth: a gust that jumps is not a gust. One frame may not move the
        // wind by more than a small fraction of its whole range.
        let span = hi - lo;
        for t in 1..w.len() {
            assert!(
                (w[t] - w[t - 1]).abs() * 20 < span,
                "the wind jumped {} at frame {t} (range {span})",
                w[t] - w[t - 1]
            );
        }
        // And it visits the middle of its range, not just the extremes.
        let mid = w.iter().filter(|&&v| v.abs() * 4 < span).count();
        assert!(mid > 2_000, "the wind is a square wave ({mid} mid samples)");
    }

    /// A petal that falls like a stone is the failure mode. It has to descend,
    /// it has to move sideways in BOTH directions over a gust cycle, and its
    /// outline has to change as it tumbles.
    #[test]
    fn petals_fall_drift_and_flutter() {
        let p = panel();
        let mut s = sakura();
        let mut buf = vec![0u32; p.buf_len()];

        // One petal, watched: a population average hides a stone.
        s.petals.truncate(1);
        saver::frame(&mut s, &mut buf, &p);

        let (mut left, mut right, mut fell) = (0, 0, 0);
        let mut glyphs = std::collections::BTreeSet::new();
        let mut last = (s.petals[0].x, s.petals[0].y);
        for _ in 0..6_000 {
            saver::frame(&mut s, &mut buf, &p);
            let q = s.petals[0];
            if q.rest == 0 && q.y > last.1 {
                fell += 1;
                match q.x.cmp(&last.0) {
                    std::cmp::Ordering::Less => left += 1,
                    std::cmp::Ordering::Greater => right += 1,
                    std::cmp::Ordering::Equal => {}
                }
                if q.at != NOWHERE {
                    glyphs.insert(s.grid.cell(q.at as usize).glyph());
                }
            }
            last = (q.x, q.y);
        }
        assert!(fell > 1_000, "the petal barely descended ({fell} frames)");
        assert!(left > 200, "the petal never drifted left ({left} frames)");
        assert!(
            right > 200,
            "the petal never drifted right ({right} frames)"
        );
        // Four of these are the tumble alone; more than that needs the
        // sub-cell position to be moving too, which is the sway.
        assert!(
            glyphs.len() > 8,
            "the petal wore {} outlines: it is not tumbling",
            glyphs.len()
        );
        assert!(s.petals[0].sway_amp > 0, "the petal has no sway at all");
    }

    /// They have to reach the lake, float, and never sink through the panel.
    #[test]
    fn petals_land_on_the_water_and_float() {
        let p = panel();
        let mut s = sakura();
        let mut buf = vec![0u32; p.buf_len()];
        let rows = s.grid.rows() as i32;
        let water = s.water_y as i32;

        let mut ever_rested = 0;
        let mut ever_flew = 0;
        for f in 0..900 {
            saver::frame(&mut s, &mut buf, &p);
            for q in &s.petals {
                if q.rest > 0 {
                    ever_rested += 1;
                    let row = q.y.div_euclid(FIX);
                    assert!(
                        (water..rows).contains(&row),
                        "frame {f}: a floating petal is at row {row}, water starts at {water}"
                    );
                } else {
                    ever_flew += 1;
                }
                if q.at != NOWHERE {
                    let row = (q.at / s.grid.cols() as u32) as i32;
                    assert!(row < rows, "a petal is off the bottom of the grid");
                }
            }
        }
        assert!(
            ever_rested > 10_000,
            "hardly anything landed ({ever_rested})"
        );
        assert!(ever_flew > 10_000, "hardly anything flew ({ever_flew})");
    }

    /// CLAUDE.md makes the frame loop the top constraint in this repo.
    /// Capacity, not length: a `Vec` that never grows past the capacity `new`
    /// reserved never reallocates, and `clear` keeps capacity.
    #[test]
    fn render_never_allocates() {
        let p = panel();
        let mut s = sakura();
        let mut buf = vec![0u32; p.buf_len()];
        let reserved = s.dirty.capacity();
        assert!(reserved > 0, "nothing was reserved for the frame loop");
        let mut worst = 0;
        for _ in 0..20_000 {
            saver::frame(&mut s, &mut buf, &p);
            worst = worst.max(s.dirty.len());
            assert_eq!(
                s.dirty.capacity(),
                reserved,
                "`dirty` grew past its reserve: the render path allocated"
            );
        }
        assert!(
            worst > s.petals.len(),
            "`dirty` never held a whole frame's worth ({worst})"
        );
    }

    /// The scene itself: a tree in the sky, a lake under it, and a reflection
    /// that is dimmer than what it reflects. A reflection at the subject's
    /// brightness reads as a second tree.
    #[test]
    fn the_scene_is_a_tree_over_a_lake_with_a_dimmer_reflection() {
        let s = sakura();
        let cols = s.grid.cols();
        let mut above = [0usize; 21];
        let mut below = [0usize; 21];
        for (i, c) in s.scene.iter().enumerate() {
            if *c == Cell::CLEAR {
                continue;
            }
            if i / cols < s.water_y {
                above[c.colour()] += 1;
            } else {
                below[c.colour()] += 1;
            }
        }
        assert!(
            above[TRUNK as usize] + above[BRANCH as usize] > 60,
            "no tree above the water"
        );
        let bloom: usize = BLOSSOM.iter().map(|&c| above[c as usize]).sum();
        // Measured at 671 on a 1920x1080 panel; the bound is that with room.
        assert!(
            bloom > 450,
            "only {bloom} blossom cells: that is not a crown"
        );
        // Every tier of the crown is used, or the shading is not shading.
        for &c in &BLOSSOM {
            assert!(above[c as usize] > 40, "blossom tier {c} is unused");
        }
        let water: usize = WATER.iter().map(|&c| below[c as usize]).sum();
        assert!(water > 600, "only {water} water cells: that is not a lake");
        assert!(below[WATER_LINE as usize] > cols / 2, "no surface line");

        // Reflections exist, are below the line only, and are darker.
        for &c in BLOSSOM.iter().chain([TRUNK, BRANCH].iter()) {
            let r = reflect(c);
            assert_ne!(r, 0, "colour {c} has no reflection");
            assert!(
                lum(r) * 2 < lum(c),
                "reflection {r} ({}) is not dark enough against {c} ({})",
                lum(r),
                lum(c)
            );
            assert_eq!(above[r as usize], 0, "reflection {r} drawn above the water");
        }
        // As a whole, not per tier: the crown's top tier is high enough that
        // its mirror image falls off the bottom of the panel, which is what a
        // real reflection does.
        let mirrored: usize = (13..=18).map(|r| below[r]).sum();
        assert!(
            mirrored > 150,
            "only {mirrored} reflected cells in the lake"
        );
    }

    /// A colour with no glyph is an invisible cell, and a cell that claims a
    /// colour while drawing nothing is how a scene silently loses a feature.
    #[test]
    fn every_cell_the_scene_paints_is_actually_drawn() {
        let s = sakura();
        let mut lit = 0;
        for (i, c) in s.scene.iter().enumerate() {
            if *c == Cell::CLEAR {
                continue;
            }
            lit += 1;
            assert!(
                font::GLYPHS[c.glyph()].iter().any(|&b| b != 0),
                "cell {i} has colour {} and an empty glyph",
                c.colour()
            );
            assert!(c.colour() != 0, "cell {i} is painted in the background");
        }
        assert!(lit > 1_500, "only {lit} cells painted");
    }

    /// Mirroring rows without mirroring inside each cell leaves the reflection
    /// a quarter-cell out of register exactly at the waterline.
    #[test]
    fn a_braille_cell_mirrors_top_to_bottom() {
        assert_eq!(flip_bits(0x01), 0x40);
        assert_eq!(flip_bits(0x40), 0x01);
        assert_eq!(flip_bits(0x08), 0x80);
        assert_eq!(flip_bits(0x02), 0x04);
        assert_eq!(flip_bits(0xFF), 0xFF);
        assert_eq!(flip_bits(0x00), 0x00);
        for b in 0..=255u8 {
            assert_eq!(
                flip_bits(flip_bits(b)),
                b,
                "flip is not an involution at {b}"
            );
        }
    }
}
