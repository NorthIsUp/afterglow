//! night coast: a lighthouse on a wooded headland under moonlit clouds. The
//! beam turns every eight seconds, the clouds drift, and the sea carries the
//! moon's road and the lamp's reflection.
//!
//! Upstream's `Float32Array`s stay `f32` here: their rounding is part of the
//! picture, and an f64 copy drifts off the golden at dither boundaries.
//!
//! At any size the headland keeps the left and the open sea the rest: wider
//! panels push the moon, the far shore and the beam's reach out with the
//! frame and raise a second hummock beside the moon's road; narrower ones draw
//! the headland in to the cottage; taller ones add sky above and sea below.

use std::f64::consts::PI;

use super::halftone::{bayer, Dots};
use super::math::{clamp, fbm, hash, mix, noise, smooth};
use super::stretch::Stretch;
use super::{hex, Piece};
use crate::grid::Cell;

const LAMP_X: f64 = 30.0;
/// Cloud field width: the clouds wrap at this many columns.
const CW: usize = 640;

const AIR: u8 = 0;
const LAND: u8 = 1;
const TREE: u8 = 2;
const TOWER: u8 = 3;
const LANTERN: u8 = 4;
const CAP: u8 = 5;
const WALL: u8 = 6;
const PANE: u8 = 7;
const ROOF: u8 = 8;
const ISLE: u8 = 9;

/// Where things sit in a `w x h` frame. Upstream's 200x100 is the anchor
/// every value moves from, so there it is exact.
struct Layout {
    w: usize,
    h: usize,
    horizon: f64,
    moon: [f64; 2],
    /// Where the headland falls from its crown to the sea.
    shoulder: [f64; 2],
    /// The woods: the column they stop at, and where they shrink from and to.
    trees: [f64; 3],
    /// The headland's seaward end, and where its foot starts to fade.
    shore: [f64; 2],
    /// Where the far shore starts.
    far_shore: f64,
    /// Its hummocks: [height, centre, half width]; the second only on a
    /// panel wide enough to hold it beside the moon's road.
    hummocks: [[f64; 3]; 2],
    /// The buoy light, on the big hummock's crown.
    buoy: [usize; 2],
    /// How far the beam reaches when it runs across the frame.
    reach: f64,
}

impl Layout {
    fn new(w: usize, h: usize) -> Self {
        let s = Stretch::new(w, h);
        let Stretch { w: wf, tall, .. } = s;
        let horizon = 60.0 + 0.55 * tall;
        let moon = [s.grow(150.0, 86.0, 68.0), 19.0 + 0.35 * tall];
        let far_shore = s.grow(148.0, 30.0, 68.0);
        let hummocks = [
            [
                s.grow(4.5, 1.5, 1.0),
                s.grow(180.0, 108.0, 86.0),
                s.grow(26.0, 6.0, 12.0),
            ],
            [
                3.2 * clamp((wf - 240.0) / 80.0),
                far_shore + (moon[0] - far_shore) * 27.0 / 58.0,
                18.0,
            ],
        ];
        Self {
            w,
            h,
            horizon,
            moon,
            shoulder: [s.grow(86.0, 16.0, 22.0), s.grow(50.0, 8.0, 4.0)],
            trees: [s.grow(76.0, 16.0, 16.0), s.grow(78.0, 16.0, 16.0), s.grow(56.0, 10.0, 10.0)],
            shore: [s.grow(90.0, 16.0, 24.0), s.grow(74.0, 14.0, 18.0)],
            far_shore,
            hummocks,
            buoy: [hummocks[0][1] as usize + 4, (horizon - 5.0) as usize],
            reach: s.grow(115.0, 50.0, 50.0),
        }
    }

    fn top(&self, x: f64) -> f64 {
        self.horizon
            - 15.0 * smooth(self.shoulder[0], self.shoulder[1], x) * (1.0 - 0.12 * smooth(24.0, 0.0, x))
            - 1.6 * fbm(x * 0.15, 3.7, 3, 0.0)
    }
}

pub struct NightCoast {
    l: Layout,
    dots: Dots,
    mat: Vec<u8>,
    shade: Vec<f32>,
    haze: Vec<f32>,
    cover: Vec<f32>,
    lit: Vec<f32>,
    tower_top: f64,
}

