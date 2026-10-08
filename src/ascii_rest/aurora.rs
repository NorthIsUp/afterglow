//! aurora: a curtain of light over a spruce treeline at night. Its lower hem
//! is brightest and ripples; streaks rise from it and fade out below a clear
//! sky of stars, and light drifts along it in slow surges.
//!
//! `aurora-wide` is the same night at the panel's size: the curtain folds on
//! across the whole width, its hem sagging and lifting in a slow swell along
//! it, the treeline runs edge to edge, and the hem, the streaks and the trees
//! grow with the panel's height.

use std::f64::consts::PI;

use super::math::Mulberry32;
use super::{hex, text, Canvas, Piece};
use crate::grid::Cell;

/// Dim to bright, in upright strokes.
const RAMP: [Cell; 6] = text::cells([' ', '.', ':', '!', '|', 'I']);
const BLANK: Cell = text::cell(' ');
const DOT: Cell = text::cell('.');
const PLUS: Cell = text::cell('+');
const STAR: Cell = text::cell('*');
const TAU: f64 = PI * 2.0;
const ROWS: usize = 20;

/// Where things sit: upstream's literals, or the same scaled to a grid.
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
    /// The wide hem's long swell, in rows: 0 upstream.
    swell: f64,
}

const ORIGINAL: Layout = Layout {
    hem: 12,
    ridge: 18,
    stars: 40,
    fold: [1.2, 0.55],
    hem_w: [0.6, 0.35],
    glow: 1.5,
    height: [1.5, 6.5],
    tall: 1.0,
    swell: 0.0,
};

impl Layout {
    /// Upstream's proportions on a `cols x rows` grid, stars as dense.
    fn fit(cols: usize, rows: usize) -> Self {
        let sy = rows as f64 / ROWS as f64;
        let hem = (rows * 12 + 10) / 20;
        Self {
            hem,
            ridge: ((rows * 18 + 10) / 20).min(rows - 1),
            stars: 40 * cols * (hem + 2) / (64 * 14),
            fold: [1.2 * sy, 0.55 * sy],
            hem_w: [0.6 * sy, 0.35 * sy],
            glow: 1.5 * sy,
            height: [1.5 * sy, 6.5 * sy],
            tall: sy,
            swell: 1.6 * sy,
        }
    }
}

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
}

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
        for s in &self.stars {
            if out[s.k] != BLANK {
                continue;
            }
            let tw = (t * s.rate * TAU + s.ph).sin();
            out[s.k] = if s.bright {
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
        }
    }
}

pub struct Aurora(Scene);

impl Piece for Aurora {
    const NAME: &'static str = "aurora";
    const COLS: usize = 64;
    const ROWS: usize = ROWS;
    const FPS: u32 = 20;
    const CELL: usize = 2;
    const PALETTE: &'static [u32] = &[hex("#7dffb0")];
    const GROUND: u32 = 0;

    fn new() -> Self {
        Self(Scene::new(Self::COLS, Self::ROWS, ORIGINAL))
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        self.0.frame(t, out);
    }
}

pub struct AuroraWide(Scene);

impl Canvas for AuroraWide {
    const NAME: &'static str = "aurora-wide";
    const FPS: u32 = Aurora::FPS;
    const PALETTE: &'static [u32] = Aurora::PALETTE;

    fn new(cols: usize, rows: usize) -> Self {
        Self(Scene::new(cols, rows, Layout::fit(cols, rows)))
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        self.0.frame(t, out);
    }
}
