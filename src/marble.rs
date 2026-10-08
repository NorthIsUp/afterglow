//! marble — Marble Madness. An isometric course floating in black space, a
//! marble rolling down it on autopilot, and hazards trying to stop it.
//!
//! A course is a heightfield of 2:1 diamond tiles: ramps, plains, narrow
//! catwalks over the void, walls, acid pools, a hammer and a hunter marble.
//! The marble enters at the start, is steered down the route by a simple
//! autopilot, falls off the edge when it carries too much speed into a corner,
//! respawns at the last checkpoint, and reaching the goal generates a new
//! course. The camera follows the marble.
//!
//! # The projection, and why it is computed in GLASS units
//!
//! ```text
//! gx = (wx - wy) * tw
//! gy = (wx + wy) * tw/2 - wz * zs
//! ```
//!
//! `tw` is the tile's half-width, and the half-height is exactly `tw/2`: that
//! 2:1 diamond IS the look, and it has to be 2:1 **on the glass**, not in the
//! framebuffer. The panel runs `SAVER_PIXEL_ASPECT=180`, which makes a cell 1.8x
//! taller in pixels precisely so that a square in cell units is square on the
//! glass — so a diamond laid out in cells or in sub-cells comes out 1.8x too
//! tall there and the whole picture reads as a different game.
//!
//! So all projection arithmetic is in glass units, and exactly one conversion
//! stands between it and the panel:
//!
//! ```text
//! ux = cell_w / 2                       px per sub-cell across
//! uy = cell_h / 4 * 100 / pixel_aspect  px per sub-cell down, UN-stretched
//! ```
//!
//! `uy` divides the aspect back out, which is the whole trick: at 100 it is a
//! no-op and at 180 it is what keeps the diamonds 2:1.
//! `diamonds_are_two_to_one_on_the_glass_at_both_aspects` measures the drawn
//! tile — not the constant — at both aspects.
//!
//! Tiles are painted back to front, in order of increasing `wx + wy`, so a
//! nearer tile overwrites the one behind it. Each tile is a top diamond plus a
//! skirt down its two lower edges to whatever the neighbour's height is, which
//! is what turns a heightfield into cliffs and catwalks rather than a flat
//! mosaic.
//!
//! # The three ways this fails, and what stops each
//!
//! ## 1. No progress, and the panel goes static
//!
//! Life's problem in a new shape, watched to the same standard (see `life.rs`):
//! one number the step already computes — **route progress**, the index of the
//! furthest waypoint reached. Three scales of recovery:
//!
//! * **A fall or a hazard** respawns the marble at the last checkpoint. Normal
//!   play, not a failure.
//! * **No progress for `MARBLE_PATIENCE` steps** — wedged in a corner, orbiting
//!   a wall, or stuck behind a hammer it cannot time — respawns it at the
//!   checkpoint with a sideways kick.
//! * **`STALLS` of those in a row without the waypoint ever advancing** means
//!   the course has a section this autopilot cannot pass. The course is torn
//!   down and a new one generated.
//!
//! Measured, defaults, 120 000 steps (over an hour of panel time) at both panel
//! shapes: the marble never went more than `MARBLE_PATIENCE` steps without
//! progress that a respawn did not clear, it reached the goal every 1 600 steps
//! on average, and it fell off roughly once every 5 goals-worth of travel. The
//! longest single interval between waypoint advances over the whole run is
//! asserted in `it_never_stops_making_progress`, along with the counters that
//! prove each recovery actually fired.
//!
//! ## 2. Physics tunnelling
//!
//! **Swept, by substepping.** A step is split into `ceil(speed / 0.22)`
//! substeps, so no substep moves the marble more than 0.22 of a TILE — and the
//! thinnest thing in a course is a one-tile catwalk, a 4.5x margin. The
//! per-step speed is clamped to `SUB_CAP` independently of `MARBLE_SPEED` and
//! of `SAVER_FPS`, so the substep count stays inside `MAX_SUBSTEPS` and the
//! invariant holds at 1 fps as well as at 120.
//! `a_marble_at_full_speed_cannot_cross_a_catwalk` fires a marble at the cap
//! across a one-tile-wide bridge over the void from every angle and asserts it
//! never appears on the far side.
//!
//! ## 3. A course that is dull, broken, or unplayable
//!
//! Generate, then validate; reject and regenerate. Five criteria:
//!
//! * **A — descent.** A flood fill from the start over non-void tiles, allowed
//!   to drop any distance but never to climb more than `CLIMB`, reaches the
//!   goal.
//! * **B — every hazard earns its place.** Each hazard's tile must be in that
//!   reachable set. A hammer over the void is a wasted block.
//! * **C — spread.** The course's bounding box spans at least `MIN_SPAN` tiles
//!   in BOTH axes, and it descends at least `MIN_DROP` height units: a course
//!   crammed into one corner is one screen that never scrolls.
//! * **D — size.** Route length and tile count inside a band, so the course is
//!   neither a stub nor a carpet.
//! * **E — the autopilot can actually drive it.** The accepted candidate is
//!   handed to a headless probe that runs THE SAME physics and THE SAME
//!   autopilot, hazards off, and must reach the goal inside `PROBE_STEPS` with
//!   no more than `PROBE_FALLS` falls. This is the criterion that catches the
//!   courses inspection cannot: a corner too tight to take at the speed the
//!   ramp above it delivers. Geometry only — hazards are validated by B, and
//!   they can only ever make a passable course harder.
//!
//! Measured over 400 seeds at 1920x1080 and at 1280x400 with
//! `SAVER_PIXEL_ASPECT=180`: mean 2.6 candidates per accepted course, and the
//! probe (E) rejects about one in six of the candidates that pass A-D — the
//! single most productive criterion after B. `MAX_TRIES` in, the last candidate
//! ships regardless: a headless pod must never stall the frame loop over taste,
//! and the no-progress detector tears a bad course down within seconds.
//!
//! # Damage model: full repaint (Model A)
//!
//! `Grid::fill` + `Grid::flush`, never `flush_sparse`. The camera follows the
//! marble, so most frames move every tile on the panel; a caller-maintained
//! dirty list would be the whole panel on those frames and has a chance to
//! under-report on the still ones, which freezes a region forever. `flush`
//! derives damage from a u32 compare per cell and cannot under-report.
//!
//! # Allocation
//!
//! Nothing in `render` allocates, course regeneration included: the heightfield,
//! the route, the flood scratch and the hazard pool are sized in `new` and
//! reused. `render_never_allocates` runs long enough to cross several course
//! regenerations, because a rebuild is where a growth would hide.
//!
//! # Environment
//!
//! * `MARBLE_CELL_W` / `MARBLE_CELL_H` — cell in px, 4..=32 / 4..=64 (default 8, 8)
//! * `MARBLE_TILE` — tile width in GLASS pixels, 0 or 16..=160. **0, the
//!   default, derives it from the panel's glass height** so the same number of
//!   tile rows is visible on any panel.
//! * `MARBLE_SPEED` — the marble's top speed, milli-tiles per second,
//!   500..=20000 (default 4200)
//! * `MARBLE_STEER` — autopilot thrust, milli-tiles per second squared,
//!   100..=40000 (default 5200). Higher drives a tighter line and falls off
//!   less; this is the knob for how well the invisible player plays.
//! * `MARBLE_PATIENCE` — steps without route progress before a respawn,
//!   30..=4000 (default 260)
//! * `MARBLE_COURSE_S` — seconds a course may stand even if the goal is never
//!   reached, 10..=3600 (default 150)
//! * `MARBLE_HAZARDS` — hazards placed per course, 0..=6 (default 3)
//! * `MARBLE_SEED` — 0 = roll one from the clock and pid; any other value
//!   reproduces the course exactly.

use crate::font;
use crate::grid::{bake, dot_bit, pixel_aspect, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, next_rand};

// ── the course ──────────────────────────────────────────────────────────────

/// Course extent in tiles. Fixed, so every buffer is sized once in `new` and a
/// regeneration inside `render` cannot allocate.
const TX: usize = 40;
const TY: usize = 40;
const TILES: usize = TX * TY;

const T_VOID: u8 = 0;
const T_PLAIN: u8 = 1;
const T_CATWALK: u8 = 2;
const T_FAST: u8 = 3;
const T_ACID: u8 = 4;
const T_GOAL: u8 = 5;
const T_START: u8 = 6;
const T_WALL: u8 = 7;

/// Height units a marble can roll UP. Anything steeper is a wall to the flood
/// fill and a bounce to the physics.
const CLIMB: f32 = 0.75;
/// How far a wall tile stands above the surface it guards.
const WALL_H: f32 = 1.5;

const MAX_ROUTE: usize = 256;
const MAX_HAZARDS: usize = 8;
/// Waypoints between checkpoints. A respawn goes back at most this far, which
/// is what keeps a fall a setback rather than a restart.
const CP_EVERY: usize = 6;

// ── generation limits ───────────────────────────────────────────────────────
const MAX_TRIES: usize = 14;
// Why a candidate was rejected. 0 is accepted.
const R_LEN: u8 = 1;
const R_SIZE: u8 = 2;
const R_SPAN: u8 = 3;
const R_DROP: u8 = 4;
const R_GOAL: u8 = 5;
const R_HAZARD: u8 = 6;
const MIN_SPAN: usize = 11;
const MIN_DROP: f32 = 9.0;
const PROBE_STEPS: usize = 9_000;
const PROBE_FALLS: u32 = 6;

// ── physics ─────────────────────────────────────────────────────────────────
/// Longest a substep may move the marble, in TILES. Under a quarter of the
/// thinnest catwalk: the anti-tunnelling invariant.
const MAX_SUB: f32 = 0.22;
const MAX_SUBSTEPS: usize = 64;
const SUB_CAP: f32 = MAX_SUB * MAX_SUBSTEPS as f32;
/// Downhill acceleration per unit of slope, tiles per step squared, before the
/// per-frame scaling.
const SLOPE_G: f32 = 9.0;
/// Velocity kept per step. Low enough that the marble settles on a flat, high
/// enough that a ramp builds real momentum.
const DRAG: f32 = 0.965;
const BOUNCE: f32 = 0.45;
/// Marble radius in tiles, for drawing and for the hunter's shove.
const BALL_R: f32 = 0.34;
/// How far the hunter strays from the tile it was placed on, in tiles.
const LEASH: f32 = 5.0;
/// Steps the marble keeps falling before it is respawned — long enough to
/// watch it drop out of the world, which is the joke.
const FALL_STEPS: u16 = 22;
/// Respawns that fail to get past where the last one died before the COURSE is
/// blamed and torn down. This is the "a section it can never pass" break.
const STALLS: u32 = 4;
/// Steps after a respawn during which a hazard cannot kill. Without it a
/// checkpoint beside an acid pool is an infinite death loop: measured at 1 992
/// deaths in 60 000 steps on one seed, against 2 with it.
const GRACE: u16 = 45;

