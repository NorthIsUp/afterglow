//! synthwave: the eighties horizon. A grid floor rolls toward the viewer under
//! a setting sun cut by thinning stripes, behind a ridge of mountains.
//!
//! `synthwave-wide` is the same horizon at the panel's size: the sun, the
//! ridge and the floor's depth scale with its height, the ridge runs edge to
//! edge, and as many rails fan out from the vanishing point as reach the
//! sides.

use super::math::{js_round, Mulberry32};
use super::{hex, text, Canvas, Piece};
use crate::grid::Cell;

/// Rail k crosses k * S columns a row.
const S: f64 = 0.7;
/// Cross lines passed a second.
const SPEED: f64 = 0.9;
/// A line at the top, middle or bottom of a cell.
const LEVEL: [char; 3] = ['▔', '─', '▁'];

fn band(k: f64) -> f64 {
    0.2 * k + (0.3 * k * k) / 26.0
}

/// Where things sit: upstream's literals, or the same scaled to a grid.
struct Layout {
    cols: usize,
    rows: usize,
    /// The horizon: the first row of floor, where the rails meet.
    hz: usize,
    /// The sun's radius and centre, in half rows (one cell wide, half tall).
    sun_r: f64,
    sun_y: f64,
    /// Upstream stripes per stripe here: the sun's radius over upstream's.
    stripe: f64,
    /// Cross line n sits K / n rows below the horizon.
    k: f64,
    /// Rows down to where the lines are close enough to blur.
    far: usize,
    /// Rails each side of the middle one.
    rails: usize,
    stars: usize,
    /// The ridge: its tallest, in rows, and how fast it climbs, and the
    /// share of the half width kept low under the sun.
    peak: i64,
    climb: f64,
    quiet: f64,
}

impl Layout {
    const ORIGINAL: Self = Self {
        cols: 65,
        rows: 28,
        hz: 12,
        sun_r: 13.0,
        sun_y: 2.0 * 12.0 - 6.0,
        stripe: 1.0,
        k: 24.0,
        far: 5,
        rails: 6,
        stars: 24,
        peak: 7,
        climb: 1.0,
        quiet: 0.25,
    };

    /// Upstream's proportions on a `cols x rows` grid: the sky and the floor
    /// keep their shares of the height, and the floor its depth.
    fn fit(cols: usize, rows: usize) -> Self {
        let o = Self::ORIGINAL;
        let hz = ((rows * o.hz + o.rows / 2) / o.rows).min(rows - 1);
        let sy = hz as f64 / o.hz as f64;
        let fr = (rows - hz) as f64;
        let k = o.k * fr / (o.rows - o.hz) as f64;
        let cx = cols as f64 / 2.0;
        let sun_r = o.sun_r * sy;
        Self {
            cols,
            rows,
            hz,
            sun_r,
            sun_y: 2.0 * hz as f64 - 6.0 * sy,
            stripe: sy,
            k,
            far: js_round(k.sqrt()) as usize,
            rails: o.rails.max((cx / (S * fr * 0.5)).ceil() as usize),
            stars: o.stars * cols * hz.saturating_sub(3) / (o.cols * (o.hz - 3)),
            peak: (7.0 * sy).round() as i64,
            climb: sy,
            quiet: 0.6 * sun_r / cx,
        }
    }

    fn cx(&self) -> f64 {
        self.cols as f64 / 2.0
    }

    /// Half-row pixel (c, p): lit by the sun or not. The lower half is cut by
    /// gaps one pixel thick that sink, closer together toward the horizon.
    fn sun(&self, c: f64, p: f64, s: f64) -> usize {
        let (dx, dy) = (c + 0.5 - self.cx(), p + 0.5 - self.sun_y);
        if dx * dx + dy * dy > self.sun_r * self.sun_r {
            return 0;
        }
        let k = (p - (self.sun_y - 7.0 * self.stripe)) / self.stripe;
        if k < 0.0 {
            return 1;
        }
        usize::from((band(k + 1.0) - s).floor() == (band(k) - s).floor())
    }
}

struct Scene {
    lay: Layout,
    /// The ridge: heights in rows at each column boundary.
    h: Vec<i64>,
    stars: Vec<(usize, usize, f64)>,
    count: Vec<u32>,
    level: Vec<usize>,
    kind: Vec<char>,
}

