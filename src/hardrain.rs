//! Hard Rain — a downpour. Steep gusting streaks, a mist the sky is veiled in,
//! squalls sweeping across, and standing water at the bottom that ripples where
//! the rain lands and runs sideways.
//!
//! # How this differs from `rain`
//!
//! `rain` is the drizzle: a few hundred near-vertical streaks over black, a
//! constant lean, and a one-cell splash on a reserved ground row. Everything
//! below is the storm version of a decision `rain` made the gentle way.
//!
//! * **The wind is a variable, not a constant.** Two incommensurate sines drive
//!   the slant, so the whole field leans one way, slackens, and leans back. A
//!   streak is re-derived from its head every frame, so a gust re-leans the
//!   rain that is already falling instead of only the rain that spawns next.
//! * **Four tiers, and the nearest one is a MIST.** The dimmest tier is
//!   one-to-two sub-rows of speck at six-sixteenths of the draw, so the sky is
//!   veiled rather than black between streaks. `rain` has three tiers and no
//!   veil, which is why it reads as weather you would walk through.
//! * **Squalls.** A band of columns sweeps across; inside it every streak
//!   is drawn a whole depth tier brighter and half again longer. That is the sheet of heavier
//!   rain, and it is the thing that makes the scene have weather in it rather
//!   than a steady rate.
//! * **Water, not a splash row.** The bottom is a pool several rows deep with a
//!   1D wave on its surface: an impact digs a dip, the dip runs out both ways
//!   and reflects off the edges, and a moving crest catches foam. Near rain
//!   also throws spray that arcs up and falls back.
//!
//! # Damage model: full repaint (Model A)
//!
//! `rain` is sparse and hands `flush_sparse` a dirty list. That model is fast
//! and it CAN under-report: a cell written into the grid but left out of the
//! list freezes on the panel forever, and a framebuffer diff is structurally
//! blind to it. Nothing here is sparse anyway — the mist, the squall and the
//! pool all touch broad regions every frame — so the scene is accumulated into
//! a per-cell field and handed to `Grid::flush`, which derives damage from a
//! u32 compare per cell and cannot under-report.
//!
//! Two fields, rebuilt from zero every frame: `pat` (braille dots OR'd
//! together) and `col` (palette index, kept at its MAXIMUM). Because they are
//! rebuilt rather than patched, there is no erase pass, no per-drop cell list
//! and no trail bug to have — the two `rain` spends most of its module doc on.
//!
//! # Why the palette is ordered by depth
//!
//! `col[i] = col[i].max(c)` is the whole compositor, and it only composites
//! correctly if a higher index means NEARER. So the table runs mist, far, mid,
//! near, then the water (which is in front of all the rain), then spray (which
//! is in front of the water). `the_palette_is_ordered_by_depth` pins it.
//!
//! # Sub-cell geometry
//!
//! Braille, for the reason `rain` gives: unifont's `|` stops two pixels short
//! of its cell, so stacked bars are a column of dashes, and a drop moving a
//! whole cell a frame jumps 16px. Braille in this atlas is FILLED 2x4
//! quadrants, so a cell is four stackable 4x16 quarters and a slant is a lean
//! rather than a staircase.
//!
//! # Both aspect ratios
//!
//! Nothing here is in absolute rows. Speed is hundredths of a PANEL HEIGHT per
//! second, streak length is a percent of panel height, and the pool is a
//! percent of rows — so 1280x400 gets short streaks over a shallow pool and
//! 1920x1080 gets long ones, and a fall takes about the same time on both. A
//! vertical streak on a 1280x400 panel is a quarter of the screen tall if you
//! size it in cells; that is what the percentages are for, and the steep
//! default slant is what makes a streak read ACROSS a panel that wide.
//!
//! # Environment
//!
//! * `HARDRAIN_CELL_W` / `HARDRAIN_CELL_H` — cell in px (8, 16). 8x16 is the
//!   braille cell's own size; anything else stretches the quadrants.
//! * `HARDRAIN_DENSITY` — drops per 1000 cells, 0..=400 (default 55)
//! * `HARDRAIN_SPEED` — hundredths of a panel height per second, 10..=1000
//!   (default 110)
//! * `HARDRAIN_WIND` — base slant, cells sideways per 100 cells of fall,
//!   -300..=300 (default 115). Signed: negative blows the other way.
//! * `HARDRAIN_GUST` — gust swing in the same units, 0..=300 (default 50).
//!   The base stays well clear of `rain`'s 34 even at the slack end of the
//!   cycle: a gust that slackened to a drizzle's lean reads as the other saver.
//! * `HARDRAIN_GUST_SECS` — seconds in the main gust cycle, 1..=600 (default 11)
//! * `HARDRAIN_POOL_PCT` — pool depth as a percent of rows, 0..=40 (default 9).
//!   0 turns the water off.
//! * `HARDRAIN_SQUALL_SECS` — mean seconds between squalls, 0..=600
//!   (default 17). 0 turns them off.
//! * `HARDRAIN_SPRAY` — spray droplets thrown per near impact, 0..=4 (default 2)
//!
//! # Per-frame cost
//!
//! A full repaint every frame, so this is in `moire`'s class rather than
//! `rain`'s — see the README for the measured ratio. The Pi budget is 500m and
//! a storm is allowed to cost more than a drizzle.

use crate::font;
use crate::grid::{bake, dot_bit, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, next_rand};

/// Sub-rows per cell: a braille cell is 2x4 filled quadrants.
const SUB: i32 = 4;

