//! aurora: a curtain of light over a spruce treeline at night. Its lower hem
//! is brightest and ripples; streaks rise from it and fade out below a clear
//! sky of stars, and light drifts along it in slow surges.
//!
//! Drawn at the panel's size: the curtain folds on across the whole width,
//! its hem sagging and lifting in a slow swell along it on a panel wider than
//! upstream's, and the treeline runs edge to edge. The hem, the streaks and
//! the trees grow with the panel's height, or on a narrow panel with its
//! width, standing on the ground line with any rows left over going to sky.
//! `AURORA_COLOR` (on) paints each part its own colour; off, at upstream's
//! 64x20, it is upstream's picture cell for cell.

use std::f64::consts::PI;

use super::math::Mulberry32;
use super::{hex, text, Canvas};
use crate::grid::Cell;

/// Dim to bright, in upright strokes.
const RAMP: [Cell; 6] = text::cells([' ', '.', ':', '!', '|', 'I']);
const BLANK: Cell = text::cell(' ');
const DOT: Cell = text::cell('.');
const PLUS: Cell = text::cell('+');
const STAR: Cell = text::cell('*');
const TAU: f64 = PI * 2.0;
const ROWS: usize = 20;

/// Where things sit on the grid.
struct Layout {
    /// The row the hem ripples about.
    hem: usize,
    /// The row the trees stand on.
    ridge: usize,
    stars: usize,
    /// The hem's slow fold and quicker ripple, in rows.
    fold: [f64; 2],
    /// The hem's thickness above and below, the glow's fall under it, and
    /// how high the streaks stand: least and the span over it.
    hem_w: [f64; 2],
    glow: f64,
    height: [f64; 2],
    /// Upstream rows to a row of trees.
    tall: f64,
    /// The hem's long swell, in rows: 0 at upstream's width.
    swell: f64,
    /// Colour by part (`PALETTE`) rather than upstream's one ink.
    colour: bool,
}

impl Layout {
    /// Upstream's proportions on a `cols x rows` grid, scaled to its height
    /// or, when narrower than `NARROW` columns to a row, its width: the
    /// ground and hem keep upstream's rows above the bottom at that scale,
    /// the rest is sky, which the streaks rise into, and stars are as dense. At upstream's 64x20 this is
    /// upstream's layout exactly.
    fn fit(cols: usize, rows: usize, colour: bool) -> Self {
        let sy = rows as f64 / ROWS as f64;
        let s = sy.min(cols as f64 / NARROW);
        // A tall panel's streaks reach up into its extra sky.
        let rise = sy.min(2.0 * s);
        let ridge = rows.saturating_sub((2.0 * s).round() as usize).min(rows - 1);
        let hem = ridge.saturating_sub((6.0 * s).round() as usize);
        // The swell needs width to read as one: none at upstream's, all of
        // it from twice that.
        let wide = ((cols as f64 / s - 64.0) / 64.0).clamp(0.0, 1.0);
        Self {
            hem,
            ridge,
            stars: 40 * cols * (hem + 2) / (64 * 14),
            fold: [1.2 * s, 0.55 * s],
            hem_w: [0.6 * s, 0.35 * s],
            glow: 1.5 * s,
            height: [1.5 * s, 6.5 * rise],
            tall: s,
            swell: 1.6 * s * wide,
            colour,
        }
    }
}

/// Columns to a row of upstream's scale below which the width sets it.
const NARROW: f64 = 32.0;

/// This piece's own hash: integer lattice, three mixing rounds, not
/// `math::hash`.
fn hash(x: f64, y: f64) -> f64 {
    let xi = x as i64 as i32 as u32;
    let yi = y as i64 as i32 as u32;
    let mut h = xi.wrapping_mul(0x27d4_eb2d) ^ yi.wrapping_mul(0x1656_67b1);
    h = (h ^ (h >> 15)).wrapping_mul(0x85eb_ca6b);
    h = (h ^ (h >> 13)).wrapping_mul(0xc2b2_ae35);
    f64::from(h ^ (h >> 16)) / 4_294_967_296.0
}

