//! Points of View — a rotating platonic solid drawn as a grid of dots on its
//! own surface, bursting apart and reassembling as the next solid every ten
//! seconds.
//!
//! # The effect
//!
//! Five solids in a fixed cycle: tetrahedron, cube, octahedron, dodecahedron,
//! icosahedron. The surface is SAMPLED, not outlined — a triangular lattice
//! over every face — because a wireframe of a dodecahedron reads as a ball of
//! sticks, where a dot grid reads as a solid whose faces slide past one another
//! as it turns. Faces and edges stay legible for free: an edge is shared by two
//! faces, so both lattices land on it and it comes out twice as dense.
//!
//! Back faces are drawn too. There is no normal test and no cull: depth does
//! the work instead, so the far side shows through as a dim scatter and the
//! near side as a bright one, which is what makes the rotation read as rotation
//! rather than as a flat blob changing shape.
//!
//! # The burst
//!
//! Every solid change is a burst, not a dissolve. Each point leaves its old
//! position, is pushed OUTWARD along its own direction by a `sin(pi*u)` bump
//! that peaks mid-flight and is exactly zero at both ends, and lands on its new
//! position through an ease with a real overshoot (`easeOutBack`) — so the
//! figure snaps past its final shape and settles back into it. A linear lerp
//! was tried first and reads as a crossfade; the overshoot is what makes it
//! land.
//!
//! The scatter direction is per point, fixed at construction, and flipped at
//! use time to whichever sign points away from the point's origin — so the
//! cloud genuinely expands rather than half of it passing through the middle.
//!
//! ## Points with no destination
//!
//! The solids do not have equal point counts (the dodecahedron has the most
//! surface area at a fixed circumradius, the tetrahedron the least). The pool
//! is sized to the LARGEST, and point `p` of the pool maps to sample `p % m` of
//! whichever solid is being drawn. So no point is ever without an origin or a
//! destination and nothing has to fade in or out: on a shrink several points
//! converge onto one landing site and merge, on a grow several points leave one
//! site and split apart. Both read as the burst doing something rather than as
//! points blinking out.
//!
//! # Geometry
//!
//! Only the vertices are typed in — the ones easy to get subtly wrong, the
//! golden-ratio coordinates of the dodecahedron and icosahedron, are stated
//! once each. Edges are the pairs at the minimum distance, and faces are
//! derived: a plane through a vertex and two of its neighbours that no vertex
//! lies outside IS a face plane, and the face is every vertex on it, sorted by
//! angle about the normal. That derivation is what `faces_are_regular_polygons`
//! checks, so a mistyped coordinate fails a test instead of shipping a
//! plausible-looking lump.
//!
//! # Geometry at any panel shape
//!
//! Nothing here assumes 16:9. The figure is sized off the SHORTER panel side in
//! dots, so on the 1280x400 panel it is limited by the 400 and simply has a lot
//! of empty width either side, and the sample spacing is in DOTS, so the point
//! count falls with the panel instead of packing a fixed count into a quarter
//! of the area and reading as a filled disc. A braille dot is `cell_w/2` by
//! `cell_h/4`, which is square at the default 8x16 — keep that ratio if you
//! change the cell, or the solid comes out as an ellipsoid.
//!
//! # Damage model: full repaint (`Grid::flush`)
//!
//! Not sparse. The whole figure moves every frame and the transition moves
//! every point at once, so "what changed" is most of the figure most of the
//! time. `Grid::flush` derives it from a u32 compare per cell and structurally
//! cannot under-report — the failure mode `flush_sparse` has, where a cell
//! written but left out of a dirty list freezes on the panel forever, is not
//! reachable from here.
//!
//! # Knobs
//!
//! | var | default | what |
//! | --- | --- | --- |
//! | `POV_HOLD_SECS` | 10 | seconds a solid holds before the burst |
//! | `POV_BURST_MS` | 1200 | length of the burst itself |
//! | `POV_SPACING` | 6 | dots between surface samples; smaller is denser |
//! | `POV_SCALE` | 420 | figure radius, thousandths of the shorter panel side |
//! | `POV_BURST` | 450 | outward scatter, thousandths of the figure radius |
//! | `POV_Z_DIST` | 6000 | eye distance, thousandths of the figure radius |
//! | `POV_RATE_XY` / `_XZ` / `_YZ` | 7 / 23 / 13 | milli-revolutions per second |
//! | `POV_CELL_W` / `POV_CELL_H` | 8 / 16 | cell size in pixels |
//!
//! Both durations are in real time and converted with `fps`, so the hold is ten
//! seconds at 15fps and at 30fps.

use std::f32::consts::{PI, TAU};

use crate::font;
use crate::grid::{bake, dot_bit, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, next_rand};

/// Index 0 is unlit. 1..=16 runs from the far side of the figure to the near
/// one: deep indigo through cyan to a warm white. Hue and brightness both climb
/// so the depth survives for a viewer who cannot separate the hues — the same
/// argument `hypercube` makes for its ramp.
const PAL_RGB: [[u8; 3]; 17] = [
    [0x00, 0x00, 0x00],
    [0x1C, 0x1E, 0x5E],
    [0x20, 0x28, 0x74],
    [0x24, 0x34, 0x8C],
    [0x26, 0x42, 0xA4],
    [0x27, 0x52, 0xBA],
    [0x27, 0x64, 0xCE],
    [0x29, 0x78, 0xDE],
    [0x2E, 0x8C, 0xE9],
    [0x3A, 0xA0, 0xEF],
    [0x4E, 0xB4, 0xF1],
    [0x6C, 0xC6, 0xEC],
    [0x91, 0xD6, 0xE2],
    [0xB8, 0xE4, 0xD4],
    [0xDA, 0xEE, 0xC8],
    [0xF2, 0xF5, 0xD2],
    [0xFF, 0xFF, 0xF4],
];