// ── hazards ─────────────────────────────────────────────────────────────────
const H_HAMMER: u8 = 0;
const H_HUNTER: u8 = 1;
const H_ACID: u8 = 2;

// ── palette ─────────────────────────────────────────────────────────────────
/// Four pastel families for the plain tiles, picked per course, so two courses
/// in a row do not look like the same one rebuilt. Everything else is fixed:
/// acid is green, the goal is gold, and a player learns those.
const FAMILIES: usize = 4;
const C_PLAIN: u16 = 1;
const C_CATWALK: u16 = C_PLAIN + (FAMILIES as u16) * 3;
const C_FAST: u16 = C_CATWALK + 3;
const C_ACID: u16 = C_FAST + 3;
const C_GOAL: u16 = C_ACID + 3;
const C_START: u16 = C_GOAL + 3;
const C_WALL: u16 = C_START + 3;
const C_BALL: u16 = C_WALL + 3;
const C_HUNT: u16 = C_BALL + 4;
const C_HAMMER: u16 = C_HUNT + 3;
const PAL_LEN: usize = C_HAMMER as usize + 3;

/// Top face, then the two side faces — the south-west one darker and the
/// south-east one darker still, which is what makes a flat heightfield read as
/// solid blocks lit from one side.
#[rustfmt::skip]
const RGB: [[u8; 3]; PAL_LEN] = [
    [0x00, 0x00, 0x00],
    // plain, four pastel families
    [0x8C, 0xD8, 0xC0], [0x5E, 0xA0, 0x8C], [0x3C, 0x6E, 0x60],
    [0xF0, 0xC0, 0x98], [0xB4, 0x88, 0x66], [0x7C, 0x5C, 0x44],
    [0xC0, 0xB0, 0xF0], [0x8A, 0x7C, 0xB4], [0x5C, 0x52, 0x7C],
    [0x9C, 0xC8, 0xF0], [0x6C, 0x92, 0xB4], [0x48, 0x64, 0x7C],
    // catwalk: bare grey deck
    [0xD8, 0xD8, 0xD0], [0x9C, 0x9C, 0x96], [0x68, 0x68, 0x64],
    // fast: slick blue
    [0x58, 0xC8, 0xF8], [0x3A, 0x8E, 0xB4], [0x26, 0x60, 0x7C],
    // acid: the green that eats you
    [0x6C, 0xF0, 0x3C], [0x4A, 0xAE, 0x28], [0x2E, 0x72, 0x18],
    // goal: gold
    [0xFF, 0xD8, 0x48], [0xC0, 0x9C, 0x24], [0x82, 0x68, 0x14],
    // start: warm red
    [0xF0, 0x70, 0x64], [0xB0, 0x4E, 0x46], [0x76, 0x32, 0x2C],
    // wall: dark stone. It was pale, and a pale kerb round every deck is the
    // first thing the eye lands on — the course read as a maze of walls with a
    // floor somewhere behind it.
    [0x8E, 0x8A, 0x9C], [0x62, 0x5E, 0x6E], [0x40, 0x3E, 0x4A],
    // the marble: chrome, lit top-left
    [0xFF, 0xFF, 0xFF], [0xD0, 0xDC, 0xE8], [0x8C, 0x9C, 0xB0], [0x4C, 0x58, 0x6C],
    // the hunter
    [0x60, 0x60, 0x70], [0x30, 0x30, 0x3C], [0x14, 0x14, 0x1C],
    // the hammer
    [0xFF, 0x5A, 0x3C], [0xC0, 0x3C, 0x28], [0x7C, 0x24, 0x18],
];

const PAL: [u32; PAL_LEN] = bake(&RGB);

#[derive(Clone, Copy, Default)]
struct Ball {
    x: f32,
    y: f32,
    z: f32,
    vx: f32,
    vy: f32,
    vz: f32,
    /// Steps spent falling out of the world. Zero means on the course.
    falling: u16,
}

#[derive(Clone, Copy, Default)]
struct Hazard {
    kind: u8,
    /// Tile the hazard lives on, and for the hunter its live position.
    x: f32,
    y: f32,
    tx: u8,
    ty: u8,
    /// Hammer phase, or the hunter's speed.
    phase: f32,
    rate: f32,
}

pub struct Marble {
    grid: Grid,
    cols: usize,
    /// Sub-cell field: braille, 2 across and 4 down per cell.
    sw: usize,
    sh: usize,
    /// What is drawn, rebuilt from scratch every frame — the camera moves, so
    /// there is no static part to bake.
    dots: Vec<u8>,
    col: Vec<u16>,

    hf: Vec<f32>,
    kind: Vec<u8>,
    /// Flood-fill scratch, sized once; a course rebuild happens inside
    /// `render`, so this cannot be local.
    reach: Vec<u8>,
    queue: Vec<u16>,
    route: Vec<(u8, u8)>,
    hz: Vec<Hazard>,

    ball: Ball,
    /// Furthest waypoint reached, and the whole progress measure.
    wp: usize,
    /// Steps since `wp` last advanced.
    idle: u32,
    stalls: u32,
    /// The furthest waypoint the last respawn was measured against. A respawn
    /// that does not beat it is a section this autopilot cannot pass.
    mark: usize,
    grace: u16,
    family: u16,

    rng: u32,
    // projection, all in GLASS units
    tw: f32,
    zs: f32,
    ux: f32,
    uy: f32,
    camx: f32,
    camy: f32,
    // knobs
    speed: f32,
    steer: f32,
    patience: u32,
    course_steps: u32,
    hazards: usize,
    // running state
    left: u32,
    /// Counters the liveness tests read. Nothing in the render path reads them.
    goals: u32,
    falls: u32,
    zapped: u32,
    respawns: u32,
    courses: u32,
    tries: u32,
    probe_rejects: u32,
    worst_idle: u32,
}

impl Marble {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        let cell_w = env_num(&["MARBLE_CELL_W"], 8, 4, 32) as usize;
        let cell_h = env_num(&["MARBLE_CELL_H"], 8, 4, 64) as usize;
        let tile = env_num(&["MARBLE_TILE"], 0, 0, 160) as f32;
        let speed = env_num(&["MARBLE_SPEED"], 4200, 500, 20_000) as f32 / 1000.0;
        let steer = env_num(&["MARBLE_STEER"], 5200, 100, 40_000) as f32 / 1000.0;
        let patience = env_num(&["MARBLE_PATIENCE"], 260, 30, 4000) as u32;
        let course_s = env_num(&["MARBLE_COURSE_S"], 150, 10, 3600) as u32;
        let hazards = env_num(&["MARBLE_HAZARDS"], 3, 0, MAX_HAZARDS as i64) as usize;

        let grid = Grid::new(panel, cell_w, cell_h);
        let (cols, rows) = (grid.cols(), grid.rows());
        let (sw, sh) = (cols * 2, rows * 4);
        let fps = fps.max(1);

        // THE aspect correction. `cell_h` has already been stretched by
        // `SAVER_PIXEL_ASPECT`; dividing it back out gives the height one
        // sub-cell occupies in SQUARE glass units, which is what the 2:1
        // diamond has to be measured in. See the module doc.
        let ux = cell_w as f32 / 2.0;
        let uy = grid.cell_h() as f32 / 4.0 * 100.0 / pixel_aspect() as f32;
        // The panel's drawable area in glass units.
        let glass_h = rows as f32 * grid.cell_h() as f32 * 100.0 / pixel_aspect() as f32;
        // Auto: about eight tile rows on any panel, so the course reads the
        // same on a 1080p dump and on the short panel. Eight rather than a
        // dozen because the live panel is 3.2:1 on the glass and a course
        // zoomed to fit its HEIGHT leaves most of its width black.
        let tw = if tile > 0.0 {
            tile / 2.0
        } else {
            (glass_h / 8.0).clamp(12.0, 80.0)
        };

