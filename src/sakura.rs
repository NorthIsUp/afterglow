//! Sakura — a cherry tree at night, shedding blossom on a slow wind. The tree
//! is grown from a seed that changes every time the pod starts, and it stands
//! in one of three places: beside a pond, on a mountain spur, or in a rock
//! garden. One start in three grows it windswept instead of upright — the
//! bonsai fukinagashi, laid over by a wind that never stops.
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
//! * **Nothing in the tree is drawn straight.** The trunk is six short
//!   segments that each turn on the last — the lean working toward the angle
//!   the crown grows out of, one slow S over the height, and a wobble on top of
//!   that — thickening toward a flared foot with roots running out of it, and
//!   every limb above it is bowed off its own chord. A straight tapered run is
//!   a post, and a crown of straight runs is a diagram of a tree; all of this
//!   is silhouette, because at 12px cells silhouette is what there is.
//! * **The trunk is drawn in two woods.** The palette always called entry 4
//!   "branches AND THE TRUNK'S SHADOW SIDE", but nothing drew the shadow, so
//!   every trunk was a flat bar. `bole` paints the far third of a limb's width
//!   in the dark wood — under it where it lies across the panel, on the leaning
//!   side where it stands up — which is the whole difference between a bar and
//!   a cylinder, and it costs one comparison per sub-cell.
//! * **The style is the scene's weather, not the tree's habit.** `Style` is
//!   rolled once per scene and both the cherry and the sapling behind it are
//!   grown in it, because one swept tree beside one upright one reads as a bug.
//!   A windswept tree is the SAME generator: a trunk laid over three times as
//!   far, an apex that carries on leaning instead of reaching back up for the
//!   light, `Shape::sweep` combing every limb downwind a little harder each
//!   generation, and the limbs thrown into the wind cut short.
//! * **A crown is grown until it is not a mirror.** The first fork throws its
//!   limbs symmetrically about the trunk and none of them near the vertical, so
//!   a fair share of the shape rolls came up as two matched lobes with a hole
//!   between them — a tree the eye reads as one sprite flipped. `plant` grows
//!   the skeleton DRY, judges it with `splits_evenly`, and only draws one that
//!   passes; rerolling is what keeps the variety that tuning the angles away
//!   would have cost.
//! * **Sub-cell motion is free.** A petal's braille pattern is chosen from its
//!   fractional position inside the cell, so it moves in 6x4 px steps through a
//!   12x16 px cell without any extra cost.
//!
//! # The seed
//!
//! `SAKURA_SEED` defaults to 0, which means "pick one from the clock and the
//! pid" — so a pod restart shows a new tree in a new place. Any non-zero value
//! reproduces its draw exactly, which is how every test here pins a scene:
//! they call `build` with a literal seed, setting and style rather than going
//! through `new`. `SAKURA_SCENE` pins where the tree stands and `SAKURA_TREE`
//! (`windswept` / `upright`) pins how it grew; both otherwise roll.
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

/// Branch recursion depth for an upright tree. Four generations is where the
/// tips stop being individually visible at 12px cells and start being canopy.
/// `Shape::depth` is what `branch` actually reads.
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

/// How a tree is shaped. The setting decides what is under the tree; this
/// decides the tree, and one scene's trees are all in the same weather.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Style {
    /// The cherry the saver has always drawn: a trunk a few degrees off the
    /// vertical under a crown that spreads both ways.
    Upright,
    /// The bonsai fukinagashi — the windswept style. A trunk laid right over by
    /// a wind that never stops, every limb combed downwind of it, and the
    /// windward side all but bare. It is the same generator: a bigger lean, an
    /// apex that carries on leaning instead of reaching back up for the light,
    /// and `Shape::sweep`.
    Windswept,
}

impl Style {
    /// `SAKURA_TREE` pins one; anything else (including a typo) rolls, on the
    /// same forgiving-by-default argument as `env_num`. One start in three is
    /// swept: often enough to be a scene the panel shows, rare enough that it
    /// is still a surprise when it does.
    fn pick(rng: &mut u32) -> Self {
        match env_str(&["SAKURA_TREE"], "").as_str() {
            "windswept" | "swept" | "bonsai" => Self::Windswept,
            "upright" | "straight" => Self::Upright,
            _ if next_rand(rng).is_multiple_of(3) => Self::Windswept,
            _ => Self::Upright,
        }
    }

    /// Trunk length as a percent of the upright tree's. A leaning trunk spends
    /// its length going sideways, so a windswept one is grown longer to stand
    /// as tall — without this the whole tree sits in the bottom corner.
    fn reach(self) -> i32 {
        match self {
            Self::Upright => 100,
            Self::Windswept => 120,
        }
    }

    /// The trunk's lean off the vertical, in 1024-turn units, before the side
    /// it leans to is applied. A windswept trunk is laid over far enough that
    /// the lean is the first thing read about the tree — about 16 to 28
    /// degrees, where an upright one is under ten.
    fn lean(self, rng: &mut u32) -> i32 {
        match self {
            Self::Upright => 8 + (next_rand(rng) % 19) as i32,
            Self::Windswept => 46 + (next_rand(rng) % 34) as i32,
        }
    }
}