const PAL: [u32; 17] = bake(&PAL_RGB);
const SHADES: usize = 16;

/// The golden ratio, `(1 + sqrt(5)) / 2`. The dodecahedron and the icosahedron
/// are the two solids whose coordinates are easy to get subtly wrong, so it is
/// named once and `phi_is_the_golden_ratio` pins it.
const PHI: f32 = std::f32::consts::GOLDEN_RATIO;

const SOLIDS: usize = 5;

/// The three rotation planes, as the coordinate pair each mixes.
const PLANES: [(usize, usize); 3] = [(0, 1), (0, 2), (1, 2)];
const RATE_KEYS: [&str; 3] = ["POV_RATE_XY", "POV_RATE_XZ", "POV_RATE_YZ"];
/// Milli-revolutions per second. Pairwise coprime and none divides another, so
/// the composed pose has no period short enough to notice — and none of them is
/// a whole number of turns per hold, so a solid is never re-presented in the
/// pose it was last left in.
const RATE_DEFAULT: [i64; 3] = [7, 23, 13];

/// `easeOutBack`. `C3` is the standard `C1 + 1`; the pair make `e(0) = 0`,
/// `e(1) = 1` and a ~5% overshoot around `u = 0.8`. That overshoot is the point:
/// without it the points arrive and stop, which reads as a crossfade.
const C1: f32 = 1.70158;
const C3: f32 = C1 + 1.0;

#[inline]
fn ease_back(u: f32) -> f32 {
    let m = u - 1.0;
    1.0 + C3 * m * m * m + C1 * m * m
}

/// The outward kick, as a fraction of `POV_BURST`. Zero at both ends — a bump
/// that is not would leave the figure permanently displaced — and deliberately
/// NOT symmetric: `sin(pi*u)` alone holds the cloud out until the last frame
/// and the new solid then appears in one step. The `(1 - u)` puts the peak at
/// about a third of the way through and gives the whole back half of the
/// transition to the points visibly finding their places. `1.71` is `1/max`, so
/// the knob still means the displacement it says.
#[inline]
fn bump(u: f32) -> f32 {
    (PI * u).sin() * (1.0 - u) * 1.71
}

type V3 = [f32; 3];

#[inline]
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[inline]
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[inline]
fn norm(a: V3) -> f32 {
    dot(a, a).sqrt()
}

fn unit(a: V3) -> V3 {
    let n = norm(a).max(1e-9);
    [a[0] / n, a[1] / n, a[2] / n]
}

/// The three cyclic rotations of `(0, p, q)` over every sign of `p` and `q` —
/// the shape both the icosahedron's and the dodecahedron's golden-ratio
/// families take. Written once so the two cannot disagree about what "cyclic"
/// means, which is exactly how a dodecahedron ends up not being the dual of its
/// own icosahedron.
fn cyclic(p: f32, q: f32, out: &mut Vec<V3>) {
    for (i, j) in [(1usize, 2usize), (2, 0), (0, 1)] {
        for sp in [1.0f32, -1.0] {
            for sq in [1.0f32, -1.0] {
                let mut v = [0.0f32; 3];
                v[i] = sp * p;
                v[j] = sq * q;
                out.push(v);
            }
        }
    }
}

/// The five vertex sets, normalised to circumradius 1 so every solid draws at
/// the same size and one `POV_SCALE` means the same thing for all of them.
fn solid_verts(which: usize) -> Vec<V3> {
    let mut v = Vec::new();
    match which {
        // Alternate corners of a cube.
        0 => v.extend_from_slice(&[
            [1.0, 1.0, 1.0],
            [1.0, -1.0, -1.0],
            [-1.0, 1.0, -1.0],
            [-1.0, -1.0, 1.0],
        ]),
        1 => {
            for i in 0..8u8 {
                v.push(std::array::from_fn(|b| {
                    if i & (1 << b) != 0 {
                        1.0
                    } else {
                        -1.0
                    }
                }));
            }
        }
        2 => {
            for a in 0..3usize {
                for s in [1.0f32, -1.0] {
                    let mut p = [0.0f32; 3];
                    p[a] = s;
                    v.push(p);
                }
            }
        }
        // (+-1, +-1, +-1) with the cyclic (0, +-1/phi, +-phi) family.
        3 => {
            for i in 0..8u8 {
                v.push(std::array::from_fn(|b| {
                    if i & (1 << b) != 0 {
                        1.0
                    } else {
                        -1.0
                    }
                }));
            }
            cyclic(1.0 / PHI, PHI, &mut v);
        }
        // The cyclic (0, +-1, +-phi) family, and nothing else.
        _ => cyclic(1.0, PHI, &mut v),
    }
    v.iter().map(|&p| unit(p)).collect()
}

/// Pairs at the minimum vertex separation. For a regular solid that IS the edge
/// set — every other pair is a diagonal and strictly longer.
fn edges_of(v: &[V3]) -> Vec<(usize, usize)> {
    let mut best = f32::MAX;
    for i in 0..v.len() {
        for j in i + 1..v.len() {
            best = best.min(norm(sub(v[i], v[j])));
        }
    }
    let mut out = Vec::new();
    for i in 0..v.len() {
        for j in i + 1..v.len() {
            if norm(sub(v[i], v[j])) < best * 1.001 {
                out.push((i, j));
            }
        }
    }
    out
}

