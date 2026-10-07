//! fractal tree: a trunk that forks, and forks again, seven times over, each
//! limb a little shorter than its parent, the outer twigs gathered into lobes
//! of leaves lit from the upper left. Wind bends every limb, the tips most.
//!
//! The limb ends stay `f32` as upstream's `Float32Array`s: a child grows from
//! its parent's rounded end, so an f64 copy drifts down the tree.

use std::f64::consts::PI;

use super::math::{hash2, js_round, sign_or_one, Mulberry32};
use super::{hex, text, Piece};
use crate::grid::Cell;

const DEPTH: u8 = 7;
/// Each limb this deep carries a lobe of leaves over its twigs.
const LOBE: u8 = 6;
/// Leaves in shadow to leaves in light.
const LEAF: &[u8] = b" .:oO";
/// Seconds; every wind term repeats within it.
const LOOP: f64 = 12.0;
const TAU: f64 = PI * 2.0;
/// No limb leans further than this from upright, in radians.
const MAX_LEAN: f64 = 1.1;
/// Upstream's `seed` option, at its default.
const SEED: u32 = 4;
/// The row the trunk stands on.
const GROUND: usize = 23;
/// The row the trunk first forks at.
const FORK: usize = 15;
const X0: f64 = 60.0 / 2.0 - 0.5;



/// The wind: a slow sway, a quicker shiver and a gust, all within LOOP.
fn wind(u: f64) -> f64 {
    0.65 * ((TAU * u) / 6.0).sin()
        + 0.25 * ((TAU * u) / 2.4 + 1.0).sin()
        + 0.2 * ((TAU * u) / LOOP).sin().powf(3.0)
}

struct Limb {
    parent: Option<usize>,
    depth: u8,
    turn: f64,
    len: f64,
}

/// The twigs above one limb, `limbs[start..end]`: it sits over their ends.
struct Lobe {
    start: usize,
    end: usize,
    ph: f64,
    bump: f64,
}

#[derive(Clone, Copy, Default)]
struct Placed {
    lobe: usize,
    cx: f64,
    cy: f64,
    rx: f64,
    ry: f64,
}

/// The limbs, parents before children: depth, turn from the parent, length
/// (in columns; a row is two). The first forks are the widest.
fn grow(limbs: &mut Vec<Limb>, rand: &mut Mulberry32, parent: Option<usize>, depth: u8, len: f64) {
    for side in [-1.0, 1.0] {
        let base = match depth {
            1 => 0.58,
            2 => 0.48,
            _ => 0.4 + rand.next() * 0.14,
        };
        let turn = base * side + (rand.next() - 0.5) * 0.12;
        let len_ = len * (0.92 + rand.next() * 0.16);
        limbs.push(Limb {
            parent,
            depth,
            turn,
            len: len_,
        });
        let i = limbs.len() - 1;
        if depth < DEPTH {
            grow(limbs, rand, Some(i), depth + 1, len * 0.84);
        }
    }
}

pub struct FractalTree {
    limbs: Vec<Limb>,
    lobes: Vec<Lobe>,
    order: Vec<Placed>,
    ink: Vec<u8>,
    depth_at: Vec<u8>,
    shade: Vec<f32>,
    /// How far inside the crown.
    inner: Vec<f32>,
    px: Vec<f32>,
    py: Vec<f32>,
    pa: Vec<f32>,
    cells: [Cell; 128],
}

impl FractalTree {
    fn put(&mut self, c: i64, r: i64, ch: u8, d: u8) {
        if c < 0 || c >= Self::COLS as i64 || r < 0 || r >= Self::ROWS as i64 {
            return;
        }
        let k = c as usize + r as usize * Self::COLS;
        if self.depth_at[k] != 0 && self.depth_at[k] <= d {
            return;
        }
        self.depth_at[k] = d;
        self.ink[k] = ch;
    }

