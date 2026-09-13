//! Moire interference from overlapping line families.
//!
//! Two gratings — straight, concentric or radial, `MOIRE_KINDS` picks — drift
//! and rotate against each other. What you are meant to see is the BEAT: where
//! the two coincide their ink lands on the same dots and the coverage halves;
//! where they interleave it nearly doubles. The individual lines are
//! incidental, and the fringes they beat into sweep as the gratings turn.
//!
//! A cell's colour is its OVERLAP DEPTH, not its coverage: one grating draws a
//! mid blue, two crossing draw near-white. Coverage was the obvious choice and
//! is wrong — a near-vertical line drifting across the dot pitch makes the
//! coverage of each cell flicker between two values, and that sparkle drowns
//! the fringe it is supposed to be showing.
//!
//! Two decisions carry the cost, and this saver is not cheap:
//!
//! * Ink accumulates into a 2x4-per-cell dot buffer read out as braille, so the
//!   geometry is eight times finer than the cell grid without eight times the
//!   blit. The default cell is 8x32, i.e. 4x8 px dots: the gratings run close to
//!   vertical, so horizontal resolution is what shows and vertical resolution is
//!   what gets spent.
//! * A straight family's phase is linear in x and y, so ALL of them stamp in one
//!   pass, in wrapping u32 fixed point where the overflow is the modulo — a
//!   wrapping add, an unsigned compare and a byte add per dot, no branch and no
//!   float. Curved families have no linear phase and pay a sqrt (concentric) or
//!   an atan2 (radial) per dot, each in its own pass with the kind matched
//!   outside the loops.
//!
//! Measured against matrix on the same machine, same panel: the default `sc`
//! costs 2.1x matrix per frame, `ss` 1.4x. It is a full repaint — the fringes
//! sweep everywhere, so nearly every cell changes and damage is the whole panel
//! every frame. `MOIRE_KINDS=r` is roughly five straight families and is why the
//! default has no radial.

use crate::env_num;
use crate::env_str;
use crate::font;
use crate::grid::{bake, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};

/// Index 0 is black (an uncovered cell); 1..=15 ramp deep indigo -> cyan ->
/// pale. The low end is not pure blue: a grating this fine at low brightness
/// reads as noise unless the darkest ink still carries some green.
#[rustfmt::skip]
const PAL_RGB: [[u8; 3]; 16] = [
    [0x00, 0x00, 0x00],
    [0x0A, 0x0C, 0x2A], [0x12, 0x16, 0x3E], [0x1A, 0x22, 0x54], [0x20, 0x30, 0x6B],
    [0x24, 0x40, 0x82], [0x26, 0x52, 0x99], [0x26, 0x66, 0xAE], [0x24, 0x7B, 0xC1],
    [0x22, 0x91, 0xD1], [0x24, 0xA7, 0xDE], [0x30, 0xBC, 0xE8], [0x46, 0xCF, 0xEF],
    [0x68, 0xDF, 0xF4], [0x95, 0xEC, 0xF8], [0xCB, 0xF7, 0xFC],
];
const PAL: [u32; 16] = bake(&PAL_RGB);

/// Braille bit for dot (row, col), indexed `row * 2 + col`. Dots 7 and 8 are
/// the bottom row and live in the high bits — the Unicode layout, not a typo.
#[rustfmt::skip]
const DOT_BIT: [u8; 8] = [
    1 << 0, 1 << 3,
    1 << 1, 1 << 4,
    1 << 2, 1 << 5,
    1 << 6, 1 << 7,
];

const STRAIGHT: u8 = 0;
const CONCENTRIC: u8 = 1;
const RADIAL: u8 = 2;

/// Angle between consecutive straight families, in radians. The beat period is
/// `spacing / sin(SPLAY)`, so 0.09 rad puts about four fringes across 1920 px
/// at the default spacing. Not a knob: `MOIRE_SPIN` already moves this, and a
/// second control over the same quantity is two ways to get the same wrong
/// picture.
const SPLAY: f32 = 0.09;