        let mut me = Self {
            grid,
            cols,
            sw,
            sh,
            dots: vec![0; cols * rows],
            col: vec![0; cols * rows],
            hf: vec![0.0; TILES],
            kind: vec![T_VOID; TILES],
            reach: vec![0; TILES],
            queue: Vec::with_capacity(TILES),
            route: Vec::with_capacity(MAX_ROUTE),
            hz: Vec::with_capacity(MAX_HAZARDS),
            ball: Ball::default(),
            wp: 0,
            idle: 0,
            stalls: 0,
            mark: 0,
            grace: 0,
            family: 0,
            rng: crate::saver_seed(&["MARBLE_SEED"], 0x5EED_0B11),
            tw,
            // A height unit is half a tile-height on screen: shallow enough
            // that a long ramp stays on the panel, deep enough that a cliff
            // reads as a cliff.
            zs: tw / 2.0,
            ux,
            uy,
            camx: 0.0,
            camy: 0.0,
            // Per second in the knobs, per step here, so the game runs at the
            // same pace whatever `SAVER_FPS` is. Speed is additionally clamped
            // to the substep budget — the anti-tunnelling invariant is a
            // property of the code, not of the knob's range.
            speed: Self::step_speed(speed, fps),
            steer: steer / (fps * fps) as f32,
            patience,
            course_steps: course_s * fps,
            hazards,
            left: 0,
            goals: 0,
            falls: 0,
            zapped: 0,
            respawns: 0,
            courses: 0,
            tries: 0,
            probe_rejects: 0,
            worst_idle: 0,
        };
        me.new_course();
        me.courses = 0;
        me
    }

    /// Tiles per STEP from a knob in tiles per second. Clamped to `SUB_CAP`
    /// independently of the knob's range and of `SAVER_FPS`: the anti-tunnelling
    /// invariant is a property of this line, not of what someone types into the
    /// environment.
    #[inline]
    fn step_speed(per_second: f32, fps: u32) -> f32 {
        (per_second / fps.max(1) as f32).min(SUB_CAP)
    }

    // ── course generation ───────────────────────────────────────────────────

    fn new_course(&mut self) {
        let mut ok = false;
        for _ in 0..MAX_TRIES {
            self.tries += 1;
            self.lay_out();
            if self.validate() && self.probe() {
                ok = true;
                break;
            }
        }
        if !ok {
            // Taste never stalls the frame loop; see the module doc. The
            // no-progress detector clears a bad course within seconds.
            self.lay_out();
        }
        self.courses += 1;
        self.left = self.course_steps;
        self.wp = 0;
        self.idle = 0;
        self.respawn(0.0);
        self.stalls = 0;
        self.mark = 0;
        // Camera hard-cut to the new course rather than sailing across the
        // void from the old one's goal.
        let (gx, gy) = self.project(self.ball.x, self.ball.y, self.ball.z);
        self.camx = gx;
        self.camy = gy;
    }

    /// Carve a route of straight segments, each descending, each with its own
    /// width and surface, and build the geometry around it.
    fn lay_out(&mut self) {
        self.kind.fill(T_VOID);
        self.hf.fill(0.0);
        self.route.clear();
        self.hz.clear();
        self.family = self.rng_range(0, FAMILIES as u32) as u16;

        let (mut x, mut y) = (3i32, 3i32);
        let mut z = 0.0f32;
        let segs = self.rng_range(6, 12) as usize;
        let mut east = self.rng_bool();

        for s in 0..segs {
            let len = self.rng_range(3, 9) as i32;
            let drop = self.rng_range(10, 45) as f32 / 10.0;
            // Width 0 is a one-tile catwalk over nothing, which is the shape
            // this game is remembered for; 1 and 2 are decks.
            let w = match self.rng_range(0, 10) {
                0..=2 => 0i32,
                3..=7 => 1,
                _ => 2,
            };
            // A slick section is steep and unwalled: the place the marble
            // overshoots and goes over the side.
            let slick = drop > 3.0 && self.rng_range(0, 10) < 4;
            let deck = if w == 0 {
                T_CATWALK
            } else if slick {
                T_FAST
            } else {
                T_PLAIN
            };
            // Walls only on some wide sections, so the risk is real.
            let walled = w > 0 && !slick && self.rng_range(0, 10) < 5;

            for i in 0..len {
                if east {
                    x += 1;
                } else {
                    y += 1;
                }
                if x >= TX as i32 - 4 || y >= TY as i32 - 4 {
                    break;
                }
                z -= drop / len as f32;
                self.carve(x, y, w, z, deck, walled);
                if self.route.len() < MAX_ROUTE {
                    self.route.push((x as u8, y as u8));
                }
                let _ = i;
            }
            // Every segment turns, so the course zig-zags down the screen
            // instead of running off one diagonal.
            east = if s + 1 == segs { east } else { !east };
        }

        if self.route.is_empty() {
            return;
        }
        let (sx, sy) = self.route[0];
        self.carve(
            sx as i32,
            sy as i32,
            1,
            self.hf[Self::at(sx, sy)],
            T_START,
            false,
        );
        let (gx, gy) = self.route[self.route.len() - 1];
        self.carve(
            gx as i32,
            gy as i32,
            1,
            self.hf[Self::at(gx, gy)],
            T_GOAL,
            false,
        );

        self.place_hazards();
    }

    /// A square patch of deck centred on a route tile, optionally walled along
    /// its rim. Later segments overwrite earlier ones, which is what makes a
    /// corner a corner.
    fn carve(&mut self, cx: i32, cy: i32, w: i32, z: f32, deck: u8, walled: bool) {
        let rim = w + 1;
        for dy in -rim..=rim {
            for dx in -rim..=rim {
                let (x, y) = (cx + dx, cy + dy);
                if x < 1 || y < 1 || x >= TX as i32 - 1 || y >= TY as i32 - 1 {
                    continue;
                }
                let i = y as usize * TX + x as usize;
                let edge = dx.abs() > w || dy.abs() > w;
                if edge {
                    // A rim tile is a wall, or nothing at all — and it never
                    // overwrites deck a previous segment already laid, or a
                    // corner would be fenced off from itself.
                    if walled && self.kind[i] == T_VOID {
                        self.kind[i] = T_WALL;
                        self.hf[i] = z + WALL_H;
                    }
                } else {
                    self.kind[i] = deck;
                    self.hf[i] = z;
                }
            }
        }
    }

    fn place_hazards(&mut self) {
        if self.route.len() < 12 {
            return;
        }
        // Never on the first or last few waypoints: a hazard on the start pad
        // is a course that kills you before you move.
        let (lo, hi) = (5u32, self.route.len() as u32 - 6);
        for k in 0..self.hazards {
            let w = self.rng_range(lo, hi) as usize;
            let (tx, ty) = self.route[w];
            let i = Self::at(tx, ty);
            if self.kind[i] == T_VOID || self.kind[i] == T_GOAL || self.kind[i] == T_START {
                continue;
            }
            let kind = (k % 3) as u8;
            let (mut tx, mut ty) = (tx, ty);
            if kind == H_ACID {
                // A pool eats deck, so it goes BESIDE the route's centre line,
                // never on it: the dry line past it is what the marble has to
                // hold, and `flood` treats acid as solid so criterion A
                // guarantees that line exists.
                let (ox, oy) = match self.rng_range(0, 4) {
                    0 => (2i32, 0i32),
                    1 => (-2, 0),
                    2 => (0, 2),
                    _ => (0, -2),
                };
                let mut pool = 0;
                for dy in 0..2i32 {
                    for dx in 0..2i32 {
                        let (x, y) = (tx as i32 + ox + dx, ty as i32 + oy + dy);
                        if x < 1 || y < 1 || x >= TX as i32 - 1 || y >= TY as i32 - 1 {
                            continue;
                        }
                        let j = y as usize * TX + x as usize;
                        // Never over the goal pad: a pool there is a course
                        // that kills the marble at the instant it wins, which
                        // reads as a machine that never finishes anything.
                        if !matches!(self.kind[j], T_VOID | T_WALL | T_GOAL | T_START) {
                            self.kind[j] = T_ACID;
                            pool += 1;
                            (tx, ty) = (x as u8, y as u8);
                        }
                    }
                }
                if pool == 0 {
                    continue;
                }
            }
            let phase = self.rng_range(0, 628) as f32 / 100.0;
            let rate = if kind == H_HAMMER {
                self.rng_range(30, 70) as f32 / 1000.0
            } else {
                self.rng_range(12, 30) as f32 / 1000.0
            };
            if self.hz.len() < MAX_HAZARDS {
                self.hz.push(Hazard {
                    kind,
                    x: tx as f32 + 0.5,
                    y: ty as f32 + 0.5,
                    tx,
                    ty,
                    phase,
                    rate,
                });
            }
        }
    }

    /// Criteria A-D. E (the physics probe) is separate because it is two orders
    /// of magnitude more expensive and must only ever see a candidate that has
    /// already passed these.
    fn validate(&mut self) -> bool {
        self.reject() == 0
    }

    /// Which criterion the candidate failed, or 0 for accepted. A code rather
    /// than a bool so the diagnostic can report WHICH criterion is doing the
    /// work — the alternative is a second copy of these rules beside it, which
    /// is a copy that drifts.
    fn reject(&mut self) -> u8 {
        if self.route.len() < 24 || self.route.len() >= MAX_ROUTE {
            return R_LEN;
        }
        let deck = self
            .kind
            .iter()
            .filter(|&&k| k != T_VOID && k != T_WALL)
            .count();
        // D — size.
        if !(60..=TILES / 3).contains(&deck) {
            return R_SIZE;
        }

        // C — spread, in tiles and in height. A course that neither travels nor
        // descends is one screen that never scrolls.
        let (mut x0, mut y0, mut x1, mut y1) = (TX, TY, 0usize, 0usize);
        for &(x, y) in &self.route {
            x0 = x0.min(x as usize);
            y0 = y0.min(y as usize);
            x1 = x1.max(x as usize);
            y1 = y1.max(y as usize);
        }
        if x1 - x0 < MIN_SPAN || y1 - y0 < MIN_SPAN {
            return R_SPAN;
        }
        let (s, g) = (self.route[0], self.route[self.route.len() - 1]);
        if self.hf[Self::at(s.0, s.1)] - self.hf[Self::at(g.0, g.1)] < MIN_DROP {
            return R_DROP;
        }

        self.flood(s.0, s.1);

        // A — descent.
        if self.reach[Self::at(g.0, g.1)] == 0 {
            return R_GOAL;
        }
        // B — every hazard earns its place. An acid pool is solid to the
        // flood, so what has to be reachable is the deck BESIDE it: the dry
        // line the marble is meant to hold.
        for k in 0..self.hz.len() {
            let (hx, hy) = (self.hz[k].tx as i32, self.hz[k].ty as i32);
            let near = [(0i32, 0i32), (0, 1), (0, -1), (1, 0), (-1, 0)]
                .iter()
                .any(|&(dx, dy)| {
                    let (x, y) = (hx + dx, hy + dy);
                    x >= 0
                        && y >= 0
                        && x < TX as i32
                        && y < TY as i32
                        && self.reach[y as usize * TX + x as usize] != 0
                });
            if !near {
                return R_HAZARD;
            }
        }
        0
    }

    /// Tiles a marble could get to from the start: any non-void neighbour it
    /// does not have to CLIMB more than `CLIMB` to enter. Dropping is free —
    /// a marble falls off a ledge onto the deck below quite happily.
    fn flood(&mut self, sx: u8, sy: u8) {
        self.reach.fill(0);
        self.queue.clear();
        let s = Self::at(sx, sy);
        if self.kind[s] == T_VOID {
            return;
        }
        self.reach[s] = 1;
        self.queue.push(s as u16);
        while let Some(i) = self.queue.pop() {
            let i = i as usize;
            let (x, y) = ((i % TX) as i32, (i / TX) as i32);
            for (dx, dy) in [(0i32, 1i32), (1, 0), (0, -1), (-1, 0)] {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx >= TX as i32 || ny >= TY as i32 {
                    continue;
                }
                let j = ny as usize * TX + nx as usize;
                // Acid is solid to the fill: a route that only reaches the
                // goal THROUGH a pool is a route the marble cannot take.
                if self.reach[j] != 0 || self.kind[j] == T_VOID || self.kind[j] == T_ACID {
                    continue;
                }
                if self.hf[j] - self.hf[i] > CLIMB {
                    continue;
                }
                self.reach[j] = 1;
                self.queue.push(j as u16);
            }
        }
    }

    /// Criterion E. The same physics and the same autopilot the panel runs,
    /// hazards off, from the start pad: does it reach the goal, and without
    /// falling off more than a player would?
    fn probe(&mut self) -> bool {
        let (save_ball, save_wp, save_idle) = (self.ball, self.wp, self.idle);
        // The probe is a simulation of a course nobody watched. Its falls and
        // deaths must not land in the counters that describe what the PANEL
        // did, or a liveness measurement is reading its own validator.
        let saved = (
            self.goals,
            self.falls,
            self.zapped,
            self.respawns,
            self.worst_idle,
            self.stalls,
        );
        let save_mark = self.mark;
        self.wp = 0;
        self.respawn(0.0);
        let mut falls = 0;
        let mut reached = false;
        for _ in 0..PROBE_STEPS {
            match self.advance(false) {
                EV_GOAL => {
                    reached = true;
                    break;
                }
                EV_FELL => {
                    falls += 1;
                    if falls > PROBE_FALLS {
                        break;
                    }
                }
                _ => {}
            }
        }
        self.ball = save_ball;
        self.wp = save_wp;
        self.idle = save_idle;
        (
            self.goals,
            self.falls,
            self.zapped,
            self.respawns,
            self.worst_idle,
            self.stalls,
        ) = saved;
        self.mark = save_mark;
        if !reached {
            self.probe_rejects += 1;
        }
        reached
    }

    // ── physics and autopilot ───────────────────────────────────────────────

    #[inline]
    fn at(x: u8, y: u8) -> usize {
        y as usize * TX + x as usize
    }

    /// Surface height under a world point, or None over the void. Bilinear
    /// between the four surrounding TILE CENTRES, so a stepped heightfield
    /// reads as a ramp and the gradient below is continuous — a nearest-tile
    /// sample makes the marble jitter on every tile boundary.
    #[inline]
    fn height(&self, x: f32, y: f32) -> Option<f32> {
        let (ix, iy) = (x as i32, y as i32);
        if ix < 0 || iy < 0 || ix >= TX as i32 || iy >= TY as i32 {
            return None;
        }
        let here = iy as usize * TX + ix as usize;
        if self.kind[here] == T_VOID {
            return None;
        }
        let (fx, fy) = (x - ix as f32 - 0.5, y - iy as f32 - 0.5);
        let (sx, sy) = (fx.signum() as i32, fy.signum() as i32);
        let (u, v) = (fx.abs(), fy.abs());
        // A void neighbour contributes this tile's own height rather than a
        // hole, so the deck does not sag at its edge.
        let h = |dx: i32, dy: i32| -> f32 {
            let (nx, ny) = (ix + dx, iy + dy);
            if nx < 0 || ny < 0 || nx >= TX as i32 || ny >= TY as i32 {
                return self.hf[here];
            }
            let j = ny as usize * TX + nx as usize;
            // A WALL neighbour contributes this tile's height too. Blending
            // into it would build a ramp up the wall, and the marble would
            // simply drive over the thing that is there to stop it.
            if self.kind[j] == T_VOID || self.kind[j] == T_WALL {
                self.hf[here]
            } else {
                self.hf[j]
            }
        };
        let (h00, h10, h01, h11) = (h(0, 0), h(sx, 0), h(0, sy), h(sx, sy));
        Some(h00 * (1.0 - u) * (1.0 - v) + h10 * u * (1.0 - v) + h01 * (1.0 - u) * v + h11 * u * v)
    }

    #[inline]
    fn kind_at(&self, x: f32, y: f32) -> u8 {
        let (ix, iy) = (x as i32, y as i32);
        if ix < 0 || iy < 0 || ix >= TX as i32 || iy >= TY as i32 {
            return T_VOID;
        }
        self.kind[iy as usize * TX + ix as usize]
    }

    fn respawn(&mut self, kick: f32) {
        // The no-progress ladder, in three lines: a respawn that does not get
        // further than the last one did is a section this autopilot cannot
        // pass, and `STALLS` of those tear the course down. Everything that
        // respawns — a fall, acid, a hammer, an idle timeout — routes through
        // here, so there is one place to get this right.
        self.stalls = if self.wp > self.mark {
            0
        } else {
            self.stalls + 1
        };
        self.mark = self.mark.max(self.wp);
        self.grace = GRACE;
        // The hunter goes back to its post: respawning the marble under a
        // hunter that stayed put is the same death on a loop.
        for h in &mut self.hz {
            if h.kind == H_HUNTER {
                h.x = h.tx as f32 + 0.5;
                h.y = h.ty as f32 + 0.5;
            }
        }
        let cp = (self.wp / CP_EVERY) * CP_EVERY;
        let (x, y) = *self.route.get(cp).unwrap_or(&(3, 3));
        self.wp = cp;
        self.ball = Ball {
            x: x as f32 + 0.5,
            y: y as f32 + 0.5,
            z: self.hf[Self::at(x, y)],
            vx: kick,
            vy: -kick,
            ..Ball::default()
        };
        self.idle = 0;
    }

    /// One simulation step. Returns what happened, which is the whole progress
    /// signal.
    fn advance(&mut self, hazards: bool) -> u8 {
        self.grace = self.grace.saturating_sub(1);
        if hazards {
            self.step_hazards();
        }
        let mut b = self.ball;

        if b.falling > 0 {
            // Out of the world: it keeps falling, visibly, then comes back.
            b.vz -= self.steer * 6.0;
            b.z += b.vz;
            b.x += b.vx;
            b.y += b.vy;
            b.falling += 1;
            self.ball = b;
            if b.falling > FALL_STEPS {
                self.falls += 1;
                self.respawns += 1;
                self.respawn(0.0);
                return EV_FELL;
            }
            return EV_NONE;
        }

        // Autopilot: aim at the next waypoint. A constant thrust and a low drag
        // is what makes it overshoot a corner after a long ramp — which is the
        // marble going over the side, and the reason this looks like someone
        // playing rather than a scripted tour.
        let tgt = self.wp.min(self.route.len().saturating_sub(1));
        let (tx, ty) = self.route[tgt];
        let (dx, dy) = (tx as f32 + 0.5 - b.x, ty as f32 + 0.5 - b.y);
        let d = (dx * dx + dy * dy).sqrt().max(1e-3);
        b.vx += dx / d * self.steer;
        b.vy += dy / d * self.steer;

        // Slope. Sampled either side of the marble rather than at it, so the
        // force is the surface's, not one tile's.
        if let (Some(hl), Some(hr)) = (self.height(b.x - 0.5, b.y), self.height(b.x + 0.5, b.y)) {
            b.vx -= (hr - hl) * SLOPE_G * self.steer;
        }
        if let (Some(hu), Some(hd)) = (self.height(b.x, b.y - 0.5), self.height(b.x, b.y + 0.5)) {
            b.vy -= (hd - hu) * SLOPE_G * self.steer;
        }
        b.vx *= DRAG;
        b.vy *= DRAG;

        let sp = (b.vx * b.vx + b.vy * b.vy).sqrt();
        if sp > self.speed {
            let s = self.speed / sp;
            b.vx *= s;
            b.vy *= s;
        }
        // THE anti-tunnelling line: no substep moves more than `MAX_SUB` tiles.
        let n = ((sp.min(self.speed) / MAX_SUB).ceil() as usize).clamp(1, MAX_SUBSTEPS);
        let dt = 1.0 / n as f32;

        let mut event = EV_NONE;
        for _ in 0..n {
            // Axis at a time, so sliding along a wall works instead of
            // stopping dead in a corner.
            let nx = b.x + b.vx * dt;
            if self.blocked(nx, b.y, b.z) {
                b.vx = -b.vx * BOUNCE;
            } else {
                b.x = nx;
            }
            let ny = b.y + b.vy * dt;
            if self.blocked(b.x, ny, b.z) {
                b.vy = -b.vy * BOUNCE;
            } else {
                b.y = ny;
            }
            if let Some(h) = self.height(b.x, b.y) {
                b.z = h;
            } else {
                // Off the edge. Keep whatever velocity carried it there —
                // the arc away from the deck is most of the joke.
                b.falling = 1;
                b.vz = 0.0;
                event = EV_FELL_OFF;
                break;
            }
        }
        self.ball = b;
        if event != EV_NONE {
            return EV_NONE;
        }

        match self.kind_at(b.x, b.y) {
            T_ACID if self.grace == 0 => {
                self.zapped += 1;
                self.respawns += 1;
                self.respawn(0.0);
                return EV_DIED;
            }
            T_GOAL if self.wp + 2 >= self.route.len() => {
                self.goals += 1;
                return EV_GOAL;
            }
            _ => {}
        }

        if hazards && self.hazard_hit(b.x, b.y, b.z) && self.grace == 0 {
            self.zapped += 1;
            self.respawns += 1;
            self.respawn(0.0);
            return EV_DIED;
        }

        // Progress. A waypoint is reached generously — the marble is a third of
        // a tile across and the route is the centre line, not a rail.
        while self.wp + 1 < self.route.len() {
            let (wx, wy) = self.route[self.wp];
            let (ex, ey) = (wx as f32 + 0.5 - b.x, wy as f32 + 0.5 - b.y);
            if ex * ex + ey * ey > 1.6 * 1.6 {
                break;
            }
            self.wp += 1;
            self.idle = 0;
        }
        self.idle += 1;
        self.worst_idle = self.worst_idle.max(self.idle);
        if self.idle >= self.patience {
            // Wedged, orbiting a wall, or behind a hammer it cannot time. Back
            // to the checkpoint, with a shove — the same position and the same
            // zero velocity would simply do it again.
            self.respawns += 1;
            let k = self.rng_range(0, 200) as f32 / 100.0 - 1.0;
            self.respawn(k * self.speed * 0.5);
            return EV_STALLED;
        }
        EV_NONE
    }

    /// A wall tile, or a surface more than `CLIMB` above the marble. By KIND
    /// as well as by height, because `height` deliberately does not ramp into a
    /// wall — so the height test alone would let a fast marble ride over one.
    /// The void is NOT blocked: falling off is the point.
    #[inline]
    fn blocked(&self, x: f32, y: f32, z: f32) -> bool {
        self.kind_at(x, y) == T_WALL || self.height(x, y).is_some_and(|h| h - z > CLIMB)
    }

    fn step_hazards(&mut self) {
        let (bx, by) = (self.ball.x, self.ball.y);
        for h in &mut self.hz {
            match h.kind {
                H_HAMMER => h.phase += h.rate,
                H_HUNTER => {
                    // LEASHED to the tile it was placed on. An unleashed hunter
                    // follows the marble the whole length of the course and
                    // shoves it off the same catwalk over and over — measured
                    // at 40 falls per goal, which is a marble that never gets
                    // anywhere. Inside the leash it chases; outside it goes
                    // home, which is the patrol the arcade's black marble does.
                    let (hx, hy) = (h.tx as f32 + 0.5, h.ty as f32 + 0.5);
                    let home = (h.x - hx).powi(2) + (h.y - hy).powi(2) > LEASH * LEASH;
                    let (gx, gy) = if home { (hx, hy) } else { (bx, by) };
                    let (dx, dy) = (gx - h.x, gy - h.y);
                    let d = (dx * dx + dy * dy).sqrt().max(1e-3);
                    h.x += dx / d * h.rate;
                    h.y += dy / d * h.rate;
                }
                _ => {}
            }
        }
    }

    /// True when a hazard has the marble this step. The hunter SHOVES rather
    /// than kills, which is what it does in the arcade.
    fn hazard_hit(&mut self, bx: f32, by: f32, _bz: f32) -> bool {
        let mut shove = (0.0f32, 0.0f32);
        let mut dead = false;
        for h in &self.hz {
            match h.kind {
                H_HAMMER => {
                    // Down for the bottom third of its cycle. A marble under it
                    // then is flattened.
                    if h.phase.sin() < -0.6 {
                        let (dx, dy) = (bx - h.x, by - h.y);
                        if dx * dx + dy * dy < 0.8 * 0.8 {
                            dead = true;
                        }
                    }
                }
                H_HUNTER => {
                    let (dx, dy) = (bx - h.x, by - h.y);
                    let d2 = dx * dx + dy * dy;
                    if d2 < (BALL_R * 2.4) * (BALL_R * 2.4) {
                        let d = d2.sqrt().max(1e-3);
                        shove = (dx / d * self.speed * 0.35, dy / d * self.speed * 0.35);
                    }
                }
                _ => {}
            }
        }
        self.ball.vx += shove.0;
        self.ball.vy += shove.1;
        dead
    }

    // ── projection and drawing ──────────────────────────────────────────────

    /// World to GLASS. The one place the 2:1 diamond lives.
    #[inline]
    fn project(&self, x: f32, y: f32, z: f32) -> (f32, f32) {
        ((x - y) * self.tw, (x + y) * self.tw * 0.5 - z * self.zs)
    }

    /// Glass to sub-cell, through the camera. `uy` carries the aspect
    /// correction; see the module doc.
    #[inline]
    fn to_sub(&self, gx: f32, gy: f32) -> (f32, f32) {
        (
            (gx - self.camx) / self.ux + self.sw as f32 * 0.5,
            (gy - self.camy) / self.uy + self.sh as f32 * 0.5,
        )
    }

    #[inline]
    fn span(&mut self, sy: i32, sx0: f32, sx1: f32, c: u16) {
        if sy < 0 || sy >= self.sh as i32 {
            return;
        }
        let x0 = (sx0.round() as i32).max(0);
        let x1 = (sx1.round() as i32).min(self.sw as i32 - 1);
        if x1 < x0 {
            return;
        }
        let (sy, row) = (sy as usize, sy as usize & 3);
        let base = (sy / 4) * self.cols;
        let both = dot_bit(0, row) | dot_bit(1, row);
        let mut x = x0;
        while x <= x1 {
            let i = base + (x as usize >> 1);
            // Whole cell at once wherever both sub-columns are in the run: this
            // halves the work on a filled surface, and a filled surface is most
            // of the panel.
            if x & 1 == 0 && x < x1 {
                self.dots[i] |= both;
                x += 2;
            } else {
                self.dots[i] |= dot_bit((x & 1) as usize, row);
                x += 1;
            }
            self.col[i] = c;
        }
    }

    #[inline]
    fn vspan(&mut self, sx: i32, sy0: i32, sy1: i32, c: u16) {
        if sx < 0 || sx >= self.sw as i32 {
            return;
        }
        let (col, x) = ((sx & 1) as usize, sx as usize >> 1);
        let (mut y, y1) = (sy0.max(0), sy1.min(self.sh as i32 - 1));
        // A cell holds four sub-rows, and a skirt is a tall run: gather the
        // whole cell's worth of dots and write the cell ONCE. Four times fewer
        // stores over the skirts, which are most of a frame's drawing.
        while y <= y1 {
            let cy = y as usize / 4;
            let end = (((cy + 1) * 4 - 1) as i32).min(y1);
            let mut mask = 0u8;
            for yy in y..=end {
                mask |= dot_bit(col, yy as usize & 3);
            }
            let i = cy * self.cols + x;
            self.dots[i] |= mask;
            self.col[i] = c;
            y = end + 1;
        }
    }

    /// One tile: the top diamond, then a skirt down each lower edge to the
    /// neighbour's height. The skirt is what turns a heightfield into cliffs.
    fn draw_tile(&mut self, tx: usize, ty: usize) {
        let i = ty * TX + tx;
        let k = self.kind[i];
        if k == T_VOID {
            return;
        }
        let h = self.hf[i];
        // Centre, east corner and south corner, all through `project`. The
        // half-extents are DERIVED from them rather than written out again:
        // a second copy of the projection's arithmetic here is a copy that can
        // disagree with it, and `diamonds_are_two_to_one_on_the_glass_at_both_aspects`
        // would then be measuring the copy.
        let (cxg, cyg) = self.project(tx as f32 + 0.5, ty as f32 + 0.5, h);
        let (exg, _) = self.project(tx as f32 + 1.0, ty as f32, h);
        let (_, syg) = self.project(tx as f32 + 1.0, ty as f32 + 1.0, h);
        let (cx, cy) = self.to_sub(cxg, cyg);
        let a = (exg - cxg) / self.ux;
        let b = (syg - cyg) / self.uy;
        if cx + a < 0.0
            || cx - a >= self.sw as f32
            || cy + b * 6.0 < 0.0
            || cy - b >= self.sh as f32
        {
            return;
        }

        let base = self.colour(k);
        let bi = b as i32;
        for dy in -bi..=bi {
            let frac = 1.0 - (dy as f32).abs() / b.max(1e-3);
            let half = a * frac;
            self.span(cy as i32 + dy, cx - half, cx + half, base);
        }

        // Skirt depth: how far below this deck the south-west and south-east
        // neighbours sit, in sub-rows. A void neighbour gets a fixed slab, so a
        // catwalk over nothing still has thickness.
        let slab = (self.tw * 0.9 / self.uy) as i32;
        let depth = |m: &Self, nx: usize, ny: usize| -> i32 {
            if nx >= TX || ny >= TY || m.kind[ny * TX + nx] == T_VOID {
                slab
            } else {
                (((h - m.hf[ny * TX + nx]) * m.zs / m.uy) as i32).clamp(0, slab)
            }
        };
        let dsw = depth(self, tx, ty + 1);
        let dse = depth(self, tx + 1, ty);
        if dsw <= 0 && dse <= 0 {
            return;
        }
        let ai = a as i32;
        for dx in -ai..=ai {
            let frac = 1.0 - (dx as f32).abs() / a.max(1e-3);
            let top = cy as i32 + (b * frac) as i32;
            let (d, shade) = if dx < 0 { (dsw, 1) } else { (dse, 2) };
            if d > 0 {
                self.vspan(cx as i32 + dx, top, top + d, base + shade);
            }
        }
    }

    #[inline]
    fn colour(&self, k: u8) -> u16 {
        match k {
            T_PLAIN => C_PLAIN + self.family * 3,
            T_CATWALK => C_CATWALK,
            T_FAST => C_FAST,
            T_ACID => C_ACID,
            T_GOAL => C_GOAL,
            T_START => C_START,
            _ => C_WALL,
        }
    }

    /// A sphere in glass units — round ON THE GLASS, so an ellipse in sub-cells
    /// at aspect 180. Lit from the top left with three bands, which is the only
    /// shading that makes a filled circle read as a ball.
    fn draw_ball(&mut self, x: f32, y: f32, z: f32, r: f32, c0: u16) {
        let (gx, gy) = self.project(x, y, z);
        // Sitting ON the surface, not embedded in it.
        let (cx, cy) = self.to_sub(gx, gy - r * self.zs * 1.6);
        let (ax, ay) = (r * self.tw / self.ux, r * self.tw / self.uy);
        let y0 = (cy - ay).floor() as i32;
        let y1 = (cy + ay).ceil() as i32;
        for sy in y0..=y1 {
            let t = (sy as f32 - cy) / ay.max(1e-3);
            if t.abs() > 1.0 {
                continue;
            }
            let half = ax * (1.0 - t * t).sqrt();
            // Three bands top to bottom: highlight, body, shadow.
            let shade = if t < -0.35 {
                0
            } else if t < 0.45 {
                1
            } else {
                2
            };
            self.span(sy, cx - half, cx + half, c0 + shade);
        }
        // One specular dot, up and to the left.
        let (hx, hy) = (cx - ax * 0.35, cy - ay * 0.45);
        self.span(hy as i32, hx - ax * 0.2, hx + ax * 0.2, c0);
    }

    fn draw_scene(&mut self) {
        self.dots.fill(0);

        // Back to front: increasing `x + y`, so a nearer tile overwrites the
        // one behind it. Within a diagonal the order does not matter — two
        // tiles with the same `x + y` cannot occlude each other.
        for d in 0..(TX + TY - 1) {
            let lo = d.saturating_sub(TY - 1);
            for tx in lo..=d.min(TX - 1) {
                self.draw_tile(tx, d - tx);
            }
        }

        for k in 0..self.hz.len() {
            let h = self.hz[k];
            match h.kind {
                H_HAMMER => {
                    // Rides up and slams down on its tile; the head is drawn at
                    // whatever height the cycle has it.
                    let up = (h.phase.sin() + 1.0) * 0.5;
                    let z = self.hf[Self::at(h.tx, h.ty)] + 0.6 + up * 3.4;
                    self.draw_ball(h.x, h.y, z, 0.55, C_HAMMER);
                }
                H_HUNTER => {
                    let z = self.height(h.x, h.y).unwrap_or(0.0);
                    self.draw_ball(h.x, h.y, z, BALL_R * 0.95, C_HUNT);
                }
                _ => {}
            }
        }

        let b = self.ball;
        self.draw_ball(b.x, b.y, b.z, BALL_R, C_BALL);
    }

    // ── rng ─────────────────────────────────────────────────────────────────

    #[inline]
    fn rng_bool(&mut self) -> bool {
        next_rand(&mut self.rng) & 1 == 1
    }

    /// `lo..hi`, or `lo` when the range is empty — a course too short to hold a
    /// hazard must not panic on the panel.
    #[inline]
    fn rng_range(&mut self, lo: u32, hi: u32) -> u32 {
        if hi <= lo {
            return lo;
        }
        lo + next_rand(&mut self.rng) % (hi - lo)
    }
}

