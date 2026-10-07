//! synthwave: the eighties horizon. A grid floor rolls toward the viewer under
//! a setting sun cut by thinning stripes, behind a ridge of mountains.

use super::math::{js_round, Mulberry32};
use super::{hex, text, Piece};
use crate::grid::Cell;

const COLS: usize = 65;
const ROWS: usize = 28;
/// The horizon: the first row of floor, where the rails meet.
const HZ: usize = 12;
/// The sun's radius, in half rows (one cell wide, half tall).
const SUN_R: f64 = 13.0;
/// The sun's centre, in half rows from the top.
const SUN_Y: f64 = 2.0 * HZ as f64 - 6.0;
/// Cross line n sits K / n rows below the horizon.
const K: f64 = 24.0;
/// Rows down to where the lines are close enough to blur.
const FAR: usize = 5;
/// Rail k crosses k * S columns a row.
const S: f64 = 0.7;
/// Rails each side of the middle one.
const RAILS: usize = 6;
/// Cross lines passed a second.
const SPEED: f64 = 0.9;
/// A line at the top, middle or bottom of a cell.
const LEVEL: [char; 3] = ['▔', '─', '▁'];
/// Floor rows.
const FR: usize = ROWS - HZ;
const CX: f64 = COLS as f64 / 2.0;
/// The middle column.
const MID: i64 = COLS as i64 / 2;

fn band(k: f64) -> f64 {
    0.2 * k + (0.3 * k * k) / 26.0
}

/// Half-row pixel (c, p): lit by the sun or not. The lower half is cut by
/// gaps one pixel thick that sink, closer together toward the horizon.
fn sun(c: f64, p: f64, s: f64) -> usize {
    let (dx, dy) = (c + 0.5 - CX, p + 0.5 - SUN_Y);
    if dx * dx + dy * dy > SUN_R * SUN_R {
        return 0;
    }
    let k = p - (SUN_Y - 7.0);
    if k < 0.0 {
        return 1;
    }
    usize::from((band(k + 1.0) - s).floor() == (band(k) - s).floor())
}

pub struct Synthwave {
    /// The ridge: heights in rows at each column boundary.
    h: [i64; COLS + 1],
    stars: [(usize, usize, f64); 24],
    count: [u32; FR],
    level: [usize; FR],
    kind: [char; FR],
}

impl Piece for Synthwave {
    const NAME: &'static str = "synthwave";
    const COLS: usize = COLS;
    const ROWS: usize = ROWS;
    const FPS: u32 = 20;
    const CELL: usize = 2;
    const PALETTE: &'static [u32] = &[hex("#ff4fb8")];
    const GROUND: u32 = 0;

    fn new() -> Self {
        // A walk that climbs toward the edges and keeps low under the sun.
        let mut rng = Mulberry32(5);
        let mut rand = || rng.next();
        let mut h = [0i64; COLS + 1];
        h[0] = 6;
        for c in 1..=COLS {
            let d = (c as f64 - CX).abs() / CX;
            let want = 1.0 + 6.0 * (d - 0.25) + 1.5 * (c as f64 * 0.45).sin();
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
            if d < 0.25 {
                step = if prev > 0 { -1 } else { 0 };
            } else if prev + step < 0 || prev + step > 7 {
                step = 0;
            }
            h[c] = prev + step;
        }
        let mut stars = [(0, 0, 0.0); 24];
        for s in &mut stars {
            let c = (rand() * COLS as f64).floor() as usize;
            let r = (rand() * (HZ - 3) as f64).floor() as usize;
            *s = (c, r, rand());
        }
        Self {
            h,
            stars,
            count: [0; FR],
            level: [0; FR],
            kind: [' '; FR],
        }
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
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
            out[r * COLS + c] = text::cell(ch);
        }

        // The sun, two pixels to a cell.
        let s = t * 0.6;
        for r in 0..HZ {
            for c in 0..COLS {
                let top = sun(c as f64, 2.0 * r as f64, s);
                let bot = sun(c as f64, 2.0 * r as f64 + 1.0, s);
                if top + bot > 0 {
                    out[r * COLS + c] = text::cell([' ', '▀', '▄', '█'][top + 2 * bot]);
                }
            }
        }

        // The mountains: an outline over a dark mass that hides the sun.
        for c in 0..COLS {
            let (a, b) = (self.h[c], self.h[c + 1]);
            if a == 0 && b == 0 {
                continue;
            }
            let top = HZ - 1 - a.min(b) as usize;
            out[top * COLS + c] = text::cell(if b > a {
                '/'
            } else if b < a {
                '\\'
            } else {
                '_'
            });
            for r in top + 1..HZ {
                out[r * COLS + c] = text::cell(' ');
            }
        }

        // Cross lines, K / n rows down, come on toward the viewer. Count them a
        // row: one is drawn at its height in the cell, two as a double line, and
        // past that, near the horizon, they blur into haze.
        let n0 = (t * SPEED) % 1.0;
        self.count.fill(0);
        for n in 1..400 {
            let e = K / (f64::from(n) - n0);
            if e >= FR as f64 {
                continue;
            }
            if e < 1.0 {
                break;
            }
            let y = e.floor() as usize;
            self.count[y] += 1;
            self.level[y] = (((e % 1.0) * 3.0).floor() as usize).min(2);
        }
        for y in 0..FR {
            let k = self.count[y];
            self.kind[y] = if y == 0 {
                '░'
            } else if y == 1 {
                '═'
            } else if y < FAR {
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
        let vx = MID as f64 + 0.5;
        for y in 0..FR {
            let row = &mut out[(HZ + y) * COLS..(HZ + y + 1) * COLS];
            let k = self.kind[y];
            row.fill(text::cell(k));
            if y == 0 {
                continue;
            }
            row[MID as usize] = text::cell(match k {
                '─' => '┼',
                '═' => '╪',
                _ => '│',
            });
            let mut both = |c: i64, l: char, r: char| {
                if (0..COLS as i64).contains(&c) {
                    row[c as usize] = text::cell(l);
                }
                let m = 2 * MID - c;
                if (0..COLS as i64).contains(&m) {
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
            for m in (every..=RAILS).step_by(every) {
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
