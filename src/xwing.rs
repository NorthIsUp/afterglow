//! xwing — the Death Star run from the cockpit, in three acts on a loop.
//!
//! # The three acts
//!
//! 1. **Approach.** Black space, a starfield, and a grey sphere whose limb
//!    swells until it is the whole frame. The station is lit from one side, so
//!    the terminator and the curvature of the limb are what say "sphere" rather
//!    than "grey circle"; the equatorial trench and the superlaser dish are cut
//!    into the same surface coordinates the plating is generated in, so they
//!    grow with it.
//! 2. **Surface.** The sphere has become a plane: plating rushing under the
//!    camera, with towers and dish masts rising out of it and sliding past.
//! 3. **The trench.** Two walls close in on both sides, the sky narrows to a
//!    slit, green turret fire comes up at the camera and red fire goes out, and
//!    the targeting computer swings down over the view. It ends in a flash and
//!    cuts back to act 1.
//!
//! # The joins are flight, not edits
//!
//! Act 1 ends with the plating covering the frame, and act 2 opens NOSE-DOWN —
//! the steepest possible look at a plain is also plating covering the frame, so
//! the cut lands on matching pixels — and then the horizon sweeps down into
//! place over `XWING_PITCH_MS`, which is what pulling out of a dive looks like
//! from inside it. Act 3 opens on the plain act 2 ended on, already wide, and
//! the walls GROW out of it over `XWING_RISE_MS` before closing in. Only the
//! last join is a cut, and it is behind the explosion that ends act 3.
//!
//! # What else is out there
//!
//! Green fire comes in and red fire goes out in EVERY act — a gun emplacement
//! on the station's face while it is still a sphere ahead, a surface battery
//! over the plain, a wall turret in the trench, and TIE fighters wherever they
//! are. A sortie is one of three: a crossing shot, a pursuit ahead of the
//! camera, or a pass straight across the canopy. Red fire that reaches one
//! takes it, and what is left is an explosion — flash, expanding shell,
//! debris, fade — which also happens on its own to a surface installation
//! every `XWING_BOOM_SECS` or so.
//!
//! # Everything grey is generated, not tiled
//!
//! [`XWing::greeble`] is the only surface texture in the file, and the sphere,
//! the plain and the trench walls all call it with their own two surface
//! coordinates. It hashes at two scales — a block that may be sunken (a trench
//! within a trench) or raised (a housing), and a third-size detail inside it
//! (a vent or a nub) — so nothing repeats at any scale a viewer can catch.
//!
//! Panel seams are drawn as NEGATIVE space: a plated cell is a full braille
//! block with the dots the seam crosses punched out. One colour per cell means a
//! seam drawn as a darker SHADE would have to be a whole cell wide; drawn as
//! unlit dots it is a quarter of one, and it lands where the seam actually is.
//! `seam_pos` is what finds that, and it gives up once a cell is wider than a
//! block — at which point the seams are finer than the panel can draw, and
//! drawing them anyway is moire.
//!
//! # Geometry is in SQUARE-pixel units
//!
//! `SAVER_PIXEL_ASPECT=180` makes a cell 1.8x taller, so `rows` nearly halves —
//! a trench whose walls were placed at "a quarter of `cols`" would close at a
//! different rate on the panel than in a 1080p dump. So every projection here
//! is computed in VISUAL units — 1 unit = the glass width of one framebuffer
//! pixel, and since the panel squashes vertically by `aspect`, a framebuffer
//! pixel is `100 / aspect` units TALL. `Grid` has already multiplied `cell_h`
//! by the aspect to make the cell square on the glass, so `row_v` divides it
//! back out and comes to the same number as `col_v`: that equality is the
//! invariant, and `the_cell_is_square_on_the_glass` pins it. The conversion to
//! cells happens once, at the stamp.
//!
//! The live panel is 1920x1080 at `SAVER_PIXEL_ASPECT=180`, which is 1920x600
//! on the glass — 3.2:1, not 16:9. Nothing here assumes a frame shape; it
//! projects into whatever glass rectangle the panel turns out to be.
//!
//! # Damage model: full repaint (`Grid::flush`)
//!
//! The whole frame moves every frame — that is what the saver IS — so "what
//! changed" is "everything", and `flush` derives that from a u32 compare per
//! cell. Not `flush_sparse`: a hand-maintained dirty list can under-report, and
//! a cell left out of one freezes on the panel for the life of the pod.
//!
//! # Knobs
//!
//! | var | default | what |
//! | --- | --- | --- |
//! | `XWING_SEED` | clock+pid | RNG seed; set it to replay an identical run |
//! | `XWING_APPROACH_SECS` | 11 | act 1, seconds |
//! | `XWING_SURFACE_SECS` | 9 | act 2, seconds |
//! | `XWING_TRENCH_SECS` | 13 | act 3, seconds |
//! | `XWING_SPEED` | 900 | world units flown per second in acts 2 and 3 |
//! | `XWING_GREEBLE` | 60 | plating block size, world units |
//! | `XWING_FOV` | 800 | focal length, thousandths of the visual panel width |
//! | `XWING_STARS` | 170 | stars in the pool |
//! | `XWING_TOWERS` | 16 | towers and dishes in the pool |
//! | `XWING_BOLTS` | 28 | turret/cannon bolts in the pool |
//! | `XWING_TIE_SECS` | 7 | mean seconds between TIE sorties, 0..600; 0 = none |
//! | `XWING_TIES` | 6 | TIEs in the pool, 0..200 — also the rate limiter |
//! | `XWING_BOOM_SECS` | 9 | mean seconds between surface explosions, 0..600; 0 = none |
//! | `XWING_BOOMS` | 6 | explosions in the pool, 0..200 |
//! | `XWING_PITCH_MS` | 1600 | act 2's nose coming up, 0..10000; 0 = a hard cut |
//! | `XWING_RISE_MS` | 1400 | act 3's walls rising, 0..10000; 0 = a hard cut |
//! | `XWING_CELL_W` / `XWING_CELL_H` | 8 / 8 | cell size in pixels |
//!
//! The act lengths are in real time and divided by `fps`, so the run is the
//! same length at 15fps and at 30.
//!
//! # Per-frame cost
//!
//! A full repaint with a couple of divides and two hashes per cell. Measured at
//! 1920x1080 against `moire`: act 1 1.55x, act 2 1.24x, act 3 1.92x — the
//! trench is the expensive one because every cell resolves a wall AND a floor
//! and takes the nearer. TIEs, explosions and the extra fire cost 5-13% of an
//! act each: they are sprites over a bounded box, where the scene is every
//! cell. On the live panel (1920x1080 at aspect 180) the whole cycle is 1.43x
//! `moire`, around 200m of the pod's 500m.

use crate::font;
use crate::grid::{bake, dot_bit, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, next_rand, saver_seed};

/// Cold greys, black space, and the three things that are not grey: green
/// turret fire, red cannon fire, and the targeting computer's amber.
///
/// The greys are blue-shifted at every step (`b > g > r`) because the whole
/// point of this saver beside the other forward-motion ones is that it is cold
/// and nearly monochrome — a neutral ramp reads as concrete.
///
/// The ORDER is load-bearing: cells are composited with `max`, so a brighter
/// index wins the cell. Plating < stars < green < red < amber < the flash.
#[rustfmt::skip]
const PAL_RGB: [[u8; 3]; 24] = [
    [0x00, 0x00, 0x00], //  0 space
    [0x08, 0x0A, 0x0E], //  1 plating, deepest shadow
    [0x11, 0x14, 0x1A],
    [0x1A, 0x1E, 0x26],
    [0x25, 0x2A, 0x34],
    [0x32, 0x38, 0x44],
    [0x40, 0x47, 0x55],
    [0x50, 0x58, 0x68],
    [0x62, 0x6B, 0x7C],
    [0x78, 0x82, 0x94],
    [0x92, 0x9C, 0xAE],
    [0xB4, 0xBE, 0xCE], // 11 plating, sunlit edge
    [0x3A, 0x40, 0x50], // 12 star, far
    [0x8C, 0x96, 0xA8], // 13 star
    [0xE8, 0xEC, 0xF4], // 14 star, near
    [0x1C, 0x50, 0x24], // 15 green bolt, trail
    [0x36, 0xC0, 0x46], // 16 green bolt
    [0xC8, 0xFF, 0xD0], // 17 green bolt, core
    [0x60, 0x12, 0x18], // 18 red bolt, trail
    [0xC8, 0x2C, 0x36], // 19 red bolt
    [0xFF, 0xD0, 0xC8], // 20 red bolt, core
    [0xB0, 0x78, 0x1C], // 21 targeting computer
    [0xFF, 0xC8, 0x40], // 22 targeting computer, lit
    [0xFF, 0xFF, 0xFF], // 23 the hit
];
const PAL: [u32; 24] = bake(&PAL_RGB);

const C_SPACE: u8 = 0;
const C_DARKEST: u8 = 1;
const C_BRIGHT: u8 = 11;
const C_STAR_FAR: u8 = 12;
const C_GREEN_TRAIL: u8 = 15;
const C_GREEN_CORE: u8 = 17;
const C_RED_TRAIL: u8 = 18;
const C_RED_CORE: u8 = 20;
const C_TIE_BODY: u8 = 3;
const C_TIE_BALL: u8 = 5;
const C_TIE_RIM: u8 = 8;
const C_TIE_GLASS: u8 = 10;
const C_HUD: u8 = 21;
const C_HUD_LIT: u8 = 22;
const C_FLASH: u8 = 23;

/// Dots of one braille sub-column, and of one sub-row. Seams are punched out of
/// a full block with these.
const COL_DOTS: [u8; 2] = [
    dot_bit(0, 0) | dot_bit(0, 1) | dot_bit(0, 2) | dot_bit(0, 3),
    dot_bit(1, 0) | dot_bit(1, 1) | dot_bit(1, 2) | dot_bit(1, 3),
];
const ROW_DOTS: [u8; 4] = [
    dot_bit(0, 0) | dot_bit(1, 0),
    dot_bit(0, 1) | dot_bit(1, 1),
    dot_bit(0, 2) | dot_bit(1, 2),
    dot_bit(0, 3) | dot_bit(1, 3),
];
const FULL: u8 = 0xFF;

/// The detail scale inside a plating block, as a fraction of it.
const FINE: f32 = 0.34;

/// Camera height over the plating in acts 2 and 3, world units. The trench is
/// sized against it, so this is the one number the whole ground scale hangs on.
const CAM_H: f32 = 150.0;
/// Nothing is drawn past here; the plating fades to black well before it, so
/// the far end reads as distance rather than as a cut.
const FAR: f32 = 9000.0;
/// A tower or a bolt this close is behind the camera.
const NEAR: f32 = 60.0;
/// Trench half-width at the start of act 3 and at the end of it. It STARTS as
/// wide as the towers of act 2 so the cut lands on a surface that is already
/// there, and closes to a third of that.
const TRENCH_W0: f32 = 1500.0;
const TRENCH_W1: f32 = 430.0;
/// Trench wall height above the camera. The sky slit narrows with the walls.
const TRENCH_TOP: f32 = 900.0;
/// Fraction of act 3 left when the targeting computer starts to swing down.
const HUD_FRAC: f32 = 0.42;
/// How long a bolt is, in seconds of its own flight. Fixed in the WORLD, so a
/// bolt is a rod that foreshortens with distance rather than a streak whose
/// length happens to be however far it moved since the last frame.
const BOLT_SECS: f32 = 0.10;
/// A TIE's wing panel is this tall in world units, half-height. The whole
/// silhouette is four of these across, which is the ratio that makes the
/// hexagons read rather than the ball.
const TIE_R: f32 = 52.0;
/// How long one explosion lasts.
const BOOM_SECS: f32 = 1.15;
/// Debris thrown by one. Derived from the boom's seed rather than pooled: a
/// spark is a direction, and a direction is a hash away.
const BOOM_SPARKS: usize = 14;
/// Seconds of white-out at the very end of act 3. Short: this is a cut, not a
/// strobe, and a long one on a panel two feet from a bed is unkind.
const FLASH_SECS: f32 = 0.22;