/// The braille bit for (half of the cell, quarter down it), as a table because
/// the hot loops index it rather than calling per dot.
const BITS: [[u8; SUB as usize]; 2] = [
    [dot_bit(0, 0), dot_bit(0, 1), dot_bit(0, 2), dot_bit(0, 3)],
    [dot_bit(1, 0), dot_bit(1, 1), dot_bit(1, 2), dot_bit(1, 3)],
];

/// Both halves of one quarter — a full-width band, which is what the pool is
/// made of.
const BAND: [u8; SUB as usize] = [
    BITS[0][0] | BITS[1][0],
    BITS[0][1] | BITS[1][1],
    BITS[0][2] | BITS[1][2],
    BITS[0][3] | BITS[1][3],
];

/// Ordered by DEPTH, nearest last — `col[i].max(c)` is the compositor and that
/// ordering is what makes it correct. Mist through near is the rain; then the
/// water, which is in front of all of it; then spray, which is in front of the
/// water.
#[rustfmt::skip]
const PAL_RGB: [[u8; 3]; 14] = [
    [0x00, 0x00, 0x00], //  0 background
    [0x16, 0x1E, 0x2A], //  1 mist body — the veil, barely above black
    [0x22, 0x2E, 0x3C], //  2 mist head
    [0x24, 0x33, 0x46], //  3 far body
    [0x3A, 0x50, 0x6A], //  4 far head
    [0x56, 0x74, 0x94], //  5 mid body
    [0x82, 0xA6, 0xC6], //  6 mid head
    [0xA6, 0xC6, 0xDE], //  7 near body
    [0xF4, 0xFC, 0xFF], //  8 near head — the only near-white in the sky
    [0x14, 0x24, 0x2E], //  9 water, deep
    [0x24, 0x48, 0x5A], // 10 water, body
    [0x4A, 0x84, 0x98], // 11 water, just under the surface
    [0xB4, 0xDC, 0xEA], // 12 foam on a moving crest
    [0xDC, 0xF0, 0xFA], // 13 spray thrown off an impact
];
const PAL: [u32; 14] = bake(&PAL_RGB);

const C_WATER_DEEP: u8 = 9;
const C_WATER_BODY: u8 = 10;
const C_WATER_SURF: u8 = 11;
const C_FOAM: u8 = 12;
const C_SPRAY: u8 = 13;

/// One depth. Speed, length, brightness and whether it disturbs the water all
/// move together — a far streak that was merely dim but as fast and as heavy as
/// a near one reads as a dim near streak, not as distance.
struct Tier {
    /// Percent of the base fall speed.
    speed_pct: i32,
    /// Length as a percent of PANEL HEIGHT, floored at `len_min` sub-rows so a
    /// short panel still has streaks rather than specks.
    len_pct: i32,
    len_min: u8,
    len_span: u8,
    body: u8,
    head: u8,
    /// Impulse into the water surface on impact, in 8.8 sub-rows. 0 means this
    /// tier lands behind the scene and disturbs nothing.
    impact: i32,
    /// Only the nearest tier throws spray: a droplet arcing off a raindrop you
    /// cannot individually see is noise.
    spray: bool,
}

#[rustfmt::skip]
const TIERS: [Tier; 4] = [
    Tier { speed_pct:  45, len_pct: 0, len_min: 1, len_span: 2, body: 1, head: 2, impact:   0, spray: false },
    Tier { speed_pct:  62, len_pct: 3, len_min: 3, len_span: 3, body: 3, head: 4, impact:   0, spray: false },
    Tier { speed_pct:  80, len_pct: 5, len_min: 5, len_span: 4, body: 5, head: 6, impact: 144, spray: false },
    Tier { speed_pct: 100, len_pct: 8, len_min: 8, len_span: 6, body: 7, head: 8, impact: 352, spray: true  },
];

/// Weighted draw, mist-heavy: the veil is most of the draw and the near streaks
/// are the exception. A uniform draw gives a curtain of equally near streaks,
/// which is the flat look this is avoiding.
#[rustfmt::skip]
const TIER_DRAW: [u8; 16] = [0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 2, 2, 2, 3, 3];

const _: () = {
    let mut i = 0;
    while i < TIERS.len() {
        assert!(TIERS[i].len_span >= 1, "a tier has no length spread");
        assert!(
            (TIERS[i].body as usize) < PAL.len() && (TIERS[i].head as usize) < PAL.len(),
            "a tier colours outside the palette"
        );
        assert!(
            PAL_RGB[TIERS[i].head as usize][2] > PAL_RGB[TIERS[i].body as usize][2],
            "a tier's head is no brighter than its body"
        );
        // Depth is speed AND length AND brightness together, and the palette
        // index has to climb with it or `col.max` composites the wrong one in
        // front.
        assert!(
            i == 0
                || (TIERS[i].body > TIERS[i - 1].head
                    && TIERS[i].speed_pct > TIERS[i - 1].speed_pct
                    && TIERS[i].len_min > TIERS[i - 1].len_min
                    && PAL_RGB[TIERS[i].body as usize][2] > PAL_RGB[TIERS[i - 1].head as usize][2]),
            "the tiers do not climb in index, speed, length and brightness together"
        );
        i += 1;
    }
    assert!(
        TIERS[TIERS.len() - 1].head < C_WATER_DEEP,
        "the rain is not behind the water"
    );
    // `col[i].max(c)` is the whole compositor, so "in front" and "later in the
    // table" have to be the same thing, all the way up.
    assert!(
        C_WATER_DEEP < C_WATER_BODY
            && C_WATER_BODY < C_WATER_SURF
            && C_WATER_SURF < C_FOAM
            && C_FOAM < C_SPRAY,
        "the water and the spray are not in depth order"
    );
    assert!(C_SPRAY as usize == PAL.len() - 1, "spray is not in front");
    let mut i = 0;
    while i < TIER_DRAW.len() {
        assert!(
            (TIER_DRAW[i] as usize) < TIERS.len(),
            "a tier draw names a tier that does not exist"
        );
        i += 1;
    }
};