/// One grating. Angle, offset and centre are derived from `t` each frame rather
/// than integrated, so a pod up for a week cannot accumulate float drift into
/// the geometry.
struct Family {
    kind: u8,
    inv_d: f32,
    ang0: f32,
    spin: f32,
    off0: f32,
    off_rate: f32,
    /// Lissajous wander of the centre, for the kinds that have one. A pinned
    /// centre makes a concentric family look like a printed target.
    cx0: f32,
    cy0: f32,
    ax: f32,
    ay: f32,
    fx: f32,
    fy: f32,
    ph: f32,
    spokes: f32,
}

/// Phase as a fraction of a turn, in u32 fixed point. A straight family's phase
/// is linear in x and y, so in this representation the wraparound IS the modulo:
/// the inner loop becomes a wrapping add, an unsigned compare and a byte add,
/// with no branch and no float. That is the difference between this saver
/// fitting the CPU budget and not.
#[inline]
fn turns(v: f32) -> u32 {
    ((v - v.floor()) * 4_294_967_296.0) as u64 as u32
}

impl Family {
    #[inline]
    fn centre(&self, t: f32) -> (f32, f32) {
        (
            self.cx0 + self.ax * (self.fx * t + self.ph).sin(),
            self.cy0 + self.ay * (self.fy * t).sin(),
        )
    }
}

pub struct Moire {
    grid: Grid,
    families: Vec<Family>,
    /// Coverage count per dot, `dw * dh`, one byte each. Sized in `new` and
    /// only ever `fill`ed; nothing in `render` can grow it.
    acc: Vec<u8>,
    dw: usize,
    dh: usize,
    sub_w: f32,
    sub_h: f32,
    duty: f32,
    /// Palette index per overlap depth, `0..=families.len()`. Brightness is
    /// how many gratings cross in a cell, NOT how much of the cell is covered:
    /// coverage makes a near-vertical line sparkle cell by cell as it drifts
    /// across the dot pitch, and that sparkle drowns the fringe it is meant to
    /// be showing.
    lvl_of: [u16; 5],
    t: f32,
    dt: f32,
}

impl Moire {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        let cell_w = env_num(&["MOIRE_CELL_W"], 8, 4, 64) as usize;
        let cell_h = env_num(&["MOIRE_CELL_H"], 32, 8, 128) as usize;
        let spacing = env_num(&["MOIRE_SPACING"], 64, 6, 400) as f32;
        let duty = env_num(&["MOIRE_DUTY"], 22, 2, 90) as f32 / 100.0;
        // Millidegrees per SECOND. 0.4 deg/s: two counter-rotating gratings
        // then take about a minute to sweep their beat period across 1920 px.
        let spin = env_num(&["MOIRE_SPIN"], 400, 0, 60_000) as f32;
        let drift = env_num(&["MOIRE_DRIFT"], 9, 0, 600) as f32;
        let kinds = env_str(&["MOIRE_KINDS"], "sc");

        let grid = Grid::new(panel, cell_w, cell_h);
        let (dw, dh) = (grid.cols() * 2, grid.rows() * 4);
        let (w, h) = (panel.w as f32, panel.h as f32);

        let spin_rad = spin.to_radians() / 1000.0;
        // An all-junk MOIRE_KINDS still has to produce a picture: a headless pod
        // showing black is indistinguishable from a crashed one.
        let mut kind_of: Vec<u8> = kinds
            .chars()
            .filter_map(|c| match c {
                's' => Some(STRAIGHT),
                'c' => Some(CONCENTRIC),
                'r' => Some(RADIAL),
                _ => None,
            })
            .take(4)
            .collect();
        if kind_of.is_empty() {
            kind_of = vec![STRAIGHT, CONCENTRIC];
        }