/// One tree as `build` places it: where it is rooted, how big, and its habit.
/// Bundled because `plant` grows it twice — once dry to measure, once for real
/// — and eight loose arguments threaded through that is how one gets dropped.
#[derive(Clone, Copy)]
struct Trunk {
    /// Root, in sub-columns and sub-rows.
    x: i32,
    y: i32,
    /// Length along the trunk in sub-rows, and its lean off the vertical in
    /// 1024-turn units: positive leans toward the panel's right.
    len: i32,
    lean: i32,
    /// Trunk width in sub-columns at the root.
    thick: i32,
    style: Style,
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
    /// Swallow every `px` while a tree is being grown only to be measured. A
    /// crown is judged by where its tips landed, which costs a recursion and
    /// no pixels — see `plant`.
    dry: bool,
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
    /// Generations of forking. One more than the upright tree's for a
    /// windswept one: its crown is drawn out along the wind instead of balled
    /// over the trunk, and at the same tip count that is clusters on bare wire
    /// rather than a canopy.
    depth: u32,
    /// Which side of a steep limb is in shadow: the side the trunk leans
    /// toward, which is the side leaning away from the sky.
    shade: i32,
    /// Angle added per generation to comb the whole crown one way: the wind in
    /// a windswept tree, and zero in an upright one. Signed, because 256 is up
    /// and the angle falls toward the panel's right — so a tree leaning right
    /// sweeps with a NEGATIVE sweep.
    sweep: i32,
}

impl Raster {
    fn new(cols: usize, rows: usize) -> Self {
        Self {
            cols: cols as i32,
            rows: rows as i32,
            bits: vec![0; cols * rows],
            col: vec![0; cols * rows],
            tips: Vec::new(),
            dry: false,
            shape: Shape {
                spread0: 70,
                spread0_var: 120,
                spread: 60,
                spread_var: 90,
                droop: 20,
                scale0: 88,
                scale: 68,
                kids0: 4,
                depth: MAX_DEPTH,
                shade: 1,
                sweep: 0,
            },
        }
    }

    /// Light one sub-cell dot and claim the cell for `c`. A cell holds one
    /// colour, so the last writer wins — which is why wood is drawn before
    /// blossom and the reflection after everything.
    #[inline]
    fn px(&mut self, sx: i32, sy: i32, c: u16) {
        if self.dry {
            return;
        }
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
        self.bole(x0, y0, x1, y1, t0, t1, c, c, 1);
    }