/// Faces as cyclically ordered vertex index lists.
///
/// A plane through a vertex and two of its neighbours that leaves every vertex
/// on one side is a supporting plane, and for a convex polyhedron a supporting
/// plane through three non-collinear vertices is a face plane. So: enumerate
/// those candidates, drop the ones some vertex pokes through, dedupe by normal,
/// and read each face off as the vertices ON its plane. Derived rather than
/// typed because twenty pentagons of five indices each is a hundred numbers
/// nobody can review, and a single transposed pair in them draws a solid that
/// looks almost right.
fn faces_of(v: &[V3]) -> Vec<Vec<usize>> {
    let mut adj = vec![Vec::new(); v.len()];
    for (a, b) in edges_of(v) {
        adj[a].push(b);
        adj[b].push(a);
    }
    let mut normals: Vec<V3> = Vec::new();
    for c in 0..v.len() {
        for i in 0..adj[c].len() {
            for j in i + 1..adj[c].len() {
                let (a, b) = (adj[c][i], adj[c][j]);
                let mut n = unit(cross(sub(v[a], v[c]), sub(v[b], v[c])));
                if dot(n, v[c]) < 0.0 {
                    n = [-n[0], -n[1], -n[2]];
                }
                let d = dot(n, v[c]);
                if v.iter().any(|&u| dot(n, u) > d + 1e-3) {
                    continue;
                }
                if !normals.iter().any(|&m| dot(m, n) > 0.999) {
                    normals.push(n);
                }
            }
        }
    }
    normals
        .into_iter()
        .map(|n| {
            let d = v.iter().fold(f32::MIN, |m, &u| m.max(dot(n, u)));
            // An in-plane basis, so "sorted around the face" is an atan2 and
            // not a walk over the edge graph.
            let seed = if n[0].abs() < 0.9 {
                [1.0, 0.0, 0.0]
            } else {
                [0.0, 1.0, 0.0]
            };
            let ax = unit(cross(n, seed));
            let ay = cross(n, ax);
            let mut f: Vec<usize> = (0..v.len()).filter(|&i| dot(n, v[i]) > d - 1e-3).collect();
            f.sort_by(|&i, &j| {
                let ang = |k: usize| dot(v[k], ay).atan2(dot(v[k], ax));
                ang(i).total_cmp(&ang(j))
            });
            f
        })
        .collect()
}

/// A triangular lattice over one triangle, at whatever resolution puts its
/// samples about `spacing` dots apart. The longest side sets the resolution, so
/// a sliver triangle is sampled by its long edge and not by its short one.
fn lattice(tri: [V3; 3], dots: f32, spacing: f32, out: &mut Vec<V3>) {
    let side = (0..3)
        .map(|i| norm(sub(tri[i], tri[(i + 1) % 3])))
        .fold(0.0f32, f32::max);
    let n = ((side * dots / spacing).round() as usize).clamp(1, 64);
    let inv = 1.0 / n as f32;
    for i in 0..=n {
        for j in 0..=n - i {
            let k = n - i - j;
            let (a, b, c) = (i as f32 * inv, j as f32 * inv, k as f32 * inv);
            out.push(std::array::from_fn(|x| {
                tri[0][x] * a + tri[1][x] * b + tri[2][x] * c
            }));
        }
    }
}

/// How much to inflate a solid so all five read as the same size.
///
/// Circumradius-normalised is the honest presentation — five solids in one
/// sphere — and it looks wrong: the tetrahedron's faces sit at a THIRD of its
/// circumradius, so on screen it is half the size of the icosahedron and the
/// cycle reads as the figure shrinking and growing rather than as one object
/// changing shape. Each solid is scaled to unit MIDRADIUS, the mean of its in-
/// and circumradius, which brings the five to within a few percent of each
/// other. The inradius is just the distance to a face centroid — the closest
/// point of a face plane to the centre.
fn midradius_gain(which: usize) -> f32 {
    let v = solid_verts(which);
    let f = &faces_of(&v)[0];
    let k = 1.0 / f.len() as f32;
    let c: V3 = std::array::from_fn(|x| f.iter().map(|&i| v[i][x]).sum::<f32>() * k);
    2.0 / (1.0 + norm(c))
}

/// Every sample on one solid's surface. Quads and pentagons fan from the face
/// CENTROID rather than from a vertex: a fan from a vertex makes triangles of
/// wildly different shapes and the lattice density visibly changes across the
/// face. A triangular face is used as it is — fanning it would only make three
/// skinnier triangles.
fn sample_solid(which: usize, dots: f32, spacing: f32, out: &mut Vec<V3>) {
    let g = midradius_gain(which);
    let v: Vec<V3> = solid_verts(which)
        .iter()
        .map(|p| std::array::from_fn(|x| p[x] * g))
        .collect();
    for f in faces_of(&v) {
        if f.len() == 3 {
            lattice([v[f[0]], v[f[1]], v[f[2]]], dots, spacing, out);
            continue;
        }
        let k = 1.0 / f.len() as f32;
        let c: V3 = std::array::from_fn(|x| f.iter().map(|&i| v[i][x]).sum::<f32>() * k);
        for i in 0..f.len() {
            lattice([c, v[f[i]], v[f[(i + 1) % f.len()]]], dots, spacing, out);
        }
    }
}

pub struct Pov {
    grid: Grid,
    cols: usize,
    /// Dot grid: `cols * 2` by `rows * 4`.
    dw: usize,
    dh: usize,
    /// Every solid's samples, concatenated. `span[i]` is `(offset, len)`.
    pts: Vec<V3>,
    span: [(usize, usize); SOLIDS],
    /// One scatter vector per POOL slot, already scaled by `POV_BURST`. Fixed
    /// at construction: the figure is in a different pose at every burst, so a
    /// fresh set per transition would buy nothing a viewer could see.
    dir: Vec<V3>,
    pool: usize,
    cur: usize,
    nxt: usize,
    /// Frames into the current hold-then-burst cycle.
    t: u32,
    hold: u32,
    burst: u32,
    /// As `hypercube`: an integer turn fraction, advanced by wrapping addition.
    /// A float radian accumulator quantises after a few million frames and this
    /// pod runs for weeks.
    phase: [u32; 3],
    step: [u32; 3],
    z_dist: f32,
    scale: f32,
    /// Per cell: the braille dots lit this frame, and the NEAREST shade among
    /// them. Nearest rather than latest, so a near point is never dimmed by a
    /// far one landing in the same cell afterwards.
    mask: Vec<u8>,
    shade: Vec<u8>,
}