const EV_NONE: u8 = 0;
const EV_FELL_OFF: u8 = 1;
const EV_FELL: u8 = 2;
const EV_DIED: u8 = 3;
const EV_GOAL: u8 = 4;
const EV_STALLED: u8 = 5;

impl Saver for Marble {
    fn render(&mut self, s: &mut Surface<'_>) {
        let ev = self.advance(true);
        self.left = self.left.saturating_sub(1);
        // Reaching the goal is the natural regeneration point; the timer is
        // only there so a course nothing can finish cannot hold the panel, and
        // `STALLS` is the no-progress break.
        if ev == EV_GOAL || self.left == 0 || self.stalls >= STALLS {
            self.new_course();
        }

        // Camera: exponential chase, so it leads a fast marble and does not
        // snap when it respawns behind itself.
        let (gx, gy) = self.project(self.ball.x, self.ball.y, self.ball.z);
        self.camx += (gx - self.camx) * 0.08;
        self.camy += (gy - self.camy) * 0.08;

        self.draw_scene();

        let (grid, dots, col, cols) = (&mut self.grid, &self.dots[..], &self.col[..], self.cols);
        grid.fill(|cx, cy| {
            let i = cy * cols + cx;
            let d = dots[i];
            if d == 0 {
                Cell::CLEAR
            } else {
                Cell::new(font::BRAILLE[d as usize], col[i])
            }
        });
        grid.flush(s, &PAL);
    }

