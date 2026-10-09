//! kyoto dusk: a five-storey pagoda dark against an indigo to rose sky with a
//! thin crescent moon, a temple pond holding its reflection, and in front a
//! cherry tree in full bloom lit from below by a stone lantern. Petals fall
//! and drift through the frame, the lantern flickers, the pond shivers.
//!
//! Its dither is its own (Bayer at 0.4 plus hashed noise) and its colours go
//! through a soft-knee tone curve first; the rest is [`Dots::ink`].
//!
//! At any size the cherry tree and lantern keep the left and the pagoda the
//! far bank: wider panels run the bank on with a temple hall beside the
//! pagoda, the hills further east and the moon out with the frame; narrower
//! ones pull the tree's limbs in and the pagoda toward it, the moon going
//! over to the spire's left when there is no sky right of it; taller ones add
//! sky above and pond below.

mod plans;

use plans::{hall, hill_b, lantern, pagoda, town};

use super::halftone::{Dots, BAYER};
use super::math::{clamp, fbm, hash, js_round, mix, noise, smooth};
use super::stretch::Stretch;
use super::{hex, Piece};
use crate::font;
use crate::grid::Cell;

/// The far bank of the pond, where the pagoda stands, on upstream's grid.
const SHORE: f64 = 80.0;
/// The lantern's foot and scale.
const LB: f64 = 98.0;
const LS: f64 = 1.8;

const SKY: u8 = 0;
const HILL: u8 = 1;
const TOWN: u8 = 2;
const PAGODA: u8 = 3;
const POND: u8 = 4;
const BANK: u8 = 5;
const TREE: u8 = 6;
const BLOOM: u8 = 7;
const STONE: u8 = 8;
const FLAME: u8 = 9;

/// Cloud field width: the clouds wrap at this many columns, in rows C0..C1.
const CW: usize = 500;
const C0: usize = 34;
const C1: usize = 68;