/// One streak. Positions are 8.8 fixed point in CELLS: a drop that moved a
/// whole number of cells a frame would quantise every speed to the same few
/// values and the tiers would stop differing.
#[derive(Clone, Copy)]
struct Drop {
    x: i32,
    y: i32,
    /// Cells per frame, down. At least 1/256 so nothing ever stalls.
    vy: i32,
    /// Length in sub-rows.
    len: u8,
    tier: u8,
    /// Hit the water already, so a streak that takes several frames to clear
    /// the surface disturbs it once and not five times.
    landed: bool,
}

/// A droplet thrown off an impact: a single sub-cell dot on a ballistic arc.
#[derive(Clone, Copy)]
struct Spray {
    x: i32,
    y: i32,
    vx: i32,
    vy: i32,
    live: bool,
}

pub struct HardRain {
    grid: Grid,
    cols: i32,
    rows: i32,
    /// The scene, rebuilt from zero every frame. `pat` is the braille dots the
    /// cell holds, `col` the nearest thing that claimed it.
    pat: Vec<u8>,
    col: Vec<u8>,
    drops: Vec<Drop>,
    spray: Vec<Spray>,
    /// Water surface displacement per column, 8.8 SUB-ROWS, positive is up.
    /// `hp` is the previous frame — the wave equation needs both.
    h: Vec<i32>,
    hp: Vec<i32>,
    /// First grid row of the pool. `== rows` when the water is off.
    water_top: i32,
    /// Base lean and the swing around it, cells sideways per sub-row of fall,
    /// 8.8. `slant` is this frame's answer.
    wind_base: i32,
    gust_amp: i32,
    slant: i32,
    gust_p: f32,
    gust_dp: f32,
    gust2_p: f32,
    gust2_dp: f32,
    /// The squall band: centre in 8.8 cells, and how wide half of it is.
    squall_x: i32,
    squall_vx: i32,
    squall_half: i32,
    squall_on: bool,
    /// Frames until the next squall, and the mean to redraw it from.
    squall_wait: i32,
    squall_mean: i32,
    vy_base: i32,
    spray_n: u8,
    /// Only the squall needs it, to size a crossing in seconds.
    fps: i32,
    rng: u32,
}

/// Two full turns of the primary gust, so the phase accumulator can be wrapped
/// with one subtract rather than a `%`.
const TAU: f32 = std::f32::consts::TAU;

impl HardRain {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        let cell_w = env_num(&["HARDRAIN_CELL_W"], 8, 4, 64) as usize;
        let cell_h = env_num(&["HARDRAIN_CELL_H"], 16, 4, 128) as usize;
        let grid = Grid::new(panel, cell_w, cell_h);
        let (cols, rows) = (grid.cols() as i32, grid.rows() as i32);

        // Per THOUSAND CELLS rather than an absolute count, so the same number
        // reads the same on any panel. 55 is about 880 drops at 1920x1080 —
        // five times `rain`, which is what a downpour costs.
        let per_mille = env_num(&["HARDRAIN_DENSITY"], 55, 0, 400);
        // Hundredths of a PANEL HEIGHT per second. 110 crosses the panel in a
        // little under a second whatever shape it is.
        let speed = env_num(&["HARDRAIN_SPEED"], 110, 10, 1000) as i32;
        let wind = env_num(&["HARDRAIN_WIND"], 115, -300, 300) as i32;
        let gust = env_num(&["HARDRAIN_GUST"], 50, 0, 300) as i32;
        let gust_secs = env_num(&["HARDRAIN_GUST_SECS"], 11, 1, 600) as f32;
        let pool_pct = env_num(&["HARDRAIN_POOL_PCT"], 9, 0, 40) as i32;
        let squall_secs = env_num(&["HARDRAIN_SQUALL_SECS"], 17, 0, 600) as i32;
        let spray_n = env_num(&["HARDRAIN_SPRAY"], 2, 0, 4) as u8;

        let fps = fps.max(1) as i32;
        let n = ((cols as i64 * rows as i64) * per_mille / 1000).max(1) as usize;
        // Cells per frame = rows * (speed/100) / fps, in 8.8.
        let vy_base = ((rows * speed * 256) / (100 * fps)).max(1);
        let pool_rows = if pool_pct == 0 {
            0
        } else {
            (rows * pool_pct / 100).clamp(1, (rows - 1).max(0))
        };
        // A spray budget rather than a per-impact allocation. Every near drop
        // could land on the same frame; in practice a handful do.
        let spray_cap = (n / 4 + 32) * spray_n.max(1) as usize;

        let to_slant = |cells_per_100: i32| cells_per_100 * 256 / 100 / SUB;