    /// A limb, one cell a row where the line crosses the middle of the row, in
    /// a glyph by its slant. A shallow limb steps with underscores between
    /// rows; a thick one has a heavy core on its inner side.
    fn stroke(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, depth: u8) {
        let s = (x1 - x0) / (y0 - y1); // columns a row, rightward going up
        let ch = if s.abs() < 0.42 {
            b'|'
        } else if s > 0.0 {
            b'/'
        } else {
            b'\\'
        };
        let r_top = js_round(y1) as i64;
        let r_bot = js_round(y0) as i64 - 1;
        let mut last: Option<i64> = None;
        for r in (r_top..=r_bot).rev() {
            let c = (x0 + s * (y0 - (r as f64 + 0.5))).floor() as i64;
            self.put(c, r, ch, depth + 1);
            if depth == 1 {
                self.put(c - sign_or_one(s) as i64, r, b'#', depth + 1);
            }
            if let Some(last) = last {
                for b in c.min(last) + 1..c.max(last) {
                    self.put(if s > 0.0 { b - 1 } else { b + 1 }, r + 1, b'_', depth + 1);
                }
            }
            last = Some(c);
        }
    }
}

impl Piece for FractalTree {
    const NAME: &'static str = "fractal-tree";
    const COLS: usize = 60;
    const ROWS: usize = 24;
    const FPS: u32 = 15;
    const CELL: usize = 2;
    const PALETTE: &'static [u32] = &[hex("#9be36b")];
    const GROUND: u32 = 0;

