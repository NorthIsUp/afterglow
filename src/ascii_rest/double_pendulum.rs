//! double-pendulum: two equal rods hung end to end from one pivot, stepped
//! with RK4. Chaotic, so it never repeats; the lower bob leaves a fading trail.

use std::collections::VecDeque;

use super::math::js_round;
use super::{hex, text, Piece};
use crate::grid::Cell;

const COLS: usize = 48;
const ROWS: usize = 25;
const G: f64 = 9.81;
/// Integrator step, in seconds of model time.
const H: f64 = 1.0 / 240.0;
/// Model seconds per second of play.
const SPEED: f64 = 0.6;
/// Steps between trail samples.
const TAP: u64 = 3;
/// Trail samples kept.
const TRAIL: usize = 110;
/// Trail glyphs, oldest first.
const FADE: [Cell; 4] = text::cells(['·', ':', '+', '*']);
/// Both angles from straight down, then their rates.
const START: [f64; 4] = [2.55, 2.9, 0.0, 0.0];
/// Model seconds run before the first frame.
const WARM: f64 = 7.0;
const CX: f64 = (COLS as f64 - 1.0) / 2.0;
const CY: f64 = (ROWS as f64 - 1.0) / 2.0;
/// Both rods reach any edge but none clips.
const SY: f64 = (CY - 0.6) / 2.0;
const SX: f64 = SY * 2.0;

type State = [f64; 4];

/// Unit masses and rods. s = [angle 1, angle 2, rate 1, rate 2].
fn deriv([a, b, p, q]: State) -> State {
    let d = a - b;
    let (sd, cd, den) = (d.sin(), d.cos(), 3.0 - (2.0 * d).cos());
    [
        p,
        q,
        (-3.0 * G * a.sin() - G * (a - 2.0 * b).sin() - 2.0 * sd * (q * q + p * p * cd)) / den,
        (2.0 * sd * (2.0 * p * p + 2.0 * G * a.cos() + q * q * cd)) / den,
    ]
}

fn rk4(s: State) -> State {
    let add = |u: State, k: State, h: f64| -> State { std::array::from_fn(|i| u[i] + k[i] * h) };
    let k1 = deriv(s);
    let k2 = deriv(add(s, k1, H / 2.0));
    let k3 = deriv(add(s, k2, H / 2.0));
    let k4 = deriv(add(s, k3, H));
    std::array::from_fn(|i| s[i] + (H / 6.0) * (k1[i] + 2.0 * k2[i] + 2.0 * k3[i] + k4[i]))
}

fn kinetic([a, b, p, q]: State) -> f64 {
    p * p + 0.5 * q * q + p * q * (a - b).cos()
}

fn potential([a, b, ..]: State) -> f64 {
    -2.0 * G * a.cos() - G * b.cos()
}

/// The lower bob's place on screen.
fn tip([a, b, ..]: State) -> (f64, f64) {
    (CX + SX * (a.sin() + b.sin()), CY + SY * (a.cos() + b.cos()))
}

fn put(grid: &mut [Cell], x: f64, y: f64, ch: Cell) {
    let (c, r) = (js_round(x), js_round(y));
    if c >= 0.0 && c < COLS as f64 && r >= 0.0 && r < ROWS as f64 {
        grid[r as usize * COLS + c as usize] = ch;
    }
}