impl Pov {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        let cell_w = env_num(&["POV_CELL_W"], 8, 4, 64) as usize;
        let cell_h = env_num(&["POV_CELL_H"], 16, 8, 128) as usize;
        let grid = Grid::new(panel, cell_w, cell_h);
        let (cols, rows) = (grid.cols(), grid.rows());
        let (dw, dh) = (cols * 2, rows * 4);

        // The SHORTER side sizes the figure. On 1280x400 that is the height, so
        // the solid stays whole with empty width either side rather than being
        // sized off the width and running off the top and bottom.
        let short = dw.min(dh) as f32;
        // `POV_SCALE` is the radius of the BIGGEST solid, so the knob means the
        // same thing whatever `midradius_gain` does to each of them.
        let gmax = (0..SOLIDS).map(midradius_gain).fold(0.0f32, f32::max);
        let scale = short * env_num(&["POV_SCALE"], 420, 50, 600) as f32 / 1000.0 / gmax;
        let spacing = env_num(&["POV_SPACING"], 6, 2, 24) as f32;

        let mut pts = Vec::new();
        let mut span = [(0usize, 0usize); SOLIDS];
        for (i, s) in span.iter_mut().enumerate() {
            let off = pts.len();
            sample_solid(i, scale, spacing, &mut pts);
            *s = (off, pts.len() - off);
        }
        // The pool is the largest solid's count, so every solid's samples are
        // reachable; smaller solids repeat theirs via `p % len`. See the module
        // doc on points with no destination.
        let pool = span.iter().map(|&(_, n)| n).max().unwrap_or(1).max(1);

        let amount = env_num(&["POV_BURST"], 450, 0, 2000) as f32 / 1000.0;
        let mut rng = 0x50F1_C001u32;
        let dir = (0..pool)
            .map(|_| {
                // z uniform on [-1,1] plus a uniform azimuth is an exactly
                // uniform direction on the sphere, with no rejection loop.
                let z = next_rand(&mut rng) as f32 / 1_073_741_824.0 - 1.0;
                let a = next_rand(&mut rng) as f32 / 2_147_483_648.0 * TAU;
                let m = amount * (0.5 + next_rand(&mut rng) as f32 / 2_147_483_648.0);
                let r = (1.0 - z * z).max(0.0).sqrt();
                [r * a.cos() * m, r * a.sin() * m, z * m]
            })
            .collect();

        let mut step = [0u32; 3];
        for ((s, key), def) in step.iter_mut().zip(RATE_KEYS).zip(RATE_DEFAULT) {
            let r = env_num(&[key], def, -2000, 2000);
            // milli-revs/sec -> u32 turn fractions per frame, exactly.
            *s = ((r as i128) * (1i128 << 32) / (1000 * fps.max(1) as i128)) as i64 as u32;
        }

        let fps = fps.max(1);
        Self {
            grid,
            cols,
            dw,
            dh,
            pts,
            span,
            dir,
            pool,
            cur: 0,
            nxt: 1,
            t: 0,
            // Both are real time converted by fps, so ten seconds is ten
            // seconds whether the panel runs at 15 or at 30.
            hold: env_num(&["POV_HOLD_SECS"], 10, 1, 600) as u32 * fps,
            burst: (env_num(&["POV_BURST_MS"], 1200, 100, 5000) as u32 * fps / 1000).max(1),
            phase: [0; 3],
            step,
            z_dist: env_num(&["POV_Z_DIST"], 6000, 2000, 40_000) as f32 / 1000.0,
            scale,
            mask: vec![0; cols * rows],
            shade: vec![0; cols * rows],
        }
    }

    /// Where pool slot `p` is in model space this frame: on the current solid
    /// during the hold, and in flight during the burst.
    #[inline]
    fn point(&self, p: usize, u: Option<f32>) -> V3 {
        let (off, len) = self.span[self.cur];
        let from = self.pts[off + p % len];
        let Some(u) = u else {
            return from;
        };
        let (noff, nlen) = self.span[self.nxt];
        let to = self.pts[noff + p % nlen];
        let e = ease_back(u);
        let b = bump(u);
        let d = self.dir[p];
        // Away from where the point started, so the cloud expands instead of
        // half of it ploughing through the middle.
        let s = if dot(d, from) < 0.0 { -b } else { b };
        std::array::from_fn(|x| from[x] + (to[x] - from[x]) * e + d[x] * s)
    }

    /// Light one braille dot, keeping the nearest shade the cell has seen.
    #[inline]
    fn plot(&mut self, dx: usize, dy: usize, shade: u8) {
        let i = (dy >> 2) * self.cols + (dx >> 1);
        self.mask[i] |= dot_bit(dx & 1, dy & 3);
        if shade > self.shade[i] {
            self.shade[i] = shade;
        }
    }

    fn draw(&mut self) {
        let mut sc = [(0.0f32, 0.0f32); 3];
        for (t, &p) in sc.iter_mut().zip(self.phase.iter()) {
            *t = (p as f32 * (TAU / 4_294_967_296.0)).sin_cos();
        }
        let (cx, cy) = (self.dw as f32 * 0.5, self.dh as f32 * 0.5);
        let u = (self.t >= self.hold).then(|| (self.t - self.hold + 1) as f32 / self.burst as f32);

        for p in 0..self.pool {
            let mut v = self.point(p, u);
            for ((a, b), (sin, cos)) in PLANES.into_iter().zip(sc) {
                let (q, r) = (v[a], v[b]);
                v[a] = q * cos - r * sin;
                v[b] = q * sin + r * cos;
            }
            // Clamped for the same reason `hypercube` clamps: `z_dist` is a
            // knob and a user can put the eye inside the figure, where an
            // unclamped divide throws a point thousands of dots across.
            let k = self.z_dist / (self.z_dist - v[2]).max(self.z_dist * 0.2);
            let x = cx + v[0] * k * self.scale;
            let y = cy + v[1] * k * self.scale;
            // Off the dot grid is DROPPED, not clamped: clamping piles the
            // scatter into a bright line along whichever edge it left by.
            if !(x >= 0.0 && y >= 0.0) {
                continue;
            }
            let (dx, dy) = (x as usize, y as usize);
            if dx >= self.dw || dy >= self.dh {
                continue;
            }
            // The WHOLE ramp across the figure's own depth, not a slice of it:
            // the model is a unit ball, so `z` is already -1..1 and anything
            // narrower than that leaves the near face and the far face a couple
            // of palette steps apart and the figure reads flat.
            let s = ((v[2] + 1.0) * 0.5).clamp(0.0, 1.0) * (SHADES - 1) as f32;
            self.plot(dx, dy, s as u8);
        }
    }
}

