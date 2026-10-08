//! Hypercube — a rotating, inverting tesseract wireframe.
//!
//! # The effect
//!
//! A 4-cube is 16 vertices and 32 edges. What makes it read as *four*
//! dimensional rather than as a cube in a box is the 4D perspective divide:
//! `1/(w_dist - w)` shrinks the half of the figure that is far in W, so the
//! tesseract's two cubes render as one cube inside another. Turn on any
//! rotation that touches W and the two swap places — the inner cube swells
//! through the outer one and becomes it. That turning-inside-out IS the saver;
//! a tesseract animated only in XY/XZ/YZ is a spinning cube with a frame around
//! it and nobody can tell it is 4D. So `new` refuses to leave all three W
//! planes at zero, and `the_w_rotation_actually_inverts_the_figure` fails if
//! the inversion stops happening.
//!
//! Six planes are available (XY, XZ, YZ, XW, YW, ZW), each with its own rate
//! knob. The defaults run three at rates with no small common period, so the
//! figure never visibly returns to a pose it held a minute ago.
//!
//! # Why braille
//!
//! `font::BRAILLE` is a 2x4 grid of filled quadrants per cell, so at the
//! default 8x16 cell a dot is a square 4x4 pixel and the drawing surface is
//! 8x the cell grid — 480x268 dots at 1080p. Wireframe lines live or die on
//! that resolution. Colour stays per CELL, which is fine: depth cueing is a
//! gradient along an edge, not per-dot detail.
//!
//! # Per-frame cost
//!
//! The figure moves entirely every frame, so this cannot be as sparse as
//! `city`. It is still nowhere near a repaint: 32 clipped Bresenham runs touch
//! a couple of thousand of the ~16k cells. The bookkeeping is the `city`
//! pattern — cells touched last frame are cleared, cells touched this frame are
//! written, and `dirty` is the union. Both lists are reserved in `new` against
//! a hard bound (clipping guarantees an edge cannot touch more cells than the
//! longer side of the dot grid), so `render` never allocates.
//!
//! Depth cueing does double duty. Shade is interpolated per dot from W (mostly)
//! and Z (a little), and where edges share a cell the NEARER shade wins. At
//! peak inversion every edge crosses the centre at once; without nearest-wins
//! the middle of the figure averages into a flat smear and the near face stops
//! reading as near.

use std::f32::consts::TAU;

use crate::font;
use crate::grid::{bake, dot_bit, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, env_str};

/// Index 0 is the unlit cell. 1..=16 is one ramp, cool and dim at the far end
/// of W to hot and bright at the near end.
///
/// Cool-to-hot rather than one hue ramped in brightness because both cues are
/// doing work at once: on black, blue reads as receding and amber as
/// advancing, so the hue carries depth even where two edges are a similar
/// brightness — and brightness still rises monotonically along the ramp, so
/// the depth survives for a viewer who cannot separate the hues at all.
const PAL_RGB: [[u8; 3]; 17] = [
    [0x00, 0x00, 0x00],
    [0x0E, 0x16, 0x4A],
    [0x12, 0x22, 0x6C],
    [0x15, 0x32, 0x8E],
    [0x17, 0x45, 0xAD],
    [0x19, 0x5A, 0xC6],
    [0x1C, 0x73, 0xD6],
    [0x21, 0x8E, 0xE0],
    [0x2B, 0xA8, 0xE4],
    [0x3E, 0xC0, 0xDC],
    [0x5E, 0xD2, 0xC4],
    [0x8B, 0xE0, 0xA4],
    [0xBC, 0xE8, 0x84],
    [0xE2, 0xDE, 0x6A],
    [0xF8, 0xC4, 0x56],
    [0xFF, 0xDE, 0x92],
    [0xFF, 0xF6, 0xDC],
];

const PAL: [u32; 17] = bake(&PAL_RGB);
const SHADES: usize = 16;

