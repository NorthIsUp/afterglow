//! Matrix digital rain.
//!
//! # What this is a copy of
//!
//! The "classic" look — the Reloaded/Revolutions title sequences — not the
//! literal 1999 one. The first film's on-screen code is flat-brightness
//! (`brightnessOverride 0.22`) with only the travelling cursor lit, which on a
//! glyph-grid renderer reads as a bug rather than as a homage.
//!
//! Three details separate this from the usual imitation, and all three are
//! cheap:
//!
//! * The glyphs are MIRRORED left to right. The production designer turned the
//!   artwork back to front "as if we were in the code looking at a screen of
//!   code from the inside". That is baked into the atlas (see `tools/genfont.py`),
//!   so nothing here knows about it.
//! * The glyphs never move. The grid is stationary and what falls is a wave of
//!   illumination over it, which is also why a cell that only changes colour
//!   still costs one blit and no bookkeeping.
//! * Every column is always raining. There is no spawn, no respawn and no idle
//!   column: the sawtooth is periodic and unconditional, so several drops share
//!   a column at different speeds for free. Discrete drops with black gaps
//!   between them are the second-biggest tell after unmirrored glyphs.
//!
//! Deliberately absent: bloom (a separable Gaussian at 1080p blows the CPU
//! budget — the head gets its own out-of-ramp colour instead), per-pixel dither
//! (a banding fix for a four-stop palette; sixteen steps do not band), and the
//! film's fixed glyph cycling order (documented as a technical detail with no
//! visual payoff, and abandoned by the project that discovered it).

use std::f32::consts::SQRT_2;

use crate::env_num;
use crate::font;
use crate::grid::{bake, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};

/// Trail ladder (hue 108°, saturation 0.9) then the head. Indices 0..=15 are the
/// trail; 16 is the head, which sits deliberately OUTSIDE the ramp.
///
/// The trail tops out near index 9 by construction (the contrast clamp below),
/// so 10..=15 are unreached. That gap between the brightest trail cell and the
/// head is the effect; it is not a bug to fix.
///
/// The head is not white. Flat white is the cmatrix convention and reads cold
/// and hard-edged; on film the head is a bright green emitter that blooms to
/// near-white only at its centre. `#00FF41` is fan lore with no production
/// provenance and is not used here.
const PAL_RGB: [[u8; 3]; 17] = [
    [0x00, 0x00, 0x00],
    [0x08, 0x20, 0x02],
    [0x10, 0x41, 0x03],
    [0x17, 0x61, 0x05],
    [0x1F, 0x81, 0x07],
    [0x27, 0xA2, 0x08],
    [0x2F, 0xC2, 0x0A],
    [0x37, 0xE2, 0x0C],
    [0x48, 0xF3, 0x1D],
    [0x62, 0xF5, 0x3D],
    [0x7C, 0xF6, 0x5D],
    [0x96, 0xF8, 0x7E],
    [0xB0, 0xFA, 0x9E],
    [0xB0, 0xFA, 0x9E],
    [0xB0, 0xFA, 0x9E],
    [0xB0, 0xFA, 0x9E],
    [0xE8, 0xFF, 0xD0],
];

const PAL: [u32; 17] = bake(&PAL_RGB);
const HEAD: u16 = 16;

/// 0.2 rather than the film's 0.3. At 15 fps, 0.3 is 15-30 rows/sec, i.e. one to
/// two rows per frame, which is visibly steppy. 0.2 gives 10-20 rows/sec — well
/// inside cmatrix's own range — and crosses a 33-row grid in 1.65-3.3 s.
const FALL: f32 = 0.2;
const RAINDROP_LEN: f32 = 0.75;
const ROW_COEFF: f32 = 0.01;
const SQRT5: f32 = 2.236_068;

/// Per-cell glyph churn, in Hz. The film's classic rate is 1.8 Hz; this is the
/// one place the effect is dialled back for CPU, because a glyph change is a
/// full cell blit whether or not the cell also changed colour.
const MUTATE_HZ: f32 = 1.0;

pub struct Matrix {
    cols: usize,
    rows: usize,
    /// Per column, the only state the motion needs: a random phase so columns
    /// are decorrelated, and a speed multiplier in [0.5, 1.0] — an exact 2:1
    /// spread. Uniform column speed is an immediate tell.
    phase: Vec<f32>,
    speed: Vec<f32>,
    /// Per cell: the glyph it is currently showing, and a mutation countdown
    /// with a random initial phase (an even shimmer rather than a clumpy one).
    glyph: Vec<u16>,
    age: Vec<u8>,
    shade: Vec<u8>,
    t: f32,
    dt: f32,
    inc: u8,
    rng: u32,
    grid: Grid,
}