        let mut r = Self {
            grid,
            cols,
            rows,
            pat: vec![0; (cols * rows) as usize],
            col: vec![0; (cols * rows) as usize],
            drops: Vec::with_capacity(n),
            spray: Vec::with_capacity(spray_cap),
            h: vec![0; cols.max(1) as usize],
            hp: vec![0; cols.max(1) as usize],
            water_top: rows - pool_rows,
            wind_base: to_slant(wind),
            gust_amp: to_slant(gust),
            slant: to_slant(wind),
            gust_p: 0.0,
            gust_dp: TAU / (gust_secs * fps as f32),
            gust2_p: 1.7,
            // Deliberately not a ratio of the first: two cycles that share a
            // period give one clean sine and the wind becomes predictable.
            gust2_dp: TAU / (gust_secs * 0.37 * fps as f32),
            squall_x: 0,
            squall_vx: 0,
            squall_half: (cols * 18 / 100).max(1),
            squall_on: false,
            squall_wait: 0,
            squall_mean: if squall_secs == 0 {
                0
            } else {
                (squall_secs * fps).max(1)
            },
            vy_base,
            spray_n,
            fps,
            rng: 0xD0_17_5A_1E,
        };
        r.squall_wait = r.squall_mean;
        for _ in 0..n {
            let mut d = Drop {
                x: 0,
                y: 0,
                vy: vy_base,
                len: 1,
                tier: 0,
                landed: false,
            };
            r.respawn(&mut d);
            // Spread over the whole panel, so frame 0 is a storm already
            // falling rather than a clear sky that fills in over a second.
            d.y = (next_rand(&mut r.rng) as i32 % rows.max(1)) << 8;
            r.drops.push(d);
        }
        for _ in 0..spray_cap {
            r.spray.push(Spray {
                x: 0,
                y: 0,
                vx: 0,
                vy: 0,
                live: false,
            });
        }
        r
    }

    fn respawn(&mut self, d: &mut Drop) {
        let tier = TIER_DRAW[next_rand(&mut self.rng) as usize % TIER_DRAW.len()];
        let t = &TIERS[tier as usize];
        d.tier = tier;
        let base = ((self.rows * SUB * t.len_pct / 100) as u8).max(t.len_min);
        d.len = base.saturating_add((next_rand(&mut self.rng) % t.len_span as u32) as u8);
        // +/-25% on top of the tier, so two near streaks are not in lockstep.
        let jitter = 75 + (next_rand(&mut self.rng) % 51) as i32;
        d.vy = ((self.vy_base * t.speed_pct / 100) * jitter / 100).max(1);
        // Half of what respawns during a squall respawns INTO it, so the band
        // is genuinely more water and not only brighter water. The band moves,
        // so this fills in behind the front over a second or so rather than
        // switching on — which is what a sheet arriving looks like.
        let span = if self.squall_on && next_rand(&mut self.rng) & 1 == 0 {
            let lo = (self.squall_x >> 8) - self.squall_half;
            lo + (next_rand(&mut self.rng) as i32 % (self.squall_half * 2).max(1))
        } else {
            next_rand(&mut self.rng) as i32 % self.cols.max(1)
        };
        d.x = span.rem_euclid(self.cols.max(1)) << 8;
        // The whole streak above the top edge plus a random gap, or every drop
        // that respawns on the same frame enters in one rank.
        d.y = -((d.len as i32) << 6) - ((next_rand(&mut self.rng) as i32 % 40) << 8);
        d.landed = false;
    }

    /// This frame's lean. Two sines that do not share a period, so the wind
    /// wanders rather than swinging metronomically.
    fn gust(&mut self) {
        self.gust_p += self.gust_dp;
        if self.gust_p >= TAU {
            self.gust_p -= TAU;
        }
        self.gust2_p += self.gust2_dp;
        if self.gust2_p >= TAU {
            self.gust2_p -= TAU;
        }
        let g = self.gust_p.sin() * 0.65 + self.gust2_p.sin() * 0.35;
        self.slant = self.wind_base + (self.gust_amp as f32 * g) as i32;
    }

    /// Move the squall band, or count down to the next one. The band enters
    /// from the upwind side and leaves the other, so it reads as weather
    /// arriving rather than fading up in place.
    fn squall(&mut self) {
        if self.squall_mean == 0 {
            return;
        }
        if self.squall_on {
            self.squall_x += self.squall_vx;
            let span = (self.cols << 8) + (self.squall_half << 9);
            if self.squall_x < -(self.squall_half << 8) || self.squall_x > span {
                self.squall_on = false;
                // Half the mean plus a full mean of jitter, so squalls are not
                // a metronome either.
                self.squall_wait =
                    self.squall_mean / 2 + (next_rand(&mut self.rng) as i32 % self.squall_mean);
            }
            return;
        }
        self.squall_wait -= 1;
        if self.squall_wait > 0 {
            return;
        }
        self.squall_on = true;
        // Downwind, and across the whole panel plus both margins in ~6s.
        let frames = (6 * self.fps).max(1);
        let travel = (self.cols << 8) + (self.squall_half << 9);
        let step = (travel / frames).max(1);
        if self.slant >= 0 {
            self.squall_x = -(self.squall_half << 8);
            self.squall_vx = step;
        } else {
            self.squall_x = travel;
            self.squall_vx = -step;
        }
    }

    #[inline]
    fn in_squall(&self, x: i32) -> bool {
        self.squall_on && (x - self.squall_x).abs() < (self.squall_half << 8)
    }

    /// Stamp one streak into the field, head first, clipped to the sky. Returns
    /// the column it landed in, if this is the frame it reached the water.
    fn stamp_drop(&mut self, d: &Drop) -> Option<i32> {
        let t = &TIERS[d.tier as usize];
        // A whole TIER, not a shade: the palette is two entries per tier, so
        // +2 draws mist as far rain and far as mid. A one-step bump was tried
        // and is invisible — most of the field is mist, and mist one shade
        // brighter is still black.
        let boost = if self.in_squall(d.x) { 2 } else { 0 };
        let (body, head) = (
            (t.body + boost).min(C_WATER_DEEP - 1),
            (t.head + boost).min(C_WATER_DEEP - 1),
        );
        // And two and a half times as long. Brightness alone was measured at
        // about 20% more luminance in the band against 30% frame-to-frame
        // noise — invisible. Length is what puts water in the band.
        let len = if boost > 0 {
            (d.len as i32) * 5 / 2
        } else {
            d.len as i32
        };
        let top = d.y >> 6;
        let (cols, slant) = (self.cols.max(1), self.slant);
        for k in 0..len {
            let s = top - k;
            let r = s >> 2;
            if r < 0 {
                // Sub-rows walk upward, so everything left is off the top too.
                break;
            }
            if r >= self.water_top {
                continue;
            }
            let xs = d.x - k * slant;
            let c = (xs >> 8).rem_euclid(cols);
            // Bit 7 of the fraction: which half of the cell the streak is in.
            // This is what makes a slant a lean and not a staircase.
            let bit = BITS[((xs >> 7) & 1) as usize][(s & 3) as usize];
            let i = (r * cols + c) as usize;
            self.pat[i] |= bit;
            let want = if k == 0 { head } else { body };
            self.col[i] = self.col[i].max(want);
        }
        (!d.landed && (top >> 2) >= self.water_top && t.impact > 0)
            .then(|| (d.x >> 8).rem_euclid(cols))
    }

    /// Dig a dip in the surface and throw spray off it.
    fn impact(&mut self, c: i32, t: &Tier) {
        if self.water_top >= self.rows {
            return;
        }
        // Over THREE columns, not one. A single-column impulse is a spike the
        // wave equation propagates as a spike, and the surface renders as a
        // picket fence of 8px teeth rather than as water.
        let n = self.h.len() as i32;
        for (d, share) in [(-1, 4), (0, 2), (1, 4)] {
            let k = (c + d).clamp(0, n - 1) as usize;
            self.h[k] = (self.h[k] - t.impact / share).max(-(6 << 8));
        }
        if !t.spray {
            return;
        }
        for _ in 0..self.spray_n {
            let Some(k) = self.spray.iter().position(|s| !s.live) else {
                return;
            };
            // Up and mostly downwind, at a fraction of the fall speed — spray
            // that flew as fast as the rain reads as rain going the wrong way.
            let r1 = next_rand(&mut self.rng) as i32;
            let r2 = next_rand(&mut self.rng) as i32;
            self.spray[k] = Spray {
                x: (c << 8) + (r1 % 256),
                y: (self.water_top << 8) - 128,
                vx: self.slant * SUB / 2 + (r2 % 96) - 48,
                vy: -(self.vy_base / 3 + (r1 >> 8) % (self.vy_base / 3).max(1)),
                live: true,
            };
        }
    }

    /// One step of a 1D wave, damped, written in place into `hp` (whose value
    /// at `i` is consumed before it is overwritten) and then swapped in. Edges
    /// reflect, which is what makes a pool read as bounded water rather than as
    /// a scrolling texture.
    fn run_water(&mut self) {
        if self.water_top >= self.rows {
            return;
        }
        let n = self.h.len();
        for i in 0..n {
            let l = self.h[i.saturating_sub(1)];
            let r = self.h[(i + 1).min(n - 1)];
            let v = l + r - self.hp[i];
            self.hp[i] = (v - (v >> 6)).clamp(-(8 << 8), 6 << 8);
        }
        std::mem::swap(&mut self.h, &mut self.hp);
    }

    /// Paint the pool: every sub-row from the surface down to the bottom edge.
    /// The surface cell takes foam when the crest is MOVING, which is what
    /// makes the water read as running rather than as a painted band.
    fn stamp_water(&mut self) {
        if self.water_top >= self.rows {
            return;
        }
        let cols = self.cols.max(1);
        let bottom = self.rows * SUB;
        for c in 0..cols {
            let up = (self.h[c as usize] >> 8).clamp(-2 * SUB, SUB / 2);
            let vel = (self.h[c as usize] - self.hp[c as usize]).abs();
            // Never below the last sub-row: a 1280x400 panel has a pool ONE
            // row deep, and a dip deeper than that would drain the column and
            // put a hole in the waterline.
            let surface = (self.water_top * SUB - up).min(bottom - 1);
            let crest = if vel > 200 { C_FOAM } else { C_WATER_SURF };
            for s in surface.max(0)..bottom {
                let i = ((s >> 2) * cols + c) as usize;
                self.pat[i] |= BAND[(s & 3) as usize];
                let want = match s - surface {
                    0 => crest,
                    1..=3 => C_WATER_SURF,
                    4..=11 => C_WATER_BODY,
                    _ => C_WATER_DEEP,
                };
                self.col[i] = self.col[i].max(want);
            }
        }
    }

    fn stamp_spray(&mut self) {
        let (cols, bottom) = (self.cols.max(1), self.water_top << 8);
        let g = (self.vy_base / 7).max(1);
        for k in 0..self.spray.len() {
            let mut s = self.spray[k];
            if !s.live {
                continue;
            }
            s.vy += g;
            s.x += s.vx;
            s.y += s.vy;
            if s.y >= bottom && s.vy > 0 {
                s.live = false;
                self.spray[k] = s;
                continue;
            }
            self.spray[k] = s;
            let sub = s.y >> 6;
            if sub < 0 {
                continue;
            }
            let c = (s.x >> 8).rem_euclid(cols);
            let i = ((sub >> 2) * cols + c) as usize;
            if i < self.pat.len() {
                self.pat[i] |= BITS[((s.x >> 7) & 1) as usize][(sub & 3) as usize];
                self.col[i] = C_SPRAY;
            }
        }
    }
}

