//! Speeder — a first-person chase through the forest moon, low and fast between
//! redwood trunks. The trunks are the obstacle and the scenery at once.
//!
//! # What it is, and what it is not
//!
//! `warp` is a point field streaming out of a vanishing point; this is a
//! CORRIDOR of tall solid columns with a lit floor under it and a canopy over
//! it. The near miss is the subject: the camera weaves on two incommensurate
//! sines, and one spawn in [`NEAR_MISS_IN`] is placed by predicting where the
//! camera WILL be when that trunk arrives, offset by a metre — so trunks really
//! do graze the frame rather than happening to. A trunk whose screen position
//! moved more than `blur_cols` in a frame is stamped in `SHADE` rather than
//! `SOLID`, which is the stipple that reads as motion blur at the edges, where
//! the parallax is fastest. That stipple is the one screen-space texture here:
//! bark grain, floor and canopy are all sampled in the world, so they move
//! with what they are painted on.
//!
//! # Geometry lives in CELLS, which is the aspect correction
//!
//! The grid is built square — `Grid::new(panel, cell, cell)` — so
//! `SAVER_PIXEL_ASPECT` makes the cell as much taller in framebuffer pixels as
//! the panel then squashes it, and one cell is SQUARE ON THE GLASS at any
//! aspect. Every number here is therefore in cells (on screen) and metres (in
//! the world), one focal length serves both axes, and nothing needs the
//! explicit stretch `warp` and `moire` carry. What does change at 180 is that
//! there are half as many ROWS: the vertical field of view is genuinely
//! narrower on a 1280x400 panel, the horizon still sits at
//! `rows * SPEEDER_HORIZON`, and trunks run off the top of the frame instead of
//! ending under the canopy. `the_projection_is_square_pixel_derived` pins both
//! halves of that.
//!
//! # The floor and the ceiling are the same trick
//!
//! One screen row is one depth. Below the horizon the row is a point on the
//! ground plane `CAM_H` below the eye; above it, a point on the canopy
//! underside `CANOPY_H - CAM_H` above. Either way `z = height * focal /
//! |row - horizon|`, so a row costs one divide and a cell costs one multiply
//! into a wrapping 64x64 light tile. That tile IS the dappled light: baked once
//! from four integer harmonics so it wraps seamlessly, sampled in WORLD
//! coordinates, so the pools of light stream toward the camera with the right
//! parallax and drift sideways on their own as the canopy moves.
//!
//! # Per-frame cost
//!
//! Full repaint (`Grid::flush`, never `flush_sparse` — a hand-kept dirty list
//! here would have to describe overlapping trunks, and would under-report the
//! first time two of them crossed). The background is one pass over the cells
//! with a divide per ROW; trunks are painted over it far-to-near, so the total
//! is cells plus the overdraw of however many trunks are close enough to be
//! wide. Nothing in `render` allocates: the trunk pools are sized in `new` and
//! recycled as they pass the camera, and `order` is sorted in place.
//!
//! # Knobs
//!
//! * `SPEEDER_CELL` — cell side in px, 4..=32 (default 8). Square: see above.
//! * `SPEEDER_SPEED` — metres per second, 10..=300 (default 58).
//! * `SPEEDER_TRUNKS` — trunks alive in the corridor, 8..=400 (default 60).
//! * `SPEEDER_FOV` — focal length as a per-cent of the panel's COLUMNS,
//!   20..=200 (default 62). Smaller is wider-angle and faster-looking.
//! * `SPEEDER_HORIZON` — eye line as a per-cent of rows, 10..=80 (default 44).
//! * `SPEEDER_WEAVE` — how far the bike swings off the path, in DECIMETRES,
//!   0..=200 (default 64). 0 flies straight and gives up the whole point.
//! * `SPEEDER_DAPPLE` — per-cent of the forest floor lying in a pool of light,
//!   0..=100 (default 34).
//! * `SPEEDER_LOG_SECS` — mean seconds between fallen trunks to duck under,
//!   0..=600 (default 16; 0 = off).
//! * `SPEEDER_RIDER_SECS` — mean seconds between another bike crossing the
//!   view, 0..=600 (default 12; 0 = off).
//! * `SPEEDER_SEED` — 0 (default) rolls one from the clock and the pid; any
//!   other value reproduces the ride exactly.

use crate::font;
use crate::grid::{bake, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, next_rand, saver_seed};

/// Deep green, brown, and the light that comes through. Three depth buckets per
/// material, because on a black panel a flat fill at every distance is the one
/// thing that stops a forest reading as deep.
#[rustfmt::skip]
const PAL_RGB: [[u8; 3]; 33] = [
    [0x00, 0x00, 0x00], //  0 the panel's own black
    // canopy, overhead -> far: mass / leaf / the gap the light comes through
    [0x08, 0x12, 0x0A], //  1
    [0x14, 0x28, 0x0F], //  2
    [0xB9, 0xCE, 0x7A], //  3
    [0x0E, 0x1C, 0x0D], //  4
    [0x1E, 0x3C, 0x15], //  5
    [0xC6, 0xD9, 0x8C], //  6
    [0x18, 0x2A, 0x18], //  7
    [0x2B, 0x4A, 0x22], //  8
    [0xD4, 0xE3, 0xA4], //  9
    // forest floor, underfoot -> far: duff / fern / the pool of light on it
    [0x0F, 0x14, 0x08], // 10
    [0x33, 0x50, 0x1A], // 11
    [0x93, 0xAE, 0x42], // 12
    [0x16, 0x20, 0x0C], // 13
    [0x3C, 0x5C, 0x1E], // 14
    [0xA8, 0xC2, 0x4F], // 15
    [0x22, 0x30, 0x1A], // 16
    [0x46, 0x68, 0x2A], // 17
    [0xB2, 0xC7, 0x65], // 18
    [0x4A, 0x5C, 0x3A], // 19 mist on the eye line, where floor meets canopy
    // trunks, grazing the camera -> far: shadow side / bark / the lit edge
    [0x0B, 0x08, 0x05], // 20
    [0x1C, 0x11, 0x09], // 21
    [0x5A, 0x3E, 0x27], // 22
    [0x15, 0x0F, 0x0A], // 23
    [0x2A, 0x1D, 0x12], // 24
    [0x6E, 0x4E, 0x32], // 25
    [0x22, 0x25, 0x1C], // 26
    [0x31, 0x36, 0x26], // 27
    [0x4A, 0x52, 0x38], // 28
    [0x13, 0x0D, 0x08], // 29 the fallen trunk, underside
    [0x4E, 0x35, 0x20], // 30 the fallen trunk, lit along the top
    [0xB8, 0x62, 0x1A], // 31 the other bike's wake
    [0xFF, 0xE7, 0xB0], // 32 the other bike
];
const PAL: [u32; 33] = bake(&PAL_RGB);