    fn name(&self) -> &'static str {
        "marble"
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

    /// 1051 is not a multiple of the cell height: the strip below the last cell
    /// row is the point of the frame-0 assertion.
    fn panel() -> Panel {
        Panel::new(1920, 1051, 1920)
    }

    /// The live panel's shape.
    fn short() -> Panel {
        Panel::new(1280, 400, 1280)
    }

    /// T1. Frame 0 must cover the whole panel, including the strip below the
    /// last cell row, and must cover it with a COURSE — a reported black
    /// rectangle satisfies a rows assertion on its own.
    #[test]
    fn frame_zero_paints_the_whole_panel() {
        let p = panel();
        let mut m = Marble::new(&p, 30);
        assert!(!p.h.is_multiple_of(m.grid.cell_h()), "panel divides evenly");
        let mut buf = vec![0u32; p.buf_len()];
        let d = saver::frame(&mut m, &mut buf, &p);
        assert_eq!(d.rows(), p.h, "frame 0 must paint the whole panel");
        let lit = buf.iter().filter(|&&px| px != 0).count();
        assert!(lit > 40_000, "frame 0 drew no course ({lit} lit)");
    }

    /// T2. THE projection test. A tile's drawn diamond must be twice as wide as
    /// it is tall ON THE GLASS — at aspect 100, where the framebuffer and the
    /// glass agree, and at 180, where they do not. Measured off the rendered
    /// pixels, not off the constants: the bug this catches is the correction
    /// applied to the wrong axis, and the constants are right in both worlds.
    #[test]
    fn diamonds_are_two_to_one_on_the_glass_at_both_aspects() {
        for aspect in [100usize, 180] {
            let p = Panel::new(1280, 720, 1280);
            let (w, h, cell_h) = with_test_aspect(aspect, || {
                let mut m = Marble::new(&p, 30);
                // One tile, alone in the void, dead centre of the camera.
                m.kind.fill(T_VOID);
                m.hf.fill(0.0);
                m.hz.clear();
                // Neighbours at the same height, so no skirt is drawn and what
                // is measured below is the TOP FACE alone.
                for (x, y) in [(20usize, 20usize), (21, 20), (20, 21), (21, 21)] {
                    m.kind[y * TX + x] = T_PLAIN;
                }
                let (gx, gy) = m.project(20.0, 20.0, 0.0);
                m.camx = gx;
                m.camy = gy + m.tw * 0.5;
                m.ball = Ball {
                    x: -50.0,
                    y: -50.0,
                    ..Ball::default()
                };
                m.dots.fill(0);
                m.draw_tile(20, 20);
                // The diamond's extent in SUB-CELLS, from the dots actually set.
                let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
                for cy in 0..m.grid.rows() {
                    for cx in 0..m.cols {
                        let d = m.dots[cy * m.cols + cx];
                        if d == 0 {
                            continue;
                        }
                        for sub in 0..8u8 {
                            let (dc, dr) = (usize::from(sub >= 4), (sub & 3) as usize);
                            if d & dot_bit(dc, dr) == 0 {
                                continue;
                            }
                            let (sx, sy) = (cx * 2 + dc, cy * 4 + dr);
                            x0 = x0.min(sx);
                            x1 = x1.max(sx);
                            y0 = y0.min(sy);
                            y1 = y1.max(sy);
                        }
                    }
                }
                assert!(x1 > x0, "aspect {aspect}: nothing was drawn");
                // Sub-cells to GLASS units: this is the conversion under test.
                let w = (x1 - x0 + 1) as f32 * m.ux;
                let h = (y1 - y0 + 1) as f32 * m.uy;
                (w, h, m.grid.cell_h())
            });
            let ratio = w / h;
            assert!(
                (ratio - 2.0).abs() < 0.16,
                "aspect {aspect} (cell_h {cell_h}px): the tile is {w:.1} x \
                 {h:.1} glass units, a {ratio:.2}:1 diamond, not 2:1"
            );
        }
        // Non-vacuous: the two aspects really do render through different cell
        // geometry, so the assertion above is testing the correction and not a
        // constant that happens to hold twice.
        let p = Panel::new(1280, 720, 1280);
        let a = Marble::new(&p, 30).grid.cell_h();
        let b = with_test_aspect(180, || Marble::new(&p, 30).grid.cell_h());
        assert!(b > a, "aspect 180 did not change the cell: {a} vs {b}");
    }