    fn new() -> Self {
        let n = Self::COLS * Self::ROWS;
        let mut rand = Mulberry32(SEED);
        let mut limbs = Vec::new();
        grow(&mut limbs, &mut rand, None, 1, 6.0);
        let mut lobes = Vec::new();
        for (i, l) in limbs.iter().enumerate() {
            if l.depth != LOBE {
                continue;
            }
            let mut end = i + 1;
            while end < limbs.len() && limbs[end].depth > LOBE {
                end += 1;
            }
            let ph = rand.next() * TAU;
            let bump = 0.12 + rand.next() * 0.1;
            lobes.push(Lobe {
                start: i,
                end,
                ph,
                bump,
            });
        }
        let mut cells = [Cell::CLEAR; 128];
        for (b, cell) in cells.iter_mut().enumerate().take(0x7f).skip(0x20) {
            *cell = text::cell(b as u8 as char);
        }
        Self {
            order: vec![Placed::default(); lobes.len()],
            px: vec![0.0; limbs.len()],
            py: vec![0.0; limbs.len()],
            pa: vec![0.0; limbs.len()],
            limbs,
            lobes,
            ink: vec![b' '; n],
            depth_at: vec![0; n],
            shade: vec![0.0; n],
            inner: vec![0.0; n],
            cells,
        }
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        let (cols, rows) = (Self::COLS, Self::ROWS);
        self.ink.fill(b' ');
        self.depth_at.fill(0);
        self.shade.fill(-1.0);
        self.inner.fill(0.0);
        // The trunk: two cells of heartwood between its edges, flaring into
        // roots at the foot.
        for r in FORK..=GROUND {
            let r_ = r as i64;
            let c = js_round(X0 - 1.0) as i64;
            let flare = (r_ - GROUND as i64 + 2).max(0);
            for i in -flare..2 + flare {
                self.put(c + i, r_, b'#', 1);
            }
            self.put(c - 1 - flare, r_, if flare != 0 { b'/' } else { b'|' }, 1);
            self.put(c + 2 + flare, r_, if flare != 0 { b'\\' } else { b'|' }, 1);
        }
        // Each limb feels the wind a moment after its parent.
        for i in 0..self.limbs.len() {
            let (parent, depth, turn, len) = {
                let l = &self.limbs[i];
                (l.parent, l.depth, l.turn, l.len)
            };
            let df = f64::from(depth);
            let bend = (0.02 + 0.035 * df) * (0.25 + wind(t - df * 0.12));
            let (x, y, mut a) = match parent {
                None => (X0 + 0.5, FORK as f64 + 0.5, 0.0),
                Some(p) => (
                    f64::from(self.px[p]),
                    f64::from(self.py[p]),
                    f64::from(self.pa[p]),
                ),
            };
            // Limbs reach for the light: each turns a little back toward upright.
            a = (a * if depth > 2 { 0.85 } else { 1.0 } + turn + bend).clamp(-MAX_LEAN, MAX_LEAN);
            let x1 = x + a.sin() * len;
            let y1 = y - (a.cos() * len) / 2.0;
            if depth < LOBE + 1 {
                self.stroke(x, y, x1, y1, depth);
            }
            self.px[i] = x1 as f32;
            self.py[i] = y1 as f32;
            self.pa[i] = a as f32;
        }
        // The lobes, highest first, so each lower one stands in front of the
        // shaded underside of the one above it. The light falls on the crown
        // as a whole from the upper left, and on each lobe the same way.
        let (px, py) = (&self.px, &self.py);
        let (mut x0, mut x1, mut y0, mut y1) = (cols as f64, 0.0f64, rows as f64, 0.0f64);
        for b in &self.lobes {
            for j in b.start..b.end {
                let (x, y) = (f64::from(px[j]), f64::from(py[j]));
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
        }
        let (gx, gy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let (grx, gry) = ((x1 - x0) / 2.0 + 3.0, (y1 - y0) / 2.0 + 1.5);
        for (li, b) in self.lobes.iter().enumerate() {
            // Over the twigs' ends and the fork they spring from.
            let p = self.limbs[b.start]
                .parent
                .expect("a lobe's limb has a parent");
            let ends = (b.start..b.end).chain(std::iter::once(p));
            let count = (b.end - b.start + 1) as f64;
            let (mut sx, mut sy) = (0.0, 0.0);
            for j in ends.clone() {
                sx += f64::from(px[j]);
                sy += f64::from(py[j]);
            }
            let (cx, cy) = (sx / count, sy / count);
            let (mut wx, mut wy) = (0.0f64, 0.0f64);
            for j in ends {
                wx = wx.max((f64::from(px[j]) - cx).abs());
                wy = wy.max((f64::from(py[j]) - cy).abs());
            }
            self.order[li] = Placed {
                lobe: li,
                cx,
                cy: cy - 0.4,
                rx: 1.8 + wx * 0.9,
                ry: 1.0 + wy * 0.9,
            };
        }
        // Stable, as upstream's sort is; insertion so it never allocates.
        for i in 1..self.order.len() {
            let mut j = i;
            while j > 0 && self.order[j - 1].cy > self.order[j].cy {
                self.order.swap(j - 1, j);
                j -= 1;
            }
        }
        for &Placed {
            lobe,
            cx,
            cy,
            rx,
            ry,
        } in &self.order
        {
            let b = &self.lobes[lobe];
            let mut r = (cy - ry - 1.0).floor();
            while r <= cy + ry + 1.0 {
                let mut c = (cx - rx - 1.0).floor();
                while c <= cx + rx + 1.0 {
                    let (ci, ri) = (c, r);
                    c += 1.0;
                    if ci < 0.0 || ci >= cols as f64 || ri < 0.0 || ri >= rows as f64 {
                        continue;
                    }
                    let nx = (ci + 0.5 - cx) / rx;
                    let ny = (ri + 0.5 - cy) / ry;
                    let edge = 1.0 + b.bump * (ny.atan2(nx) * 3.0 + b.ph + (TAU * t) / LOOP).sin();
                    let d2 = (nx * nx + ny * ny) / (edge * edge);
                    if d2 >= 1.0 {
                        continue;
                    }
                    let gnx = (ci + 0.5 - gx) / grx;
                    let gny = (ri + 0.5 - gy) / gry;
                    let k = ci as usize + ri as usize * cols;
                    let v = 0.36 - 0.12 * gnx - 0.16 * gny - 0.24 * nx - 0.4 * ny
                        + 0.16 * (hash2(ci as i64, ri as i64) - 0.5);
                    self.shade[k] = v.clamp(0.0, 1.0) as f32;
                    self.inner[k] = f64::from(self.inner[k]).max(1.0 - d2) as f32;
                }
                r += 1.0;
            }
        }
        // Leaves hide the twigs; the main limbs show where the leaves are
        // dark. At the rim of the crown light shows between the leaves.
        for k in 0..cols * rows {
            let v = f64::from(self.shade[k]);
            let d = self.depth_at[k];
            if v < 0.0 || (d != 0 && d <= 3) || (d == 4 && v < 0.45) {
                continue;
            }
            let i = (v * 3.0).floor().min(2.0) as usize;
            self.ink[k] = LEAF[if f64::from(self.inner[k]) < 0.22 {
                if v < 0.4 {
                    1
                } else {
                    2
                }
            } else {
                2 + i
            }];
        }
        // The ground either side of the roots.
        for c in 3..cols - 3 {
            let k = c + GROUND * cols;
            if self.ink[k] == b' ' {
                self.ink[k] = if c < 6 || c > cols - 7 { b'.' } else { b'_' };
            }
        }
        for (cell, &b) in out.iter_mut().zip(self.ink.iter()) {
            *cell = self.cells[b as usize];
        }
    }
}