/// Distance from a point to a segment, and how far along it the nearest
/// point is.
fn seg(px: f64, py: f64, ax: f64, ay: f64, bx: f64, by: f64) -> (f64, f64) {
    let (dx, dy) = (bx - ax, by - ay);
    let k = clamp(((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy));
    let (ex, ey) = (ax + dx * k - px, ay + dy * k - py);
    ((ex * ex + ey * ey).sqrt(), k)
}

fn cdens(x: f64, y: f64) -> f64 {
    let (c0, c1) = (C0 as f64, C1 as f64);
    fbm(x * 0.014, y * 0.13, 4, CW as f64 * 0.014)
        - 0.3 * (1.0 - smooth(c0, c0 + 10.0, y) * smooth(c1, c1 - 8.0, y))
}

/// A falling petal's own drift, fall, sway and tumble.
struct Petal {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    sw: f64,
    sf: f64,
    ph: f64,
    tum: f64,
}

/// Where things sit in a `w x h` frame. Upstream's 200x100 is the anchor
/// every value moves from, so there it is exact.
struct Layout {
    w: usize,
    h: usize,
    /// Rows of sky added above the far bank, and how far the near bank (the
    /// tree, the lantern) moves down to stay on the bottom row.
    sky: f64,
    fg: f64,
    /// The far bank of the pond.
    shore: f64,
    /// The near bank's columns draw in toward the trunk on a narrow panel:
    /// [scale, shift], both about the trunk's top.
    fg_squeeze: [f64; 2],
    /// The pagoda's axis.
    px: f64,
    /// The temple hall's axis.
    hall: Option<f64>,
    moon: [i32; 2],
    /// The glow low in the west: its centre column, and how far it spreads.
    west: [f64; 2],
    /// How far the glow lights the clouds' undersides.
    west_cloud: f64,
    /// Where the far hills rise from low in the west to full height.
    hills: [f64; 2],
    /// Where the town along the far bank starts, behind the tree.
    town_from: f64,
    /// The lantern's axis, and its lit opening.
    lx: f64,
    lamp: [f64; 2],
}

/// The trunk's top: the near bank draws in about it.
const PIVOT: f64 = 31.0;

impl Layout {
    fn new(w: usize, h: usize) -> Self {
        let s = Stretch::new(w, h);
        let Stretch { w: wf, wide, narrow, tall } = s;
        let sky = (tall * 0.25).round();
        let px = s.grow(141.0, 85.0, 57.0);
        // right of the pagoda while there is sky for it, else left of the spire
        let gap = wf - (px + 14.5);
        let moon = if wf >= 200.0 {
            [176.0 + 108.0 * wide, 17.0 + 0.6 * sky]
        } else if gap >= 14.0 {
            [px + 14.5 + gap / 2.0, 17.0 + 0.6 * sky]
        } else {
            [px - 22.0, 8.0 + 0.8 * sky]
        };
        let mut l = Self {
            w,
            h,
            sky,
            fg: tall,
            shore: SHORE + sky,
            fg_squeeze: [1.0 - 0.5 * narrow, -10.0 * narrow],
            px,
            hall: (wf >= 290.0).then_some(px - 60.0),
            moon: moon.map(|v| v.round() as i32),
            west: [s.grow(80.0, 10.0, 40.0), s.grow(120.0, 50.0, 60.0)],
            west_cloud: s.grow(90.0, 50.0, 45.0),
            hills: [s.grow(70.0, 20.0, 35.0), s.grow(175.0, 95.0, 75.0)],
            town_from: 70.0,
            lx: 53.0,
            lamp: [0.0; 2],
        };
        l.town_from = l.fg_x(70.0);
        l.lx = l.fg_x(53.0);
        l.lamp = [l.lx, LB - (LB - 83.0) * LS + l.fg];
        l
    }

    /// Upstream's column `x` on the near bank, drawn in on a narrow panel.
    fn fg_x(&self, x: f64) -> f64 {
        let [s, shift] = self.fg_squeeze;
        if s == 1.0 {
            x
        } else {
            PIVOT + (x - PIVOT) * s + shift
        }
    }

    fn sky_at(&self, x: f64, y: f64) -> [f64; 3] {
        let v = clamp(y / self.shore);
        // indigo overhead, through violet, to rose and peach low in the west (left)
        let west = (-(x - self.west[0]).abs() / self.west[1]).exp();
        let mut a = smooth(0.0, 0.5, v);
        let (mut r, mut g, mut b) = (mix(0.07, 0.22, a), mix(0.07, 0.14, a), mix(0.25, 0.38, a));
        a = smooth(0.42, 0.86, v);
        (r, g, b) = (mix(r, 0.52, a), mix(g, 0.32, a), mix(b, 0.58, a));
        a = smooth(0.78, 0.98, v) * (0.55 + 0.45 * west);
        (r, g, b) = (mix(r, 1.0, a), mix(g, 0.62, a), mix(b, 0.48, a));
        let band = (-(y - 74.0 - self.sky).abs() / 4.0).exp() * west * 0.18;
        r += band;
        g += band * 0.66;
        b += band * 0.45;
        // a faint unevenness, so no stretch of sky is one flat tone
        let veil = 0.86 + 0.28 * fbm(x * 0.035, y * 0.09, 3, 0.0);
        r *= veil;
        g *= veil;
        b *= veil;
        // the moon's halo
        let (dx, dy) = (x - f64::from(self.moon[0]), y - f64::from(self.moon[1]));
        let d = (dx * dx + dy * dy).sqrt();
        let halo = (-d / 5.0).exp() * 0.24 + (-d / 16.0).exp() * 0.07;
        [r + halo * 0.75, g + halo * 0.72, b + halo]
    }

    /// Low in the west where the glow is, rising behind the pagoda and the town.
    fn hill_a(&self, x: f64) -> f64 {
        77.5 + self.sky
            - (3.0 + 13.0 * smooth(self.hills[0], self.hills[1], x))
                * (0.3 + 1.1 * fbm(x * 0.022 + 3.0, 1.0, 4, 0.0))
    }

    /// The near bank's edge on row `y`.
    fn bank_x(&self, y: f64) -> f64 {
        let y = y - self.fg;
        self.fg_x(38.0 + (y - SHORE) * 2.3 + 4.0 * fbm(y * 0.2, 4.0, 2, 0.0))
    }

    /// The lantern drawn a size up from its plan, standing on the bank.
    fn lantern_at(&self, xc: f64, y: f64) -> u8 {
        lantern((xc - self.lx) / LS, LB - (LB - (y - self.fg)) / LS)
    }
}

pub struct KyotoDusk {
    l: Layout,
    dots: Dots,
    mat: Vec<u8>,
    r: Vec<f32>,
    g: Vec<f32>,
    b: Vec<f32>,
    floor: Vec<f32>,
    /// How much of the lantern's light each cell takes.
    warmth: Vec<f32>,
    /// The reflection source: the far side before the tree covers it.
    rr: Vec<f32>,
    rg: Vec<f32>,
    rb: Vec<f32>,
    glow: Vec<f32>,
    /// [cell, phase, speed].
    stars: Vec<(usize, f64, f64)>,
    ccov: Vec<f32>,
    clit: Vec<f32>,
    petals: Vec<Petal>,
    /// Petals afloat on the pond: [x, y, speed].
    floaters: Vec<[f64; 3]>,
    dith: Vec<f32>,
    tone: Vec<f32>,
    pr: Vec<f32>,
    pg: Vec<f32>,
    pb: Vec<f32>,
    pmask: Vec<bool>,
    moon_ink: Cell,
    star_dim: Cell,
    star_bright: Cell,
    floater_ink: Cell,
}

impl Piece for KyotoDusk {
    const NAME: &'static str = "kyoto-dusk";
    const FPS: u32 = 15;
    const GROUND: u32 = hex("#0b0a16");
    #[rustfmt::skip]
    const PALETTE: &'static [u32] = &[
        hex("#1c1d4a"), hex("#262759"), hex("#33316b"), hex("#433d7c"), hex("#574a8c"), hex("#6e5a9a"),
        hex("#8a5a92"), hex("#a8668f"), hex("#c4748f"), hex("#db8a95"), hex("#ec9f9c"), hex("#f6b8a8"), hex("#fbd0b8"),
        hex("#fff1d8"), hex("#fffaf0"), hex("#7e5f9e"),
        hex("#f9d3dc"), hex("#f2b2c4"), hex("#e28fab"), hex("#c66e92"), hex("#9c5279"), hex("#74405f"),
        hex("#ffd9a0"), hex("#ffbb6a"), hex("#f29a4a"), hex("#d0763a"), hex("#a35a35"),
        hex("#2a2240"), hex("#3a2c50"), hex("#4a3860"),
        hex("#5b4f86"), hex("#7a6aa0"),
        hex("#2c3a3a"), hex("#3d4c46"), hex("#56604f"),
        hex("#9a6aa0"), hex("#b47ea6"),
        hex("#4a3038"), hex("#6a4448"),
        hex("#2b3a6a"), hex("#3e4f86"),
    ];

    fn new(cols: usize, rows: usize) -> Self {
        let l = Layout::new(cols, rows);
        let (w, h, shore) = (l.w, l.h, l.shore);
        let (n, hf) = (w * h, h as f64);
        let mut mat = vec![SKY; n];
        let mut rv = vec![0f32; n];
        let mut gv = vec![0f32; n];
        let mut bv = vec![0f32; n];
        let mut floor = vec![0.12f32; n];
        let mut warmth = vec![0f32; n];

        // the cherry tree's skeleton: [ax, ay, bx, by, w0, w1]
        #[rustfmt::skip]
        let mut branches: Vec<[f64; 6]> = vec![
            // trunk, leaning up and to the right
            [16.0, 101.0, 20.0, 86.0, 5.2, 4.4], [20.0, 86.0, 27.0, 70.0, 4.4, 3.6], [27.0, 70.0, 31.0, 58.0, 3.6, 3.0],
            // limbs
            [31.0, 58.0, 50.0, 46.0, 3.0, 2.1], [50.0, 46.0, 74.0, 37.0, 2.1, 1.4], [74.0, 37.0, 98.0, 31.0, 1.4, 0.9], [98.0, 31.0, 116.0, 30.0, 0.9, 0.5],
            [31.0, 58.0, 24.0, 40.0, 2.6, 1.8], [24.0, 40.0, 12.0, 26.0, 1.8, 1.1], [12.0, 26.0, 0.0, 18.0, 1.1, 0.7],
            [29.0, 50.0, 42.0, 30.0, 2.0, 1.3], [42.0, 30.0, 52.0, 19.0, 1.3, 0.8], [52.0, 19.0, 70.0, 13.0, 0.8, 0.5],
            [50.0, 46.0, 60.0, 52.0, 1.2, 0.7], [74.0, 37.0, 86.0, 44.0, 1.0, 0.5], [42.0, 30.0, 66.0, 24.0, 1.0, 0.6], [66.0, 24.0, 90.0, 21.0, 0.6, 0.4],
            [24.0, 40.0, 34.0, 25.0, 1.0, 0.6], [12.0, 26.0, 6.0, 16.0, 0.7, 0.4], [98.0, 31.0, 104.0, 40.0, 0.6, 0.35],
            // sprays drooping low over the lantern
            [40.0, 51.0, 44.0, 58.0, 1.1, 0.4], [58.0, 43.0, 62.0, 55.0, 0.9, 0.35], [70.0, 38.0, 76.0, 47.0, 0.7, 0.3],
        ];
        // short twigs off every limb, reaching up and out
        let limbs = branches.len();
        for b in 3..limbs {
            let [ax, ay, bx, by, w0, w1] = branches[b];
            let bf = b as f64;
            let ang = (by - ay).atan2(bx - ax);
            for j in 0..3 {
                let jf = f64::from(j);
                let k = 0.25 + 0.65 * hash(bf * 7.0 + jf, 31.0);
                let (sx, sy) = (mix(ax, bx, k), mix(ay, by, k));
                let turn =
                    (if j & 1 != 0 { 1.0 } else { -1.0 }) * (0.5 + 0.6 * hash(bf, jf + 40.0));
                let len = 4.0 + 7.0 * hash(bf + jf, 41.0);
                let a2 = ang + turn - 0.25;
                branches.push([
                    sx,
                    sy,
                    sx + a2.cos() * len,
                    sy + a2.sin() * len,
                    mix(w0, w1, k) * 0.55,
                    0.3,
                ]);
            }
        }
        // blossom clumps all along the limbs and twigs, thickest at their ends
        let mut clusters: Vec<[f64; 3]> = Vec::new();
        for (b, &[ax, ay, bx, by, w0, _]) in branches.iter().enumerate().skip(3) {
            let bf = b as f64;
            let len = (bx - ax).hypot(by - ay);
            let n = 1.0 + js_round(len / 1.7);
            let mut i = 0.0;
            while i <= n {
                let k = i / n;
                if !(k < 0.3 && w0 > 2.0) {
                    let h1 = hash(bf * 13.0 + i, 7.0);
                    let h2 = hash(bf * 5.0 + i, 11.0);
                    let h3 = hash(bf * 3.0 + i, 17.0);
                    let cx = mix(ax, bx, k) + (h1 - 0.5) * 6.0;
                    let cy = mix(ay, by, k) + (h2 - 0.5) * 4.5 - 1.0;
                    // only the drooping sprays (and their twigs) hang below the crown
                    let spray = (b >= limbs - 3 && b < limbs) || b >= limbs + 3 * (limbs - 6);
                    // the crown is domed: thinner toward its top, never cut by the frame
                    let dome = 9.0 + 10.0 * clamp((cx - 52.0).abs() / 70.0).powf(1.6);
                    if !(cx > 122.0 || cy < dome || cy > if spray { 55.0 } else { 47.0 }) {
                        clusters.push([cx, cy, 1.4 + 1.6 * h3 + 0.8 * k]);
                    }
                }
                i += 1.0;
            }
        }

        for b in &mut branches {
            [b[0], b[1], b[2], b[3]] = [l.fg_x(b[0]), b[1] + l.fg, l.fg_x(b[2]), b[3] + l.fg];
        }
        for c in &mut clusters {
            [c[0], c[1]] = [l.fg_x(c[0]), c[1] + l.fg];
        }

        // build the static picture
        for r in 0..h {
            for x in 0..w {
                let k = r * w + x;
                let y = r as f64 + 0.5;
                let xc = x as f64 + 0.5;
                let (mut m, mut fl, mut warm) = (SKY, 0.12, 0.0);
                let (mut cr, mut cg, mut cb) = (0.0, 0.0, 0.0);
                if y < shore {
                    [cr, cg, cb] = l.sky_at(xc, y);
                    fl = 0.04;
                    // distant hills in haze, then a nearer ridge
                    let ha = l.hill_a(xc);
                    if y >= ha {
                        // the Higashiyama hills, flat and violet in the haze
                        m = HILL;
                        let s = l.sky_at(xc, ha);
                        let d = smooth(ha, ha + 8.0, y);
                        // wooded slopes: clumps of trees in the haze
                        let tex = fbm(xc * 0.3, y * 0.45, 3, 0.0);
                        let a = 0.6 + 0.2 * d + 0.25 * (tex - 0.5);
                        (cr, cg, cb) = (mix(s[0], 0.14, a), mix(s[1], 0.08, a), mix(s[2], 0.24, a));
                        // the ridge line catches the afterglow
                        let lip = smooth(ha + 1.4, ha, y) * 0.3;
                        (cr, cg, cb) = (
                            mix(cr, s[0] * 1.1, lip),
                            mix(cg, s[1] * 1.05, lip),
                            mix(cb, s[2], lip),
                        );
                    }
                    let hb = hill_b(xc) + l.sky;
                    if y >= hb {
                        m = HILL;
                        let tex = fbm(xc * 0.3, y * 0.3, 3, 0.0);
                        (cr, cg, cb) = (0.1 + 0.05 * tex, 0.07 + 0.03 * tex, 0.17 + 0.05 * tex);
                        // mist settling along the water
                        let mist = smooth(hb + 1.0, shore, y) * 0.5;
                        (cr, cg, cb) =
                            (mix(cr, 0.7, mist), mix(cg, 0.44, mist), mix(cb, 0.58, mist));
                    }
                    let tw = town(xc) + l.sky;
                    if y >= tw && xc > l.town_from {
                        m = TOWN;
                        (cr, cg, cb) = (0.08, 0.06, 0.14);
                        // the roof ridges catch the sky
                        let tf = (((xc + 3.0) / 11.0) % 1.0 - 0.5).abs() * 2.0;
                        if y < tw + 0.9 && tf < 0.45 {
                            (cr, cg, cb, fl) = (0.5, 0.31, 0.44, 0.06);
                        }
                        // a few lit shoji under the eaves
                        let wi = (xc / 4.0).floor();
                        if y > shore - 1.6
                            && y < shore - 0.5
                            && (xc % 4.0) < 1.2
                            && hash(wi, 7.0) > 0.74
                        {
                            (cr, cg, cb, fl) = (1.0, 0.66, 0.32, 0.3);
                        }
                    }
                    let yu = y - l.sky;
                    let (p, dx) = match (pagoda(xc - l.px, yu), l.hall) {
                        (0, Some(hx)) => (hall(xc - hx, yu), xc - hx),
                        (p, _) => (p, xc - l.px),
                    };
                    if p != 0 {
                        m = PAGODA;
                        let rim = (-(dx + 4.0).abs() / 30.0).exp(); // the sky to the west lights its left
                                                                    // dark timber; the tiled roofs pick up the sky, their ridges most of all
                        match p {
                            1 => (cr, cg, cb) = (0.07, 0.05, 0.12),
                            2 => {
                                (cr, cg, cb) =
                                    (0.2 + 0.08 * rim, 0.13 + 0.03 * rim, 0.27 + 0.04 * rim);
                            }
                            3 => (cr, cg, cb) = (0.05, 0.04, 0.1),
                            4 => {
                                (cr, cg, cb) = (0.9 * rim + 0.25, 0.5 * rim + 0.15, 0.5 * rim + 0.3);
                            }
                            5 => {
                                (cr, cg, cb) =
                                    (0.3 + 0.2 * if dx < 0.0 { 1.0 } else { 0.0 }, 0.22, 0.32);
                            }
                            _ => (cr, cg, cb, fl) = (0.7, 0.42, 0.24, 0.2),
                        }
                    }
                } else if xc < l.bank_x(y) {
                    // the near bank where the lantern stands
                    m = BANK;
                    let tex = fbm(xc * 0.3, y * 0.5, 3, 0.0);
                    (cr, cg, cb) = (0.08 + 0.05 * tex, 0.08 + 0.05 * tex, 0.1 + 0.05 * tex);
                    let (xf, rf) = (x as f64, r as f64);
                    // fallen petals on the moss
                    if hash(xf * 7.0, rf * 3.0) > 0.975 - 0.015 * smooth(shore + 4.0, hf, y) {
                        (cr, cg, cb, fl) = (0.42, 0.22, 0.3, 0.06);
                    }
                    // the stone lip of the pond
                    if xc > l.bank_x(y) - 2.6 {
                        let s = 0.85 + 0.3 * hash(xf * 3.0, rf * 5.0);
                        (cr, cg, cb, fl) = (0.45 * s, 0.38 * s, 0.46 * s, 0.1);
                    }
                    warm = 1.0;
                } else {
                    m = POND;
                }
                mat[k] = m;
                rv[k] = cr as f32;
                gv[k] = cg as f32;
                bv[k] = cb as f32;
                floor[k] = fl as f32;
                warmth[k] = warm as f32;
            }
        }
        let (rr, rg, rb) = (rv.clone(), gv.clone(), bv.clone());

        // the stone lantern
        for r in 45 + l.fg as usize..h {
            for x in (l.lx as usize - 14)..=(l.lx as usize + 14) {
                let y = r as f64 + 0.5;
                let xc = x as f64 + 0.5;
                let lt = l.lantern_at(xc, y);
                if lt == 0 {
                    continue;
                }
                let k = r * w + x;
                let py = LB - (LB - (y - l.fg)) / LS;
                let pdx = (xc - l.lx) / LS;
                if lt == 2 {
                    mat[k] = FLAME;
                    // hottest at the heart of the firebox
                    let c = (-(pdx * pdx * 0.5 + (py - 82.8) * (py - 82.8) * 0.35)).exp();
                    rv[k] = (0.85 + 0.15 * c) as f32;
                    gv[k] = (0.5 + 0.38 * c) as f32;
                    bv[k] = (0.2 + 0.4 * c * c) as f32;
                    floor[k] = 0.35;
                } else {
                    mat[k] = STONE;
                    // dark granite, its top surfaces catching the last of the sky
                    let g = 0.8 + 0.4 * hash(x as f64 * 5.0, r as f64 * 3.0);
                    let edge = if l.lantern_at(xc, y - 1.0) == 0 {
                        1.0
                    } else {
                        0.0
                    };
                    rv[k] = ((0.05 + 0.22 * edge) * g) as f32;
                    gv[k] = ((0.04 + 0.14 * edge) * g) as f32;
                    bv[k] = ((0.08 + 0.2 * edge) * g) as f32;
                    // the roof's underside and the platform take the flame's light
                    let under = (lt == 3 && py > 78.6) || (85.5..86.4).contains(&py);
                    if under {
                        let f = (-(py - 82.8).abs() / 3.0).exp() * 0.75;
                        rv[k] = (f64::from(rv[k]) + f) as f32;
                        gv[k] = (f64::from(gv[k]) + f * 0.5) as f32;
                        bv[k] = (f64::from(bv[k]) + f * 0.18) as f32;
                    }
                    floor[k] = 0.02;
                }
                warmth[k] = if lt == 2 { 0.0 } else { 0.75 };
            }
        }

        // the cherry tree
        let reach = (l.fg_x(130.0).ceil() as usize).min(w);
        for r in 0..h {
            for x in 0..reach {
                let k = r * w + x;
                let y = r as f64 + 0.5;
                let xc = x as f64 + 0.5;
                // blossom density: soft clouds, broken up by noise into clumps
                let (mut d, mut best, mut up) = (0.0, 0.0, 0.0);
                for &[cx, cy, rad] in &clusters {
                    let (dx, dy) = (xc - cx, (y - cy) * 1.15);
                    let q = (dx * dx + dy * dy) / (rad * rad);
                    if q < 3.0 {
                        let c = (-q * 1.4).exp();
                        d += c;
                        if c > best {
                            best = c;
                            up = (dy + dx * 0.4) / rad;
                        }
                    }
                }
                // big holes where the sky shows through, small ones between clumps
                let n = fbm(xc * 0.11, y * 0.15, 3, 0.0);
                let n2 = noise(xc * 0.5, y * 0.6, 0.0);
                let bloom = (1.0 - (-d * 1.5).exp()) * (0.15 + 1.35 * n) * (0.75 + 0.5 * n2);
                let (mut on_branch, mut bw) = (false, 0.0f64);
                for &[ax, ay, bx, by, w0, w1] in &branches {
                    let (dist, kk) = seg(xc, y, ax, ay, bx, by);
                    let w = mix(w0, w1, kk) * 0.5 + 0.15;
                    if dist < w {
                        on_branch = true;
                        let side = if xc < ax + (bx - ax) * kk { -1.0 } else { 1.0 };
                        bw = bw.max((dist / w) * side);
                    }
                }
                if on_branch && bloom < 0.85 {
                    mat[k] = TREE;
                    // dark bark threading through the blossom; the side toward the lantern catches it
                    let lit = smooth(0.35, 1.0, bw);
                    rv[k] = (0.05 + 0.03 * lit) as f32;
                    gv[k] = (0.035 + 0.01 * lit) as f32;
                    bv[k] = 0.07;
                    floor[k] = 0.0;
                    warmth[k] = 0.8;
                } else if bloom > 0.5 {
                    mat[k] = BLOOM;
                    // each spray pale pink on top where the sky lights it, deep rose beneath
                    let inner = hash(x as f64 * 3.0, r as f64 * 7.0);
                    let mut b = clamp(
                        0.62 - 0.6 * up + 0.26 * (inner - 0.5) - 0.3 * smooth(0.75, 0.5, bloom),
                    );
                    b = mix(b, 0.12, smooth(0.3, 1.1, up));
                    // the lower crown sits in its own shade
                    b *= 1.0 - 0.3 * smooth(28.0, 48.0, y - l.fg);
                    rv[k] = (0.34 + 0.64 * b) as f32;
                    gv[k] = (0.08 + 0.62 * b * b) as f32;
                    bv[k] = (0.2 + 0.5 * b) as f32;
                    floor[k] = 0.1;
                    warmth[k] = 1.0;
                }
            }
        }

        // the lantern's light: how far each cell sits from the lit opening
        let mut glow = vec![0f32; n];
        for r in 0..h {
            for x in 0..w {
                let k = r * w + x;
                let dx = x as f64 + 0.5 - l.lamp[0];
                let dy = (r as f64 + 0.5 - l.lamp[1]) * 1.1;
                let d = (dx * dx + dy * dy).sqrt();
                glow[k] = ((-d / 4.0).exp() * 0.6
                    + (-d / 11.0).exp() * 0.32
                    + (-d / 32.0).exp() * 0.1) as f32;
                // the blossom overhead takes the lamp's light from below, from further off
                let m = mat[k];
                if m == BLOOM || m == TREE {
                    glow[k] = (f64::from(glow[k]) + (-d / 11.0).exp() * 1.6) as f32;
                }
                // and a pool of light on the moss round its foot
                if m == BANK {
                    let px = (x as f64 + 0.5 - l.lx) / 20.0;
                    let py = (r as f64 + 0.5 - (LB + l.fg) + 3.0) / 8.0;
                    glow[k] = (f64::from(glow[k]) + (-(px * px + py * py)).exp() * 0.6) as f32;
                }
            }
        }

        // a few faint stars in the indigo
        let mut stars = Vec::new();
        let high = shore - 40.0;
        for i in 0..400 * w * high as usize / 8000 {
            let i = i as f64;
            let x = (hash(i, 91.0) * w as f64).floor() as usize;
            let r = (hash(i, 92.0) * high).floor() as usize;
            let k = r * w + x;
            if mat[k] == SKY && hash(i, 93.0) > 0.72 {
                stars.push((k, hash(i, 94.0) * 6.28, 0.6 + hash(i, 95.0) * 1.6));
            }
        }

        // thin streaks of cloud low in the sky, dark violet, their undersides
        // lit by the set sun; they wrap so they can drift forever
        let mut ccov = vec![0f32; CW * (C1 - C0)];
        let mut clit = vec![0f32; CW * (C1 - C0)];
        for r in C0..C1 {
            for x in 0..CW {
                // on upstream's rows: the band moves down with the shore
                let (xf, y) = (x as f64, r as f64 + 0.5);
                let d = cdens(xf, y);
                ccov[(r - C0) * CW + x] = smooth(0.56, 0.7, d) as f32;
                clit[(r - C0) * CW + x] = clamp(0.5 + (d - cdens(xf - 1.0, y + 2.2)) * 7.0) as f32;
            }
        }

        // petals: most of them near the tree, thinning out downwind
        let petals = (0..50 * n / 20_000)
            .map(|i| {
                let i = i as f64;
                Petal {
                    x: hash(i, 101.0).powf(2.2) * 140.0,
                    y: hash(i, 102.0) * (hf + 20.0),
                    vx: 3.0 + hash(i, 103.0) * 4.0,
                    vy: 1.6 + hash(i, 104.0) * 2.2,
                    sw: 1.0 + hash(i, 105.0) * 2.0,
                    sf: 0.6 + hash(i, 106.0) * 1.2,
                    ph: hash(i, 107.0) * 6.28,
                    tum: 2.0 + hash(i, 108.0) * 4.0,
                }
            })
            .collect();
        let floaters = (0..26 * w / 200)
            .map(|i| {
                let i = i as f64;
                [
                    hash(i, 111.0) * w as f64,
                    shore + 3.0 + hash(i, 112.0) * (hf - shore),
                    0.3 + hash(i, 113.0) * 0.5,
                ]
            })
            .collect();

        let dith = (0..n)
            .map(|k| {
                (BAYER[((k / w) & 3) * 4 + ((k % w) & 3)] * 0.4
                    + (hash(k as f64, 77.0) - 0.5) * 0.5) as f32
            })
            .collect();
        let tone = (0..=1024)
            .map(|i| {
                let v = f64::from(i) / 256.0;
                (if v < 0.75 {
                    v
                } else {
                    0.75 + 0.25 * (1.0 - (-(v - 0.75) * 4.0).exp())
                }) as f32
            })
            .collect();

        let mut dots = Dots::new(Self::PALETTE);
        let ink = |dots: &mut Dots, step: usize, rgb: [f64; 3]| {
            Cell::new(
                font::HALFTONE[step],
                u16::from(dots.nearest(rgb[0], rgb[1], rgb[2])),
            )
        };
        let moon_ink = ink(&mut dots, 3, [1.0, 0.96, 0.86]);
        let star_dim = ink(&mut dots, 1, [0.9, 0.88, 1.0]);
        let star_bright = ink(&mut dots, 2, [0.9, 0.88, 1.0]);
        let floater_ink = ink(&mut dots, 2, [0.95, 0.72, 0.8]);

        Self {
            l,
            dots,
            mat,
            r: rv,
            g: gv,
            b: bv,
            floor,
            warmth,
            rr,
            rg,
            rb,
            glow,
            stars,
            ccov,
            clit,
            petals,
            floaters,
            dith,
            tone,
            pr: vec![0f32; n],
            pg: vec![0f32; n],
            pb: vec![0f32; n],
            pmask: vec![false; n],
            moon_ink,
            star_dim,
            star_bright,
            floater_ink,
        }
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        let l = &self.l;
        let (w, wf, hf, cw) = (l.w, l.w as f64, l.h as f64, CW as f64);
        let (shore, band0, band1) = (l.shore, C0 + l.sky as usize, C1 + l.sky as usize);
        let (west, west_cloud) = (l.west[0], l.west_cloud);
        // the flame breathes, with now and then a gutter
        let flick = 0.82
            + 0.1 * (t * 7.3).sin() * (t * 3.1 + 1.0).sin()
            + 0.12 * (noise(t * 4.0, 3.3, 0.0) - 0.5) * 2.0;
        let cdrift = ((t * 0.8) % cw + cw) % cw;

        // petals in the air this frame
        self.pmask.fill(false);
        for p in &self.petals {
            let xx = ((p.x + p.vx * t + p.sw * (t * p.sf + p.ph).sin()) % (wf + 40.0) + wf + 40.0)
                % (wf + 40.0)
                - 20.0;
            let yy =
                ((p.y + p.vy * t + 0.6 * (t * p.sf * 1.7 + p.ph).sin()) % (hf + 20.0) + hf + 20.0)
                    % (hf + 20.0)
                    - 10.0;
            let (x, r) = (js_round(xx), js_round(yy));
            if x < 0.0 || x >= wf || r < 0.0 || r >= hf {
                continue;
            }
            let k = r as usize * w + x as usize;
            // tumbling, catching light; dim against the dark bank and water
            let dim = if self.mat[k] == POND || self.mat[k] == BANK {
                0.6
            } else {
                1.0
            };
            let face = (0.55 + 0.45 * (t * p.tum + p.ph).sin().abs()) * dim;
            let lg = f64::from(self.glow[k]) * flick * 1.6;
            self.pr[k] = (face + lg * 0.6) as f32;
            self.pg[k] = (0.76 * face + lg * 0.35) as f32;
            self.pb[k] = (0.84 * face + lg * 0.1) as f32;
            self.pmask[k] = true;
        }

        for r in 0..l.h {
            let y = r as f64 + 0.5;
            for x in 0..w {
                let k = r * w + x;
                let m = self.mat[k];
                let xf = x as f64;
                let (mut cr, mut cg, mut cb);
                let mut fl = f64::from(self.floor[k]);
                let mut fade = 1.0;
                if m == POND {
                    // the far side, upside down, shaken by small ripples
                    let depth = (y - shore) / (hf - shore);
                    let wob = (y * 1.7 - t * 2.0 + (xf * 0.13 + t * 0.6).sin() * 1.4).sin()
                        * (0.05 + 0.8 * depth);
                    let sx = js_round(xf + wob).clamp(0.0, wf - 1.0);
                    let wv = noise(xf * 0.14 + t * 0.2, y * 1.1 - t * 0.8, 0.0);
                    // ripples also tip the image up and down, so level lines break
                    // (drawn a little stretched, so the pagoda's lower roofs land in the pond)
                    let sr =
                        js_round(shore - 1.0 - (r as f64 - shore) * 1.35 + (wv - 0.5) * 3.0 * depth)
                            .clamp(0.0, shore - 1.0);
                    let j = sr as usize * w + sx as usize;
                    // crisp and bright right under the bank, darker further out
                    let lit = mix(0.95, 0.6, smooth(shore + 1.0, shore + 6.0, y)) * (0.8 + 0.4 * wv);
                    cr = f64::from(self.rr[j]) * lit * 0.9;
                    cg = f64::from(self.rg[j]) * lit * 0.92;
                    cb = f64::from(self.rb[j]) * lit + 0.03;
                    let glint = smooth(0.74, 0.95, wv) * 0.06;
                    cr += glint;
                    cg += glint * 0.8;
                    cb += glint;
                    // the lantern's light laid on the water as a broken warm streak
                    let sl = (-(xf + 0.5 - l.lx - 9.0 - (y - shore) * 0.25).abs()
                        / (3.5 + depth * 3.0))
                        .exp()
                        * smooth(0.45, 0.8, wv)
                        * 1.0
                        * flick;
                    cr += sl;
                    cg += sl * 0.62;
                    cb += sl * 0.25;
                    fade = smooth(hf + 2.0, hf - 9.0, y);
                } else {
                    cr = f64::from(self.r[k]);
                    cg = f64::from(self.g[k]);
                    cb = f64::from(self.b[k]);
                    if m == SKY && (band0..band1).contains(&r) {
                        let sx = xf + cdrift;
                        let ixf = sx.floor();
                        let fx = sx - ixf;
                        let ix = ixf as usize;
                        let i0 = (r - band0) * CW + ix % CW;
                        let i1 = (r - band0) * CW + (ix + 1) % CW;
                        let (c0, c1) = (f64::from(self.ccov[i0]), f64::from(self.ccov[i1]));
                        let c = c0 + (c1 - c0) * fx;
                        if c > 0.01 {
                            let (l0, l1) = (f64::from(self.clit[i0]), f64::from(self.clit[i1]));
                            let l = l0 + (l1 - l0) * fx;
                            // brighter undersides toward the west and the horizon
                            let sun = l
                                * (0.45 + 0.55 * smooth(band0 as f64, band1 as f64, y))
                                * (0.6 + 0.4 * (-(xf - west).abs() / west_cloud).exp());
                            let a = c * 0.85;
                            cr = mix(cr, mix(0.2, 1.0, sun), a);
                            cg = mix(cg, mix(0.12, 0.58, sun), a);
                            cb = mix(cb, mix(0.27, 0.5, sun), a);
                        }
                    }
                }
                // lamplight on everything near the lantern
                let wm = f64::from(self.warmth[k]);
                if wm > 0.0 {
                    let g = f64::from(self.glow[k]) * flick * wm;
                    cr += g;
                    cg += g * 0.58;
                    cb += g * 0.22;
                } else if m == SKY || m == POND {
                    let g = f64::from(self.glow[k]) * flick * 0.6;
                    cr += g;
                    cg += g * 0.6;
                    cb += g * 0.3;
                }
                if m == FLAME {
                    let f = 0.85 + 0.25 * flick;
                    (cr, cg, cb) = (f, 0.72 * f, 0.38 * f);
                }
                if self.pmask[k] && m != FLAME {
                    cr = f64::from(self.pr[k]);
                    cg = f64::from(self.pg[k]);
                    cb = f64::from(self.pb[k]);
                    fl = 0.3;
                }

                let tone = |v: f64| f64::from(self.tone[(((v * 256.0) as i32).min(1024)) as usize]);
                (cr, cg, cb) = (tone(cr), tone(cg), tone(cb));
                let peak = cr.max(cg).max(cb).max(1e-4);
                let level = clamp(fl + (1.0 - fl) * peak.powf(1.1) * 1.02) * fade;
                let step = Dots::step(level, f64::from(self.dith[k]));
                out[k] = self.dots.ink(step, level, [cr, cg, cb], peak);
            }
        }
        // the moon: a thin crescent, lit on the side toward the set sun
        let [mx, my] = l.moon;
        for r in my - 6..=my + 6 {
            for x in mx - 6..=mx + 6 {
                let dx = f64::from(x) + 0.5 - f64::from(mx);
                let dy = f64::from(r) + 0.5 - f64::from(my);
                let inside = dx * dx + dy * dy < 5.2 * 5.2;
                let (ex, ey) = (dx - 2.2, dy + 1.6); // the shadowed disc, offset up and right
                if inside && ex * ex + ey * ey > 4.9 * 4.9 {
                    out[r as usize * w + x as usize] = self.moon_ink;
                }
            }
        }
        // stars twinkle
        for &(k, ph, sp) in &self.stars {
            let s = (t * sp + ph).sin();
            if s > -0.3 {
                out[k] = if s > 0.7 {
                    self.star_bright
                } else {
                    self.star_dim
                };
            }
        }
        // petals resting on the pond
        for &[fx, fy, sp] in &self.floaters {
            let x = js_round((fx + t * sp) % wf);
            let r = js_round(fy + 0.3 * (t * 0.8 + fx).sin());
            if r >= hf {
                continue;
            }
            // x can round up to w, which upstream's flat index wraps onto the
            // next row's first cell
            let k = r as usize * w + x as usize;
            if k >= self.mat.len() || self.mat[k] != POND {
                continue;
            }
            out[k] = self.floater_ink;
        }
    }
}