impl Saver for HardRain {
    fn render(&mut self, s: &mut Surface<'_>) {
        // Rebuilt from zero, so a trail is not expressible. Two memsets over
        // one byte per cell — a rounding error beside the blit.
        self.pat.fill(0);
        self.col.fill(0);

        self.gust();
        self.squall();

        for k in 0..self.drops.len() {
            let mut d = self.drops[k];
            d.y += d.vy;
            d.x += (d.vy * self.slant * SUB) >> 8;
            if let Some(c) = self.stamp_drop(&d) {
                d.landed = true;
                self.impact(c, &TIERS[d.tier as usize]);
            }
            // Gone once the whole streak is at or past the surface.
            if (((d.y >> 6) - d.len as i32 + 1) >> 2) >= self.water_top {
                self.respawn(&mut d);
            }
            self.drops[k] = d;
        }

        self.run_water();
        self.stamp_water();
        self.stamp_spray();

        let (pat, col) = (&self.pat, &self.col);
        let cols = self.grid.cols();
        self.grid.fill(|cx, cy| {
            let i = cy * cols + cx;
            Cell::new(font::BRAILLE[pat[i] as usize], col[i] as u16)
        });
        // `flush`, not `flush_sparse`: damage is a u32 compare per cell and
        // cannot under-report. See the module doc.
        self.grid.flush(s, &PAL);
    }