impl Saver for Pov {
    fn render(&mut self, s: &mut Surface<'_>) {
        for (p, &d) in self.phase.iter_mut().zip(self.step.iter()) {
            *p = p.wrapping_add(d);
        }
        // The burst ends by ADOPTING the solid it was flying to; nothing here
        // allocates, which is what `render_never_allocates` crosses a change to
        // prove.
        self.t += 1;
        if self.t >= self.hold + self.burst {
            self.t = 0;
            self.cur = self.nxt;
            self.nxt = (self.nxt + 1) % SOLIDS;
        }
        self.mask.fill(0);
        self.shade.fill(0);
        self.draw();

        let (grid, mask, shade, cols) =
            (&mut self.grid, &self.mask[..], &self.shade[..], self.cols);
        grid.fill(|cx, cy| {
            let i = cy * cols + cx;
            if mask[i] == 0 {
                return Cell::CLEAR;
            }
            Cell::new(font::BRAILLE[mask[i] as usize], shade[i] as u16 + 1)
        });
        grid.flush(s, &PAL);
    }

    fn name(&self) -> &'static str {
        "pov"
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
    use std::collections::HashSet;

    /// Vertex, edge and face counts in cycle order. Nothing BUILDS from these —
    /// the builders derive their own — which is the whole reason they are worth
    /// writing down.
    const VERTS: [usize; SOLIDS] = [4, 8, 6, 20, 12];
    const EDGES: [usize; SOLIDS] = [6, 12, 12, 30, 30];
    const FACES: [usize; SOLIDS] = [4, 6, 8, 12, 20];

    fn panel() -> Panel {
        Panel::new(1920, 1080, 1920)
    }

    /// Env vars are process-global and the test binary is threaded, so nothing
    /// here sets one.
    fn pov() -> Pov {
        Pov::new(&panel(), 30)
    }

    /// The one constant a mistyped digit would corrupt both golden-ratio solids
    /// with, silently.
    #[test]
    fn phi_is_the_golden_ratio() {
        assert!((PHI * PHI - PHI - 1.0).abs() < 1e-5, "phi^2 != phi + 1");
        assert!((PHI - (1.0 + 5.0f32.sqrt()) / 2.0).abs() < 1e-6);
    }

    /// The check that a wrong coordinate cannot survive. For each solid: the
    /// vertex count, every vertex the same distance from the centre, and
    /// EXACTLY ONE edge length — the minimum separation, occurring exactly as
    /// many times as the solid has edges and with a clear gap to the next
    /// distance up. A dodecahedron with one golden-ratio coordinate wrong has
    /// two edge lengths and fails here.
    #[test]
    fn every_solid_is_actually_that_platonic_solid() {
        for which in 0..SOLIDS {
            let v = solid_verts(which);
            assert_eq!(v.len(), VERTS[which], "solid {which}: vertex count");
            for p in &v {
                assert!(
                    (norm(*p) - 1.0).abs() < 1e-4,
                    "solid {which}: vertex {p:?} is not on the circumsphere"
                );
            }
            let mut d: Vec<f32> = Vec::new();
            for i in 0..v.len() {
                for j in i + 1..v.len() {
                    d.push(norm(sub(v[i], v[j])));
                }
            }
            d.sort_by(f32::total_cmp);
            let e = d[0];
            let n = d.iter().filter(|&&x| x < e * 1.001).count();
            assert_eq!(n, EDGES[which], "solid {which}: edge count at length {e}");
            assert_eq!(edges_of(&v).len(), EDGES[which], "solid {which}: edges_of");
            // One edge length, not two that nearly match: the next distance up
            // is a diagonal and has to be clearly longer.
            // The tetrahedron has no diagonals at all: every pair IS an edge.
            if let Some(&next) = d.get(n) {
                assert!(
                    next > e * 1.05,
                    "solid {which}: a second near-edge length {next} vs {e}"
                );
            }
        }
    }