/// Unit tesseract, one vertex per 4-bit pattern. Bit b is axis b, so a vertex
/// pair differing in exactly one bit is an edge — which is how `EDGES` is
/// derived rather than typed out.
const fn base_verts() -> [[f32; 4]; 16] {
    let mut v = [[0.0f32; 4]; 16];
    let mut i = 0;
    while i < 16 {
        let mut b = 0;
        while b < 4 {
            v[i][b] = if i & (1 << b) != 0 { 1.0 } else { -1.0 };
            b += 1;
        }
        i += 1;
    }
    v
}

/// The 32 edges: every pair of vertices at Hamming distance 1, kept once.
const fn base_edges() -> [(u8, u8); 32] {
    let mut e = [(0u8, 0u8); 32];
    let mut n = 0;
    let mut i = 0;
    while i < 16 {
        let mut b = 0;
        while b < 4 {
            let j = i ^ (1 << b);
            if j > i {
                e[n] = (i as u8, j as u8);
                n += 1;
            }
            b += 1;
        }
        i += 1;
    }
    e
}

const VERTS: [[f32; 4]; 16] = base_verts();
const EDGES: [(u8, u8); 32] = base_edges();

/// The six rotation planes, as the pair of coordinate indices each mixes.
/// Order is fixed and the composition does not commute, but any fixed order
/// gives the same family of motions — this one only has to stay stable so the
/// rate knobs keep meaning the same thing.
const PLANES: [(usize, usize); 6] = [(0, 1), (0, 2), (1, 2), (0, 3), (1, 3), (2, 3)];
const PLANE_KEYS: [&str; 6] = [
    "HYPERCUBE_RATE_XY",
    "HYPERCUBE_RATE_XZ",
    "HYPERCUBE_RATE_YZ",
    "HYPERCUBE_RATE_XW",
    "HYPERCUBE_RATE_YW",
    "HYPERCUBE_RATE_ZW",
];
/// Milli-revolutions per second. The three non-zero defaults are pairwise
/// coprime and none divides another, so the composed pose has no period short
/// enough to notice. XW is the fastest because it is the one inverting the
/// figure — a full turn-inside-out every 21 seconds.
const RATE_DEFAULT: [i64; 6] = [0, 13, 29, 47, 0, 0];
/// Indices into `PLANES` of the planes that touch W. At least one must turn.
const W_PLANES: [usize; 3] = [3, 4, 5];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Style {
    /// 2x4 dots per cell. The default, and the only one that draws a line
    /// rather than a staircase of cells.
    Braille,
    /// The fire ramp keyed to how much of the cell the line covers — a
    /// wireframe as text, for a panel where a 4px dot is too fine to see.
    Ascii,
    /// Whole cell lit. Chunky, but unmistakable across a room.
    Block,
}

impl Style {
    fn from_env() -> Self {
        match env_str(&["HYPERCUBE_STYLE"], "braille").as_str() {
            "ascii" => Self::Ascii,
            "block" => Self::Block,
            _ => Self::Braille,
        }
    }

    /// A cell's dot mask as a glyph. Only `Braille` uses the dot positions; the
    /// other two use the population count. A mask of 0 is unreachable — a cell
    /// is only committed once a dot lit it — so every style yields a lit glyph.
    #[inline]
    fn glyph(self, mask: u8) -> u16 {
        match self {
            Self::Braille => font::BRAILLE[mask as usize],
            Self::Ascii => font::RAMP[mask.count_ones() as usize + 1],
            Self::Block => font::SOLID,
        }
    }
}

/// A projected vertex: dot coordinates and its depth shade.
#[derive(Clone, Copy, Default)]
struct Proj {
    x: f32,
    y: f32,
    s: f32,
}