        let mut families = Vec::with_capacity(kind_of.len());
        for (k, &kind) in kind_of.iter().enumerate() {
            let f = k as f32;
            let sgn = if k % 2 == 0 { 1.0 } else { -1.0 };
            // Spacings differ by a few percent on purpose: equal spacing at
            // equal angle has no beat at all, which is the failure mode this
            // saver is one edit away from.
            let d = spacing * (1.0 + 0.11 * f);
            families.push(Family {
                kind,
                inv_d: 1.0 / d,
                ang0: 0.35 + f * SPLAY,
                spin: spin_rad * sgn * (1.0 + 0.3 * f),
                off0: f * 0.37,
                off_rate: drift * sgn / d,
                cx0: w * 0.5,
                cy0: h * 0.5,
                ax: w * 0.17,
                ay: h * 0.21,
                fx: 0.031 + 0.011 * f,
                fy: 0.023 + 0.013 * f,
                ph: f * 1.7,
                spokes: 48.0 + 12.0 * f,
            });
        }

        let n = families.len() as f32;
        let mut lvl_of = [0u16; 5];
        for (d, l) in lvl_of.iter_mut().enumerate().skip(1) {
            *l = (7.0 + 8.0 * (d as f32 - 1.0) / (n - 1.0).max(1.0)).min(15.0) as u16;
        }
        Self {
            acc: vec![0u8; dw * dh],
            grid,
            families,
            dw,
            dh,
            sub_w: cell_w as f32 / 2.0,
            sub_h: cell_h as f32 / 4.0,
            duty,
            lvl_of,
            t: 0.0,
            dt: 1.0 / fps as f32,
        }
    }

    /// Add every family's ink into the dot buffer. The kind is matched once per
    /// family per frame — never per dot — which is also what lets the straight
    /// case reduce to two adds and a floor.
    fn stamp(&mut self) {
        let (dw, dh, duty, t) = (self.dw, self.dh, self.duty, self.t);
        let duty_u32 = turns(duty);
        let (sw, sh) = (self.sub_w, self.sub_h);

        // Every straight family in ONE pass, so a dot is loaded and stored once
        // however many gratings cross it — and the first write is a store, which
        // is why there is no memset here. The two straight families of the
        // default cost barely more than one.
        let mut ns = 0usize;
        let (mut p0, mut sx, mut sy) = ([0u32; 4], [0u32; 4], [0u32; 4]);
        for f in self.families.iter().filter(|f| f.kind == STRAIGHT) {
            let (sa, ca) = (f.ang0 + f.spin * t).sin_cos();
            let (dpx, dpy) = (sw * ca * f.inv_d, sh * sa * f.inv_d);
            p0[ns] = turns(0.5 * dpx + 0.5 * dpy + f.off0 + f.off_rate * t);
            sx[ns] = turns(dpx);
            sy[ns] = turns(dpy);
            ns += 1;
        }

        let acc = &mut self.acc[..];
        if ns == 0 {
            acc.fill(0);
        } else {
            for j in 0..dh {
                let mut p = p0;
                for k in 0..ns {
                    p[k] = p[k].wrapping_add((j as u32).wrapping_mul(sy[k]));
                }
                for v in acc[j * dw..j * dw + dw].iter_mut() {
                    let mut c = 0u8;
                    for k in 0..ns {
                        c += u8::from(p[k] < duty_u32);
                        p[k] = p[k].wrapping_add(sx[k]);
                    }
                    *v = c;
                }
            }
        }

        // The curved kinds have no linear phase to walk, so they pay a sqrt or
        // an atan2 per dot and get a pass each, kind matched ONCE outside the
        // loops. `MOIRE_KINDS=r` costs roughly five straight families; that is
        // why the default has none.
        for f in self.families.iter().filter(|f| f.kind != STRAIGHT) {
            let off = f.off0 + f.off_rate * t;
            let (cx, cy) = f.centre(t);
            if f.kind == CONCENTRIC {
                for j in 0..dh {
                    let dy = (j as f32 + 0.5) * sh - cy;
                    let dy2 = dy * dy;
                    let mut dx = 0.5 * sw - cx;
                    for v in acc[j * dw..j * dw + dw].iter_mut() {
                        let p = (dx * dx + dy2).sqrt() * f.inv_d + off;
                        *v += u8::from(p - p.floor() < duty);
                        dx += sw;
                    }
                }
            } else {
                let k = f.spokes * std::f32::consts::FRAC_1_PI * 0.5;
                for j in 0..dh {
                    let dy = (j as f32 + 0.5) * sh - cy;
                    let mut dx = 0.5 * sw - cx;
                    for v in acc[j * dw..j * dw + dw].iter_mut() {
                        let p = dy.atan2(dx) * k + off;
                        *v += u8::from(p - p.floor() < duty);
                        dx += sw;
                    }
                }
            }
        }
    }
}

