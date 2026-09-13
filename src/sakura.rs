//! Sakura — a cherry tree at night, shedding blossom on a slow wind. The tree
//! is grown from a seed that changes every time the pod starts, and it stands
//! in one of three places: beside a pond, on a mountain spur, or in a rock
//! garden.
//!
//! # What a reader needs that the code does not say
//!
//! * **The scene is built once and never redrawn.** `build` rasterises the
//!   tree and its setting into `scene`, copies it into the grid, and from then
//!   on `render` touches only the cells petals occupy. Per-frame work is
//!   O(petals), never O(cells) — so a setting may cost as much as it likes at
//!   build time and nothing at all after. Every setting here is therefore made
//!   of stipple and dashes rather than anything cheap-looking.
//! * **`scene` is the restore source, not a per-petal save.** Two petals in one
//!   cell would otherwise save each other's pixels and leave one behind — the
//!   trail bug in its usual costume. Restoring from the immutable scene makes
//!   overlap correct by construction, and makes a new setting free: the petal
//!   code never learns what is under it.
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
//! # The seed
//!
//! `SAKURA_SEED` defaults to 0, which means "pick one from the clock and the
//! pid" — so a pod restart shows a new tree in a new place. Any non-zero value
//! reproduces its draw exactly, which is how every test here pins a scene:
//! they call `build` with a literal seed rather than going through `new`.
//!
//! # Colour
//!
//! Night, not daylight: the panel's unlit pixels are black, so a dark sky costs
//! nothing and every lit thing is a silhouette against it. Blossom runs
//! `#FFD9E8` → `#9E5070` (near-white through to a dusty rose) because pink at
//! full saturation on black reads as neon; the trunk is `#584437`, a warm brown
//! dark enough to stay behind the blossom. The reflection is the same hues at
//! roughly 40% luminance and pulled toward the water's blue — a reflection at
//! the subject's brightness reads as a second tree. The ridges recede the same
//! way, each layer behind the last at roughly half its luminance, which is the
//! only depth cue available on a black panel.

use crate::font;
use crate::grid::{bake, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, env_str, next_rand};

#[rustfmt::skip]
const PAL_RGB: [[u8; 3]; 35] = [
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
    [0x1C, 0x22, 0x30], // 21 ridge, farthest — half the luminance of the next
    [0x32, 0x3A, 0x4E], // 22 ridge, middle
    [0x4E, 0x58, 0x70], // 23 ridge, nearest
    [0x3E, 0x48, 0x5C], // 24 mist lying in the valley
    [0x22, 0x1E, 0x22], // 25 the near spur, in shadow
    [0x50, 0x4A, 0x52], // 26 the near spur's lit crest
    [0x2A, 0x28, 0x26], // 27 gravel
    [0x6E, 0x6A, 0x62], // 28 a rake groove's lit ridge
    [0x7A, 0x76, 0x70], // 29 a set stone, lit top
    [0x2E, 0x2C, 0x2A], // 30 a set stone, in shadow
    [0x33, 0x4A, 0x2E], // 31 moss at a stone's foot
    [0x30, 0x2A, 0x26], // 32 the garden wall
    [0x55, 0x4C, 0x44], // 33 the wall's coping
    [0xC9, 0x92, 0xA6], // 34 a petal resting on gravel
];
const PAL: [u32; 35] = bake(&PAL_RGB);

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
/// Farthest to nearest. Each layer is roughly twice the luminance of the one
/// behind it; on a black panel that ratio is the entire sense of distance.
const RIDGE: [u16; 3] = [21, 22, 23];
const MIST: u16 = 24;
const SPUR: u16 = 25;
const SPUR_EDGE: u16 = 26;
const GRAVEL: u16 = 27;
const RAKE: u16 = 28;
const STONE_LIT: u16 = 29;
const STONE_DARK: u16 = 30;
const MOSS: u16 = 31;
const WALL: u16 = 32;
const WALL_TOP: u16 = 33;
const PETAL_DRY: u16 = 34;

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

/// Where a petal that reaches the ground ends up. The sub-row the rake leaves
/// its groove in, so a settled petal sits IN the groove rather than beside it.
/// Furrows are on odd rows below the ground line — see `garden`.
const GROOVE_SUB: i32 = 1;

/// Rings of rake raked round each set stone, four sub-columns apart.
const RINGS: i32 = 3;

/// Where the tree stands. The scene is otherwise identical: same tree, same
/// wind, same petals, and the setting only decides what is under and behind
/// them, and what a petal does when it arrives.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Setting {
    Pond,
    Mountain,
    Garden,
}

impl Setting {
    const ALL: [Setting; 3] = [Setting::Pond, Setting::Mountain, Setting::Garden];

    /// `SAKURA_SCENE` pins one; anything else (including a typo) draws one, on
    /// the same forgiving-by-default argument as `env_num`.
    fn pick(rng: &mut u32) -> Self {
        match env_str(&["SAKURA_SCENE"], "").as_str() {
            "pond" => Self::Pond,
            "mountain" => Self::Mountain,
            "garden" | "rock" => Self::Garden,
            _ => Self::ALL[(next_rand(rng) % 3) as usize],
        }
    }

    /// Percent of the panel above the ground line. The pond wants a low horizon
    /// so the lake is a lake; the mountain wants a high one so there is a drop
    /// under the spur for petals to blow out over.
    fn horizon_pct(self) -> i64 {
        match self {
            Self::Pond => 64,
            Self::Mountain => 56,
            Self::Garden => 58,
        }
    }
}

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
    /// 0 while airborne, else frames since it came to rest.
    rest: u32,
}

const NOWHERE: u32 = u32::MAX;