pub struct Hypercube {
    grid: Grid,
    style: Style,
    /// Dot grid: `cols * 2` by `rows * 4`.
    dw: usize,
    dh: usize,
    cols: usize,
    /// Rotation phase per plane as a fraction of a turn in u32 units, advanced
    /// by wrapping integer addition. NOT a float accumulator: this pod runs for
    /// weeks, and a radian counter that keeps adding a small dt loses its low
    /// bits once it is large — after a few million frames the step quantises
    /// and the motion visibly stutters, then stops. An integer phase is exact
    /// forever and wraps at exactly one turn.
    phase: [u32; 6],
    step: [u32; 6],
    w_dist: f32,
    z_dist: f32,
    scale: f32,
    /// Per cell: the dots this frame lit, and the nearest shade among them.
    mask: Vec<u8>,
    shade: Vec<u8>,
    /// Frame id per cell, so a cell three edges cross enters `touched` once.
    stamp: Vec<u32>,
    frame: u32,
    touched: Vec<u32>,
    prev_touched: Vec<u32>,
    dirty: Vec<u32>,
    proj: [Proj; 16],
}

impl Hypercube {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        let cell_w = env_num(&["HYPERCUBE_CELL_W"], 8, 4, 64) as usize;
        let cell_h = env_num(&["HYPERCUBE_CELL_H"], 16, 8, 128) as usize;
        let grid = Grid::new(panel, cell_w, cell_h);
        let (cols, rows) = (grid.cols(), grid.rows());
        let cells = cols * rows;
        let (dw, dh) = (cols * 2, rows * 4);

        let mut rate = RATE_DEFAULT;
        for ((r, key), def) in rate.iter_mut().zip(PLANE_KEYS).zip(RATE_DEFAULT) {
            *r = env_num(&[key], def, -2000, 2000);
        }
        // The inversion is the whole saver, so an env block that switched every
        // W plane off would silently ship a spinning cube. Put ZW back.
        if W_PLANES.iter().all(|&i| rate[i] == 0) {
            eprintln!(
                "[screensaver] hypercube: every W-plane rate is 0, which is just a 3D cube; \
                 using HYPERCUBE_RATE_ZW=29"
            );
            rate[5] = 29;
        }

        let mut step = [0u32; 6];
        for (s, &r) in step.iter_mut().zip(rate.iter()) {
            // milli-revs/sec -> u32 turn fractions per frame, exactly.
            *s = ((r as i128) * (1i128 << 32) / (1000 * fps.max(1) as i128)) as i64 as u32;
        }

        // The shorter panel side, in dots, sets the size: the figure swells
        // past 3x its rest radius at peak inversion, and clipping that
        // overshoot is better than drawing the whole thing small enough that it
        // never clips.
        let short = dw.min(dh) as f32;

        // Hard bound on the cells one edge can touch. Every edge is clipped
        // INTO the dot rect before it is walked, so a Bresenham run is at most
        // `max(dw, dh)` steps and cannot touch more cells than steps.
        let cap = EDGES.len() * (dw.max(dh) + 2);