/// Two coordinates to one well-mixed word. splitmix-shaped, because the
/// greebles take two hashes from ADJACENT block indices and an LCG's
/// correlation between neighbours is exactly the diagonal banding that would
/// make generated plating read as a tiled pattern.
#[inline]
fn hash2(a: i32, b: i32) -> u32 {
    let mut z = (a as u32)
        .wrapping_mul(0x9E37_79B9)
        .wrapping_add((b as u32).wrapping_mul(0x85EB_CA6B));
    z ^= z >> 15;
    z = z.wrapping_mul(0x2545_F491);
    z ^ (z >> 13)
}

/// Where a panel seam falls inside this cell, 0..1 across it, or None when
/// there is none in the cell.
///
/// `f` is the cell centre's position INSIDE its block, 0..1, and `span` the
/// cell's width in blocks. Once a cell is nearly a whole block wide the seams
/// are finer than the cell and drawing them is aliasing, so this gives up —
/// which doubles as the distance LOD, since `span` grows with depth.
///
/// The fraction is handed in rather than computed: the caller already divided
/// by the block size for the hash, and a second divide plus a `rem_euclid` per
/// cell per axis is four function calls a cell on a full-repaint saver.
#[inline]
fn seam_at(f: f32, span: f32) -> Option<f32> {
    // NaN falls through both comparisons below and yields None.
    if span <= 0.0 || span >= 0.9 {
        return None;
    }
    let half = span * 0.5;
    if f < half {
        Some(0.5 - f / span)
    } else if f > 1.0 - half {
        Some(0.5 + (1.0 - f) / span)
    } else {
        None
    }
}

/// Which part of a TIE is at this point of its own silhouette, in units of a
/// wing panel's half-height, or None for the gaps — which are most of it.
///
/// The proportions are the whole point: two hexagonal panels at `|dx| ~ 1.5`
/// with cut corners, a ball at the origin, and the struts between. At eight
/// cells across, that outline is still the only thing in the rotation it could
/// be; drawn as two rectangles it is a barbell.
#[inline]
fn tie_part(dx: f32, dy: f32) -> Option<u8> {
    let (ax, ay) = (dx.abs(), dy.abs());
    if (0.95..=2.05).contains(&ax) {
        // 0 at the panel's spine, 1 at its outer and inner edge.
        let e = (ax - 1.5).abs() / 0.55;
        if e <= 1.0 && ay <= 1.0 - 0.62 * (e - 0.30).max(0.0) / 0.70 {
            // The rim is what survives when the whole panel is three cells
            // wide: an unframed hexagon at that size is a grey lozenge.
            return Some(if e > 0.82 || ay > 0.80 {
                C_TIE_RIM
            } else {
                C_TIE_BODY
            });
        }
        return None;
    }
    let r2 = dx * dx + dy * dy;
    if r2 < 0.045 {
        return Some(C_TIE_GLASS);
    }
    if r2 < 0.20 {
        return Some(C_TIE_BALL);
    }
    if ay < 0.10 && ax < 1.0 {
        return Some(C_TIE_BODY);
    }
    None
}

#[derive(Clone, Copy, Default)]
struct Star {
    x: f32,
    y: f32,
    dx: f32,
    lev: u8,
}

/// A tower or a dish mast standing on the plating in act 2.
#[derive(Clone, Copy, Default)]
struct Tower {
    /// Across-track offset and distance ahead, world units.
    u: f32,
    z: f32,
    h: f32,
    w: f32,
    /// 0 = blockhouse, 1 = dish on a mast.
    kind: u8,
}

/// One bolt. Green ones come up at the camera; red ones leave it.
#[derive(Clone, Copy, Default)]
struct Bolt {
    u: f32,
    v: f32,
    z: f32,
    du: f32,
    dv: f32,
    dz: f32,
    live: bool,
    /// Colour base: `C_GREEN_TRAIL` or `C_RED_TRAIL`.
    tint: u8,
}

/// A TIE fighter. Three sorties use the same struct and differ only in the
/// velocity they are born with — a crossing shot, a pursuit ahead of the
/// camera, and a pass right across the canopy.
#[derive(Clone, Copy, Default)]
struct Tie {
    u: f32,
    v: f32,
    z: f32,
    du: f32,
    dv: f32,
    dz: f32,
    /// Seconds until it shoots. Negative once it has.
    fire_in: f32,
    live: bool,
}

/// One explosion: a flash, an expanding shell, and debris.
#[derive(Clone, Copy, Default)]
struct Boom {
    u: f32,
    v: f32,
    z: f32,
    /// World radius the shell reaches.
    r: f32,
    age: f32,
    /// Fixes this explosion's debris directions for its whole life — a boom
    /// whose sparks were re-rolled every frame is a ball of static.
    seed: u32,
    live: bool,
}

pub struct XWing {
    grid: Grid,
    /// Braille dots and palette index per cell — the whole frame, composited
    /// here and handed to the grid in one `fill`. See `hardrain`, same shape.
    pat: Vec<u8>,
    col: Vec<u8>,
    cols: usize,
    rows: usize,

    /// Visual (square-pixel) geometry. `col_v`/`row_v` are one cell; `sub_w`/
    /// `sub_h` one braille dot.
    col_v: f32,
    row_v: f32,
    sub_w: f32,
    sub_h: f32,
    vw: f32,
    vh: f32,
    hx: f32,
    hy: f32,
    focal: f32,

    act: u8,
    t: f32,
    dt: f32,
    act_secs: [f32; 3],
    /// Distance flown in acts 2 and 3 — what slides the plating past.
    travel: f32,
    speed: f32,
    greeble: f32,
    /// Apparent radius of the station at the start and end of act 1, in visual
    /// units. Interpolated geometrically, so the swell accelerates.
    ar0: f32,
    ar1: f32,
    /// The horizon, after the pitch that opens act 2. Everything in acts 2 and
    /// 3 projects about THIS and not about `hy`: a pitch is exactly the
    /// vanishing point moving up or down the frame.
    hz: f32,
    /// Seconds the nose takes to come up at the start of act 2, and the walls
    /// take to rise at the start of act 3.
    pitch_secs: f32,
    rise_secs: f32,
    /// The trench's half-width this frame, for the turrets on its walls.
    trench_w: f32,
    /// Seconds until the next TIE sortie and the next surface explosion. A
    /// zero interval turns that event off.
    next_tie: f32,
    tie_secs: f32,
    next_boom: f32,
    boom_secs: f32,
    /// Seconds until the next turret bolt, and the cannon's burst state.
    next_turret: f32,
    next_cannon: f32,

    stars: Vec<Star>,
    towers: Vec<Tower>,
    bolts: Vec<Bolt>,
    ties: Vec<Tie>,
    booms: Vec<Boom>,
    rng: u32,
}

impl XWing {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        let cw = env_num(&["XWING_CELL_W"], 8, 4, 32) as usize;
        let ch = env_num(&["XWING_CELL_H"], 8, 4, 32) as usize;
        let grid = Grid::new(panel, cw, ch);
        let (cols, rows) = (grid.cols(), grid.rows());

        // `cell_h` has ALREADY been stretched by SAVER_PIXEL_ASPECT, so
        // turning it back into the height the cell occupies on the glass is a
        // DIVISION. Multiplying applies the correction a second time, in the
        // wrong direction: at 180 that is 3.24x (the aspect squared) too tall,
        // and the whole shot composes into a 1:1 frame that the panel then
        // shows on 3.2:1 glass. `marble.rs` does the same conversion and says
        // the same thing.
        let col_v = grid.cell_w() as f32;
        let row_v = grid.cell_h() as f32 * 100.0 / crate::grid::pixel_aspect() as f32;
        let (vw, vh) = (cols as f32 * col_v, rows as f32 * row_v);
        let focal = vw * env_num(&["XWING_FOV"], 800, 200, 3000) as f32 / 1000.0;
        let diag = (vw * vw + vh * vh).sqrt();