    fn name(&self) -> &'static str {
        "hardrain"
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
    use crate::rain::Rain;
    use crate::saver;

    /// 1080 is not a multiple of 16: 67 rows and an 8-line strip below them
    /// that belongs to no cell, which is the strip frame 0 has to paint. 400 is
    /// the panel this actually ships to — 25 rows, very wide and very short.
    const PANELS: [(usize, usize); 2] = [(1920, 1080), (1280, 400)];

    fn at(w: usize, h: usize) -> (Panel, HardRain) {
        let p = Panel::new(w, h, w);
        let s = HardRain::new(&p, 30);
        (p, s)
    }

    fn lit(r: &HardRain) -> usize {
        r.grid
            .cells()
            .iter()
            .filter(|c| c.glyph() != font::BLANK as usize)
            .count()
    }

    /// T1 + T2, on both panel shapes. Frame 0 covers the whole panel including
    /// the bottom strip, and after that every scanline that changed is
    /// reported. `flush` cannot under-report by construction — it diffs cur
    /// against prev — which is exactly why this saver uses it.
    #[test]
    fn damage_covers_every_changed_scanline() {
        for (w, h) in PANELS {
            let (p, mut r) = at(w, h);
            let mut buf = vec![0u32; p.buf_len()];
            let mut prev = buf.clone();

            let d = saver::frame(&mut r, &mut buf, &p);
            assert_eq!(d.rows(), p.h, "{w}x{h}: frame 0 must paint the whole panel");
            assert!(
                buf.iter().filter(|&&px| px != 0).count() > w * h / 100,
                "{w}x{h}: frame 0 painted nothing"
            );

            let mut rows = Vec::new();
            for n in 1..600 {
                prev.copy_from_slice(&buf);
                let d = saver::frame(&mut r, &mut buf, &p);
                for y in 0..p.h {
                    if prev[y * p.w..][..p.w] == buf[y * p.w..][..p.w] {
                        continue;
                    }
                    assert!(
                        dump::row_reported(&prev[y * p.w..][..p.w], &buf[y * p.w..][..p.w], y, &d),
                        "{w}x{h} frame {n}: scanline {y} changed outside every reported rect"
                    );
                }
                rows.push(d.rows());
            }
            rows.sort_unstable();
            assert!(rows[rows.len() / 2] > 0, "{w}x{h}: no damage after frame 0");
        }
    }

    /// T3. `render` allocates NOTHING — not "does not grow a Vec", which is
    /// what a capacity check proves. The two scene fields, the drop table and
    /// the spray table are all fixed for the life of the saver, and the water
    /// is two buffers that are swapped rather than reallocated.
    #[test]
    fn render_never_allocates() {
        let (p, mut r) = at(1920, 1080);
        let mut buf = vec![0u32; p.buf_len()];
        // Frame 0 and a warm-up outside the window: the first frames touch
        // nothing this one does not, but the assertion is about the steady
        // state and a lazily-initialised anything would poison it.
        for _ in 0..8 {
            saver::frame(&mut r, &mut buf, &p);
        }

        let before = crate::testalloc::count();
        for _ in 0..4_000 {
            saver::frame(&mut r, &mut buf, &p);
        }
        let after = crate::testalloc::count();
        assert_eq!(
            after,
            before,
            "the render path allocated {} times",
            after - before
        );

        // Non-vacuous: the probe has to be able to see an allocation at all,
        // and it has to be seeing THIS thread.
        let v: Vec<u8> = Vec::with_capacity(64);
        assert!(
            crate::testalloc::count() > after,
            "the allocation probe counts nothing"
        );
        drop(v);
        // And the tables really are the ones the frame loop used.
        assert!(!r.drops.is_empty() && !r.spray.is_empty());
    }

