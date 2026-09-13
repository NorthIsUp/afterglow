use crate::font;
use crate::grid::{bake, pixel_aspect, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, next_rand, saver_seed};

/// Ochre, rust and bleached bone. Nothing here is green and nothing is blue:
/// a frame of this must not be recolourable into a forest or into space.
///
/// Each material is a six-step ramp ordered NEAR to FAR, so aerial haze is
/// `+1` and proximity is `-1` — the ramps are addressed by `depth_ix`, which
/// only ever produces that direction.
#[rustfmt::skip]
const PAL_RGB: [[u8; 3]; 33] = [
    [0x00, 0x00, 0x00], //  0 nothing (a blank cell; no glyph here ever uses it)
    // sky, top of frame down to the horizon
    [0xA8, 0x60, 0x36], //  1
    [0xC2, 0x7A, 0x42], //  2
    [0xD6, 0x97, 0x55], //  3
    [0xE6, 0xB4, 0x70], //  4
    [0xF2, 0xD2, 0x99], //  5
    [0xFC, 0xEE, 0xCE], //  6 bleached bone at the horizon
    // desert floor, near to far
    [0x6B, 0x46, 0x22], //  7
    [0x82, 0x59, 0x2C], //  8
    [0x9B, 0x70, 0x3A], //  9
    [0xB4, 0x8A, 0x50], // 10
    [0xCB, 0xA7, 0x6D], // 11
    [0xDE, 0xC4, 0x93], // 12
    // left wall: away from the sun, near to far
    [0x3E, 0x22, 0x16], // 13
    [0x56, 0x30, 0x1E], // 14
    [0x6E, 0x42, 0x28], // 15
    [0x8A, 0x5C, 0x3A], // 16
    [0xA8, 0x7C, 0x55], // 17
    [0xC6, 0xA2, 0x79], // 18
    // right wall: lit, near to far
    [0x7A, 0x36, 0x1C], // 19
    [0x96, 0x4A, 0x26], // 20
    [0xB0, 0x62, 0x32], // 21
    [0xC6, 0x80, 0x48], // 22
    [0xD8, 0xA0, 0x68], // 23
    [0xE6, 0xC2, 0x94], // 24
    // engine
    [0x1C, 0x18, 0x16], // 25 shadow / bell throat / cable
    [0x39, 0x31, 0x2A], // 26 casing
    [0x6E, 0x5A, 0x3E], // 27 lit bronze
    [0xB6, 0xA2, 0x80], // 28 rim
    // exhaust
    [0xB4, 0x28, 0x10], // 29
    [0xF0, 0x6C, 0x12], // 30
    [0xFF, 0xC0, 0x3A], // 31
    [0xFF, 0xF6, 0xDC], // 32 core
];
const PAL: [u32; 33] = bake(&PAL_RGB);

const SKY0: u16 = 1;
const GND0: u16 = 7;
const ROCK_L: u16 = 13;
const ROCK_R: u16 = 19;
const ENG_DARK: u16 = 25;
const ENG_BODY: u16 = 26;
const ENG_LIT: u16 = 27;
const ENG_RIM: u16 = 28;
const FLARE0: u16 = 29;
/// Steps in EVERY material ramp, sky included. The sky shipped one step short
/// of this for an afternoon and its darkest step addressed the floor's — which
/// every geometry test passed and `the_sky_is_above_the_rim_and_never_below_it`
/// caught on frame 0.
const RAMP: usize = 6;

/// The course repeats every this many metres, and `s` is kept inside it. An
/// f32 that counted metres forever would lose its fraction inside an hour and
/// the canyon would go straight; every wave frequency below is an integer
/// number of cycles per loop, so the wrap is seamless.
const LOOP_M: f32 = 4096.0;
/// Wall hits are searched out to here; past it the ray is treated as open
/// desert and the column is floor and sky only.
const Z_FAR: f32 = 900.0;
/// Camera eye height above the floor, metres. A podracer sits low.
const CAM_H: f32 = 2.6;
/// Canyon rim height, metres, before the per-metre raggedness below. Low
/// enough that the far rim falls below the top of the frame: a wall that
/// always fills the frame is a texture, and the strip of bleached sky over the
/// rim is what makes it a canyon.
const WALL_H: f32 = 19.0;
/// Metres ahead the pilot aims at. Short: this is a twitchy vehicle.
const LOOK: f32 = 26.0;
/// Distance at which a material reaches the hazy far end of its ramp. Rock is
/// the near one because a wall you can see is a wall you are about to pass.
const ROCK_SPAN: f32 = 170.0;
const GND_SPAN: f32 = 340.0;
/// Geometric ratio of the depth march, and the bisections that refine the hit.
const MARCH: f32 = 1.11;
const BISECT: usize = 3;
/// Exhaust halo, as a multiple of the bell radius. It is drawn as SHADE — a
/// dither against black — so it has to stay a rim: at 1.35 it was a solid disc
/// that ate two thirds of the panel.
const HALO: f32 = 1.16;

/// A `sin` table indexed in TURNS, not radians — every caller here has a
/// frequency in cycles per metre or per second, so radians would be a `2 * PI`
/// at every call site and a rounding difference between them.
const WAVE_N: usize = 256;

/// An arch spans the canyon and has a bore you fly through; a spire is a rock
/// pillar standing in the middle of it. Both are the same five numbers, which
/// is why they share a pool.
const ARCH: u8 = 0;
const SPIRE: u8 = 1;

pub struct Podracer {
    grid: Grid,
    rng: u32,
    /// Distance along the course, wrapped into `LOOP_M`.
    s: f32,
    /// Metres per frame, and seconds per frame.
    ds: f32,
    dt: f32,
    t: f32,
    /// Lateral position and heading (dx per metre of course) of the pod.
    cam_x: f32,
    hdg: f32,
    /// Per-engine lateral swing, lagging the heading at its own rate — the two
    /// engines are on separate cables and do not move together.
    eng_x: [f32; 2],
    /// Course-wave phases, rolled from the seed so a different seed is a
    /// different route rather than the same canyon shifted.
    ph: [f32; 3],

    /// Feature pool: arches and spires, recycled to the far end as they pass.
    /// `f_z` is distance AHEAD, not an absolute course position, so nothing
    /// here has to care that `s` wraps.
    f_kind: Vec<u8>,
    f_z: Vec<f32>,
    f_x: Vec<f32>,
    f_w: Vec<f32>,
    f_h: Vec<f32>,
    /// Painter's order, far to near. Sorted in place; never reallocated.
    f_order: Vec<u8>,