        let mut x = Self {
            pat: vec![0; cols * rows],
            col: vec![0; cols * rows],
            cols,
            rows,
            col_v,
            row_v,
            sub_w: col_v / 2.0,
            sub_h: row_v / 4.0,
            vw,
            vh,
            hx: vw / 2.0,
            hy: vh / 2.0,
            focal,
            act: 0,
            t: 0.0,
            dt: 1.0 / fps.max(1) as f32,
            act_secs: [
                env_num(&["XWING_APPROACH_SECS"], 11, 1, 600) as f32,
                env_num(&["XWING_SURFACE_SECS"], 9, 1, 600) as f32,
                env_num(&["XWING_TRENCH_SECS"], 13, 1, 600) as f32,
            ],
            travel: 0.0,
            speed: env_num(&["XWING_SPEED"], 900, 50, 20_000) as f32,
            greeble: env_num(&["XWING_GREEBLE"], 60, 4, 2000) as f32,
            // Sized off the frame's DIAGONAL, not its height: the station
            // has to end act 1 covering every corner, and on the 3.2:1 glass
            // the live panel actually is, a radius set against the height
            // leaves space showing at the sides — which breaks the one thing
            // the cut into act 2 depends on.
            ar0: diag * 0.05,
            ar1: diag * 0.60,
            hz: vh / 2.0,
            pitch_secs: env_num(&["XWING_PITCH_MS"], 1600, 0, 10_000) as f32 / 1000.0,
            rise_secs: env_num(&["XWING_RISE_MS"], 1400, 0, 10_000) as f32 / 1000.0,
            trench_w: TRENCH_W0,
            next_tie: 0.0,
            tie_secs: env_num(&["XWING_TIE_SECS"], 7, 0, 600) as f32,
            next_boom: 0.0,
            boom_secs: env_num(&["XWING_BOOM_SECS"], 9, 0, 600) as f32,
            next_turret: 0.0,
            next_cannon: 0.0,
            stars: vec![Star::default(); env_num(&["XWING_STARS"], 170, 0, 4000) as usize],
            towers: vec![Tower::default(); env_num(&["XWING_TOWERS"], 16, 0, 400) as usize],
            bolts: vec![Bolt::default(); env_num(&["XWING_BOLTS"], 28, 0, 400) as usize],
            ties: vec![Tie::default(); env_num(&["XWING_TIES"], 6, 0, 200) as usize],
            booms: vec![Boom::default(); env_num(&["XWING_BOOMS"], 6, 0, 200) as usize],
            rng: saver_seed(&["XWING_SEED"], 0x58_57_49_4E),
            grid,
        };
        for i in 0..x.stars.len() {
            x.spawn_star(i, false);
        }
        x.enter_act();
        x
    }

    #[inline]
    fn unit01(&mut self) -> f32 {
        (next_rand(&mut self.rng) & 0xFFFF) as f32 / 65536.0
    }

    /// -1..1.
    #[inline]
    fn unit11(&mut self) -> f32 {
        self.unit01() * 2.0 - 1.0
    }

    fn spawn_star(&mut self, i: usize, at_edge: bool) {
        let (vw, vh) = (self.vw, self.vh);
        let x = if at_edge { vw } else { self.unit01() * vw };
        let y = self.unit01() * vh;
        // A slow drift only, and only across: it is parallax from a ship that
        // is nearly pointed at what it is flying towards, not a starfield the
        // camera is turning through.
        let dx = -(0.04 + self.unit01() * 0.10) * self.col_v;
        let r = self.unit01();
        self.stars[i] = Star {
            x,
            y,
            dx,
            lev: C_STAR_FAR + (r * r * 3.0) as u8,
        };
    }

    /// Reset the pools and the clocks for whichever act was just entered. This
    /// is on the render path — it runs at an act boundary — so it must not
    /// allocate: every pool is written IN PLACE and none of them is resized.
    fn enter_act(&mut self) {
        self.travel = 0.0;
        // Fire carries across the cut rather than restarting with it: an act
        // change is meant to read as continuing flight, and a frame where
        // every bolt in the air vanishes at once is an edit.
        self.next_turret = 0.35 + self.unit01() * 0.6;
        if self.act == 1 {
            for i in 0..self.towers.len() {
                let z = NEAR + self.unit01() * (FAR - NEAR);
                self.respawn_tower(i, z);
            }
        }
    }

    fn respawn_tower(&mut self, i: usize, z: f32) {
        // Off to the sides of the flight path, never on it: a tower the camera
        // flies THROUGH is a grey wipe, not a tower.
        let side = if self.unit01() < 0.5 { -1.0 } else { 1.0 };
        let u = side * (330.0 + self.unit01() * 1500.0);
        let h = 120.0 + self.unit01() * 520.0;
        let w = 45.0 + self.unit01() * 130.0;
        let kind = u8::from(self.unit01() < 0.32);
        self.towers[i] = Tower { u, z, h, w, kind };
    }

    /// Advance the clock and cut to the next act when this one is up.
    fn advance(&mut self) {
        self.t += self.dt;
        if self.act > 0 {
            self.travel += self.speed * self.dt;
        }
        if self.t >= self.act_secs[self.act as usize] {
            self.t = 0.0;
            self.act = (self.act + 1) % 3;
            self.enter_act();
        }
        // Act 2 opens nose-down: act 1 ended with plating filling the frame,
        // and the steepest possible look at a plain is plating filling the
        // frame too, so the cut lands on matching pixels. The horizon then
        // sweeps DOWN into place, which is what pulling out of a dive looks
        // like from inside it. Smoothstep, so the nose eases rather than
        // arriving at the level with a corner in the motion.
        self.hz = self.hy;
        if self.act == 1 && self.t < self.pitch_secs {
            let k = self.t / self.pitch_secs;
            let ease = k * k * (3.0 - 2.0 * k);
            self.hz = self.hy - (self.hy + self.vh * 0.6) * (1.0 - ease);
        }
        self.tick_events();
    }

    /// How far the trench walls have risen, 0..1. Act 3 opens on the plain act
    /// 2 ended on and the walls grow out of it — a trench that arrived at full
    /// height on one frame is the edit this replaces.
    #[inline]
    fn rise(&self) -> f32 {
        if self.act != 2 || self.rise_secs <= 0.0 {
            return 1.0;
        }
        let k = (self.t / self.rise_secs).min(1.0);
        k * k * (3.0 - 2.0 * k)
    }

    /// The two occasional events, on their own clocks so they survive an act
    /// change: a TIE sortie and a surface explosion.
    fn tick_events(&mut self) {
        if self.tie_secs > 0.0 {
            self.next_tie -= self.dt;
            if self.next_tie <= 0.0 {
                // Mean interval, jittered either side: a fixed period reads as
                // a metronome once you have watched it twice.
                self.next_tie = self.tie_secs * (0.55 + self.unit01() * 0.9);
                self.spawn_tie();
            }
        }
        // Surface installations only: in space there is nothing to blow up
        // that a TIE did not already fly into.
        if self.boom_secs > 0.0 && self.act > 0 {
            self.next_boom -= self.dt;
            if self.next_boom <= 0.0 {
                self.next_boom = self.boom_secs * (0.55 + self.unit01() * 0.9);
                let side = if self.unit01() < 0.5 { -1.0 } else { 1.0 };
                let u = if self.act == 2 {
                    side * self.trench_w * 0.9
                } else {
                    side * (400.0 + self.unit01() * 1400.0)
                };
                let z = 3200.0 + self.unit01() * 3400.0;
                let v = CAM_H - self.unit01() * 260.0;
                let r = 150.0 + self.unit01() * 120.0;
                self.spawn_boom(u, v, z, r);
            }
        }
    }

    /// Fraction through the current act, 0..1.
    #[inline]
    fn phase(&self) -> f32 {
        (self.t / self.act_secs[self.act as usize]).clamp(0.0, 1.0)
    }

    // ---- stamping -------------------------------------------------------

    /// One braille dot at a visual position, brightest colour wins the cell.
    #[inline]
    fn dot(&mut self, vx: f32, vy: f32, lev: u8) {
        if vx < 0.0 || vy < 0.0 || vx >= self.vw || vy >= self.vh {
            return;
        }
        let (cx, cy) = ((vx / self.col_v) as usize, (vy / self.row_v) as usize);
        if cx >= self.cols || cy >= self.rows {
            return;
        }
        let i = cy * self.cols + cx;
        let sx = (((vx - cx as f32 * self.col_v) / self.sub_w) as usize).min(1);
        let sy = (((vy - cy as f32 * self.row_v) / self.sub_h) as usize).min(3);
        self.pat[i] |= dot_bit(sx, sy);
        self.col[i] = self.col[i].max(lev);
    }

    /// A whole cell, replacing whatever was there. The plating and the
    /// silhouettes use this; anything that should sit ON them uses `dot`.
    #[inline]
    fn put(&mut self, cx: usize, cy: usize, bits: u8, lev: u8) {
        if cx < self.cols && cy < self.rows {
            let i = cy * self.cols + cx;
            self.pat[i] = bits;
            self.col[i] = lev;
        }
    }

    /// A filled visual rectangle, clipped to the grid. Cell-granular on
    /// purpose: these are silhouettes seen at distance, and a sub-cell edge on
    /// a shape that is three cells wide buys nothing.
    fn rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, lev: u8) {
        let cx0 = (x0 / self.col_v).floor().max(0.0) as usize;
        let cy0 = (y0 / self.row_v).floor().max(0.0) as usize;
        let cx1 = ((x1 / self.col_v).ceil().max(0.0) as usize).min(self.cols);
        let cy1 = ((y1 / self.row_v).ceil().max(0.0) as usize).min(self.rows);
        for cy in cy0..cy1 {
            for cx in cx0..cx1 {
                self.put(cx, cy, FULL, lev);
            }
        }
    }

    // ---- the surface texture --------------------------------------------

    /// The plating, at two scales. `a` and `b` are surface coordinates in world
    /// units; `span_a`/`span_b` are how many blocks one cell covers along each,
    /// which is what decides whether the seams are still worth drawing.
    ///
    /// Returns the palette index and the braille pattern — the seams are the
    /// dots MISSING from a full block.
    #[inline]
    fn greeble(&self, a: f32, b: f32, span_a: f32, span_b: f32, light: f32) -> (u8, u8) {
        let g = self.greeble;
        let detail = span_a.max(span_b);
        // Past a cell per block there is no block left to draw, and hashing one
        // anyway is per-cell noise that reads as a band of static along the
        // horizon — where EVERY surface here converges. Fade to the flat shade
        // instead: distance is supposed to lose detail.
        if detail >= 2.2 {
            return (
                ((5.5 * light) as i32).clamp(C_DARKEST as i32, C_BRIGHT as i32) as u8,
                FULL,
            );
        }
        // One divide per axis for the whole function: the block index, the fine
        // detail index and the seam fraction all come off it.
        let (qa, qb) = (a / g, b / g);
        let (fa, fb) = (qa.floor(), qb.floor());
        let h = hash2(fa as i32, fb as i32);
        let mut s = 4 + (h & 3) as i32;
        if h & 0x1C == 0 {
            s -= 3; // a block sunk into the surface: a trench within the trench
        } else if h & 0xE0 == 0 {
            s += 3; // a housing standing proud of it
        }
        // A third the size, offset, so the detail inside a block never lines up
        // with the block: vents read as recesses, nubs as hardware. Dropped as
        // soon as it is finer than a cell, for the same reason as above.
        if detail < FINE {
            let hf = hash2(
                (qa / FINE).floor() as i32 ^ 0x5A5A,
                (qb / FINE).floor() as i32,
            );
            if hf & 7 == 0 {
                s += 1;
            } else if hf & 0x38 == 0 {
                s -= 1;
            }
        }
        let lev = ((s as f32 * light) as i32).clamp(C_DARKEST as i32, C_BRIGHT as i32) as u8;

        let mut bits = FULL;
        if let Some(p) = seam_at(qa - fa, span_a) {
            bits &= !COL_DOTS[((p * 2.0) as usize).min(1)];
        }
        if let Some(p) = seam_at(qb - fb, span_b) {
            bits &= !ROW_DOTS[((p * 4.0) as usize).min(3)];
        }
        (lev, bits)
    }

    // ---- act 1: approach -------------------------------------------------

    fn approach(&mut self) {
        // Geometric, not linear: the station swells the way something you are
        // closing on at a constant speed swells, slowly then all at once.
        let ar = self.ar0 * (self.ar1 / self.ar0).powf(self.phase());
        let inv_ar = 1.0 / ar;
        // Off centre, so the limb crosses the frame at an angle and the equator
        // is not a line through the middle of the screen.
        let dcx = self.hx + self.vw * 0.07;
        let dcy = self.hy - self.vh * 0.09;
        // A slow roll, so the surface turns under the approach.
        let spin = self.t * 0.035;
        let (cs, sn) = (spin.cos(), spin.sin());

        let cx0 = (((dcx - ar) / self.col_v).floor().max(0.0) as usize).min(self.cols);
        let cx1 = ((((dcx + ar) / self.col_v).ceil().max(0.0)) as usize).min(self.cols);
        let cy0 = (((dcy - ar) / self.row_v).floor().max(0.0) as usize).min(self.rows);
        let cy1 = ((((dcy + ar) / self.row_v).ceil().max(0.0)) as usize).min(self.rows);
        // A cell's worth of surface, for the seam LOD. The limb compresses it,
        // but a seam a cell wide at the centre is a seam nobody can see at the
        // rim either way.
        let span = self.col_v * inv_ar * 2.2 / self.greeble * 900.0;

        for cy in cy0..cy1 {
            let vy = (cy as f32 + 0.5) * self.row_v;
            let ny = (vy - dcy) * inv_ar;
            for cx in cx0..cx1 {
                let vx = (cx as f32 + 0.5) * self.col_v;
                let nx = (vx - dcx) * inv_ar;
                let n2 = nx * nx + ny * ny;
                if n2 >= 1.0 {
                    // Off the disc: space, and the stars already drawn stay.
                    // The limb cells straddling the edge are handled below.
                    if n2 < 1.25 {
                        self.limb(cx, cy, dcx, dcy, ar, nx, ny, cs, sn, span);
                    }
                    continue;
                }
                let nz = (1.0 - n2).sqrt();
                let (lev, bits) = self.sphere(nx, ny, nz, cs, sn, span);
                self.put(cx, cy, bits, lev);
            }
        }
    }

    /// Shade and pattern for one point on the station's surface, given its
    /// normal. The surface coordinates are gnomonic (`n / (nz + k)`), which is
    /// what compresses the plating towards the limb — a linear mapping there
    /// reads as a flat disc with a texture on it.
    #[inline]
    fn sphere(&self, nx: f32, ny: f32, nz: f32, cs: f32, sn: f32, span: f32) -> (u8, u8) {
        // Light from over the left shoulder, so the terminator cuts the disc
        // and the lit rim is opposite it.
        let lam = nx * -0.46 + ny * -0.52 + nz * 0.72;
        let k = 1.0 / (nz + 0.30);
        // Rolled, so a fixed feature on the surface turns with the approach.
        let (rx, ry) = (nx * cs - ny * sn, nx * sn + ny * cs);
        let (u, v) = (rx * k, ry * k);
        let light = 0.30 + 1.05 * lam.max(0.0);

        // The equatorial trench, and the dish, are cut in the SAME coordinates
        // the plating is generated in, so they swell with it for free.
        if v.abs() < 0.040 {
            return (C_DARKEST, FULL);
        }
        if v.abs() < 0.058 {
            return (((6.0 * light) as u8).clamp(C_DARKEST, C_BRIGHT), FULL);
        }
        let (du, dv) = (u - 0.34, v + 0.40);
        let d2 = du * du + dv * dv;
        if d2 < 0.0529 {
            // The superlaser dish: a bowl of concentric steps with a hot eye.
            let d = d2.sqrt();
            let step = (d * 42.0) as i32 & 1;
            let lev = if d < 0.022 {
                C_BRIGHT
            } else {
                (((3 + step * 2) as f32 * light) as u8).clamp(C_DARKEST, C_BRIGHT)
            };
            return (lev, FULL);
        }
        if d2 < 0.0676 {
            return ((((9.0) * light) as u8).clamp(C_DARKEST, C_BRIGHT), FULL);
        }
        // 900 is the surface's arbitrary world scale: it only has to put a
        // `greeble` block at a few cells across when the station fills the
        // frame, which is where the plating has to read as plating.
        self.greeble(u * 900.0, v * 900.0, span, span, light)
    }

    /// A cell the limb crosses: test all eight dots rather than the centre, so
    /// the edge of the station is a quarter-cell curve and not a staircase.
    #[expect(clippy::too_many_arguments, reason = "the projection's whole state")]
    fn limb(
        &mut self,
        cx: usize,
        cy: usize,
        dcx: f32,
        dcy: f32,
        ar: f32,
        nx: f32,
        ny: f32,
        cs: f32,
        sn: f32,
        span: f32,
    ) {
        let inv_ar = 1.0 / ar;
        let mut bits = 0u8;
        for sy in 0..4 {
            for sx in 0..2 {
                let vx = (cx as f32 + (sx as f32 + 0.5) * 0.5) * self.col_v;
                let vy = (cy as f32 + (sy as f32 + 0.5) * 0.25) * self.row_v;
                let (dx, dy) = ((vx - dcx) * inv_ar, (vy - dcy) * inv_ar);
                if dx * dx + dy * dy < 1.0 {
                    bits |= dot_bit(sx, sy);
                }
            }
        }
        if bits == 0 {
            return;
        }
        // Shaded from the cell centre's normal, pushed onto the sphere: at the
        // limb `nz` is zero and the exact normal is unstable.
        let n2 = (nx * nx + ny * ny).min(0.999);
        let (lev, _) = self.sphere(nx, ny, (1.0 - n2).sqrt(), cs, sn, span);
        let i = cy * self.cols + cx;
        self.pat[i] = bits;
        self.col[i] = lev;
    }

    // ---- acts 2 and 3: the ground ----------------------------------------

    /// The plating of the plane below the camera, for one row of cells. `sy` is
    /// the row's distance below the horizon in visual units; rows at or above
    /// it are sky and are not called.
    ///
    /// Returns the depth of that row, so the caller can decide what else is in
    /// front of it.
    fn floor_row(&mut self, cy: usize, sy: f32, x0: usize, x1: usize) -> f32 {
        let z = self.focal * CAM_H / sy;
        if z > FAR {
            return z;
        }
        // Squared, so the far end of the plain genuinely goes to black instead
        // of ending in a flat grey slab across the vanishing point.
        let fog = 1.0 - z / FAR;
        let light = 0.16 + 0.34 * fog + 0.52 * fog * fog;
        // One cell spans this much depth and this much width at this row.
        let span_z = (self.focal * CAM_H / (sy + self.row_v) - z).abs() / self.greeble;
        let span_u = self.col_v * z / self.focal / self.greeble;
        let zw = z + self.travel;
        let du = self.col_v * z / self.focal;
        let mut u = ((x0 as f32 + 0.5) * self.col_v - self.hx) * z / self.focal;
        for cx in x0..x1 {
            let (lev, bits) = self.greeble(u, zw, span_u, span_z, light);
            self.put(cx, cy, bits, lev);
            u += du;
        }
        z
    }

    fn surface(&mut self) {
        let (cols, rows) = (self.cols, self.rows);
        for cy in 0..rows {
            let sy = (cy as f32 + 0.5) * self.row_v - self.hz;
            if sy <= self.row_v * 0.25 {
                continue; // sky: the stars are already there
            }
            self.floor_row(cy, sy, 0, cols);
        }
        self.towers();
    }

    fn towers(&mut self) {
        for i in 0..self.towers.len() {
            let t = self.towers[i];
            let z = t.z - self.travel;
            if z < NEAR {
                // Recycled at the far end rather than allocated: this is the
                // act's steady state and it runs for minutes.
                let far = FAR + self.travel;
                self.respawn_tower(i, far);
                continue;
            }
            if z > FAR {
                continue;
            }
            let sc = self.focal / z;
            let (x0, x1) = (self.hx + (t.u - t.w) * sc, self.hx + (t.u + t.w) * sc);
            if x1 < 0.0 || x0 >= self.vw {
                continue;
            }
            let base = self.hz + CAM_H * sc;
            let top = self.hz + (CAM_H - t.h) * sc;
            let fog = 1.0 - z / FAR;
            let body = (1.0 + 5.0 * fog * fog) as u8;
            if t.kind == 0 {
                self.rect(x0, top, x1, base, body.clamp(C_DARKEST, C_BRIGHT));
                // The lit face: one cell column down the sunward side, which is
                // what stops a blockhouse reading as a hole in the plating.
                self.rect(
                    x1 - self.col_v,
                    top,
                    x1,
                    base,
                    (body + 2).clamp(C_DARKEST, C_BRIGHT),
                );
            } else {
                let mid = (x0 + x1) * 0.5;
                self.rect(mid - self.col_v, top, mid + self.col_v, base, body);
                // A dish: a wide, shallow cap on the mast.
                let r = (x1 - x0) * 0.9;
                self.rect(
                    mid - r,
                    top - self.row_v,
                    mid + r,
                    top + self.row_v,
                    (body + 3).clamp(C_DARKEST, C_BRIGHT),
                );
            }
        }
    }

    // ---- act 3: the trench -----------------------------------------------

    fn trench(&mut self) {
        // The walls close in over the act. Linear, and it is the one motion in
        // the saver that is meant to be felt rather than seen.
        let p = self.phase();
        let w = TRENCH_W0 + (TRENCH_W1 - TRENCH_W0) * p;
        self.trench_w = w;
        // The walls grow out of the plain act 2 ended on rather than arriving
        // at full height on the first frame of the act.
        let top = TRENCH_TOP * self.rise();
        let (cols, rows) = (self.cols, self.rows);
        for cy in 0..rows {
            let sy = (cy as f32 + 0.5) * self.row_v - self.hz;
            for cx in 0..cols {
                let sx = (cx as f32 + 0.5) * self.col_v - self.hx;
                // Where this ray meets the wall, and where it meets the floor.
                // The nearer of the two is what it sees.
                let zw = if sx.abs() > 1e-3 {
                    self.focal * w / sx.abs()
                } else {
                    FAR
                };
                let zf = if sy > 0.0 {
                    self.focal * CAM_H / sy
                } else {
                    FAR
                };
                if zw < zf && zw < FAR {
                    // Height up the wall, positive downwards. Above the wall's
                    // top edge is the sky slit.
                    let v = sy * zw / self.focal;
                    if v < -top {
                        continue;
                    }
                    let fog = 1.0 - zw / FAR;
                    // A shade under the floor's: the walls face each other and
                    // nothing lights them, which is what makes the trench read
                    // as deeper than the surface it was cut into.
                    let light = 0.16 + 0.32 * fog + 0.44 * fog * fog;
                    let span_z = self.col_v * zw / sx.abs() / self.greeble;
                    let span_v = self.row_v * zw / self.focal / self.greeble;
                    let (lev, bits) =
                        self.greeble(zw + self.travel, v + w * 0.37, span_z, span_v, light);
                    self.put(cx, cy, bits, lev);
                } else if zf < FAR {
                    self.floor_row(cy, sy, cx, cx + 1);
                }
            }
        }
        self.hud(p);
    }

    /// Incoming green fire and outgoing red, in EVERY act — the run is under
    /// fire from the moment the station is in front of you, not only once the
    /// trench walls are up. What changes per act is where the green comes
    /// from: a gun emplacement on the station's face while it is still a
    /// sphere ahead, a surface battery in act 2, a wall turret in act 3, and
    /// it gets busier as the trench closes.
    fn fire(&mut self) {
        self.next_turret -= self.dt;
        if self.next_turret <= 0.0 {
            let (u, v, z) = match self.act {
                // Out of the face of the station: anywhere across the frame,
                // and far enough that the bolt has a flight to be seen in.
                0 => (
                    self.unit11() * 2400.0,
                    self.unit11() * 1100.0,
                    4200.0 + self.unit01() * 3600.0,
                ),
                // Off the plain, beside the flight path, climbing.
                1 => (
                    self.unit11() * 2600.0,
                    CAM_H - self.unit01() * 60.0,
                    3000.0 + self.unit01() * 4000.0,
                ),
                // Off a trench wall, at gun height up it.
                _ => {
                    let side = if self.unit01() < 0.5 { -1.0 } else { 1.0 };
                    (
                        side * self.trench_w * (0.80 + self.unit01() * 0.18),
                        250.0 + self.unit01() * 350.0,
                        2600.0 + self.unit01() * 4200.0,
                    )
                }
            };
            // Sparser out in the open, relentless by the end of the trench.
            let base = match self.act {
                0 => 0.85,
                1 => 0.70,
                _ => 0.55 - 0.35 * self.phase(),
            };
            self.next_turret = base + self.unit01() * 0.5;
            // Aimed near the camera but not at it: a bolt that always hit
            // would read as a hit, and nothing here takes damage.
            let flight = 1.1 + self.unit01() * 0.5;
            let (du, dv) = (
                (self.unit11() * 260.0 - u) / flight,
                (self.unit11() * 200.0 - v) / flight,
            );
            self.spawn_bolt(u, v, z, du, dv, -z / flight, C_GREEN_TRAIL);
        }
        self.next_cannon -= self.dt;
        if self.next_cannon <= 0.0 {
            self.next_cannon = match self.act {
                0 => 1.5,
                1 => 1.2,
                _ => 0.9,
            } + self.unit01() * 1.4;
            // Four cannons, fired as a burst from the wingtips.
            for k in 0..4 {
                let u = if k & 1 == 0 { -340.0 } else { 340.0 };
                let v = if k < 2 { -150.0 } else { 190.0 };
                self.spawn_bolt(u, v, NEAR * 1.5, -u / 2.4, -v / 2.4, 5200.0, C_RED_TRAIL);
            }
        }
    }

    #[expect(clippy::too_many_arguments, reason = "one bolt's whole state")]
    fn spawn_bolt(&mut self, u: f32, v: f32, z: f32, du: f32, dv: f32, dz: f32, tint: u8) {
        // First dead slot, or nothing: the pool is the rate limiter, so a burst
        // that outruns it simply does not fire rather than growing the Vec.
        if let Some(b) = self.bolts.iter_mut().find(|b| !b.live) {
            *b = Bolt {
                u,
                v,
                z,
                du,
                dv,
                dz,
                live: true,
                tint,
            };
        }
    }

    fn bolts(&mut self) {
        for i in 0..self.bolts.len() {
            let mut b = self.bolts[i];
            if !b.live {
                continue;
            }
            b.u += b.du * self.dt;
            b.v += b.dv * self.dt;
            b.z += b.dz * self.dt;
            if b.z < NEAR || b.z > FAR {
                b.live = false;
                self.bolts[i] = b;
                continue;
            }
            // Outgoing fire that reaches a TIE takes it: the one thing in
            // the saver where two objects interact, and the reason the
            // explosions read as consequences rather than as scenery.
            if b.tint == C_RED_TRAIL {
                for j in 0..self.ties.len() {
                    let t = self.ties[j];
                    if t.live
                        && (b.z - t.z).abs() < 320.0
                        && (b.u - t.u).abs() < TIE_R * 1.9
                        && (b.v - t.v).abs() < TIE_R * 1.2
                    {
                        self.ties[j] = Tie::default();
                        b.live = false;
                        self.spawn_boom(t.u, t.v, t.z, TIE_R * 2.4);
                        break;
                    }
                }
                if !b.live {
                    self.bolts[i] = b;
                    continue;
                }
            }
            self.bolts[i] = b;

            // The bolt is a ROD of fixed length in the world, not the smear
            // between two frames: a frame-to-frame streak is a few pixels long
            // while the bolt is still far away, which is exactly when it should
            // already be readable as fire coming at you.
            let tail = (b.z - b.dz * BOLT_SECS).clamp(NEAR, FAR);
            let (sc0, sc1) = (self.focal / tail, self.focal / b.z);
            let (x0, y0) = (
                self.hx + (b.u - b.du * BOLT_SECS) * sc0,
                self.hz + (b.v - b.dv * BOLT_SECS) * sc0,
            );
            let (x1, y1) = (self.hx + b.u * sc1, self.hz + b.v * sc1);
            let (dx, dy) = (x1 - x0, y1 - y0);
            let steps =
                ((dx.abs() / self.sub_w).max(dy.abs() / self.sub_h).ceil() as usize).clamp(1, 64);
            let inv = 1.0 / steps as f32;
            // Thickness is proximity: a bolt about to pass the canopy is a bar
            // of light several dots across, and one at the far end is a spark.
            let fat = ((sc1 * 3.0) as i32).clamp(0, 3);
            for k in 0..=steps {
                let f = k as f32 * inv;
                let (px, py) = (x0 + dx * f, y0 + dy * f);
                // The head is the core colour, the tail the trail colour.
                let lev = b.tint + if k == steps { 2 } else { u8::from(f > 0.5) };
                for t in 0..=fat {
                    let o = t as f32 * 0.5;
                    self.dot(px + o * self.sub_w, py, lev);
                    self.dot(px - o * self.sub_w, py, lev);
                    self.dot(px, py + o * self.sub_h, lev);
                    self.dot(px, py - o * self.sub_h, lev);
                }
            }
        }
    }

    // ---- TIE fighters ----------------------------------------------------

    /// One sortie. Three shapes out of one struct: the kind decides only what
    /// velocity it is born with.
    fn spawn_tie(&mut self) {
        let Some(i) = self.ties.iter().position(|t| !t.live) else {
            // The pool IS the rate limiter. A sortie that finds it full simply
            // does not launch, rather than growing a Vec on the render thread.
            return;
        };
        let kind = next_rand(&mut self.rng) % 3;
        let side = if self.unit01() < 0.5 { -1.0 } else { 1.0 };
        let (u, v, z, du, dv, dz) = match kind {
            // Crossing shot: in from one side, well ahead, gone in a second.
            0 => (
                side * 2400.0,
                self.unit11() * 300.0 - 80.0,
                2400.0 + self.unit01() * 2400.0,
                -side * (1500.0 + self.unit01() * 900.0),
                self.unit11() * 120.0,
                -260.0,
            ),
            // Pursuit: holds station ahead of the camera, weaving, closing
            // slowly — the shot where you are behind one and it cannot shake
            // you. The only sortie that lasts long enough to shoot twice.
            1 => (
                self.unit11() * 700.0,
                self.unit11() * 220.0 - 70.0,
                2600.0 + self.unit01() * 2200.0,
                self.unit11() * 260.0,
                self.unit11() * 110.0,
                -140.0,
            ),
            // Straight across the canopy: comes at you and blows past.
            _ => (
                self.unit11() * 420.0,
                self.unit11() * 200.0,
                6400.0,
                self.unit11() * 600.0,
                self.unit11() * 260.0,
                -3200.0,
            ),
        };
        self.ties[i] = Tie {
            u,
            v,
            z,
            du,
            dv,
            dz,
            fire_in: 0.25 + self.unit01() * 0.9,
            live: true,
        };
    }

    fn ties(&mut self) {
        for i in 0..self.ties.len() {
            let mut t = self.ties[i];
            if !t.live {
                continue;
            }
            t.u += t.du * self.dt;
            t.v += t.dv * self.dt;
            t.z += t.dz * self.dt;
            let x = self.hx + t.u * self.focal / t.z.max(1.0);
            // Retired once it is past the camera or has left the frame far
            // enough that it cannot come back.
            if t.z < NEAR * 3.0 || t.z > FAR || x < -self.vw || x > self.vw * 2.0 {
                self.ties[i] = Tie::default();
                continue;
            }
            t.fire_in -= self.dt;
            if t.fire_in <= 0.0 {
                t.fire_in = 0.9 + self.unit01() * 1.6;
                let flight = (t.z / 3200.0).max(0.45);
                let (du, dv) = (
                    (self.unit11() * 220.0 - t.u) / flight,
                    (self.unit11() * 180.0 - t.v) / flight,
                );
                self.spawn_bolt(t.u, t.v, t.z, du, dv, -t.z / flight, C_GREEN_TRAIL);
            }
            self.ties[i] = t;
            self.draw_tie(&t);
        }
    }

    /// Opaque, per dot, over whatever is behind it: a TIE against the plating
    /// has to occlude it, and a silhouette composited by `max` would let the
    /// brighter greebles show through the panels.
    fn draw_tie(&mut self, t: &Tie) {
        let sc = self.focal / t.z;
        let (x, y) = (self.hx + t.u * sc, self.hz + t.v * sc);
        let s = TIE_R * sc;
        if s < self.sub_h * 0.8 {
            return; // further off than one dot: a speck, and a lying one
        }
        let inv = 1.0 / s;
        let (hw, hh) = (s * 2.1, s * 1.1);
        let cx0 = (((x - hw) / self.col_v).floor().max(0.0) as usize).min(self.cols);
        let cx1 = ((((x + hw) / self.col_v).ceil().max(0.0)) as usize).min(self.cols);
        let cy0 = (((y - hh) / self.row_v).floor().max(0.0) as usize).min(self.rows);
        let cy1 = ((((y + hh) / self.row_v).ceil().max(0.0)) as usize).min(self.rows);
        for cy in cy0..cy1 {
            for cx in cx0..cx1 {
                let (mut bits, mut lev) = (0u8, 0u8);
                for sy in 0..4 {
                    let py = (cy as f32 + (sy as f32 + 0.5) * 0.25) * self.row_v;
                    for sx in 0..2 {
                        let px = (cx as f32 + (sx as f32 + 0.5) * 0.5) * self.col_v;
                        if let Some(l) = tie_part((px - x) * inv, (py - y) * inv) {
                            bits |= dot_bit(sx, sy);
                            lev = lev.max(l);
                        }
                    }
                }
                if bits != 0 {
                    self.put(cx, cy, bits, lev);
                }
            }
        }
    }

    // ---- explosions -------------------------------------------------------

    fn spawn_boom(&mut self, u: f32, v: f32, z: f32, r: f32) {
        let seed = next_rand(&mut self.rng) | 1;
        if let Some(b) = self.booms.iter_mut().find(|b| !b.live) {
            *b = Boom {
                u,
                v,
                z,
                r,
                age: 0.0,
                seed,
                live: true,
            };
        }
    }

    fn booms(&mut self) {
        for i in 0..self.booms.len() {
            let mut b = self.booms[i];
            if !b.live {
                continue;
            }
            b.age += self.dt;
            // The blast stands still in the world and the flight goes past it.
            if self.act > 0 {
                b.z -= self.speed * self.dt;
            }
            if b.age > BOOM_SECS || b.z < NEAR {
                self.booms[i] = Boom::default();
                continue;
            }
            self.booms[i] = b;
            self.draw_boom(&b);
        }
    }

    /// Flash, shell, debris, fade — in that order and overlapping, because all
    /// four at once is a white blob and a viewer reads a white blob as a
    /// dropped frame.
    fn draw_boom(&mut self, b: &Boom) {
        let sc = self.focal / b.z;
        let (x, y) = (self.hx + b.u * sc, self.hz + b.v * sc);
        let a = b.age / BOOM_SECS;
        // Out fast, then stalling: a shell decelerating into its own debris.
        // `sqrt` is the cheap version of that curve and the right shape.
        let r = (b.r * sc * a.sqrt() * 1.9).min(self.vh * 0.9);
        if r < self.sub_h {
            return;
        }
        // The shell thins as it expands, so it reads as a shell rather than as
        // a growing disc.
        let thick = (r * 0.22).max(self.sub_h);
        let (r2, in2) = (r * r, (r - thick) * (r - thick));
        let core = if a < 0.26 {
            (r * 0.8 * (1.0 - a / 0.26)).max(self.sub_h)
        } else {
            0.0
        };
        let core2 = core * core;
        let shell = match a {
            _ if a < 0.30 => C_FLASH,
            _ if a < 0.55 => C_BRIGHT,
            _ if a < 0.80 => 8,
            _ => 5,
        };
        let cy0 = (((y - r) / self.row_v).floor().max(0.0) as usize).min(self.rows);
        let cy1 = ((((y + r) / self.row_v).ceil().max(0.0)) as usize).min(self.rows);
        for cy in cy0..cy1 {
            for sy in 0..4 {
                let py = (cy as f32 + (sy as f32 + 0.5) * 0.25) * self.row_v;
                let dy = py - y;
                let span = (r2 - dy * dy).max(0.0).sqrt();
                let mut px = x - span;
                while px <= x + span {
                    let d2 = (px - x) * (px - x) + dy * dy;
                    if d2 < core2 {
                        self.dot(px, py, C_FLASH);
                    } else if d2 > in2 {
                        self.dot(px, py, shell);
                    }
                    px += self.sub_w;
                }
            }
        }
        // Debris: fixed directions per boom, thrown past the shell and fading
        // through the red end as they go.
        if a > 0.12 {
            let spark = if a < 0.45 {
                C_FLASH
            } else if a < 0.75 {
                C_RED_CORE
            } else {
                C_RED_TRAIL + 1
            };
            for k in 0..BOOM_SPARKS {
                let h = hash2(b.seed as i32, k as i32);
                let ang = (h & 1023) as f32 / 1024.0 * std::f32::consts::TAU;
                let reach = 1.15 + ((h >> 10) & 255) as f32 / 255.0 * 1.5;
                let d = r * reach;
                self.dot(x + ang.cos() * d, y + ang.sin() * d, spark);
            }
        }
    }

    /// The targeting computer, swinging down over the view in the last stretch
    /// of the act and staying there.
    fn hud(&mut self, p: f32) {
        if p < 1.0 - HUD_FRAC {
            return;
        }
        let drop = ((p - (1.0 - HUD_FRAC)) / (HUD_FRAC * 0.45)).clamp(0.0, 1.0);
        // Eased, so it swings and settles rather than sliding.
        let ease = 1.0 - (1.0 - drop) * (1.0 - drop);
        let h = self.vh * 0.34;
        let wdt = self.vw * 0.20;
        let cx = self.hx;
        let top = -h + ease * (h + self.vh * 0.20);
        let bot = top + h;
        // The housing it hangs from, so it reads as hardware lowered into the
        // cockpit rather than as a graphic drawn on the glass.
        self.rect(
            cx - wdt - self.col_v,
            top - self.row_v * 1.5,
            cx + wdt + self.col_v,
            top,
            C_HUD,
        );

        let step = self.sub_h.min(self.sub_w);
        // The frame.
        let mut x = cx - wdt;
        while x <= cx + wdt {
            self.dot(x, top, C_HUD_LIT);
            self.dot(x, bot, C_HUD_LIT);
            x += step;
        }
        let mut y = top;
        while y <= bot {
            self.dot(cx - wdt, y, C_HUD_LIT);
            self.dot(cx + wdt, y, C_HUD_LIT);
            y += step;
        }
        // The crosshair, and the two marks that close on it — the whole reason
        // anyone remembers this display.
        let my = (top + bot) * 0.5;
        let gap = wdt * (1.0 - 0.75 * ease);
        let mut d = -wdt * 0.35;
        while d <= wdt * 0.35 {
            self.dot(cx + d, my, C_HUD);
            d += step;
        }
        // The vertical arm is measured against the box's HEIGHT. Against its
        // width — which is what this did — the crosshair grows out through the
        // top and bottom of its own frame on any panel wider than 16:9.
        let mut d = -h * 0.35;
        while d <= h * 0.35 {
            self.dot(cx, my + d, C_HUD);
            d += step;
        }
        let mut m = -self.row_v * 1.5;
        while m <= self.row_v * 1.5 {
            self.dot(cx - gap, my + m, C_HUD_LIT);
            self.dot(cx + gap, my + m, C_HUD_LIT);
            m += step;
        }
    }

    // ---- always on screen -------------------------------------------------

    fn stars(&mut self) {
        for i in 0..self.stars.len() {
            let mut s = self.stars[i];
            s.x += s.dx;
            if s.x < 0.0 {
                self.spawn_star(i, true);
                continue;
            }
            self.stars[i] = s;
            self.dot(s.x, s.y, s.lev);
        }
    }

    /// The canopy: two struts across the top corners with the S-foil roots
    /// under them, and the console strip along the bottom. Present in every
    /// act, because every act is the same shot out of the same cockpit.
    fn cockpit(&mut self) {
        let (cols, rows) = (self.cols, self.rows);
        // Struts: a diagonal from each top corner, two cells thick.
        let run = (cols / 5).max(2);
        let rise = (rows / 6).max(1);
        for k in 0..run {
            let depth = rise - k * rise / run;
            for d in 0..=depth {
                self.put(k, d, FULL, C_DARKEST);
                self.put(cols - 1 - k, d, FULL, C_DARKEST);
            }
            self.put(k, depth, FULL, 5);
            self.put(cols - 1 - k, depth, FULL, 5);
        }
        // The console: a dark strip with a row of indicator lights.
        let strip = (rows / 12).max(1);
        for cy in rows - strip..rows {
            for cx in 0..cols {
                self.put(cx, cy, FULL, 2);
            }
        }
        let cy = rows - strip;
        for k in 0..cols / 6 {
            let cx = cols / 2 - cols / 12 + k;
            // Blinks off the act clock, so the console is alive without a
            // second timer: the pattern walks as the run goes on.
            let on = (self.t * 3.0) as usize + k;
            let lev = match on % 5 {
                0 => C_RED_CORE - 1,
                1 => C_GREEN_CORE - 1,
                2 => C_HUD_LIT,
                _ => 4,
            };
            self.put(cx, cy, FULL, lev);
        }
    }

    /// The hit: the last fraction of a second of act 3 blows out from the far
    /// end of the trench.
    ///
    /// A disc rather than a full-screen wash: a wash is a dropped frame with a
    /// grey in it, where something arriving OUT of the vanishing point is the
    /// shot the whole act was building to. It runs over everything, targeting
    /// computer included.
    fn flash(&mut self) {
        if self.act != 2 {
            return;
        }
        let left = self.act_secs[2] - self.t;
        if left > FLASH_SECS {
            return;
        }
        let r = (1.0 - left / FLASH_SECS) * self.vh * 1.7;
        let (r2, rim2) = (r * r, (r * 0.72) * (r * 0.72));
        let cy0 = (((self.hy - r) / self.row_v).floor().max(0.0) as usize).min(self.rows);
        let cy1 = ((((self.hy + r) / self.row_v).ceil().max(0.0)) as usize).min(self.rows);
        for cy in cy0..cy1 {
            let dy = (cy as f32 + 0.5) * self.row_v - self.hy;
            for cx in 0..self.cols {
                let dx = (cx as f32 + 0.5) * self.col_v - self.hx;
                let d2 = dx * dx + dy * dy;
                if d2 > r2 {
                    continue;
                }
                self.put(cx, cy, FULL, if d2 < rim2 { C_FLASH } else { C_BRIGHT });
            }
        }
    }
}