    /// The scene is EXACTLY the field, and the field is rebuilt from zero: a
    /// cell lit in the grid that `pat` does not claim is a trail, and one dark
    /// that `pat` does claim is a hole. Cell for cell, not a ratio — a trail
    /// saturates rather than growing without bound, so a threshold lets it
    /// through.
    #[test]
    fn the_scene_is_exactly_what_the_storm_stamps() {
        for (w, h) in PANELS {
            let (p, mut r) = at(w, h);
            let mut buf = vec![0u32; p.buf_len()];
            let mut merged = 0;
            for n in 0..900 {
                saver::frame(&mut r, &mut buf, &p);
                // Streaks OR into the field rather than overwriting it, so two
                // crossing streaks merge instead of clipping each other. One
                // streak lights one half per sub-row — four dots at most — so
                // a SKY cell holding five is two streaks sharing it.
                merged += usize::from(
                    r.pat[..(r.water_top * r.cols) as usize]
                        .iter()
                        .any(|&q| q.count_ones() > 4),
                );
                for (i, cell) in r.grid.cells().iter().enumerate() {
                    assert_eq!(
                        cell,
                        &Cell::new(font::BRAILLE[r.pat[i] as usize], r.col[i] as u16),
                        "{w}x{h} frame {n}: cell {i} is not what the field says"
                    );
                }
                // A lit cell always has a colour and a dark one never does:
                // the two fields are written together or not at all.
                assert!(r
                    .pat
                    .iter()
                    .zip(&r.col)
                    .all(|(&q, &c)| (q == 0) == (c == 0)));
            }
            assert!(
                lit(&r) > (w / 8) * (h / 16) / 20,
                "{w}x{h}: the storm drained away"
            );
            assert!(merged > 450, "{w}x{h}: streaks never merged ({merged}/900)");
        }
    }

    /// It falls DOWNWARD, and it comes back round. Flip one sign and you get
    /// rain rising off the floor, which every other test here passes.
    #[test]
    fn the_rain_falls_downward_and_recycles() {
        let (p, mut r) = at(1920, 1080);
        let mut buf = vec![0u32; p.buf_len()];
        let mut y: Vec<i32> = r.drops.iter().map(|d| d.y).collect();
        let mut respawns = 0;
        for n in 0..1200 {
            saver::frame(&mut r, &mut buf, &p);
            for (k, d) in r.drops.iter().enumerate() {
                if d.y < y[k] {
                    // The only legal way back up is a respawn, which starts the
                    // whole streak above the top edge.
                    assert!(d.y < 0, "frame {n}: drop {k} moved up to {}", d.y);
                    respawns += 1;
                } else {
                    assert!(d.y > y[k], "frame {n}: drop {k} stalled at {}", d.y);
                }
                y[k] = d.y;
            }
        }
        assert!(respawns > r.drops.len(), "only {respawns} respawns");
    }

    /// The wind is the headline difference from `rain`, whose slant is a
    /// constant. It has to actually swing, and it has to swing by enough to see
    /// — a gust that moves the lean by a tenth of a cell is a constant with
    /// noise on it.
    #[test]
    fn the_wind_gusts() {
        let (p, mut r) = at(1920, 1080);
        let mut buf = vec![0u32; p.buf_len()];
        let (mut lo, mut hi) = (i32::MAX, i32::MIN);
        // A full gust cycle is 11s; 900 frames at 30fps is nearly three.
        for _ in 0..900 {
            saver::frame(&mut r, &mut buf, &p);
            lo = lo.min(r.slant);
            hi = hi.max(r.slant);
        }
        // The knobs are +/-70 on a base of 95 cells per 100 of fall, which is
        // most of the base. Less than the configured swing means the gust never
        // reaches its amplitude.
        assert!(hi - lo > r.gust_amp, "the wind barely moved: {lo}..{hi}");
        assert!(lo > 0, "the storm reversed direction: {lo}..{hi}");
    }

    /// Squalls sweep ACROSS. Not "a squall happens" — one that spawned in the
    /// middle and sat there passes that. It has to enter off one edge and leave
    /// off the other, and it has to happen more than once.
    #[test]
    fn squalls_cross_the_panel_and_come_back() {
        let (p, mut r) = at(1920, 1080);
        let mut buf = vec![0u32; p.buf_len()];
        let (mut lo, mut hi, mut crossings, mut was) = (i32::MAX, i32::MIN, 0, false);
        let mut heavier = 0;
        for _ in 0..4_000 {
            saver::frame(&mut r, &mut buf, &p);
            if r.squall_on {
                lo = lo.min(r.squall_x);
                hi = hi.max(r.squall_x);
            }
            if was && !r.squall_on {
                crossings += 1;
            }
            was = r.squall_on;
            // A band that sweeps across but changes nothing is decoration. Once
            // it is well inside the panel, the sky under it has to be visibly
            // more water than the sky beside it. Measured at 4-5x luminance in
            // the dump; 1.8x in lit cells is a floor with room for the roll.
            let mid = r.squall_x >> 8;
            if !r.squall_on || mid < r.squall_half || mid > r.cols - r.squall_half {
                continue;
            }
            let (mut inside, mut outside) = (0usize, 0usize);
            for row in 0..r.water_top {
                for c in 0..r.cols {
                    if r.pat[(row * r.cols + c) as usize] == 0 {
                        continue;
                    }
                    if (c - mid).abs() < r.squall_half {
                        inside += 1;
                    } else {
                        outside += 1;
                    }
                }
            }
            let (wide, rest) = (2 * r.squall_half, r.cols - 2 * r.squall_half);
            let (a, b) = (inside as f64 / wide as f64, outside as f64 / rest as f64);
            assert!(a > b * 1.8, "the squall is not a sheet: {a:.1} vs {b:.1}");
            heavier += 1;
        }
        assert!(
            heavier > 100,
            "the squall was only measurable {heavier} times"
        );
        assert!(crossings >= 2, "only {crossings} squalls in 4000 frames");
        assert!(lo <= 0, "a squall never entered off an edge: {lo}");
        assert!(
            hi >= r.cols << 8,
            "a squall never left off the far edge: {hi}"
        );
    }

