//! Escape-time fractals: four families in rotation, each zooming continuously
//! into a point known to sit on its boundary.
//!
//! # Why this is affordable at all
//!
//! Escape-time iteration is per-SAMPLE, and a sample per pixel at 1920x1080 is
//! 2M orbits a frame — an order of magnitude past the whole pod budget. The
//! only lever that matters is sample count, so the sample is the CELL, not the
//! pixel: a 20px cell is 96x54 = 5,184 orbits, 400x fewer. Every cell is
//! `SOLID`, so the fractal reads as chunky pixel art, which is the look the
//! atlas can actually draw.
//!
//! What is left is still expensive. Measured against the other full-repaint
//! savers on one machine at 1920x1080, a frame is 2.3x matrix's and 1.6x
//! ascii fire's, which puts it near 240 milli-cores on the Pi against their
//! 113 and 144 — inside the 500m pod limit with room, but not cheap.
//! `FRACTAL_CELL` is the lever that matters: orbits scale with its square, so
//! 28 roughly halves the iteration cost and 14 roughly doubles it. The floor
//! under all of it is the full-panel blit, which no cell size changes: under a
//! zoom every cell's colour moves every frame, so there is no sparse version
//! of this saver.
//!
//! Iteration depth rises with zoom depth (detail near the boundary needs it)
//! but is capped, because an uncapped budget is an unbounded frame: at the cap
//! the deepest zooms simply flatten, which looks like a softer boundary rather
//! than a stall.
//!
//! # Why a cut, not a crossfade
//!
//! Crossfading two families means computing both, i.e. doubling the only
//! expensive thing here. Instead the palette carries three brightness tiers of
//! the same 30 colours, and a cycle dips through them into near-darkness on
//! either side of the cut. The cut happens at the dim tier, so nothing pops.
//!
//! # Why f64
//!
//! f32 runs out of mantissa around a 10^4 zoom and the image dissolves into
//! flat blocks. A cycle here reaches ~600x and the next one would be worse;
//! aarch64 does f64 in hardware, so the only cost is cache.

use crate::env_num;
use crate::font;
use crate::grid::{bake, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};

/// Colours in one loop of the ramp. The ramp is PERIODIC — entry 29 leads back
/// into entry 0 — because iteration count is reduced modulo this, and a seam
/// would draw a hard false contour across every band boundary.
const BANDS: usize = 30;
/// Brightness tiers of that ramp; tier 0 is full, the rest are the fade.
const TIERS: usize = 3;
const PAL_LEN: usize = 1 + BANDS * TIERS;

/// The Ultra Fractal "default" gradient — blue, white, amber, near-black, back
/// to blue — sampled at 30 stops. Its dark stop is lifted off pure black so it
/// cannot be mistaken for the interior.
#[rustfmt::skip]
const BASE_RGB: [[u8; 3]; BANDS] = [
    [0x00, 0x07, 0x64], [0x07, 0x1C, 0x79], [0x0D, 0x31, 0x8F], [0x14, 0x46, 0xA4], [0x1B, 0x5A, 0xBA],
    [0x25, 0x6F, 0xCC], [0x40, 0x82, 0xD3], [0x5A, 0x95, 0xDA], [0x74, 0xA8, 0xE0], [0x8E, 0xBB, 0xE7],
    [0xA9, 0xCE, 0xEE], [0xC3, 0xE1, 0xF4], [0xDD, 0xF4, 0xFB], [0xEE, 0xFA, 0xF0], [0xF1, 0xED, 0xCA],
    [0xF3, 0xE0, 0xA3], [0xF6, 0xD4, 0x7D], [0xF9, 0xC7, 0x57], [0xFC, 0xBA, 0x31], [0xFE, 0xAE, 0x0B],
    [0xE4, 0x97, 0x03], [0xBE, 0x7E, 0x06], [0x98, 0x64, 0x0A], [0x73, 0x4B, 0x0E], [0x4D, 0x31, 0x12],
    [0x27, 0x18, 0x15], [0x0B, 0x05, 0x1D], [0x08, 0x06, 0x2F], [0x06, 0x06, 0x40], [0x03, 0x07, 0x52],
];

