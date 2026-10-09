//! reaction diffusion: a Gray-Scott reaction whose spots on the left give way
//! to stripes on the right. Every few seconds the kill rate rises, the pattern
//! dies back to a few survivors, and they grow out again.
//!
//! A simulation: every frame runs the steps since the last, so `frame` must
//! see every tick in order, as [`super::Fill`] gives it. The chemicals stay
//! `f32` as upstream's `Float32Array`s, so the pattern grows the same way.
//!
//! The dish is the panel, two cells to a character as upstream's is. Spots
//! turn to stripes along its long side — left to right on a landscape panel,
//! top to bottom on a portrait one — and seeds scale with its area, so it
//! fills as fast. `REACTION_DIFFUSION_COLOR` (on) gives each step of the ramp
//! its own colour; off, at upstream's 60x24, it is upstream's picture cell
//! for cell.

use std::f64::consts::PI;

use super::math::{smooth, Mulberry32};
use super::{hex, text, Canvas};
use crate::grid::Cell;

/// The grid, two cells to a character so they are square.
const W: usize = 60;
const H: usize = 48;
const N: usize = W * H;
#[cfg(test)]
const COLS: usize = 60;
#[cfg(test)]
const ROWS: usize = 24;
/// Feed and kill rates.
const SPOTS: [f64; 2] = [0.0367, 0.0649];
const STRIPES: [f64; 2] = [0.03, 0.057];
/// Diffusion.
const DU: f64 = 0.45;
const DV: f64 = 0.225;
/// Reaction steps a second.
const RATE: f64 = 1000.0;
/// Cell steps a second a dish may take: a wider dish steps slower rather than
/// cost more. Upstream's 2,880 cells fit; pine's, 8,000, runs at 88% and
/// 1080p's, 14,400, at 49%, which holds it under 1.5x matrix there.
const BUDGET: f64 = 7.0e6;
/// Steps from one die back to the next.
const CYCLE: f64 = 8000.0;
/// The kill rate's rise over a cycle, and its height.
const BACK: [f64; 4] = [0.1, 0.25, 0.35, 0.48];
const BUMP: f64 = 0.012;
/// Seeds scattered at the start, and steps run before the first frame.
const SEEDS: usize = 48;
/// Under this much of the second chemical over the dish, a die back has
/// taken everything.
const BARE: f64 = 8.0;
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

/// One step of one cell, from the nine around it: rows above, at and below,
/// each left, middle and right. Upstream's sums in upstream's order.
#[inline]
fn react(u: [f32; 9], v: [f32; 9], feed: f32, kill: f32, bite: f32, b: f64) -> (f32, f32) {
    let [um, ux, up, ul, uc, ur, un, uy, uq] = u.map(f64::from);
    let [vm, vx, vp, vl, vc, vr, vn, vy, vq] = v.map(f64::from);
    let lu = 0.2 * (ux + uy + ul + ur) + 0.05 * (um + up + un + uq) - uc;
    let lv = 0.2 * (vx + vy + vl + vr) + 0.05 * (vm + vp + vn + vq) - vc;
    let uvv = uc * vc * vc;
    let (feed, kill) = (f64::from(feed), f64::from(kill));
    (
        (uc + DU * lu - uvv + feed * (1.0 - uc)) as f32,
        (vc + DV * lv + uvv - (feed + kill + b * f64::from(bite)) * vc) as f32,
    )
}

/// `n` cells of `a` from `o`.
#[inline]
fn run(a: &[f32], o: usize, n: usize) -> &[f32] {
    &a[o..o + n]
}