/// A thin rod, one glyph a cell. A shallow rod is a run of dashes on each
/// row, which meets the next row low or high in the cell; a steep one is a
/// bar or a slash a row. A cell is twice as tall as it is wide, so steep
/// means more than one row in two columns.
fn rod(grid: &mut [Cell], x0: f64, y0: f64, x1: f64, y1: f64) {
    let (dx, dy) = (x1 - x0, y1 - y0);
    let up = dx * dy < 0.0;
    let slash = if up { '/' } else { '\\' };
    if dx.abs() > (2.0 * dy).abs() {
        let c0 = js_round(x0.min(x1));
        let c1 = js_round(x0.max(x1));
        let row = |c: f64| (y0 + (dy * (c1.min(c0.max(c)) - x0)) / dx + 0.5).floor();
        let mut c = c0;
        while c <= c1 {
            let r = row(c);
            let (first, end) = (row(c - 1.0) != r, row(c + 1.0) != r);
            // Left to right a rising rod comes in low and leaves high; a falling
            // one the other way round.
            let ch = if first && end {
                slash
            } else if first {
                if up {
                    '_'
                } else if row(c + 2.0) != r {
                    slash
                } else {
                    '`'
                }
            } else if end {
                if up {
                    if row(c - 2.0) != r {
                        slash
                    } else {
                        '\''
                    }
                } else {
                    '_'
                }
            } else {
                '-'
            };
            put(grid, c, r, text::cell(ch));
            c += 1.0;
        }
    } else {
        let steep = (dx / dy).abs() < 0.6;
        let mut r = js_round(y0.min(y1));
        while r <= js_round(y0.max(y1)) {
            let x = x0 + (dx * (r - y0)) / dy;
            let c = (x + 0.5).floor();
            let ch = if steep && (x - c).abs() < 0.3 {
                '|'
            } else {
                slash
            };
            put(grid, c, r, text::cell(ch));
            r += 1.0;
        }
    }
}

pub struct DoublePendulum {
    s: State,
    trail: VecDeque<(f64, f64)>,
    steps: u64,
    acc: f64,
    last: f64,
    e0: f64,
}

impl DoublePendulum {
    fn step(&mut self) {
        self.s = rk4(self.s);
        // Hold the energy where it began, so long runs neither die down nor run away.
        let k = kinetic(self.s);
        let room = self.e0 - potential(self.s);
        if k > 1e-9 && room > 0.0 {
            let f = (room / k).sqrt();
            self.s[2] *= f;
            self.s[3] *= f;
        }
        self.steps += 1;
        if self.steps.is_multiple_of(TAP) {
            self.trail.push_back(tip(self.s));
            if self.trail.len() > TRAIL {
                self.trail.pop_front();
            }
        }
    }

    fn reset(&mut self) {
        self.s = START;
        self.trail.clear();
        self.steps = 0;
        self.acc = 0.0;
        self.last = 0.0;
        let mut i = 0.0;
        while i < WARM / H {
            self.step();
            i += 1.0;
        }
    }
}

impl Piece for DoublePendulum {
    const NAME: &'static str = "double-pendulum";
    const COLS: usize = COLS;
    const ROWS: usize = ROWS;
    const FPS: u32 = 30;
    const CELL: usize = 2;
    const PALETTE: &'static [u32] = &[hex("#6ee7ff")];
    const GROUND: u32 = 0;

    fn new() -> Self {
        let mut me = Self {
            s: START,
            // One over TRAIL: a push lands before the shift.
            trail: VecDeque::with_capacity(TRAIL + 1),
            steps: 0,
            acc: 0.0,
            last: 0.0,
            e0: kinetic(START) + potential(START),
        };
        me.reset();
        me
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        if t < self.last {
            self.reset();
        }
        self.acc += (t - self.last).clamp(0.0, 0.1) * SPEED;
        self.last = t;
        while self.acc >= H {
            self.step();
            self.acc -= H;
        }
        out.fill(text::cell(' '));
        // Oldest first, a segment at a time so a fast swing leaves no gaps.
        let len = self.trail.len();
        for i in 1..len {
            let ch = FADE[((i as f64 / len as f64) * FADE.len() as f64).floor() as usize];
            let (x0, y0) = self.trail[i - 1];
            let (x1, y1) = self.trail[i];
            let n = ((x1 - x0).abs().max((y1 - y0).abs()) * 2.0).ceil();
            let n = if n == 0.0 || n.is_nan() { 1.0 } else { n };
            let mut j = 0.0;
            while j < n {
                put(out, x0 + ((x1 - x0) * j) / n, y0 + ((y1 - y0) * j) / n, ch);
                j += 1.0;
            }
        }
        let a = self.s[0];
        let (x1, y1) = (CX + SX * a.sin(), CY + SY * a.cos());
        let (x2, y2) = tip(self.s);
        rod(out, CX, CY, x1, y1);
        rod(out, x1, y1, x2, y2);
        put(out, CX - 1.0, CY, text::cell('('));
        put(out, CX, CY, text::cell('+'));
        put(out, CX + 1.0, CY, text::cell(')'));
        put(out, x1, y1, text::cell('o'));
        put(out, x2, y2, text::cell('@'));
    }
}