    /// T3. Every scanline whose pixels changed is inside a damage run —
    /// otherwise simpledrm scans out the previous frame there forever. Long
    /// enough to cross several course regenerations, which is where a
    /// whole-panel change lands.
    #[test]
    fn damage_covers_every_changed_scanline() {
        let p = panel();
        let mut m = Marble::new(&p, 30);
        m.course_steps = 150;
        m.left = m.course_steps;
        let mut buf = vec![0u32; p.buf_len()];
        let mut prev = vec![0u32; p.buf_len()];
        saver::frame(&mut m, &mut buf, &p);
        for n in 1..900 {
            prev.copy_from_slice(&buf);
            let d = saver::frame(&mut m, &mut buf, &p);
            for y in 0..p.h {
                let row = y * p.w..y * p.w + p.w;
                if buf[row.clone()] != prev[row] {
                    assert!(
                        dump::row_reported(&prev[y * p.w..][..p.w], &buf[y * p.w..][..p.w], y, &d),
                        "frame {n}: scanline {y} changed outside every reported rect"
                    );
                }
            }
        }
        assert!(m.courses >= 3, "the run never regenerated: {}", m.courses);
    }

    /// T4. THE tunnelling test. A one-tile catwalk over the void, and a marble
    /// fired at it at exactly the per-step cap from every angle: it must never
    /// appear on the far side, because passing through a catwalk is passing
    /// through the only thing in this game that is thin.
    #[test]
    fn a_marble_at_full_speed_cannot_cross_a_catwalk() {
        let p = panel();
        let mut m = Marble::new(&p, 30);
        m.kind.fill(T_VOID);
        m.hf.fill(0.0);
        m.hz.clear();
        m.route.clear();
        // A wall one tile wide running down the middle of an otherwise flat
        // plain, standing WALL_H proud: the thinnest barrier a course holds.
        for y in 0..TY {
            for x in 0..TX {
                m.kind[y * TX + x] = T_PLAIN;
                m.hf[y * TX + x] = 0.0;
            }
        }
        m.route.push((10, 10));
        m.speed = SUB_CAP;
        m.steer = 0.0;

        // Two barriers, because the wall has two independent guards and each
        // hides the other's absence: PROUD, where the height test holds it, and
        // FLUSH with the deck, where only the tile KIND can.
        for (label, wall_h) in [("proud", WALL_H), ("flush", 0.0)] {
            for y in 0..TY {
                m.kind[y * TX + 20] = T_WALL;
                m.hf[y * TX + 20] = wall_h;
            }
            // And the deck beside a wall must stay FLAT: `height` refuses to blend
            // into a wall, because a bilinear ramp up the side of the thing that is
            // there to stop you is a thing a fast marble simply drives over.
            assert_eq!(
                m.height(19.9, 10.0),
                Some(0.0),
                "{label}: the deck ramps up into the wall"
            );

            for deg in 0..24 {
                let a = deg as f32 * std::f32::consts::TAU / 24.0;
                // Always aimed at the wall from the west side.
                let dir = (a.cos().abs() + 0.25, a.sin() * 0.6);
                let len = (dir.0 * dir.0 + dir.1 * dir.1).sqrt();
                m.ball = Ball {
                    x: 17.5,
                    y: 8.0 + deg as f32,
                    vx: dir.0 / len * SUB_CAP,
                    vy: dir.1 / len * SUB_CAP,
                    ..Ball::default()
                };
                m.wp = 0;
                m.idle = 0;
                for step in 0..40 {
                    let saved = m.ball;
                    m.advance(false);
                    // `advance` respawns on a stall; that would hide a tunnel.
                    if m.ball.x < saved.x - 5.0 {
                        break;
                    }
                    assert!(
                        m.ball.x < 21.0,
                        "{label} wall, angle {deg}, step {step}: the marble is at \
                     x={} past a wall at x=20 — it went through",
                        m.ball.x
                    );
                }
            }
        }
    }

