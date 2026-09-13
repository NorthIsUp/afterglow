//! zot — lightning. A leader crosses the panel, forking as it goes, each fork
//! forking again less often; the channel strobes through a few return strokes,
//! a faint wash lights the whole panel, and then it is dark for a while.
//!
//! # The geometry is generated ONCE per bolt, not per frame
//!
//! A bolt is static for its whole (third of a second) life — what changes is
//! how bright it is. So `strike` rasterises the whole branching tree into
//! `dots` (braille sub-cell bits) and `halo` (cells next to the channel) once,
//! and every frame after that is one global intensity envelope plus a table
//! lookup per cell. No trig, no branching, no per-cell decay in the frame path.
//!
//! Generation is bounded by a step BUDGET rather than by the shape of the tree:
//! fork counts are random, and a random branching process with no ceiling has a
//! tail that eventually lands a strike that takes a second to generate. The
//! budget makes the worst bolt cost the same as the average one.
//!
//! # Damage model: full repaint (`Grid::flush`)
//!
//! Sparse is the obvious move for a saver that is black most of the time, and
//! it is the wrong one: the afterglow wash re-colours EVERY cell on the frame
//! it steps a level, so the frames that are not empty are full repaints anyway,
//! and the frames that are empty cost `flush` a u32 compare per cell and
//! nothing else — it reports zero rows and issues no ioctl. `flush_sparse`
//! would buy that same nothing in exchange for a dirty list that can
//! under-report and freeze a region of the panel forever.
//!
//! # Aspect independence
//!
//! Endpoints are drawn on the PERIMETER of the sub-cell field, parameterised by
//! arc length, and the far end is 30%-70% of the perimeter away. That crosses
//! the panel at any shape: on a 1280x400 strip most of the perimeter is the top
//! and bottom edges, so most bolts run along it, and on 1920x1080 the draw is
//! closer to even. Nothing here knows an aspect ratio.
//!
//! # Knobs
//!
//! * `ZOT_CELL_W` / `ZOT_CELL_H` — cell in px (8, 16)
//! * `ZOT_BOLT_MS` — how long ONE stroke lasts, 60..=3000 (default 380). Each
//!   stroke varies this by ±25%.
//! * `ZOT_RESTRIKE_PCT` — chance that another return stroke follows down the
//!   same channel, 0..=100 (default 70), rolled again for each stroke after
//!   the first up to `MAX_STROKES`. This is what makes a bolt flash twice.
//! * `ZOT_STROKE_GAP_MS` — dark between those strokes, 10..=1000 (default
//!   110). Short enough that the two flashes read as one bolt.
//! * `ZOT_GAP_MIN_MS` / `ZOT_GAP_MAX_MS` — darkness between bolts,
//!   100..=60000 (default 700 / 2400)
//! * `ZOT_FORK_PCT` — chance per step that the leader forks, 0..=100
//!   (default 9), and every fork's own forks at the same rate. The tree
//!   thins out because a fork is only a fraction of what is left of its
//!   parent, not because the rate drops.
//! * `ZOT_AIR_PCT` — percent of bolts that end in mid-air instead of on the far
//!   edge, 0..=100 (default 30)
//! * `ZOT_JITTER` — milli-radians of wander per step, 10..=3000 (default 900)
//! * `ZOT_GLOW_PCT` — afterglow wash peak, as a percent of the channel's
//!   brightness, 0..=100 (default 45)
//! * `ZOT_HALO_PCT` — halo brightness, as a percent of the channel's,
//!   0..=100 (default 70)

use crate::font;
use crate::grid::{bake, dot_bit, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, next_rand};

/// Brightness steps per ramp. The channel gets more than the halo and the wash
/// because it is the thing the eye tracks; the other two are dim enough that a
/// sixth step is below the panel's noise floor.
const CORE_LEVELS: usize = 6;
const HALO_LEVELS: usize = 5;
const GLOW_LEVELS: usize = 5;

const CORE_BASE: usize = 1;
const HALO_BASE: usize = CORE_BASE + CORE_LEVELS;
const GLOW_BASE: usize = HALO_BASE + HALO_LEVELS;
const PAL_LEN: usize = GLOW_BASE + GLOW_LEVELS;

/// Three separate ramps, not one ramp dimmed: the channel is white with barely
/// any blue in it, the halo is unmistakably blue, and the wash is a near-black
/// blue. Paint all three off one ramp and the halo reads as a fat channel
/// rather than as light coming off one.
const RAMPS: [([u8; 3], usize, usize); 3] = [
    ([0xF6, 0xF8, 0xFF], CORE_BASE, CORE_LEVELS),
    ([0x58, 0x78, 0xD8], HALO_BASE, HALO_LEVELS),
    ([0x24, 0x2E, 0x54], GLOW_BASE, GLOW_LEVELS),
];

/// `(l+1)(l+2) / L(L+1)` per ramp — quadratic, for the reason lissajous is:
/// a linear ramp spends half its steps below where an 8x16 cell is visible at
/// all, so the fade-out happens in one step instead of over the ramp.
const fn ramps() -> [[u8; 3]; PAL_LEN] {
    let mut out = [[0u8; 3]; PAL_LEN];
    let mut r = 0;
    while r < RAMPS.len() {
        let (top, base, levels) = (RAMPS[r].0, RAMPS[r].1, RAMPS[r].2);
        let den = (levels * (levels + 1)) as u32;
        let mut l = 0;
        while l < levels {
            let num = ((l + 1) * (l + 2)) as u32;
            let mut c = 0;
            while c < 3 {
                out[base + l][c] = (top[c] as u32 * num / den) as u8;
                c += 1;
            }
            l += 1;
        }
        r += 1;
    }
    out
}

const PAL: [u32; PAL_LEN] = bake(&ramps());