/// A dish `w x h` cells, shown on a `cols x rows` grid two cells to a
/// character. A grid smaller than upstream's still gets upstream's dish,
/// cropped from `(ox, oy)`: a smaller one cannot hold a spot.
struct Dish {
    w: usize,
    h: usize,
    cols: usize,
    rows: usize,
    ox: usize,
    oy: usize,
    u: Vec<f32>,
    v: Vec<f32>,
    u2: Vec<f32>,
    v2: Vec<f32>,
    /// Per cell, so the spots-to-stripes sweep can run down a portrait dish.
    feed: Vec<f32>,
    kill: Vec<f32>,
    fade: Vec<f32>,
    bite: Vec<f32>,
    rand: Mulberry32,
    n: f64,
    cycle: f64,
    /// `BARE`, scaled to this dish's area.
    bare: f64,
    /// Steps a second: `RATE`, or less for a dish over `BUDGET`.
    rate: f64,
    /// Upstream's f64 arithmetic, cell for cell, on a dish no bigger than
    /// upstream's; a bigger one steps in f32 across whole rows, which the
    /// compiler vectorises.
    exact: bool,
    /// Each step of the ramp its own colour, rather than upstream's one ink.
    colour: bool,
}

impl Dish {
    fn new(cols: usize, rows: usize, colour: bool) -> Self {
        let (w, h) = (cols.max(W), (rows * 2).max(H));
        let n = w * h;
        let down = h > w;
        let len = if down { h } else { w };
        let mut feed = vec![0.0; n];
        let mut kill = vec![0.0; n];
        for i in 0..len {
            let s = smooth(
                0.1,
                0.9,
                (1.0 + ((2.0 * PI * (i as f64 + 0.5)) / len as f64).sin()) / 2.0,
            );
            let (f, k) = (
                (STRIPES[0] + (SPOTS[0] - STRIPES[0]) * s) as f32,
                (STRIPES[1] + (SPOTS[1] - STRIPES[1]) * s) as f32,
            );
            if down {
                feed[i * w..(i + 1) * w].fill(f);
                kill[i * w..(i + 1) * w].fill(k);
            } else {
                for y in 0..h {
                    feed[y * w + i] = f;
                    kill[y * w + i] = k;
                }
            }
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
            w,
            h,
            cols,
            rows,
            ox: (w - cols) / 2,
            oy: (h - rows * 2) / 2,
            u: vec![1.0; n],
            v: vec![0.0; n],
            u2: vec![0.0; n],
            v2: vec![0.0; n],
            feed,
            kill,
            fade,
            bite: vec![0.0; n],
            rand: Mulberry32(5),
            n: 0.0,
            cycle: -1.0,
            bare: BARE * n as f64 / N as f64,
            rate: RATE.min(BUDGET / n as f64),
            exact: n <= N,
            colour,
        };
        rd.scatter((SEEDS * n).div_ceil(N));
        while rd.n < WARM {
            rd.step();
        }
        rd
    }

    fn rand(&mut self) -> f64 {
        self.rand.next()
    }

    fn seed(&mut self, cx: usize, cy: usize) {
        let (w, h) = (self.w, self.h);
        for dy in -2..=2i64 {
            for dx in -2..=2i64 {
                let y = (cy as i64 + dy + 2 * h as i64) as usize % h;
                let x = (cx as i64 + dx + 2 * w as i64) as usize % w;
                self.u[y * w + x] = 0.5;
                self.v[y * w + x] = 0.25;
            }
        }
    }

    fn scatter(&mut self, n: usize) {
        for _ in 0..n {
            let x = (self.rand() * self.w as f64).floor() as usize;
            let y = (self.rand() * self.h as f64).floor() as usize;
            self.seed(x, y);
        }
    }

