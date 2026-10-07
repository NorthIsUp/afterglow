//! reaction diffusion: a Gray-Scott reaction whose spots on the left give way
//! to stripes on the right. Every few seconds the kill rate rises, the pattern
//! dies back to a few survivors, and they grow out again.
//!
//! A simulation: every frame runs the steps since the last, so `frame` must
//! see every tick in order, as [`super::Play`] gives it. The chemicals stay
//! `f32` as upstream's `Float32Array`s, so the pattern grows the same way.

use std::f64::consts::PI;

use super::math::{smooth, Mulberry32};
use super::{hex, text, Piece};
use crate::grid::Cell;

/// The grid, two cells to a character so they are square.
const W: usize = 60;
const H: usize = 48;
const N: usize = W * H;
/// Feed and kill rates.
const SPOTS: [f64; 2] = [0.0367, 0.0649];
const STRIPES: [f64; 2] = [0.03, 0.057];
/// Diffusion.
const DU: f64 = 0.45;
const DV: f64 = 0.225;
/// Reaction steps a second.
const RATE: f64 = 1000.0;
/// Steps from one die back to the next.
const CYCLE: f64 = 8000.0;
/// The kill rate's rise over a cycle, and its height.
const BACK: [f64; 4] = [0.1, 0.25, 0.35, 0.48];
const BUMP: f64 = 0.012;
/// Seeds scattered at the start, and steps run before the first frame.
const SEEDS: usize = 48;
const WARM: f64 = 1200.0;
// Concentration of the second chemical. Below the floor is bare ground; above
// it a steep curve, so every feature has a crisp edge.
const RAMP: [Cell; 6] = text::cells(['-', '=', '*', '#', '%', '@']);
const BLANK: Cell = text::cell(' ');
const FLOOR: f64 = 0.12;
const TOP: f64 = 0.28;

fn bump(p: f64) -> f64 {
    BUMP * smooth(BACK[0], BACK[1], p) * (1.0 - smooth(BACK[2], BACK[3], p))
}

pub struct ReactionDiffusion {
    u: Vec<f32>,
    v: Vec<f32>,
    u2: Vec<f32>,
    v2: Vec<f32>,
    feed: Vec<f32>,
    kill: Vec<f32>,
    fade: Vec<f32>,
    bite: Vec<f32>,
    rand: Mulberry32,
    n: f64,
    cycle: f64,
}

impl ReactionDiffusion {
    fn rand(&mut self) -> f64 {
        self.rand.next()
    }

    fn seed(&mut self, cx: usize, cy: usize) {
        for dy in -2..=2i64 {
            for dx in -2..=2i64 {
                let y = (cy as i64 + dy + H as i64) as usize % H;
                let x = (cx as i64 + dx + W as i64) as usize % W;
                self.u[y * W + x] = 0.5;
                self.v[y * W + x] = 0.25;
            }
        }
    }

    fn scatter(&mut self, n: usize) {
        for _ in 0..n {
            let x = (self.rand() * W as f64).floor() as usize;
            let y = (self.rand() * H as f64).floor() as usize;
            self.seed(x, y);
        }
    }

    /// Each die back is uneven: a smooth random field, new every cycle, sets
    /// how hard it bites where, so the survivors fall differently each time.
    fn reshape(&mut self) {
        let mut waves = [[0.0f64; 3]; 3];
        for w in &mut waves {
            let (mut kx, mut ky) = (0.0, 0.0);
            while kx == 0.0 && ky == 0.0 {
                kx = (self.rand() * 5.0).floor() - 2.0;
                ky = (self.rand() * 5.0).floor() - 2.0;
            }
            *w = [kx, ky, self.rand() * 6.283];
        }
        for y in 0..H {
            for x in 0..W {
                let mut s = 0.0;
                for [kx, ky, p] in waves {
                    s += (2.0 * PI * ((kx * x as f64) / W as f64 + (ky * y as f64) / H as f64) + p)
                        .cos();
                }
                self.bite[y * W + x] = (1.0 + s / 6.0) as f32;
            }
        }
    }