/// Sub-cells advanced per generation step. Three is short enough that the
/// per-step angle jitter reads as a crooked channel rather than as a polyline,
/// and long enough that a full-panel leader is a couple of hundred steps.
const STEP: f32 = 3.0;

/// Generations of fork. Beyond three a fork is a few sub-cells long and stops
/// being visible as a separate branch at all.
const MAX_DEPTH: u8 = 3;

/// Branches pending at once, and the total steps one strike may take. Both are
/// hard ceilings on generation cost — see the module doc.
const MAX_BRANCHES: usize = 64;
const STEP_BUDGET: u32 = 6000;

/// How hard the leader is steered back toward its target each step, against
/// the jitter. Low enough to leave a zigzag, high enough to arrive.
const LEADER_PULL: f32 = 0.22;

/// Frames of the return-stroke strobe, bounded by the width of the bitmask it
/// is drawn from.
const FLICKER_FRAMES: u32 = 32;

/// Strokes one channel may carry, the first included. Real flashes run to a
/// dozen; past three the channel is dimmer than its own afterglow and the
/// flicker reads as a fault rather than as lightning.
const MAX_STROKES: u32 = 3;

/// Each stroke's peak and length as a percent of the one before it. A
/// re-strike runs down a channel the last one already half-discharged, so it
/// is dimmer and quicker — which is also what keeps a three-stroke bolt from
/// outstaying its welcome.
const RESTRIKE_PEAK_PCT: u32 = 72;
const RESTRIKE_LIFE_PCT: u32 = 80;

/// A branch of the channel, walked to exhaustion and free to spawn more.
#[derive(Clone, Copy)]
struct Branch {
    x: f32,
    y: f32,
    dir: f32,
    /// Sub-cells of channel still to draw. Counts down by `STEP`.
    len: f32,
    depth: u8,
}

/// A point at arc length `t` around the perimeter of a `w` x `h` field,
/// clamped inside it. Arc length rather than "pick an edge, then a position on
/// it" because the edge choice then has to be weighted by edge length to be
/// uniform, and getting that weighting wrong is exactly the hardcoded aspect
/// assumption this saver must not have.
fn perim(t: f32, w: f32, h: f32) -> (f32, f32) {
    let p = 2.0 * (w + h);
    let t = t.rem_euclid(p);
    let (x, y) = if t < w {
        (t, 0.0)
    } else if t < w + h {
        (w - 1.0, t - w)
    } else if t < 2.0 * w + h {
        (2.0 * w + h - t, h - 1.0)
    } else {
        (0.0, p - t)
    };
    (x.clamp(0.0, w - 1.0), y.clamp(0.0, h - 1.0))
}