    /// A rival's engine wash: hot blobs drifting across the view.
    w_z: Vec<f32>,
    w_x: Vec<f32>,
    w_y: Vec<f32>,
    w_vx: Vec<f32>,
    w_life: Vec<f32>,
    /// Frames until the next wash crosses. 0 disables it entirely.
    wash_in: f32,
    wash_secs: f32,

    /// One ray per cell column: depth of the wall hit, which side it was, and
    /// how tall the rim is there.
    hit_z: Vec<f32>,
    hit_side: Vec<u8>,
    hit_h: Vec<f32>,
    /// Per cell row: the floor depth under that row, and its haze ramp step.
    row_z: Vec<f32>,
    row_ix: Vec<u8>,

    sin: Vec<f32>,

    // Geometry, all in square-pixel units.
    w: f32,
    h: f32,
    f: f32,
    fy: f32,
    /// Horizon in pixels INCLUDING this frame's pitch bob.
    hz: f32,
    horizon: f32,
    yk: f32,
    half_w: f32,
    pinch: f32,
    spread: f32,
    eng_r: f32,
    shimmer: f32,
}

impl Podracer {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        let cell = env_num(&["PODRACER_CELL"], 8, 4, 32) as usize;
        let speed = env_num(&["PODRACER_SPEED"], 300, 40, 900) as f32;
        let fov = env_num(&["PODRACER_FOV"], 78, 30, 200) as f32 / 100.0;
        let half_w = env_num(&["PODRACER_WIDTH"], 30, 6, 90) as f32;
        let pinch = env_num(&["PODRACER_PINCH"], 64, 0, 90) as f32 / 100.0;
        let spread = env_num(&["PODRACER_SPREAD"], 46, 10, 90) as f32 / 100.0;
        let eng_r = env_num(&["PODRACER_ENGINE"], 7, 3, 30) as f32 / 100.0;
        let shimmer = env_num(&["PODRACER_SHIMMER"], 70, 0, 100) as f32 / 100.0;
        let features = env_num(&["PODRACER_FEATURES"], 7, 0, 24) as usize;
        let wash_secs = env_num(&["PODRACER_WASH_SECS"], 9, 0, 600) as f32;

        let grid = Grid::new(panel, cell, cell);
        let (cols, rows) = (grid.cols(), grid.rows());
        let (w, h) = (panel.w as f32, panel.h as f32);
        let yk = pixel_aspect() as f32 / 100.0;
        let f = w * fov;
        // The wash pool is a burst of blobs; one burst is alive at a time.
        let wash = 12;