/// Per-tier brightness, as a numerator over 255.
const TIER_NUM: [u32; TIERS] = [255, 96, 30];

/// Index 0 is the interior and stays black in every tier, so an interior cell
/// never flickers during a fade.
const fn tiered(base: &[[u8; 3]; BANDS]) -> [[u8; 3]; PAL_LEN] {
    let mut out = [[0u8; 3]; PAL_LEN];
    let mut t = 0;
    while t < TIERS {
        let mut i = 0;
        while i < BANDS {
            let mut k = 0;
            while k < 3 {
                out[1 + t * BANDS + i][k] = ((base[i][k] as u32 * TIER_NUM[t]) / 255) as u8;
                k += 1;
            }
            i += 1;
        }
        t += 1;
    }
    out
}

const PAL: [u32; PAL_LEN] = bake(&tiered(&BASE_RGB));

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Mandel,
    Julia,
    Ship,
    Newton,
}

/// One fractal family: what to draw, where it is worth looking, and how fast
/// that view closes in.
struct Family {
    kind: Kind,
    /// Zoom target. Every one of these sits ON the boundary — a target inside
    /// the set zooms to a black screen, one outside zooms to flat colour, and
    /// both look like the renderer broke.
    cx: f64,
    cy: f64,
    /// Complex units across the panel at the start of a cycle.
    span: f64,
    /// Multiplier on the global zoom rate. Julia crawls: its detail is in the
    /// drifting `c`, and a fast zoom on top of that reads as two effects
    /// fighting.
    rate: f64,
}

const FAMILIES: [Family; 4] = [
    Family {
        kind: Kind::Mandel,
        // Seahorse valley, the canonical deep-zoom coordinate.
        cx: -0.743_643_887_037_151,
        cy: 0.131_825_904_205_330,
        span: 3.2,
        rate: 1.0,
    },
    Family {
        // Centre and span are placeholders: Julia re-centres itself every frame
        // on the repelling fixed point of its drifting c (see `julia_view`).
        kind: Kind::Julia,
        cx: 0.0,
        cy: 0.0,
        span: 3.4,
        rate: 0.35,
    },
    Family {
        kind: Kind::Ship,
        // On the hull's edge above the main ship. Found by boundary descent
        // rather than by eye: a coordinate that looks like it is on the
        // boundary at a 5x zoom is usually deep inside the hull at 100x, and
        // the panel goes black with the structure sliding off a corner.
        cx: -1.775,
        cy: -0.015_996_6,
        span: 3.2,
        // Half the others': the ship's fine structure falls below one cell
        // sooner than the rest, and past ~20x the panel is confetti. The
        // recognisable hull is the point of this family, so it stops there.
        rate: 0.45,
    },
    Family {
        kind: Kind::Newton,
        // A triple point of the three basins, found by boundary descent. The
        // origin is one too, but it is the degenerate one: f'(0) = 0, and its
        // neighbourhood is the trivial six-fold star with no bulb chain in it.
        cx: -0.000_022_512_967,
        cy: -0.050_188_908_811,
        span: 3.0,
        rate: 0.85,
    },
];

/// Cube roots of unity.
const ROOTS: [(f64, f64); 3] = [
    (1.0, 0.0),
    (-0.5, 0.866_025_403_784_439),
    (-0.5, -0.866_025_403_784_439),
];

/// Squared distance at which Newton's iteration counts as converged.
const NEWTON_EPS: f64 = 1e-8;

#[inline]
fn band(i: u32) -> u16 {
    1 + (i % BANDS as u32) as u16
}

#[inline]
fn mandel(x: f64, y: f64, max: u32) -> u16 {
    let (mut zx, mut zy) = (0.0f64, 0.0f64);
    for i in 0..max {
        let (x2, y2) = (zx * zx, zy * zy);
        if x2 + y2 > 4.0 {
            return band(i);
        }
        zy = 2.0 * zx * zy + y;
        zx = x2 - y2 + x;
    }
    0
}

