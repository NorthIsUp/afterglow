//! kyoto dusk: a five-storey pagoda dark against an indigo to rose sky with a
//! thin crescent moon, a temple pond holding its reflection, and in front a
//! cherry tree in full bloom lit from below by a stone lantern. Petals fall
//! and drift through the frame, the lantern flickers, the pond shivers.
//!
//! Its dither is its own (Bayer at 0.4 plus hashed noise) and its colours go
//! through a soft-knee tone curve first; the rest is [`Dots::ink`].

use super::halftone::{Dots, BAYER};
use super::math::{clamp, fbm, hash, js_round, mix, noise, smooth};
use super::{Fit, Piece, hex};
use crate::font;
use crate::grid::Cell;

const W: usize = 200;
const H: usize = 100;
const N: usize = W * H;
/// The far bank of the pond, where the pagoda stands.
const SHORE: f64 = 80.0;
/// The pagoda's axis.
const PX: f64 = 141.0;
const MOON: [i32; 2] = [176, 17];
/// The lantern's axis, foot and scale.
const LX: f64 = 53.0;
const LB: f64 = 98.0;
const LS: f64 = 1.8;
/// Its lit opening.
const LAMP: [f64; 2] = [LX, LB - (LB - 83.0) * LS];

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

/// Where each roof's eave sits.
const EAVE: [f64; 5] = [68.5, 58.0, 48.5, 40.0, 32.5];
/// Each roof's half-width.
const ROOF: [f64; 5] = [14.5, 13.4, 12.3, 11.2, 10.1];
const BODY: [f64; 5] = [6.4, 5.9, 5.4, 4.9, 4.4];

/// The pagoda, in cells about its axis: a shade code or 0 for air. 1 body,
/// 2 roof top, 3 roof underside, 4 roof rim, 5 spire, 6 lit doorway.
fn pagoda(dx: f64, y: f64) -> u8 {
    let ax = dx.abs();
    // stone base
    if (76.0..SHORE + 0.5).contains(&y) {
        return u8::from(ax <= 9.5 - if y < 77.0 { 1.0 } else { 0.0 });
    }
    for i in 0..5 {
        let (e, r) = (EAVE[i], ROOF[i]);
        // a roof: thin at its upturned tips, rising in a shallow concave curve to the body
        let u = ax / r;
        if u <= 1.0 {
            let lift = 3.5 * u * u * u * u;
            let bottom = e - lift + 0.6;
            let top = e - lift - 1.2 - 3.6 * (1.0 - u).powf(1.7);
            if y >= top && y < bottom {
                if y < top + 0.9 {
                    return 4;
                }
                if y > bottom - 1.0 {
                    return 3;
                }
                return 2;
            }
        }
        // the storey beneath this roof, up from the roof below it (or the base)
        let floor = if i == 0 { 76.0 } else { EAVE[i - 1] - 3.6 };
        if y >= e + 0.6 && y < floor && ax <= BODY[i] {
            if i == 0 && ax <= 1.0 && y > 72.0 && y < 75.5 {
                return 6;
            }
            // a railed balcony under each roof
            if y < e + 2.0 && ax <= BODY[i] + 1.2 {
                return 3;
            }
            return 1;
        }
        if i > 0 && y >= e + 0.6 && y < e + 2.0 && ax <= BODY[i] + 1.2 {
            return 3;
        }
    }
    // the spire: a mast with nine rings and a flame-shaped finial
    let top = EAVE[4] - 1.2 - 3.6 - 0.2;
    if y < top && y >= 9.0 {
        if y >= top - 1.5 {
            return if ax <= 2.4 { 3 } else { 0 }; // the roof box
        }
        if y >= 15.0 && y < top - 1.5 {
            let ring = ((y - 15.0) / 1.2).floor() as i32 & 1;
            return if ax <= if ring != 0 { 1.4 } else { 0.55 } {
                5
            } else {
                0
            };
        }
        if y >= 12.0 {
            return if ax <= 1.3 - (y - 12.0) * 0.2 { 5 } else { 0 };
        }
        return if ax <= 0.6 { 5 } else { 0 };
    }
    0
}