        let mut p = Self {
            rng: saver_seed(&["PODRACER_SEED"], 0x9F0D_1A77),
            s: 0.0,
            ds: speed / fps.max(1) as f32,
            dt: 1.0 / fps.max(1) as f32,
            t: 0.0,
            cam_x: 0.0,
            hdg: 0.0,
            eng_x: [0.0; 2],
            ph: [0.0; 3],
            f_kind: vec![ARCH; features],
            f_z: vec![0.0; features],
            f_x: vec![0.0; features],
            f_w: vec![0.0; features],
            f_h: vec![0.0; features],
            f_order: vec![0; features],
            w_z: vec![0.0; wash],
            w_x: vec![0.0; wash],
            w_y: vec![0.0; wash],
            w_vx: vec![0.0; wash],
            w_life: vec![0.0; wash],
            wash_in: 0.0,
            wash_secs,
            hit_z: vec![Z_FAR; cols],
            hit_side: vec![0; cols],
            hit_h: vec![WALL_H; cols],
            row_z: vec![0.0; rows],
            row_ix: vec![0; rows],
            sin: (0..WAVE_N)
                .map(|i| (i as f32 / WAVE_N as f32 * std::f32::consts::TAU).sin())
                .collect(),
            w,
            h,
            f,
            fy: f * yk,
            hz: h * 0.44,
            horizon: h * 0.44,
            yk,
            half_w,
            pinch,
            spread: w * spread * 0.5,
            // Sized off the width, but CAPPED by the height the panel actually
            // has once the aspect stretch is paid: pine's framebuffer is
            // 1280x400 and 1.8x taller per pixel, so a bell sized as a
            // fraction of the width alone is three quarters of the panel.
            eng_r: (w * eng_r).min(h / yk * 0.22),
            shimmer,
            grid,
        };
        p.ph = [p.unit01(), p.unit01(), p.unit01()];
        // A course the pod is already flying down, not one it starts at the
        // mouth of: features are spread through the depth range on frame 0.
        for i in 0..features {
            p.respawn_feature(i);
            p.f_z[i] = 40.0 + p.unit01() * (Z_FAR - 40.0);
        }
        p.cam_x = p.centre(0.0);
        p.wash_in = wash_secs;
        p
    }

    /// Re-roll everything the seed decides, so a test can pin a route without
    /// touching the process-wide environment `PODRACER_SEED` reads — cargo
    /// runs tests in parallel threads and `set_var` would land one test's
    /// course in another's saver.
    #[cfg(test)]
    fn reseed(&mut self, seed: u32) {
        self.rng = seed;
        self.ph = [self.unit01(), self.unit01(), self.unit01()];
        for i in 0..self.f_z.len() {
            self.respawn_feature(i);
            self.f_z[i] = 40.0 + self.unit01() * (Z_FAR - 40.0);
        }
        self.cam_x = self.centre(0.0);
    }

    #[inline]
    fn unit01(&mut self) -> f32 {
        (next_rand(&mut self.rng) & 0xFFFF) as f32 / 65536.0
    }

    /// `sin` of a phase measured in TURNS.
    #[inline]
    fn wav(&self, turns: f32) -> f32 {
        self.sin[(turns * WAVE_N as f32) as i32 as usize & (WAVE_N - 1)]
    }

    /// Lateral offset of the course centreline at course position `u`, metres.
    /// Two harmonics, both an integer number of cycles per `LOOP_M`, so the
    /// wrap of `s` is invisible.
    #[inline]
    fn centre(&self, u: f32) -> f32 {
        let a = self.wav(u * (5.0 / LOOP_M) + self.ph[0]);
        let b = self.wav(u * (11.0 / LOOP_M) + self.ph[1]);
        a * (self.half_w * 1.7) + b * (self.half_w * 0.55)
    }

    /// Half-width of the corridor at `u`. Cubing the pinch wave keeps the
    /// course mostly open and makes the slots rare, short and alarming — a
    /// sinusoidal width is a corridor that is always half shut.
    #[inline]
    fn halfw(&self, u: f32) -> f32 {
        let q = 0.5 + 0.5 * self.wav(u * (3.0 / LOOP_M) + self.ph[2]);
        self.half_w * (1.0 - self.pinch * q * q * q)
    }

    /// Rim height at `u`, on `side` (0 left, 1 right). Ragged, and the two
    /// sides differ, so the skyline is rock rather than a fence.
    #[inline]
    fn rim(&self, u: f32, side: u8) -> f32 {
        let k = if side == 0 { 0.0 } else { 0.37 };
        WALL_H * (0.62 + 0.55 * (0.5 + 0.5 * self.wav(u * (37.0 / LOOP_M) + k)))
    }

    /// Screen x of a world point `x` metres across at `z` metres ahead.
    #[inline]
    fn px_of(&self, x: f32, z: f32) -> f32 {
        self.w * 0.5 + self.f * ((x - self.cam_x) / z - self.hdg)
    }

    /// Screen y of a world point `y` metres above the floor at `z` ahead.
    #[inline]
    fn py_of(&self, y: f32, z: f32) -> f32 {
        self.hz + self.fy * (CAM_H - y) / z
    }

    /// NEAR = 0, FAR = RAMP-1, saturating at `span` metres. Square-rooted so
    /// haze builds fast with distance and the far end reads as distance rather
    /// than as paint.
    ///
    /// `span` is per material and is NOT `Z_FAR`: the canyon wall a ray hits is
    /// almost always inside 150 m, so a ramp scaled to the 900 m draw distance
    /// spends five of its six steps on rock nobody can see and paints every
    /// wall on the panel the same flat colour.
    #[inline]
    fn depth_ix(z: f32, span: f32) -> usize {
        let t = (z / span).clamp(0.0, 1.0);
        ((t.sqrt() * RAMP as f32) as usize).min(RAMP - 1)
    }

    fn respawn_feature(&mut self, i: usize) {
        let r = self.unit01();
        self.f_kind[i] = if r < 0.42 { ARCH } else { SPIRE };
        self.f_z[i] = Z_FAR + self.unit01() * 260.0;
        self.f_x[i] = (self.unit01() * 2.0 - 1.0) * 0.7;
        self.f_w[i] = 0.16 + self.unit01() * 0.34;
        self.f_h[i] = 0.35 + self.unit01() * 0.75;
    }

    /// Steering, bob and the engines' swing. The pilot aims `LOOK` metres up
    /// the course and the pod chases that with a lag, so `hdg` — the lateral
    /// slope of the path — is what the canyon is seen through AND what the
    /// engines swing against.
    fn fly(&mut self) {
        self.t += self.dt;
        self.s += self.ds;
        if self.s >= LOOP_M {
            self.s -= LOOP_M;
        }
        let aim =
            self.centre(self.s + LOOK) + self.halfw(self.s + LOOK) * 0.35 * self.wav(self.t * 0.11);
        let before = self.cam_x;
        self.cam_x += (aim - self.cam_x) * (5.0 * self.dt).min(0.5);
        // Slope of the path, metres across per metre along. Smoothed, because
        // a per-frame difference of a lagged follow is noisy at high fps.
        let slope = (self.cam_x - before) / self.ds.max(1e-3);
        self.hdg += (slope - self.hdg) * 0.25;

        // Pitch bob: the pod is riding a repulsor cushion, so the horizon
        // never quite sits still. In square-pixel units, stretched for glass.
        self.horizon = self.h * 0.44;
        self.hz = self.horizon + (self.wav(self.t * 1.7) * 0.006 * self.h) * self.yk;

        // Independent yaw: the two engines lag the turn at different rates and
        // carry their own idle sway, so a hard left swings them apart. Clamped
        // because they are on CABLES, not on a rail: past a quarter of the
        // engine radius the cable is taut, and an unclamped swing walks an
        // engine clean off the panel on a hard turn.
        let swing = (-self.hdg * self.w * 0.9).clamp(-self.eng_r * 0.9, self.eng_r * 0.9);
        for i in 0..2 {
            let rate = if i == 0 { 0.16 } else { 0.11 };
            let sway = self.wav(self.t * if i == 0 { 0.43 } else { 0.31 }) * self.w * 0.008;
            self.eng_x[i] += (swing + sway - self.eng_x[i]) * rate;
        }
    }

    /// One ray per cell column, marched until it leaves the corridor.
    fn cast(&mut self) {
        let (cols, cw) = (self.grid.cols(), self.grid.cell_w() as f32);
        for cx in 0..cols {
            let px = (cx as f32 + 0.5) * cw;
            let tx = (px - self.w * 0.5) / self.f + self.hdg;
            let mut z0 = 1.0f32;
            let mut z = 4.0f32;
            let mut side = 0u8;
            let mut hit = false;
            while z < Z_FAR {
                let u = self.s + z;
                let d = self.cam_x + tx * z - self.centre(u);
                if d.abs() > self.halfw(u) {
                    side = u8::from(d > 0.0);
                    hit = true;
                    break;
                }
                z0 = z;
                z *= MARCH;
            }
            if hit {
                // The march overshoots by up to 11%; three bisections put the
                // wall inside 1.5% of where it is, which at the far end is
                // less than one cell and at the near end is the difference
                // between a wall and a wall that judders.
                let (mut lo, mut hi) = (z0, z);
                for _ in 0..BISECT {
                    let mid = (lo + hi) * 0.5;
                    let u = self.s + mid;
                    if (self.cam_x + tx * mid - self.centre(u)).abs() > self.halfw(u) {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                z = hi;
            } else {
                z = Z_FAR;
            }
            self.hit_z[cx] = z;
            self.hit_side[cx] = side;
            self.hit_h[cx] = if hit { self.rim(self.s + z, side) } else { 0.0 };
        }
    }

    /// Sky, wall and floor for every cell, from the rays. This pass writes
    /// EVERY cell — features and engines only ever paint over it.
    fn paint_world(&mut self) {
        let (cols, rows) = (self.grid.cols(), self.grid.rows());
        let ch = self.grid.cell_h() as f32;

        // Floor depth per row, shared by every column: the ground plane solve
        // depends on the scanline alone.
        for cy in 0..rows {
            let py = (cy as f32 + 0.5) * ch;
            let d = py - self.hz;
            let z = if d > 0.5 {
                (self.fy * CAM_H / d).min(Z_FAR)
            } else {
                Z_FAR
            };
            self.row_z[cy] = z;
            // Transverse bands, cut from the course position the row looks at.
            // Their contrast dies off close in: at 300 m/s the near floor is
            // moving too fast to hold a pattern, and that IS the blur.
            let band = self.wav((self.s + z) * 0.09);
            let blur = (z * 0.05).min(1.0);
            let step = Self::depth_ix(z, GND_SPAN) as f32 + band * blur * 1.9;
            self.row_ix[cy] = step.clamp(0.0, RAMP as f32 - 1.0) as u8;
        }

        for cx in 0..cols {
            let zh = self.hit_z[cx];
            let rock = if self.hit_side[cx] == 0 {
                ROCK_L
            } else {
                ROCK_R
            };
            let wall_base = self.py_of(0.0, zh);
            let wall_top = self.py_of(self.hit_h[cx], zh);
            let rock_ix = Self::depth_ix(zh, ROCK_SPAN);
            // Vertical striation, keyed to where the wall stands on the
            // course, so it sweeps past rather than crawling.
            let stria = self.wav((self.s + zh) * 0.42) + 0.35 * self.wav((self.s + zh) * 1.9);
            for cy in 0..rows {
                let py = (cy as f32 + 0.5) * ch;
                let c = if py >= wall_base {
                    GND0 + u16::from(self.row_ix[cy])
                } else if py >= wall_top && self.hit_h[cx] > 0.0 {
                    // Ribs down the rock face: the same ramp, one step either
                    // way, so a wall is never a flat slab of one colour.
                    // Bedding planes: horizontal in the WORLD, so they rake
                    // across the face and rush past as the wall closes.
                    let rib = stria + 1.3 * self.wav((self.hz - py) * (0.34 * zh / self.fy));
                    let ix = (rock_ix as f32 + rib * 1.1).clamp(0.0, RAMP as f32 - 1.0);
                    rock + ix as u16
                } else {
                    // Sky, with the heat shimmer strongest just over the rim.
                    let q = (py / self.hz.max(1.0)).clamp(0.0, 1.0);
                    let heat = self.shimmer
                        * q
                        * q
                        * self.wav(self.t * 2.3 + cx as f32 * 0.07 + py * 0.004)
                        * 1.6;
                    let ix = (q * RAMP as f32 - 1.0 + heat).clamp(0.0, RAMP as f32 - 1.0);
                    SKY0 + ix as u16
                };
                self.grid.set(cy * cols + cx, Cell::new(font::SOLID, c));
            }
        }
    }

    /// Paint a vertical run of one column, in PIXELS, clipped to the grid.
    /// Rows are claimed by their centre, which is what keeps a span that is
    /// thinner than a cell from disappearing and reappearing as it moves.
    #[inline]
    fn span(&mut self, cx: usize, y0: f32, y1: f32, glyph: u16, colour: u16) {
        if cx >= self.grid.cols() || y1 <= y0 {
            return;
        }
        let ch = self.grid.cell_h() as f32;
        let rows = self.grid.rows() as i32;
        let r0 = (y0 / ch - 0.5).ceil().max(0.0) as i32;
        let r1 = ((y1 / ch - 0.5).ceil() as i32).min(rows);
        let cols = self.grid.cols();
        for cy in r0..r1 {
            self.grid
                .set(cy as usize * cols + cx, Cell::new(glyph, colour));
        }
    }

    /// Arches and spires, far to near so the near one wins. Insertion sort
    /// over a pool of a couple of dozen: nothing here allocates and nothing
    /// here is worth a better algorithm.
    fn paint_features(&mut self) {
        let n = self.f_z.len();
        for i in 0..n {
            self.f_z[i] -= self.ds;
            if self.f_z[i] < 3.0 {
                self.respawn_feature(i);
            }
        }
        for i in 0..n {
            self.f_order[i] = i as u8;
            let mut j = i;
            while j > 0 && self.f_z[self.f_order[j - 1] as usize] < self.f_z[i] {
                self.f_order[j] = self.f_order[j - 1];
                j -= 1;
            }
            self.f_order[j] = i as u8;
        }
        for k in 0..n {
            let i = self.f_order[k] as usize;
            match self.f_kind[i] {
                ARCH => self.paint_arch(i),
                _ => self.paint_spire(i),
            }
        }
    }

    fn paint_arch(&mut self, i: usize) {
        let z = self.f_z[i];
        let u = self.s + z;
        let (c, hw) = (self.centre(u), self.halfw(u));
        // The arch is the canyon's own rock, so it springs from the walls and
        // its bore is a fraction of whatever the corridor is doing there.
        let bore = hw * (0.62 + self.f_w[i] * 0.7);
        // The bore has to be most of the rim height: a low opening reads as a
        // solid slab across the canyon rather than as something you fly
        // THROUGH, and at 300 m/s you are through it in a third of a second.
        let bore_h = WALL_H * (0.62 + self.f_h[i] * 0.3);
        let top = WALL_H * (1.05 + self.f_h[i] * 0.5);
        let ix = Self::depth_ix(z, ROCK_SPAN);
        let cw = self.grid.cell_w() as f32;
        let x0 = self.px_of(c - hw * 1.6, z).max(0.0);
        let x1 = self.px_of(c + hw * 1.6, z).min(self.w);
        let (a, b) = ((x0 / cw) as usize, (x1 / cw).ceil() as usize);
        let ground = self.py_of(0.0, z);
        let y_top = self.py_of(top, z);
        for cx in a..b.min(self.grid.cols()) {
            let px = (cx as f32 + 0.5) * cw;
            // Invert the projection for this column at the arch's depth.
            let x = self.cam_x + z * ((px - self.w * 0.5) / self.f + self.hdg);
            let d = (x - c) / bore;
            let y_low = if d.abs() < 1.0 {
                self.py_of(bore_h * (1.0 - d * d).sqrt(), z)
            } else {
                ground
            };
            // The keystone is lit and the springing is in shadow, which is
            // what stops an arch reading as a black bar.
            let shade = if d.abs() < 0.55 { 0 } else { 1 };
            self.span(
                cx,
                y_top,
                y_low,
                font::SOLID,
                ROCK_R + (ix + shade).min(RAMP - 1) as u16,
            );
        }
    }

    fn paint_spire(&mut self, i: usize) {
        let z = self.f_z[i];
        let u = self.s + z;
        let (c, hw) = (self.centre(u), self.halfw(u));
        let x = c + self.f_x[i] * hw;
        let rw = hw * 0.10 * (0.5 + self.f_w[i]);
        let top = WALL_H * (0.3 + self.f_h[i] * 0.7);
        let ix = Self::depth_ix(z, ROCK_SPAN);
        let cw = self.grid.cell_w() as f32;
        let x0 = self.px_of(x - rw, z).max(0.0);
        let x1 = self.px_of(x + rw, z).min(self.w);
        let (a, b) = ((x0 / cw) as usize, (x1 / cw).ceil() as usize);
        let ground = self.py_of(0.0, z);
        let cols = self.grid.cols();
        for cx in a..b.min(cols) {
            // A spire tapers: the outer columns are shorter than the axis.
            let q = if b > a + 1 {
                (cx - a) as f32 / (b - 1 - a) as f32 * 2.0 - 1.0
            } else {
                0.0
            };
            let y_top = self.py_of(top * (1.0 - q * q * 0.45), z);
            let lit = usize::from(q > 0.1);
            self.span(
                cx,
                y_top,
                ground,
                font::SOLID,
                ROCK_L + (ix + lit).min(RAMP - 1) as u16,
            );
        }
    }

    /// A rival's engine wash: a burst of hot blobs drifting across the view,
    /// drawn as SHADE so the canyon shows through it.
    fn paint_wash(&mut self) {
        if self.wash_secs <= 0.0 {
            return;
        }
        self.wash_in -= self.dt;
        if self.wash_in <= 0.0 {
            // Poisson-ish rather than metronomic, and always from one side.
            self.wash_in = self.wash_secs * (0.4 + self.unit01() * 1.6);
            let dir = if self.unit01() < 0.5 { -1.0 } else { 1.0 };
            let z = 45.0 + self.unit01() * 95.0;
            for i in 0..self.w_z.len() {
                self.w_z[i] = z + self.unit01() * 22.0;
                self.w_x[i] = -dir * (self.half_w * 2.2) + self.unit01() * self.half_w * 0.8;
                self.w_y[i] = CAM_H * (0.4 + self.unit01() * 2.2);
                self.w_vx[i] = dir * (self.half_w * (2.4 + self.unit01()));
                self.w_life[i] = 0.7 + self.unit01() * 0.9;
            }
        }
        let cw = self.grid.cell_w() as f32;
        for i in 0..self.w_z.len() {
            if self.w_life[i] <= 0.0 {
                continue;
            }
            self.w_life[i] -= self.dt;
            self.w_x[i] += self.w_vx[i] * self.dt;
            self.w_z[i] -= self.ds;
            if self.w_z[i] < 6.0 {
                self.w_life[i] = 0.0;
                continue;
            }
            let z = self.w_z[i];
            let cx0 = self.px_of(self.w_x[i], z);
            let cy0 = self.py_of(self.w_y[i], z);
            // Radius shrinks with distance like everything else, and the blob
            // is smeared ACROSS the view because that is the direction it is
            // travelling at speed.
            let r = self.f * 1.9 / z;
            let hot = FLARE0 + 1 + (self.w_life[i] * 2.0).clamp(0.0, 2.0) as u16;
            let a = ((cx0 - r * 2.2) / cw).max(0.0) as usize;
            let b = (((cx0 + r * 2.2) / cw).ceil().max(0.0) as usize).min(self.grid.cols());
            for cx in a..b {
                let dx = ((cx as f32 + 0.5) * cw - cx0) / (r * 2.2);
                if dx.abs() >= 1.0 {
                    continue;
                }
                let hh = r * (1.0 - dx * dx).sqrt() * self.yk;
                // Solid at the core, dithered at the edges: a plume that is
                // SHADE all the way through reads as a smear on the rock
                // rather than as something burning.
                let glyph = if dx.abs() < 0.45 {
                    font::SOLID
                } else {
                    font::SHADE
                };
                self.span(cx, cy0 - hh, cy0 + hh, glyph, hot);
            }
        }
    }

    /// The two engines and their cables — the signature silhouette, and the
    /// last thing painted because nothing is in front of them.
    fn paint_pod(&mut self) {
        // Cables first: they run from the engines back past the camera, so
        // they pass BEHIND the engine bodies.
        for i in 0..2 {
            let (ex, ey) = self.engine_at(i);
            let side = if i == 0 { -1.0 } else { 1.0 };
            for k in 0..2 {
                let hitch = self.w * 0.5 + side * self.w * 0.06 * (1.0 + k as f32);
                self.cable(ex + side * self.eng_r * (0.1 - 0.5 * k as f32), ey, hitch);
            }
        }
        for i in 0..2 {
            self.engine(i);
        }
    }

    /// Where engine `i` sits this frame, in pixels.
    #[inline]
    fn engine_at(&self, i: usize) -> (f32, f32) {
        let side = if i == 0 { -1.0 } else { 1.0 };
        let bob = self.wav(self.t * (0.7 + 0.2 * i as f32) + 0.3 * i as f32) * self.h * 0.012;
        (
            self.w * 0.5 + side * self.spread + self.eng_x[i],
            self.hz + (self.h * 0.055 + bob) * self.yk,
        )
    }

    /// One cable, engine to cockpit hitch at the bottom of the frame. Two
    /// cells wide, with the lit edge on the sunward side.
    fn cable(&mut self, ex: f32, ey: f32, hitch: f32) {
        let cw = self.grid.cell_w() as f32;
        let ch = self.grid.cell_h() as f32;
        let rows = self.grid.rows();
        let r0 = (ey / ch).max(0.0) as usize;
        for cy in r0..rows {
            let py = (cy as f32 + 0.5) * ch;
            let t = ((py - ey) / (self.h - ey).max(1.0)).clamp(0.0, 1.0);
            // Slack, not a taut string: the cable whips with the engine swing.
            let sag = self.wav(self.t * 1.3 + t * 0.4) * self.w * 0.004 * (1.0 - t);
            let px = ex + (hitch - ex) * t * t + sag;
            let cx = (px / cw) as i32;
            let cols = self.grid.cols() as i32;
            for (k, c) in [(0i32, ENG_DARK), (1, ENG_LIT)] {
                let x = cx + k;
                if x >= 0 && x < cols {
                    self.grid
                        .set(cy * cols as usize + x as usize, Cell::new(font::SOLID, c));
                }
            }
        }
    }

    fn engine(&mut self, i: usize) {
        let (ex, ey) = self.engine_at(i);
        // Slightly wider than tall on GLASS: the engines are ahead and out to
        // the sides, so they are seen at an angle rather than dead astern.
        let rx = self.eng_r * 1.12;
        let ry = self.eng_r * 0.94 * self.yk;
        // Flare flicker: a combustion chamber, not a lamp.
        let flick = 1.0
            + 0.16 * self.wav(self.t * 9.1 + i as f32 * 0.5)
            + 0.09 * self.wav(self.t * 23.0 + i as f32);
        let cw = self.grid.cell_w() as f32;
        let ch = self.grid.cell_h() as f32;
        let cols = self.grid.cols();
        let rows = self.grid.rows();
        let a = ((ex - rx * HALO) / cw).max(0.0) as usize;
        let b = ((((ex + rx * HALO) / cw).ceil()).max(0.0) as usize).min(cols);
        let r0 = ((ey - ry * HALO) / ch).max(0.0) as usize;
        let r1 = ((((ey + ry * HALO) / ch).ceil()).max(0.0) as usize).min(rows);
        for cx in a..b {
            let dx = ((cx as f32 + 0.5) * cw - ex) / rx;
            for cy in r0..r1 {
                let dy = ((cy as f32 + 0.5) * ch - ey) / ry;
                let t2 = dx * dx + dy * dy;
                if t2 >= HALO * HALO {
                    continue;
                }
                let t = t2.sqrt();
                let (glyph, colour) = if t > 1.0 {
                    // Exhaust halo washing over the canyon behind it.
                    (font::SHADE, FLARE0)
                } else if t > 0.90 {
                    (font::SOLID, if dy < -0.2 { ENG_RIM } else { ENG_DARK })
                } else if t > 0.58 {
                    (
                        font::SOLID,
                        if dy < -0.1 && dx * (if i == 0 { -1.0 } else { 1.0 }) > 0.0 {
                            ENG_LIT
                        } else {
                            ENG_BODY
                        },
                    )
                } else if t > 0.46 {
                    (font::SOLID, ENG_DARK)
                } else {
                    let q = t / 0.46 * flick;
                    let ix = (q * 3.6).clamp(0.0, 3.0) as u16;
                    (font::SOLID, FLARE0 + 3 - ix)
                };
                self.grid.set(cy * cols + cx, Cell::new(glyph, colour));
            }
        }
    }
}

impl Saver for Podracer {
    fn render(&mut self, s: &mut Surface<'_>) {
        self.fly();
        self.cast();
        self.paint_world();
        self.paint_features();
        self.paint_wash();
        self.paint_pod();
        // `flush`, never `flush_sparse`: nearly every cell moves every frame,
        // so a dirty list would be the whole panel plus a way to under-report
        // it and freeze a region for the life of the pod.
        self.grid.flush(s, &PAL);
    }

    fn name(&self) -> &'static str {
        "podracer"
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
    use crate::dump::verify;
    use crate::grid::with_test_aspect;
    use crate::saver;
    use crate::testalloc::allocs_during;

    /// The live panel is 1280x400; 1920x1080 is what the firmware hands the
    /// Pi. Both are tested, at both aspects, because this saver measures in
    /// framebuffer pixels and the cell cannot carry the correction for it.
    const PANELS: [(usize, usize); 2] = [(1920, 1080), (1280, 400)];

    fn build(w: usize, h: usize, aspect: usize) -> (Panel, Podracer) {
        seeded(w, h, aspect, 0x00C0_FFEE)
    }

    fn seeded(w: usize, h: usize, aspect: usize, seed: u32) -> (Panel, Podracer) {
        let p = Panel::new(w, h, w);
        let mut s = with_test_aspect(aspect, || Podracer::new(&p, 30));
        s.reseed(seed);
        (p, s)
    }

    /// Colour index of every cell of the drawn frame.
    fn cols_of(s: &Podracer) -> Vec<usize> {
        s.grid.cells().iter().map(|c| c.colour()).collect()
    }

    /// Nothing may be written that is not reported: on simpledrm an
    /// unreported region shows a stale frame for the life of the pod, and
    /// `verify` is the only check that can see it without a monitor.
    #[test]
    fn every_changed_pixel_is_reported_at_both_aspects() {
        for (w, h) in PANELS {
            for aspect in [100, 180] {
                let (p, mut s) = build(w, h, aspect);
                let mut buf = vec![0u32; p.buf_len()];
                let mut prev = vec![0u32; p.buf_len()];
                for n in 0..60 {
                    prev.copy_from_slice(&buf);
                    let d = saver::frame(&mut s, &mut buf, &p);
                    verify(&prev, &buf, &d, &p, n)
                        .unwrap_or_else(|e| panic!("{w}x{h} aspect {aspect}: {e}"));
                    // `verify` alone passes vacuously for a saver that stops
                    // blitting: nothing changed, so nothing was under-reported.
                    // The whole world is moving every frame, so a frame that
                    // moved no pixels is a saver that has gone dark on the
                    // panel — which is exactly what a `flush_sparse` with a
                    // short dirty list looks like from here.
                    assert!(
                        n == 0 || prev != buf,
                        "{w}x{h} aspect {aspect} frame {n}: the frame did not move"
                    );
                    assert!(
                        d.px() > p.w * p.h / 8,
                        "{w}x{h} aspect {aspect} frame {n}: only {} px reported",
                        d.px()
                    );
                }
                // Frame 0 covers the panel, so nothing may still be black
                // except the inside of an engine bell.
                assert!(
                    buf.iter().filter(|&&v| v == 0).count() < p.buf_len() / 4,
                    "{w}x{h} aspect {aspect}: the desert is mostly black"
                );
            }
        }
    }

    /// The frame loop is the product. A `Vec` that grows inside `render` is a
    /// malloc per frame against a 500m budget.
    #[test]
    fn render_never_allocates() {
        let (p, mut s) = build(1280, 400, 180);
        let mut buf = vec![0u32; p.buf_len()];
        saver::frame(&mut s, &mut buf, &p);
        let n = allocs_during(|| {
            for _ in 0..600 {
                saver::frame(&mut s, &mut buf, &p);
            }
        });
        assert_eq!(n, 0, "render allocated {n} times over 600 frames");
        // Non-vacuous: 600 frames at 300 m/s is 6 km of course, so every pool
        // above has recycled many times over.
        assert!(s.f_z.iter().all(|z| *z > 0.0));
    }

    /// The signature silhouette. Two engines, one left of centre and one
    /// right, with a hot bell in each, on EVERY frame — including while the
    /// pod is hard over in a turn, which is when a screen-space sprite that
    /// was allowed to drift would leave the frame.
    #[test]
    fn both_engines_are_in_every_frame() {
        for (w, h) in PANELS {
            for aspect in [100, 180] {
                let (p, mut s) = build(w, h, aspect);
                let mut buf = vec![0u32; p.buf_len()];
                let cols = s.grid.cols();
                for n in 0..300 {
                    saver::frame(&mut s, &mut buf, &p);
                    let (mut left, mut right) = (0, 0);
                    for (i, c) in s.grid.cells().iter().enumerate() {
                        // The flare core: nothing else in the palette is here.
                        if c.colour() >= FLARE0 as usize + 2 {
                            if i % cols < cols / 2 {
                                left += 1;
                            } else {
                                right += 1;
                            }
                        }
                    }
                    assert!(
                        left > 0 && right > 0,
                        "{w}x{h} aspect {aspect} frame {n}: engines left={left} right={right}"
                    );
                    // And they must leave a canyon to look at. Measured in
                    // ROWS, because the failure this catches is a bell sized
                    // off the panel WIDTH alone landing on a 1280x400 panel
                    // whose pixels are 1.8x taller. The CASING colour, not the
                    // whole engine palette: the cables are drawn in two of
                    // those and run to the bottom of the frame by design.
                    let rows = s.grid.rows();
                    let tall = (0..rows)
                        .filter(|cy| {
                            s.grid.cells()[cy * cols..(cy + 1) * cols]
                                .iter()
                                .any(|c| c.colour() == ENG_BODY as usize)
                        })
                        .count();
                    assert!(
                        tall * 2 < rows,
                        "{w}x{h} aspect {aspect} frame {n}: the engines fill {tall} of {rows} rows"
                    );
                }
            }
        }
    }

    /// The engines must also MOVE independently — one rigid pair welded to the
    /// centreline is a HUD, not a podracer. Over a run the two lateral offsets
    /// must differ, and each must travel.
    #[test]
    fn the_engines_yaw_independently() {
        let (p, mut s) = build(1920, 1080, 100);
        let mut buf = vec![0u32; p.buf_len()];
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        let mut apart: f32 = 0.0;
        for _ in 0..400 {
            saver::frame(&mut s, &mut buf, &p);
            for i in 0..2 {
                lo[i] = lo[i].min(s.eng_x[i]);
                hi[i] = hi[i].max(s.eng_x[i]);
            }
            apart = apart.max((s.eng_x[0] - s.eng_x[1]).abs());
        }
        for i in 0..2 {
            assert!(
                hi[i] - lo[i] > s.w * 0.01,
                "engine {i} barely moved: {} px",
                hi[i] - lo[i]
            );
        }
        assert!(
            apart > s.w * 0.004,
            "the engines move as one rigid pair ({apart} px apart at most)"
        );
    }

    /// Canyon walls on BOTH sides, and a route that really does pinch. The
    /// wall test is per frame; the slot is over a run, because slots are rare
    /// by construction (`halfw` cubes its wave).
    #[test]
    fn the_canyon_has_two_walls_and_sometimes_a_slot() {
        let (p, mut s) = build(1920, 1080, 100);
        let mut buf = vec![0u32; p.buf_len()];
        let mut narrowest = f32::MAX;
        for n in 0..900 {
            saver::frame(&mut s, &mut buf, &p);
            let cols = s.grid.cols();
            let l = s.hit_z[..cols / 4].iter().fold(f32::MAX, |a, b| a.min(*b));
            let r = s.hit_z[cols * 3 / 4..]
                .iter()
                .fold(f32::MAX, |a, b| a.min(*b));
            assert!(
                l < Z_FAR && r < Z_FAR,
                "frame {n}: no wall on one side (left {l}, right {r})"
            );
            narrowest = narrowest.min(s.halfw(s.s + LOOK));
        }
        assert!(
            narrowest < s.half_w * 0.5,
            "the course never pinched: narrowest half-width {narrowest} of {}",
            s.half_w
        );
    }

    /// The projection is derived from SQUARE-pixel units and stretched for the
    /// glass, so at aspect 180 a feature sits 1.8x further from the horizon —
    /// in CELL ROWS, which is the thing a scalar assertion would miss. A
    /// projection built out of cells instead would put it at the same row
    /// fraction at both aspects and pass anything weaker than this.
    #[test]
    fn the_picture_is_stretched_for_the_panel_not_squashed() {
        let centre_row = |aspect: usize| -> f32 {
            let (p, mut s) = seeded(1920, 1080, aspect, 12345);
            let mut buf = vec![0u32; p.buf_len()];
            // Fixed pose, so this measures the projection and not the bob.
            for _ in 0..4 {
                saver::frame(&mut s, &mut buf, &p);
            }
            let cols = s.grid.cols();
            let (mut sum, mut n) = (0.0f32, 0.0f32);
            for (i, c) in s.grid.cells().iter().enumerate() {
                if c.colour() >= FLARE0 as usize + 2 {
                    sum += (i / cols) as f32;
                    n += 1.0;
                }
            }
            assert!(n > 0.0, "aspect {aspect}: no flare to measure");
            sum / n / s.grid.rows() as f32
        };
        let q100 = centre_row(100);
        let q180 = centre_row(180);
        // The stretch is about the HORIZON, which is where the projection's
        // origin is — not about the middle of the panel.
        let (_, s) = build(1920, 1080, 100);
        let qh = s.horizon / s.h;
        let want = qh + (q100 - qh) * 1.8;
        assert!(
            (q180 - want).abs() < 0.04,
            "flare at {q100} of the panel at aspect 100 landed at {q180} at 180, want {want}"
        );
        // And it must actually have moved: an aspect that changed nothing
        // would satisfy a sloppier tolerance on its own.
        assert!(
            (q180 - q100).abs() > 0.03,
            "aspect 180 moved the picture by nothing ({q100} -> {q180})"
        );

        // The line above rides on `yk`, which places the engines. `fy` — the
        // vertical half of the PROJECTION — appears nowhere in it, and
        // dropping the stretch there is a squashed canyon that this measures:
        // the floor under a given row fraction is 1.8x further away when each
        // pixel is 1.8x taller.
        let floor_z = |aspect: usize| -> f32 {
            let (p, mut s) = seeded(1920, 1080, aspect, 12345);
            let mut buf = vec![0u32; p.buf_len()];
            for _ in 0..4 {
                saver::frame(&mut s, &mut buf, &p);
            }
            s.row_z[s.grid.rows() * 17 / 20]
        };
        let ratio = floor_z(180) / floor_z(100);
        assert!(
            (1.6..2.0).contains(&ratio),
            "the floor under the same row is {ratio}x further at aspect 180, want ~1.8x"
        );
    }

    /// One wall is in shadow and the other is lit, and which is which is the
    /// only thing that tells a viewer where the sun is. Aggregated over a run
    /// because the pod is often turned, and a single frame can genuinely show
    /// more of one wall than the other.
    #[test]
    fn the_sunlit_wall_is_on_the_right() {
        let (p, mut s) = build(1920, 1080, 100);
        let mut buf = vec![0u32; p.buf_len()];
        let (mut ll, mut lr, mut rl, mut rr) = (0u32, 0u32, 0u32, 0u32);
        for _ in 0..200 {
            saver::frame(&mut s, &mut buf, &p);
            let cols = s.grid.cols();
            for (i, c) in s.grid.cells().iter().enumerate() {
                let left_half = i % cols < cols / 2;
                let shade = (ROCK_L as usize..ROCK_L as usize + RAMP).contains(&c.colour());
                let lit = (ROCK_R as usize..ROCK_R as usize + RAMP).contains(&c.colour());
                match (left_half, shade, lit) {
                    (true, true, _) => ll += 1,
                    (true, _, true) => lr += 1,
                    (false, true, _) => rl += 1,
                    (false, _, true) => rr += 1,
                    _ => {}
                }
            }
        }
        assert!(
            ll > lr,
            "the left of the frame is lit rock ({ll} shadow, {lr} lit)"
        );
        assert!(
            rr > rl,
            "the right of the frame is shadowed rock ({rr} lit, {rl} shadow)"
        );
    }

    /// Speed is the subject: the world has to be moving, and moving because
    /// the COURSE advanced rather than because a sine wobbled. Same seed, two
    /// speeds, and the slower one must be behind after the same wall time.
    #[test]
    fn the_course_advances_with_speed() {
        let (p, mut s) = seeded(1280, 400, 180, 99);
        let mut buf = vec![0u32; p.buf_len()];
        let before = cols_of(&s);
        for _ in 0..30 {
            saver::frame(&mut s, &mut buf, &p);
        }
        let changed = cols_of(&s)
            .iter()
            .zip(&before)
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            changed > s.grid.cols() * s.grid.rows() / 4,
            "a second of flight changed only {changed} cells"
        );
        // Half the frames at the same speed must be less far along.
        let (_, mut half) = seeded(1280, 400, 180, 99);
        let mut buf2 = vec![0u32; p.buf_len()];
        for _ in 0..15 {
            saver::frame(&mut half, &mut buf2, &p);
        }
        assert!(
            half.s < s.s,
            "fifteen frames ({}) got as far as thirty ({})",
            half.s,
            s.s
        );
    }

    /// A seed reproduces a run exactly, and a different seed is a different
    /// route — four savers shipped without this and could not be dumped and
    /// compared against another build at all.
    #[test]
    fn the_seed_pins_the_route() {
        let run = |seed: u32| {
            let (p, mut s) = seeded(1280, 400, 180, seed);
            let mut buf = vec![0u32; p.buf_len()];
            for _ in 0..40 {
                saver::frame(&mut s, &mut buf, &p);
            }
            cols_of(&s)
        };
        assert_eq!(run(7), run(7), "the same seed drew a different frame");
        assert_ne!(run(7), run(8), "two seeds drew the same frame");
    }

    /// The floor must read as ground and the sky as sky: a bug that let the
    /// ground ramp address the sky ramp (or vice versa) still passes every
    /// damage and geometry test above, and is the whole look of the saver.
    ///
    /// Below the horizon is checked EVERY frame and everywhere, because "the
    /// sky is under the floor" is never acceptable. Sky above the rim is
    /// checked over a run instead: in a slot the rim really can close over the
    /// top of the frame, and asserting sky on every frame would be asserting
    /// the canyon is never deep.
    #[test]
    fn the_sky_is_above_the_rim_and_never_below_it() {
        let (p, mut s) = build(1920, 1080, 100);
        let mut buf = vec![0u32; p.buf_len()];
        let mut sky_frames = 0;
        for n in 0..240 {
            saver::frame(&mut s, &mut buf, &p);
            let cols = s.grid.cols();
            let horizon_row = (s.hz / s.grid.cell_h() as f32) as usize;
            let mut sky = 0;
            for (i, c) in s.grid.cells().iter().enumerate() {
                let is_sky = c.colour() >= SKY0 as usize && c.colour() < SKY0 as usize + RAMP;
                if i / cols > horizon_row + 1 {
                    assert!(
                        !is_sky,
                        "frame {n}: row {} below the horizon is sky colour {}",
                        i / cols,
                        c.colour()
                    );
                }
                sky += usize::from(is_sky);
            }
            sky_frames += usize::from(sky > 20);
        }
        assert!(
            sky_frames > 120,
            "only {sky_frames} of 240 frames showed any sky over the rim"
        );
    }

    /// Nothing in the palette may be green or blue: the whole point of this
    /// saver beside the other forward-motion ones is that a frame of it cannot
    /// be recoloured into a forest or into space. Red is the dominant channel
    /// of every colour here except black.
    #[test]
    fn the_palette_is_desert() {
        for (i, [r, g, b]) in PAL_RGB.iter().enumerate().skip(1) {
            assert!(
                r >= g && g >= b,
                "palette {i} is not warm: {r:02x}{g:02x}{b:02x}"
            );
        }
    }
}