impl Saver for Moire {
    fn render(&mut self, s: &mut Surface<'_>) {
        self.t += self.dt;
        self.stamp();
        let (grid, acc, dw, lvl_of) = (&mut self.grid, &self.acc[..], self.dw, self.lvl_of);
        grid.fill(|cx, cy| {
            let base = cy * 4 * dw + cx * 2;
            let mut pat = 0u8;
            let mut deep = 0u8;
            for r in 0..4 {
                for c in 0..2 {
                    let v = acc[base + r * dw + c];
                    deep = deep.max(v);
                    if v != 0 {
                        pat |= DOT_BIT[r * 2 + c];
                    }
                }
            }
            if pat == 0 {
                return Cell::CLEAR;
            }
            Cell::new(font::BRAILLE[pat as usize], lvl_of[deep as usize])
        });
        grid.flush(s, &PAL);
    }

    fn name(&self) -> &'static str {
        "moire"
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

    /// 1080 / 16 = 67.5: the 8-line bottom remainder belongs to no cell and is
    /// exactly what frame 0 has to cover.
    fn panel() -> Panel {
        Panel::new(1920, 1080, 1920)
    }

    /// T1. Frame 0 reaches every scanline, including the strip below the last
    /// cell row, and painted the SCENE rather than a reported black rectangle.
    #[test]
    fn frame_zero_paints_the_whole_panel() {
        let p = panel();
        let mut m = Moire::new(&p, 15);
        assert_ne!(p.h % m.grid.cell_h(), 0, "test panel divides exactly");
        let mut buf = vec![0u32; p.buf_len()];
        let d = saver::frame(&mut m, &mut buf, &p);
        assert_eq!(d.rows(), p.h, "frame 0 must paint the whole panel");
        assert!(
            buf.iter().filter(|&&px| px != 0).count() > 10_000,
            "frame 0 painted nothing"
        );
    }

    /// T2. Damage covers every scanline whose pixels actually moved, checked
    /// against the real framebuffer frame by frame. The second half records
    /// what this saver costs: it is a full repaint and the bound would catch a
    /// change that silently stopped redrawing.
    #[test]
    fn damage_covers_every_changed_scanline() {
        let p = panel();
        let mut m = Moire::new(&p, 15);
        let mut buf = vec![0u32; p.buf_len()];
        let mut before = vec![0u32; p.buf_len()];
        let mut total = 0usize;
        let stride = p.buf_len() / p.h;
        const N: usize = 40;
        for n in 0..N {
            before.copy_from_slice(&buf);
            let d = saver::frame(&mut m, &mut buf, &p);
            total += d.rows();
            for y in 0..p.h {
                let row = y * stride..y * stride + p.w;
                if before[row.clone()] != buf[row] {
                    let hit = d
                        .runs()
                        .iter()
                        .any(|&(a, b)| y as u16 >= a && (y as u16) < b);
                    assert!(hit, "frame {n}: scanline {y} changed but was not reported");
                }
            }
        }
        assert!(
            total / N > p.h * 9 / 10,
            "moire is a full-repaint saver; damage collapsed to {} rows/frame",
            total / N
        );
    }