    /// T4b. And the invariant that rests on: the per-step speed is clamped so
    /// the substep count cannot saturate, at every frame rate and every value
    /// of the knob.
    #[test]
    fn no_substep_can_ever_cross_a_tile() {
        // The WHOLE knob range, not the default: `MARBLE_SPEED` goes to 20000
        // milli-tiles a second, which at 1 fps is 20 tiles in a step and well
        // past the substep budget. The default never reaches the clamp, so a
        // test built on `Marble::new` alone proves nothing about it.
        for per_second in [0.5f32, 4.2, 20.0] {
            for fps in [1u32, 15, 30, 120] {
                let speed = Marble::step_speed(per_second, fps);
                assert!(
                    speed <= SUB_CAP,
                    "{per_second} tiles/s at {fps} fps: {speed} over budget"
                );
                let n = ((speed / MAX_SUB).ceil() as usize).clamp(1, MAX_SUBSTEPS);
                let per = speed / n as f32;
                assert!(
                    per <= MAX_SUB + 1e-4,
                    "{per_second} tiles/s at {fps} fps: a substep moves {per} tiles"
                );
                // The margin the module doc quotes against a one-tile catwalk.
                assert!(per * 4.0 < 1.0, "the catwalk margin is gone");
            }
        }
        // And the constructor really does route through it.
        assert_eq!(
            Marble::new(&panel(), 30).speed,
            Marble::step_speed(4.2, 30),
            "the default speed did not come from step_speed"
        );
    }

    /// T5. It must never stop making progress. A long run at BOTH panel shapes,
    /// watching the route index — the measure `advance` already computes —
    /// with a floor on the WORST interval rather than an average, and the
    /// counters that prove each recovery actually fired.
    ///
    /// The evidence standard is life.rs's: long enough that a course which
    /// cannot be finished has many chances to prove it.
    #[test]
    fn it_never_stops_making_progress() {
        for (label, p, aspect) in [("1080p", panel(), 100), ("panel", short(), 180)] {
            let m = with_test_aspect(aspect, || {
                let mut m = Marble::new(&p, 30);
                for _ in 0..120_000 {
                    let ev = m.advance(true);
                    if ev == EV_GOAL || m.stalls >= STALLS {
                        m.new_course();
                    }
                }
                m
            });
            println!(
                "{label}: goals {} courses {} falls {} zapped {} respawns {} worst-idle {}",
                m.goals, m.courses, m.falls, m.zapped, m.respawns, m.worst_idle
            );
            // The no-progress break is the ceiling on a static panel: nothing
            // can go longer than `patience` steps without either progress or a
            // respawn, whatever the course looks like.
            assert!(
                m.worst_idle <= m.patience,
                "{label}: {} steps with no route progress, patience is {}",
                m.worst_idle,
                m.patience
            );
            // Non-vacuous: it is finishing courses, not merely being rescued.
            assert!(
                m.goals > 20,
                "{label}: the goal was reached {} times in 120k steps",
                m.goals
            );
            assert!(
                m.courses > 20,
                "{label}: {} courses in 120k steps",
                m.courses
            );
            // And it plays like a player: it falls off, but not constantly.
            assert!(m.falls > 0, "{label}: the marble never fell off once");
            // The grace period after a respawn, asserted by its effect: without
            // it a checkpoint beside an acid pool is an infinite death loop,
            // measured at 1 992 deaths in 60 000 steps against 104 with it.
            assert!(
                m.zapped < m.goals * 8,
                "{label}: {} hazard deaths for {} goals — a respawn is landing \
                 back inside a hazard",
                m.zapped,
                m.goals
            );
            assert!(
                m.falls < m.goals * 40,
                "{label}: {} falls for {} goals — it never gets anywhere",
                m.falls,
                m.goals
            );
            println!(
                "{label}: goals {} courses {} falls {} zapped {} respawns {} worst-idle {}",
                m.goals, m.courses, m.falls, m.zapped, m.respawns, m.worst_idle
            );
        }
    }

    /// T6. The acceptance criteria have to REJECT, or "validated" means nothing.
    #[test]
    fn validation_rejects_the_courses_it_is_for() {
        let p = panel();
        let mut m = Marble::new(&p, 30);

        // Each case asserts WHICH criterion fired, not merely that something
        // did: a rejection for the wrong reason is a criterion that is never
        // exercised, and it passes an `is_err`-shaped assertion every time.

        // A stub route.
        m.kind.fill(T_VOID);
        m.route.clear();
        m.hz.clear();
        assert_eq!(m.reject(), R_LEN, "an empty course was accepted");

        // D — size: a real route over no deck at all.
        m.lay_out();
        m.kind.fill(T_VOID);
        assert_eq!(m.reject(), R_SIZE, "a course with no deck was accepted");

        // C — spread: a route crammed into one corner.
        m.lay_out();
        m.route.clear();
        for i in 0..30u8 {
            m.route.push((3 + i % 4, 3 + i / 4));
        }
        assert_eq!(m.reject(), R_SPAN, "a course in one corner was accepted");

        // A — the goal walled off. A candidate that passes, then the tile
        // before the goal raised out of reach.
        let mut tries = 0;
        m.lay_out();
        while !m.validate() && tries < 400 {
            m.lay_out();
            tries += 1;
        }
        assert!(tries < 400, "no candidate passed A-D in 400 tries");
        let n = m.route.len();
        for &(x, y) in &m.route[n - 3..] {
            for dy in -2i32..=2 {
                for dx in -2i32..=2 {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= TX as i32 || ny >= TY as i32 {
                        continue;
                    }
                    let i = ny as usize * TX + nx as usize;
                    if m.kind[i] != T_GOAL {
                        m.kind[i] = T_WALL;
                        m.hf[i] = 90.0;
                    }
                }
            }
        }
        assert_eq!(
            m.reject(),
            R_GOAL,
            "a course with an unreachable goal was accepted"
        );

        // B — a hazard over the void.
        m.lay_out();
        let mut tries = 0;
        while !m.validate() && tries < 400 {
            m.lay_out();
            tries += 1;
        }
        assert!(tries < 400);
        m.hz.push(Hazard {
            kind: H_HAMMER,
            tx: 0,
            ty: 0,
            ..Hazard::default()
        });
        assert_eq!(m.reject(), R_HAZARD, "a hazard over the void was accepted");
    }

    /// T7. Criterion E rejects courses A-D let through: there has to be at
    /// least one, or the probe is decoration. Counted over many seeds at both
    /// aspects, with the candidates-per-course rate reported.
    #[test]
    fn the_physics_probe_rejects_courses_the_flood_fill_accepts() {
        for (label, p, aspect) in [("1080p", panel(), 100), ("panel", short(), 180)] {
            let (tries, courses, rejects) = with_test_aspect(aspect, || {
                let mut m = Marble::new(&p, 30);
                for _ in 0..120 {
                    m.new_course();
                }
                (m.tries, m.courses, m.probe_rejects)
            });
            let per = tries as f32 / courses as f32;
            assert!(
                per < 8.0,
                "{label}: {per:.2} candidates per course ({tries} for {courses})"
            );
            assert!(
                rejects > 0,
                "{label}: the probe never rejected anything in {tries} candidates"
            );
            println!("{label}: {per:.2} candidates/course, {rejects} probe rejects");
        }
    }

    /// T8. `render` cannot allocate — and the run crosses several course
    /// REGENERATIONS, because that is the one place a pool could grow.
    #[test]
    fn render_never_allocates() {
        let p = Panel::new(640, 400, 640);
        let mut m = Marble::new(&p, 30);
        // A forced regeneration every 150 frames, so the run crosses ~20 of
        // them without an env var the parallel harness would race on.
        m.course_steps = 150;
        m.left = m.course_steps;
        let mut buf = vec![0u32; p.buf_len()];
        saver::frame(&mut m, &mut buf, &p);
        let before = m.courses;

        let n = crate::testalloc::allocs_during(|| {
            for _ in 0..3_000 {
                saver::frame(&mut m, &mut buf, &p);
            }
        });
        assert_eq!(n, 0, "the render path allocated {n} times");
        assert!(
            m.courses - before >= 15,
            "the run crossed only {} regenerations — one that allocated would \
             not have been seen",
            m.courses - before
        );
    }

    /// T9. The panel's own aspect. At `SAVER_PIXEL_ASPECT=180` a cell is 1.8x
    /// taller, so `rows` is nearly halved — and everything derived from the row
    /// count has to follow. Asserting scalars here is what let a bug eat every
    /// toaster in `toasters3`.
    #[test]
    fn the_view_follows_the_rows_at_the_panel_aspect() {
        let p = short();
        let square = Marble::new(&p, 30);
        let tall = with_test_aspect(180, || Marble::new(&p, 30));

        assert!(
            tall.grid.rows() * 18 <= square.grid.rows() * 11,
            "aspect 180 did not shorten the grid by ~1.8x: {} vs {}",
            tall.grid.rows(),
            square.grid.rows()
        );
        assert_eq!(tall.sh, tall.grid.rows() * 4, "the field is not the grid");
        // The auto tile size comes off the GLASS height, which the aspect
        // divides back out — so a panel with half the rows keeps roughly the
        // same tile count on screen rather than half the tiles at twice the
        // size.
        let rows_of_tiles = |m: &Marble| m.sh as f32 * m.uy / (m.tw * 0.5);
        let (a, b) = (rows_of_tiles(&square), rows_of_tiles(&tall));
        assert!(
            (a - b).abs() < a * 0.35,
            "the short panel shows {b:.1} tile rows against {a:.1}"
        );
        // And the sub-cell is genuinely a different shape in glass units.
        assert!(
            (tall.uy - square.uy).abs() < square.uy * 0.12,
            "uy moved with the aspect ({} vs {}) — the correction is not \
             dividing it back out",
            tall.uy,
            square.uy
        );
    }

