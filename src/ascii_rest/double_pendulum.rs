//! double-pendulum: two equal rods hung end to end from one pivot, stepped
//! with RK4. Chaotic, so it never repeats; the lower bob leaves a fading trail.
//!
//! `double-pendulum-wide` hangs as many pendulums across the panel as fit
//! side by side, each scaled to the panel's height and let go from a slightly
//! different angle, so they start nearly together and soon have nothing in
//! common.

use std::collections::VecDeque;

use super::math::js_round;
use super::{hex, text, Canvas, Piece};
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

/// Where a pendulum hangs on a grid, and its scale: rows and columns per
/// unit rod.
#[derive(Clone, Copy)]
struct Hang {
    cx: f64,
    cy: f64,
    sx: f64,
    sy: f64,
}

impl Hang {
    /// Centred in `cols` columns from `x0` on a grid `rows` tall: both rods
    /// reach any edge but none clips.
    fn new(x0: f64, cols: usize, rows: usize) -> Self {
        let cy = (rows as f64 - 1.0) / 2.0;
        let sy = (cy - 0.6) / 2.0;
        Self {
            cx: x0 + (cols as f64 - 1.0) / 2.0,
            cy,
            sx: sy * 2.0,
            sy,
        }
    }

    /// The lower bob's place on screen.
    fn tip(&self, [a, b, ..]: State) -> (f64, f64) {
        (
            self.cx + self.sx * (a.sin() + b.sin()),
            self.cy + self.sy * (a.cos() + b.cos()),
        )
    }
}

/// Writes clipped to a `cols x rows` grid.
struct Grid<'a> {
    out: &'a mut [Cell],
    cols: usize,
    rows: usize,
}

fn put(grid: &mut Grid<'_>, x: f64, y: f64, ch: Cell) {
    let (c, r) = (js_round(x), js_round(y));
    if c >= 0.0 && c < grid.cols as f64 && r >= 0.0 && r < grid.rows as f64 {
        grid.out[r as usize * grid.cols + c as usize] = ch;
    }
}

/// A thin rod, one glyph a cell. A shallow rod is a run of dashes on each
/// row, which meets the next row low or high in the cell; a steep one is a
/// bar or a slash a row. A cell is twice as tall as it is wide, so steep
/// means more than one row in two columns.
fn rod(grid: &mut Grid<'_>, x0: f64, y0: f64, x1: f64, y1: f64) {
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

struct Pendulum {
    hang: Hang,
    start: State,
    s: State,
    trail: VecDeque<(f64, f64)>,
    steps: u64,
    acc: f64,
    last: f64,
    e0: f64,
}

impl Pendulum {
    fn new(hang: Hang, start: State) -> Self {
        let mut me = Self {
            hang,
            start,
            s: start,
            // One over TRAIL: a push lands before the shift.
            trail: VecDeque::with_capacity(TRAIL + 1),
            steps: 0,
            acc: 0.0,
            last: 0.0,
            e0: kinetic(start) + potential(start),
        };
        me.reset();
        me
    }

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
            self.trail.push_back(self.hang.tip(self.s));
            if self.trail.len() > TRAIL {
                self.trail.pop_front();
            }
        }
    }

    fn reset(&mut self) {
        self.s = self.start;
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

    fn advance(&mut self, t: f64) {
        if t < self.last {
            self.reset();
        }
        self.acc += (t - self.last).clamp(0.0, 0.1) * SPEED;
        self.last = t;
        while self.acc >= H {
            self.step();
            self.acc -= H;
        }
    }

    /// Oldest first, a segment at a time so a fast swing leaves no gaps.
    fn draw_trail(&self, out: &mut Grid<'_>) {
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
    }

    fn draw_rods(&self, out: &mut Grid<'_>) {
        let Hang { cx, cy, sx, sy } = self.hang;
        let a = self.s[0];
        let (x1, y1) = (cx + sx * a.sin(), cy + sy * a.cos());
        let (x2, y2) = self.hang.tip(self.s);
        rod(out, cx, cy, x1, y1);
        rod(out, x1, y1, x2, y2);
        put(out, cx - 1.0, cy, text::cell('('));
        put(out, cx, cy, text::cell('+'));
        put(out, cx + 1.0, cy, text::cell(')'));
        put(out, x1, y1, text::cell('o'));
        put(out, x2, y2, text::cell('@'));
    }
}

struct Scene {
    cols: usize,
    rows: usize,
    swing: Vec<Pendulum>,
}

impl Scene {
    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        for p in &mut self.swing {
            p.advance(t);
        }
        out.fill(text::cell(' '));
        let mut grid = Grid {
            out,
            cols: self.cols,
            rows: self.rows,
        };
        for p in &self.swing {
            p.draw_trail(&mut grid);
        }
        for p in &self.swing {
            p.draw_rods(&mut grid);
        }
    }
}

pub struct DoublePendulum(Scene);

impl Piece for DoublePendulum {
    const NAME: &'static str = "double-pendulum";
    const COLS: usize = COLS;
    const ROWS: usize = ROWS;
    const FPS: u32 = 30;
    const CELL: usize = 2;
    const PALETTE: &'static [u32] = &[hex("#6ee7ff")];
    const GROUND: u32 = 0;

    fn new() -> Self {
        let hang = Hang::new(0.0, COLS, ROWS);
        Self(Scene {
            cols: COLS,
            rows: ROWS,
            swing: vec![Pendulum::new(hang, START)],
        })
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        self.0.frame(t, out);
    }
}

pub struct DoublePendulumWide(Scene);

impl Canvas for DoublePendulumWide {
    const NAME: &'static str = "double-pendulum-wide";
    const FPS: u32 = DoublePendulum::FPS;
    const PALETTE: &'static [u32] = DoublePendulum::PALETTE;

    /// One pendulum a slot as wide as its reach (with a little air), the
    /// slots sharing out the width; each lets go a little further round.
    fn new(cols: usize, rows: usize) -> Self {
        let reach = 4.0 * Hang::new(0.0, cols, rows).sx + 6.0;
        let n = ((cols as f64 / reach).round() as usize).max(1);
        let swing = (0..n)
            .map(|i| {
                let (x0, x1) = (i * cols / n, (i + 1) * cols / n);
                let [a, b, p, q] = START;
                let k = i as f64 - (n - 1) as f64 / 2.0;
                let hang = Hang::new(x0 as f64, x1 - x0, rows);
                Pendulum::new(hang, [a + 0.04 * k, b - 0.03 * k, p, q])
            })
            .collect();
        Self(Scene { cols, rows, swing })
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        self.0.frame(t, out);
    }
}