    fn step(&mut self) {
        // The cycle starts on the first frame.
        let k = ((self.n - WARM) / CYCLE).floor();
        if k != self.cycle {
            self.cycle = k;
            self.reshape();
        }
        let p = (self.n - WARM) / CYCLE - k;
        let b = bump(p);
        self.n += 1.0;
        let (u, v) = (&self.u, &self.v);
        for y in 0..H {
            let (ym, y0, yp) = (((y + H - 1) % H) * W, y * W, ((y + 1) % H) * W);
            for x in 0..W {
                let (xm, xp) = ((x + W - 1) % W, (x + 1) % W);
                let i = y0 + x;
                let f = |a: &[f32], j: usize| f64::from(a[j]);
                let lu = 0.2 * (f(u, ym + x) + f(u, yp + x) + f(u, y0 + xm) + f(u, y0 + xp))
                    + 0.05 * (f(u, ym + xm) + f(u, ym + xp) + f(u, yp + xm) + f(u, yp + xp))
                    - f(u, i);
                let lv = 0.2 * (f(v, ym + x) + f(v, yp + x) + f(v, y0 + xm) + f(v, y0 + xp))
                    + 0.05 * (f(v, ym + xm) + f(v, ym + xp) + f(v, yp + xm) + f(v, yp + xp))
                    - f(v, i);
                let (ui, vi) = (f(u, i), f(v, i));
                let uvv = ui * vi * vi;
                let (feed, kill) = (f64::from(self.feed[x]), f64::from(self.kill[x]));
                self.u2[i] = (ui + DU * lu - uvv + feed * (1.0 - ui)) as f32;
                self.v2[i] =
                    (vi + DV * lv + uvv - (feed + kill + b * f64::from(self.bite[i])) * vi) as f32;
            }
        }
        std::mem::swap(&mut self.u, &mut self.u2);
        std::mem::swap(&mut self.v, &mut self.v2);
        // Should a die back ever take everything, start again from a few seeds.
        if p < BACK[3]
            && p + 1.0 / CYCLE >= BACK[3]
            && self.v.iter().fold(0.0, |s, &x| s + f64::from(x)) < 8.0
        {
            self.scatter(4);
        }
    }
}

impl Piece for ReactionDiffusion {
    const NAME: &'static str = "reaction-diffusion";
    const COLS: usize = 60;
    const ROWS: usize = 24;
    const FPS: u32 = 20;
    const CELL: usize = 2;
    const PALETTE: &'static [u32] = &[hex("#ff8a3d")];
    const GROUND: u32 = 0;

    fn new() -> Self {
        let (cols, rows) = (Self::COLS, Self::ROWS);
        let mut feed = vec![0.0; W];
        let mut kill = vec![0.0; W];
        for x in 0..W {
            let s = smooth(
                0.1,
                0.9,
                (1.0 + ((2.0 * PI * (x as f64 + 0.5)) / W as f64).sin()) / 2.0,
            );
            feed[x] = (STRIPES[0] + (SPOTS[0] - STRIPES[0]) * s) as f32;
            kill[x] = (STRIPES[1] + (SPOTS[1] - STRIPES[1]) * s) as f32;
        }
        // The frame fades toward its edges, so the pattern thins out there.
        let mut fade = vec![0.0; cols * rows];
        for r in 0..rows {
            for c in 0..cols {
                let ax = ((c as f64 + 0.5) / cols as f64 - 0.5).abs() * 2.0;
                let ay = ((r as f64 + 0.5) / rows as f64 - 0.5).abs() * 2.0;
                fade[r * cols + c] =
                    (1.0 - 0.6 * smooth(0.55, 1.0, (ax.powf(3.0) + ay.powf(3.0)).cbrt())) as f32;
            }
        }
        let mut rd = Self {
            u: vec![1.0; N],
            v: vec![0.0; N],
            u2: vec![0.0; N],
            v2: vec![0.0; N],
            feed,
            kill,
            fade,
            bite: vec![0.0; N],
            rand: Mulberry32(5),
            n: 0.0,
            cycle: -1.0,
        };
        rd.scatter(SEEDS);
        while rd.n < WARM {
            rd.step();
        }
        rd
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        let (cols, rows) = (Self::COLS, Self::ROWS);
        let want = WARM + (t * RATE).floor();
        // A long pause skips ahead rather than stalls.
        if want - self.n > RATE / 5.0 {
            self.n = want - (RATE / 5.0).ceil();
        }
        while self.n < want {
            self.step();
        }
        for r in 0..rows {
            for c in 0..cols {
                let val = ((f64::from(self.v[2 * r * W + c])
                    + f64::from(self.v[(2 * r + 1) * W + c]))
                    / 2.0)
                    * f64::from(self.fade[r * cols + c]);
                let q = ((val - FLOOR) / (TOP - FLOOR)).min(1.0).sqrt();
                out[r * cols + c] = if val < FLOOR {
                    BLANK
                } else {
                    RAMP[(RAMP.len() - 1).min((q * RAMP.len() as f64).floor() as usize)]
                };
            }
        }
    }
}
