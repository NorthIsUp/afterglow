//! fractal tree: a trunk that forks, and forks again, seven times over, each
//! limb a little shorter than its parent, the outer twigs gathered into lobes
//! of leaves lit from the upper left. Wind bends every limb, the tips most.
//!
//! The limb ends stay `f32` as upstream's `Float32Array`s: a child grows from
//! its parent's rounded end, so an f64 copy drifts down the tree.
//!
//! `fractal-tree-wide` grows a grove across the panel: upstream's tree, scaled
//! to the panel's height, in the middle, smaller trees of other seeds either
//! side out past both edges, all on one ground line that runs edge to edge.
//! The wind reaches each tree a moment after the one to its left, so a gust
//! crosses the grove.

use std::f64::consts::PI;

use super::math::{hash2, js_round, sign_or_one, Mulberry32};
use super::{hex, text, Canvas, Piece};
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
const COLS: usize = 60;
const ROWS: usize = 24;
/// The row the trunk stands on.
const GROUND: usize = 23;
/// The row the trunk first forks at.
const FORK: usize = 15;
const X0: f64 = 60.0 / 2.0 - 0.5;
/// The trunk's first limbs, in columns.
const LEN: f64 = 6.0;

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

/// One tree: where it stands, how big, and its limbs and lobes.
struct Tree {
    x0: f64,
    fork: usize,
    /// Upstream's sizes to this tree's.
    s: f64,
    /// Seconds the wind reaches it late.
    lag: f64,
    /// The share of its brightest leaves in blossom, in the twin.
    blossom: f64,
    limbs: Vec<Limb>,
    lobes: Vec<Lobe>,
    order: Vec<Placed>,
    px: Vec<f32>,
    py: Vec<f32>,
    pa: Vec<f32>,
}

impl Tree {
    fn new(x0: f64, fork: usize, s: f64, lag: f64, seed: u32, blossom: f64) -> Self {
        let mut rand = Mulberry32(seed);
        let mut limbs = Vec::new();
        grow(&mut limbs, &mut rand, None, 1, LEN * s);
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
        Self {
            x0,
            fork,
            s,
            lag,
            blossom,
            order: vec![Placed::default(); lobes.len()],
            px: vec![0.0; limbs.len()],
            py: vec![0.0; limbs.len()],
            pa: vec![0.0; limbs.len()],
            limbs,
            lobes,
        }
    }
}

/// One tree's strokes and leaves, cell by cell, before it joins the grove.
struct Canopy {
    cols: usize,
    rows: usize,
    ink: Vec<u8>,
    depth_at: Vec<u8>,
    shade: Vec<f32>,
    /// How far inside the crown.
    inner: Vec<f32>,
}