fn density(x: f64, y: f64, horizon: f64) -> f64 {
    let cw = CW as f64;
    let q = fbm(x * 0.008, y * 0.02, 3, cw * 0.008);
    let d = fbm(x * 0.018 + q * 2.4, y * 0.036 + q * 1.1, 5, cw * 0.018);
    // heaped mid-sky, thinner overhead, wisps at the horizon
    d + 0.05 * smooth(8.0, 22.0, y)
        - 0.04 * smooth(10.0, 0.0, y)
        - 0.16 * smooth(horizon - 24.0, horizon - 4.0, y)
}

impl Piece for NightCoast {
    const NAME: &'static str = "night-coast";
    const FPS: u32 = 15;
    const GROUND: u32 = hex("#080b12");
    #[rustfmt::skip]
    const PALETTE: &'static [u32] = &[
        hex("#15203d"), hex("#1b2a50"), hex("#223463"), hex("#2b4077"), hex("#364e8b"), hex("#445f9f"), hex("#5874b2"), hex("#7089c0"),
        hex("#8a9cc4"), hex("#a3b1cf"), hex("#bec9dc"), hex("#d8dfea"), hex("#eef1f6"), hex("#f7f8fc"),
        hex("#ffe9ae"), hex("#ffd27c"), hex("#f3a64a"), hex("#c46c2d"),
        hex("#121c1b"), hex("#182724"), hex("#203530"), hex("#2c473d"), hex("#3c5c4c"),
        hex("#232a36"), hex("#363e4c"), hex("#555d6c"), hex("#2a3550"), hex("#3a4868"),
        hex("#9c4136"), hex("#ff5d4d"),
    ];

    fn new(cols: usize, rows: usize) -> Self {
        let l = Layout::new(cols, rows);
        let (w, h, horizon) = (l.w, l.h, l.horizon);
        let n = w * h;
        let mut mat = vec![AIR; n];
        let mut shade = vec![0f32; n];
        let base = l.top(LAMP_X);
        let tower_top = base - 15.0;
        let mut trees: Vec<[f64; 3]> = Vec::new();
        let mut x = 1.0;
        while x < l.trees[0] {
            // the clearing for the tower and the cottage
            if !(x > 22.0 && x < 49.0) {
                trees.push([
                    x + hash(x, 2.0) * 1.2,
                    (4.0 + hash(x, 3.0) * 7.0) * smooth(l.trees[1], l.trees[2], x),
                    2.0 + hash(x, 4.0) * 1.6,
                ]);
            }
            x += 2.6 + hash(x * 7.0, 1.0) * 2.6;
        }
        let cottage = l.top(41.0);
        for r in 0..h {
            for xi in 0..w {
                let k = r * w + xi;
                let x = xi as f64;
                let y = r as f64 + 0.5;
                let t0 = l.top(x);
                let mut isle = horizon;
                for &[hh, hx, hw] in l.hummocks.iter().filter(|m| m[0] > 0.0) {
                    isle -= hh * (1.0 - ((x - hx) / hw).powi(2)).max(0.0);
                }
                isle -= 1.4 * fbm(x * 0.2, 9.0, 2, 0.0);
                if y >= t0 && y < horizon + 4.0 && x < l.shore[0] {
                    mat[k] = LAND;
                    // rock and scrub, the brow of the slope catching the moon
                    shade[k] = (0.2
                        + 0.45 * fbm(x * 0.35, y * 0.35, 3, 0.0)
                        + 0.5 * smooth(t0 + 3.0, t0, y)
                        - 0.2 * smooth(horizon - 4.0, horizon + 4.0, y))
                        as f32;
                } else if x > l.far_shore && y >= isle && y < horizon {
                    mat[k] = ISLE;
                }
                for &[tx, th, tw] in &trees {
                    if th < 1.0 {
                        continue;
                    }
                    let tb = l.top(tx);
                    let dy = y - (tb - th);
                    if dy >= 0.0 && y < tb + 2.0 && (x + 0.5 - tx).abs() <= (dy / th) * tw + 0.4 {
                        mat[k] = TREE;
                        shade[k] = (0.1
                            + 0.35 * hash(x * 13.0 + r as f64, 5.0)
                            + if x + 0.5 > tx { 0.35 } else { 0.0 })
                            as f32;
                    }
                }
                // the tower: tapered, white, lit from the moon on its right
                let dx = x + 0.5 - LAMP_X;
                if y >= tower_top && y < base + 2.0 {
                    let hw = 3.0 - 1.1 * smooth(base, tower_top, y);
                    if dx.abs() <= hw {
                        mat[k] = TOWER;
                        shade[k] = (0.45 + 0.5 * (dx / hw)) as f32;
                    }
                }
                if y >= tower_top - 1.0 && y < tower_top + 1.0 && dx.abs() <= 3.6 {
                    mat[k] = CAP;
                    shade[k] = 0.55;
                }
                if y >= tower_top - 7.0 && y < tower_top - 1.0 && dx.abs() <= 2.0 {
                    mat[k] = LANTERN;
                }
                if y >= tower_top - 10.0
                    && y < tower_top - 7.0
                    && dx.abs() <= 2.8 - (tower_top - 7.0 - y) * 0.8
                {
                    mat[k] = CAP;
                    shade[k] = 0.4;
                }
                // the keeper's cottage, lit inside
                if (36..=47).contains(&xi) && y >= cottage - 5.0 && y < cottage + 1.0 {
                    mat[k] = WALL;
                    shade[k] = (0.3 + 0.4 * (x - 36.0) / 11.0) as f32;
                    if (xi == 39 || xi == 44) && y >= cottage - 4.0 && y < cottage - 1.5 {
                        mat[k] = PANE;
                    }
                }
                if y >= cottage - 10.0
                    && y < cottage - 5.0
                    && (x + 0.5 - 42.0).abs() <= 7.5 - (cottage - 5.0 - y) * 1.2
                {
                    mat[k] = ROOF;
                }
            }
        }

        // a faint unevenness in the clear sky, so it is never one flat tone
        let haze = (0..n)
            .map(|k| fbm((k % w) as f64 * 0.05, (k / w) as f64 * 0.08, 3, 0.0) as f32)
            .collect();

        // the clouds: heaps of noise that wrap, so they can drift forever
        let rows = horizon.ceil() as usize;
        let mut cover = vec![0f32; CW * rows];
        let mut lit = vec![0f32; CW * rows];
        for r in 0..rows {
            for x in 0..CW {
                let (xf, y) = (x as f64, r as f64 + 0.5);
                let d = density(xf, y, horizon);
                cover[r * CW + x] = smooth(0.52, 0.63, d) as f32;
                // Lit on the side facing the moon (up and right), shadowed
                // underneath, a little darker deep inside.
                let toward = density(xf + 2.5, y - 3.0, horizon);
                lit[r * CW + x] = clamp(0.5 + (d - toward) * 11.0 - (d - 0.6) * 1.2) as f32;
            }
        }

        Self {
            l,
            dots: Dots::new(Self::PALETTE),
            mat,
            shade,
            haze,
            cover,
            lit,
            tower_top,
        }
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        let beam = (t / 8.0) * PI * 2.0 - 0.75; // starts out over the sea
        let (cb, sb) = (beam.cos(), beam.sin());
        let flash = sb.max(0.0).powi(14) * 2.2; // pointed at us
        let lamp_y = self.tower_top - 4.0;
        let drift = t * 1.4;
        let l = &self.l;
        let (hf, horizon) = (l.h as f64, l.horizon);

        for r in 0..l.h {
            for xi in 0..l.w {
                let k = r * l.w + xi;
                let x = xi as f64;
                let rf = r as f64;
                let y = rf + 0.5;
                let m = self.mat[k];
                let (mut cr, mut cg, mut cbl) = (0.0, 0.0, 0.0);
                let (mut floor, mut fade) = (0.3, 1.0);
                let (dmx, dmy) = (x + 0.5 - l.moon[0], y - l.moon[1]);
                let dm = (dmx * dmx + dmy * dmy).sqrt();

                if y < horizon && m == AIR {
                    // sky: deep navy up top to a hazy horizon, brighter round the moon
                    let v = y / horizon;
                    let halo = (-dm / 24.0).exp() * 0.35 + (-dm / 9.0).exp() * 0.45;
                    let glow_h = v.powi(3) * 0.24; // the horizon's last light
                    let veil = 0.85 + 0.3 * f64::from(self.haze[k]);
                    cr = (0.04 + 0.1 * v * v + glow_h * 0.9) * veil + 0.3 * halo;
                    cg = (0.07 + 0.13 * v * v + glow_h) * veil + 0.35 * halo;
                    cbl = (0.17 + 0.2 * v * v + glow_h * 1.1) * veil + 0.42 * halo;
                    floor = 0.15;
                    let sx = x + drift;
                    let ix = sx.floor();
                    let fx = sx - ix;
                    let ix = ix as usize;
                    let i0 = r * CW + ix % CW;
                    let i1 = r * CW + (ix + 1) % CW;
                    let (c0, c1) = (f64::from(self.cover[i0]), f64::from(self.cover[i1]));
                    let (l0, l1) = (f64::from(self.lit[i0]), f64::from(self.lit[i1]));
                    // a break in the cloud round the moon
                    let c = (c0 + (c1 - c0) * fx) * (0.15 + 0.85 * smooth(7.0, 24.0, dm));
                    let l = l0 + (l1 - l0) * fx;
                    if dm < 6.0 {
                        // the moon, dimmed where cloud crosses it
                        let face = 0.86 + 0.14 * fbm(x * 0.5, y * 0.5, 2, 0.0);
                        let a = (1.0 - c * 0.8) * smooth(6.0, 5.0, dm);
                        cr = mix(cr, 0.96 * face, a);
                        cg = mix(cg, 0.97 * face, a);
                        cbl = mix(cbl, face, a);
                    } else if c < 0.05 && hash(x, rf * 3.0 + 11.0) > 0.982 {
                        let tw =
                            0.6 + 0.4 * (t * (1.5 + hash(x, rf) * 3.0) + hash(rf, x) * 6.28).sin();
                        let s = (0.5 + 0.5 * tw) * (1.0 - halo) * smooth(horizon, 20.0, y);
                        cr = cr.max(s * 0.92);
                        cg = cg.max(s * 0.94);
                        cbl = cbl.max(s);
                    }
                    if c > 0.01 {
                        // shadow slate, through moonlit grey, to a silver rim near
                        // the moon; low cloud is lit from below by the horizon
                        let near = (-dm / 30.0).exp();
                        let b = clamp(l * (0.5 + 0.6 * near) + 0.25 * smooth(30.0, horizon, y));
                        let ramp = |lo: f64, mid: f64, hi: f64| {
                            if b < 0.5 {
                                mix(lo, mid, b * 2.0)
                            } else {
                                mix(mid, hi, (b - 0.5) * 2.0)
                            }
                        };
                        let (kr, kg, kb) = (
                            ramp(0.1, 0.36, 0.95),
                            ramp(0.13, 0.42, 0.96),
                            ramp(0.25, 0.58, 1.0),
                        );
                        let a = (c * 1.1).min(1.0);
                        cr = mix(cr, kr, a);
                        cg = mix(cg, kg, a);
                        cbl = mix(cbl, kb, a);
                    }
                } else if y >= horizon
                    && (m == AIR || m == LAND)
                    && !(m == LAND && y < horizon + 4.0)
                {
                    // sea: cold, darker toward us, waves stretched along the swell
                    let v = (y - horizon) / (hf - horizon);
                    let w = 0.6 * noise(x * 0.07 + t * 0.12, y * 0.45 - t * 0.6, 0.0)
                        + 0.4 * noise(x * 0.2 - t * 0.25, y * 0.9 - t * 1.1, 0.0);
                    let swell = 0.45 + 0.95 * w;
                    cr = (0.08 - 0.04 * v) * swell;
                    cg = (0.13 - 0.06 * v) * swell;
                    cbl = (0.28 - 0.12 * v) * swell;
                    // the moon's road: wider toward us, broken into glints
                    let road_w = 3.0 + (y - horizon) * 0.55;
                    let road = (-((x + 0.5 - l.moon[0]) / road_w).powi(2)).exp();
                    let glint = smooth(0.5, 0.8, w) * road;
                    cr += 0.95 * glint + 0.07 * road;
                    cg += 0.95 * glint + 0.09 * road;
                    cbl += 0.95 * glint + 0.15 * road;
                    // the lamp's reflection, warm, under the tower
                    let lw = 1.3 + (y - horizon) * 0.22;
                    let refl = (-((x + 0.5 - LAMP_X) / lw).powi(2)).exp()
                        * smooth(0.45, 0.8, w)
                        * (0.75 + 0.6 * flash);
                    cr += refl;
                    cg += 0.72 * refl;
                    cbl += 0.32 * refl;
                    // surf where the headland meets the water
                    if y < horizon + 8.0 && x < l.shore[0] {
                        let edge = smooth(l.shore[0], l.shore[1], x) * smooth(horizon + 8.0, horizon + 3.0, y);
                        let foam =
                            smooth(0.5, 0.85, noise(x * 0.45 - t * 0.6, y * 0.8 + t * 0.4, 0.0))
                                * edge;
                        cr += 0.65 * foam;
                        cg += 0.7 * foam;
                        cbl += 0.75 * foam;
                    }
                    let haze = (-(y - horizon) / 2.5).exp() * 0.16;
                    cr += haze * 0.8;
                    cg += haze * 0.9;
                    cbl += haze;
                    floor = 0.18;
                    fade = smooth(hf, hf - 26.0, y); // the bottom rows thin out into the ground
                } else if m == LAND {
                    let s = f64::from(self.shade[k]);
                    (cr, cg, cbl) = (0.05 + 0.14 * s, 0.09 + 0.2 * s, 0.1 + 0.18 * s);
                    floor = 0.1;
                } else if m == TREE {
                    let s = f64::from(self.shade[k]);
                    (cr, cg, cbl) = (0.03 + 0.07 * s, 0.07 + 0.14 * s, 0.07 + 0.1 * s);
                    floor = 0.06;
                } else if m == ISLE {
                    (cr, cg, cbl) = (0.06, 0.08, 0.15);
                    floor = 0.1;
                    if [xi, r] == l.buoy {
                        // a buoy light out on the point, blinking
                        let on = (t * 2.2).sin() > 0.55;
                        (cr, cg, cbl) = if on {
                            (1.0, 0.36, 0.3)
                        } else {
                            (0.35, 0.12, 0.1)
                        };
                    }
                } else if m == TOWER || m == CAP {
                    let s = f64::from(self.shade[k]);
                    (cr, cg, cbl) = (0.4 + 0.58 * s, 0.42 + 0.56 * s, 0.48 + 0.52 * s);
                    if m == CAP {
                        cr *= 0.7;
                        cg *= 0.66;
                        cbl *= 0.68;
                    }
                } else if m == LANTERN {
                    let g = 0.85 + 0.15 * flash.min(1.0);
                    (cr, cg, cbl) = (g, 0.84 * g, 0.5 * g);
                } else if m == WALL {
                    let s = f64::from(self.shade[k]);
                    (cr, cg, cbl) = (0.3 + 0.4 * s, 0.32 + 0.4 * s, 0.38 + 0.4 * s);
                } else if m == PANE {
                    let g = 0.85 + 0.15 * (t * 3.0 + x).sin();
                    (cr, cg, cbl) = (g, 0.72 * g, 0.32 * g);
                } else if m == ROOF {
                    (cr, cg, cbl) = (0.4, 0.17, 0.14);
                }

                // The beam: a soft cone from the lamp to one side, shortened as
                // it turns toward or away from us, and a glow that flares when
                // it faces us.
                let (bx, by) = (x + 0.5 - LAMP_X, y - lamp_y);
                if m != TOWER && m != CAP && m != LANTERN {
                    if bx * cb > 0.0 {
                        let along = bx.abs() / (cb.abs() * l.reach + 1.0);
                        if along < 1.0 {
                            let spread = 1.4 + bx.abs() * 0.11;
                            let b = (1.0 - along).powf(1.8) * (-(by / spread).powi(2)).exp() * 0.85;
                            cr += b;
                            cg += b * 0.84;
                            cbl += b * 0.5;
                        }
                    }
                    let dl = (bx * bx + by * by).sqrt();
                    let glow = (-dl / (3.5 + 3.0 * flash)).exp() * (0.5 + 0.6 * flash)
                        + (-dl / 14.0).exp() * 0.12;
                    cr += glow;
                    cg += glow * 0.82;
                    cbl += glow * 0.5;
                }

                let peak = cr.max(cg).max(cbl).max(1e-4);
                let level = clamp(floor + (1.0 - floor) * peak.powf(0.85) * 0.95) * fade;
                let step = Dots::step(level, bayer(r, xi));
                out[k] = self.dots.ink(step, level, [cr, cg, cbl], peak);
            }
        }
    }
}