    /// The water has to RUN: the surface has to differ across columns (a flat
    /// pool is a painted band) and it has to keep moving. And it has to stay in
    /// the pool — a wave that climbed out would paint the sky solid.
    #[test]
    fn the_water_pools_and_runs() {
        for (w, h) in PANELS {
            let (p, mut r) = at(w, h);
            let mut buf = vec![0u32; p.buf_len()];
            assert!(r.water_top < r.rows, "{w}x{h}: there is no pool");
            // The default pool AND one exactly one row deep: a dip deeper than
            // the pool drains the column and puts a hole in the waterline, and
            // only a shallow pool gets anywhere near that.
            for shallow in [false, true] {
                if shallow {
                    r.water_top = r.rows - 1;
                }
                let mut moved = 0;
                let mut spread = 0;
                let mut step = 0;
                for _ in 0..900 {
                    saver::frame(&mut r, &mut buf, &p);
                    let lo = *r.h.iter().min().unwrap();
                    let hi = *r.h.iter().max().unwrap();
                    spread = spread.max(hi - lo);
                    step = step.max(
                        r.h.windows(2)
                            .map(|q| (q[0] - q[1]).abs())
                            .max()
                            .unwrap_or(0),
                    );
                    moved += usize::from(r.h.iter().zip(&r.hp).any(|(a, b)| a != b));
                    // Clamped in `run_water`, and the stamp clamps again — so
                    // the pool can crest a cell but never reaches the sky.
                    assert!(lo >= -(8 << 8) && hi <= 6 << 8, "{w}x{h}: pool escaped");
                }
                assert!(spread > 256, "{w}x{h}: the surface is flat: {spread}");
                assert!(moved > 800, "{w}x{h}: the surface moved {moved} frames");
                // Water, not a picket fence. An impact spread over ONE column
                // is a spike, the wave equation carries a spike, and the
                // surface renders as 8px teeth. A cell is 4 sub-rows, so a
                // step of a whole cell between neighbours is a tooth.
                assert!(step < SUB << 8, "{w}x{h}: the surface steps {step}");

                // The bottom row is water on every column, every frame.
                let bottom = (r.rows - 1) * r.cols;
                for c in 0..r.cols {
                    let i = (bottom + c) as usize;
                    assert!(r.pat[i] != 0, "{w}x{h}: column {c} has no water");
                }
            }
        }
    }

    /// The reason this saver exists: in a rotation it must not read as `rain`
    /// again. The two are compared on the same panel over the same frames, on
    /// the three things the eye actually picks up — how much of the sky is wet,
    /// how far a streak leans, and whether the bottom of the panel is water.
    #[test]
    fn it_does_not_read_as_rain() {
        let p = Panel::new(1920, 1080, 1920);
        let mut hard = HardRain::new(&p, 30);
        let mut soft = Rain::new(&p, 30);
        let mut a = vec![0u32; p.buf_len()];
        let mut b = vec![0u32; p.buf_len()];
        for _ in 0..120 {
            saver::frame(&mut hard, &mut a, &p);
            saver::frame(&mut soft, &mut b, &p);
        }

        let wet = lit(&hard);
        let drizzle = soft
            .grid()
            .cells()
            .iter()
            .filter(|c| c.glyph() != font::BLANK as usize)
            .count();
        assert!(
            wet > drizzle * 3,
            "hardrain lights {wet} cells, rain {drizzle} — not a downpour"
        );

        // Lean, at the SLACK end of the gust — a storm whose wind drops to a
        // drizzle's lean reads as the other saver for those few seconds, which
        // is the failure this whole saver exists to avoid. `rain`'s constant is
        // 34 cells per 100 of fall.
        let drizzle = 34 * 256 / 100 / SUB;
        let mut slackest = i32::MAX;
        for _ in 0..900 {
            saver::frame(&mut hard, &mut a, &p);
            slackest = slackest.min(hard.slant.abs());
        }
        assert!(
            slackest > drizzle * 3 / 2,
            "the lean slackens to {slackest} against rain's {drizzle}"
        );

        // And the bottom of the panel is standing water rather than black with
        // the odd ripple in it.
        let bottom = ((hard.rows - 1) * hard.cols) as usize;
        assert!(hard.pat[bottom..].iter().all(|&q| q != 0));
    }

    /// `HARDRAIN_POOL_PCT=0` and `HARDRAIN_SQUALL_SECS=0` are documented as
    /// OFF, and both are early-return guards that nothing else here reaches.
    /// Set through the fields rather than the environment on purpose: env vars
    /// are process-global, `cargo test` runs this module in parallel with
    /// every other test in the crate, and a var set here was read by three of
    /// them before this comment existed.
    #[test]
    fn the_off_switches_leave_a_storm_that_still_renders() {
        for (w, h) in PANELS {
            let (p, mut r) = at(w, h);
            r.water_top = r.rows;
            r.squall_mean = 0;
            r.squall_on = false;
            let mut buf = vec![0u32; p.buf_len()];
            for _ in 0..300 {
                saver::frame(&mut r, &mut buf, &p);
                assert!(!r.squall_on, "{w}x{h}: a squall started with them off");
                assert!(r.h.iter().all(|&v| v == 0), "{w}x{h}: the pool moved");
            }
            assert!(lit(&r) > 0, "{w}x{h}: a pool-less storm rendered nothing");
            // And the bottom row is sky now, not water.
            let bottom = ((r.rows - 1) * r.cols) as usize;
            assert!(
                r.pat[bottom..].contains(&0),
                "{w}x{h}: there is still water with the pool off"
            );
        }
    }
}