    /// The same run, painted in two woods: `lit` and, down the far third of its
    /// width, `dark`. The palette has always had both — entry 4 is "branches
    /// AND THE TRUNK'S SHADOW SIDE" — but nothing ever drew the shadow, so
    /// every trunk was a flat brown bar. A third of the width in the dark
    /// colour is the whole difference between a bar and a cylinder, and it
    /// costs one comparison per sub-cell.
    ///
    /// The light is overhead, which is where the blossom's tiers put it too:
    /// the shadow falls UNDER a limb lying across the panel, and on a steep one
    /// it falls on the side the trunk leans toward, which is the side that
    /// leans away from the sky.
    #[allow(clippy::too_many_arguments)]
    fn bole(
        &mut self,
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        t0: i32,
        t1: i32,
        lit: u16,
        dark: u16,
        side: i32,
    ) {
        let n = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
        let steep = (y1 - y0).abs() >= (x1 - x0).abs();
        for k in 0..=n {
            let x = x0 + (x1 - x0) * k / n;
            let y = y0 + (y1 - y0) * k / n;
            let h = (t0 + (t1 - t0) * k / n) / 2;
            for o in -h..=h {
                // A limb one sub-cell thick has no sides to shade.
                let shadow = h > 0 && o * if steep { side } else { 1 } * 3 > h;
                let c = if shadow { dark } else { lit };
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
    ///
    /// `sweep` is what makes a windswept tree windswept, and it is applied per
    /// GENERATION so the comb tightens with height: a limb thrown upwind low
    /// down is bent back over the trunk by the time it has forked twice, which
    /// is the shape a wind that never stops leaves behind.
    #[allow(clippy::too_many_arguments)]
    fn branch(&mut self, x: i32, y: i32, ang: i32, len: i32, thick: i32, d: u32, rng: &mut u32) {
        let x1 = x + ((cos1024(ang) * len) >> 10);
        let y1 = y - ((sin1024(ang) * len * ASPECT_NUM / ASPECT_DEN) >> 10);
        let next = (thick * 2 / 3).max(1);
        // Drawn as two runs with a bow between them rather than as one: a limb
        // that is straight from fork to fork is a wire, and a crown of wires is
        // a diagram of a tree. The bow grows with the limb, so the long ones
        // that show under the blossom curve and the twigs inside it do not
        // waste a break on a line nobody can see. The ENDS do not move, so the
        // recursion above knows nothing about it.
        let (dx, dy) = (x1 - x, y1 - y);
        let n = dx.abs().max(dy.abs()).max(1);
        let bow = ((next_rand(rng) % 5) as i32 - 2) * n / 24;
        let (mx, my) = ((x + x1) / 2 - dy * bow / n, (y + y1) / 2 + dx * bow / n);
        let mid = (thick + next) / 2;
        // The limbs off the trunk are trunk-thick and get its shading; above
        // them a branch is a line, and a line has no sides.
        if d == 0 {
            let side = self.shape.shade;
            self.bole(x, y, mx, my, thick, mid, TRUNK, BRANCH, side);
            self.bole(mx, my, x1, y1, mid, next, TRUNK, BRANCH, side);
        } else {
            self.limb(x, y, mx, my, thick, mid, BRANCH);
            self.limb(mx, my, x1, y1, mid, next, BRANCH);
        }
        if d >= self.shape.depth || len < 4 {
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
            a += s.sweep * (d as i32 + 1);
            // The sweep may lay a limb flat but not tip it over: a branch
            // pointing downwind AND downward is a snapped one, and a crown of
            // them hangs off the trunk like weed off a post.
            if s.sweep != 0 {
                a = a.clamp(-16, 528);
            }
            let scale = if d == 0 { s.scale0 } else { s.scale };
            let mut l = len * (scale + (next_rand(rng) % 20) as i32) / 100;
            // A limb thrown INTO the wind gets nowhere. Without this the sweep
            // only tilts a crown that is still symmetrical about the trunk,
            // which reads as a tree on a hill rather than as a tree in a wind.
            if s.sweep != 0 && (a - 256).signum() == -s.sweep.signum() {
                l = l * 7 / 10;
            }
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

/// Bins the crown's width is cut into to look for a split. Nine is enough that
/// one bin is a plausible gap — about a ninth of the crown — and few enough
/// that a bin still holds a countable number of tips.
const SPLIT_BINS: usize = 9;

/// Side of the coarse cell the crown's mass is measured in, in sub-columns
/// (and twice that in sub-rows, which are half the height). Two cells across:
/// the width of one blossom stamp, so two tips inside one of these paint
/// essentially the same patch of crown.
const SPLIT_Q: i32 = 4;

/// Narrowest bin, in coarse cells, the split test will believe. A gap thinner
/// than the blossom stamp is filled in by the stamp and never reaches the eye;
/// measuring it would reject trees that look perfectly whole.
const SPLIT_BIN_MIN: i32 = 2;

/// How much bigger one side of a gap has to be for the gap to be allowed, as a
/// fraction: five halves, where the eye applies about two to the finished
/// panel. The measure below is the SKELETON's, and blossom spreads past the
/// tips that carry it, which flatters the smaller lobe — at a flat two, seeds
/// were still rendering as mirrored pairs when the frames were looked at.
const SPLIT_DOM_NUM: usize = 5;
const SPLIT_DOM_DEN: usize = 2;

/// Segments a trunk is drawn in. One straight run reads as a mast and two read
/// as a mast with a kink; six is where the joins stop being visible at 12px
/// cells and the line starts reading as grown rather than drawn.
const TRUNK_SEGS: i32 = 6;

/// Trunk width at the top as a percent of its width at the root.
const TRUNK_TIP: i32 = 38;

/// Sub-rows over which the foot flares out, and by how much at the very
/// bottom, in percent. This is the nebari, and on a panel this coarse it is
/// two sub-cells of extra width — but they are the two that decide whether the
/// tree is growing out of the ground or standing on it.
const FLARE_UP: i32 = 140;
const FLARE_PCT: i32 = 45;

/// Width at `f` (0..=1024) of the way up the trunk, as a percent of the width
/// at the root. Quadratic, not linear: a trunk loses most of its girth in its
/// first few feet and then hardly tapers at all. A linear taper draws a spike.
fn taper(f: i32) -> i32 {
    let up = (1024 - f).clamp(0, 1024);
    let body = TRUNK_TIP + (100 - TRUNK_TIP) * (up * up / 1024) / 1024;
    let flare = FLARE_PCT * (FLARE_UP - f).clamp(0, FLARE_UP) / FLARE_UP;
    body + flare
}

/// Sub-rows of sky a crown's topmost tip has to leave above it. Blossom is
/// stamped up to 6 sub-rows above the tip it hangs on, so a crown grown right
/// to the edge loses its palest tier — which is its top, and the whole reason
/// the canopy reads as lit from above.
const FIT_MARGIN: i32 = 6;

/// Rounds of cutting the trunk back when no shape of crown will fit, and how
/// much is taken off each time. A tree that cannot fit the panel at the height
/// it was asked for is a SHORTER tree, not a clipped one — the crown is the
/// subject and the trunk is what gives.
const FIT_ROUNDS: i32 = 4;
const FIT_CUT: i32 = 12;

/// Does the crown fit on the panel? Only the top edge: a tree is rooted on the
/// ground line and placed with room either side, but nothing until here stopped
/// one growing straight off the top, where the palest tier — the lit top of the
/// canopy — is the first thing lost.
fn fits(tips: &[(i32, i32)]) -> bool {
    tips.iter().all(|&(_, ty)| ty >= FIT_MARGIN)
}

/// Crowns grown and thrown away before `plant` takes what it is given. Each
/// attempt draws nothing and costs one recursion; the odds of nine in a row
/// splitting evenly are small enough that the fall-through is a formality, and
/// a loop that cannot end is worse than a tree that reads as a mirror.
const SPLIT_TRIES: usize = 9;

/// Does this crown read as one sprite mirrored — a hole down the middle with
/// about as much blossom either side of it?
///
/// A hole in a crown is welcome; a real cherry is full of them. What makes this
/// one wrong is that the two sides BALANCE, so the gap stops reading as a tree
/// that grew round something and starts reading as a seam. The rule is
/// therefore about mass, not about position: a gap is fine as long as one side
/// carries at least twice the blossom of the other.
///
/// The tips stand in for the blossom because `plant` stamps every dot on a
/// UNIFORMLY random tip: where the tips are is where the mass is. What the
/// tips do NOT have is the stamp's spread, so they are measured as area rather
/// than as a count, at a grain no finer than the stamp itself — see `SPLIT_Q`,
/// `SPLIT_BIN_MIN` and `SPLIT_DOM_NUM`, each of which is that correction.
fn splits_evenly(tips: &[(i32, i32)]) -> bool {
    // Tips quantised to a coarse cell and deduplicated, which turns a tip
    // COUNT into a crown AREA. Blossom stamped on two tips in the same place
    // covers the same cells twice and the eye sees it once, so counting tips
    // raw reports a dense lobe as bigger than it looks.
    let mut mass: Vec<(i32, i32)> = tips
        .iter()
        .map(|&(tx, ty)| (tx.div_euclid(SPLIT_Q), ty.div_euclid(SPLIT_Q * 2)))
        .collect();
    mass.sort_unstable();
    mass.dedup();
    // Too small a crown to read as two lobes at all: a sapling is a smudge,
    // and a smudge cannot be symmetrical.
    if mass.len() < 12 {
        return false;
    }
    let (mut x0, mut x1) = (i32::MAX, i32::MIN);
    for &(qx, _) in &mass {
        x0 = x0.min(qx);
        x1 = x1.max(qx);
    }
    let span = x1 - x0 + 1;
    if span < SPLIT_BIN_MIN * SPLIT_BINS as i32 {
        return false;
    }
    let mut hist = [0usize; SPLIT_BINS];
    for &(qx, _) in &mass {
        let b = ((qx - x0) * SPLIT_BINS as i32 / span).clamp(0, SPLIT_BINS as i32 - 1);
        hist[b as usize] += 1;
    }
    // Any interior bin holding under a third of an even share is a gap. The
    // outermost bins are excluded: the thin edge of a crown is not a seam.
    for b in 1..SPLIT_BINS - 1 {
        if hist[b] * SPLIT_BINS * 3 >= mass.len() {
            continue;
        }
        let left: usize = hist[..b].iter().sum();
        let right: usize = hist[b + 1..].iter().sum();
        // An empty side is not a split — that is a crown leaning off one
        // shoulder, which is exactly the tree this is trying to get.
        if left.max(right) * SPLIT_DOM_DEN < SPLIT_DOM_NUM * left.min(right) {
            return true;
        }
    }
    false
}

/// Grow one tree's wood and the tip list its blossom hangs from: the shape
/// roll, the trunk, the branch recursion and the bare-limb cut. Everything that
/// decides WHERE the blossom goes and nothing that puts it there, so `plant`
/// can run this dry, measure the crown, and run it again for real.
///
/// The bounds on every drawn parameter are the point of this function. They
/// were chosen against the fixed tree the saver used to draw, which is the one
/// known-good sample: each range brackets that tree's value by about as much as
/// still reads as a cherry, and no further. Widening `droop` past ~30 drops the
/// outer branches below horizontal and the crown collapses into the trunk;
/// narrowing it past ~12 grows the tree straight off the top of the panel.
/// `kids0` never goes below 3 because two limbs off the trunk is a Y, not a
/// crown. `spread0` below ~55 gives a poplar and above ~230 a hedge.
fn grow(r: &mut Raster, rng: &mut u32, t: Trunk) {
    r.shape = Shape {
        spread0: 55 + (next_rand(rng) % 30) as i32,
        spread0_var: 95 + (next_rand(rng) % 55) as i32,
        spread: 50 + (next_rand(rng) % 25) as i32,
        spread_var: 70 + (next_rand(rng) % 45) as i32,
        // A windswept tree droops less: the sweep already carries every limb
        // toward the horizontal, and the two together put the whole crown
        // below it.
        droop: match t.style {
            Style::Upright => 13 + (next_rand(rng) % 16) as i32,
            Style::Windswept => 7 + (next_rand(rng) % 9) as i32,
        },
        scale0: 80 + (next_rand(rng) % 14) as i32,
        // A swept limb keeps more of its parent's length: its crown is a tail
        // rather than a ball, and short children make that tail a stub.
        scale: match t.style {
            Style::Upright => 62 + (next_rand(rng) % 13) as i32,
            Style::Windswept => 72 + (next_rand(rng) % 13) as i32,
        },
        kids0: 3 + (next_rand(rng) % 100 / 70) as i32,
        depth: match t.style {
            Style::Upright => MAX_DEPTH,
            Style::Windswept => MAX_DEPTH + 1,
        },
        shade: t.lean.signum(),
        // Downwind is the way the trunk already leans, so the tree and the
        // petals it sheds agree. Past ~40 a generation the crown leaves the
        // trunk behind and the tree reads as a flag on a pole; under ~20 it is
        // an upright cherry that happens to lean.
        sweep: match t.style {
            Style::Upright => 0,
            Style::Windswept => -t.lean.signum() * (24 + (next_rand(rng) % 13) as i32),
        },
    };

    // The trunk, as a run of short segments that each turn a little on the
    // last. A tree's trunk is never straight and never a smooth arc either: it
    // leaves the ground at its lean, works back toward the angle its crown
    // grows out of, wanders on the way, and thickens toward its foot. Every
    // one of those is a line here, and together they are the difference
    // between a tree and a post with a bush on it.
    //
    // An upright tree straightens toward the vertical, which is what a leaning
    // tree does — it grows back up toward the light. A windswept one is the
    // tree that never got to: its apex carries on the way the wind laid it,
    // and that unbroken line from root to apex is what says fukinagashi rather
    // than "fell over".
    r.tips.clear();
    let a0 = 256 - t.lean;
    let apex = match t.style {
        Style::Upright => 256 - t.lean / 4,
        Style::Windswept => 256 - t.lean * 5 / 4,
    };
    // One slow S on the way up, against the lean at the foot and with it
    // higher. A trunk that bends one way only is a bow, and a cherry — or any
    // bonsai worth looking at — does not grow as a bow.
    let ess = (26 + (next_rand(rng) % 34) as i32) * -t.lean.signum();
    // Where the bend sits. Without this every trunk in every scene turns at
    // the same height, which is a signature and reads as one.
    let phase = (next_rand(rng) % 256) as i32 - 128;
    let seg = (t.len / TRUNK_SEGS).max(2);
    let (mut x, mut y, mut ang) = (t.x, t.y, a0);
    for k in 0..TRUNK_SEGS {
        // Angle at this segment's middle: the lean working toward the apex,
        // the S over the whole height, and a wobble that keeps the line from
        // being drawn with a compass.
        let f = (k * 2 + 1) * 512 / TRUNK_SEGS;
        ang = a0
            + (apex - a0) * f / 1024
            + ess * sin1024(f + phase) / 1024
            + (next_rand(rng) % 27) as i32
            - 13;
        let nx = x + ((cos1024(ang) * seg) >> 10);
        let ny = y - ((sin1024(ang) * seg * ASPECT_NUM / ASPECT_DEN) >> 10);
        let w0 = (t.thick * taper(k * 1024 / TRUNK_SEGS) / 100).max(1);
        let w1 = (t.thick * taper((k + 1) * 1024 / TRUNK_SEGS) / 100).max(1);
        r.bole(x, y, nx, ny, w0, w1, TRUNK, BRANCH, r.shape.shade);
        (x, y) = (nx, ny);
    }
    // Roots. Two or three flares running out of the foot and down to the
    // ground line, drawn AFTER the trunk so they read as growing out of it.
    // Whatever the setting covers below the line, what shows above it is a
    // tree that grew where it stands instead of one pushed into the scenery.
    let foot = t.y - (4 + (next_rand(rng) % 7) as i32);
    for k in 0..2 + (next_rand(rng) % 2) as i32 {
        let out = (t.thick * (70 + (next_rand(rng) % 90) as i32) / 100).max(2);
        let side = if k % 2 == 0 { 1 } else { -1 };
        let w = (t.thick * (35 + (next_rand(rng) % 30) as i32) / 100).max(1);
        r.bole(
            t.x,
            foot,
            t.x + out * side,
            t.y,
            w,
            1,
            TRUNK,
            BRANCH,
            r.shape.shade,
        );
    }

    // Out of the last segment, at the angle it was actually going: a crown
    // hung off a nominal apex angle shows a kink at the top of the trunk.
    r.branch(
        x,
        y,
        ang,
        (t.len - seg * TRUNK_SEGS / 2).max(4),
        (t.thick * TRUNK_TIP / 100).max(2),
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
}

/// Grow one tree from `(x, y)` and hang its blossom on it.
///
/// The crown is grown until it is not a mirror. The first fork throws its limbs
/// symmetrically about the trunk and none of them near the vertical, so a fair
/// share of the shape rolls come up as two matched lobes with a hole between
/// them straddling the trunk — a shape the eye reads as one sprite flipped
/// rather than as a tree. Rolling again is the cheapest fix that keeps the
/// variety: the structure is grown dry, judged by `splits_evenly`, and only
/// redrawn for real once it passes.
///
/// A crown that will not fit the panel is rejected the same way, and when no
/// shape of crown fits, the trunk is cut back and the whole thing tried again:
/// the tree the panel cannot hold at full height is a shorter tree, never a
/// clipped one. Both tests are cheap because the attempt is grown DRY — `dry`
/// swallows every `px`, so an attempt costs one recursion and no pixels.
///
/// Replaying from the accepted attempt's `rng` state and trunk regrows EXACTLY
/// the tree that was measured, because everything `grow` draws comes out of
/// `rng` and nothing else. Rejected attempts leave no trace but the rng they
/// burned, and even that is rewound.
fn plant(r: &mut Raster, rng: &mut u32, t: Trunk, bloom: usize) {
    // The rng state the kept attempt started from, and the trunk it grew on,
    // so the accepted one can be grown again.
    let mut accepted = *rng;
    let mut kept = t;
    'fitted: for round in 0..FIT_ROUNDS {
        let cut = Trunk {
            len: t.len * (100 - round * FIT_CUT) / 100,
            ..t
        };
        for _ in 0..SPLIT_TRIES {
            accepted = *rng;
            kept = cut;
            r.dry = true;
            grow(r, rng, cut);
            r.dry = false;
            if !splits_evenly(&r.tips) && fits(&r.tips) {
                break 'fitted;
            }
        }
    }
    *rng = accepted;
    grow(r, rng, kept);

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
        let style = Style::pick(&mut pick);
        Self::build(panel, fps, seed, setting, style)
    }

    /// The seed, the setting and the style come in as arguments so the tests
    /// can pin a scene without `set_var`: cargo runs tests in parallel threads
    /// and the environment is process-wide, so one test's setting would land in
    /// another's saver.
    fn build(panel: &Panel, fps: u32, seed: u32, setting: Setting, style: Style) -> Self {
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
        // The style is one scene's weather, not one tree's habit: it arrives
        // from `new` and both trees here are grown in it, because one swept
        // cherry beside one upright one reads as a bug rather than as a grove.
        //
        // A swept tree is rooted further out toward the edge it leans away
        // from, because its crown is thrown clear of the trunk rather than
        // balanced on it and needs the whole panel to be thrown across.
        let stand = match style {
            Style::Upright => 26 + (next_rand(&mut rng) % 17) as i32,
            Style::Windswept => 15 + (next_rand(&mut rng) % 13) as i32,
        };
        let trunk_x = (cols as i32 * stand / 100) * 2;
        let trunk_y = ground_y as i32 * 4;
        // A SHORT trunk. The crown, not the trunk, is what has to fill the
        // upper half — a trunk that reaches two thirds of the way up pushes
        // every branch off the top of the panel. Measured from the GROUND line
        // rather than from the panel, or a setting with a high horizon gets a
        // stunted tree for free.
        let top = ground_y as i32 * (55 + (next_rand(&mut rng) % 20) as i32) / 100;
        let trunk_len = ((((ground_y as i32 - top) * 4) * ASPECT_DEN / ASPECT_NUM).max(8)
            * style.reach())
            / 100;
        // Girth at the foot, before `taper` flares it. A cherry of this height
        // is a stout tree, and a trunk under about three cells wide reads as a
        // pole holding the blossom up rather than as the thing that grew it.
        let thick =
            ((cell_w as i32 * 2 / 3).clamp(5, 12) * (85 + (next_rand(&mut rng) % 41) as i32) / 100)
                .clamp(4, 14);
        let away = if trunk_x * 2 < subcols as i32 { 1 } else { -1 };
        let lean = away * style.lean(&mut rng);
        // Roughly three quarters of a dot per CELL of panel. Below about half
        // that the crown stops being a mass and goes back to confetti.
        let bloom_pct = env_num(&["SAKURA_BLOOM"], 100, 0, 400) as usize;
        let dots =
            cols * rows * 3 / 4 * bloom_pct / 100 * (80 + next_rand(&mut rng) as usize % 46) / 100;
        plant(
            &mut r,
            &mut rng,
            Trunk {
                x: trunk_x,
                y: trunk_y,
                len: trunk_len,
                lean,
                thick,
                style,
            },
            dots,
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
            // Leaning the other way from the big tree, so the pair reads as two
            // trees rather than as one drawn twice — except in a wind, which
            // blows on both of them and lays them the same way.
            let sap_lean = match style {
                Style::Upright => -away * (6 + (next_rand(&mut rng) % 15) as i32),
                Style::Windswept => away * style.lean(&mut rng),
            };
            let sap_thick = (thick * 3 / 5).max(3);
            plant(
                &mut r,
                &mut rng,
                Trunk {
                    x: far,
                    y: trunk_y,
                    len,
                    lean: sap_lean,
                    thick: sap_thick,
                    style,
                },
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
    use crate::dump;
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
        at_style(seed, setting, Style::Upright)
    }

    fn at_style(seed: u32, setting: Setting, style: Style) -> Sakura {
        Sakura::build(&panel(), 30, seed, setting, style)
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
            let mut px = Vec::new();
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
                        dump::row_reported(&prev[y * p.w..][..p.w], &buf[y * p.w..][..p.w], y, &d),
                        "{setting:?} frame {f}: scanline {y} changed outside every reported rect"
                    );
                }
                moved += usize::from(changed > 0);
                px.push(d.px());
            }
            assert!(
                moved > 650,
                "{setting:?}: the petals stalled ({moved}/699 frames moved)"
            );

            // The point of the saver: a repaint regression is what this
            // catches. In PIXELS, not scanlines — petals are scattered down
            // the whole panel, so they touch most SCANLINES while the rects
            // that carry them cover a fraction of the panel, and a scanline
            // count stopped bounding the copy when damage grew an x extent.
            // The bound is what was MEASURED with a margin, not an aspiration.
            px.sort_unstable();
            let median = px[px.len() / 2];
            let panel = p.w * p.h;
            assert!(
                median * 4 < panel,
                "{setting:?}: median damage {median}px of {panel}: the scene is not static"
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
            for style in [Style::Upright, Style::Windswept] {
                for seed in SEEDS {
                    let s = at_style(seed, setting, style);
                    let (above, _) = census(&s);
                    let cols = s.grid.cols();
                    // Wood is counted AFTER the blossom has overdrawn it, so a
                    // densely flowering tree shows very little. BOTH woods are
                    // asserted because the trunk is drawn in both — its lit
                    // side in `TRUNK` and its shadow side in `BRANCH` — and a
                    // trunk that lost its shading would still pass a test that
                    // only counted the pair. Measured over 1200 scenes: lit
                    // 9..74 cells, dark 13..186.
                    let wood = above[TRUNK as usize] + above[BRANCH as usize];
                    assert!(
                        wood > 18,
                        "{setting:?}/{style:?}/{seed:#x}: only {wood} cells of wood"
                    );
                    assert!(
                        above[TRUNK as usize] > 6,
                        "{setting:?}/{style:?}/{seed:#x}: only {} cells of lit wood",
                        above[TRUNK as usize]
                    );
                    assert!(
                        above[BRANCH as usize] > 8,
                        "{setting:?}/{style:?}/{seed:#x}: only {} cells of shadowed wood",
                        above[BRANCH as usize]
                    );
                    // The floor is what stops a bare tree, the ceiling what
                    // stops a pink cloud. A swept crown is a tail rather than a
                    // ball and carries less: measured 208..905 against the
                    // upright tree's 322..1005 over 600 scenes of each, so it
                    // gets its own floor rather than the upright one lowered to
                    // fit it.
                    let floor = match style {
                        Style::Upright => 280,
                        Style::Windswept => 180,
                    };
                    let bloom: usize = BLOSSOM.iter().map(|&c| above[c as usize]).sum();
                    assert!(
                        (floor..2_000).contains(&bloom),
                        "{setting:?}/{style:?}/{seed:#x}: {bloom} blossom cells is not a crown"
                    );
                    // Every tier in use, so the crown is lit from above rather
                    // than flat. The palest is the top of the canopy, which on
                    // a swept tree is the thin leading edge of a wedge instead
                    // of the whole top of a ball: the thinnest tier measured 9
                    // cells at worst against the upright tree's 21.
                    let tier_floor = match style {
                        Style::Upright => 12,
                        Style::Windswept => 5,
                    };
                    for &c in &BLOSSOM {
                        assert!(
                            above[c as usize] > tier_floor,
                            "{setting:?}/{style:?}/{seed:#x}: blossom tier {c} is unused"
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
                    "{setting:?}/{style:?}/{seed:#x}: crown is {w}x{h} — too narrow for a cherry"
                );
                    assert!(
                    w < cols * 19 / 20,
                    "{setting:?}/{style:?}/{seed:#x}: crown is {w} of {cols} columns — that is a hedge"
                );
                    // And it is a tree, not a bush: the crown's foot is clear of
                    // the ground, with trunk under it.
                    assert!(
                        y1 < s.ground_y,
                        "{setting:?}/{style:?}/{seed:#x}: blossom reaches the ground line"
                    );
                    assert!(
                        y0 < s.ground_y / 2,
                        "{setting:?}/{style:?}/{seed:#x}: the crown never reaches the upper half"
                    );
                }
            }
        }
    }

    /// The trunk `build` would plant, for the tests that want to grow a great
    /// many of them. The ranges are copied out of `build` rather than shared
    /// with it: what these tests stand in for is the tree the saver actually
    /// plants, and a helper both sides called could drift away from the panel
    /// together without anything noticing.
    fn a_trunk(rng: &mut u32, cols: usize, ground_y: i32, style: Style) -> Trunk {
        let stand = match style {
            Style::Upright => 26 + (next_rand(rng) % 17) as i32,
            Style::Windswept => 15 + (next_rand(rng) % 13) as i32,
        };
        let top = ground_y * (55 + (next_rand(rng) % 20) as i32) / 100;
        Trunk {
            x: (cols as i32 * stand / 100) * 2,
            y: ground_y * 4,
            len: (((ground_y - top) * 4) * ASPECT_DEN / ASPECT_NUM).max(8) * style.reach() / 100,
            // Positive, so downwind is the panel's right in every test here.
            lean: style.lean(rng),
            thick: 6,
            style,
        }
    }

    /// A 1920x1080 panel at the default 12x16 cell, at the lowest and the
    /// highest horizon any setting asks for.
    const COLS: usize = 160;
    const ROWS: usize = 67;
    fn horizons() -> [i32; 2] {
        [
            (ROWS as i64 * Setting::Mountain.horizon_pct() / 100) as i32,
            (ROWS as i64 * Setting::Pond.horizon_pct() / 100) as i32,
        ]
    }

    /// Every tree `plant` hands back is one the split test passes and one that
    /// fits on the panel.
    ///
    /// The photo that started the first of those showed the failure: two
    /// matched lobes with a hole down the middle straddling the trunk, which
    /// the eye reads as one sprite flipped rather than as a tree. The first
    /// fork throws its limbs symmetrically about the trunk and none of them
    /// near the vertical, so the shape rolls landed there about one crown in
    /// six. The second is what the windswept style made obvious: a crown grown
    /// off the top of the panel loses its palest tier, which is the lit top of
    /// the canopy, and the tree goes flat.
    ///
    /// Swept rather than pinned to `SEEDS` for the same reason in both cases:
    /// eight trees cannot say anything about a shape that came up one time in
    /// six.
    #[test]
    fn no_tree_is_planted_split_down_the_middle_or_off_the_panel() {
        for style in [Style::Upright, Style::Windswept] {
            for seed in 1..200u32 {
                for ground_y in horizons() {
                    let mut rng = seed.wrapping_mul(0x9E37_79B9) ^ ground_y as u32;
                    let mut r = Raster::new(COLS, ROWS);
                    let t = a_trunk(&mut rng, COLS, ground_y, style);
                    plant(&mut r, &mut rng, t, COLS * ROWS * 3 / 4);
                    assert!(
                        !splits_evenly(&r.tips),
                        "{style:?}/seed {seed}/horizon {ground_y}: the crown is a mirrored pair"
                    );
                    assert!(
                        fits(&r.tips),
                        "{style:?}/seed {seed}/horizon {ground_y}: the crown grew off the panel"
                    );
                }
            }
        }
    }

    /// What makes the windswept style windswept: the whole crown is thrown
    /// clear of the trunk downwind and the windward side is left bare. Measured
    /// beside the upright tree, because "the crown is downwind" says nothing
    /// unless a crown that is not downwind measures differently — the upright
    /// numbers here are the control, and they are the ones that would go on
    /// passing if `Shape::sweep` were quietly dropped.
    ///
    /// Tips, not cells: the sapling's blossom is in the scene too, and this is
    /// about one tree.
    #[test]
    fn the_windswept_style_combs_the_whole_crown_downwind() {
        for seed in 1..120u32 {
            for ground_y in horizons() {
                for style in [Style::Upright, Style::Windswept] {
                    let mut rng = seed.wrapping_mul(0x9E37_79B9) ^ ground_y as u32;
                    let mut r = Raster::new(COLS, ROWS);
                    let t = a_trunk(&mut rng, COLS, ground_y, style);
                    plant(&mut r, &mut rng, t, COLS * ROWS * 3 / 4);
                    let n = r.tips.len().max(1) as i32;
                    // Where the crown sits relative to the trunk, in
                    // sub-columns, and how much of it is upwind of it.
                    let drift = r.tips.iter().map(|p| p.0).sum::<i32>() / n - t.x;
                    let upwind = r.tips.iter().filter(|p| p.0 < t.x).count() as i32 * 100 / n;
                    let at = format!("{style:?}/seed {seed}/horizon {ground_y}");
                    // Measured over 1200 trees of each style: swept drift
                    // 12..87 sub-columns against the upright tree's -38..11,
                    // and swept upwind 0..48% against upright 22..100%. Drift
                    // is the measure that separates the two cleanly; the
                    // upwind share is the looser one, and both bounds are the
                    // measured ranges with room rather than the midpoint
                    // between them.
                    match style {
                        Style::Upright => {
                            assert!(drift < 25, "{at}: upright crown drifted {drift} downwind");
                            assert!(
                                upwind > 10,
                                "{at}: only {upwind}% of an upright crown is upwind"
                            );
                        }
                        Style::Windswept => {
                            assert!(drift > 5, "{at}: swept crown only drifted {drift} downwind");
                            assert!(upwind < 60, "{at}: {upwind}% of a swept crown is upwind");
                        }
                    }
                }
            }
        }
    }

    /// `splits_evenly` faults BALANCE, not gaps. A crown full of holes is a
    /// cherry; the one arrangement it rejects is the hole with as much tree on
    /// one side of it as the other.
    #[test]
    fn the_split_test_faults_balance_not_holes() {
        /// A solid lobe `w` by `h` coarse cells, its left edge `q` cells in.
        fn lobe(q: i32, w: i32, h: i32) -> Vec<(i32, i32)> {
            (0..w)
                .flat_map(move |x| (0..h).map(move |y| ((q + x) * SPLIT_Q, y * SPLIT_Q * 2)))
                .collect()
        }
        assert!(
            splits_evenly(&[lobe(0, 6, 4), lobe(20, 6, 4)].concat()),
            "two matched lobes with a hole between them is the whole fault"
        );
        assert!(
            !splits_evenly(&[lobe(0, 6, 6), lobe(20, 3, 3)].concat()),
            "a crown four times the size of what hangs off the other side is a tree, not a mirror"
        );
        assert!(
            !splits_evenly(&lobe(0, 26, 4)),
            "a crown with no hole in it cannot be split by one"
        );
        assert!(
            !splits_evenly(&[lobe(0, 2, 2), lobe(20, 2, 2)].concat()),
            "a sapling is a smudge, and a smudge cannot be symmetrical"
        );
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
            // is what a real reflection does — and a tree with a tall bare
            // trunk loses most of its crown that way. Measured 57..296 over
            // these seeds and both styles, so the floor is what proves there
            // is a reflection at all, not how much of one.
            let mirrored: usize = (13..=18).map(|r| below[r]).sum();
            assert!(mirrored > 40, "{seed:#x}: only {mirrored} reflected cells");
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
            // Two or three stones, and the ones set at the back of the bed are
            // small because the bed is in perspective: measured 29..100 cells
            // over these seeds and both styles, so this is "there are stones",
            // not "there are big ones".
            assert!(
                below[STONE_LIT as usize] + below[STONE_DARK as usize] > 24,
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