/// First index of each material's three-per-bucket ramp.
const CANOPY0: u16 = 1;
const GROUND0: u16 = 10;
const HAZE: u16 = 19;
const TRUNK0: u16 = 20;
const LOG_DARK: u16 = 29;
const LOG_LIT: u16 = 30;
const RIDER_WAKE: u16 = 31;
const RIDER: u16 = 32;

/// Metres: eye height, the canopy's underside, and the depth range a trunk
/// lives in before it is recycled.
const CAM_H: f32 = 1.5;
const CANOPY_H: f32 = 28.0;
const Z_NEAR: f32 = 0.9;
const Z_FAR: f32 = 140.0;
/// Beyond this the background is all mist anyway, and an unclamped `z` at the
/// eye line samples the light tile at a spatial frequency that is pure noise.
const Z_HAZE: f32 = 160.0;
/// Trunks stand within this many metres either side of the path.
const CORRIDOR: f32 = 26.0;
const R_MIN: f32 = 1.1;
const R_SPAN: f32 = 2.5;
/// One spawn in this many is aimed at where the camera will BE when it arrives,
/// which is what makes the near miss a property of the code rather than a
/// coincidence the viewer has to wait for.
const NEAR_MISS_IN: u32 = 20;
/// How close that aimed trunk passes: metres of clearance past the bark.
const GRAZE_MIN: f32 = 1.0;
const GRAZE_SPAN: f32 = 2.2;
/// Metres of bark the camera is guaranteed to clear on ANY trunk.
const CLEARANCE: f32 = 2.4;

/// The light tile: 64x64 samples at [`TILE_PER_M`] to the metre, so it wraps
/// every ~47 metres — far enough that nothing reads as a repeat at speed.
const TILE: usize = 64;
const TMASK: i32 = TILE as i32 - 1;
const TILE_PER_M: f32 = 1.35;
const TILE_M: f32 = TILE as f32 / TILE_PER_M;
/// Frequency multiplier for the floor's undergrowth sample. Not an integer, so
/// the fern texture never lines up with the pool of light lying over it.
const DETAIL: f32 = 6.3;

/// Bark: furrow strips across a trunk's diameter, and metres per fibre.
const BARK_STRIPS: f32 = 24.0;
const BARK_SEG: f32 = 0.9;

/// Rows either side of the eye line that are pure mist. Two of them on the real
/// panel: a row next to the horizon is hundreds of metres of forest, and there
/// is nothing honest to draw in it.
const MIST_ROWS: f32 = 1.2;

/// "Not in the scene". NEGATIVE infinity, not positive: the timer that starts
/// the next one only runs once the current one is past the camera, and a
/// sentinel at +inf is never past anything — which is a log and a rider that
/// never appear at all, and no still frame can show you that.
const OFF: f32 = f32::NEG_INFINITY;

const LOG_H: f32 = 2.4;
const LOG_R: f32 = 0.62;
const RIDER_H: f32 = 1.35;
const RIDER_R: f32 = 0.9;

/// Which third of the depth range something is in. Silhouette up close, bark in
/// the middle, hazy grey-green at the back.
#[inline]
fn bucket(z: f32) -> u16 {
    if z < 14.0 {
        0
    } else if z < 45.0 {
        1
    } else {
        2
    }
}

pub struct Speeder {
    grid: Grid,
    cols: usize,
    rows: usize,
    /// Trunk pools, recycled in place as they pass the camera.
    tx: Vec<f32>,
    tz: Vec<f32>,
    tr: Vec<f32>,
    tseed: Vec<u32>,
    /// Where each trunk was on screen last frame — the blur test. NaN for a
    /// trunk that has just respawned, so a recycled trunk never blurs.
    prev_sx: Vec<f32>,
    /// Trunk indices, far to near. Sorted in place every frame: painter's
    /// algorithm is what makes a near trunk hide a far one, and a z-buffer for
    /// a scene of fewer than 400 opaque columns is a buffer to keep in step.
    order: Vec<u32>,
    /// The dappled light, baked once.
    tile: Vec<u8>,
    rng: u32,

    focal: f32,
    horizon: f32,
    cx: f32,
    /// Metres per frame, seconds per frame, and the screen movement past which
    /// a trunk stipples instead of stamping a hard edge.
    dz: f32,
    dt: f32,
    blur_cols: f32,

    /// The weave: two phases advanced per frame and kept in 0..2pi, so a pod up
    /// for a month weaves like one up for a minute. Absolute time in an f32
    /// does not.
    p1: f32,
    p2: f32,
    w1: f32,
    w2: f32,
    weave: f32,
    cam_x: f32,
    /// Distance travelled, wrapped to the light tile — invisible, because the
    /// tile wraps there too, and it keeps f32 precision constant for weeks.
    cam_z: f32,
    /// The canopy's own slow drift across the floor, same wrap.
    drift: f32,

    /// Floor-light thresholds, from `SPEEDER_DAPPLE`.
    lit_at: u8,
    hot_at: u8,

    log_z: f32,
    log_next: f32,
    log_every: f32,
    rider_z: f32,
    rider_x: f32,
    rider_vx: f32,
    rider_next: f32,
    rider_every: f32,
}