    /// Faces are derived, so the derivation is what needs pinning: the right
    /// number of them, every one a regular polygon of the right order (all
    /// sides equal, all vertices the same distance from the face centroid), and
    /// every vertex on the same number of faces.
    #[test]
    fn faces_are_regular_polygons() {
        let order = [3usize, 4, 3, 5, 3];
        for which in 0..SOLIDS {
            let v = solid_verts(which);
            let f = faces_of(&v);
            assert_eq!(f.len(), FACES[which], "solid {which}: face count");
            let mut seen = vec![0usize; v.len()];
            for face in &f {
                assert_eq!(face.len(), order[which], "solid {which}: face {face:?}");
                let k = 1.0 / face.len() as f32;
                let c: V3 = std::array::from_fn(|x| face.iter().map(|&i| v[i][x]).sum::<f32>() * k);
                let mut side = Vec::new();
                for (n, &i) in face.iter().enumerate() {
                    seen[i] += 1;
                    let j = face[(n + 1) % face.len()];
                    side.push(norm(sub(v[i], v[j])));
                    assert!(
                        (norm(sub(v[i], c)) - norm(sub(v[face[0]], c))).abs() < 1e-3,
                        "solid {which}: face {face:?} is not regular"
                    );
                }
                // Cyclic ordering: consecutive pairs are all EDGES. Sorted the
                // wrong way round a pentagon these come out as two lengths.
                let lo = side.iter().copied().fold(f32::MAX, f32::min);
                let hi = side.iter().copied().fold(0.0f32, f32::max);
                assert!(
                    hi - lo < 1e-3,
                    "solid {which}: face {face:?} sides {lo}..{hi}: not a cycle"
                );
            }
            // Every vertex on the same number of faces — a face silently
            // derived twice, or missed, shows up here and nowhere else.
            assert!(
                seen.iter().all(|&n| n == seen[0]) && seen[0] >= 3,
                "solid {which}: vertices on {seen:?} faces"
            );
        }
    }

    /// The samples have to be ON the surface, not merely near the solid: every
    /// one inside the circumsphere and outside the insphere, and every face
    /// carrying some. The failure this catches is a lattice that interpolates
    /// through the interior — which draws a fuzzy ball for every solid, and
    /// every other test still passes.
    #[test]
    fn samples_sit_on_the_surface_of_every_face() {
        for which in 0..SOLIDS {
            let v = solid_verts(which);
            let faces = faces_of(&v);
            let g = midradius_gain(which);
            let mut pts = Vec::new();
            sample_solid(which, 100.0, 8.0, &mut pts);
            assert!(pts.len() > 200, "solid {which}: only {} samples", pts.len());

            // Inradius: the face planes' common distance from the centre.
            let face_n = |f: &[usize]| {
                unit(f.iter().fold([0.0; 3], |a: V3, &i| {
                    std::array::from_fn(|x| a[x] + v[i][x])
                }))
            };
            // Both radii in the SAMPLED solid's units, which `midradius_gain`
            // has scaled up.
            let inr = g / faces[0].len() as f32
                * dot(
                    faces[0].iter().fold([0.0; 3], |a: V3, &i| {
                        std::array::from_fn(|x| a[x] + v[i][x])
                    }),
                    face_n(&faces[0]),
                );

            let mut per_face = vec![0usize; faces.len()];
            for p in &pts {
                let r = norm(*p);
                assert!(
                    r <= g + 1e-3 && r >= inr - 1e-3,
                    "solid {which}: sample at radius {r}, surface is {inr}..{g}"
                );
                // On SOME face plane, which is what "on the surface" means.
                let mut on = usize::MAX;
                for (i, f) in faces.iter().enumerate() {
                    let n = face_n(f);
                    if (dot(n, *p) - inr).abs() < 1e-3 {
                        on = i;
                        break;
                    }
                }
                assert_ne!(
                    on,
                    usize::MAX,
                    "solid {which}: sample {p:?} is off every face"
                );
                per_face[on] += 1;
            }
            assert!(
                per_face.iter().all(|&n| n > 10),
                "solid {which}: a face got almost no samples: {per_face:?}"
            );
        }
    }

    /// The five solids have to read as ONE object changing shape, which means
    /// they have to be about the same size on screen. Circumradius alone does
    /// not give that — the tetrahedron's faces sit at a third of its
    /// circumradius, so with no `midradius_gain` it carries far less of the
    /// frame than the icosahedron does. Stated on the mean distance of the
    /// SAMPLES from the centre, which is what a viewer reads as size.
    #[test]
    fn the_five_solids_are_the_same_apparent_size() {
        let r: Vec<f32> = (0..SOLIDS)
            .map(|w| {
                let mut pts = Vec::new();
                sample_solid(w, 100.0, 8.0, &mut pts);
                pts.iter().map(|p| norm(*p)).sum::<f32>() / pts.len() as f32
            })
            .collect();
        let lo = r.iter().copied().fold(f32::MAX, f32::min);
        let hi = r.iter().copied().fold(0.0f32, f32::max);
        assert!(
            hi < lo * 1.16,
            "apparent sizes span {lo}..{hi}: the cycle will read as the figure \
             growing and shrinking, not as one object changing shape ({r:?})"
        );
    }

    /// Frame 0 must cover the panel, and cover it with the figure rather than
    /// with a reported black frame.
    #[test]
    fn frame_zero_paints_the_whole_panel() {
        let p = panel();
        let mut h = pov();
        let mut buf = vec![0u32; p.buf_len()];
        let d = saver::frame(&mut h, &mut buf, &p);
        assert_eq!(d.rows(), p.h, "frame 0 must paint every scanline");
        assert!(
            buf.iter().filter(|&&px| px != 0).count() > 20_000,
            "frame 0 drew nothing"
        );
    }

    /// The hold is a real-time promise, so it has to survive a change of frame
    /// rate: the SAME wall-clock schedule at 15fps and at 30fps.
    #[test]
    fn the_hold_is_ten_seconds_at_any_frame_rate() {
        for fps in [15u32, 30, 60] {
            let h = Pov::new(&panel(), fps);
            assert_eq!(h.hold, 10 * fps, "{fps}fps: hold");
            assert_eq!(h.hold / fps, 10, "{fps}fps: hold is not 10 seconds");
            assert_eq!(h.burst, 1200 * fps / 1000, "{fps}fps: burst");
        }
    }