pub struct Sakura {
    grid: Grid,
    /// The still scene: tree, ground, whatever is behind it. The restore source
    /// for every cell a petal vacates, and never written after `build`.
    scene: Vec<Cell>,
    /// Cells a petal may detach from — the blossom mass.
    blossom: Vec<u32>,
    petals: Vec<Petal>,
    /// This frame's changed cells. Reserved in `build`, only ever cleared.
    dirty: Vec<u32>,
    setting: Setting,
    /// The horizon: first grid row that is ground under the tree.
    ground_y: usize,
    /// First grid row that is ground in EACH column, or `rows` where there is
    /// none. Flat at `ground_y` for the pond and the garden; on the mountain it
    /// follows the spur's crest down and then falls off it, which is what makes
    /// a petal settle on the rock in one column and go over the edge in the
    /// next.
    ground_row: Vec<u16>,
    /// What a petal at rest is painted in, and for how many frames it shows a
    /// splash first — zero anywhere there is no water to splash.
    at_rest: u16,
    ripple_frames: u32,
    /// Steady drift and gust amplitude, 8.8 cells per frame.
    wind_base: i32,
    wind_amp: i32,
    /// Periods of the two gust oscillators in frames. Deliberately
    /// incommensurate, so the pattern does not repeat on a countable cycle.
    gust_p: i32,
    gust_q: i32,
    /// Frames a landed petal lies there before it recycles.
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
// Scene construction. All of this runs once, in `build`.
// ---------------------------------------------------------------------------

/// Sub-cell raster the scene is composed in: one byte of braille bits and one
/// colour per cell, so wood, blossom, water and reflection can overdraw each
/// other before anything becomes a `Cell`.
struct Raster {
    cols: i32,
    rows: i32,
    bits: Vec<u8>,
    col: Vec<u16>,
    /// Branch endpoints of the tree being grown, where blossom clusters are
    /// stamped. Cleared per tree: a second, smaller tree tiered against the
    /// first one's height comes out uniformly dark.
    tips: Vec<(i32, i32)>,
    shape: Shape,
}

/// How one tree grows. Every field is drawn from a bounded range in `plant`,
/// and the bounds are the whole reason a random tree is still a cherry tree —
/// see the comment there for where each came from.
#[derive(Clone, Copy)]
struct Shape {
    /// First fork's half-angle off its parent, and how much it varies, in
    /// 1024-turn units. That fork is what sets how broad the crown is.
    spread0: i32,
    spread0_var: i32,
    /// The same for every fork above the first.
    spread: i32,
    spread_var: i32,
    /// Extra angle toward the horizontal per generation: the droop of a laden
    /// cherry branch. Without it the whole tree grows off the top of the panel.
    droop: i32,
    /// Child length as a percent of its parent, off the trunk and above it.
    scale0: i32,
    scale: i32,
    /// Limbs off the trunk.
    kids0: i32,
}

impl Raster {
    fn new(cols: usize, rows: usize) -> Self {
        Self {
            cols: cols as i32,
            rows: rows as i32,
            bits: vec![0; cols * rows],
            col: vec![0; cols * rows],
            tips: Vec::new(),
            shape: Shape {
                spread0: 70,
                spread0_var: 120,
                spread: 60,
                spread_var: 90,
                droop: 20,
                scale0: 88,
                scale: 68,
                kids0: 4,
            },
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
    /// The trunk forks into three or four main limbs rather than two, so the
    /// crown is wide instead of a Y; and every generation is pulled further
    /// toward the horizontal, because a laden cherry branch droops.
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
        let s = self.shape;
        let kids = if d == 0 {
            s.kids0
        } else {
            2 + (next_rand(rng) % 2) as i32
        };
        for k in 0..kids {
            let sign = if k % 2 == 0 { 1 } else { -1 };
            // Wider off the trunk than anywhere else: that first fork sets how
            // broad the crown is, and everything above it only refines it.
            let spread = if d == 0 {
                s.spread0 + (next_rand(rng) % s.spread0_var as u32) as i32
            } else {
                s.spread + (next_rand(rng) % s.spread_var as u32) as i32
            };
            let mut a = ang + sign * spread + (next_rand(rng) % 27) as i32 - 13;
            a += (a - 256).signum() * (s.droop * d as i32);
            let scale = if d == 0 { s.scale0 } else { s.scale };
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

/// Grow one tree from `(x, y)` and hang its blossom on it.
///
/// The bounds on every drawn parameter are the point of this function. They
/// were chosen against the fixed tree the saver used to draw, which is the one
/// known-good sample: each range brackets that tree's value by about as much as
/// still reads as a cherry, and no further. Widening `droop` past ~30 drops the
/// outer branches below horizontal and the crown collapses into the trunk;
/// narrowing it past ~12 grows the tree straight off the top of the panel.
/// `kids0` never goes below 3 because two limbs off the trunk is a Y, not a
/// crown. `spread0` below ~55 gives a poplar and above ~230 a hedge.
#[allow(clippy::too_many_arguments)]
fn plant(
    r: &mut Raster,
    rng: &mut u32,
    x: i32,
    y: i32,
    len: i32,
    lean: i32,
    thick: i32,
    bloom: usize,
) {
    r.shape = Shape {
        spread0: 55 + (next_rand(rng) % 30) as i32,
        spread0_var: 95 + (next_rand(rng) % 55) as i32,
        spread: 50 + (next_rand(rng) % 25) as i32,
        spread_var: 70 + (next_rand(rng) % 45) as i32,
        droop: 13 + (next_rand(rng) % 16) as i32,
        scale0: 80 + (next_rand(rng) % 14) as i32,
        scale: 62 + (next_rand(rng) % 13) as i32,
        kids0: 3 + (next_rand(rng) % 100 / 70) as i32,
    };

    // Two segments with a kink: a dead-straight trunk reads as a mast. The
    // upper segment straightens toward the vertical, which is what a leaning
    // tree does — it grows back up toward the light.
    r.tips.clear();
    let mid = len / 2;
    let a0 = 256 - lean;
    let kx = x + ((cos1024(a0) * mid) >> 10);
    let ky = y - ((sin1024(a0) * mid * ASPECT_NUM / ASPECT_DEN) >> 10);
    r.limb(x, y, kx, ky, thick, thick * 4 / 5, TRUNK);
    r.branch(
        kx,
        ky,
        256 - lean / 4,
        (len - mid).max(4),
        (thick * 4 / 5).max(1),
        0,
        rng,
    );

    // One limb left bare, some of the time. The tips of a whole subtree are a
    // CONTIGUOUS run in `tips` because `branch` recurses depth-first, so
    // dropping a run is exactly "this limb carries no blossom".
    let n = r.tips.len();
    if n > 12 && next_rand(rng) % 100 < 40 {
        let cut = n / 7 + (next_rand(rng) as usize % (n / 7).max(1));
        let at = next_rand(rng) as usize % (n - cut).max(1);
        r.tips.drain(at..(at + cut).min(n));
    }

    // Stamped around the branch tips rather than into a free-floating ellipse,
    // so the crown sits on the structure that holds it up.
    if r.tips.is_empty() {
        return;
    }
    let (mut lo, mut hi) = (i32::MAX, i32::MIN);
    for &(_, ty) in &r.tips {
        lo = lo.min(ty);
        hi = hi.max(ty);
    }
    let span = (hi - lo).max(1);
    for _ in 0..bloom {
        let t = r.tips[next_rand(rng) as usize % r.tips.len()];
        let sx = t.0 + tri(rng, 4);
        let sy = t.1 + tri(rng, 6);
        // Tier by height in the crown, jittered by one so the bands do not
        // become visible stripes. Weighted to the middle of the ramp: an even
        // split across the four tiers makes the crown one flat pale mass,
        // because the two pale tiers are half of it.
        const TIER: [usize; 6] = [0, 1, 1, 2, 2, 3];
        let t = ((sy - lo) * 6 / span + (next_rand(rng) % 3) as i32 - 1).clamp(0, 5) as usize;
        r.px(sx, sy, BLOSSOM[TIER[t]]);
    }
}

/// A sparse scatter of horizontal dashes across one cell row. Horizontal
/// because a vertical mark in water — or in mist, or in gravel — reads as a
/// post. Sparse and STREAKY: a dash per cell at any real density reads as
/// static; a few long ones per row read as a swell.
fn dash_row(r: &mut Raster, rng: &mut u32, cy: i32, density: u32, c: u16) {
    let mut cx = 0i32;
    while cx < r.cols {
        if next_rand(rng) % 100 >= density {
            cx += 1;
            continue;
        }
        let sy = cy * 4 + (next_rand(rng) % 4) as i32;
        let run = 3 + (next_rand(rng) % 9) as i32;
        for o in 0..run {
            r.px(cx * 2 + o, sy, c);
        }
        cx += run / 2 + 2;
    }
}

/// One ridgeline as a sub-row per sub-column: control points every `step`
/// sub-columns, linearly interpolated, plus a finer octave a third the scale.
/// Two octaves is what separates a mountain from a sine wave; a random walk
/// instead gives noise with no peaks in it at all.
fn ridgeline(rng: &mut u32, n: usize, base: i32, amp: i32, step: usize) -> Vec<i32> {
    let amp = amp.max(2);
    let lerp = |p: &[i32], s: usize, i: usize| {
        let (a, f) = (i / s, (i % s) as i32);
        p[a] + (p[a + 1] - p[a]) * f / s as i32
    };
    let s1 = step.max(2);
    let p1: Vec<i32> = (0..n / s1 + 2)
        .map(|_| -((next_rand(rng) % (amp as u32 + 1)) as i32))
        .collect();
    let s2 = (s1 / 3).max(2);
    let p2: Vec<i32> = (0..n / s2 + 2)
        .map(|_| -((next_rand(rng) % (amp as u32 / 3 + 1)) as i32))
        .collect();
    (0..n)
        .map(|i| base + lerp(&p1, s1, i) + lerp(&p2, s2, i))
        .collect()
}

/// Paint one ridge: a two-sub-row crest, then stipple below it thinning with
/// depth. The thinning is the whole effect — a solid fill would be a black
/// cut-out, and a uniform stipple a grey rectangle; a crest that dissolves
/// downward reads as a slope going into haze.
fn ridge_fill(r: &mut Raster, rng: &mut u32, ys: &[i32], floor: i32, top: u32, c: u16) {
    for (sx, &y) in ys.iter().enumerate() {
        let sx = sx as i32;
        r.px(sx, y, c);
        r.px(sx, y + 1, c);
        for sy in y + 2..floor {
            let d = top.saturating_sub((sy - y) as u32 * top / 18).max(2);
            if next_rand(rng) % 100 < d {
                r.px(sx, sy, c);
            }
        }
    }
}

/// A set stone: a parabolic dome on a flat base, lit across the top third.
/// Not an ellipse — a stone chosen for a karesansui is set so that only its
/// shoulder shows, and a full ellipse reads as an egg on the gravel.
fn stone(r: &mut Raster, sx0: i32, sy0: i32, rx: i32, ry: i32) {
    let ry = ry.max(2);
    for dy in -ry..=0 {
        let hw = rx * (ry * ry - dy * dy) / (ry * ry);
        let c = if dy < -ry / 2 { STONE_LIT } else { STONE_DARK };
        for o in -hw..=hw {
            r.px(sx0 + o, sy0 + dy, c);
        }
    }
}

/// A dashed ring of raked gravel around a stone. Dashed rather than solid
/// because the rake leaves ridges, not a line, and because a solid ellipse
/// closes into a shape the eye reads as an object.
fn rake_ring(r: &mut Raster, sx0: i32, sy0: i32, rx: i32) {
    // 3:2 converts sub-columns to sub-rows for a circle in PIXELS, halved
    // again for the angle the garden is seen from.
    let ry = (rx * ASPECT_DEN / ASPECT_NUM).max(1);
    for a in (0..1024).step_by(5) {
        if (a / 40) % 2 == 1 {
            continue;
        }
        r.px(
            sx0 + ((cos1024(a) * rx) >> 10),
            sy0 + ((sin1024(a) * ry) >> 10),
            RAKE,
        );
    }
}

impl Sakura {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        // 0 means "a different scene every time the pod starts". The clock
        // alone is a poor seed — two pods started in the same second would
        // draw the same tree — so the pid goes in too, and the pair is mixed
        // rather than used raw.
        let mut seed = env_num(&["SAKURA_SEED"], 0, 0, u32::MAX as i64) as u32;
        if seed == 0 {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos() ^ d.as_secs() as u32)
                .unwrap_or(0x5a_2a_91_7d);
            seed = nanos ^ std::process::id().wrapping_mul(0x9E37_79B9);
            next_rand(&mut seed);
            seed = seed.max(1);
        }
        let mut pick = seed;
        let setting = Setting::pick(&mut pick);
        Self::build(panel, fps, seed, setting)
    }

    /// The seed and the setting come in as arguments so the tests can pin a
    /// scene without `set_var`: cargo runs tests in parallel threads and the
    /// environment is process-wide, so one test's setting would land in
    /// another's saver.
    fn build(panel: &Panel, fps: u32, seed: u32, setting: Setting) -> Self {
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
        // How long a petal lies where it landed before it is recycled to the
        // crown. It is also what sets how many petals are in the AIR: a petal
        // spends its fall airborne and this at rest, so a long rest empties
        // the sky and piles every petal onto the same few rows of ground.
        let rest_frames = env_num(&["SAKURA_REST_SECS"], 5, 1, 3600) as u32 * fps as u32;
        let ripple_ms = env_num(&["SAKURA_RIPPLE_MS"], 420, 0, 10_000) as u32;
        let n_stars = env_num(&["SAKURA_STARS"], 110, 0, 20_000) as usize;
        // Where the horizon sits. Low enough that the ground is ground and not
        // a puddle, high enough that the tree has room to be a tree.
        let pct = env_num(
            &["SAKURA_HORIZON_PCT", "SAKURA_WATER_PCT"],
            setting.horizon_pct(),
            10,
            95,
        );
        let ground_y = (rows * pct as usize / 100).max(1);

        let mut rng = seed;
        let mut r = Raster::new(cols, rows);
        let subcols = cols * 2;

        // --- what is behind the tree ------------------------------------
        if setting == Setting::Mountain {
            // Three ridges receding. Each is higher, dimmer and smoother than
            // the one in front of it, which is the only depth cue a black
            // panel offers.
            let floor = ground_y as i32 * 4;
            let h = rows as i32 * 4;
            for (k, &c) in RIDGE.iter().enumerate() {
                let near = k as i32;
                let base = floor - h * (18 - near * 6) / 100;
                let amp = h * (13 - near * 3) / 100;
                let step = (26 - near * 6) as usize;
                let ys = ridgeline(&mut rng, subcols, base, amp, step);
                ridge_fill(&mut r, &mut rng, &ys, floor, 45 + near as u32 * 8, c);
            }
            // Mist lying in the valley: the lake's dashes, in a band, which is
            // the same trick doing a different job.
            let band = (rows / 12).max(2);
            for cy in ground_y.saturating_sub(band + 1)..ground_y.saturating_sub(1) {
                dash_row(&mut r, &mut rng, cy as i32, 16, MIST);
            }
        }

        // --- the tree ---------------------------------------------------
        // Rooted on the ground line and leaning AWAY from the nearer edge, so
        // the crown always has panel to spread into — and, beside a pond, so
        // the canopy sits over its own reflection.
        let trunk_x = (cols as i32 * (26 + (next_rand(&mut rng) % 17) as i32) / 100) * 2;
        let trunk_y = ground_y as i32 * 4;
        // A SHORT trunk. The crown, not the trunk, is what has to fill the
        // upper half — a trunk that reaches two thirds of the way up pushes
        // every branch off the top of the panel. Measured from the GROUND line
        // rather than from the panel, or a setting with a high horizon gets a
        // stunted tree for free.
        let top = ground_y as i32 * (55 + (next_rand(&mut rng) % 20) as i32) / 100;
        let trunk_len = (((ground_y as i32 - top) * 4) * ASPECT_DEN / ASPECT_NUM).max(8);
        let thick = ((cell_w as i32 / 2).clamp(4, 9) * (80 + (next_rand(&mut rng) % 46) as i32)
            / 100)
            .clamp(3, 11);
        let away = if trunk_x * 2 < subcols as i32 { 1 } else { -1 };
        let lean = away * (8 + (next_rand(&mut rng) % 19) as i32);
        // Roughly three quarters of a dot per CELL of panel. Below about half
        // that the crown stops being a mass and goes back to confetti.
        let bloom_pct = env_num(&["SAKURA_BLOOM"], 100, 0, 400) as usize;
        let dots =
            cols * rows * 3 / 4 * bloom_pct / 100 * (80 + next_rand(&mut rng) as usize % 46) / 100;
        plant(
            &mut r, &mut rng, trunk_x, trunk_y, trunk_len, lean, thick, dots,
        );

        // A sapling on the far side, sometimes. Downwind of the big tree and
        // well short of it: a second tree of the same height reads as a
        // repeated sprite rather than as a grove.
        if next_rand(&mut rng) % 100 < 35 && !(setting == Setting::Mountain && trunk_x < 24) {
            // Downwind of the big tree — except on the mountain, where the
            // only ground is UPHILL of the trunk and a sapling placed downwind
            // stands in mid air over the drop.
            let far = if setting == Setting::Mountain {
                (next_rand(&mut rng) as i32).rem_euclid(trunk_x - 16) + 6
            } else if away > 0 {
                subcols as i32 * (72 + (next_rand(&mut rng) % 20) as i32) / 100
            } else {
                subcols as i32 * (8 + (next_rand(&mut rng) % 20) as i32) / 100
            };
            let len = (trunk_len * (32 + (next_rand(&mut rng) % 19) as i32) / 100).max(6);
            let sap_lean = -away * (6 + (next_rand(&mut rng) % 15) as i32);
            let sap_thick = (thick * 3 / 5).max(3);
            plant(
                &mut r,
                &mut rng,
                far,
                trunk_y,
                len,
                sap_lean,
                sap_thick,
                dots * 30 / 100,
            );
        }

        // --- the stars --------------------------------------------------
        // Only in empty sky: a star inside the canopy, or on a ridge, would
        // recolour the whole cell and punch a grey hole in it.
        for _ in 0..n_stars {
            let cx = next_rand(&mut rng) as i32 % cols.max(1) as i32;
            let cy = next_rand(&mut rng) as i32 % ground_y.max(1) as i32;
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

        // --- the ground, which is drawn OVER the tree --------------------
        // The trunk's foot disappears into the water, the gravel or the rock,
        // which is what standing in something looks like.
        let mut ground_row = vec![ground_y as u16; cols];
        match setting {
            Setting::Pond => Self::lake(&mut r, &mut rng, ground_y, rows),
            Setting::Mountain => Self::spur(
                &mut r,
                &mut rng,
                trunk_x,
                ground_y,
                rows,
                subcols,
                &mut ground_row,
            ),
            Setting::Garden => Self::garden(&mut r, &mut rng, ground_y, rows, subcols),
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
            setting,
            ground_y,
            ground_row,
            at_rest: match setting {
                Setting::Pond => PETAL_WET,
                _ => PETAL_DRY,
            },
            ripple_frames: match setting {
                Setting::Pond => ripple_ms * fps as u32 / 1000,
                _ => 0,
            },
            wind_base: per_frame(base_cps),
            wind_amp: per_frame(gust_cps),
            gust_p: gust_p.max(1),
            gust_q: gust_q.max(1),
            rest_frames,
            fall: per_frame(fall_cps).max(1),
            sway_amp_units: sway_cps * FIX / 100,
            sway_rate_units: (1024 * sway_rate / (10 * fps)).max(1),
            spin_rate_units: (1024 * 1000 / (spin_ms * fps)).max(1),
            frame: 0,
            rng,
        };
        let last_col = cols as i32 - 1;
        for k in 0..n_petals {
            me.respawn(k);
            // Stagger BOTH phases, or the tree sheds its whole crown on frame 1
            // and then the whole crown lands on the same frame five seconds
            // later — which looks exactly like a bug and is one.
            // Through the sway, exactly as `render` does: the column a petal
            // is PAINTED in is the one whose ground has to be under it, and on
            // the spur's crest the neighbouring column is two rows lower.
            let q = me.petals[k];
            let cx = (q.x + ((sin1024(q.sway) * q.sway_amp) >> 10))
                .div_euclid(FIX)
                .clamp(0, last_col);
            let grounded = (me.ground_row[cx as usize] as usize) < rows;
            if grounded && next_rand(&mut me.rng) % 100 < 35 {
                me.petals[k].rest = 1 + next_rand(&mut me.rng) % rest_frames.max(1);
                me.land(k, cx);
            } else {
                let drop = (me.ground_y as i32 * FIX - me.petals[k].y).max(1);
                me.petals[k].y += (next_rand(&mut me.rng) as i32).rem_euclid(drop);
            }
        }
        me
    }

    /// A lake: a surface line, a thinning scatter of swell, and the tree
    /// mirrored into it.
    fn lake(r: &mut Raster, rng: &mut u32, ground_y: usize, rows: usize) {
        let cols = r.cols;
        // The surface line first: a full-width bar one sub-row tall, which is
        // the single strongest cue that the bottom of the panel is water.
        for cx in 0..cols {
            r.px(cx * 2, ground_y as i32 * 4, WATER_LINE);
            r.px(cx * 2 + 1, ground_y as i32 * 4, WATER_LINE);
        }
        for cy in (ground_y + 1)..rows {
            let k = cy - ground_y;
            let band = (k * 3 / (rows - ground_y).max(1)).min(2);
            let density = 13u32.saturating_sub(k as u32 / 3).max(3);
            dash_row(r, rng, cy as i32, density, WATER[band]);
        }

        // A vertical mirror about the waterline with a per-row horizontal
        // shear, which is the cheapest thing that reads as a rippled surface.
        // Rows further from the line are sheared further and dissolve, so the
        // reflection fades into open water instead of stopping dead.
        for k in 0..(rows - ground_y - 1) {
            let dst_y = ground_y + 1 + k;
            let Some(src_y) = ground_y.checked_sub(1 + k) else {
                break;
            };
            let amp = 1 + (k as i32) / 5;
            let jit = ((sin1024(k as i32 * 167 + 240) * amp) >> 10).clamp(-4, 4);
            // Past a third of the way down the reflection breaks up.
            let keep =
                100u32.saturating_sub(k as u32 * 100 / (rows - ground_y).max(1) as u32 * 2 / 3);
            for cx in 0..cols {
                let sx = cx + jit;
                if sx < 0 || sx >= cols {
                    continue;
                }
                let si = (src_y as i32 * cols + sx) as usize;
                let rc = reflect(r.col[si]);
                if rc == 0 || r.bits[si] == 0 {
                    continue;
                }
                if next_rand(rng) % 100 >= keep {
                    continue;
                }
                let di = (dst_y as i32 * cols + cx) as usize;
                r.bits[di] |= flip_bits(r.bits[si]);
                r.col[di] = rc;
            }
        }
    }

    /// The near spur the tree stands on: a lit crest that runs in from the left
    /// edge, crosses under the trunk, and falls away to nothing part way across
    /// the panel. What is to the RIGHT of where it ends is the point of the
    /// setting — open air, all the way down, for the petals to blow out over.
    #[allow(clippy::too_many_arguments)]
    fn spur(
        r: &mut Raster,
        rng: &mut u32,
        trunk_x: i32,
        ground_y: usize,
        rows: usize,
        n: usize,
        ground_row: &mut [u16],
    ) {
        let (g, bottom) = (ground_y as i32 * 4, rows as i32 * 4);
        let end = (n as i32 * (58 + (next_rand(rng) % 21) as i32) / 100).max(trunk_x + 8);
        let rise = (bottom - g) * 12 / 100;
        let bumps = ridgeline(rng, n, 0, 5, 11);
        // Every column starts as open air; the crest below writes back the row
        // it actually covers, so a petal knows which columns can catch it.
        ground_row.fill(rows as u16);
        for sx in 0..end.min(n as i32) {
            let y = if sx <= trunk_x {
                g - rise * (trunk_x - sx) / trunk_x.max(1)
            } else {
                g + (bottom - g) * (sx - trunk_x) / (end - trunk_x).max(1)
            } + bumps[sx as usize];
            r.px(sx, y, SPUR_EDGE);
            r.px(sx, y + 1, SPUR_EDGE);
            let col = &mut ground_row[(sx / 2).clamp(0, r.cols - 1) as usize];
            *col = (*col).min((y.div_euclid(4).clamp(0, rows as i32 - 1)) as u16);
            for sy in y + 2..bottom {
                let d = 70u32.saturating_sub((sy - y) as u32 * 70 / 30).max(5);
                if next_rand(rng) % 100 < d {
                    r.px(sx, sy, SPUR);
                }
            }
        }
    }

    /// A karesansui: a wall at the back, a bed of raked gravel, set stones with
    /// the rake carried round them in rings, and moss at one stone's foot.
    ///
    /// The bed alternates furrow row and gravel row because a cell holds ONE
    /// colour: a furrow in every row would claim every cell in the bed for the
    /// rake, and the gravel between the furrows — which is the whole contrast
    /// the pattern is made of — would have nowhere to be.
    fn garden(r: &mut Raster, rng: &mut u32, ground_y: usize, rows: usize, n: usize) {
        let g = ground_y as i32;
        // The wall, and its coping — the lit line along the top is what makes
        // the band read as a wall rather than as a smudge.
        for cy in g.saturating_sub(3)..g {
            dash_row(r, rng, cy, 70, WALL);
        }
        for sx in 0..n as i32 {
            r.px(sx, (g - 3).max(0) * 4, WALL_TOP);
            r.px(sx, g * 4, RAKE);
        }

        // Stones first: the rake runs round them, so where they are decides
        // where the straight furrows have to stop.
        let bed = (rows as i32 - g).max(2);
        let mut set: Vec<(i32, i32, i32, i32)> = Vec::new();
        for _ in 0..2 + (next_rand(rng) % 2) as i32 {
            let cy = g + 2 + (next_rand(rng) as i32).rem_euclid((bed - 3).max(1));
            let sx = n as i32 * (12 + (next_rand(rng) % 70) as i32) / 100;
            // Nearer the viewer is bigger, which is the only perspective the
            // flat bed gets.
            let rx = 6 + (cy - g) * 9 / bed;
            set.push((sx, cy * 4 + 2, rx, rx + 4 * RINGS));
        }
        // Inside a stone's rings the gravel is raked round it, not across.
        let clear_of_stones = |sx: i32, sy: i32| {
            set.iter().all(|&(x, y, _, out)| {
                let (dx, dy) = (sx - x, (sy - y) * ASPECT_NUM / ASPECT_DEN);
                dx * dx + dy * dy * 9 / 4 > out * out
            })
        };

        for cy in g + 1..rows as i32 {
            if (cy - g) % 2 == 0 {
                // A gravel row: a sparse speckle, so the eye reads texture
                // between the furrows rather than a flat band.
                for _ in 0..r.cols {
                    r.px(
                        next_rand(rng) as i32 % n as i32,
                        cy * 4 + (next_rand(rng) % 4) as i32,
                        GRAVEL,
                    );
                }
                continue;
            }
            // A furrow row: one unbroken line, which is what a rake leaves and
            // what a dashed line does not. Straight, because a karesansui's
            // "water" is raked in parallel lines everywhere the stones allow.
            let sy = cy * 4 + GROOVE_SUB;
            for sx in 0..n as i32 {
                if clear_of_stones(sx, sy) {
                    r.px(sx, sy, RAKE);
                }
            }
        }

        for &(sx, sy, rx, _) in &set {
            for ring in 1..=RINGS {
                rake_ring(r, sx, sy, rx + ring * 4);
            }
            stone(r, sx, sy, rx, rx.max(4));
        }
        // Moss at the foot of the first stone only: moss on every stone reads
        // as a colour wash rather than as one damp corner of the garden.
        if let Some(&(sx, sy, rx, _)) = set.first() {
            for _ in 0..18 {
                r.px(
                    sx + tri(rng, rx + 3),
                    sy + 1 + (next_rand(rng) % 2) as i32,
                    MOSS,
                );
            }
        }
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

    /// Put a petal on the ground. Spread over several rows and sub-rows rather
    /// than snapped to the ground line, or every petal that ever lands stacks
    /// into one solid bar of pink across the scene. On gravel it spreads over
    /// the whole bed and snaps to the rake's groove, which is where a petal
    /// blown across raked gravel actually stops.
    fn land(&mut self, k: usize, cx: i32) {
        let ground = self.ground_y as i32;
        let rows = self.grid.rows() as i32;
        let (row, sub) = if self.setting == Setting::Mountain {
            // On the crest, exactly where it is: the spur's line is the shape
            // the eye is following, and a scattered petal would blur it.
            (
                self.ground_row[cx.clamp(0, self.grid.cols() as i32 - 1) as usize] as i32,
                (next_rand(&mut self.rng) % 4) as i32,
            )
        } else if self.setting == Setting::Garden {
            // Odd rows below the ground line are the furrow rows, so this
            // lands the petal IN a groove rather than on the gravel beside it.
            let bed = ((rows - ground) / 2).max(1);
            (
                ground + 1 + 2 * (next_rand(&mut self.rng) as i32).rem_euclid(bed),
                GROOVE_SUB,
            )
        } else {
            (
                ground + (next_rand(&mut self.rng) % 5) as i32,
                (next_rand(&mut self.rng) % 4) as i32,
            )
        };
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

        for k in 0..self.petals.len() {
            let p = self.petals[k];
            let mut p = p;

            if p.rest == 0 {
                p.y += p.vy;
                p.x += (w * p.drag) >> 8;
                p.sway = (p.sway + p.sway_rate).rem_euclid(1024);
                p.spin = (p.spin + p.spin_rate).rem_euclid(1024);
            } else {
                // At rest: nudged along by the surface, which is slower than
                // the air above it — but only where the surface actually
                // carries it. Water does, loose gravel does, and rock does
                // not: a petal creeping along the spur's crest would drift
                // out over the drop and hang there.
                if self.setting != Setting::Mountain {
                    p.x += (w * p.drag) >> 10;
                }
                p.rest += 1;
            }

            // The sway is an offset, never accumulated — see the module doc.
            let dx = p.x + ((sin1024(p.sway) * p.sway_amp) >> 10);
            // A petal edge-on to its fall stalls a little. Two sway cycles per
            // tumble, so the stall lands on the flicker.
            let dy = p.y + ((sin1024(p.sway * 2) * p.sway_amp) >> 12);

            // Off the side, out of time, or — over the mountain's drop, where
            // no column has any ground in it — clean off the bottom.
            let cx = dx.div_euclid(FIX);
            if cx < 0 || cx >= cols || p.rest > self.rest_frames || p.y.div_euclid(FIX) >= rows {
                self.petals[k] = p;
                self.respawn(k);
                continue;
            }
            // No ground in this column, no landing — the petal keeps going and
            // recycles off the bottom next frame. `g == rows` is the drop, and
            // without this guard the stall offset lets a petal one dot short of
            // the bottom row "land" on the last row of empty air.
            let g = self.ground_row[cx as usize] as i32;
            if p.rest == 0 && g < rows && dy >= g * FIX {
                p.rest = 1;
                self.petals[k] = p;
                self.land(k, cx);
                p = self.petals[k];
            }
            let cy = p.y.div_euclid(FIX).min(rows - 1);

            let (gx, gy) = if p.rest == 0 { (dx, dy) } else { (dx, p.y) };
            let sx = (gx.div_euclid(FIX / 2)).rem_euclid(2);
            let sy = (gy.div_euclid(FIX / 4)).rem_euclid(4);
            let bits = if p.rest == 0 {
                Self::petal_bits(p.spin, sx, sy)
            } else {
                // At rest it lies flat: a horizontal pair, which also happens
                // to be the shape of the ripple it arrived in.
                dot(0, sy) | dot(1, sy)
            };
            let colour = if p.rest > 0 && p.rest <= self.ripple_frames {
                RIPPLE
            } else if p.rest > 0 {
                self.at_rest
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

    /// Seeds the scene tests sweep. Random generation only fails on SOME draws,
    /// so a single seed proves a single tree; these are arbitrary constants,
    /// fixed so a failure is reproducible.
    const SEEDS: [u32; 8] = [
        1,
        0x5a_2a_91_7d,
        0xDEAD_BEEF,
        7,
        0x0123_4567,
        0xFFFF_FFFF,
        0x9E37_79B9,
        1234,
    ];

    fn panel() -> Panel {
        Panel::new(1920, 1080, 1920)
    }

    fn at(seed: u32, setting: Setting) -> Sakura {
        Sakura::build(&panel(), 30, seed, setting)
    }

    fn sakura() -> Sakura {
        at(0x5a_2a_91_7d, Setting::Pond)
    }

    fn cells(s: &Sakura) -> usize {
        s.grid.cols() * s.grid.rows()
    }

    /// Relative luminance, near enough for "is this darker than that".
    fn lum(c: u16) -> u32 {
        let [r, g, b] = PAL_RGB[c as usize];
        r as u32 * 30 + g as u32 * 59 + b as u32 * 11
    }

    /// Cells of each colour in the scene, split at the ground line.
    fn census(s: &Sakura) -> ([usize; 35], [usize; 35]) {
        let cols = s.grid.cols();
        let (mut above, mut below) = ([0usize; 35], [0usize; 35]);
        for (i, c) in s.scene.iter().enumerate() {
            if *c == Cell::CLEAR {
                continue;
            }
            if i / cols < s.ground_y {
                above[c.colour()] += 1;
            } else {
                below[c.colour()] += 1;
            }
        }
        (above, below)
    }

    /// The bottom strip below the last cell row is a real error line on the
    /// panel, and a frame-0 assertion that only checks the cells cannot see it.
    /// The pixel count is what stops this passing on a black panel.
    #[test]
    fn frame_zero_paints_the_whole_panel_and_shows_a_scene() {
        for setting in Setting::ALL {
            let p = panel();
            let mut s = at(SEEDS[1], setting);
            let mut buf = vec![0u32; p.buf_len()];
            let d = saver::frame(&mut s, &mut buf, &p);
            assert_eq!(d.rows(), p.h, "frame 0 must paint every scanline");
            let lit = buf.iter().filter(|&&px| px != 0).count();
            assert!(
                lit > 50_000,
                "{setting:?}: frame 0 painted {lit} lit pixels: not a scene"
            );
            for i in 0..cells(&s) {
                assert_eq!(s.grid.cell(i), s.grid.cells()[i], "frame 0 left {i} stale");
            }
        }
    }

    /// The `flush_sparse` under-report bug, which a framebuffer diff is
    /// structurally blind to: an unreported write is never blitted, so the
    /// framebuffer never changes and the diff finds nothing. `cur` and `prev`
    /// are identical after every flush, so a cell written and left out of
    /// `dirty` is a mismatch HERE even though the panel would just freeze.
    #[test]
    fn every_written_cell_is_reported() {
        for setting in Setting::ALL {
            let p = panel();
            let mut s = at(SEEDS[1], setting);
            let mut buf = vec![0u32; p.buf_len()];
            let mut prev = buf.clone();
            saver::frame(&mut s, &mut buf, &p);

            let n = cells(&s);
            let mut rows = Vec::new();
            let mut moved = 0;
            for f in 1..700 {
                prev.copy_from_slice(&buf);
                let d = saver::frame(&mut s, &mut buf, &p);
                for i in 0..n {
                    assert_eq!(
                        s.grid.cell(i),
                        s.grid.cells()[i],
                        "{setting:?} frame {f}: cell {i} was written but not reported"
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
                        "{setting:?} frame {f}: scanline {y} changed but was not reported"
                    );
                }
                moved += usize::from(changed > 0);
                rows.push(d.rows());
            }
            assert!(
                moved > 650,
                "{setting:?}: the petals stalled ({moved}/699 frames moved)"
            );

            // The point of the saver: a repaint regression is what this
            // catches. Petals are scattered down the whole panel, so
            // scanline-granular damage is inherently coarse — the bound is
            // what was MEASURED with a margin, not an aspiration.
            rows.sort_unstable();
            let median = rows[rows.len() / 2];
            assert!(
                median < p.h * 3 / 4,
                "{setting:?}: median damage {median} of {} scanlines: the scene is not static",
                p.h
            );
        }
    }

    /// A petal must leave NOTHING behind. Exact, cell for cell, against the
    /// still scene — not a ratio: trails saturate rather than growing without
    /// bound, so a ratio test waves a real trail straight through.
    #[test]
    fn a_petal_restores_exactly_what_was_under_it() {
        for setting in Setting::ALL {
            let p = panel();
            let mut s = at(SEEDS[1], setting);
            let mut buf = vec![0u32; p.buf_len()];
            let n = cells(&s);
            let mut occupied = vec![false; n];
            let mut over_scene = 0usize;

            for f in 0..600 {
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
                        "{setting:?} frame {f}: cell {i} is not the scene — a petal left a trail"
                    );
                }
            }
            // Non-vacuous: petals must actually have crossed painted scene
            // cells, or "nothing left behind" is "nothing happened".
            assert!(
                over_scene > 5_000,
                "{setting:?}: petals only covered {over_scene} painted cells: nothing to restore"
            );
        }
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

    /// Where a petal ends up is the setting's business: it floats on the pond,
    /// it settles into a rake groove in the garden, and on the mountain it goes
    /// over the edge and off the bottom of the panel without ever resting.
    #[test]
    fn a_petal_ends_where_the_setting_says() {
        for setting in Setting::ALL {
            let p = panel();
            let mut s = at(SEEDS[1], setting);
            let mut buf = vec![0u32; p.buf_len()];
            let rows = s.grid.rows() as i32;
            let ground = s.ground_y as i32;
            let (mut rested, mut flew, mut below_ground) = (0, 0, 0);
            for f in 0..900 {
                saver::frame(&mut s, &mut buf, &p);
                let cols = s.grid.cols() as u32;
                for q in &s.petals {
                    if q.rest == 0 {
                        flew += 1;
                        below_ground += usize::from(q.y.div_euclid(FIX) >= ground);
                        continue;
                    }
                    rested += 1;
                    if q.at == NOWHERE {
                        continue;
                    }
                    // The cell it is PAINTED in, against that column's OWN
                    // ground rather than the horizon: the spur's crest rises
                    // above the horizon uphill of the tree and falls below it
                    // downhill, and a petal's sway puts it a column either
                    // side of where `x` alone says it is.
                    let (col, row) = ((q.at % cols) as usize, (q.at / cols) as i32);
                    let g = s.ground_row[col] as i32;
                    assert!(
                        (g..rows).contains(&row),
                        "{setting:?} frame {f}: a resting petal is at row {row}, \
                         column {col} has ground at {g}"
                    );
                    if setting == Setting::Garden {
                        assert_eq!(
                            q.y.rem_euclid(FIX) / (FIX / 4),
                            GROOVE_SUB,
                            "a settled petal missed the rake groove"
                        );
                    }
                }
            }
            assert!(flew > 10_000, "{setting:?}: hardly anything flew ({flew})");
            assert!(
                rested > 10_000,
                "{setting:?}: hardly anything landed ({rested})"
            );
            if setting == Setting::Mountain {
                // The point of the setting: past the end of the spur there is
                // no ground at all, and a petal that gets there keeps going.
                assert!(
                    below_ground > 5_000,
                    "only {below_ground} petal-frames below the horizon: \
                     nothing went over the edge"
                );
            }
        }
    }

    /// CLAUDE.md makes the frame loop the top constraint in this repo.
    /// Capacity, not length: a `Vec` that never grows past the capacity `build`
    /// reserved never reallocates, and `clear` keeps capacity.
    #[test]
    fn render_never_allocates() {
        for setting in Setting::ALL {
            let p = panel();
            let mut s = at(SEEDS[1], setting);
            let mut buf = vec![0u32; p.buf_len()];
            let reserved = s.dirty.capacity();
            assert!(reserved > 0, "nothing was reserved for the frame loop");
            let mut worst = 0;
            for _ in 0..6_000 {
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
                "{setting:?}: `dirty` never held a whole frame's worth ({worst})"
            );
        }
    }

    /// A cherry tree, on EVERY seed. The failure mode of a random generator is
    /// the one draw in twenty that comes out a shrub or a telegraph pole, so
    /// this is a sweep and not a sample: wood, a crown of the right size, all
    /// four shading tiers used, and a crown wider than it is tall.
    #[test]
    fn every_seed_grows_a_cherry_tree() {
        for setting in Setting::ALL {
            for seed in SEEDS {
                let s = at(seed, setting);
                let (above, _) = census(&s);
                let cols = s.grid.cols();
                // Wood is counted AFTER the blossom has overdrawn it, so a
                // densely flowering tree shows very little: measured 22..216
                // over these seeds, and the trunk alone 21..55. Both bounds
                // are the measured floor with room, not an aspiration.
                let wood = above[TRUNK as usize] + above[BRANCH as usize];
                assert!(
                    wood > 18,
                    "{setting:?}/{seed:#x}: only {wood} cells of wood"
                );
                assert!(
                    above[TRUNK as usize] > 14,
                    "{setting:?}/{seed:#x}: only {} cells of trunk",
                    above[TRUNK as usize]
                );
                // Measured 317..884 over these seeds. The floor is what stops
                // a bare tree, the ceiling what stops a pink cloud.
                let bloom: usize = BLOSSOM.iter().map(|&c| above[c as usize]).sum();
                assert!(
                    (280..2_000).contains(&bloom),
                    "{setting:?}/{seed:#x}: {bloom} blossom cells is not a crown"
                );
                for &c in &BLOSSOM {
                    assert!(
                        above[c as usize] > 30,
                        "{setting:?}/{seed:#x}: blossom tier {c} is unused"
                    );
                }

                // The crown's bounding box. A cherry is broader than it is
                // tall; a tall narrow crown is the telegraph pole.
                let (mut x0, mut x1, mut y0, mut y1) = (usize::MAX, 0, usize::MAX, 0);
                for (i, c) in s.scene.iter().enumerate() {
                    if *c == Cell::CLEAR || !BLOSSOM.contains(&(c.colour() as u16)) {
                        continue;
                    }
                    let (x, y) = (i % cols, i / cols);
                    x0 = x0.min(x);
                    x1 = x1.max(x);
                    y0 = y0.min(y);
                    y1 = y1.max(y);
                }
                let (w, h) = (x1 + 1 - x0, y1 + 1 - y0);
                assert!(
                    w * 4 > h * 5,
                    "{setting:?}/{seed:#x}: crown is {w}x{h} — too narrow for a cherry"
                );
                assert!(
                    w < cols * 19 / 20,
                    "{setting:?}/{seed:#x}: crown is {w} of {cols} columns — that is a hedge"
                );
                // And it is a tree, not a bush: the crown's foot is clear of
                // the ground, with trunk under it.
                assert!(
                    y1 < s.ground_y,
                    "{setting:?}/{seed:#x}: blossom reaches the ground line"
                );
                assert!(
                    y0 < s.ground_y / 2,
                    "{setting:?}/{seed:#x}: the crown never reaches the upper half"
                );
            }
        }
    }

    /// No two seeds draw the same scene. Without this, "randomly generated" can
    /// regress to a fixed tree and every other test still passes.
    #[test]
    fn different_seeds_grow_different_trees() {
        for setting in Setting::ALL {
            let scenes: Vec<Vec<Cell>> = SEEDS.iter().map(|&s| at(s, setting).scene).collect();
            for (i, a) in scenes.iter().enumerate() {
                for b in &scenes[i + 1..] {
                    let same = a.iter().zip(b.iter()).filter(|(x, y)| x == y).count();
                    assert!(
                        same * 10 < a.len() * 9,
                        "{setting:?}: two seeds drew scenes {}% identical",
                        same * 100 / a.len()
                    );
                }
            }
        }
    }

    /// The pond: a lake under the tree and a reflection that is dimmer than
    /// what it reflects. A reflection at the subject's brightness reads as a
    /// second tree.
    #[test]
    fn the_pond_has_a_lake_and_a_dimmer_reflection() {
        for seed in SEEDS {
            let s = at(seed, Setting::Pond);
            let (above, below) = census(&s);
            let cols = s.grid.cols();
            let water: usize = WATER.iter().map(|&c| below[c as usize]).sum();
            assert!(water > 600, "{seed:#x}: only {water} water cells");
            assert!(below[WATER_LINE as usize] > cols / 2, "no surface line");

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
            // As a whole, not per tier: the crown's top tier is high enough
            // that its mirror image falls off the bottom of the panel, which
            // is what a real reflection does.
            let mirrored: usize = (13..=18).map(|r| below[r]).sum();
            assert!(mirrored > 150, "{seed:#x}: only {mirrored} reflected cells");
            // And nothing that belongs to another setting leaked in.
            for c in [RIDGE[0], GRAVEL, RAKE, SPUR, WALL] {
                assert_eq!(
                    below[c as usize] + above[c as usize],
                    0,
                    "{c} is in the pond"
                );
            }
        }
    }

    /// The mountain: three ridges behind the tree, each dimmer than the one in
    /// front of it, mist in the valley, a spur under the tree — and open air
    /// beside the spur, which is what the petals blow out over. Distance on a
    /// black panel is nothing but that luminance ladder, so it is pinned the
    /// same way the reflection is.
    #[test]
    fn the_mountain_recedes_into_dimmer_ridges() {
        assert!(
            lum(RIDGE[0]) * 2 < lum(RIDGE[2]),
            "the farthest ridge is not half the nearest"
        );
        assert!(lum(RIDGE[0]) < lum(RIDGE[1]) && lum(RIDGE[1]) < lum(RIDGE[2]));
        assert!(lum(SPUR) < lum(SPUR_EDGE), "the spur has no lit crest");

        for seed in SEEDS {
            let s = at(seed, Setting::Mountain);
            let (above, below) = census(&s);
            let cols = s.grid.cols();
            for (k, &c) in RIDGE.iter().enumerate() {
                assert!(
                    above[c as usize] > 300,
                    "{seed:#x}: ridge {k} has {} cells",
                    above[c as usize]
                );
            }
            assert!(
                above[MIST as usize] > 100,
                "{seed:#x}: no mist in the valley"
            );
            assert!(
                below[SPUR as usize] + below[SPUR_EDGE as usize] > 700,
                "{seed:#x}: no spur under the tree"
            );
            // The drop: a column of the lower panel with no ground in it at
            // all. Without one there is nothing for a petal to fall past.
            let rows = s.grid.rows();
            let open = (0..cols)
                .filter(|&x| ((s.ground_y + 2)..rows).all(|y| s.scene[y * cols + x] == Cell::CLEAR))
                .count();
            assert!(
                open > cols / 8,
                "{seed:#x}: only {open} columns of open air"
            );
            for c in [WATER_LINE, GRAVEL, RAKE, 15, 16] {
                assert_eq!(
                    below[c as usize] + above[c as usize],
                    0,
                    "{c} is on the mountain"
                );
            }
        }
    }

    /// The rock garden: a bed of gravel with rake grooves over it, set stones
    /// with a lit shoulder, rings raked round them, and a wall behind. The
    /// grooves are what make it a karesansui rather than a beach, so they are
    /// pinned by luminance against the gravel the way the reflection is against
    /// the tree.
    #[test]
    fn the_rock_garden_is_raked_gravel_and_set_stones() {
        assert!(
            lum(GRAVEL) * 2 < lum(RAKE),
            "a rake groove's ridge does not catch the light"
        );
        assert!(lum(STONE_DARK) * 2 < lum(STONE_LIT), "the stones are flat");

        // Glyph index back to its braille bits, so a cell can be asked which
        // dots it lights rather than only what colour it claims.
        let bits: std::collections::HashMap<usize, u8> = font::BRAILLE
            .iter()
            .enumerate()
            .map(|(b, &g)| (g as usize, b as u8))
            .collect();

        for seed in SEEDS {
            let s = at(seed, Setting::Garden);
            let (above, below) = census(&s);
            let cols = s.grid.cols();
            assert!(
                below[GRAVEL as usize] > 1_000,
                "{seed:#x}: {} gravel cells is not a bed",
                below[GRAVEL as usize]
            );
            assert!(
                below[RAKE as usize] > 2_000,
                "{seed:#x}: the gravel is unraked"
            );
            assert!(
                below[STONE_LIT as usize] + below[STONE_DARK as usize] > 40,
                "{seed:#x}: no set stones"
            );
            assert!(below[MOSS as usize] > 4, "{seed:#x}: no moss");
            assert!(
                above[WALL as usize] + above[WALL_TOP as usize] > cols / 2,
                "{seed:#x}: no wall behind the garden"
            );
            // The bed alternates: an unbroken furrow on every odd row below
            // the ground line, gravel on every even one. A furrow that is
            // merely mostly there reads as a dashed line, so the bound is
            // most of the row — the gap is the stones the rake goes round.
            let rows = s.grid.rows();
            for y in (s.ground_y + 1)..rows {
                let raked = (0..cols)
                    .filter(|&x| s.scene[y * cols + x].colour() == RAKE as usize)
                    .count();
                if (y - s.ground_y) % 2 == 1 {
                    assert!(
                        raked > cols * 3 / 4,
                        "{seed:#x}: furrow row {y} has {raked} of {cols} raked cells"
                    );
                    // And UNBROKEN: both sub-columns of the cell lit at the
                    // groove's sub-row, or the furrow is a dotted line with a
                    // gap at every cell boundary, which reads as static. A
                    // cell-count test alone cannot see that — one dot per cell
                    // counts exactly the same.
                    let both = DOT[0][GROOVE_SUB as usize] | DOT[1][GROOVE_SUB as usize];
                    let solid = (0..cols)
                        .filter(|&x| {
                            let g = s.scene[y * cols + x].glyph();
                            bits.get(&g).is_some_and(|b| b & both == both)
                        })
                        .count();
                    assert!(
                        solid > cols * 3 / 4,
                        "{seed:#x}: furrow row {y} is unbroken in only {solid} of {cols} cells"
                    );
                } else {
                    let gravel = (0..cols)
                        .filter(|&x| s.scene[y * cols + x].colour() == GRAVEL as usize)
                        .count();
                    assert!(
                        gravel > cols / 3,
                        "{seed:#x}: gravel row {y} has {gravel} of {cols} gravel cells"
                    );
                }
            }
            for c in [WATER_LINE, RIDGE[0], SPUR, 15, 16] {
                assert_eq!(
                    below[c as usize] + above[c as usize],
                    0,
                    "{c} is in the garden"
                );
            }
        }
    }

    /// A colour with no glyph is an invisible cell, and a cell that claims a
    /// colour while drawing nothing is how a scene silently loses a feature.
    #[test]
    fn every_cell_the_scene_paints_is_actually_drawn() {
        for setting in Setting::ALL {
            for seed in SEEDS {
                let s = at(seed, setting);
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
                    assert!(
                        c.colour() < PAL.len(),
                        "cell {i} has colour {} and the palette has {}",
                        c.colour(),
                        PAL.len()
                    );
                }
                assert!(
                    lit > 1_500,
                    "{setting:?}/{seed:#x}: only {lit} cells painted"
                );
            }
        }
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