/// The stone lantern in its plan: 1 stone, 2 lit opening, 3 roof, 0 air.
fn lantern(dx: f64, y: f64) -> u8 {
    let ax = dx.abs();
    if (93.5..LB).contains(&y) {
        return u8::from(ax <= 4.2 - if y < 94.5 { 0.8 } else { 0.0 }); // foot
    }
    if (87.0..93.5).contains(&y) {
        return u8::from(ax <= 1.4); // post
    }
    if (85.5..87.0).contains(&y) {
        return u8::from(ax <= 3.6 - if y < 86.2 { 0.6 } else { 0.0 }); // platform
    }
    if (80.0..85.5).contains(&y) {
        if ax <= 1.7 && (80.8..84.8).contains(&y) {
            return 2; // lit opening
        }
        return u8::from(ax <= 2.9);
    }
    if (76.5..80.0).contains(&y) {
        // the roof, flaring out with upturned corners
        let u = (80.0 - y) / 3.5;
        let hw = 5.6 - 4.1 * u.powf(0.8) + if y > 79.2 { 0.6 } else { 0.0 };
        return if ax <= hw { 3 } else { 0 };
    }
    if (73.5..76.5).contains(&y) {
        // finial
        return if ax <= 1.3 - (y - 75.0).abs() * 0.25 || ax <= 0.5 {
            3
        } else {
            0
        };
    }
    0
}

/// The lantern drawn a size up from its plan, standing on the bank.
fn lantern_at(xc: f64, y: f64) -> u8 {
    lantern((xc - LX) / LS, LB - (LB - y) / LS)
}

fn sky_at(x: f64, y: f64) -> [f64; 3] {
    let v = clamp(y / SHORE);
    // indigo overhead, through violet, to rose and peach low in the west (left)
    let west = (-(x - 80.0).abs() / 120.0).exp();
    let mut a = smooth(0.0, 0.5, v);
    let (mut r, mut g, mut b) = (mix(0.07, 0.22, a), mix(0.07, 0.14, a), mix(0.25, 0.38, a));
    a = smooth(0.42, 0.86, v);
    (r, g, b) = (mix(r, 0.52, a), mix(g, 0.32, a), mix(b, 0.58, a));
    a = smooth(0.78, 0.98, v) * (0.55 + 0.45 * west);
    (r, g, b) = (mix(r, 1.0, a), mix(g, 0.62, a), mix(b, 0.48, a));
    let band = (-(y - 74.0).abs() / 4.0).exp() * west * 0.18;
    r += band;
    g += band * 0.66;
    b += band * 0.45;
    // a faint unevenness, so no stretch of sky is one flat tone
    let veil = 0.86 + 0.28 * fbm(x * 0.035, y * 0.09, 3, 0.0);
    r *= veil;
    g *= veil;
    b *= veil;
    // the moon's halo
    let (dx, dy) = (x - f64::from(MOON[0]), y - f64::from(MOON[1]));
    let d = (dx * dx + dy * dy).sqrt();
    let halo = (-d / 5.0).exp() * 0.24 + (-d / 16.0).exp() * 0.07;
    [r + halo * 0.75, g + halo * 0.72, b + halo]
}

/// Low in the west where the glow is, rising behind the pagoda and the town.
fn hill_a(x: f64) -> f64 {
    77.5 - (3.0 + 13.0 * smooth(70.0, 175.0, x)) * (0.3 + 1.1 * fbm(x * 0.022 + 3.0, 1.0, 4, 0.0))
}

fn hill_b(x: f64) -> f64 {
    77.0 - 2.5 * fbm(x * 0.04 + 9.0, 2.0, 4, 0.0)
}

/// Low tiled roofs along the far bank, a few lit.
fn town(x: f64) -> f64 {
    let i = ((x + 3.0) / 11.0).floor();
    let f = (x + 3.0) / 11.0 - i;
    // a hipped roof: a short level ridge, sloping ends, a gap between houses
    let h = 2.0 + hash(i, 5.0) * 2.5;
    let e = (f - 0.5).abs() * 2.0;
    if e > 0.86 {
        SHORE
    } else {
        SHORE - 1.0 - h + (e - 0.35).max(0.0) * 6.0
    }
}