        Self {
            grid,
            style: Style::from_env(),
            dw,
            dh,
            cols,
            phase: [0; 6],
            step,
            w_dist: env_num(&["HYPERCUBE_W_DIST"], 2400, 1200, 20_000) as f32 / 1000.0,
            z_dist: env_num(&["HYPERCUBE_Z_DIST"], 6000, 2000, 40_000) as f32 / 1000.0,
            scale: short * env_num(&["HYPERCUBE_SCALE"], 130, 20, 400) as f32 / 1000.0,
            mask: vec![0; cells],
            shade: vec![0; cells],
            stamp: vec![0; cells],
            frame: 0,
            touched: Vec::with_capacity(cap),
            prev_touched: Vec::with_capacity(cap),
            dirty: Vec::with_capacity(cap * 2),
            proj: [Proj::default(); 16],
        }
    }

    /// Rotate every vertex, project 4D -> 3D -> 2D, and record dot coordinates
    /// and depth shade. Trig is six sin/cos for the whole frame, not per vertex.
    fn project(&mut self) {
        let mut sc = [(0.0f32, 0.0f32); 6];
        for (t, &p) in sc.iter_mut().zip(self.phase.iter()) {
            *t = (p as f32 * (TAU / 4_294_967_296.0)).sin_cos();
        }
        let (cx, cy) = (self.dw as f32 * 0.5, self.dh as f32 * 0.5);
        for (out, base) in self.proj.iter_mut().zip(VERTS.iter()) {
            let mut v = *base;
            for ((a, b), (sin, cos)) in PLANES.into_iter().zip(sc) {
                let (p, q) = (v[a], v[b]);
                v[a] = p * cos - q * sin;
                v[b] = p * sin + q * cos;
            }
            // 4D -> 3D. This divide is what makes the far-in-W cube the small
            // one; an orthographic 4D projection renders both the same size and
            // the inversion disappears entirely. Clamped because w_dist is a
            // knob and a user can set it inside the figure. Clamping to a
            // FIFTH of w_dist rather than to an epsilon caps the swell at 5x
            // whatever the knobs say: an epsilon lets a vertex near the eye
            // project to thousands of dots, which is legal but reads as the
            // figure exploding.
            let k4 = self.w_dist / (self.w_dist - v[3]).max(self.w_dist * 0.2);
            let (x, y, z) = (v[0] * k4, v[1] * k4, v[2] * k4);
            let k3 = self.z_dist / (self.z_dist - z).max(self.z_dist * 0.2);
            out.x = cx + x * k3 * self.scale;
            out.y = cy + y * k3 * self.scale;
            // Depth cue, mostly W and a little Z. W dominates because the
            // inversion is a W event: keyed to Z alone the two cubes would swap
            // size while keeping their colour, which reads as a glitch.
            let d = 0.72 * (v[3] * 0.55) + 0.28 * (v[2] * 0.55);
            out.s = ((d + 1.0) * 0.5).clamp(0.0, 1.0) * (SHADES - 1) as f32;
        }
    }

    /// Blank every cell the last frame lit, and open `dirty` with them. A cell
    /// this frame lights again is written back below; one that is not stays
    /// clear, which is the whole of "the figure leaves no trail".
    fn clear_prev(&mut self) {
        self.dirty.clear();
        for &i in &self.prev_touched {
            self.mask[i as usize] = 0;
            self.shade[i as usize] = 0;
            self.grid.set(i as usize, Cell::CLEAR);
            self.dirty.push(i);
        }
        self.touched.clear();
    }

    #[inline]
    fn plot(&mut self, dx: usize, dy: usize, shade: f32) {
        let i = (dy >> 2) * self.cols + (dx >> 1);
        if self.stamp[i] != self.frame {
            self.stamp[i] = self.frame;
            self.touched.push(i as u32);
        }
        self.mask[i] |= dot_bit(dx & 1, dy & 3);
        // Nearest wins. At peak inversion most edges pass through the middle of
        // the figure at once; averaging there flattens the near face into the
        // far one and the whole centre reads as a smear.
        let s = shade as u8;
        if s > self.shade[i] {
            self.shade[i] = s;
        }
    }

    fn draw_edges(&mut self) {
        for &(a, b) in &EDGES {
            let (p, q) = (self.proj[a as usize], self.proj[b as usize]);
            let Some((t0, t1)) = clip(p.x, p.y, q.x, q.y, self.dw as f32, self.dh as f32) else {
                continue;
            };
            let lerp = |u: f32, v: f32, t: f32| u + (v - u) * t;
            let x0 = lerp(p.x, q.x, t0).round() as i32;
            let y0 = lerp(p.y, q.y, t0).round() as i32;
            let x1 = lerp(p.x, q.x, t1).round() as i32;
            let y1 = lerp(p.y, q.y, t1).round() as i32;
            let s0 = lerp(p.s, q.s, t0);
            let s1 = lerp(p.s, q.s, t1);

            let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
            let sx = if x0 < x1 { 1 } else { -1 };
            let sy = if y0 < y1 { 1 } else { -1 };
            let steps = dx.max(-dy).max(1) as f32;
            let (mut x, mut y, mut err) = (x0, y0, dx + dy);
            let mut n = 0.0f32;
            loop {
                // Clipping already put the segment inside the rect; the clamp
                // covers the half-dot the rounding above can push out.
                let px = (x.max(0) as usize).min(self.dw - 1);
                let py = (y.max(0) as usize).min(self.dh - 1);
                self.plot(px, py, s0 + (s1 - s0) * (n / steps));
                if x == x1 && y == y1 {
                    break;
                }
                let e2 = 2 * err;
                if e2 >= dy {
                    err += dy;
                    x += sx;
                }
                if e2 <= dx {
                    err += dx;
                    y += sy;
                }
                n += 1.0;
            }
        }
    }

    fn commit(&mut self) {
        for &i in &self.touched {
            let c = Cell::new(
                self.style.glyph(self.mask[i as usize]),
                self.shade[i as usize] as u16 + 1,
            );
            self.grid.set(i as usize, c);
            self.dirty.push(i);
        }
    }
}