#[inline]
fn julia(x: f64, y: f64, cx: f64, cy: f64, max: u32) -> u16 {
    let (mut zx, mut zy) = (x, y);
    for i in 0..max {
        let (x2, y2) = (zx * zx, zy * zy);
        if x2 + y2 > 4.0 {
            return band(i);
        }
        zy = 2.0 * zx * zy + cy;
        zx = x2 - y2 + cx;
    }
    0
}

/// Burning Ship. The absolute value before squaring is the whole difference
/// from Mandelbrot, and it is what breaks the symmetry into a hull.
#[inline]
fn ship(x: f64, y: f64, max: u32) -> u16 {
    let (mut zx, mut zy) = (0.0f64, 0.0f64);
    for i in 0..max {
        let (x2, y2) = (zx * zx, zy * zy);
        if x2 + y2 > 4.0 {
            return band(i);
        }
        zy = (2.0 * zx * zy).abs() + y;
        zx = x2 - y2 + x;
    }
    0
}

/// Newton's method on z^3 - 1: which root it falls into picks the hue band, how
/// long it took picks the shade within it. Unlike the escape-time three, almost
/// every point converges, so index 0 here is only the boundary the iteration
/// never leaves.
#[inline]
fn newton(x: f64, y: f64, max: u32) -> u16 {
    let (mut zx, mut zy) = (x, y);
    for i in 0..max {
        let (ax, ay) = (zx * zx - zy * zy, 2.0 * zx * zy);
        let (bx, by) = (ax * zx - ay * zy, ax * zy + ay * zx);
        let (dx, dy) = (3.0 * ax, 3.0 * ay);
        let den = dx * dx + dy * dy;
        if den < 1e-30 {
            return 0;
        }
        let (nx, ny) = (bx - 1.0, by);
        zx -= (nx * dx + ny * dy) / den;
        zy -= (ny * dx - nx * dy) / den;
        let mut r = 0;
        while r < ROOTS.len() {
            let (ex, ey) = (zx - ROOTS[r].0, zy - ROOTS[r].1);
            if ex * ex + ey * ey < NEWTON_EPS {
                // Modulo, not a clamp: at a deep zoom every cell is near the
                // boundary and takes many steps, so a clamp collapses each
                // basin to one flat colour. Cycling keeps the contour rings.
                return 1 + (r * 10 + i as usize % 10) as u16;
            }
            r += 1;
        }
    }
    0
}

/// The drifting c, and the point to zoom into for it.
///
/// c walks just inside the main cardioid (`c = mu/2 - mu^2/4`, `|mu| < 1`), so
/// the Julia set stays CONNECTED — a c outside the Mandelbrot set is Cantor
/// dust, which at cell resolution is a flat rectangle of one colour.
///
/// The centre is the repelling fixed point `beta = (1 + sqrt(1-4c))/2`, which
/// lies on the Julia set for every c. That is the whole trick: the set's shape
/// changes as c drifts, so no fixed coordinate stays on the boundary, and a
/// zoom target off the boundary lands in flat interior or flat exterior.
#[inline]
fn julia_view(theta: f64) -> (f64, f64, f64, f64) {
    let (s, c) = theta.sin_cos();
    let (mx, my) = (0.985 * c, 0.985 * s);
    let (cx, cy) = (
        mx * 0.5 - (mx * mx - my * my) * 0.25,
        my * 0.5 - (2.0 * mx * my) * 0.25,
    );
    // sqrt(1 - 4c), principal branch.
    let (wx, wy) = (1.0 - 4.0 * cx, -4.0 * cy);
    let m = (wx * wx + wy * wy).sqrt();
    let rx = ((m + wx) * 0.5).max(0.0).sqrt();
    let ry = ((m - wx) * 0.5).max(0.0).sqrt() * if wy < 0.0 { -1.0 } else { 1.0 };
    (cx, cy, (1.0 + rx) * 0.5, ry * 0.5)
}

/// Where the camera is: centre, and complex units per cell.
struct View {
    cx: f64,
    cy: f64,
    step: f64,
}