    /// Each die back is uneven: a smooth random field, new every cycle, sets
    /// how hard it bites where, so the survivors fall differently each time.
    fn reshape(&mut self) {
        let (w, h) = (self.w, self.h);
        let mut waves = [[0.0f64; 3]; 3];
        for wave in &mut waves {
            let (mut kx, mut ky) = (0.0, 0.0);
            while kx == 0.0 && ky == 0.0 {
                kx = (self.rand() * 5.0).floor() - 2.0;
                ky = (self.rand() * 5.0).floor() - 2.0;
            }
            *wave = [kx, ky, self.rand() * 6.283];
        }
        for y in 0..h {
            for x in 0..w {
                let mut s = 0.0;
                for [kx, ky, p] in waves {
                    s += (2.0 * PI * ((kx * x as f64) / w as f64 + (ky * y as f64) / h as f64) + p)
                        .cos();
                }
                self.bite[y * w + x] = (1.0 + s / 6.0) as f32;
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
        if self.exact {
            self.react_exact(b);
        } else {
            self.react_fast(b as f32);
        }
        std::mem::swap(&mut self.u, &mut self.u2);
        std::mem::swap(&mut self.v, &mut self.v2);
        // Should a die back ever take everything, start again from a few seeds.
        if p < BACK[3]
            && p + 1.0 / CYCLE >= BACK[3]
            && self.v.iter().fold(0.0, |s, &x| s + f64::from(x)) < self.bare
        {
            self.scatter(4);
        }
    }

    /// `u2`, `v2` from `u`, `v`: f32 throughout, each row a run of
    /// independent lanes with the wrapped ends patched after.
    fn react_fast(&mut self, b: f32) {
        let (w, h) = (self.w, self.h);
        let (du, dv) = (DU as f32, DV as f32);
        let (u, v) = (&self.u, &self.v);
        for y in 0..h {
            let (ym, y0, yp) = (((y + h - 1) % h) * w, y * w, ((y + 1) % h) * w);
            let (um, uc, up) = (&u[ym..ym + w], &u[y0..y0 + w], &u[yp..yp + w]);
            let (vm, vc, vp) = (&v[ym..ym + w], &v[y0..y0 + w], &v[yp..yp + w]);
            let bite = &self.bite[y0..y0 + w];
            let (feed, kill) = (&self.feed[y0..y0 + w], &self.kill[y0..y0 + w]);
            let (u2, v2) = (&mut self.u2[y0..y0 + w], &mut self.v2[y0..y0 + w]);
            let cell = |x: usize, xm: usize, xp: usize| {
                let lu = 0.2 * (um[x] + up[x] + uc[xm] + uc[xp])
                    + 0.05 * (um[xm] + um[xp] + up[xm] + up[xp])
                    - uc[x];
                let lv = 0.2 * (vm[x] + vp[x] + vc[xm] + vc[xp])
                    + 0.05 * (vm[xm] + vm[xp] + vp[xm] + vp[xp])
                    - vc[x];
                let (ui, vi) = (uc[x], vc[x]);
                let uvv = ui * vi * vi;
                (
                    ui + du * lu - uvv + feed[x] * (1.0 - ui),
                    vi + dv * lv + uvv - (feed[x] + kill[x] + b * bite[x]) * vi,
                )
            };
            // The run between the ends, every slice cut to its length so the
            // loop carries no bounds checks and vectorises.
            let n = w.saturating_sub(2);
            if n > 0 {
                let cut = |a, o| run(a, o, n);
                let (uml, umc, umr) = (cut(um, 0), cut(um, 1), cut(um, 2));
                let (ucl, ucc, ucr) = (cut(uc, 0), cut(uc, 1), cut(uc, 2));
                let (upl, upc, upr) = (cut(up, 0), cut(up, 1), cut(up, 2));
                let (vml, vmc, vmr) = (cut(vm, 0), cut(vm, 1), cut(vm, 2));
                let (vcl, vcc, vcr) = (cut(vc, 0), cut(vc, 1), cut(vc, 2));
                let (vpl, vpc, vpr) = (cut(vp, 0), cut(vp, 1), cut(vp, 2));
                let (fe, ki, bi) = (cut(feed, 1), cut(kill, 1), cut(bite, 1));
                let (uo, vo) = (&mut u2[1..=n], &mut v2[1..=n]);
                for i in 0..n {
                    let lu = 0.2 * (umc[i] + upc[i] + ucl[i] + ucr[i])
                        + 0.05 * (uml[i] + umr[i] + upl[i] + upr[i])
                        - ucc[i];
                    let lv = 0.2 * (vmc[i] + vpc[i] + vcl[i] + vcr[i])
                        + 0.05 * (vml[i] + vmr[i] + vpl[i] + vpr[i])
                        - vcc[i];
                    let (ui, vi) = (ucc[i], vcc[i]);
                    let uvv = ui * vi * vi;
                    uo[i] = ui + du * lu - uvv + fe[i] * (1.0 - ui);
                    vo[i] = vi + dv * lv + uvv - (fe[i] + ki[i] + b * bi[i]) * vi;
                }
            }
            (u2[0], v2[0]) = cell(0, w - 1, 1 % w);
            if w > 1 {
                (u2[w - 1], v2[w - 1]) = cell(w - 1, w - 2, 0);
            }
        }
    }

    /// `u2`, `v2` from `u`, `v` as upstream computes them.
    fn react_exact(&mut self, b: f64) {
        let (w, h) = (self.w, self.h);
        let (u, v) = (&self.u, &self.v);
        let (feed, kill) = (&self.feed[..], &self.kill[..]);
        for y in 0..h {
            let (ym, y0, yp) = (((y + h - 1) % h) * w, y * w, ((y + 1) % h) * w);
            let near = |a: &[f32], xm: usize, x: usize, xp: usize| {
                [
                    a[ym + xm],
                    a[ym + x],
                    a[ym + xp],
                    a[y0 + xm],
                    a[y0 + x],
                    a[y0 + xp],
                    a[yp + xm],
                    a[yp + x],
                    a[yp + xp],
                ]
            };
            let bite = &self.bite[y0..y0 + w];
            let (feed, kill) = (&feed[y0..y0 + w], &kill[y0..y0 + w]);
            let (u2, v2) = (&mut self.u2[y0..y0 + w], &mut self.v2[y0..y0 + w]);
            let mut at = |xm: usize, x: usize, xp: usize| {
                (u2[x], v2[x]) = react(
                    near(u, xm, x, xp),
                    near(v, xm, x, xp),
                    feed[x],
                    kill[x],
                    bite[x],
                    b,
                );
            };
            // The wrap only at the two ends, so the run between them has no
            // modulo in it.
            at(w - 1, 0, 1 % w);
            for x in 1..w.saturating_sub(1) {
                at(x - 1, x, x + 1);
            }
            if w > 1 {
                at(w - 2, w - 1, 0);
            }
        }
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        let (cols, rows, w, ox, oy) = (self.cols, self.rows, self.w, self.ox, self.oy);
        let rate = self.rate;
        let want = WARM + (t * rate).floor();
        // A long pause skips ahead rather than stalls.
        if want - self.n > rate / 5.0 {
            self.n = want - (rate / 5.0).ceil();
        }
        while self.n < want {
            self.step();
        }
        for r in 0..rows {
            for c in 0..cols {
                let val = ((f64::from(self.v[(2 * r + oy) * w + c + ox])
                    + f64::from(self.v[(2 * r + 1 + oy) * w + c + ox]))
                    / 2.0)
                    * f64::from(self.fade[r * cols + c]);
                let q = ((val - FLOOR) / (TOP - FLOOR)).min(1.0).sqrt();
                out[r * cols + c] = if val < FLOOR {
                    BLANK
                } else {
                    let i = (RAMP.len() - 1).min((q * RAMP.len() as f64).floor() as usize);
                    if self.colour {
                        text::tint(RAMP[i], i as u16)
                    } else {
                        RAMP[i]
                    }
                };
            }
        }
    }
}

pub struct ReactionDiffusion(Dish);

impl Canvas for ReactionDiffusion {
    const NAME: &'static str = "reaction-diffusion";
    #[cfg(test)]
    const COLS: usize = COLS;
    #[cfg(test)]
    const ROWS: usize = ROWS;
    const FPS: u32 = 20;
    const COLOR: &'static str = "REACTION_DIFFUSION_COLOR";
    const PALETTE: &'static [u32] = PALETTE;
    const INK: u32 = hex("#ff8a3d");

    fn new(cols: usize, rows: usize, colour: bool) -> Self {
        Self(Dish::new(cols, rows, colour))
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        self.0.frame(t, out);
    }
}

/// Thin to thick: violet edges warming through rose and orange to a pale-gold
/// core.
const PALETTE: &[u32] = &[
    hex("#7a3cc8"),
    hex("#b04ad0"),
    hex("#e0508c"),
    hex("#ff7040"),
    hex("#ffa83a"),
    hex("#ffe27a"),
];