impl Canopy {
    fn put(&mut self, c: i64, r: i64, ch: u8, d: u8) {
        if c < 0 || c >= self.cols as i64 || r < 0 || r >= self.rows as i64 {
            return;
        }
        let k = c as usize + r as usize * self.cols;
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

    /// `tree` at `t`, standing on row `ground`, into a cleared canopy.
    fn draw(&mut self, tree: &mut Tree, ground: usize, t: f64) {
        let (cols, rows, s) = (self.cols, self.rows, tree.s);
        self.ink.fill(b' ');
        self.depth_at.fill(0);
        self.shade.fill(-1.0);
        self.inner.fill(0.0);
        // The trunk: two cells of heartwood between its edges, flaring into
        // roots at the foot.
        for r in tree.fork..=ground {
            let r_ = r as i64;
            let c = js_round(tree.x0 - 1.0) as i64;
            let flare = (r_ - ground as i64 + 2).max(0);
            for i in -flare..2 + flare {
                self.put(c + i, r_, b'#', 1);
            }
            self.put(c - 1 - flare, r_, if flare != 0 { b'/' } else { b'|' }, 1);
            self.put(c + 2 + flare, r_, if flare != 0 { b'\\' } else { b'|' }, 1);
        }
        // Each limb feels the wind a moment after its parent.
        for i in 0..tree.limbs.len() {
            let (parent, depth, turn, len) = {
                let l = &tree.limbs[i];
                (l.parent, l.depth, l.turn, l.len)
            };
            let df = f64::from(depth);
            let bend = (0.02 + 0.035 * df) * (0.25 + wind(t - tree.lag - df * 0.12));
            let (x, y, mut a) = match parent {
                None => (tree.x0 + 0.5, tree.fork as f64 + 0.5, 0.0),
                Some(p) => (
                    f64::from(tree.px[p]),
                    f64::from(tree.py[p]),
                    f64::from(tree.pa[p]),
                ),
            };
            // Limbs reach for the light: each turns a little back toward upright.
            a = (a * if depth > 2 { 0.85 } else { 1.0 } + turn + bend).clamp(-MAX_LEAN, MAX_LEAN);
            let x1 = x + a.sin() * len;
            let y1 = y - (a.cos() * len) / 2.0;
            if depth < LOBE + 1 {
                self.stroke(x, y, x1, y1, depth);
            }
            tree.px[i] = x1 as f32;
            tree.py[i] = y1 as f32;
            tree.pa[i] = a as f32;
        }
        // The lobes, highest first, so each lower one stands in front of the
        // shaded underside of the one above it. The light falls on the crown
        // as a whole from the upper left, and on each lobe the same way.
        let (px, py) = (&tree.px, &tree.py);
        let (mut x0, mut x1, mut y0, mut y1) = (cols as f64, 0.0f64, rows as f64, 0.0f64);
        for b in &tree.lobes {
            for j in b.start..b.end {
                let (x, y) = (f64::from(px[j]), f64::from(py[j]));
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
        }
        let (gx, gy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let (grx, gry) = ((x1 - x0) / 2.0 + 3.0 * s, (y1 - y0) / 2.0 + 1.5 * s);
        for (li, b) in tree.lobes.iter().enumerate() {
            // Over the twigs' ends and the fork they spring from.
            let p = tree.limbs[b.start]
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
            tree.order[li] = Placed {
                lobe: li,
                cx,
                cy: cy - 0.4 * s,
                rx: 1.8 * s + wx * 0.9,
                ry: 1.0 * s + wy * 0.9,
            };
        }
        // Stable, as upstream's sort is; insertion so it never allocates.
        for i in 1..tree.order.len() {
            let mut j = i;
            while j > 0 && tree.order[j - 1].cy > tree.order[j].cy {
                tree.order.swap(j - 1, j);
                j -= 1;
            }
        }
        for &Placed {
            lobe,
            cx,
            cy,
            rx,
            ry,
        } in &tree.order
        {
            let b = &tree.lobes[lobe];
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
    }
}

/// The ground line's ends: columns left bare, then columns of dots before
/// the rule.
struct Ground {
    row: usize,
    bare: usize,
    dots: usize,
}

/// The twin's colours: a brown trunk darkening into the branches, olive
/// twigs, leaves in four greens by their light, blossom, and earth.
const TRUNK: u8 = 0;
const BARK: u8 = 1;
const TWIG: u8 = 2;
const LEAVES: u8 = 3;
const BLOSSOM: u8 = 7;
const EARTH: u8 = 8;
const WIDE_PALETTE: &[u32] = &[
    hex("#9a6236"),
    hex("#7a5a30"),
    hex("#7a8a3a"),
    hex("#2e6a2a"),
    hex("#4f9a3a"),
    hex("#7ccf4a"),
    hex("#b4f070"),
    hex("#ffb0d0"),
    hex("#7a5a3a"),
];

/// A cell's colour in the twin, from its glyph and how deep its limb is:
/// `LEAF`'s glyphs are leaves, by light, now and then in blossom at
/// `blossom` (a share of the brightest leaves); the rest are wood.
fn tone_of(ink: u8, depth: u8, blossom: f64, c: usize, r: usize) -> u8 {
    match LEAF.iter().position(|&l| l == ink) {
        Some(i) if i >= 3 && hash2(c as i64 + 911, r as i64) < blossom => BLOSSOM,
        Some(i) => LEAVES + i.saturating_sub(1) as u8,
        None if depth <= 2 => TRUNK,
        None if depth <= 4 => BARK,
        None => TWIG,
    }
}

struct Scene {
    /// Back to front.
    trees: Vec<Tree>,
    ground: Ground,
    canopy: Canopy,
    ink: Vec<u8>,
    cells: [Cell; 128],
    /// Each cell's `tone_of` this frame, for the twin; empty upstream.
    tone: Vec<u8>,
}

impl Scene {
    fn new(cols: usize, rows: usize, trees: Vec<Tree>, ground: Ground, colour: bool) -> Self {
        let n = cols * rows;
        let mut cells = [Cell::CLEAR; 128];
        for (b, cell) in cells.iter_mut().enumerate().take(0x7f).skip(0x20) {
            *cell = text::cell(b as u8 as char);
        }
        Self {
            trees,
            ground,
            canopy: Canopy {
                cols,
                rows,
                ink: vec![b' '; n],
                depth_at: vec![0; n],
                shade: vec![0.0; n],
                inner: vec![0.0; n],
            },
            ink: vec![b' '; n],
            cells,
            tone: if colour { vec![0; n] } else { Vec::new() },
        }
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        let cols = self.canopy.cols;
        self.ink.fill(b' ');
        let colour = !self.tone.is_empty();
        for tree in &mut self.trees {
            self.canopy.draw(tree, self.ground.row, t);
            for (k, (o, &i)) in self.ink.iter_mut().zip(&self.canopy.ink).enumerate() {
                if i != b' ' {
                    *o = i;
                    if colour {
                        let d = self.canopy.depth_at[k];
                        self.tone[k] = tone_of(i, d, tree.blossom, k % cols, k / cols);
                    }
                }
            }
        }
        // The ground either side of the roots.
        let Ground { row, bare, dots } = self.ground;
        for c in bare..cols - bare {
            let k = c + row * cols;
            if self.ink[k] == b' ' {
                self.ink[k] = if c < bare + dots || c + bare + dots >= cols {
                    b'.'
                } else {
                    b'_'
                };
                if colour {
                    self.tone[k] = EARTH;
                }
            }
        }
        if colour {
            for ((cell, &b), &k) in out.iter_mut().zip(&self.ink).zip(&self.tone) {
                *cell = text::tint(self.cells[b as usize], u16::from(k));
            }
        } else {
            for (cell, &b) in out.iter_mut().zip(self.ink.iter()) {
                *cell = self.cells[b as usize];
            }
        }
    }
}

pub struct FractalTree(Scene);

impl Piece for FractalTree {
    const NAME: &'static str = "fractal-tree";
    const COLS: usize = COLS;
    const ROWS: usize = ROWS;
    const FPS: u32 = 15;
    const CELL: usize = 2;
    const PALETTE: &'static [u32] = &[hex("#9be36b")];
    const GROUND: u32 = 0;

    fn new() -> Self {
        let tree = Tree::new(X0, FORK, 1.0, 0.0, SEED, 0.0);
        let ground = Ground {
            row: GROUND,
            bare: 3,
            dots: 3,
        };
        Self(Scene::new(COLS, ROWS, vec![tree], ground, false))
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        self.0.frame(t, out);
    }
}

pub struct FractalTreeWide(Scene);

impl Canvas for FractalTreeWide {
    const NAME: &'static str = "fractal-tree-wide";
    const FPS: u32 = FractalTree::FPS;
    #[cfg(test)]
    const COLS: usize = <FractalTree as super::Piece>::COLS;
    #[cfg(test)]
    const ROWS: usize = <FractalTree as super::Piece>::ROWS;
    #[cfg(test)]
    const UPSTREAM: &'static [(&'static str, &'static str)] = &[];

    /// Upstream's tree scaled to the height in the middle, then trees of
    /// 55% to 80% of it walking out from it either side until one stands past
    /// the edge, the farthest first so the middle one is in front.
    fn new(cols: usize, rows: usize) -> Self {
        /// Half a full-size crown's width in columns, less a little so
        /// neighbours' leaves touch.
        const CROWN: f64 = 21.0;
        let s = rows as f64 / ROWS as f64;
        let ground = rows - 1;
        let at = |x: f64, k: f64, seed: u32| {
            let fork = ground.saturating_sub(((GROUND - FORK) as f64 * s * k).round() as usize);
            // Every other tree out from the middle is a cherry in bloom.
            let blossom = if seed.is_multiple_of(2) { 0.06 } else { 0.35 };
            Tree::new(x - 0.5, fork, s * k, 1.6 * x / cols as f64, seed, blossom)
        };
        let mid = cols as f64 / 2.0;
        let mut trees = Vec::new();
        let mut rng = Mulberry32(SEED + 100);
        for side in [-1.0, 1.0] {
            let (mut x, mut last) = (mid, 1.0);
            while (x - mid).abs() < mid + 4.0 {
                let k = 0.55 + 0.25 * rng.next();
                x += side * CROWN * s * (last + k);
                last = k;
                trees.push((x, k));
            }
        }
        // Farthest from the middle drawn first.
        trees.sort_by(|a, b| (b.0 - mid).abs().total_cmp(&(a.0 - mid).abs()));
        let mut grove: Vec<Tree> = trees
            .iter()
            .enumerate()
            .map(|(i, &(x, k))| at(x, k, SEED + 1 + i as u32))
            .collect();
        grove.push(at(mid, 1.0, SEED));
        let ground = Ground {
            row: ground,
            bare: 0,
            dots: 0,
        };
        Self(Scene::new(cols, rows, grove, ground, true))
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        self.0.frame(t, out);
    }
    fn palette(&self) -> &'static [u32] {
        WIDE_PALETTE
    }
}