impl Saver for XWing {
    fn render(&mut self, s: &mut Surface<'_>) {
        // Rebuilt from zero every frame, so no trail is expressible. Two
        // memsets over one byte per cell, which is nothing beside the blit.
        self.pat.fill(0);
        self.col.fill(C_SPACE);

        self.advance();
        self.stars();
        match self.act {
            0 => self.approach(),
            1 => self.surface(),
            _ => self.trench(),
        }
        // Order is the depth sort: the scene is opaque, TIEs occlude it, fire
        // and explosions composite over both by `max`, then the canopy the
        // whole thing is seen through.
        self.ties();
        self.fire();
        self.bolts();
        self.booms();
        self.cockpit();
        self.flash();

        let (pat, col, cols) = (&self.pat, &self.col, self.grid.cols());
        self.grid.fill(|cx, cy| {
            let i = cy * cols + cx;
            Cell::new(font::BRAILLE[pat[i] as usize], col[i] as u16)
        });
        // `flush`, not `flush_sparse`: damage is a u32 compare per cell and
        // cannot under-report. See the module doc.
        self.grid.flush(s, &PAL);
    }

    fn name(&self) -> &'static str {
        "xwing"
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
    use crate::grid::with_test_aspect;
    use crate::saver;

    /// The panel this actually drives, and the dump size everything else is
    /// measured at. Both are 16:9 in VISUAL units once the aspect is applied,
    /// which is the whole point of the geometry being in those units.
    /// The 1080p dump, THE LIVE PANEL (1920x1080 at 180, which is 1920x600 —
    /// 3.2:1 — on the glass), a short panel at an extreme glass ratio, and a
    /// 4:3 one. Nothing here may assume a frame shape.
    const PANELS: [(usize, usize, usize); 4] = [
        (1920, 1080, 100),
        (1920, 1080, 180),
        (1280, 400, 180),
        (800, 600, 100),
    ];