/// Map the grid onto the complex plane and fill `shade`. Generic so the
/// family's iteration monomorphises into the loop: a `&dyn Fn` here would be
/// one indirect call per cell, which is the thing the trait doc bans.
#[inline]
fn scan<F: Fn(f64, f64) -> u16>(shade: &mut [u16], cols: usize, rows: usize, v: &View, f: F) {
    let (hw, hh) = (cols as f64 * 0.5, rows as f64 * 0.5);
    for cy in 0..rows {
        let y = v.cy + (cy as f64 - hh + 0.5) * v.step;
        let row = &mut shade[cy * cols..][..cols];
        for (cx, out) in row.iter_mut().enumerate() {
            *out = f(v.cx + (cx as f64 - hw + 0.5) * v.step, y);
        }
    }
}

pub struct Fractal {
    cols: usize,
    rows: usize,
    shade: Vec<u16>,
    grid: Grid,
    view: View,
    /// Index into FAMILIES.
    fam: usize,
    /// Seconds into the current family's cycle.
    t: f64,
    dt: f64,
    cycle: f64,
    fade: f64,
    /// Palette tier for this frame: 0 full, TIERS-1 nearly dark.
    tier: usize,
    /// Per-frame zoom multiplier for the current family.
    step_mul: f64,
    /// Zoom rate as a per-second scale multiplier, before the family's factor.
    zoom_sec: f64,
    iter_base: u32,
    iter_cap: u32,
    iter: u32,
    /// Julia's c, and how fast its argument turns.
    jc: (f64, f64),
    jdrift: f64,
    jtheta: f64,
}

impl Fractal {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        let cell = env_num(&["FRACTAL_CELL"], 20, 4, 64) as usize;
        let grid = Grid::new(panel, cell, cell);
        let (cols, rows) = (grid.cols(), grid.rows());
        let cycle = env_num(&["FRACTAL_SECONDS"], 26, 5, 600) as f64;
        let fade = env_num(&["FRACTAL_FADE_MS"], 900, 0, 5000) as f64 / 1000.0;
        // Percent of the view width the zoom eats each second. 22% compounds to
        // roughly 600x across a 26s cycle: a steady crawl rather than a dive,
        // and well inside f64's comfortable range.
        let zoom_sec = 1.0 - env_num(&["FRACTAL_ZOOM_PCT"], 22, 1, 90) as f64 / 100.0;
        let jdrift = env_num(&["FRACTAL_JULIA_DRIFT"], 90, 0, 2000) as f64 / 1000.0;
        let mut f = Self {
            cols,
            rows,
            shade: vec![0u16; cols * rows],
            grid,
            view: View {
                cx: 0.0,
                cy: 0.0,
                step: 1.0,
            },
            fam: 0,
            // Start past the fade-in: there is no previous image for frame 0
            // to dip out of, and a screensaver whose first second is black
            // reads as one that failed to start.
            t: fade.min(cycle * 0.4),
            dt: 1.0 / fps as f64,
            cycle,
            // A fade longer than half a cycle would never reach full
            // brightness, which reads as a broken palette rather than a fade.
            fade: fade.min(cycle * 0.4),
            tier: TIERS - 1,
            step_mul: 1.0,
            zoom_sec,
            iter_base: env_num(&["FRACTAL_ITER"], 40, 16, 512) as u32,
            iter_cap: env_num(&["FRACTAL_ITER_MAX"], 110, 32, 2000) as u32,
            iter: 0,
            jc: (0.0, 0.0),
            jdrift,
            jtheta: 2.2,
        };
        f.begin();
        f
    }

    /// Reset the camera onto the current family's target.
    fn begin(&mut self) {
        let fam = &FAMILIES[self.fam];
        self.view.cx = fam.cx;
        self.view.cy = fam.cy;
        self.view.step = fam.span / self.cols as f64;
        self.step_mul = self.zoom_sec.powf(fam.rate * self.dt);
        self.iter = self.iter_base;
    }

    fn advance(&mut self) {
        self.t += self.dt;
        if self.t >= self.cycle {
            self.t = 0.0;
            self.fam = (self.fam + 1) % FAMILIES.len();
            self.begin();
        } else {
            self.view.step *= self.step_mul;
        }

        // Detail per cell is constant, so iteration depth has to track zoom
        // depth or a deep view flattens into one band.
        let span0 = FAMILIES[self.fam].span / self.cols as f64;
        let octaves = (span0 / self.view.step).log2().max(0.0);
        self.iter = (self.iter_base + (octaves * 8.0) as u32).min(self.iter_cap);

        self.jtheta += self.jdrift * self.dt;
        let (jx, jy, bx, by) = julia_view(self.jtheta);
        self.jc = (jx, jy);
        if FAMILIES[self.fam].kind == Kind::Julia {
            self.view.cx = bx;
            self.view.cy = by;
        }

        let edge = self.t.min(self.cycle - self.t);
        self.tier = if self.fade <= 0.0 || edge >= self.fade {
            0
        } else {
            TIERS - 1 - ((edge / self.fade) * TIERS as f64) as usize
        };
    }

    fn scan_family(&mut self) {
        let (shade, cols, rows, view, iter) = (
            &mut self.shade[..],
            self.cols,
            self.rows,
            &self.view,
            self.iter,
        );
        // One match per FRAME, not per cell: each arm monomorphises `scan`.
        match FAMILIES[self.fam].kind {
            Kind::Mandel => scan(shade, cols, rows, view, |x, y| mandel(x, y, iter)),
            Kind::Julia => {
                let (jx, jy) = self.jc;
                scan(shade, cols, rows, view, |x, y| julia(x, y, jx, jy, iter))
            }
            Kind::Ship => scan(shade, cols, rows, view, |x, y| ship(x, y, iter)),
            // Newton converges in a handful of steps or never; the escape-time
            // cap would only buy boundary noise at forty times the price.
            Kind::Newton => {
                let n = iter.min(40);
                scan(shade, cols, rows, view, |x, y| newton(x, y, n))
            }
        }
    }
}