impl Scene {
    fn new(lay: Layout) -> Self {
        let (cols, cx) = (lay.cols, lay.cx());
        // A walk that climbs toward the edges and keeps low under the sun.
        let mut rng = Mulberry32(5);
        let mut rand = || rng.next();
        let mut h = vec![0i64; cols + 1];
        h[0] = lay.peak.min(6);
        for c in 1..=cols {
            let d = (c as f64 - cx).abs() / cx;
            let want =
                1.0 + 6.0 * lay.climb * (d - lay.quiet) + 1.5 * lay.climb * (c as f64 * 0.45).sin();
            let prev = h[c - 1];
            let r = rand();
            let mut step = if r < 0.6 {
                let w = js_round(want - prev as f64);
                if w > 0.0 {
                    1
                } else if w < 0.0 {
                    -1
                } else {
                    0
                }
            } else if r < 0.8 {
                1
            } else {
                -1
            };
            if d < lay.quiet {
                step = if prev > 0 { -1 } else { 0 };
            } else if prev + step < 0 || prev + step > lay.peak {
                step = 0;
            }
            h[c] = prev + step;
        }
        let stars = (0..lay.stars)
            .map(|_| {
                let c = (rand() * cols as f64).floor() as usize;
                let r = (rand() * lay.hz.saturating_sub(3) as f64).floor() as usize;
                (c, r, rand())
            })
            .collect();
        let fr = lay.rows - lay.hz;
        Self {
            lay,
            h,
            stars,
            count: vec![0; fr],
            level: vec![0; fr],
            kind: vec![' '; fr],
        }
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        let lay = &self.lay;
        let (cols, hz) = (lay.cols, lay.hz);
        let fr = lay.rows - hz;
        let mid = cols as i64 / 2;
        out.fill(text::cell(' '));
        for &(c, r, p) in &self.stars {
            let tw = (t * (0.8 + p) + p * 30.0).sin();
            let ch = if tw > 0.6 {
                if p < 0.3 {
                    '+'
                } else {
                    '·'
                }
            } else if tw > -0.4 {
                '.'
            } else {
                ' '
            };
            out[r * cols + c] = text::cell(ch);
        }

        // The sun, two pixels to a cell.
        let s = t * 0.6;
        for r in 0..hz {
            for c in 0..cols {
                let top = lay.sun(c as f64, 2.0 * r as f64, s);
                let bot = lay.sun(c as f64, 2.0 * r as f64 + 1.0, s);
                if top + bot > 0 {
                    out[r * cols + c] = text::cell([' ', '▀', '▄', '█'][top + 2 * bot]);
                }
            }
        }

        // The mountains: an outline over a dark mass that hides the sun.
        for c in 0..cols {
            let (a, b) = (self.h[c], self.h[c + 1]);
            if a == 0 && b == 0 {
                continue;
            }
            let top = (hz as i64 - 1 - a.min(b)).max(0) as usize;
            out[top * cols + c] = text::cell(if b > a {
                '/'
            } else if b < a {
                '\\'
            } else {
                '_'
            });
            for r in top + 1..hz {
                out[r * cols + c] = text::cell(' ');
            }
        }

        // Cross lines, K / n rows down, come on toward the viewer. Count them a
        // row: one is drawn at its height in the cell, two as a double line, and
        // past that, near the horizon, they blur into haze.
        let n0 = (t * SPEED) % 1.0;
        self.count.fill(0);
        for n in 1..400 {
            let e = lay.k / (f64::from(n) - n0);
            if e >= fr as f64 {
                continue;
            }
            if e < 1.0 {
                break;
            }
            let y = e.floor() as usize;
            self.count[y] += 1;
            self.level[y] = (((e % 1.0) * 3.0).floor() as usize).min(2);
        }
        for y in 0..fr {
            let k = self.count[y];
            self.kind[y] = if y == 0 {
                '░'
            } else if y == 1 {
                '═'
            } else if y < lay.far {
                '─'
            } else if k > 1 {
                '═'
            } else if k > 0 {
                LEVEL[self.level[y]]
            } else {
                ' '
            };
        }

        // Rails from the vanishing point, rail k crossing k * S columns a row.
        // Steep ones are a stroke a row; shallow ones a run along the bottom of
        // the row stepped by one slash, kept only where the next rail's run will
        // not touch it. On a row with a cross line only the step is drawn, so
        // rails and lines cross. Near the horizon, where they would crowd, only
        // every second or fourth rail is drawn.
        let vx = mid as f64 + 0.5;
        for y in 0..fr {
            let row = &mut out[(hz + y) * cols..(hz + y + 1) * cols];
            let k = self.kind[y];
            row.fill(text::cell(k));
            if y == 0 {
                continue;
            }
            row[mid as usize] = text::cell(match k {
                '─' => '┼',
                '═' => '╪',
                _ => '│',
            });
            let mut both = |c: i64, l: char, r: char| {
                if (0..cols as i64).contains(&c) {
                    row[c as usize] = text::cell(l);
                }
                let m = 2 * mid - c;
                if (0..cols as i64).contains(&m) {
                    row[m as usize] = text::cell(r);
                }
            };
            let yf = y as f64;
            let every = if S * (yf + 0.5) >= 2.4 {
                1
            } else if 2.0 * S * (yf + 0.5) >= 2.4 {
                2
            } else {
                4
            };
            for m in (every..=lay.rails).step_by(every) {
                let sl = m as f64 * S;
                let (xt, xb) = (vx - sl * yf, vx - sl * (yf + 1.0));
                if xt < 0.0 {
                    break;
                }
                if sl < 1.3 {
                    let c = (vx - sl * (yf + 0.5)).floor() as i64;
                    both(c, '/', '\\');
                    continue;
                }
                let (hi, lo) = ((xt - 1e-6).floor() as i64, xb.floor() as i64);
                if k == ' ' && y >= m {
                    for c in (lo + 1).max(0)..hi {
                        both(c, '_', '_');
                    }
                }
                both(hi, '/', '\\');
            }
        }
    }
}

pub struct Synthwave(Scene);

impl Piece for Synthwave {
    const NAME: &'static str = "synthwave";
    const COLS: usize = Layout::ORIGINAL.cols;
    const ROWS: usize = Layout::ORIGINAL.rows;
    const FPS: u32 = 20;
    const CELL: usize = 2;
    const PALETTE: &'static [u32] = &[hex("#ff4fb8")];
    const GROUND: u32 = 0;

    fn new() -> Self {
        Self(Scene::new(Layout::ORIGINAL))
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        self.0.frame(t, out);
    }
}

pub struct SynthwaveWide(Scene);

impl Canvas for SynthwaveWide {
    const NAME: &'static str = "synthwave-wide";
    const FPS: u32 = Synthwave::FPS;
    const PALETTE: &'static [u32] = Synthwave::PALETTE;

    fn new(cols: usize, rows: usize) -> Self {
        Self(Scene::new(Layout::fit(cols, rows)))
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        self.0.frame(t, out);
    }
}