    fn at(w: usize, h: usize, aspect: usize) -> (Panel, XWing) {
        let p = Panel::new(w, h, w);
        let mut x = with_test_aspect(aspect, || XWing::new(&p, 30));
        // Pinned, because `new` seeds off the clock when `XWING_SEED` is unset
        // and a test that rolls a different scene every run is one that fails
        // once a month. Set on the struct rather than through the env var:
        // cargo runs tests in parallel threads and the environment is
        // process-wide, which is the trap `satori` documents.
        x.rng = 0x5EED_4242;
        for i in 0..x.stars.len() {
            x.spawn_star(i, false);
        }
        x.enter_act();
        (p, x)
    }

    /// Run `x` from where it is to `phase` through act `act`.
    fn run_to(x: &mut XWing, p: &Panel, buf: &mut [u32], act: u8, phase: f32) {
        let mut n = 0;
        loop {
            saver::frame(x, buf, p);
            if x.act == act && (x.phase() - phase).abs() < 0.6 * x.dt / x.act_secs[act as usize] {
                return;
            }
            n += 1;
            assert!(n < 100_000, "never reached act {act} at {phase}");
        }
    }

    fn run_to_act(x: &mut XWing, p: &Panel, buf: &mut [u32], act: u8) {
        run_to(x, p, buf, act, 0.5);
    }