fn bank_x(y: f64) -> f64 {
    38.0 + (y - SHORE) * 2.3 + 4.0 * fbm(y * 0.2, 4.0, 2, 0.0)
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

pub struct KyotoDusk {
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
    const COLS: usize = W;
    const ROWS: usize = H;
    const FPS: u32 = 15;
    const CELL: usize = 1;
    const FIT: Fit = Fit::Cover { anchor: 0.25 };
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

    fn new() -> Self {
        let mut mat = vec![SKY; N];
        let mut rv = vec![0f32; N];
        let mut gv = vec![0f32; N];
        let mut bv = vec![0f32; N];
        let mut floor = vec![0.12f32; N];
        let mut warmth = vec![0f32; N];

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

        // build the static picture
        for r in 0..H {
            for x in 0..W {
                let k = r * W + x;
                let y = r as f64 + 0.5;
                let xc = x as f64 + 0.5;
                let (mut m, mut fl, mut warm) = (SKY, 0.12, 0.0);
                let (mut cr, mut cg, mut cb) = (0.0, 0.0, 0.0);
                if y < SHORE {
                    [cr, cg, cb] = sky_at(xc, y);
                    fl = 0.04;
                    // distant hills in haze, then a nearer ridge
                    let ha = hill_a(xc);
                    if y >= ha {
                        // the Higashiyama hills, flat and violet in the haze
                        m = HILL;
                        let s = sky_at(xc, ha);
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
                    let hb = hill_b(xc);
                    if y >= hb {
                        m = HILL;
                        let tex = fbm(xc * 0.3, y * 0.3, 3, 0.0);
                        (cr, cg, cb) = (0.1 + 0.05 * tex, 0.07 + 0.03 * tex, 0.17 + 0.05 * tex);
                        // mist settling along the water
                        let mist = smooth(hb + 1.0, SHORE, y) * 0.5;
                        (cr, cg, cb) =
                            (mix(cr, 0.7, mist), mix(cg, 0.44, mist), mix(cb, 0.58, mist));
                    }
                    let tw = town(xc);
                    if y >= tw && xc > 70.0 {
                        m = TOWN;
                        (cr, cg, cb) = (0.08, 0.06, 0.14);
                        // the roof ridges catch the sky
                        let tf = (((xc + 3.0) / 11.0) % 1.0 - 0.5).abs() * 2.0;
                        if y < tw + 0.9 && tf < 0.45 {
                            (cr, cg, cb, fl) = (0.5, 0.31, 0.44, 0.06);
                        }
                        // a few lit shoji under the eaves
                        let wi = (xc / 4.0).floor();
                        if y > SHORE - 1.6
                            && y < SHORE - 0.5
                            && (xc % 4.0) < 1.2
                            && hash(wi, 7.0) > 0.74
                        {
                            (cr, cg, cb, fl) = (1.0, 0.66, 0.32, 0.3);
                        }
                    }
                    let p = pagoda(xc - PX, y);
                    if p != 0 {
                        m = PAGODA;
                        let dx = xc - PX;
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
                } else if xc < bank_x(y) {
                    // the near bank where the lantern stands
                    m = BANK;
                    let tex = fbm(xc * 0.3, y * 0.5, 3, 0.0);
                    (cr, cg, cb) = (0.08 + 0.05 * tex, 0.08 + 0.05 * tex, 0.1 + 0.05 * tex);
                    let (xf, rf) = (x as f64, r as f64);
                    // fallen petals on the moss
                    if hash(xf * 7.0, rf * 3.0) > 0.975 - 0.015 * smooth(SHORE + 4.0, H as f64, y) {
                        (cr, cg, cb, fl) = (0.42, 0.22, 0.3, 0.06);
                    }
                    // the stone lip of the pond
                    if xc > bank_x(y) - 2.6 {
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
        for r in 45..H {
            for x in (LX as usize - 14)..=(LX as usize + 14) {
                let y = r as f64 + 0.5;
                let xc = x as f64 + 0.5;
                let l = lantern_at(xc, y);
                if l == 0 {
                    continue;
                }
                let k = r * W + x;
                let py = LB - (LB - y) / LS;
                let pdx = (xc - LX) / LS;
                if l == 2 {
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
                    let edge = if lantern_at(xc, y - 1.0) == 0 {
                        1.0
                    } else {
                        0.0
                    };
                    rv[k] = ((0.05 + 0.22 * edge) * g) as f32;
                    gv[k] = ((0.04 + 0.14 * edge) * g) as f32;
                    bv[k] = ((0.08 + 0.2 * edge) * g) as f32;
                    // the roof's underside and the platform take the flame's light
                    let under = (l == 3 && py > 78.6) || (85.5..86.4).contains(&py);
                    if under {
                        let f = (-(py - 82.8).abs() / 3.0).exp() * 0.75;
                        rv[k] = (f64::from(rv[k]) + f) as f32;
                        gv[k] = (f64::from(gv[k]) + f * 0.5) as f32;
                        bv[k] = (f64::from(bv[k]) + f * 0.18) as f32;
                    }
                    floor[k] = 0.02;
                }
                warmth[k] = if l == 2 { 0.0 } else { 0.75 };
            }
        }

        // the cherry tree
        for r in 0..H {
            for x in 0..130 {
                let k = r * W + x;
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
                    b *= 1.0 - 0.3 * smooth(28.0, 48.0, y);
                    rv[k] = (0.34 + 0.64 * b) as f32;
                    gv[k] = (0.08 + 0.62 * b * b) as f32;
                    bv[k] = (0.2 + 0.5 * b) as f32;
                    floor[k] = 0.1;
                    warmth[k] = 1.0;
                }
            }
        }

        // the lantern's light: how far each cell sits from the lit opening
        let mut glow = vec![0f32; N];
        for r in 0..H {
            for x in 0..W {
                let k = r * W + x;
                let dx = x as f64 + 0.5 - LAMP[0];
                let dy = (r as f64 + 0.5 - LAMP[1]) * 1.1;
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
                    let px = (x as f64 + 0.5 - LX) / 20.0;
                    let py = (r as f64 + 0.5 - LB + 3.0) / 8.0;
                    glow[k] = (f64::from(glow[k]) + (-(px * px + py * py)).exp() * 0.6) as f32;
                }
            }
        }

        // a few faint stars in the indigo
        let mut stars = Vec::new();
        for i in 0..400 {
            let i = f64::from(i);
            let x = (hash(i, 91.0) * W as f64).floor() as usize;
            let r = (hash(i, 92.0) * 40.0).floor() as usize;
            let k = r * W + x;
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
                let (xf, y) = (x as f64, r as f64 + 0.5);
                let d = cdens(xf, y);
                ccov[(r - C0) * CW + x] = smooth(0.56, 0.7, d) as f32;
                clit[(r - C0) * CW + x] = clamp(0.5 + (d - cdens(xf - 1.0, y + 2.2)) * 7.0) as f32;
            }
        }

        // petals: most of them near the tree, thinning out downwind
        let petals = (0..50)
            .map(|i| {
                let i = f64::from(i);
                Petal {
                    x: hash(i, 101.0).powf(2.2) * 140.0,
                    y: hash(i, 102.0) * (H as f64 + 20.0),
                    vx: 3.0 + hash(i, 103.0) * 4.0,
                    vy: 1.6 + hash(i, 104.0) * 2.2,
                    sw: 1.0 + hash(i, 105.0) * 2.0,
                    sf: 0.6 + hash(i, 106.0) * 1.2,
                    ph: hash(i, 107.0) * 6.28,
                    tum: 2.0 + hash(i, 108.0) * 4.0,
                }
            })
            .collect();
        let floaters = (0..26)
            .map(|i| {
                let i = f64::from(i);
                [
                    hash(i, 111.0) * W as f64,
                    SHORE + 3.0 + hash(i, 112.0) * 20.0,
                    0.3 + hash(i, 113.0) * 0.5,
                ]
            })
            .collect();

        let dith = (0..N)
            .map(|k| {
                (BAYER[((k / W) & 3) * 4 + ((k % W) & 3)] * 0.4
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
            pr: vec![0f32; N],
            pg: vec![0f32; N],
            pb: vec![0f32; N],
            pmask: vec![false; N],
            moon_ink,
            star_dim,
            star_bright,
            floater_ink,
        }
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        let (wf, hf, cw) = (W as f64, H as f64, CW as f64);
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
            let k = r as usize * W + x as usize;
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

        for r in 0..H {
            let y = r as f64 + 0.5;
            for x in 0..W {
                let k = r * W + x;
                let m = self.mat[k];
                let xf = x as f64;
                let (mut cr, mut cg, mut cb);
                let mut fl = f64::from(self.floor[k]);
                let mut fade = 1.0;
                if m == POND {
                    // the far side, upside down, shaken by small ripples
                    let depth = (y - SHORE) / (hf - SHORE);
                    let wob = (y * 1.7 - t * 2.0 + (xf * 0.13 + t * 0.6).sin() * 1.4).sin()
                        * (0.05 + 0.8 * depth);
                    let sx = js_round(xf + wob).clamp(0.0, wf - 1.0);
                    let w = noise(xf * 0.14 + t * 0.2, y * 1.1 - t * 0.8, 0.0);
                    // ripples also tip the image up and down, so level lines break
                    // (drawn a little stretched, so the pagoda's lower roofs land in the pond)
                    let sr =
                        js_round(SHORE - 1.0 - (r as f64 - SHORE) * 1.35 + (w - 0.5) * 3.0 * depth)
                            .clamp(0.0, SHORE - 1.0);
                    let j = sr as usize * W + sx as usize;
                    // crisp and bright right under the bank, darker further out
                    let lit = mix(0.95, 0.6, smooth(SHORE + 1.0, SHORE + 6.0, y)) * (0.8 + 0.4 * w);
                    cr = f64::from(self.rr[j]) * lit * 0.9;
                    cg = f64::from(self.rg[j]) * lit * 0.92;
                    cb = f64::from(self.rb[j]) * lit + 0.03;
                    let glint = smooth(0.74, 0.95, w) * 0.06;
                    cr += glint;
                    cg += glint * 0.8;
                    cb += glint;
                    // the lantern's light laid on the water as a broken warm streak
                    let sl = (-(xf + 0.5 - LX - 9.0 - (y - SHORE) * 0.25).abs()
                        / (3.5 + depth * 3.0))
                        .exp()
                        * smooth(0.45, 0.8, w)
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
                    if m == SKY && (C0..C1).contains(&r) {
                        let sx = xf + cdrift;
                        let ixf = sx.floor();
                        let fx = sx - ixf;
                        let ix = ixf as usize;
                        let i0 = (r - C0) * CW + ix % CW;
                        let i1 = (r - C0) * CW + (ix + 1) % CW;
                        let (c0, c1) = (f64::from(self.ccov[i0]), f64::from(self.ccov[i1]));
                        let c = c0 + (c1 - c0) * fx;
                        if c > 0.01 {
                            let (l0, l1) = (f64::from(self.clit[i0]), f64::from(self.clit[i1]));
                            let l = l0 + (l1 - l0) * fx;
                            // brighter undersides toward the west and the horizon
                            let sun = l
                                * (0.45 + 0.55 * smooth(C0 as f64, C1 as f64, y))
                                * (0.6 + 0.4 * (-(xf - 80.0).abs() / 90.0).exp());
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
        for r in MOON[1] - 6..=MOON[1] + 6 {
            for x in MOON[0] - 6..=MOON[0] + 6 {
                let dx = f64::from(x) + 0.5 - f64::from(MOON[0]);
                let dy = f64::from(r) + 0.5 - f64::from(MOON[1]);
                let inside = dx * dx + dy * dy < 5.2 * 5.2;
                let (ex, ey) = (dx - 2.2, dy + 1.6); // the shadowed disc, offset up and right
                if inside && ex * ex + ey * ey > 4.9 * 4.9 {
                    out[r as usize * W + x as usize] = self.moon_ink;
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
            // x can round up to W, which upstream's flat index wraps onto
            // the next row's first cell
            let k = r as usize * W + x as usize;
            if k >= N || self.mat[k] != POND {
                continue;
            }
            out[k] = self.floater_ink;
        }
    }
}