impl Saver for Fractal {
    fn render(&mut self, s: &mut Surface<'_>) {
        self.advance();
        self.scan_family();
        let (grid, shade, cols) = (&mut self.grid, &self.shade[..], self.cols);
        let off = (self.tier * BANDS) as u16;
        grid.fill(|cx, cy| {
            let b = shade[cy * cols + cx];
            Cell::new(font::SOLID, if b == 0 { 0 } else { b + off })
        });
        grid.flush(s, &PAL);
    }

    fn name(&self) -> &'static str {
        "fractal"
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

    /// 1070 is deliberately NOT a multiple of the 20px cell: the 10-line strip
    /// below the last cell row is the "error line" frame 0 has to cover.
    fn panel() -> Panel {
        Panel::new(1920, 1070, 1920)
    }

    #[test]
    fn frame_zero_paints_the_whole_panel() {
        let p = panel();
        let mut f = Fractal::new(&p, 15);
        let mut buf = vec![0u32; p.buf_len()];
        let d = saver::frame(&mut f, &mut buf, &p);
        assert_eq!(d.rows(), p.h, "frame 0 must paint the whole panel");
        // Without this a saver that reported a black rectangle would pass.
        assert!(
            buf.iter().filter(|&&px| px != 0).count() > 10_000,
            "frame 0 painted nothing"
        );
    }

    /// The "screen went blank" bug class: pixels written but never reported.
    #[test]
    fn damage_covers_every_changed_scanline() {
        let p = panel();
        let mut f = Fractal::new(&p, 15);
        let mut buf = vec![0u32; p.buf_len()];
        let mut prev = buf.clone();
        saver::frame(&mut f, &mut buf, &p);

        let mut changed_total = 0;
        for n in 1..30 {
            prev.copy_from_slice(&buf);
            let d = saver::frame(&mut f, &mut buf, &p);
            for y in 0..p.h {
                if prev[y * p.w..][..p.w] == buf[y * p.w..][..p.w] {
                    continue;
                }
                changed_total += 1;
                assert!(
                    d.runs()
                        .iter()
                        .any(|&(a, b)| y as u16 >= a && (y as u16) < b),
                    "frame {n}: scanline {y} changed but was not reported"
                );
            }
        }
        // Otherwise the loop above proves nothing: a saver drawing a still
        // image would satisfy it vacuously.
        assert!(
            changed_total > 10_000,
            "only {changed_total} scanlines ever changed; the zoom is not moving"
        );
    }