    fn lit(x: &XWing) -> usize {
        x.grid
            .cells()
            .iter()
            .filter(|c| c.glyph() != font::BLANK as usize)
            .count()
    }

    /// Cells of `x`'s drawn frame whose colour is in `range`, ABOVE the console
    /// strip — whose indicator lights are deliberately in the weapon and HUD
    /// colour families and would answer for them.
    fn above_console(x: &XWing, range: std::ops::RangeInclusive<u8>) -> usize {
        let end = (x.rows - (x.rows / 12).max(1)) * x.cols;
        x.grid.cells()[..end]
            .iter()
            .filter(|c| range.contains(&(c.colour() as u8)))
            .count()
    }

    /// Cells of `x`'s drawn frame whose colour is in `range`.
    fn count(x: &XWing, range: std::ops::RangeInclusive<u8>) -> usize {
        x.grid
            .cells()
            .iter()
            .filter(|c| range.contains(&(c.colour() as u8)))
            .count()
    }

    /// The contract every saver has with `simpledrm`: a pixel written but not
    /// reported shows a stale frame on the panel forever, and it reproduces on
    /// hardware and nowhere else. Run long enough to cross every act boundary,
    /// because an act change is where a saver is most likely to paint outside
    /// what it reported.
    #[test]
    fn damage_covers_every_changed_scanline() {
        for (w, h, a) in PANELS {
            let (p, mut x) = at(w, h, a);
            let mut buf = vec![0u32; p.buf_len()];
            let mut prev = buf.clone();

            let d = saver::frame(&mut x, &mut buf, &p);
            assert_eq!(d.rows(), p.h, "{w}x{h}@{a}: frame 0 must paint the panel");

            // Short acts so a full cycle fits, but through the real clock.
            x.act_secs = [0.7, 0.6, 0.8];
            for n in 1..900 {
                prev.copy_from_slice(&buf);
                let d = saver::frame(&mut x, &mut buf, &p);
                for y in 0..p.h {
                    let (a0, b0) = (&prev[y * p.w..][..p.w], &buf[y * p.w..][..p.w]);
                    if a0 == b0 {
                        continue;
                    }
                    assert!(
                        dump::row_reported(a0, b0, y, &d),
                        "{w}x{h}@{a} frame {n}: scanline {y} changed outside every rect"
                    );
                }
            }
        }
    }

    /// The frame loop is the product. A `Vec` that grows inside `render` is a
    /// malloc per frame against a 500m budget — and the ACT CHANGE is where one
    /// would appear, since that is the only thing here that rebuilds state, so
    /// the window has to be long enough to cross a whole cycle several times.
    #[test]
    fn render_never_allocates() {
        let (p, mut x) = at(1920, 1080, 100);
        let mut buf = vec![0u32; p.buf_len()];
        for _ in 0..8 {
            saver::frame(&mut x, &mut buf, &p);
        }
        // 30fps x 33s of acts = one full cycle; 4000 frames is four of them.
        let total: f32 = x.act_secs.iter().sum();
        assert!(
            4000.0 * x.dt > total * 3.0,
            "the window does not cross three cycles"
        );

        let mut seen = [false; 3];
        let (mut ties, mut booms) = (0, 0);
        let n = crate::testalloc::allocs_during(|| {
            for _ in 0..4000 {
                saver::frame(&mut x, &mut buf, &p);
                seen[x.act as usize] = true;
                ties += usize::from(x.ties.iter().any(|t| t.live));
                booms += usize::from(x.booms.iter().any(|b| b.live));
            }
        });
        assert_eq!(n, 0, "the render path allocated {n} times");
        assert_eq!(seen, [true; 3], "the window did not cross every act");
        // Non-vacuous for the pools added since: a window with no TIE and no
        // explosion in it has not tested the things most likely to allocate.
        assert!(ties > 0 && booms > 0, "no TIE ({ties}) or boom ({booms})");

        // Non-vacuous: the counter has to be able to see an allocation, on this
        // thread, or the assertion above is about nothing.
        let probe = crate::testalloc::allocs_during(|| {
            let v: Vec<u8> = Vec::with_capacity(64);
            std::hint::black_box(&v);
        });
        assert_eq!(probe, 1);
    }