/// The LCG the fire uses, seeded differently. Nothing here needs a better one.
#[inline]
fn next_rand(rng: &mut u32) -> u32 {
    *rng = rng.wrapping_mul(1103515245).wrapping_add(12345);
    *rng >> 16
}

#[inline]
fn rand01(rng: &mut u32) -> f32 {
    (next_rand(rng) & 0xFFFF) as f32 / 65536.0
}

/// Uniform over the film's 57 slots. The modulo bias at 57 is negligible, and
/// the sequence's duplicate `0` is faithful — it really is twice as likely.
#[inline]
fn pick_glyph(rng: &mut u32) -> u16 {
    font::MATRIX[next_rand(rng) as usize % font::MATRIX.len()]
}

impl Matrix {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        let cell_w = env_num(&["MATRIX_CELL_W"], 16, 8, 64) as usize;
        let cell_h = env_num(&["MATRIX_CELL_H"], 32, 8, 128) as usize;
        let grid = Grid::new(panel, cell_w, cell_h);
        let (cols, rows) = (grid.cols(), grid.rows());
        let mut rng = 0x2545_f491;
        let phase = (0..cols).map(|_| rand01(&mut rng) * 1000.0).collect();
        let speed = (0..cols).map(|_| 0.5 + rand01(&mut rng) * 0.5).collect();
        let glyph = (0..cols * rows).map(|_| pick_glyph(&mut rng)).collect();
        let age = (0..cols * rows)
            .map(|_| next_rand(&mut rng) as u8)
            .collect();
        Self {
            cols,
            rows,
            phase,
            speed,
            glyph,
            age,
            shade: vec![0u8; cols * rows],
            t: 0.0,
            dt: 1.0 / fps as f32,
            inc: (255.0 * MUTATE_HZ / fps as f32).round().max(1.0) as u8,
            rng,
            grid,
        }
    }

    /// Advance every cell's countdown; on wrap it draws a new glyph.
    fn mutate(&mut self) {
        let inc = self.inc;
        let mut rng = self.rng;
        for (g, a) in self.glyph.iter_mut().zip(self.age.iter_mut()) {
            let next = a.wrapping_add(inc);
            if next < *a {
                *g = pick_glyph(&mut rng);
            }
            *a = next;
        }
        self.rng = rng;
    }

    /// The closed-form sawtooth, one column at a time.
    ///
    /// Subtracting `cy * ROW_COEFF` (rather than adding it) is what makes the
    /// rain fall DOWNWARD with the head at the bottom of its trail. Flip that
    /// sign and you get upward rain that otherwise looks entirely plausible.
    fn shade_columns(&mut self) {
        let (cols, rows, t) = (self.cols, self.rows, self.t);
        let shade = &mut self.shade[..];
        for (cx, (&phase, &speed)) in self.phase.iter().zip(self.speed.iter()).enumerate() {
            let ct = phase + t * FALL * speed;
            let raw = |cy: usize| {
                let rain = (ct - cy as f32 * ROW_COEFF) / RAINDROP_LEN;
                let wob = rain + 0.3 * (SQRT_2 * rain).sin() + 0.2 * (SQRT5 * rain).sin();
                1.0 - (wob - wob.floor())
            };
            // Walk bottom-up carrying the row below, so `raw` is evaluated once
            // per cell rather than twice. The head is the sawtooth's wrap point:
            // the one row brighter than the row under it. Test the RAW value —
            // the clamp below flattens whole runs to zero, where a comparison
            // would be meaningless.
            let mut below = raw(rows);
            for cy in (0..rows).rev() {
                let b = raw(cy);
                // baseContrast 1.1, baseBrightness -0.5: this is what makes the
                // visible trail ~41 rows of a 75-row sawtooth, i.e. longer than
                // the screen, so the effect is a near-continuous gradient rather
                // than discrete drops separated by black.
                let lit = (b * 1.1 - 0.5).clamp(0.0, 1.0);
                shade[cy * cols + cx] = if b > below {
                    HEAD as u8
                } else {
                    ((lit * 16.0) as usize).min(15) as u8
                };
                below = b;
            }
        }
    }
}

impl Saver for Matrix {
    fn render(&mut self, s: &mut Surface<'_>) {
        self.t += self.dt;
        self.mutate();
        self.shade_columns();
        let (grid, glyph, shade, cols) =
            (&mut self.grid, &self.glyph[..], &self.shade[..], self.cols);
        grid.fill(|cx, cy| {
            let i = cy * cols + cx;
            Cell::new(glyph[i], shade[i] as u16)
        });
        grid.flush(s, &PAL);
    }

    fn name(&self) -> &'static str {
        "matrix"
    }
}