    /// The cycle: five solids, each held, each arrived at by a burst, and back
    /// to the first. Run over two full laps so a cycle that sticks or skips is
    /// visible.
    #[test]
    fn all_five_solids_come_round_in_order() {
        let p = Panel::new(480, 270, 480);
        let mut h = Pov::new(&p, 30);
        let mut buf = vec![0u32; p.buf_len()];
        let mut order = Vec::new();
        let mut bursts = 0;
        for _ in 0..(10 * 30 + 27) * 11 {
            saver::frame(&mut h, &mut buf, &p);
            if order.last() != Some(&h.cur) {
                order.push(h.cur);
            }
            bursts += u32::from(h.t >= h.hold);
        }
        assert_eq!(
            &order[..11],
            &[0usize, 1, 2, 3, 4, 0, 1, 2, 3, 4, 0],
            "the cycle is not five solids in order: {order:?}"
        );
        // Every change was actually flown, not cut to.
        assert!(bursts >= 10 * h.burst, "only {bursts} burst frames");
    }

    /// The burst's signature, and the thing a lerp would quietly replace. The
    /// cloud must SWELL past both of its endpoints part way through — measured
    /// as the mean model-space radius — and it must come back: at the last
    /// frame of the burst every point is exactly on its destination.
    #[test]
    fn the_burst_scatters_outward_and_lands_exactly() {
        let h = pov();
        let mean =
            |u: Option<f32>| (0..h.pool).map(|p| norm(h.point(p, u))).sum::<f32>() / h.pool as f32;
        let rest = mean(None);
        let peak = (1..20)
            .map(|i| mean(Some(i as f32 / 20.0)))
            .fold(0.0, f32::max);
        assert!(
            peak > rest * 1.15,
            "the cloud only reached {peak} from {rest}: it never swells"
        );
        // The direct statement of "not a lerp": how far the points are from the
        // straight line between their endpoints. A lerp is zero here at every
        // u, whatever the two solids' radii happen to do.
        let (aoff, alen) = h.span[h.cur];
        let (boff, blen) = h.span[h.nxt];
        let off_line = |u: f32| {
            (0..h.pool)
                .map(|p| {
                    let from = h.pts[aoff + p % alen];
                    let to = h.pts[boff + p % blen];
                    let line: V3 = std::array::from_fn(|x| from[x] + (to[x] - from[x]) * u);
                    norm(sub(h.point(p, Some(u)), line))
                })
                .sum::<f32>()
                / h.pool as f32
        };
        assert!(
            off_line(0.35) > 0.25,
            "mid-burst the points are {} off the straight line: this is a lerp",
            off_line(0.35)
        );
        assert!(off_line(1.0) < 1e-3, "the burst does not rejoin the line");
        // Lands. Not "close to" — the bump is zero at u=1 and the ease is one,
        // so every point is ON its destination or the transition leaves a
        // permanently displaced figure.
        let (off, len) = h.span[h.nxt];
        for p in 0..h.pool {
            let got = h.point(p, Some(1.0));
            let want = h.pts[off + p % len];
            assert!(
                norm(sub(got, want)) < 1e-4,
                "point {p} landed at {got:?}, not {want:?}"
            );
        }
        // And overshoots on the way in: past the destination, then back.
        let d = |u: f32| {
            (0..h.pool)
                .map(|p| norm(sub(h.point(p, Some(u)), h.pts[off + p % len])))
                .sum::<f32>()
        };
        assert!(d(0.93) > d(0.99), "the landing does not settle");
        assert!(ease_back(0.8) > 1.0, "easeOutBack does not overshoot");
        assert_eq!(bump(0.0), 0.0);
        assert_eq!(bump(1.0), 0.0);
        assert!(
            bump(0.35) > bump(0.75),
            "the kick does not fade before the landing"
        );
    }

    /// A point without a unique destination is the design's one compromise, so
    /// pin it: the pool is the largest solid, and a slot beyond a smaller
    /// solid's count doubles up on an existing sample rather than vanishing.
    #[test]
    fn extra_points_double_up_instead_of_disappearing() {
        let mut h = pov();
        let biggest = h.span.iter().map(|&(_, n)| n).max().unwrap();
        assert_eq!(h.pool, biggest);
        assert!(
            h.span.iter().any(|&(_, n)| n < biggest),
            "the solids all have the same count: this test proves nothing"
        );
        assert_eq!(h.dir.len(), h.pool, "a pool slot has no scatter vector");
        // Every slot resolves to a real sample of whichever solid is drawn.
        for which in 0..SOLIDS {
            h.cur = which;
            let (off, len) = h.span[which];
            for p in 0..h.pool {
                assert_eq!(
                    h.point(p, None),
                    h.pts[off + p % len],
                    "solid {which} slot {p}"
                );
            }
        }
    }

    /// The figure must stay on the panel on BOTH shapes, and the 1280x400 one
    /// is the hard case: very wide, very short, and a figure sized off the
    /// width would run off the top and the bottom. Checked on the drawn cells,
    /// not on the maths — the top and bottom cell rows must stay clear through
    /// a whole hold.
    #[test]
    fn the_figure_never_clips_on_either_panel() {
        for (w, h) in [(1920usize, 1080usize), (1280, 400)] {
            let p = Panel::new(w, h, w);
            let mut s = Pov::new(&p, 30);
            let mut buf = vec![0u32; p.buf_len()];
            let (cols, rows) = (s.grid.cols(), s.grid.rows());
            let (mut lo, mut hi) = (usize::MAX, 0usize);
            // A whole hold, so every pose is covered. Not the burst: the
            // scatter is meant to leave the panel.
            for n in 0..s.hold {
                saver::frame(&mut s, &mut buf, &p);
                let cells = s.grid.cells();
                for cx in 0..cols {
                    for &cy in &[0usize, rows - 1] {
                        assert_eq!(
                            cells[cy * cols + cx],
                            Cell::CLEAR,
                            "{w}x{h} frame {n}: the figure reaches row {cy}"
                        );
                    }
                }
                for &cx in &[0usize, cols - 1] {
                    for cy in 0..rows {
                        assert_eq!(
                            cells[cy * cols + cx],
                            Cell::CLEAR,
                            "{w}x{h} frame {n}: the figure reaches column {cx}"
                        );
                    }
                }
                for (i, c) in cells.iter().enumerate() {
                    if *c != Cell::CLEAR {
                        lo = lo.min(i / cols);
                        hi = hi.max(i / cols);
                    }
                }
            }
            // And it is actually big: a figure that never clips because it is
            // four cells across would pass everything above. Measured as the
            // drawn EXTENT rather than as a fill fraction — 1280x400 is mostly
            // empty width by design, so a fill fraction says nothing there.
            assert!(
                hi >= lo && hi - lo + 1 > rows / 2,
                "{w}x{h}: the figure only spans rows {lo}..={hi} of {rows}"
            );
        }
    }