    /// `render` owns no per-frame collection — `shade` is sized in `new`, only
    /// ever written through by index, and `Grid::fill` writes in place — so the
    /// only allocation this design can make is replacing one of those buffers.
    /// The ADDRESS is what catches that: a same-size replacement keeps both
    /// length and capacity, so those two alone pass a `shade = vec![...]` in
    /// the frame loop, which is the exact bug. (The address cannot see an
    /// unrelated scratch Vec allocated and dropped inside `render`; nothing
    /// here has one, and a counting global allocator is crate-wide state this
    /// module has no business declaring.)
    #[test]
    fn render_never_allocates() {
        let p = Panel::new(200, 150, 200);
        let mut f = Fractal::new(&p, 15);
        let mut buf = vec![0u32; p.buf_len()];
        let (len, cap, at) = (f.shade.len(), f.shade.capacity(), f.shade.as_ptr());
        assert!(len > 0, "nothing was reserved for the frame loop");
        let cells = f.grid.cells().len();
        for _ in 0..50_000 {
            saver::frame(&mut f, &mut buf, &p);
            assert_eq!(
                (f.shade.len(), f.shade.capacity(), f.shade.as_ptr()),
                (len, cap, at),
                "`shade` moved: the render path allocated"
            );
            assert_eq!(f.grid.cells().len(), cells, "the grid was reallocated");
        }
    }

    /// What this saver IS: four families in rotation, each closing in on a
    /// point that keeps structure on screen.
    ///
    /// The failure this catches is the one that actually happened: a zoom
    /// target that is not on the set's boundary. The iteration is still
    /// correct, the zoom is still smooth, every other test passes — and the
    /// panel shows one flat rectangle of colour. So the invariant is stated on
    /// the drawn cells, not on the maths: no single colour may own the frame.
    #[test]
    fn every_family_keeps_structure_on_screen_while_it_zooms() {
        let p = Panel::new(640, 360, 640);
        let fps = 15u32;
        let mut f = Fractal::new(&p, fps);
        let mut buf = vec![0u32; p.buf_len()];
        let frames = (f.cycle * FAMILIES.len() as f64 * fps as f64) as usize;

        let mut seen = [false; FAMILIES.len()];
        let mut worst = 0.0f64;
        let mut last_step = f64::MAX;
        let mut fam = usize::MAX;
        let mut hist = [0u32; PAL_LEN];

        for n in 0..frames {
            saver::frame(&mut f, &mut buf, &p);
            seen[f.fam] = true;

            if f.fam == fam {
                assert!(
                    f.view.step < last_step,
                    "frame {n}: the view stopped closing in ({last_step} -> {})",
                    f.view.step
                );
            }
            (fam, last_step) = (f.fam, f.view.step);

            hist.fill(0);
            for c in f.grid.cells() {
                hist[c.colour()] += 1;
            }
            let cells = f.grid.cells().len() as f64;
            let top = *hist.iter().max().unwrap() as f64 / cells;
            let lit = hist.iter().filter(|&&n| n > 0).count();
            worst = worst.max(top);
            assert!(
                top < 0.9 && lit >= 4,
                "frame {n} ({:?}): {:.0}% of the panel is one colour across {lit} colours \
                 — the zoom has left the boundary",
                FAMILIES[f.fam].kind,
                top * 100.0
            );
        }

        assert!(seen.iter().all(|&s| s), "the rotation skipped a family");
        // The bound is not vacuous: frames with a large interior really do
        // approach it.
        assert!(
            worst > 0.5,
            "the busiest frame was only {:.0}% one colour; the 90% bound is \
             nowhere near the data and proves nothing",
            worst * 100.0
        );
    }
}