/// Liang-Barsky against the dot rect, returning the surviving parameter range.
/// Clipping rather than a per-dot bounds test because it is what makes the
/// `dirty` reserve a proof: an unclipped edge at peak inversion runs for
/// thousands of off-screen steps and nothing bounds what it would push.
fn clip(x0: f32, y0: f32, x1: f32, y1: f32, w: f32, h: f32) -> Option<(f32, f32)> {
    let (dx, dy) = (x1 - x0, y1 - y0);
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p, q) in [(-dx, x0), (dx, w - 1.0 - x0), (-dy, y0), (dy, h - 1.0 - y0)] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
            continue;
        }
        let r = q / p;
        if p < 0.0 {
            if r > t1 {
                return None;
            }
            t0 = t0.max(r);
        } else {
            if r < t0 {
                return None;
            }
            t1 = t1.min(r);
        }
    }
    Some((t0, t1))
}

impl Saver for Hypercube {
    fn render(&mut self, s: &mut Surface<'_>) {
        for (p, &d) in self.phase.iter_mut().zip(self.step.iter()) {
            *p = p.wrapping_add(d);
        }
        // Frame ids are the dedupe key, so a wrap has to take the stamps with
        // it — otherwise every cell whose stamp happens to be 0 looks
        // already-visited for one frame, drops out of `touched`, and is never
        // cleared: a permanent trail. 4.5 years at 30fps, and one memset when
        // it arrives.
        self.frame = self.frame.wrapping_add(1);
        if self.frame == 0 {
            self.stamp.fill(u32::MAX);
            self.frame = 1;
        }
        self.project();
        self.clear_prev();
        self.draw_edges();
        self.commit();
        // Row-major, because `Damage::mark` only merges with the LAST run it
        // opened. Cells arrive here in edge order, so unsorted they open a
        // fresh overlapping run per edge and the reported total comes out
        // several times the panel height — correct, but several times the
        // shadow-buffer copy it needs to be.
        self.dirty.sort_unstable();
        self.grid.flush_sparse(s, &PAL, &self.dirty);
        std::mem::swap(&mut self.touched, &mut self.prev_touched);
    }