fn ease(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

fn noise(x: f64, y: f64) -> f64 {
    let (i, j) = (x.floor(), y.floor());
    let (u, v) = (ease(x - i), ease(y - j));
    let a = hash(i, j);
    let b = hash(i + 1.0, j);
    let c = hash(i, j + 1.0);
    let d = hash(i + 1.0, j + 1.0);
    a + (b - a) * u + (c - a) * v + (a - b - c + d) * u * v
}

struct Star {
    k: usize,
    bright: bool,
    rate: f64,
    ph: f64,
}

struct Scene {
    cols: usize,
    rows: usize,
    lay: Layout,
    land: Vec<Option<Cell>>,
    stars: Vec<Star>,
    field: Vec<f32>,
    /// Per column, this frame: the hem's row and how high its streaks stand.
    hem: Vec<f64>,
    reach: Vec<f64>,
}

/// The colours: curtain green, teal and violet low to high, a
/// magenta hem, white and blue-white stars, dark teal spruces.
const GREEN: u16 = 0;
const TEAL: u16 = 1;
const VIOLET: u16 = 2;
const MAGENTA: u16 = 3;
const BRIGHT: u16 = 4;
const DIM: u16 = 5;
const TREES: u16 = 6;
const PALETTE: &[u32] = &[
    hex("#6dffa0"),
    hex("#3fd8d0"),
    hex("#a878ff"),
    hex("#ff50d0"),
    hex("#f4f8ff"),
    hex("#8fa4d8"),
    hex("#1f6a58"),
];

impl Scene {
    fn new(cols: usize, rows: usize, lay: Layout) -> Self {
        let mut rng = Mulberry32(5);
        let mut rand = || rng.next();
        let (hem, ridge) = (lay.hem, lay.ridge);

        // The treeline: spruces in stepped quarter blocks, a tier of branches
        // every two rows, each standing apart with sky between, on a low ridge.
        let mut land: Vec<Option<char>> = vec![None; cols * rows];
        for c in 0..cols {
            land[c + ridge * cols] = Some(if (c as f64 * 0.13 + 1.0).sin() > 0.3 {
                '█'
            } else {
                '▄'
            });
            for r in ridge + 1..rows {
                land[c + r * cols] = Some('█');
            }
        }
        let mut x = 1 + (rand() * 2.0).floor() as i64;
        while x < cols as i64 - 1 {
            let (a, b, d) = (rand(), rand(), rand());
            let tall = 2 + (a * 2.2 + b * d * 2.5).floor() as i64; // 2 to 5 rows
            let tall = ((tall as f64 * lay.tall) as i64).min(ridge as i64);
            for k in 0..tall {
                let r = ridge as i64 - tall + k;
                let half = 1 + (k >> 1);
                for c in x - half + 1..=x + half {
                    if c < 0 || c >= cols as i64 {
                        continue;
                    }
                    let (left, right) = (c == x - half + 1, c == x + half);
                    let odd = k & 1 == 1;
                    land[c as usize + r as usize * cols] = Some(match (left, right, odd) {
                        (true, _, true) => '▟',
                        (true, _, false) => '▗',
                        (false, true, true) => '▙',
                        (false, true, false) => '▖',
                        _ => '█',
                    });
                }
            }
            for c in x - (tall >> 1)..=x + 1 + (tall >> 1) {
                if c >= 0 && c < cols as i64 {
                    land[c as usize + ridge * cols] = Some('█');
                }
            }
            x += 2 + 2 * (tall >> 1) + 1 + (rand() * 3.0).floor() as i64;
        }

        // Stars, a few of them bright, each twinkling on its own beat.
        let mut stars = Vec::with_capacity(lay.stars);
        while stars.len() < lay.stars {
            let c = (rand() * cols as f64).floor() as usize;
            let (a, b) = (rand(), rand());
            let r = ((a * b * (hem + 2) as f64).floor() as usize).min(rows - 1);
            let bright = rand() < 0.2;
            let rate = 0.3 + rand() * 1.1;
            let ph = rand() * TAU;
            stars.push(Star {
                k: c + r * cols,
                bright,
                rate,
                ph,
            });
        }

        Self {
            cols,
            rows,
            lay,
            land: land.into_iter().map(|c| c.map(text::cell)).collect(),
            stars,
            field: vec![0.0; cols * rows],
            hem: vec![0.0; cols],
            reach: vec![0.0; cols],
        }
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        let (cols, rows, lay) = (self.cols, self.rows, &self.lay);
        let [f1, f2] = lay.fold;
        for c in 0..cols {
            let x = c as f64 + 0.5;
            // The hem: a slow fold and a quicker ripple running along it.
            let mut y0 = lay.hem as f64
                + f1 * (x * 0.12 + t * 0.35).sin()
                + f2 * (x * 0.31 - t * 0.7 + 1.0).sin();
            if lay.swell != 0.0 {
                y0 += lay.swell * (x * 0.021 + t * 0.06).sin();
            }
            let slope = f1 * 0.12 * (x * 0.12 + t * 0.35).cos()
                + f2 * 0.31 * (x * 0.31 - t * 0.7 + 1.0).cos();
            // Light surges along the curtain but never leaves it, and gathers
            // where a fold is seen edge on.
            let lit = (0.62 + 0.4 * ease(noise(x * 0.07 - t * 0.25, 3.3)))
                * (1.0 + 0.9 * (slope.abs() * 2.0).min(1.0));
            // Streaks stand on the hem, sliding along it, each fading as it rises.
            let ray =
                ease(((noise(x * 0.9 - t * 1.4, 7.0 + t * 0.3) - 0.3) / 0.45).clamp(0.0, 1.0));
            let height = lay.height[0]
                + lay.height[1] * ray * (0.6 + 0.6 * noise(x * 0.12 + t * 0.15, 11.0));
            self.hem[c] = y0;
            self.reach[c] = height;
            for r in 0..rows {
                // Rows above the hem. A bright hem, the streaks above it, and
                // below it a glow that settles on the treetops, striped like the streaks.
                let d = y0 - (r as f64 + 0.5);
                let hem = (-((d.abs() - 0.5).max(0.0)
                    / if d > 0.0 { lay.hem_w[0] } else { lay.hem_w[1] })
                .powf(2.0))
                .exp();
                let v = hem.max(if d >= 0.0 {
                    (0.3 + 0.7 * ray) * (1.0 - d / height).max(0.0).powf(1.4)
                } else {
                    (d / lay.glow).exp() * (0.4 + 0.4 * ray)
                });
                self.field[c + r * cols] = (lit * v) as f32;
            }
        }
        for ((o, &f), land) in out.iter_mut().zip(&self.field).zip(&self.land) {
            let b = f64::from(f) - 0.12;
            *o = land.unwrap_or(if b > 0.0 {
                RAMP[(RAMP.len() - 1).min(1 + (b * 5.0).floor() as usize)]
            } else {
                BLANK
            });
        }
        if lay.colour {
            self.tint(t, out);
        }
        for s in &self.stars {
            if out[s.k] != BLANK {
                continue;
            }
            let tw = (t * s.rate * TAU + s.ph).sin();
            let star = if s.bright {
                if tw > 0.3 {
                    PLUS
                } else {
                    STAR
                }
            } else if tw > -0.6 {
                DOT
            } else {
                BLANK
            };
            out[s.k] = if lay.colour {
                text::tint(star, if s.bright { BRIGHT } else { DIM })
            } else {
                star
            };
        }
    }

    /// The colours over this frame's glyphs: the trees, and the
    /// curtain by height over its hem, green and teal trading places in slow
    /// bands along it.
    fn tint(&self, t: f64, out: &mut [Cell]) {
        let (cols, hem_w) = (self.cols, self.lay.hem_w[0]);
        for (k, (o, land)) in out.iter_mut().zip(&self.land).enumerate() {
            let (c, r) = (k % cols, k / cols);
            let tone = if land.is_some() {
                TREES
            } else if *o == BLANK {
                continue;
            } else {
                let d = self.hem[c] - (r as f64 + 0.5);
                let f = d / self.reach[c];
                let swap = (c as f64 * 0.045 + t * 0.07).sin() > 0.35;
                if d < -0.25 * hem_w {
                    MAGENTA
                } else if f < 0.35 {
                    if swap { TEAL } else { GREEN }
                } else if f < 0.6 {
                    if swap { GREEN } else { TEAL }
                } else {
                    VIOLET
                }
            };
            *o = text::tint(*o, tone);
        }
    }
}

pub struct Aurora(Scene);

impl Canvas for Aurora {
    const NAME: &'static str = "aurora";
    #[cfg(test)]
    const COLS: usize = 64;
    #[cfg(test)]
    const ROWS: usize = ROWS;
    const FPS: u32 = 20;
    #[cfg(test)]
    const UPSTREAM: &'static [(&'static str, &'static str)] = &[("AURORA_COLOR", "0")];

    fn new(cols: usize, rows: usize) -> Self {
        let colour = crate::env_num(&["AURORA_COLOR"], 1, 0, 1) == 1;
        Self(Scene::new(cols, rows, Layout::fit(cols, rows, colour)))
    }

    fn palette(&self) -> &'static [u32] {
        if self.0.lay.colour {
            PALETTE
        } else {
            &[INK]
        }
    }

    /// A deep night blue in colour, upstream's black without.
    fn ground(&self) -> u32 {
        if self.0.lay.colour {
            hex("#040a1c")
        } else {
            0
        }
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        self.0.frame(t, out);
    }
}

/// Upstream's one ink.
const INK: u32 = hex("#7dffb0");