    /// T3. `render` allocates nothing. There is no per-frame `Vec` at all — the
    /// dot buffer is sized in `new` and only ever `fill`ed, and `Grid::fill`
    /// writes in place — so unchanged length AND capacity across a long run is
    /// what "did not allocate" means here.
    #[test]
    fn render_never_allocates() {
        // Small panel: 50k frames at 1920x1080 takes minutes and exercises the
        // identical code path.
        let p = Panel::new(320, 200, 320);
        let mut m = Moire::new(&p, 15);
        let mut buf = vec![0u32; p.buf_len()];
        let (len, cap) = (m.acc.len(), m.acc.capacity());
        assert!(len > 0, "nothing was reserved for the frame loop");
        let fams = m.families.len();
        for _ in 0..50_000 {
            saver::frame(&mut m, &mut buf, &p);
            assert_eq!(m.acc.len(), len, "`acc` was resized in the render path");
            assert_eq!(m.acc.capacity(), cap, "`acc` reallocated: render allocated");
            assert_eq!(m.families.len(), fams, "`families` grew in the render path");
        }
        // Non-vacuous: the reserve really is being filled, not sitting at zero.
        assert!(
            m.acc.iter().any(|&v| v > 0),
            "the dot buffer was never stamped"
        );
    }

    /// Mean cell brightness per vertical band of the drawn frame — i.e. mean
    /// coverage, which is what a fringe is.
    fn bands(m: &Moire) -> Vec<f32> {
        const N: usize = 12;
        let (cols, rows) = (m.grid.cols(), m.grid.rows());
        let cells = m.grid.cells();
        (0..N)
            .map(|b| {
                let (x0, x1) = (b * cols / N, (b + 1) * cols / N);
                let mut sum = 0u32;
                for cy in 0..rows {
                    for cx in x0..x1 {
                        sum += cells[cy * cols + cx].colour() as u32;
                    }
                }
                sum as f32 / ((x1 - x0) * rows) as f32
            })
            .collect()
    }

    fn spread(b: &[f32]) -> f32 {
        b.iter().cloned().fold(0.0f32, f32::max) - b.iter().cloned().fold(f32::MAX, f32::min)
    }

    /// T4. The thing this saver IS: gratings that BEAT. Interference means
    /// coverage density varies in broad bands across the panel. A single
    /// grating — or several with identical geometry — is uniform everywhere and
    /// would sail through any "does it draw lines" test.
    ///
    /// The companion half forces every family onto identical geometry and shows
    /// the spread collapsing, so the bound is not one every implementation
    /// satisfies.
    #[test]
    fn the_families_beat_against_each_other() {
        let p = panel();
        let mut buf = vec![0u32; p.buf_len()];

        let mut m = Moire::new(&p, 15);
        for _ in 0..30 {
            saver::frame(&mut m, &mut buf, &p);
        }
        let beat = spread(&bands(&m));

        // Every family becomes a copy of the first, so the panel shows ONE
        // grating drawn twice. Anything left in the band spread after that is
        // the dot grid aliasing against the lines, not interference.
        let mut flat = Moire::new(&p, 15);
        let g = (
            flat.families[0].kind,
            flat.families[0].inv_d,
            flat.families[0].ang0,
            flat.families[0].spin,
            flat.families[0].off0,
            flat.families[0].off_rate,
        );
        for f in flat.families.iter_mut() {
            (f.kind, f.inv_d, f.ang0, f.spin, f.off0, f.off_rate) = g;
        }
        for _ in 0..30 {
            saver::frame(&mut flat, &mut buf, &p);
        }
        let flat_spread = spread(&bands(&flat));

        assert!(
            flat_spread < 0.25,
            "identical gratings must not beat, got a spread of {flat_spread}"
        );
        assert!(
            beat > 4.0 * flat_spread.max(0.05),
            "no interference: band spread {beat} against a flat baseline {flat_spread}"
        );
    }

    /// T5. The fringes sweep. Dropping the clock advance leaves a saver that
    /// passes everything above and shows one frozen picture forever.
    #[test]
    fn the_pattern_sweeps() {
        let p = panel();
        let mut m = Moire::new(&p, 15);
        let mut buf = vec![0u32; p.buf_len()];
        saver::frame(&mut m, &mut buf, &p);
        let snap: Vec<Cell> = m.grid.cells().to_vec();
        for _ in 0..15 {
            saver::frame(&mut m, &mut buf, &p);
        }
        let moved = m
            .grid
            .cells()
            .iter()
            .zip(snap.iter())
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            moved * 4 > snap.len(),
            "after a second only {moved} of {} cells moved",
            snap.len()
        );
    }
}