    fn name(&self) -> &'static str {
        "hypercube"
    }

    fn grid(&self) -> &Grid {
        &self.grid
    }

    fn palette(&self) -> &[u32] {
        &PAL
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::saver;

    fn panel() -> Panel {
        Panel::new(1920, 1080, 1920)
    }

    /// Env vars are process-global and the test binary is threaded, so nothing
    /// here sets one. Knobs are poked on the struct instead.
    fn cube() -> Hypercube {
        Hypercube::new(&panel(), 30)
    }

    /// Mean projected radius of each of the two cubes — bit 3 is the W axis, so
    /// the 8 vertices with it clear are one cube and the 8 with it set the
    /// other. Under a W rotation these trade places, which IS the inversion.
    fn cube_radii(h: &Hypercube) -> (f32, f32) {
        let (cx, cy) = (h.dw as f32 * 0.5, h.dh as f32 * 0.5);
        let mut r = [0.0f32; 2];
        for (i, p) in h.proj.iter().enumerate() {
            r[(i >> 3) & 1] += ((p.x - cx).powi(2) + (p.y - cy).powi(2)).sqrt();
        }
        (r[0] / 8.0, r[1] / 8.0)
    }

    #[test]
    fn sixteen_vertices_thirty_two_edges_each_once() {
        assert_eq!(VERTS.len(), 16);
        assert_eq!(EDGES.len(), 32);
        for &(a, b) in &EDGES {
            assert_eq!((a ^ b).count_ones(), 1, "{a}-{b} is not a cube edge");
            assert!(a < b, "{a}-{b} is the same edge twice");
        }
        let mut seen = std::collections::HashSet::new();
        for &e in &EDGES {
            assert!(seen.insert(e), "{e:?} listed twice");
        }
        // Every vertex has one neighbour per axis.
        for v in 0u8..16 {
            let deg = EDGES.iter().filter(|&&(a, b)| a == v || b == v).count();
            assert_eq!(deg, 4, "vertex {v} has degree {deg}");
        }
    }

    /// The feature most likely to be silently dropped. Over one XW revolution
    /// the two cubes must SWAP which is the larger — that sign change is the
    /// figure turning inside out — and at the extremes one must be several
    /// times the other, or the "inner cube" is not visibly inner.
    ///
    /// Zero the three W rates and this fails on the sign change: the pair
    /// stay locked at an identical radius forever.
    #[test]
    fn the_w_rotation_actually_inverts_the_figure() {
        let mut h = cube();
        let p = panel();
        let mut buf = vec![0u32; p.buf_len()];
        let (mut saw_a_bigger, mut saw_b_bigger) = (false, false);
        let mut worst_ratio = 1.0f32;
        // 47 milli-revs/sec at 30fps is a full XW turn in 21s = 638 frames.
        for _ in 0..700 {
            saver::frame(&mut h, &mut buf, &p);
            let (a, b) = cube_radii(&h);
            saw_a_bigger |= a > b * 1.2;
            saw_b_bigger |= b > a * 1.2;
            worst_ratio = worst_ratio.max(a / b).max(b / a);
        }
        assert!(
            saw_a_bigger && saw_b_bigger,
            "the two cubes never traded places: no inversion is happening"
        );
        assert!(
            worst_ratio > 2.5,
            "inner/outer ratio peaked at {worst_ratio:.2}: the 4D divide is too weak to read"
        );
    }

    /// A default env block must always be turning at least one W plane, and the
    /// guard must put one back if the operator zeroes all three.
    #[test]
    fn a_w_plane_is_always_turning() {
        assert!(
            W_PLANES.iter().any(|&i| RATE_DEFAULT[i] != 0),
            "no W plane turns by default: this is a 3D cube"
        );
        let h = cube();
        assert!(
            W_PLANES.iter().any(|&i| h.step[i] != 0),
            "no W-plane step survived construction"
        );
    }

    /// Frame 0 has to cover the panel, strip included, and has to cover it with
    /// the actual figure rather than a reported black frame.
    #[test]
    fn frame_zero_paints_the_whole_panel() {
        let p = panel();
        let mut h = cube();
        let mut buf = vec![0u32; p.buf_len()];
        let d = saver::frame(&mut h, &mut buf, &p);
        assert_eq!(d.rows(), p.h, "frame 0 must paint every scanline");
        assert!(
            buf.iter().filter(|&&px| px != 0).count() > 2_000,
            "frame 0 drew nothing"
        );
    }

    /// The "screen went blank" bug class. A framebuffer diff is structurally
    /// blind to it: a write left out of `dirty` is never blitted, so the
    /// framebuffer never changes and there is nothing to compare. The check has
    /// to be against the grid's own pair of buffers — `flush_sparse` copies
    /// `cur[i]` into `prev[i]` for exactly the indices in `dirty`, so
    /// `cell(i) != cells()[i]` after a frame means cell `i` was written and not
    /// reported.
    ///
    /// And the trail check, in the same loop and exact rather than a ratio: the
    /// set of non-clear cells must be EXACTLY the set the figure touched. A
    /// ratio waves real trails through, because a trail saturates.
    #[test]
    fn every_written_cell_is_reported_and_nothing_is_left_behind() {
        let p = panel();
        let mut h = cube();
        let mut buf = vec![0u32; p.buf_len()];
        let cells = h.grid.cols() * h.grid.rows();
        let mut rows = Vec::new();
        let mut lit_seen = 0usize;

        for n in 0..600 {
            saver::frame(&mut h, &mut buf, &p);
            for i in 0..cells {
                assert_eq!(
                    h.grid.cell(i),
                    h.grid.cells()[i],
                    "frame {n}: cell {i} was written but not reported"
                );
            }
            // `touched` and `prev_touched` were swapped by `render`, so the
            // cells this frame drew are in `prev_touched`.
            let drawn: std::collections::HashSet<u32> = h.prev_touched.iter().copied().collect();
            assert_eq!(
                drawn.len(),
                h.prev_touched.len(),
                "frame {n}: duplicate cell"
            );
            for i in 0..cells {
                let clear = h.grid.cell(i) == Cell::CLEAR;
                assert_eq!(
                    !clear,
                    drawn.contains(&(i as u32)),
                    "frame {n}: cell {i} lit={} but touched={}",
                    !clear,
                    drawn.contains(&(i as u32))
                );
            }
            lit_seen = lit_seen.max(drawn.len());
            rows.push(saver::frame(&mut h, &mut buf, &p).rows());
        }
        assert!(
            lit_seen > 500,
            "only {lit_seen} cells ever lit: nothing drawn"
        );
        rows.sort_unstable();
        let median = rows[rows.len() / 2];
        // The figure is a connected blob so damage is essentially its bounding
        // box, which is most of the panel HEIGHT. The claim worth pinning is
        // only that it is not the whole panel every frame — the real cost
        // argument is cells blitted, which `lit_seen` bounds at a fraction of
        // the grid.
        assert!(median < p.h, "median damage {median} of {} rows", p.h);
        assert!(
            lit_seen * 4 < cells,
            "{lit_seen} of {cells} cells lit: this is a repaint, not a wireframe"
        );
    }

    /// CLAUDE.md makes the frame loop the top constraint in this repo. Capacity
    /// rather than length: a Vec that never grows past its reserve never
    /// reallocates, so an unchanged capacity after a long run IS "render did
    /// not allocate". Non-vacuous because the assert below shows the lists get
    /// genuinely full.
    #[test]
    fn render_never_allocates() {
        let p = panel();
        let mut h = cube();
        let mut buf = vec![0u32; p.buf_len()];
        let (rt, rd) = (h.touched.capacity(), h.dirty.capacity());
        assert!(rt > 0 && rd > 0, "nothing was reserved for the frame loop");
        let mut worst = 0;
        for _ in 0..20_000 {
            saver::frame(&mut h, &mut buf, &p);
            worst = worst.max(h.dirty.len());
            assert_eq!(h.touched.capacity(), rt, "`touched` reallocated");
            assert_eq!(h.prev_touched.capacity(), rt, "`prev_touched` reallocated");
            assert_eq!(h.dirty.capacity(), rd, "`dirty` reallocated");
        }
        assert!(
            worst > 1_000,
            "`dirty` only ever held {worst}: reserve unproven"
        );
    }

    /// Weeks of uptime. The phase is integer, so the only way it can go wrong
    /// is if it were not — check the closed form at a frame count no test can
    /// actually run, then confirm the figure is still being drawn there.
    #[test]
    fn rotation_is_exact_after_a_billion_frames() {
        let mut h = cube();
        const N: u32 = 1_000_000_000;
        let expect: Vec<u32> = h.step.iter().map(|&s| s.wrapping_mul(N)).collect();

        let p = panel();
        let mut buf = vec![0u32; p.buf_len()];
        saver::frame(&mut h, &mut buf, &p);
        // Jump to just short of a billion frames, then walk the last few
        // through the real render path.
        for (ph, &s) in h.phase.iter_mut().zip(h.step.iter()) {
            *ph = s.wrapping_mul(N - 5);
        }
        h.frame = u32::MAX - 2;
        for _ in 0..5 {
            saver::frame(&mut h, &mut buf, &p);
        }
        assert_eq!(h.phase.to_vec(), expect, "the phase drifted");
        let (a, b) = cube_radii(&h);
        assert!(
            a.is_finite() && b.is_finite() && a > 1.0 && b > 1.0,
            "the figure collapsed after a billion frames: {a} {b}"
        );
        let cells = h.grid.cols() * h.grid.rows();
        for i in 0..cells {
            assert_eq!(
                h.grid.cell(i),
                h.grid.cells()[i],
                "cell {i} stale post-wrap"
            );
        }
    }

    /// The other half of long uptime, and the half a frame count cannot reach:
    /// the frame id is the dedupe key, so when it wraps, every stamp still
    /// holding the id it is about to reuse would make its cell look
    /// already-visited — dropped from `touched`, so never committed and, worse,
    /// never cleared, leaving stale dots in its mask forever.
    ///
    /// Delete the `stamp.fill` and this fails: the untouched cells still claim
    /// the id the counter just landed on.
    #[test]
    fn the_frame_id_wrap_resets_the_stamps() {
        let p = panel();
        let mut h = cube();
        let mut buf = vec![0u32; p.buf_len()];
        saver::frame(&mut h, &mut buf, &p);
        h.stamp.fill(1);
        h.frame = u32::MAX;
        saver::frame(&mut h, &mut buf, &p);
        assert_eq!(h.frame, 1, "the wrap did not land on the reserved id");
        let drawn: std::collections::HashSet<u32> = h.prev_touched.iter().copied().collect();
        assert!(!drawn.is_empty(), "nothing was drawn across the wrap");
        for (i, &st) in h.stamp.iter().enumerate() {
            if drawn.contains(&(i as u32)) {
                continue;
            }
            assert_eq!(
                st,
                u32::MAX,
                "cell {i} kept a stamp that collides with the reused frame id"
            );
        }
    }

    /// Clipping is what bounds the reserve, so it has to actually clip. An edge
    /// wholly off one side must vanish, and one crossing the rect must come
    /// back trimmed to it.
    #[test]
    fn edges_are_clipped_to_the_dot_rect() {
        assert!(clip(-50.0, -50.0, -10.0, -10.0, 100.0, 100.0).is_none());
        assert!(clip(200.0, 10.0, 300.0, 20.0, 100.0, 100.0).is_none());
        let (t0, t1) = clip(-100.0, 50.0, 100.0, 50.0, 100.0, 100.0).unwrap();
        // x spans -100..100; the rect keeps 0..99.
        assert!((t0 - 0.5).abs() < 1e-4, "t0={t0}");
        assert!((t1 - 0.995).abs() < 1e-3, "t1={t1}");
        let (t0, t1) = clip(10.0, 10.0, 20.0, 20.0, 100.0, 100.0).unwrap();
        assert_eq!(
            (t0, t1),
            (0.0, 1.0),
            "a segment inside the rect was trimmed"
        );
    }

    /// Every style has to yield a LIT glyph for every reachable mask, or a
    /// touched cell reads as clear and the trail invariant above becomes a lie.
    #[test]
    fn no_style_turns_a_lit_cell_blank() {
        for style in [Style::Braille, Style::Ascii, Style::Block] {
            for mask in 1u8..=255 {
                assert_ne!(
                    style.glyph(mask),
                    font::BLANK,
                    "{style:?} mask {mask:08b} is blank"
                );
            }
        }
    }
}