impl Speeder {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        Self::build(panel, fps, saver_seed(&["SPEEDER_SEED"], 0x5FEE_D00D))
    }

    /// `new` with the seed handed in. Every test pins a ride this way rather
    /// than through the environment: cargo runs tests in parallel threads and
    /// `set_var` would land one test's seed in another's saver.
    fn build(panel: &Panel, fps: u32, seed: u32) -> Self {
        let cell = env_num(&["SPEEDER_CELL"], 8, 4, 32) as usize;
        let speed = env_num(&["SPEEDER_SPEED"], 58, 10, 300) as f32;
        let trunks = env_num(&["SPEEDER_TRUNKS"], 60, 8, 400) as usize;
        let fov = env_num(&["SPEEDER_FOV"], 62, 20, 200) as f32 / 100.0;
        let horizon_pct = env_num(&["SPEEDER_HORIZON"], 44, 10, 80) as f32 / 100.0;
        let weave = env_num(&["SPEEDER_WEAVE"], 64, 0, 200) as f32 / 10.0;
        let dapple = env_num(&["SPEEDER_DAPPLE"], 34, 0, 100) as f32 / 100.0;
        let log_every = env_num(&["SPEEDER_LOG_SECS"], 16, 0, 600) as f32;
        let rider_every = env_num(&["SPEEDER_RIDER_SECS"], 12, 0, 600) as f32;

        // Square cell: the whole aspect story is in the module doc.
        let grid = Grid::new(panel, cell, cell);
        let (cols, rows) = (grid.cols(), grid.rows());
        let fps = fps.max(1) as f32;

        let mut s = Self {
            cols,
            rows,
            tx: vec![0.0; trunks],
            tz: vec![0.0; trunks],
            tr: vec![0.0; trunks],
            tseed: vec![0; trunks],
            prev_sx: vec![f32::NAN; trunks],
            order: (0..trunks as u32).collect(),
            tile: vec![0; TILE * TILE],
            rng: seed,
            focal: cols as f32 * fov,
            horizon: rows as f32 * horizon_pct,
            cx: cols as f32 / 2.0,
            dz: speed / fps,
            dt: 1.0 / fps,
            // A trunk crossing a sixtieth of the panel in one frame is moving
            // too fast for the eye to hold an edge on.
            blur_cols: cols as f32 / 60.0,
            p1: 0.0,
            p2: 1.7,
            // Incommensurate, so the weave never settles into a beat.
            w1: 0.83 / fps * std::f32::consts::TAU,
            w2: 0.31 / fps * std::f32::consts::TAU,
            weave,
            cam_x: 0.0,
            cam_z: 0.0,
            drift: 0.0,
            lit_at: (255.0 * (1.0 - dapple)) as u8,
            hot_at: (255.0 - 255.0 * dapple * 0.34) as u8,
            log_z: OFF,
            log_next: log_every * 0.4,
            log_every,
            rider_z: OFF,
            rider_x: 0.0,
            rider_vx: 0.0,
            rider_next: rider_every * 0.6,
            rider_every,
            grid,
        };
        s.bake_light();
        for i in 0..trunks {
            // Frame 0 must already be a forest, not an empty corridor with a
            // wall at the far end.
            let z = Z_NEAR + s.unit() * (Z_FAR - Z_NEAR);
            s.respawn_at(i, z);
        }
        s
    }

    /// Four INTEGER harmonics, so the tile wraps seamlessly in both axes: a
    /// non-integer frequency puts a seam across the floor every 47 metres.
    fn bake_light(&mut self) {
        let k = std::f32::consts::TAU / TILE as f32;
        for y in 0..TILE {
            for x in 0..TILE {
                let (fx, fy) = (x as f32 * k, y as f32 * k);
                let v = fx.sin() + (fy * 2.0).sin() * 0.8 - (fx * 3.0 + fy).sin() * 0.6
                    + (fx * 2.0 - fy * 3.0).sin() * 0.45;
                // -2.85..2.85 into 0..=255.
                self.tile[y * TILE + x] = (v * 44.0 + 128.0).clamp(0.0, 255.0) as u8;
            }
        }
    }

    #[inline]
    fn unit(&mut self) -> f32 {
        (next_rand(&mut self.rng) & 0xFFFF) as f32 / 65536.0
    }

    /// Where the weave puts the camera `n` FRAMES from now. The phases advance
    /// by a fixed step per frame, so this is exact rather than an estimate —
    /// which is what lets a spawn be aimed at the camera's future position, and
    /// what makes "never inside a trunk" a guarantee instead of a hope.
    #[inline]
    fn cam_x_at(&self, n: f32) -> f32 {
        let (a, b) = (self.p1 + self.w1 * n, self.p2 + self.w2 * n);
        self.weave * (a.sin() + 0.55 * b.sin()) / 1.55
    }

    fn respawn(&mut self, i: usize) {
        self.respawn_at(i, Z_FAR);
    }

    /// A trunk at depth `z`. One in [`NEAR_MISS_IN`] is aimed to graze; every
    /// other one is pushed out to [`CLEARANCE`] if it would otherwise arrive
    /// where the camera will be. A trunk the camera flies INTO is a panel that
    /// goes solid brown for a second, which is neither a near miss nor a
    /// forest.
    ///
    /// `z` is a parameter because the opening frame's trunks are spread through
    /// the whole depth range: each arrives after its OWN number of frames, and
    /// a clearance computed for the far end's arrival is a clearance against
    /// the wrong place on the weave.
    fn respawn_at(&mut self, i: usize, z: f32) {
        let r = R_MIN + self.unit() * R_SPAN;
        let aimed = next_rand(&mut self.rng).is_multiple_of(NEAR_MISS_IN);
        let side = if next_rand(&mut self.rng) & 1 == 0 {
            -1.0
        } else {
            1.0
        };
        let path = self.cam_x_at((z - Z_NEAR) / self.dz);
        let mut x = if aimed {
            path + side * (r + GRAZE_MIN + self.unit() * GRAZE_SPAN)
        } else {
            (self.unit() * 2.0 - 1.0) * CORRIDOR
        };
        let d = x - path;
        if !aimed && d.abs() < r + CLEARANCE {
            x = path + (r + CLEARANCE) * if d < 0.0 { -1.0 } else { 1.0 };
        }
        self.tx[i] = x;
        self.tr[i] = r;
        self.tz[i] = z;
        self.tseed[i] = next_rand(&mut self.rng) | 1;
        self.prev_sx[i] = f32::NAN;
    }

    fn step(&mut self) {
        self.p1 = (self.p1 + self.w1) % std::f32::consts::TAU;
        self.p2 = (self.p2 + self.w2) % std::f32::consts::TAU;
        self.cam_x = self.cam_x_at(0.0);
        self.cam_z = (self.cam_z + self.dz) % TILE_M;
        self.drift = (self.drift + self.dt * 0.9) % TILE_M;

        for i in 0..self.tz.len() {
            self.tz[i] -= self.dz;
            if self.tz[i] <= Z_NEAR {
                self.respawn(i);
            }
        }
        let Self { order, tz, .. } = self;
        order.sort_unstable_by(|&a, &b| tz[b as usize].total_cmp(&tz[a as usize]));

        self.log_z -= self.dz;
        if self.log_z <= Z_NEAR {
            self.log_z = OFF;
            self.log_next -= self.dt;
            if self.log_every > 0.0 && self.log_next <= 0.0 {
                self.log_z = Z_FAR;
                self.log_next = self.log_every * (0.5 + self.unit());
            }
        }

        self.rider_z -= self.dz;
        self.rider_x += self.rider_vx * self.dt;
        if self.rider_z <= Z_NEAR || (self.rider_x - self.cam_x).abs() > CORRIDOR * 2.0 {
            self.rider_z = OFF;
            self.rider_next -= self.dt;
            if self.rider_every > 0.0 && self.rider_next <= 0.0 {
                let side = if next_rand(&mut self.rng) & 1 == 0 {
                    -1.0
                } else {
                    1.0
                };
                // In from one side, ahead, crossing fast enough to be a flash
                // rather than a companion.
                self.rider_z = 24.0 + self.unit() * 40.0;
                self.rider_x = self.cam_x + side * CORRIDOR;
                self.rider_vx = -side * (34.0 + self.unit() * 26.0);
                self.rider_next = self.rider_every * (0.5 + self.unit());
            }
        }
    }

    /// One sample of the light tile at a world position.
    #[inline]
    fn light(tile: &[u8], wx: f32, wz: f32) -> u8 {
        let ix = ((wx * TILE_PER_M) as i32) & TMASK;
        let iz = ((wz * TILE_PER_M) as i32) & TMASK;
        tile[iz as usize * TILE + ix as usize]
    }

    /// Floor and canopy in one pass: a row is a depth, a cell is a position
    /// along it. Everything else is painted over this.
    fn paint_background(&mut self) {
        let (cols, rows) = (self.cols, self.rows);
        let (focal, horizon, cx) = (self.focal, self.horizon, self.cx);
        let (cam_x, cam_z, drift) = (self.cam_x, self.cam_z, self.drift);
        let (lit_at, hot_at) = (self.lit_at, self.hot_at);
        let Self { grid, tile, .. } = self;

        for r in 0..rows {
            let d = r as f32 + 0.5 - horizon;
            if d.abs() < MIST_ROWS {
                for c in 0..cols {
                    grid.set(r * cols + c, Cell::new(font::SHADE, HAZE));
                }
                continue;
            }
            let (z, ramp) = if d < 0.0 {
                ((CANOPY_H - CAM_H) * focal / -d, CANOPY0)
            } else {
                (CAM_H * focal / d, GROUND0)
            };
            let z = z.min(Z_HAZE);
            let ramp = ramp + bucket(z) * 3;
            // Metres per column at this depth, so the per-CELL work is one
            // multiply-add rather than a divide.
            let per_col = z / focal;
            let ground = d > 0.0;
            let wz = cam_z + z + if ground { -drift } else { drift };
            let mut wx = (0.5 - cx) * per_col + cam_x + drift;
            // The floor gets a second sample of the same tile at [`DETAIL`]
            // times the frequency: underfoot, one row spans a couple of metres
            // and the pools alone leave it a flat wash. The canopy does not —
            // up there the eye is reading the shape of the gaps.
            let wzd = wz * DETAIL;
            let mut wxd = wx * DETAIL;
            let per_col_d = per_col * DETAIL;
            for c in 0..cols {
                let v = Self::light(tile, wx, wz);
                let g = if ground {
                    Self::light(tile, wxd, wzd)
                } else {
                    v
                };
                wx += per_col;
                wxd += per_col_d;
                let (glyph, shade) = if v >= hot_at {
                    (font::SOLID, 2)
                } else if v >= lit_at {
                    (font::SHADE, 2)
                } else if g & 0x40 != 0 {
                    (font::SHADE, 1)
                } else if g & 0x20 != 0 {
                    (font::SOLID, 1)
                } else {
                    (font::SOLID, 0)
                };
                grid.set(r * cols + c, Cell::new(glyph, ramp + shade));
            }
        }
    }

    /// Trunks far to near, with the fallen log and the other bike dropped into
    /// the same ordering — a log in front of a trunk has to hide it.
    fn paint_scene(&mut self) {
        let (mut log, mut rider) = (false, false);
        for k in 0..self.order.len() {
            let i = self.order[k] as usize;
            let z = self.tz[i];
            if !log && self.log_z > z {
                log = true;
                self.paint_log();
            }
            if !rider && self.rider_z > z {
                rider = true;
                self.paint_rider();
            }
            self.paint_trunk(i);
        }
        if !log {
            self.paint_log();
        }
        if !rider {
            self.paint_rider();
        }
    }

    /// A half-open span of rows or columns, clamped to the frame.
    #[inline]
    fn span(a: f32, b: f32, n: usize) -> (usize, usize) {
        let lo = a.clamp(0.0, n as f32) as usize;
        let hi = b.clamp(0.0, n as f32) as usize;
        (lo, hi)
    }

    fn paint_trunk(&mut self, i: usize) {
        let z = self.tz[i];
        let half = self.tr[i] * self.focal / z;
        let sx = self.cx + (self.tx[i] - self.cam_x) * self.focal / z;
        // NaN on the frame after a respawn, and NaN fails this compare — which
        // is exactly the "do not blur a trunk that teleported" answer.
        let blur = (sx - self.prev_sx[i]).abs() > self.blur_cols;
        self.prev_sx[i] = sx;
        if sx + half < 0.0 || sx - half > self.cols as f32 {
            return;
        }

        let ramp = TRUNK0 + bucket(z) * 3;
        let base = self.horizon + CAM_H * self.focal / z;
        let top = self.horizon - (CANOPY_H - CAM_H) * self.focal / z;
        let (r0, r1) = Self::span(top, base, self.rows);
        // The buttressed foot: a redwood is not a post, and the flare is what
        // plants it on the ground rather than merely ending at it.
        // Never more than a third of what is visible of THIS trunk: a flare
        // sized only off the width climbs eighteen rows up a distant stub and
        // turns it into a cone standing in the canopy.
        let flare = (half * 0.8)
            .clamp(1.0, self.rows as f32 * 0.14)
            .min((base - top) * 0.35);
        let seed = self.tseed[i];
        // Bark lives on the trunk: columns are metres across it, rows metres up
        // it, so the grain slides and swells with the trunk. Strips are a
        // fixed share of the diameter; once one is narrower than a cell and a
        // half it would alias into shimmer, so a distant trunk goes plain.
        let m_per_cell = z / self.focal;
        let per_strip = m_per_cell * BARK_STRIPS / (2.0 * self.tr[i]);
        let textured = 2.0 * half / BARK_STRIPS >= 1.5;
        for r in r0..r1 {
            let seg = (CAM_H + (self.horizon - r as f32 - 0.5) * m_per_cell) / BARK_SEG;
            let up = base - r as f32;
            let hw = if up < flare {
                half * (1.0 + 0.45 * (1.0 - up / flare))
            } else {
                half
            };
            let (c0, c1) = Self::span(sx - hw, sx + hw + 1.0, self.cols);
            let inv = 0.5 / hw;
            for c in c0..c1 {
                // Across the trunk: 0 on the lit edge, 1 on the shadow side.
                let p = (c as f32 + 0.5 - sx + hw) * inv;
                let shade = if p < 0.22 {
                    2
                } else if p > 0.70 {
                    0
                } else {
                    1
                };
                // Bark grain: without it a near trunk is a flat bar the width
                // of the panel.
                let grain = textured && Self::grain(seed, (c as f32 + 0.5 - sx) * per_strip, seg);
                let glyph = if blur || grain {
                    font::SHADE
                } else {
                    font::SOLID
                };
                self.grid
                    .set(r * self.cols + c, Cell::new(glyph, ramp + shade));
            }
        }
    }

    /// Whether the bark at `u` strips across and `seg` segments up is a
    /// furrow. Segments are staggered per strip so the grain runs in vertical
    /// fibres rather than a brick wall.
    #[inline]
    fn grain(seed: u32, u: f32, seg: f32) -> bool {
        let h = seed ^ (u.floor() as i32 as u32).wrapping_mul(0x9E37_79B9);
        let stagger = (h >> 8 & 0xFF) as f32 / 256.0;
        let mut h = h ^ ((seg + stagger).floor() as i32 as u32).wrapping_mul(0x85EB_CA6B);
        next_rand(&mut h) & 3 == 0
    }

    fn paint_log(&mut self) {
        let z = self.log_z;
        if !z.is_finite() {
            return;
        }
        let row = |h: f32| self.horizon - (h - CAM_H) * self.focal / z;
        let (r0, r1) = Self::span(row(LOG_H + LOG_R), row(LOG_H - LOG_R) + 1.0, self.rows);
        for r in r0..r1 {
            let colour = if r == r0 { LOG_LIT } else { LOG_DARK };
            for c in 0..self.cols {
                self.grid
                    .set(r * self.cols + c, Cell::new(font::SOLID, colour));
            }
        }
    }

    fn paint_rider(&mut self) {
        let z = self.rider_z;
        if !z.is_finite() {
            return;
        }
        let half = (RIDER_R * self.focal / z).clamp(0.6, self.cols as f32 * 0.09);
        let sx = self.cx + (self.rider_x - self.cam_x) * self.focal / z;
        let sy = self.horizon - (RIDER_H - CAM_H) * self.focal / z;
        // The wake is behind it, which is the side it came from.
        let tail = if self.rider_vx > 0.0 { -1.0 } else { 1.0 } * half * 7.0;
        let (wr0, wr1) = Self::span(sy - half * 0.25, sy + half * 0.25 + 1.0, self.rows);
        let (w0, w1) = Self::span(sx.min(sx + tail), sx.max(sx + tail), self.cols);
        for r in wr0..wr1 {
            for c in w0..w1 {
                self.grid
                    .set(r * self.cols + c, Cell::new(font::SHADE, RIDER_WAKE));
            }
        }
        let (r0, r1) = Self::span(sy - half * 0.45, sy + half * 0.45 + 1.0, self.rows);
        let (c0, c1) = Self::span(sx - half, sx + half + 1.0, self.cols);
        for r in r0..r1 {
            for c in c0..c1 {
                self.grid
                    .set(r * self.cols + c, Cell::new(font::SOLID, RIDER));
            }
        }
    }
}