    /// Three acts that look alike would make this the same saver three times —
    /// which is the failure the whole file is arranged against. Each act is
    /// identified by something only it does, measured off the drawn frame.
    #[test]
    fn the_three_acts_do_not_look_alike() {
        for (w, h, a) in PANELS {
            let (p, mut x) = at(w, h, a);
            let mut buf = vec![0u32; p.buf_len()];
            let (cols, rows) = (x.cols, x.rows);
            let case = format!("{w}x{h}@{a}");

            // Act 1: space. The corners furthest from the station are unlit,
            // and the station itself is a solid mass of plating.
            run_to_act(&mut x, &p, &mut buf, 0);
            let plating = count(&x, C_DARKEST..=C_BRIGHT);
            assert!(
                plating > cols * rows / 12,
                "{case}: act 1 has no station ({plating} plated cells)"
            );
            let space = count(&x, C_SPACE..=C_SPACE);
            assert!(
                space > cols * rows / 20,
                "{case}: act 1 has no space left in it ({space} cells)"
            );

            // Act 2: a horizon. The bottom of the frame is plating; the top,
            // outside the canopy struts, is not.
            run_to_act(&mut x, &p, &mut buf, 1);
            let band = |y: usize| {
                (0..cols)
                    .filter(|&cx| x.grid.cells()[y * cols + cx].colour() as u8 >= C_DARKEST)
                    .count()
            };
            let low = band(rows - rows / 4);
            let high = band(rows / 4);
            assert!(
                low > cols * 3 / 4,
                "{case}: act 2 has no ground under it ({low}/{cols})"
            );
            assert!(
                high < low,
                "{case}: act 2 has no horizon — sky {high}, ground {low}"
            );
            // And the ground recedes into the dark. Without that the plain is
            // a flat sheet of texture and the towers stand on nothing.
            let mean = |y: usize| {
                (0..cols)
                    .map(|cx| x.grid.cells()[y * cols + cx].colour())
                    .sum::<usize>() as f32
                    / cols as f32
            };
            // The nearest row that is not the console, against one just under
            // the horizon.
            let (near, far) = (
                mean(rows - (rows / 12).max(1) - 1),
                mean(rows / 2 + (rows / 40).max(1)),
            );
            assert!(
                near > far * 1.5,
                "{case}: no fog — the near plating is {near:.1} and the far {far:.1}"
            );

            // Act 3: walls on BOTH sides at the vertical middle, which is what
            // no other act has.
            run_to_act(&mut x, &p, &mut buf, 2);
            let mid = rows / 2;
            let l = x.grid.cells()[mid * cols].colour() as u8;
            let r = x.grid.cells()[mid * cols + cols - 1].colour() as u8;
            assert!(
                l >= C_DARKEST && r >= C_DARKEST,
                "{case}: act 3 is not walled in (left {l}, right {r})"
            );
            assert!(
                lit(&x) > cols * rows / 2,
                "{case}: act 3 drew almost nothing"
            );
        }
    }

    /// The trench is the act with tension in it, and the tension IS the walls
    /// closing. Measured as the sky slit between the two wall tops, in CELLS —
    /// derived from `rows`/`cols`, so a projection computed in cells rather
    /// than in square-pixel units fails this at 180 and not at 100.
    #[test]
    fn the_trench_closes_in() {
        for (w, h, a) in PANELS {
            let (p, mut x) = at(w, h, a);
            let mut buf = vec![0u32; p.buf_len()];
            // Sky above the walls, across the middle of a row near the top —
            // the canopy struts eat the corners. In CELLS, because a trench
            // whose walls were placed in cells rather than in square-pixel
            // units closes at a different rate at 180 than at 100.
            let slit = |x: &XWing| {
                // A row ABOVE the targeting computer's box, which hangs from
                // a fifth of the way down and would otherwise be what this
                // measured once it drops.
                let cy = x.rows / 9;
                (x.cols / 4..x.cols * 3 / 4)
                    .filter(|&cx| x.grid.cells()[cy * x.cols + cx].colour() == 0)
                    .count()
            };
            // The first sample is after the walls have finished RISING, so
            // this measures them closing in and not them growing; the second
            // is before the final flash.
            run_to(&mut x, &p, &mut buf, 2, 0.25);
            let open = slit(&x);
            run_to(&mut x, &p, &mut buf, 2, 0.95);
            let closed = slit(&x);
            assert!(
                open > 0,
                "{w}x{h}@{a}: the trench opened onto no sky at all"
            );
            assert!(
                // A 5% margin, not a strict `<`: the sample is quantised to
                // whole cells, and on a 4:3 panel half the closing happens
                // below this row. The mutation this is really against is a
                // trench that never moves at all, which lands dead equal.
                closed * 20 < open * 19,
                "{w}x{h}@{a}: the trench did not close — {open} cells of sky, then {closed}"
            );
        }
    }

    /// Act 2 is motion, and a frozen plain reads as a still. The bottom corner
    /// is plating and nothing else — no stars, no bolts, no canopy — so a
    /// change there is the surface going past and cannot be anything else.
    #[test]
    fn the_surface_actually_moves() {
        let (p, mut x) = at(1920, 1080, 100);
        let mut buf = vec![0u32; p.buf_len()];
        run_to_act(&mut x, &p, &mut buf, 1);
        let patch = |x: &XWing| {
            let cy = x.rows - (x.rows / 12).max(1) - 2;
            (2..x.cols / 6)
                .map(|cx| x.grid.cells()[cy * x.cols + cx].raw())
                .collect::<Vec<u32>>()
        };
        let before = patch(&x);
        for _ in 0..4 {
            saver::frame(&mut x, &mut buf, &p);
        }
        assert_ne!(before, patch(&x), "the plating is not going anywhere");
    }

    /// Only this saver has weapons, and now every act does: the run is under
    /// fire from the moment the station is in front of you. Green is the one
    /// colour family nothing else in the file can produce — the plating is
    /// grey, the explosions are white and red — so a green cell above the
    /// console is incoming fire and cannot be anything else.
    ///
    /// This was one test with the targeting computer until the trench stopped
    /// being the only armed act. The HUD half of its claim is still true and
    /// is now `the_targeting_computer_is_the_trench_only`; splitting rather
    /// than deleting is the point — a test that has gone half-wrong has a
    /// right half worth keeping.
    #[test]
    fn every_act_is_under_fire() {
        let (p, mut x) = at(1920, 1080, 100);
        let mut buf = vec![0u32; p.buf_len()];
        let green = |x: &XWing| above_console(x, C_GREEN_TRAIL..=C_GREEN_CORE);
        let red = |x: &XWing| above_console(x, C_RED_TRAIL..=C_RED_CORE);

        // No TIEs: they fire green too, and the claim here is that the act
        // itself is shooting — a station gun, a surface battery, a wall
        // turret. Left in, a saver that had dropped all three would still
        // pass on the fighters' fire.
        x.tie_secs = 0.0;
        for act in [0u8, 1, 2] {
            run_to_act(&mut x, &p, &mut buf, act);
            for t in &mut x.ties {
                *t = Tie::default();
            }
            // Over a stretch of the act: a bolt is in flight for under a
            // second, so a single frame proves nothing either way.
            let (mut g, mut r) = (0, 0);
            for _ in 0..(4.0 / x.dt) as usize {
                saver::frame(&mut x, &mut buf, &p);
                g += green(&x);
                r += red(&x);
            }
            assert!(g > 40, "act {act}: nothing shot at it ({g} green cells)");
            assert!(r > 40, "act {act}: it never shot back ({r} red cells)");
        }
    }

    /// The targeting computer, though, IS the trench: it drops when there is
    /// something to drop it for. This is the surviving half of the test above.
    #[test]
    fn the_targeting_computer_is_the_trench_only() {
        let (p, mut x) = at(1920, 1080, 100);
        let mut buf = vec![0u32; p.buf_len()];
        let hud = |x: &XWing| above_console(x, C_HUD..=C_HUD_LIT);

        for act in [0u8, 1] {
            run_to_act(&mut x, &p, &mut buf, act);
            for _ in 0..(3.0 / x.dt) as usize {
                saver::frame(&mut x, &mut buf, &p);
                assert_eq!(hud(&x), 0, "act {act} has a targeting computer in it");
            }
        }

        run_to_act(&mut x, &p, &mut buf, 2);
        let mut seen = 0;
        while x.act == 2 {
            saver::frame(&mut x, &mut buf, &p);
            seen += hud(&x);
        }
        assert!(seen > 200, "the targeting computer never dropped ({seen})");
    }

    /// Greebles are GENERATED, not tiled: the whole difference between reading
    /// as a Death Star and reading as a grey corridor. A tiling pattern repeats
    /// at its period, so check that two far-apart stretches of surface do not
    /// agree, and that the shade histogram is not two values.
    #[test]
    fn the_plating_is_generated_not_tiled() {
        let (_, x) = at(1920, 1080, 100);
        let sample = |a0: f32| {
            (0..512)
                .map(|k| {
                    x.greeble(
                        a0 + k as f32 * 7.0,
                        a0 * 0.5 + k as f32 * 3.0,
                        1.0,
                        1.0,
                        1.0,
                    )
                    .0
                })
                .collect::<Vec<u8>>()
        };
        let near = sample(0.0);
        let far = sample(1_000_000.0);
        assert_ne!(
            near, far,
            "the plating repeats: it is a tile, not a texture"
        );
        let same = near.iter().zip(&far).filter(|(a, b)| a == b).count();
        assert!(same < 400, "{same}/512 samples agree — barely generated");

        let mut seen = [0usize; 16];
        for s in &near {
            seen[*s as usize] += 1;
        }
        let shades = seen.iter().filter(|n| **n > 0).count();
        assert!(
            shades >= 5,
            "plating has only {shades} shades: it is a wash"
        );
        // And the seams are real: a cell small against a block punches dots
        // out, a cell larger than one does not.
        let punched = (0..200)
            .filter(|k| x.greeble(*k as f32 * 0.9, 0.0, 0.05, 0.05, 1.0).1 != FULL)
            .count();
        assert!(punched > 4, "no panel seams at all ({punched}/200 cells)");
        assert_eq!(
            x.greeble(13.0, 29.0, 1.0, 1.0, 1.0).1,
            FULL,
            "seams drawn at a scale finer than the cell: that is moire"
        );
    }

    /// The geometry is in square-pixel units, so the SAME shot has to arrive on
    /// two panels of the same visual shape whatever the aspect does to `rows`.
    /// A projection computed in cells puts the horizon at the same ROW in both
    /// and therefore in a different place on the glass — which is the bug this
    /// is here to fail on.
    #[test]
    fn the_shot_is_the_same_at_both_pixel_aspects() {
        // The same GLASS rectangle out of two different framebuffers: 1920x1080
        // square pixels, and 1920x1944 at 180, which the panel squashes by 1.8
        // back to 1920x1080. Deriving the second size from the first is the
        // half of this test that has to be got right — a pair of sizes derived
        // from an inverted convention makes both arms wrong together and the
        // comparison passes on a broken saver, which is exactly what happened
        // here. `the_cell_is_square_on_the_glass` guards the convention itself.
        let (p_a, mut a) = at(1920, 1080, 100);
        let (p_b, mut b) = at(1920, 1944, 180);
        assert!(
            (a.vh - b.vh).abs() < a.row_v * 4.0 && (a.vw - b.vw).abs() < 1.0,
            "the two panels are not the same glass: {}x{} vs {}x{}",
            a.vw,
            a.vh,
            b.vw,
            b.vh
        );
        assert_ne!(a.rows, b.rows, "same grid on both: nothing is being tested");
        let (mut ba, mut bb) = (vec![0u32; p_a.buf_len()], vec![0u32; p_b.buf_len()]);
        run_to(&mut a, &p_a, &mut ba, 2, 0.3);
        run_to(&mut b, &p_b, &mut bb, 2, 0.3);

        // How much of the frame is sky. It is the one number that catches a
        // projection derived from CELLS: the trench walls are placed and made
        // tall in square-pixel units, so the slit they leave is the same
        // fraction of the frame on both panels — but a saver that treated a
        // 1.8x-taller cell as square would project them 1.8x taller here and
        // close the sky right up. The horizon is NOT such a number: it is at
        // the middle of the frame by construction whatever the aspect does.
        let sky = |x: &XWing| {
            x.grid.cells().iter().filter(|c| c.colour() == 0).count() as f32
                / (x.cols * x.rows) as f32
        };
        let (sa, sb) = (sky(&a), sky(&b));
        assert!(sa > 0.05, "act 3 has no sky at all to compare ({sa:.3})");
        assert!(
            (sa - sb).abs() < 0.05,
            "the shot changed with the pixel aspect: {sa:.3} of the frame is sky at 100, {sb:.3} at 180"
        );

        // And the horizon still lands in the same place, which is the cheaper
        // half of the same claim.
        let horizon = |x: &XWing| {
            (0..x.rows)
                .find(|&cy| {
                    (x.cols / 3..x.cols * 2 / 3)
                        .all(|cx| x.grid.cells()[cy * x.cols + cx].colour() >= C_DARKEST as usize)
                })
                .map(|cy| cy as f32 / x.rows as f32)
                .expect("no horizon")
        };
        let (ha, hb) = (horizon(&a), horizon(&b));
        assert!((ha - hb).abs() < 0.06, "horizon {ha:.3} vs {hb:.3}");
    }