    /// Depth cueing is what makes the rotation read as 3D rather than as a flat
    /// scatter, so a frame must use a real SPREAD of the ramp — not one shade,
    /// and not only the two ends.
    #[test]
    fn depth_shades_the_figure_across_the_ramp() {
        let p = panel();
        let mut h = pov();
        let mut buf = vec![0u32; p.buf_len()];
        for _ in 0..90 {
            saver::frame(&mut h, &mut buf, &p);
        }
        let mut hist = [0usize; PAL.len()];
        for c in h.grid.cells() {
            hist[c.colour()] += 1;
        }
        let used = hist[1..].iter().filter(|&&n| n > 20).count();
        assert!(used >= 10, "only {used} shades in use: the figure is flat");
        let lit: usize = hist[1..].iter().sum();
        let top = *hist[1..].iter().max().unwrap();
        assert!(top * 3 < lit * 2, "one shade owns {top} of {lit} lit cells");
    }

    /// CLAUDE.md makes the frame loop the top constraint in this repo, and the
    /// event this is really about is the SOLID CHANGE: swapping figures is
    /// where a `Vec<[f32;3]>` of the new solid's points would be built. So the
    /// run is long enough to cross one — a full hold plus a full burst plus
    /// slack — and the allocator count must not move by one across it.
    ///
    /// Counted rather than inferred from capacities: a scratch Vec allocated
    /// and dropped inside `render` leaves every capacity exactly where it was.
    #[test]
    fn render_never_allocates() {
        let p = Panel::new(480, 270, 480);
        let mut h = Pov::new(&p, 30);
        let mut buf = vec![0u32; p.buf_len()];
        saver::frame(&mut h, &mut buf, &p);

        let frames = h.hold + h.burst + 60;
        let before = crate::testalloc::count();
        let mut changes = 0;
        let mut last = h.cur;
        let mut burst_frames = 0;
        for _ in 0..frames {
            saver::frame(&mut h, &mut buf, &p);
            changes += u32::from(h.cur != last);
            last = h.cur;
            burst_frames += u32::from(h.t >= h.hold);
        }
        let n = crate::testalloc::count() - before;
        assert_eq!(n, 0, "render allocated {n} times over {frames} frames");
        // The crossing is the whole point of the test; without it this is a
        // steady-state run and proves nothing about the change.
        assert_eq!(changes, 1, "the run did not cross a solid change");
        assert_eq!(burst_frames, h.burst, "the run did not cover a whole burst");
    }

    /// The "screen went blank" bug class, from the other side. `Grid::flush`
    /// cannot under-report by construction, so what is left to check is that
    /// this saver genuinely uses it: the drawn cells must be exactly the cells
    /// the dot masks lit, with no trail from the frame before.
    #[test]
    fn the_frame_is_exactly_this_frame_with_no_trail() {
        let p = Panel::new(480, 270, 480);
        let mut h = Pov::new(&p, 30);
        let mut buf = vec![0u32; p.buf_len()];
        let mut seen: HashSet<usize> = HashSet::new();
        let mut lit = 0usize;
        for n in 0..400 {
            saver::frame(&mut h, &mut buf, &p);
            lit = 0;
            for (i, c) in h.grid.cells().iter().enumerate() {
                assert_eq!(
                    *c != Cell::CLEAR,
                    h.mask[i] != 0,
                    "frame {n}: cell {i} lit={} mask={}",
                    *c != Cell::CLEAR,
                    h.mask[i]
                );
                if *c != Cell::CLEAR {
                    seen.insert(i);
                    lit += 1;
                }
            }
        }
        assert!(seen.len() > 200, "only {} cells ever lit", seen.len());
        // The trail check, and the one the assert above CANNOT make: the cell
        // and the mask are two views of the same array, so they agree even if
        // the mask is never cleared. What a stale mask cannot do is give a cell
        // back — lit cells only ever accumulate — so the frame the figure has
        // moved on to has to be much smaller than everything it has ever lit.
        assert!(
            lit * 2 < seen.len(),
            "the last frame lights {lit} of the {} cells ever lit: dots are accumulating",
            seen.len()
        );
    }

    /// Weeks of uptime. The phase is integer, so the only way it can drift is
    /// if it were not — check the closed form at a frame count no test can run,
    /// then confirm the figure is still being drawn there.
    #[test]
    fn rotation_is_exact_after_a_billion_frames() {
        let p = Panel::new(480, 270, 480);
        let mut h = Pov::new(&p, 30);
        let mut buf = vec![0u32; p.buf_len()];
        const N: u32 = 1_000_000_000;
        let expect: Vec<u32> = h.step.iter().map(|&s| s.wrapping_mul(N)).collect();
        saver::frame(&mut h, &mut buf, &p);
        for (ph, &s) in h.phase.iter_mut().zip(h.step.iter()) {
            *ph = s.wrapping_mul(N - 5);
        }
        for _ in 0..5 {
            saver::frame(&mut h, &mut buf, &p);
        }
        assert_eq!(h.phase.to_vec(), expect, "the phase drifted");
        assert!(
            h.grid.cells().iter().any(|&c| c != Cell::CLEAR),
            "nothing is drawn after a billion frames"
        );
    }
}