impl Saver for Speeder {
    fn render(&mut self, s: &mut Surface<'_>) {
        self.step();
        self.paint_background();
        self.paint_scene();
        self.grid.flush(s, &PAL);
    }

    fn name(&self) -> &'static str {
        "speeder"
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

    /// The real panel's framebuffer, and a height that is NOT a multiple of the
    /// cell, so the strip below the last row is in every one of these runs.
    fn panel() -> Panel {
        Panel::new(1920, 1070, 1920)
    }

    fn speeder(aspect: usize) -> Speeder {
        with_test_aspect(aspect, || Speeder::build(&panel(), 30, 0xC0FF_EE11))
    }

    /// Colour index -> what it is. These tests assert on MATERIALS rather than
    /// on palette numbers: a ramp that moved would otherwise pass silently.
    fn material(c: usize) -> &'static str {
        match c as u16 {
            0 => "black",
            i if i < GROUND0 => "canopy",
            i if i < HAZE => "floor",
            HAZE => "mist",
            i if i < LOG_DARK => "trunk",
            LOG_DARK | LOG_LIT => "log",
            _ => "rider",
        }
    }

    fn counts(s: &Speeder) -> std::collections::HashMap<&'static str, usize> {
        let mut m = std::collections::HashMap::new();
        for cell in s.grid.cells() {
            *m.entry(material(cell.colour())).or_insert(0) += 1;
        }
        m
    }

    /// `new` is the door the panel comes through and the only one these tests
    /// do not otherwise use — everything else pins a seed through `build`. A
    /// `new` wired to the wrong knob is a saver that ships broken and passes
    /// every other test in this file.
    #[test]
    fn new_seeds_itself_and_draws_a_forest() {
        let p = panel();
        let mut s = Speeder::new(&p, 30);
        assert_eq!(s.name(), "speeder");
        assert_eq!(s.palette().len(), PAL.len());
        let mut buf = vec![0u32; p.buf_len()];
        assert_eq!(saver::frame(&mut s, &mut buf, &p).rows(), p.h);
        assert!(counts(&s).contains_key("trunk"), "no forest");
    }

    /// Frame 0 must cover the panel — including the strip no cell owns — and
    /// cover it with a FOREST. A reported black rectangle satisfies the damage
    /// assertion on its own.
    #[test]
    fn frame_zero_paints_the_whole_panel_at_both_aspects() {
        for aspect in [100, 180] {
            let p = panel();
            let mut s = speeder(aspect);
            let mut buf = vec![0u32; p.buf_len()];
            let d = saver::frame(&mut s, &mut buf, &p);
            assert_eq!(
                d.rows(),
                p.h,
                "aspect {aspect}: frame 0 left rows unpainted"
            );
            let lit = buf.iter().filter(|&&px| px != 0).count();
            assert!(
                lit > p.w * p.h / 2,
                "aspect {aspect}: frame 0 is mostly black ({lit} lit)"
            );
        }
    }

    /// The simpledrm contract: a pixel that changed and is in no reported rect
    /// shows a stale frame forever, and no framebuffer diff can see it. Run at
    /// BOTH aspects, because the row count is what changes between them.
    #[test]
    fn every_changed_pixel_is_reported_at_both_aspects() {
        for aspect in [100, 180] {
            let p = panel();
            let mut s = speeder(aspect);
            let mut buf = vec![0u32; p.buf_len()];
            let mut prev = buf.clone();
            for n in 0..200 {
                prev.copy_from_slice(&buf);
                let d = saver::frame(&mut s, &mut buf, &p);
                dump::verify(&prev, &buf, &d, &p, n)
                    .unwrap_or_else(|e| panic!("aspect {aspect}: {e}"));
            }
        }
    }

    /// The frame loop is the product. `order` is sorted in place, the trunk
    /// pools are recycled rather than grown and the light tile is baked once —
    /// so one allocation here is a malloc per frame on a 500m budget.
    #[test]
    fn render_never_allocates() {
        let p = panel();
        let mut s = speeder(100);
        let mut buf = vec![0u32; p.buf_len()];
        saver::frame(&mut s, &mut buf, &p);
        // Long enough that every trunk has been recycled many times over: a
        // respawn is where an allocation would hide.
        let n = crate::testalloc::allocs_during(|| {
            for _ in 0..2_000 {
                saver::frame(&mut s, &mut buf, &p);
            }
        });
        assert_eq!(n, 0, "the render path allocated {n} times");
    }

    /// Perspective is derived from SQUARE-pixel units — cells, which
    /// `SAVER_PIXEL_ASPECT` keeps square on the glass. So the same trunk
    /// projects to the same COLUMNS at either aspect, and the vertical geometry
    /// follows `rows`, which nearly halves at 180. Asserting scalars here would
    /// pass on a saver that drew nothing at all at 180.
    #[test]
    fn the_projection_is_square_pixel_derived() {
        let (a, b) = (speeder(100), speeder(180));
        assert_eq!(a.cols, b.cols, "the column count is the aspect-free axis");
        // 1.8x taller cells, rounded: 133 rows becomes 76.
        assert!(b.rows * 3 < a.rows * 2, "180 must cut the rows down");
        assert_eq!(a.focal, b.focal, "focal is in columns, so it cannot move");

        // The eye line is a fraction of ROWS, not a pixel count.
        for s in [&a, &b] {
            assert!(
                (s.horizon - s.rows as f32 * 0.44).abs() < 0.01,
                "the horizon is not derived from rows ({} of {})",
                s.horizon,
                s.rows
            );
        }
        // And the ground plane keeps its shape: from the eye line to a trunk's
        // foot at a fixed depth is the same number of CELLS at either aspect,
        // which is the whole claim the square cell makes.
        for z in [5.0f32, 20.0, 80.0] {
            let drop = |s: &Speeder| CAM_H * s.focal / z;
            assert_eq!(drop(&a), drop(&b), "the ground plane moved at z={z}");
        }
    }

    /// What speeder IS: trunks arriving fast and being dodged by a margin that
    /// looks too small. Both halves are asserted — one that merely came close
    /// without ever filling the frame is a distant forest, and one that filled
    /// the frame without the camera clearing it is a crash.
    #[test]
    fn trunks_graze_the_camera_and_blur_past() {
        let p = panel();
        let mut s = speeder(100);
        let mut buf = vec![0u32; p.buf_len()];
        let (mut widest, mut closest, mut blurred) = (0.0f32, f32::MAX, 0);
        let mut smeared = 0.0f32;
        for _ in 0..900 {
            // `paint_trunk` overwrites `prev_sx` with this frame's position, so
            // last frame's has to be taken before the frame, not after.
            let was = s.prev_sx.clone();
            saver::frame(&mut s, &mut buf, &p);
            for (i, &prev) in was.iter().enumerate() {
                let z = s.tz[i];
                let sx = s.cx + (s.tx[i] - s.cam_x) * s.focal / z;
                let half = s.tr[i] * s.focal / z;
                // Only a trunk actually ON the panel is a near miss.
                if sx + half > 0.0 && sx - half < s.cols as f32 {
                    widest = widest.max(2.0 * half / s.cols as f32);
                }
                // Measured on the LAST frame before the trunk is recycled,
                // which is the closest the camera ever gets to it. A lateral
                // gap taken at any other depth says nothing: a trunk dead
                // ahead at 100 metres is scenery, not a collision.
                if z < Z_NEAR + s.dz {
                    closest = closest.min((s.tx[i] - s.cam_x).abs() - s.tr[i]);
                }
                // The stipple that reads as motion blur: the same test the
                // paint makes, screen movement past a sixtieth of the panel.
                if (sx - prev).abs() > s.blur_cols {
                    blurred += 1;
                }
            }
            let (mut bark, mut stipple) = (0.0f32, 0.0f32);
            for cell in s.grid.cells() {
                if material(cell.colour()) == "trunk" {
                    bark += 1.0;
                    stipple += f32::from(cell.glyph() == font::SHADE as usize);
                }
            }
            if bark > 100.0 {
                smeared = smeared.max(stipple / bark);
            }
        }
        assert!(
            widest > 0.3,
            "no trunk ever filled a third of the frame (widest {widest:.2})"
        );
        assert!(
            closest > 0.0,
            "the camera flew through a trunk ({closest:.2} m inside the bark)"
        );
        // Inside the clearance floor, which is where only an AIMED spawn can
        // put a trunk: without the prediction every trunk is pushed out to
        // `CLEARANCE` and the closest pass is exactly that, forever.
        assert!(
            closest < CLEARANCE,
            "nothing was ever aimed at the camera's path (closest {closest:.2} m)"
        );
        assert!(blurred > 500, "trunks never blurred past ({blurred})");
        // And the blur has to reach the PANEL. Bark grain stipples about a
        // quarter of a near trunk's cells; a frame where most of the bark on screen
        // is stipple is a trunk being smeared by its own parallax, and nothing
        // else in this saver can produce one.
        assert!(
            smeared > 0.5,
            "no frame ever smeared a trunk ({smeared:.2} of its cells at best)"
        );
    }

    /// Every frame is a forest: floor below, trunks over it, and all three
    /// trunk depth ramps in use. A saver that drew one wall of bark, or one
    /// flat wash of green, passes every damage test in this file.
    #[test]
    fn every_frame_has_a_floor_and_trunks_at_three_depths() {
        let p = panel();
        let mut s = speeder(100);
        let mut buf = vec![0u32; p.buf_len()];
        let cells = s.cols * s.rows;
        let mut seen = [false; 3];
        let mut crowned = 0;
        for n in 0..300 {
            saver::frame(&mut s, &mut buf, &p);
            let m = counts(&s);
            // A far trunk ends UNDER the foliage rather than running off the
            // top of the frame; without that the forest has no ceiling, only
            // columns. The probe is a FLAT crown — three neighbouring columns
            // whose topmost bark sits on the same row, below the top of the
            // frame. A diagonal of trunk cells is the flared foot, which rises
            // above the eye line on a distant stub and would otherwise answer
            // this question with the wrong feature.
            let top_of = |c: usize| {
                (0..s.rows).find(|r| material(s.grid.cells()[r * s.cols + c].colour()) == "trunk")
            };
            crowned += usize::from((1..s.cols - 1).any(|c| {
                matches!(top_of(c), Some(r) if r > 1 && top_of(c - 1) == Some(r) && top_of(c + 1) == Some(r))
            }));
            for what in ["floor", "trunk"] {
                assert!(
                    m.get(what).copied().unwrap_or(0) > cells / 50,
                    "frame {n}: no {what} ({m:?})"
                );
            }
            for cell in s.grid.cells() {
                if material(cell.colour()) == "trunk" {
                    seen[(cell.colour() - TRUNK0 as usize) / 3] = true;
                }
            }
        }
        // The canopy is only visible where trunks do not cover it, so depth is
        // a run-wide claim rather than a per-frame one.
        assert!(
            seen.iter().all(|&b| b),
            "trunks drew at one depth: {seen:?}"
        );
        assert!(
            crowned > 150,
            "no trunk ever ended under the canopy ({crowned})"
        );
    }

    /// Depth through layers is painter's algorithm, and painter's algorithm is
    /// the sort: the NEAR trunk has to be the one you see where two overlap.
    /// Reversed, every frame still has trunks at three depths and still reports
    /// its damage — it just draws the forest inside out.
    #[test]
    fn a_near_trunk_hides_the_far_one_behind_it() {
        let p = panel();
        let mut s = speeder(100);
        let mut buf = vec![0u32; p.buf_len()];
        let mut checked = 0;
        for _ in 0..400 {
            saver::frame(&mut s, &mut buf, &p);
            // The nearest trunk standing wholly on the panel.
            let Some(near) = (0..s.tz.len())
                .filter(|&i| {
                    let sx = s.cx + (s.tx[i] - s.cam_x) * s.focal / s.tz[i];
                    let half = s.tr[i] * s.focal / s.tz[i];
                    s.tz[i] < 12.0 && sx - half > 1.0 && sx + half < s.cols as f32 - 1.0
                })
                .min_by(|&a, &b| s.tz[a].total_cmp(&s.tz[b]))
            else {
                continue;
            };
            let sx = s.cx + (s.tx[near] - s.cam_x) * s.focal / s.tz[near];
            let half = s.tr[near] * s.focal / s.tz[near];
            let want = bucket(s.tz[near]);
            // Halfway up it: clear of the flared foot, and of the log and the
            // other rider, which are allowed to be in front.
            let row = s.rows / 3;
            for c in (sx - half + 1.0) as usize..(sx + half - 1.0) as usize {
                let cell = s.grid.cells()[row * s.cols + c];
                if material(cell.colour()) != "trunk" {
                    continue;
                }
                checked += 1;
                assert_eq!(
                    (cell.colour() as u16 - TRUNK0) / 3,
                    want,
                    "a farther trunk was painted over the near one at row {row}, col {c}"
                );
            }
        }
        assert!(
            checked > 1000,
            "never saw a near trunk to check ({checked})"
        );
    }

    /// Dappled light moving over the ground is the signature. It has to MOVE,
    /// and it has to move because the WORLD moved: the floor is sampled in
    /// world coordinates, where a version sampling in screen space would hold
    /// still under the camera and pass every damage test unchanged.
    #[test]
    fn the_dappled_light_streams_over_the_floor() {
        let p = panel();
        let mut s = speeder(100);
        let mut buf = vec![0u32; p.buf_len()];
        // Well below the eye line, so no trunk foot reaches it and the sample
        // is floor and only floor.
        let row = s.rows - 2;
        let lit = |s: &Speeder| -> Vec<bool> {
            (0..s.cols)
                .map(|c| {
                    let cell = s.grid.cells()[row * s.cols + c];
                    material(cell.colour()) == "floor"
                        && (cell.colour() - GROUND0 as usize) % 3 == 2
                })
                .collect()
        };
        saver::frame(&mut s, &mut buf, &p);
        let mut prev = lit(&s);
        let (mut moved, mut ever_lit, mut varied) = (0, false, 0);
        for _ in 0..120 {
            saver::frame(&mut s, &mut buf, &p);
            let now = lit(&s);
            ever_lit |= now.iter().any(|&b| b);
            moved += now.iter().zip(&prev).filter(|(a, b)| a != b).count();
            // The row has to have a PATTERN across it as well as over time. A
            // projection that lost its per-column term paints each row one flat
            // colour and still passes every "it changed" assertion.
            let floor: Vec<Cell> = (0..s.cols)
                .map(|c| s.grid.cells()[row * s.cols + c])
                .filter(|c| material(c.colour()) == "floor")
                .collect();
            varied += usize::from(floor.iter().any(|c| *c != floor[0]));
            prev = now;
        }
        assert!(ever_lit, "no pool of light ever fell on the floor");
        assert!(
            varied > 100,
            "the floor is flat bands, not dapple ({varied}/120)"
        );
        // At 58 m/s the pattern crosses this row several times over; the
        // canopy's own drift is a 60th of that, so a threshold this high is
        // the difference between "the world moved" and "the breeze did".
        assert!(
            moved > 10 * s.cols,
            "the light barely moved over the floor ({moved})"
        );
    }

    /// The bark belongs to the trunk. Slide the camera so one trunk moves an
    /// exact number of columns and its grain has to come along cell for cell;
    /// grain hashed from screen cells stays where it was, and matches the moved
    /// trunk only by chance, three cells in four.
    #[test]
    fn the_bark_travels_with_its_trunk() {
        let mut s = speeder(100);
        let k = 7;
        // Every other trunk out of frame, so only trunk 0 is painted.
        s.tx.iter_mut().for_each(|x| *x = 1e4);
        (s.tx[0], s.tz[0], s.tr[0]) = (0.0, 6.0, 2.0);
        let paint = |s: &mut Speeder, cam_x: f32| {
            s.cam_x = cam_x;
            s.prev_sx[0] = f32::NAN;
            s.paint_background();
            s.paint_trunk(0);
            s.grid.settle();
            s.grid.cells().to_vec()
        };
        let a = paint(&mut s, 0.0);
        let shift = k as f32 * s.tz[0] / s.focal;
        let b = paint(&mut s, shift);
        let (mut same, mut total, mut stippled) = (0, 0, 0);
        // Above the flare, and clear of the trunk's edges by more than `k`.
        let sx = s.cx as usize;
        let half = (s.tr[0] * s.focal / s.tz[0]) as usize;
        for r in 0..s.rows / 2 {
            for c in sx - half + k + 2..sx + half - 2 {
                let (now, was) = (b[r * s.cols + c - k], a[r * s.cols + c]);
                assert_eq!(material(was.colour()), "trunk");
                total += 1;
                same += usize::from(now == was);
                stippled += usize::from(was.glyph() == font::SHADE as usize);
            }
        }
        assert!(
            stippled * 20 > total,
            "a near trunk has no grain ({stippled}/{total})"
        );
        assert!(
            same * 100 >= total * 99,
            "the bark stayed on the screen while the trunk moved ({same}/{total})"
        );
    }

    /// The two events. Both are rare by design, which is the only reason this
    /// can tell "every few seconds" from "never".
    #[test]
    fn a_fallen_trunk_and_another_rider_cross_the_view() {
        let p = panel();
        let mut s = speeder(100);
        let mut buf = vec![0u32; p.buf_len()];
        let (mut log, mut rider) = (0, 0);
        // 900 frames is 30 seconds: about two of each at the defaults.
        for _ in 0..900 {
            saver::frame(&mut s, &mut buf, &p);
            let m = counts(&s);
            log += usize::from(m.contains_key("log"));
            rider += usize::from(m.contains_key("rider"));
        }
        assert!(log > 0, "no fallen trunk in 30 seconds");
        assert!(rider > 0, "no other bike in 30 seconds");
        // And neither may become the scenery.
        assert!(log < 450 && rider < 450, "log {log} / rider {rider} frames");
    }

    /// The seed knob is not a convenience: without it this saver cannot be
    /// dumped and compared frame-for-frame against another build, which is how
    /// every rendering change in this tree is shown to be a no-op.
    #[test]
    fn the_seed_pins_the_ride() {
        let p = panel();
        let frames = |seed: u32| {
            let mut s = Speeder::build(&p, 30, seed);
            let mut buf = vec![0u32; p.buf_len()];
            for _ in 0..40 {
                saver::frame(&mut s, &mut buf, &p);
            }
            s.grid.cells().to_vec()
        };
        assert_eq!(frames(7), frames(7), "one seed drew two different rides");
        assert_ne!(frames(7), frames(8), "two seeds drew the same ride");
    }
}