    /// The units, pinned directly. Everything else about the geometry is
    /// derived from `row_v`, and the assertion the rest of the suite makes is
    /// a COMPARISON — which two arms sharing one inverted operator both
    /// satisfy. This one cannot be satisfied by a consistent mistake: `Grid`
    /// makes the cell square on the glass, so its height in glass units must
    /// come back equal to its width, at every aspect.
    #[test]
    fn the_cell_is_square_on_the_glass() {
        let p = Panel::new(1920, 1080, 1920);
        for aspect in [25, 50, 100, 130, 180, 250, 400] {
            let x = with_test_aspect(aspect, || XWing::new(&p, 30));
            let off = (x.row_v - x.col_v).abs() / x.col_v;
            assert!(
                off < 0.10,
                "aspect {aspect}: a cell is {}x{} on the glass, {:.0}% off square",
                x.col_v,
                x.row_v,
                off * 100.0
            );
            // And the glass rectangle is the framebuffer with the squash taken
            // out of it: 1920x1080 at 180 is 1920x600, which is the 3.2:1 the
            // live panel actually is.
            let want = 1080.0 * 100.0 / aspect as f32;
            assert!(
                (x.vh - want).abs() < want * 0.10,
                "aspect {aspect}: glass height {} wanted ~{want}",
                x.vh
            );
        }
    }

    /// Act 2 opens nose-down and the nose comes up. Measured as sky: on the
    /// first frame of the act the plain fills the frame and there is none,
    /// and once the pitch is done there is a sky again. A hard cut into level
    /// flight — the edit this replaces — has sky on the first frame.
    #[test]
    fn the_nose_comes_up_into_act_two() {
        for (w, h, a) in PANELS {
            let (p, mut x) = at(w, h, a);
            let mut buf = vec![0u32; p.buf_len()];
            let sky = |x: &XWing| {
                // The canopy struts eat the top corners, so count the middle.
                (0..x.rows / 6)
                    .flat_map(|cy| (x.cols / 3..x.cols * 2 / 3).map(move |cx| (cy, cx)))
                    .filter(|(cy, cx)| x.grid.cells()[cy * x.cols + cx].colour() == 0)
                    .count()
            };
            run_to(&mut x, &p, &mut buf, 1, 0.0);
            let diving = sky(&x);
            run_to(&mut x, &p, &mut buf, 1, 0.5);
            let level = sky(&x);
            assert!(
                diving * 4 < level,
                "{w}x{h}@{a}: no pitch — {diving} cells of sky at the cut, {level} once level"
            );
        }
    }

    /// And act 3 opens on the plain act 2 ended on, with the walls growing out
    /// of it. Measured at the frame's edge, which is the near end of the wall:
    /// it is sky at the cut and solid once they are up.
    #[test]
    fn the_trench_walls_rise_into_act_three() {
        for (w, h, a) in PANELS {
            let (p, mut x) = at(w, h, a);
            let mut buf = vec![0u32; p.buf_len()];
            // Just inboard of the canopy struts, which own the top corners in
            // every act and would answer "is there a wall there" for it.
            let wall = |x: &XWing| {
                // Just above the horizon, where a wall of ANY height reaches
                // and a flat plain cannot: near the vanishing point the test
                // is the same on a 16:9 frame and on the panel's 3.2:1 one,
                // where a row picked near the top is off the end of a wall
                // that is only 900 units tall.
                let cy = x.rows * 2 / 5;
                let k = x.cols / 5;
                (k..k + 4)
                    .chain(x.cols - k - 4..x.cols - k)
                    .filter(|&cx| {
                        // Plating only. A star or a bolt at the sample point
                        // is not a wall, and on the short panel one of them
                        // lands there.
                        (1..=C_BRIGHT as usize).contains(&x.grid.cells()[cy * x.cols + cx].colour())
                    })
                    .count()
            };
            run_to(&mut x, &p, &mut buf, 2, 0.0);
            let cut = wall(&x);
            run_to(&mut x, &p, &mut buf, 2, 0.25);
            let up = wall(&x);
            assert_eq!(cut, 0, "{w}x{h}@{a}: the walls were already up at the cut");
            assert!(up >= 6, "{w}x{h}@{a}: the walls never rose ({up}/8 cells)");
        }
    }

    /// A TIE has to be a TIE: the silhouette is the whole reason it is in
    /// here. Counted inside the sprite's OWN box and nowhere else — the four
    /// shades it is drawn in are ordinary plating greys, so a count over the
    /// whole frame answers "how much Death Star is on screen" and passes with
    /// no TIE in it at all. Driven directly rather than waiting for the
    /// interval, so it is deterministic.
    #[test]
    fn a_tie_is_a_silhouette_and_a_red_bolt_kills_it() {
        let (p, mut x) = at(1920, 1080, 100);
        let mut buf = vec![0u32; p.buf_len()];
        saver::frame(&mut x, &mut buf, &p);
        for t in &mut x.ties {
            *t = Tie::default();
        }
        // Below the station, which at the start of act 1 is a small disc above
        // centre, so the box below holds nothing but sky and this TIE.
        let t = Tie {
            u: 0.0,
            v: 300.0,
            z: 1400.0,
            fire_in: 99.0,
            live: true,
            ..Tie::default()
        };
        x.ties[0] = t;
        saver::frame(&mut x, &mut buf, &p);

        let sc = x.focal / t.z;
        let (px, py) = (x.hx + t.u * sc, x.hz + t.v * sc);
        let s = TIE_R * sc;
        let cx0 = ((px - s * 2.2) / x.col_v) as usize;
        let cx1 = (((px + s * 2.2) / x.col_v) as usize).min(x.cols);
        let cy0 = ((py - s * 1.2) / x.row_v) as usize;
        let cy1 = (((py + s * 1.2) / x.row_v) as usize).min(x.rows);
        let across = |cy: usize| {
            (cx0..cx1)
                .filter(|&cx| x.grid.cells()[cy * x.cols + cx].colour() != 0)
                .count()
        };
        let has = |lev: u8| {
            (cy0..cy1).any(|cy| {
                (cx0..cx1).any(|cx| x.grid.cells()[cy * x.cols + cx].colour() == lev as usize)
            })
        };
        let body: usize = (cy0..cy1).map(across).sum();
        assert!(body > 40, "the TIE drew {body} cells: not a silhouette");
        // The ball and its window are what separate it from any other shape.
        assert!(has(C_TIE_GLASS), "no cockpit window");
        assert!(has(C_TIE_BALL), "no cockpit ball");

        // And the panels are HEXAGONS. Measured across ONE PANEL, not across
        // the whole sprite: the ball and the struts only exist at the middle,
        // so a sprite made of two RECTANGLES is also wider there and passes a
        // whole-silhouette comparison while being a barbell.
        let panel = |cy: usize| {
            let (a, b) = (px - s * 2.05, px - s * 0.95);
            ((a / x.col_v) as usize..(b / x.col_v) as usize)
                .filter(|&cx| cx < x.cols && x.grid.cells()[cy * x.cols + cx].colour() != 0)
                .count()
        };
        let lit: Vec<usize> = (cy0..cy1).filter(|&cy| across(cy) > 0).collect();
        let (top, mid) = (lit[0], lit[lit.len() / 2]);
        assert!(
            panel(mid) > panel(top) + 1,
            "the panel has no cut corners: {} cells across its top, {} across its middle",
            panel(top),
            panel(mid)
        );

        // A red bolt through it takes it, and leaves an explosion behind.
        x.spawn_bolt(t.u, t.v, t.z, 0.0, 0.0, 10.0, C_RED_TRAIL);
        saver::frame(&mut x, &mut buf, &p);
        assert!(!x.ties[0].live, "the bolt went straight through it");
        assert!(
            x.booms.iter().any(|b| b.live),
            "it died without an explosion"
        );
    }

    /// An explosion has to have a SHAPE — flash, then an expanding shell, then
    /// debris, then gone. A one-frame white blob reads as a dropped frame,
    /// which is the thing this test is here to keep it from being.
    #[test]
    fn an_explosion_flashes_expands_and_ends() {
        let (p, mut x) = at(1920, 1080, 100);
        let mut buf = vec![0u32; p.buf_len()];
        saver::frame(&mut x, &mut buf, &p);
        for b in &mut x.booms {
            *b = Boom::default();
        }
        x.spawn_boom(0.0, -60.0, 1400.0, 260.0);
        // Frozen: the flight would carry the blast out of frame mid-test.
        x.speed = 0.0;
        let lit_white = |x: &XWing| count(x, C_FLASH..=C_FLASH);

        saver::frame(&mut x, &mut buf, &p);
        let first = lit_white(&x);
        assert!(first > 0, "the explosion never flashed");

        // It grows: the white area is bigger a few frames in than on the first.
        let mut widest = first;
        for _ in 0..(BOOM_SECS * 0.3 / x.dt) as usize {
            saver::frame(&mut x, &mut buf, &p);
            widest = widest.max(lit_white(&x));
        }
        assert!(
            widest > first * 2,
            "the explosion did not expand ({first} -> {widest} cells)"
        );

        // And it ends, rather than sitting on the panel as a disc forever.
        for _ in 0..(BOOM_SECS * 1.2 / x.dt) as usize {
            saver::frame(&mut x, &mut buf, &p);
        }
        assert!(!x.booms.iter().any(|b| b.live), "the explosion never ended");
    }

    /// Act 1 has to end with the station covering the frame — that is the
    /// whole reason the cut into act 2 lands on matching pixels. Sized off the
    /// frame's HEIGHT it does on 16:9 and does not on the panel's 3.2:1 glass,
    /// which no 1080p dump would ever show.
    #[test]
    fn the_station_fills_the_frame_before_the_cut() {
        for (w, h, a) in PANELS {
            let (p, mut x) = at(w, h, a);
            let mut buf = vec![0u32; p.buf_len()];
            run_to(&mut x, &p, &mut buf, 0, 0.99);
            let space = count(&x, C_SPACE..=C_SPACE);
            assert!(
                space * 50 < x.cols * x.rows,
                "{w}x{h}@{a}: {space} of {} cells are still space at the cut",
                x.cols * x.rows
            );
        }
    }

    /// A seed makes a run reproducible, which is the only way a rendering
    /// change can be shown to be a no-op. Four savers shipped without one.
    #[test]
    fn the_seed_knob_pins_the_run() {
        let p = Panel::new(640, 480, 640);
        let build = |seed: u32| {
            let mut x = XWing::new(&p, 30);
            x.rng = seed;
            for i in 0..x.stars.len() {
                x.spawn_star(i, false);
            }
            // `new` rolls the first turret interval from the clock-seeded RNG,
            // so re-enter the act to take it from the pinned one instead.
            x.enter_act();
            let mut buf = vec![0u32; p.buf_len()];
            for _ in 0..40 {
                saver::frame(&mut x, &mut buf, &p);
            }
            x.grid.cells().to_vec()
        };
        assert_eq!(build(12345), build(12345), "the same seed drew two scenes");
        assert_ne!(build(12345), build(999), "the seed changes nothing");
    }
}