/// Signed angle difference in -PI..=PI. Steering on the raw difference turns
/// the long way round whenever the two headings straddle the wrap point.
#[inline]
fn wrap(a: f32) -> f32 {
    (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

/// Palette level for an intensity: 0 is off, otherwise 1..=levels. Off has to
/// be its own answer — the dimmest ramp entry is still lit, and a bolt that
/// never quite reaches black leaves the panel permanently grey.
#[inline]
fn level(i: u32, levels: usize) -> usize {
    if i == 0 {
        0
    } else {
        (i as usize * levels / 256).min(levels - 1) + 1
    }
}

pub struct Zot {
    grid: Grid,
    cols: usize,
    rows: usize,
    subw: f32,
    subh: f32,
    /// Braille bits of the channel, per cell. Written by `strike`, read-only
    /// for the whole life of the bolt.
    dots: Vec<u8>,
    /// 1 where a cell neighbours the channel. Pre-computed with the channel,
    /// because dilating the grid every frame is the per-frame cost this saver
    /// exists to not have.
    halo: Vec<u8>,
    /// Generation work list. Pre-allocated to `MAX_BRANCHES` and never grown.
    branches: Vec<Branch>,
    rng: u32,
    /// Frames since this bolt struck.
    age: u32,
    /// Frames the channel is visible, the wash is visible, and the later of
    /// the two — after which the gap starts.
    life: u32,
    glow_life: u32,
    total: u32,
    /// Frames of darkness left. Zero while a bolt is on screen.
    wait: u32,
    /// Bit k set means frame k of the strike is a return stroke; on the dark
    /// bits the channel drops to a third, which is what makes a bolt flicker
    /// instead of simply dimming. Bit 0 is always set, so a bolt is never
    /// first seen at a third brightness.
    flicker: u32,
    bolt_frames: u32,
    /// Strokes still owed to the CURRENT channel, and the dark between them.
    strokes_left: u32,
    stroke_gap: u32,
    restrike_pct: u32,
    /// This stroke's peak channel brightness, 0..=255. The first stroke of a
    /// bolt gets all of it; each re-strike a fraction of the last.
    peak: u32,
    /// The wash peak for this stroke — `peak_glow` scaled by `peak`, folded in
    /// here so the frame path does not multiply it every frame.
    glow_peak: u32,
    gap_lo: u32,
    gap_hi: u32,
    peak_glow: u32,
    halo_pct: u32,
    fork_pct: u32,
    air_pct: u32,
    jitter: f32,
    /// Where the leader is heading. Only depth 0 steers.
    target: (f32, f32),
}

impl Zot {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        let cell_w = env_num(&["ZOT_CELL_W"], 8, 4, 64) as usize;
        let cell_h = env_num(&["ZOT_CELL_H"], 16, 8, 128) as usize;
        let bolt_ms = env_num(&["ZOT_BOLT_MS"], 380, 60, 3000) as u32;
        let gap_min = env_num(&["ZOT_GAP_MIN_MS"], 700, 100, 60_000) as u32;
        let gap_max = env_num(&["ZOT_GAP_MAX_MS"], 2400, 100, 60_000) as u32;
        let restrike_pct = env_num(&["ZOT_RESTRIKE_PCT"], 70, 0, 100) as u32;
        let stroke_gap_ms = env_num(&["ZOT_STROKE_GAP_MS"], 110, 10, 1000) as u32;
        let fork_pct = env_num(&["ZOT_FORK_PCT"], 9, 0, 100) as u32;
        let air_pct = env_num(&["ZOT_AIR_PCT"], 30, 0, 100) as u32;
        let jitter = env_num(&["ZOT_JITTER"], 900, 10, 3000) as f32 / 1000.0;
        let glow_pct = env_num(&["ZOT_GLOW_PCT"], 45, 0, 100) as u32;
        let halo_pct = env_num(&["ZOT_HALO_PCT"], 70, 0, 100) as u32;

        let grid = Grid::new(panel, cell_w, cell_h);
        let (cols, rows) = (grid.cols(), grid.rows());
        let fps = fps.max(1);
        let frames = |ms: u32| (ms * fps / 1000).max(1);

        let mut me = Self {
            grid,
            cols,
            rows,
            subw: (cols * 2) as f32,
            subh: (rows * 4) as f32,
            dots: vec![0; cols * rows],
            halo: vec![0; cols * rows],
            branches: Vec::with_capacity(MAX_BRANCHES),
            // Off the clock, so a restart does not replay the same storm.
            rng: crate::saver_seed(&["ZOT_SEED"], 0x20_7E_5A_10),
            age: 0,
            life: 1,
            glow_life: 1,
            total: 1,
            wait: 0,
            flicker: 1,
            bolt_frames: frames(bolt_ms),
            strokes_left: 0,
            stroke_gap: frames(stroke_gap_ms),
            restrike_pct,
            peak: 255,
            glow_peak: 255 * glow_pct / 100,
            gap_lo: frames(gap_min.min(gap_max)),
            gap_hi: frames(gap_min.max(gap_max)),
            peak_glow: 255 * glow_pct / 100,
            halo_pct,
            fork_pct,
            air_pct,
            jitter,
            target: (0.0, 0.0),
        };
        // Open on a bolt rather than on the gap: a saver that shows nothing
        // for the first two seconds looks broken, and a short CI dump would
        // never see one. Armed as a one-frame wait rather than struck here, so
        // the first bolt is drawn from age 0 like every other one — struck in
        // the constructor it would first be DRAWN at age 1, and age 1 is a
        // frame the strobe is free to have dark.
        me.wait = 1;
        me
    }

    #[inline]
    fn unit01(&mut self) -> f32 {
        // Low 16 bits: `next_rand` returns 31, so a shift sized for a full u32
        // silently halves the range. Same trap warp documents.
        (next_rand(&mut self.rng) & 0xFFFF) as f32 / 65536.0
    }

    #[inline]
    fn chance(&mut self, pct: u32) -> bool {
        next_rand(&mut self.rng) % 100 < pct
    }

    /// Light one sub-cell of the channel, and mark the cells around it as halo
    /// the first time that cell is lit. Dilating here rather than in a pass
    /// over the whole grid keeps this proportional to the bolt, not the panel.
    #[inline]
    fn dot(&mut self, sx: f32, sy: f32) {
        if sx < 0.0 || sy < 0.0 || sx >= self.subw || sy >= self.subh {
            return;
        }
        let (sx, sy) = (sx as usize, sy as usize);
        let (cx, cy) = (sx / 2, sy / 4);
        let i = cy * self.cols + cx;
        if self.dots[i] == 0 {
            for ny in cy.saturating_sub(1)..(cy + 2).min(self.rows) {
                for nx in cx.saturating_sub(1)..(cx + 2).min(self.cols) {
                    self.halo[ny * self.cols + nx] = 1;
                }
            }
        }
        self.dots[i] |= dot_bit(sx & 1, sy & 3);
    }

    /// One segment of channel. `wide` doubles the leader's thickness, which is
    /// what separates the main channel from its forks at a glance.
    fn draw(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, wide: bool) {
        let (dx, dy) = (x1 - x0, y1 - y0);
        let n = (dx.abs().max(dy.abs()).ceil() as usize).max(1);
        for k in 0..=n {
            let t = k as f32 / n as f32;
            let (x, y) = (x0 + dx * t, y0 + dy * t);
            self.dot(x, y);
            if wide {
                self.dot(x + 1.0, y);
            }
        }
    }

    /// Walk one branch to exhaustion, forking as it goes. The leader steers
    /// back toward the target every step, so wander bends the channel instead
    /// of sending it somewhere else; a fork has no target and simply drifts,
    /// which is why forks curl away and the leader does not.
    fn walk(&mut self, mut b: Branch, budget: &mut u32) {
        while b.len > 0.0 && *budget > 0 {
            *budget -= 1;
            b.len -= STEP;
            // One wander rate for every branch. Scaling it by depth was tried
            // and it is what put loops in the picture: the deepest forks got
            // four times the angle step, which no restoring pull holds.
            b.dir += (self.unit01() - 0.5) * self.jitter;
            // Jitter alone is a random walk in ANGLE, and a long enough random
            // walk in angle curls. The leader is steered back toward the point
            // it is striking every step, weakly, so the channel zigzags around
            // its course instead of being a straight line with a wobble on it.
            // A fork is not steered and does not need to be: it is a fraction
            // of what was left of its parent, and over that few dozen steps at
            // this wander rate it has no room to come back on itself.
            if b.depth == 0 {
                let aim = (self.target.1 - b.y).atan2(self.target.0 - b.x);
                b.dir += wrap(aim - b.dir) * LEADER_PULL;
            }
            let (nx, ny) = (b.x + b.dir.cos() * STEP, b.y + b.dir.sin() * STEP);
            self.draw(b.x, b.y, nx, ny, b.depth == 0);
            let out = nx < 0.0 || ny < 0.0 || nx >= self.subw || ny >= self.subh;
            if b.depth > 0 && out {
                return;
            }
            // The leader is CLAMPED rather than cut off, because its own
            // wander can carry it over the rim on the very first step of a
            // bolt that runs close to an edge — and a leader cut off there is
            // a two-cell spark in a corner, once every several bolts. It still
            // ends: on reaching its target, or on running out of length.
            b.x = nx.clamp(0.0, self.subw - 1.0);
            b.y = ny.clamp(0.0, self.subh - 1.0);
            let (nx, ny) = (b.x, b.y);
            // Arriving is what ends the leader. Without this it walks past the
            // target and the steering turns it into a circle around it.
            if b.depth == 0 {
                let (tx, ty) = (self.target.0 - nx, self.target.1 - ny);
                if tx * tx + ty * ty < (STEP * 1.5) * (STEP * 1.5) {
                    return;
                }
            }
            // One rate at every depth, and the tree still thins out, because a
            // fork gets a small fraction of what is LEFT of its parent: a
            // branch a fifth as long meets a fifth as many chances to fork.
            // Fewer forks deep down were also tried as a rate halved per
            // generation, and it changed nothing a test could see.
            if b.depth < MAX_DEPTH
                && self.branches.len() < MAX_BRANCHES
                && self.chance(self.fork_pct)
            {
                let side = if self.unit01() < 0.5 { -1.0 } else { 1.0 };
                let dir = b.dir + side * (0.35 + 0.55 * self.unit01());
                let len = b.len * (0.10 + 0.20 * self.unit01());
                self.branches.push(Branch {
                    x: nx,
                    y: ny,
                    dir,
                    len,
                    depth: b.depth + 1,
                });
            }
        }
    }

    /// Generate the next bolt and reset its envelope. Everything random about a
    /// bolt is drawn here: where it strikes, how far it reaches, how long it
    /// lasts and how it strobes — so no two look alike.
    fn strike(&mut self) {
        self.wait = 0;
        self.dots.fill(0);
        self.halo.fill(0);
        self.branches.clear();
        self.age = 0;

        let (w, h) = (self.subw, self.subh);
        let p = 2.0 * (w + h);
        let t0 = self.unit01() * p;
        let (sx, sy) = perim(t0, w, h);
        // 40%-60% of the way around FROM THE START, not an independent draw:
        // two independent points on the perimeter are often neighbours, and a
        // bolt between them is a spark in one corner. The window is narrow
        // enough that the far point is never on the same edge as the start for
        // any panel shape whose longest edge is under 40% of its perimeter —
        // which is every panel up to about 8:1 — so the bolt crosses the field
        // rather than running along the rim of it.
        let (fx, fy) = perim(t0 + p * (0.4 + 0.2 * self.unit01()), w, h);
        let (tx, ty) = if self.chance(self.air_pct) {
            let m = 0.45 + 0.35 * self.unit01();
            (sx + (fx - sx) * m, sy + (fy - sy) * m)
        } else {
            (fx, fy)
        };
        self.target = (tx, ty);

        let (dx, dy) = (tx - sx, ty - sy);
        let dist = (dx * dx + dy * dy).sqrt();
        let dir = dy.atan2(dx);
        self.branches.push(Branch {
            x: sx,
            y: sy,
            dir,
            // Slack for the wander: the steered path is longer than the
            // straight line, and a leader that runs out of length short of the
            // edge leaves a bolt hanging in the middle of the panel.
            len: dist * 1.6,
            depth: 0,
        });

        let mut budget = STEP_BUDGET;
        while let Some(b) = self.branches.pop() {
            self.walk(b, &mut budget);
        }

        // ±25% on the life, and a fresh strobe pattern. `| >> 1` biases the
        // mask toward set bits, so roughly a quarter of the strobe window is
        // dark — three or four visible return strokes.
        self.life = (self.bolt_frames * (75 + next_rand(&mut self.rng) % 51) / 100).max(1);
        self.peak = 255;
        // How many times this channel fires, rolled once here rather than
        // once per stroke, so a bolt is a fixed thing the moment it is drawn.
        self.strokes_left = 0;
        while self.strokes_left + 1 < MAX_STROKES && self.chance(self.restrike_pct) {
            self.strokes_left += 1;
        }
        self.envelope();
    }

    /// Fire the SAME channel again, dimmer and quicker. The geometry is
    /// untouched — that is the whole point: a re-lit channel is one bolt
    /// flashing twice, where a second `strike` would be two bolts in a row
    /// somewhere else on the panel.
    fn restrike(&mut self) {
        self.wait = 0;
        self.age = 0;
        self.strokes_left -= 1;
        self.peak = self.peak * RESTRIKE_PEAK_PCT / 100;
        self.life = (self.life * RESTRIKE_LIFE_PCT / 100).max(1);
        self.envelope();
    }

    /// The brightness envelope for the stroke that is about to be drawn.
    fn envelope(&mut self) {
        self.flicker = {
            let r = next_rand(&mut self.rng);
            (r | (r >> 1)) | 1
        };
        self.glow_peak = self.peak_glow * self.peak / 255;
        // The wash outlasts the channel: the panel is still faintly lit for a
        // moment after the bolt is gone, which is the afterimage.
        self.glow_life = (self.life * 17 / 10).max(1);
        self.total = self.life.max(self.glow_life);
    }

    fn step(&mut self) {
        if self.wait > 0 {
            self.wait -= 1;
            if self.wait == 0 {
                if self.strokes_left > 0 {
                    self.restrike();
                } else {
                    self.strike();
                }
            }
            return;
        }
        self.age += 1;
        // A re-strike waits only for the CHANNEL to go out, not for the wash:
        // the beat of dark between two flashes is the channel's, and the wash
        // holding through it is what says they are the same bolt.
        if self.strokes_left > 0 && self.age >= self.life {
            self.wait = self.stroke_gap;
        } else if self.age >= self.total {
            let span = self.gap_hi - self.gap_lo + 1;
            self.wait = (self.gap_lo + next_rand(&mut self.rng) % span).max(1);
        }
    }

    /// Channel brightness, 0..=255. Linear decay gated by the strobe.
    #[inline]
    fn core_intensity(&self) -> u32 {
        if self.age >= self.life {
            return 0;
        }
        // `1 - t^2`, not `1 - t`: a linear fall spends its last third in the
        // bottom two ramp steps, which on an 8x16 cell is a thread you cannot
        // see. Squaring holds the channel in the visible half of the ramp for
        // most of the stroke and then drops it off a cliff, which is also
        // closer to how a real channel cools.
        let left = self.life - self.age;
        let base = self.peak * left * (self.life + self.age) / (self.life * self.life);
        if self.age < FLICKER_FRAMES && (self.flicker >> self.age) & 1 == 0 {
            base / 3
        } else {
            base
        }
    }

    /// Afterglow wash, 0..=255. Its own slower decay, and NOT strobed — the
    /// panel-wide wash is scattered light, which does not switch off between
    /// return strokes the way the channel does.
    #[inline]
    fn glow_intensity(&self) -> u32 {
        if self.age >= self.glow_life {
            return 0;
        }
        self.glow_peak * (self.glow_life - self.age) / self.glow_life
    }
}

impl Saver for Zot {
    fn render(&mut self, s: &mut Surface<'_>) {
        self.step();
        let core = self.core_intensity();
        let core_l = level(core, CORE_LEVELS);
        let halo_l = level(core * self.halo_pct / 100, HALO_LEVELS);
        let glow_l = level(self.glow_intensity(), GLOW_LEVELS);

        let halo_cell = Cell::new(font::SOLID, (HALO_BASE + halo_l.max(1) - 1) as u16);
        let glow_cell = if glow_l == 0 {
            Cell::CLEAR
        } else {
            Cell::new(font::SOLID, (GLOW_BASE + glow_l - 1) as u16)
        };
        let (grid, dots, halo, cols) = (&mut self.grid, &self.dots[..], &self.halo[..], self.cols);
        grid.fill(|cx, cy| {
            let i = cy * cols + cx;
            if core_l > 0 && dots[i] != 0 {
                return Cell::new(
                    font::BRAILLE[dots[i] as usize],
                    (CORE_BASE + core_l - 1) as u16,
                );
            }
            if halo_l > 0 && halo[i] != 0 {
                return halo_cell;
            }
            glow_cell
        });
        grid.flush(s, &PAL);
    }

    fn name(&self) -> &'static str {
        "zot"
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

    /// 1080 is not a multiple of 16, so the bottom 8 scanlines belong to no
    /// cell — that strip is what the frame-0 assertion is about. 1280x400 is
    /// the panel the Pi is moving to, and divides exactly.
    const PANELS: [(usize, usize); 2] = [(1920, 1080), (1280, 400)];

    fn panel(wh: (usize, usize)) -> Panel {
        Panel::new(wh.0, wh.1, wh.0)
    }

    /// Deterministic across runs: `new` seeds off the clock, which is right on
    /// the panel and useless in a test that counts what a bolt drew.
    fn seeded(p: &Panel, fps: u32, seed: u32) -> Zot {
        let mut z = Zot::new(p, fps);
        z.rng = seed;
        z.strike();
        z
    }

    /// T1. Frame 0 must cover the panel — including the strip below the last
    /// cell row — and must cover it with the SCENE. A reported black rectangle
    /// satisfies the rows assertion on its own, so count lit pixels too.
    #[test]
    fn frame_zero_paints_the_whole_panel() {
        for &wh in PANELS.iter() {
            let p = panel(wh);
            let mut z = seeded(&p, 15, 0xB0_17_00_01);
            let mut buf = vec![0u32; p.buf_len()];
            let d = saver::frame(&mut z, &mut buf, &p);
            assert_eq!(d.rows(), p.h, "{:?}: frame 0 must paint all of it", wh);
            let lit = buf.iter().filter(|&&px| px != 0).count();
            assert!(lit > 10_000, "{:?}: frame 0 painted nothing ({lit})", wh);
        }
    }

    /// T2. Every scanline whose pixels changed must be inside a damage run, or
    /// simpledrm scans out the previous frame there forever. Long enough to
    /// cover several whole bolts and the gaps between them.
    #[test]
    fn damage_covers_every_changed_scanline() {
        for &wh in PANELS.iter() {
            let p = panel(wh);
            let mut z = seeded(&p, 30, 0xB0_17_00_02);
            let mut buf = vec![0u32; p.buf_len()];
            let mut prev = vec![0u32; p.buf_len()];
            let stride = p.buf_len() / p.h;
            saver::frame(&mut z, &mut buf, &p);

            let mut moved = 0usize;
            for n in 1..600 {
                prev.copy_from_slice(&buf);
                let d = saver::frame(&mut z, &mut buf, &p);
                for y in 0..p.h {
                    let row = y * stride..y * stride + p.w;
                    if buf[row.clone()] != prev[row] {
                        assert!(
                            dump::row_reported(
                                &prev[y * stride..][..p.w],
                                &buf[y * stride..][..p.w],
                                y,
                                &d
                            ),
                            "frame {n}: scanline {y} changed outside every reported rect"
                        );
                        moved += 1;
                    }
                }
            }
            assert!(moved > 0, "nothing ever changed: the check proves nothing");
        }
    }

    // The counting allocator is crate-wide — one test binary, one allocator —
    // so it lives in `testalloc` and every saver's no-alloc test shares it.
    use crate::testalloc::count as allocs;

    /// T3. `render` cannot allocate. Every buffer is sized in `new` and the
    /// frame path only indexes them — but this saver REBUILDS its scene mid-
    /// frame every couple of seconds, and generation is where a `Vec` would
    /// grow: `branches` is pushed onto by the walk, and stays bounded only
    /// because `MAX_BRANCHES` is checked before every push.
    ///
    /// Two checks, because neither is enough alone. The COUNTER catches any
    /// allocation at all, including a scratch `Vec` allocated and dropped
    /// inside the call, leaving no trace behind it. The buffer ADDRESSES catch
    /// what a counter would miss if the allocator were swapped out from under
    /// it: a `dots = vec![...]` in the frame loop keeps both length and
    /// capacity, so those two alone would pass it.
    #[test]
    fn render_never_allocates() {
        let p = Panel::new(320, 200, 320);
        let mut z = Zot::new(&p, 30);
        let mut buf = vec![0u32; p.buf_len()];
        let shape = |z: &Zot| {
            (
                z.dots.len(),
                z.dots.capacity(),
                z.dots.as_ptr(),
                z.halo.len(),
                z.halo.capacity(),
                z.halo.as_ptr(),
                z.branches.capacity(),
                z.branches.as_ptr(),
            )
        };
        let reserved = shape(&z);
        assert_eq!(reserved.6, MAX_BRANCHES, "the work list has no ceiling");
        assert_eq!(
            (reserved.0, reserved.3),
            (reserved.1, reserved.4),
            "a frame buffer is over-allocated: growth in the frame path hides here"
        );
        // Non-vacuous: the counter is wired up and does count.
        let warm = allocs();
        drop(vec![0u8; 8]);
        assert!(allocs() > warm, "the counter is not counting");

        // Long enough to span hundreds of strikes, which is the only thing
        // this saver ever does that could want more memory.
        let mut strikes = 0;
        for _ in 0..20_000 {
            let before = allocs();
            let age = z.age;
            saver::frame(&mut z, &mut buf, &p);
            assert_eq!(allocs(), before, "the render path allocated");
            assert_eq!(shape(&z), reserved, "a frame buffer moved");
            strikes += u32::from(z.age == 0 && age != 0);
        }
        assert!(
            strikes > 100,
            "only {strikes} bolts: generation was barely on the path"
        );
        assert!(
            z.dots.iter().any(|&d| d != 0),
            "20k frames drew no channel into the reserved buffers"
        );
    }

    /// Lit cells of the current bolt, and the box they span, in cells.
    fn extent(z: &Zot) -> (usize, usize, usize) {
        let (mut x0, mut x1, mut y0, mut y1) = (usize::MAX, 0usize, usize::MAX, 0usize);
        let mut n = 0;
        for (i, &d) in z.dots.iter().enumerate() {
            if d == 0 {
                continue;
            }
            n += 1;
            let (cx, cy) = (i % z.cols, i / z.cols);
            x0 = x0.min(cx);
            x1 = x1.max(cx);
            y0 = y0.min(cy);
            y1 = y1.max(cy);
        }
        if n == 0 {
            return (0, 0, 0);
        }
        (n, x1 - x0 + 1, y1 - y0 + 1)
    }

    /// T4. A bolt CROSSES the panel, at either aspect. The arc-length endpoint
    /// draw is the thing under test: weight the edge choice by anything but
    /// edge length and a 1280x400 strip gets bolts that cover a third of it.
    #[test]
    fn a_bolt_crosses_the_panel_at_either_aspect() {
        for &wh in PANELS.iter() {
            let p = panel(wh);
            let mut z = seeded(&p, 30, 0xB0_17_00_04);
            let (mut spans, mut strikes) = (0usize, 0usize);
            for _ in 0..200 {
                z.strike();
                let (n, w, h) = extent(&z);
                assert!(n > 20, "{:?}: a bolt lit {n} cells", wh);
                // Reaching over half of the longer axis is "crossed it": an
                // air-terminated bolt stops around halfway by construction.
                if w * 2 > z.cols || h * 2 > z.rows {
                    spans += 1;
                }
                strikes += 1;
            }
            assert!(
                spans * 10 >= strikes * 9,
                "{:?}: only {spans}/{strikes} bolts crossed the panel",
                wh
            );
        }
    }

    /// T5. It BRANCHES, and the branching thins with depth. A leader that
    /// never forks is a line, and one that forks at a flat rate is a bush —
    /// both still cross the panel and pass T4.
    #[test]
    fn forks_thin_out_with_depth() {
        let p = panel(PANELS[0]);
        let mut z = seeded(&p, 30, 0xB0_17_00_05);
        let mut by_depth = [0usize; MAX_DEPTH as usize + 1];
        for _ in 0..100 {
            z.strike();
            // Re-walk the same tree, counting what got pushed.
            z.dots.fill(0);
            z.halo.fill(0);
            z.branches.clear();
            let mut budget = STEP_BUDGET;
            z.branches.push(Branch {
                x: z.subw / 2.0,
                y: 0.0,
                dir: std::f32::consts::FRAC_PI_2,
                len: z.subh,
                depth: 0,
            });
            z.target = (z.subw / 2.0, z.subh - 1.0);
            while let Some(b) = z.branches.pop() {
                by_depth[b.depth as usize] += 1;
                z.walk(b, &mut budget);
            }
        }
        assert!(by_depth[1] > 50, "the leader never forked: {by_depth:?}");
        assert!(
            by_depth[2] > 0,
            "forking stopped at one generation: {by_depth:?}"
        );
        for d in 1..MAX_DEPTH as usize {
            assert!(
                by_depth[d] >= by_depth[d + 1],
                "generation {d} is not thicker than {}: {by_depth:?}",
                d + 1
            );
        }
        assert!(
            by_depth[MAX_DEPTH as usize] > 0,
            "nothing reached the last generation: {by_depth:?}"
        );
    }

    /// T6. The gap is real: after a bolt the panel goes fully black, stays
    /// black for a long stretch, and costs nothing while it does. The wash
    /// holding one quantised level for a few frames is also free — those
    /// frames report no damage while still lit, which is why "dark" here is
    /// the BUFFER being black and not the damage being empty.
    #[test]
    fn the_panel_goes_dark_between_bolts() {
        let p = panel(PANELS[0]);
        let mut z = seeded(&p, 30, 0xB0_17_00_06);
        let mut buf = vec![0u32; p.buf_len()];
        let (mut dark, mut run, mut longest, mut idle_dark) = (0usize, 0usize, 0usize, 0usize);
        for _ in 0..900 {
            let d = saver::frame(&mut z, &mut buf, &p);
            if buf.iter().all(|&px| px == 0) {
                dark += 1;
                run += 1;
                longest = longest.max(run);
                if run > 1 {
                    idle_dark += 1;
                    assert!(
                        d.is_empty(),
                        "a black frame after a black one cost {} rows",
                        d.rows()
                    );
                }
            } else {
                run = 0;
            }
        }
        // Over half the run, with the defaults as they stand (563/900 as
        // written). The bar moves with ZOT_GAP_*: shorten the gap enough and
        // this is the test that says so.
        assert!(dark > 450, "only {dark}/900 frames were black");
        // The long gap, not the beat between two strokes of one bolt: that
        // one is `stroke_gap` frames and much shorter. Tied to the configured
        // gap rather than to a number, so the two cannot be confused.
        assert!(
            longest as u32 >= z.gap_lo,
            "the longest dark run was {longest}, under the {} frame gap floor",
            z.gap_lo
        );
        assert!(
            idle_dark > 200,
            "{idle_dark} free frames: the gap is not free"
        );
    }

    /// T13. A bolt flashes more than once, down the SAME channel. Re-lighting
    /// the channel is what makes a bolt read as lightning rather than as a
    /// line fading out, and "same channel" is the half that matters: draw a
    /// fresh bolt instead and it is two strikes in a row somewhere else.
    #[test]
    fn a_bolt_flashes_again_down_the_same_channel() {
        let p = panel(PANELS[0]);
        let mut z = seeded(&p, 30, 0xB0_17_00_0D);
        let (mut multi, mut bolts) = (0usize, 0usize);
        for _ in 0..80 {
            z.strike();
            let channel = z.dots.clone();
            let expect = z.strokes_left + 1;
            let (mut flashes, mut lit, mut dark, mut longest_dark) = (1u32, true, 0u32, 0u32);
            loop {
                z.step();
                // A long wait with nothing owed is the gap: this bolt is over.
                if z.wait > 0 && z.strokes_left == 0 {
                    break;
                }
                let on = z.core_intensity() > 0;
                if on && !lit {
                    flashes += 1;
                }
                dark = if on { 0 } else { dark + 1 };
                longest_dark = longest_dark.max(dark);
                lit = on;
            }
            assert_eq!(z.dots, channel, "a re-strike moved the channel");
            assert_eq!(
                flashes, expect,
                "a bolt owed {expect} strokes, saw {flashes}"
            );
            if flashes > 1 {
                multi += 1;
                assert!(
                    longest_dark < z.gap_lo,
                    "{longest_dark} dark frames inside one bolt: that is a gap, \
                     not a flicker"
                );
            }
            bolts += 1;
        }
        assert!(
            multi * 2 > bolts,
            "only {multi}/{bolts} bolts flashed more than once"
        );
    }

    /// T7. Generation is bounded however the dice fall. Forking at every step,
    /// to every depth, must still terminate inside the budget and inside the
    /// work list — an unbounded branching process has a tail that eventually
    /// lands a strike that misses a frame deadline.
    #[test]
    fn generation_stays_inside_its_budget() {
        let p = panel(PANELS[0]);
        let mut z = seeded(&p, 30, 0xB0_17_00_07);
        z.fork_pct = 100;
        z.jitter = 1.5;
        for _ in 0..300 {
            z.strike();
            assert!(z.branches.len() <= MAX_BRANCHES);
            assert_eq!(z.branches.capacity(), MAX_BRANCHES, "the work list grew");
        }
    }

    /// T8. Halo is where the channel is not. A halo cell that also carries
    /// channel dots is fine (the channel wins in `render`), but a channel cell
    /// with no halo around it means the dilation missed, and the bolt then has
    /// a hard edge instead of light coming off it.
    #[test]
    fn every_channel_cell_is_surrounded_by_halo() {
        let p = panel(PANELS[1]);
        let mut z = seeded(&p, 30, 0xB0_17_00_08);
        for _ in 0..50 {
            z.strike();
            for i in 0..z.dots.len() {
                if z.dots[i] == 0 {
                    continue;
                }
                let (cx, cy) = (i % z.cols, i / z.cols);
                for ny in cy.saturating_sub(1)..(cy + 2).min(z.rows) {
                    for nx in cx.saturating_sub(1)..(cx + 2).min(z.cols) {
                        assert_eq!(z.halo[ny * z.cols + nx], 1, "no halo at ({nx},{ny})");
                    }
                }
            }
        }
    }

    /// T9. The strobe is a strobe: over one bolt the channel must both dim and
    /// come BACK, or it is a plain fade with a random mask that never fires.
    #[test]
    fn the_channel_strobes_before_it_dies() {
        let p = panel(PANELS[0]);
        let mut z = seeded(&p, 60, 0xB0_17_00_09);
        let mut rebrightened = 0;
        for _ in 0..40 {
            z.strike();
            let mut last = z.core_intensity();
            for _ in 0..z.life {
                z.step();
                let now = z.core_intensity();
                if now > last {
                    rebrightened += 1;
                }
                last = now;
            }
        }
        assert!(
            rebrightened > 40,
            "only {rebrightened} return strokes in 40 bolts"
        );
    }

    /// T10. The wash OUTLASTS the channel. Scattered light does not stop when
    /// the channel does, and the moment where the bolt is gone but the panel
    /// is still faintly lit is the whole afterglow — tie the two lifetimes
    /// together and there is no afterimage, just an abrupt cut to black.
    #[test]
    fn the_wash_outlasts_the_channel() {
        let p = panel(PANELS[0]);
        let mut z = seeded(&p, 30, 0xB0_17_00_0A);
        for _ in 0..40 {
            z.strike();
            while z.core_intensity() > 0 {
                z.step();
            }
            assert!(
                z.glow_intensity() > 0,
                "the wash died with the channel at age {}",
                z.age
            );
        }
    }

    /// T11. The channel is PAINTED, and painted brighter than its halo. Every
    /// channel cell is also a halo cell, so a render that lost the channel
    /// arm entirely still lights the same region, still reports the same
    /// damage and still goes dark on cue — it just has no bolt in it. Only
    /// the pixels say so.
    #[test]
    fn the_channel_is_painted_brighter_than_its_halo() {
        let p = panel(PANELS[0]);
        let mut z = Zot::new(&p, 30);
        z.rng = 0xB0_17_00_0B;
        let mut buf = vec![0u32; p.buf_len()];
        // `new` arms a one-frame wait, so THIS frame is the strike at age 0:
        // full brightness, top of every ramp.
        saver::frame(&mut z, &mut buf, &p);
        assert_eq!(z.age, 0, "the first frame is not the strike");

        // The level each ramp actually reaches at peak, which for the halo and
        // the wash is short of the top of their ramp — the top is where
        // ZOT_HALO_PCT=100 and ZOT_GLOW_PCT=100 take them.
        let core = PAL[CORE_BASE + level(255, CORE_LEVELS) - 1];
        let halo = PAL[HALO_BASE + level(255 * z.halo_pct / 100, HALO_LEVELS) - 1];
        let glow = PAL[GLOW_BASE + level(z.peak_glow, GLOW_LEVELS) - 1];
        let n = |v: u32| buf.iter().filter(|&&px| px == v).count();
        let (nc, nh, ng) = (n(core), n(halo), n(glow));
        assert!(nc > 500, "no channel on the panel ({nc} px)");
        assert!(
            nh > nc,
            "the halo ({nh} px) is not fatter than the channel ({nc})"
        );
        assert!(ng > nh * 4, "the wash ({ng} px) is not the whole panel");
        // And they are three distinct colours, brightest first.
        let lum = |v: u32| (v >> 16 & 0xFF) + (v >> 8 & 0xFF) + (v & 0xFF);
        assert!(lum(core) > lum(halo) && lum(halo) > lum(glow));
    }

    /// T12. A fork GOES somewhere. A fork is a random walk in angle with
    /// nothing steering it, and a long enough random walk in angle spirals:
    /// the branch keeps drawing and stops covering ground, and the bolt comes
    /// out as a bundle of curls and loops. What keeps it honest is that a fork
    /// is only a fraction of what was left of its parent — so this pins the
    /// ground a fork covers against the channel it draws, which is the thing
    /// that goes wrong if forks are ever handed more length.
    #[test]
    fn a_fork_goes_somewhere_instead_of_curling() {
        let p = panel(PANELS[0]);
        let mut z = seeded(&p, 30, 0xB0_17_00_0C);
        // The DEEPEST generation, because jitter is multiplied by depth: that
        // is where the heading walks fastest and where a lost restoring pull
        // shows up as a loop rather than as a slightly bent branch.
        let len = 180.0f32;
        let mut total = 0.0f64;
        const TRIALS: usize = 60;
        for k in 0..TRIALS {
            z.dots.fill(0);
            z.halo.fill(0);
            z.branches.clear();
            let dir = k as f32 * std::f32::consts::TAU / TRIALS as f32;
            let mut budget = STEP_BUDGET;
            let b = Branch {
                x: z.subw / 2.0,
                y: z.subh / 2.0,
                dir,
                len,
                depth: MAX_DEPTH,
            };
            z.walk(b, &mut budget);
            let (_, w, h) = extent(&z);
            // Cells, so scale back to the sub-cells `len` counts in.
            let (w, h) = ((w * 2) as f32, (h * 4) as f32);
            total += (w * w + h * h).sqrt() as f64 / len as f64;
        }
        let reach = total / TRIALS as f64;
        assert!(
            reach > 0.6,
            "a fork covered {reach:.2} of its own length: it is curling, not \
             branching (0.79 as written; 0.36 with the wander rate scaled by \
             depth, which is what this is here to keep out)"
        );
    }

    /// Every colour index a cell can address must exist: one off the end is an
    /// index panic inside `Grid::blit`, on the panel, weeks in.
    #[test]
    fn every_reachable_colour_index_is_in_the_palette() {
        for &(_, base, levels) in RAMPS.iter() {
            assert_eq!(level(255, levels), levels);
            assert_eq!(level(0, levels), 0);
            assert!(base + levels <= PAL_LEN);
            assert_ne!(PAL[base + levels - 1], 0, "the brightest level is black");
        }
        assert_eq!(GLOW_BASE + GLOW_LEVELS, PAL_LEN);
    }

    /// The perimeter walk is what makes this aspect-agnostic: it must stay
    /// inside the field and it must actually reach all four edges.
    #[test]
    fn the_perimeter_walk_covers_every_edge_and_stays_inside() {
        for (w, h) in [(480.0f32, 268.0f32), (320.0, 100.0)] {
            let p = 2.0 * (w + h);
            let (mut top, mut right, mut bottom, mut left) = (0, 0, 0, 0);
            for k in 0..4000 {
                let (x, y) = perim(p * k as f32 / 4000.0, w, h);
                assert!((0.0..w).contains(&x) && (0.0..h).contains(&y), "{x},{y}");
                if y == 0.0 {
                    top += 1;
                }
                if x == w - 1.0 {
                    right += 1;
                }
                if y == h - 1.0 {
                    bottom += 1;
                }
                if x == 0.0 {
                    left += 1;
                }
            }
            assert!(
                top > 0 && right > 0 && bottom > 0 && left > 0,
                "{w}x{h}: edges hit {top}/{right}/{bottom}/{left}"
            );
        }
    }
}