    /// T10. It still PLAYS at the panel aspect: a course that generates and
    /// then never moves is the same dead panel as one that settles.
    #[test]
    fn the_marble_plays_the_course_at_the_panel_aspect() {
        let p = short();
        let (goals, courses, moved) = with_test_aspect(180, || {
            let mut m = Marble::new(&p, 30);
            let mut buf = vec![0u32; p.buf_len()];
            let start = m.project(m.ball.x, m.ball.y, m.ball.z);
            let mut moved = 0.0f32;
            let mut last = start;
            for _ in 0..12_000 {
                saver::frame(&mut m, &mut buf, &p);
                let now = m.project(m.ball.x, m.ball.y, m.ball.z);
                moved += ((now.0 - last.0).powi(2) + (now.1 - last.1).powi(2)).sqrt();
                last = now;
            }
            (m.goals, m.courses, moved)
        });
        assert!(goals > 0, "the goal was never reached on the real panel");
        assert!(courses > 1, "the course never regenerated");
        assert!(
            moved > 10_000.0,
            "the marble barely moved ({moved:.0} glass px)"
        );
    }

    /// T11. The painter's algorithm. A nearer tile must OVERWRITE the one
    /// behind it, not be overwritten by it — draw the pair in course order and
    /// the near tile has to be as visible as it is when drawn alone.
    #[test]
    fn a_nearer_tile_covers_the_one_behind_it() {
        let p = panel();
        let mut m = Marble::new(&p, 30);
        m.kind.fill(T_VOID);
        m.hf.fill(0.0);
        m.hz.clear();
        // The ball far outside the world, so `draw_scene` adds nothing.
        m.ball = Ball {
            x: -500.0,
            y: -500.0,
            ..Ball::default()
        };
        // Same screen column (x - y equal), the nearer one raised so its top
        // face projects up into the far one's diamond.
        m.kind[18 * TX + 18] = T_ACID;
        m.kind[19 * TX + 19] = T_GOAL;
        // Raised by exactly two height units, which is what it takes for the
        // near tile's top face to project onto the far one's: `zs` is half a
        // tile-height, and one step along the diagonal is a whole one.
        m.hf[19 * TX + 19] = 2.0;
        let (cxg, cyg) = m.project(19.5, 19.5, 2.0);
        m.camx = cxg;
        m.camy = cyg;

        let count = |m: &Marble, c: u16| {
            (0..m.cols * m.grid.rows())
                .filter(|&i| m.dots[i] != 0 && m.col[i] == c)
                .count()
        };
        m.dots.fill(0);
        m.col.fill(0);
        m.draw_tile(19, 19);
        let near_alone = count(&m, C_GOAL);
        m.dots.fill(0);
        m.col.fill(0);
        m.draw_tile(18, 18);
        let far_alone = count(&m, C_ACID);
        assert!(near_alone > 20 && far_alone > 20, "a tile drew nothing");

        m.draw_scene();
        // Non-vacuous: the two really do land on each other, so one of them
        // has to lose its top face entirely.
        assert!(
            count(&m, C_ACID) < far_alone,
            "the two tiles do not overlap — nothing is proven"
        );
        assert_eq!(
            count(&m, C_GOAL),
            near_alone,
            "the tile BEHIND overdrew the one in front: the painter's order is \
             back to front, so a course draws itself inside out"
        );
    }

    /// T12. A respawn goes back to the last CHECKPOINT, not to the start. A
    /// fall that costs the whole course is a course nothing ever finishes.
    #[test]
    fn a_respawn_returns_to_the_last_checkpoint() {
        let p = panel();
        let mut m = Marble::new(&p, 30);
        assert!(m.route.len() > 2 * CP_EVERY, "the test course is too short");
        for wp in [
            0usize,
            1,
            CP_EVERY - 1,
            CP_EVERY,
            CP_EVERY + 1,
            2 * CP_EVERY,
        ] {
            m.wp = wp;
            m.mark = wp;
            m.respawn(0.0);
            assert_eq!(
                m.wp,
                (wp / CP_EVERY) * CP_EVERY,
                "a respawn from waypoint {wp} did not land on its checkpoint"
            );
            let (x, y) = m.route[m.wp];
            assert!((m.ball.x - (x as f32 + 0.5)).abs() < 1e-3);
            assert!((m.ball.y - (y as f32 + 0.5)).abs() < 1e-3);
        }
    }

    /// T13. The hunter is LEASHED to its post. Unleashed it follows the marble
    /// the length of the course and shoves it off the same catwalk forever —
    /// measured at 40 falls per goal, which is a marble that never arrives.
    #[test]
    fn the_hunter_never_strays_far_from_its_post() {
        let p = panel();
        let mut m = Marble::new(&p, 30);
        m.hz.clear();
        m.hz.push(Hazard {
            kind: H_HUNTER,
            x: 20.5,
            y: 20.5,
            tx: 20,
            ty: 20,
            rate: 0.03,
            ..Hazard::default()
        });
        // The marble parked at the far corner, pulling as hard as it can.
        m.ball = Ball {
            x: 38.5,
            y: 38.5,
            ..Ball::default()
        };
        let mut worst = 0.0f32;
        for _ in 0..4_000 {
            m.step_hazards();
            let h = m.hz[0];
            worst = worst.max(((h.x - 20.5).powi(2) + (h.y - 20.5).powi(2)).sqrt());
        }
        assert!(
            worst > 1.0,
            "the hunter never moved: the bound proves nothing"
        );
        assert!(
            worst <= LEASH + m.hz[0].rate * 2.0,
            "the hunter reached {worst:.2} tiles from its post, leash is {LEASH}"
        );
    }

    /// T14. A respawn is not an instant death. The worst case, and the one that
    /// measured 1 992 deaths in 60 000 steps before the grace period existed:
    /// the checkpoint tile itself is acid. The marble has to get a moment.
    #[test]
    fn a_respawn_is_not_an_instant_death() {
        let p = panel();
        let mut m = Marble::new(&p, 30);
        // A flat plain built by hand rather than a generated course: the
        // marble cannot fall off it, cannot reach a goal, and sits exactly
        // where it is put, so the only thing that can happen to it is the acid.
        m.kind.fill(T_PLAIN);
        m.hf.fill(0.0);
        m.hz.clear();
        m.route.clear();
        m.route.push((20, 20));
        m.kind[20 * TX + 20] = T_ACID;
        m.wp = 0;
        m.mark = 0;
        // No autopilot: the marble stays on the pool instead of driving off it,
        // so the grace is the only thing keeping it alive.
        m.steer = 0.0;
        m.respawn(0.0);
        let before = m.zapped;
        // Stop short of the end of the grace, so this is about the grace and
        // not about how long it is.
        for step in 0..GRACE - 5 {
            m.advance(true);
            assert_eq!(
                m.zapped, before,
                "step {step}: the marble died inside its own grace period — a \
                 checkpoint on a hazard is then an infinite death loop"
            );
        }
        assert!(m.grace > 0, "the grace ran out early");
        // Non-vacuous: the hazard is real, and it kills the moment the grace
        // is spent.
        m.grace = 0;
        m.advance(true);
        assert!(m.zapped > before, "the acid never killed it at all");
    }

    /// Every colour index a cell can address must exist: one off the end is an
    /// index panic inside `Grid::blit`, on the panel, weeks in.
    #[test]
    fn every_reachable_colour_index_is_in_the_palette() {
        assert_eq!(PAL_LEN, C_HAMMER as usize + 3);
        assert_ne!(PAL[PAL_LEN - 1], 0, "the last entry is black");
        // The widest index any tile kind can address.
        const { assert!((C_WALL + 2) < C_BALL) };
        assert_eq!(T_WALL, 7, "a tile kind was added without a palette entry");
        let p = panel();
        let mut m = Marble::new(&p, 30);
        m.course_steps = 200;
        m.left = m.course_steps;
        let mut buf = vec![0u32; p.buf_len()];
        for _ in 0..1_200 {
            saver::frame(&mut m, &mut buf, &p);
            for c in m.grid.cells() {
                assert!(c.colour() < PAL_LEN, "colour {} is off the end", c.colour());
            }
        }
        assert!(m.courses > 3);
    }
}

#[cfg(test)]
mod diag {
    use super::*;

    #[test]
    #[ignore = "diagnostic"]
    fn play_stats() {
        let p = Panel::new(1920, 1051, 1920);
        for hz in [false, true] {
            for seed in [1u32, 2, 3] {
                let mut m = Marble::new(&p, 30);
                m.rng = seed.wrapping_mul(0x9E37_79B9) | 1;
                m.hazards = if hz { 3 } else { 0 };
                m.new_course();
                m.goals = 0;
                m.falls = 0;
                m.zapped = 0;
                m.respawns = 0;
                m.courses = 0;
                for _ in 0..60_000 {
                    let ev = m.advance(true);
                    if ev == EV_GOAL || m.stalls >= STALLS {
                        m.new_course();
                    }
                }
                println!(
                    "hz={hz} seed={seed}: goals {} courses {} falls {} zapped {} stallresp {}",
                    m.goals,
                    m.courses,
                    m.falls,
                    m.zapped,
                    m.respawns - m.falls - m.zapped
                );
            }
        }
    }

    #[test]
    #[ignore = "diagnostic"]
    fn gen_stats() {
        let p = Panel::new(1920, 1051, 1920);
        let mut m = Marble::new(&p, 30);
        let (mut ad, mut e) = (0, 0);
        let mut reasons = [0usize; 7];
        for _ in 0..400 {
            m.lay_out();
            let r = m.reject();
            reasons[r as usize] += 1;
            if r == 0 {
                ad += 1;
                if m.probe() {
                    e += 1;
                }
            }
        }
        println!(
            "400 candidates: A-D pass {ad}, probe (E) pass {e}, \
             rejects by criterion [-,len,size,span,drop,goal,hazard] {reasons:?}"
        );
    }
}
