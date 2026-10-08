//! misty forest: morning in a pine forest. Ridge after ridge of pines recedes
//! into fog, each paler than the one in front, with mist lying in sheets in the
//! valleys between them. A low sun sits behind the farthest trees and sends
//! beams slanting down through the fog to a clearing on the forest floor. The
//! fog drifts, the beams shimmer, and motes of dust float in the light.
//!
//! Upstream's `Float32Array`s stay `f32` here: their rounding is part of the
//! picture.
//!
//! `misty-forest-wide` is the same forest recomposed for a 3.2:1 panel: the sun
//! and its clearing keep the right third, the ridges and fog run on west, and a
//! young pine stands in front between the two framing giants.

use std::f64::consts::PI;

use super::halftone::{bayer, Dots};
use super::math::{clamp, fbm, hash, mix, noise, smooth};
use super::{Fit, Piece, hex};
use crate::grid::Cell;

const H: usize = 100;
const SUN_R: f64 = 3.4;
const SKY: i8 = -1;
const FLOOR: i8 = 5;
const GIANT: i8 = 6;

/// The ridges, far to near: [ridge row, its rise and fall, tree spacing, tree
/// heights from and to, haze, how fast its fog drifts, mist lying in the
/// valley below it, drifting fog in front of it, sunbeam in front of it]
const LAYERS: [[f64; 10]; 5] = [
    [50.0, 6.0, 1.6, 1.0, 2.5, 0.48, 0.8, 1.0, 0.14, 0.6],
    [59.0, 6.0, 2.0, 3.0, 5.5, 0.3, 1.3, 0.8, 0.16, 0.8],
    [69.0, 7.0, 2.6, 4.0, 7.0, 0.13, 2.0, 0.66, 0.18, 0.9],
    [80.0, 6.0, 3.4, 5.0, 9.0, 0.03, 2.8, 0.44, 0.14, 1.0],
    [90.0, 1.5, 10.0, 9.0, 24.0, 0.03, 3.4, 0.0, 0.08, 0.6],
];
const NEAR: usize = LAYERS.len() - 1;

/// Fog and cloud field width: both wrap at this many columns.
const FW: usize = 400;
const CH: usize = 46;
/// Angular bins round the sun.
const RA: usize = 720;

/// Sunbeams: a handful of wide shafts through the gaps, [angle, half width,
/// strength].
const SHAFTS: [[f64; 3]; 7] = [
    [1.42, 0.05, 0.7],
    [1.66, 0.07, 1.0],
    [1.93, 0.05, 0.8],
    [2.18, 0.08, 1.0],
    [2.45, 0.05, 0.75],
    [2.7, 0.06, 0.9],
    [2.95, 0.04, 0.6],
];

// the colour of the fog: cool and green-grey, warming to cream by the sun
fn fog_r(s: f64) -> f64 {
    mix(0.7, 1.05, s)
}
fn fog_g(s: f64) -> f64 {
    mix(0.92, 0.92, s)
}
fn fog_b(s: f64) -> f64 {
    mix(0.9, 0.6, s)
}

/// Where things sit in the frame: upstream's, or the `-wide` recomposition's.
struct Layout {
    name: &'static str,
    w: usize,
    sun: [f64; 2],
    /// The columns where the nearest trees leave a clearing under the sun.
    clearing: [f64; 2],
    /// The pines in front: [column, tip row, spread, tier height].
    giants: &'static [[f64; 4]],
}

const ORIGINAL: Layout = Layout {
    name: "misty-forest",
    w: 200,
    sun: [146.0, 45.5],
    clearing: [92.0, 140.0],
    giants: &[[9.0, -12.0, 15.0, 7.0], [192.0, 12.0, 7.0, 5.0]],
};

const WIDE: Layout = Layout {
    name: "misty-forest-wide",
    w: 320,
    sun: [233.0, 45.5],
    clearing: [179.0, 227.0],
    giants: &[
        [9.0, -12.0, 15.0, 7.0],
        [104.0, 34.0, 7.0, 5.0],
        [312.0, 12.0, 7.0, 5.0],
    ],
};

pub type MistyForest = Scene<false>;
pub type MistyForestWide = Scene<true>;

pub struct Scene<const IS_WIDE: bool> {
    dots: Dots,
    sr: Vec<f32>,
    sg: Vec<f32>,
    sb: Vec<f32>,
    fog_amt: Vec<f32>,
    fog_speed: Vec<f32>,
    ray_amt: Vec<f32>,
    sun_s: Vec<f32>,
    floor_lit: Vec<f32>,
    lift: Vec<f32>,
    cloud_amt: Vec<f32>,
    abin: Vec<f32>,
    dist: Vec<f32>,
    fog: Vec<f32>,
    cloud: Vec<f32>,
    ray_a: Vec<f32>,
    ray_b: Vec<f32>,
    /// Dust in the air: [x, y, drift speed, bob phase, size].
    motes: Vec<[f64; 5]>,
    mote: Vec<f32>,
    mote_cells: Vec<usize>,
}

impl<const IS_WIDE: bool> Scene<IS_WIDE> {
    const L: Layout = if IS_WIDE { WIDE } else { ORIGINAL };
    const W: usize = Self::L.w;
}

impl<const IS_WIDE: bool> Piece for Scene<IS_WIDE> {
    const NAME: &'static str = Self::L.name;
    const COLS: usize = Self::W;
    const ROWS: usize = H;
    const FPS: u32 = 15;
    const CELL: usize = 1;
    const FIT: Fit = Fit::Cover { anchor: 0.5 };
    const GROUND: u32 = hex("#090f0e");
    #[rustfmt::skip]
    const PALETTE: &'static [u32] = &[
        // sunlight and the warm fog around it
        hex("#fffbea"), hex("#fff0c8"), hex("#fbe2a6"), hex("#f2c97e"), hex("#e0a95e"), hex("#b9834a"),
        // cool fog, pale to sea-green
        hex("#e4ebe6"), hex("#cbe0dc"), hex("#a8d2d0"), hex("#86c0c2"), hex("#68a7ac"), hex("#4f8c93"), hex("#3b7078"),
        // pines, from the haze to the dark in front of us
        hex("#557570"), hex("#456761"), hex("#365952"), hex("#2a4a44"), hex("#1f3c37"), hex("#172f2b"), hex("#11231f"),
        // moss and bark where the light touches the floor
        hex("#a4a35a"), hex("#8a8f4c"), hex("#6d7440"), hex("#4d5530"), hex("#7a5a3a"), hex("#5b4532"),
        // the clear sky above the fog, deepening overhead
        hex("#a9c3cc"), hex("#8eadb8"), hex("#6f93a2"), hex("#53798a"), hex("#3d6172"), hex("#2b4a5a"),
    ];

    fn new() -> Self {
        let n = Self::W * H;
        let hf = H as f64;

        // --- the layers, rasterised far to near so nearer ones cover farther
        let mut layer = vec![SKY; n];
        let mut edge = vec![0f32; n]; // how much a cell sits on a silhouette's sunward rim
        let mut below = vec![0f32; n]; // rows below the layer's tree line
        let mut tex = vec![0f32; n]; // the lower edge of a tier of boughs, near trees only
        let mut ground = vec![0f32; n]; // the row a cell's ridge rises from, for its valley mist
        for (i, &[ly, amp, gap, h0, h1, ..]) in LAYERS.iter().enumerate() {
            let fi = i as f64;
            // nearer ridges dip toward the sun, a valley opening onto the light
            let ridge = |x: f64| {
                ly + amp
                    * (fbm(x * (0.018 + fi * 0.003), fi * 13.0 + 2.0, 3, 0.0) - 0.5)
                    * (if i < 3 { 4.0 } else { 3.0 })
                    + if i > 0 && i < NEAR {
                        (2.0 + fi * 2.0) * (-((x - Self::L.sun[0] - 4.0) / 38.0).powi(2)).exp()
                    } else {
                        0.0
                    }
            };
            let base: Vec<f32> = (0..Self::W).map(|x| ridge(x as f64) as f32).collect();
            let mut fill = vec![0u8; n];
            let mut spire = vec![0u8; n];
            for x in 0..Self::W {
                let r0 = ridge(x as f64).floor().max(0.0) as usize;
                for r in r0..H {
                    fill[r * Self::W + x] = 1;
                }
            }
            // pines: a spire of tiers, each tier flaring out and stepping back in
            let mut tx = -2.0 + hash(fi, 1.0) * gap;
            while tx < (Self::W + 2) as f64 {
                // the nearest trees leave a clearing under the sun for the light to land in
                if !(i == NEAR && tx > Self::L.clearing[0] && tx < Self::L.clearing[1]) {
                    let th = h0 + (h1 - h0) * hash(tx * 7.0, fi + 5.0);
                    let tip = ridge(tx) - th;
                    let tier = 2.0 + th * 0.12;
                    let r_end = hf.min(ridge(tx) + 2.0);
                    let mut r = tip.floor().max(0.0) as i64;
                    while (r as f64) < r_end {
                        let d = r as f64 + 0.5 - tip;
                        if d >= 0.0 {
                            let saw = (d % tier) / tier;
                            let half = d * 0.3 * (0.6 + 0.5 * saw) + 0.35;
                            let x0 = (tx - half).floor().max(0.0) as i64;
                            let x1 = ((Self::W - 1) as f64).min((tx + half).ceil()) as i64;
                            for x in x0..=x1 {
                                let dx = (x as f64 + 0.5 - tx).abs();
                                if dx > half {
                                    continue;
                                }
                                let k = r as usize * Self::W + x as usize;
                                fill[k] = 1;
                                spire[k] = 1;
                                if i == NEAR {
                                    tex[k] = (smooth(0.62, 0.95, saw)
                                        * smooth(0.3, 0.85, dx / half)
                                        * (0.55 + 0.45 * hash(x as f64, r as f64 * 5.0)))
                                        as f32;
                                }
                            }
                        }
                        r += 1;
                    }
                }
                tx += gap * (0.7 + hash(tx, fi + 3.0) * 0.7);
            }
            // where the silhouette starts in each column, smoothed a little so the
            // mist line follows the forest rather than every single spire
            let mut top = vec![H as f32; Self::W];
            for (x, t) in top.iter_mut().enumerate() {
                if let Some(r) = (0..H).find(|&r| fill[r * Self::W + x] != 0) {
                    *t = r as f32;
                }
            }
            let line: Vec<f32> = (0..Self::W)
                .map(|x| {
                    let mut s = 0.0;
                    for d in -3i64..=3 {
                        s += f64::from(top[(x as i64 + d).clamp(0, Self::W as i64 - 1) as usize]);
                    }
                    (s / 7.0).max(f64::from(top[x])) as f32
                })
                .collect();
            let open = |xx: i64, rr: i64| {
                xx >= 0 && xx < Self::W as i64 && (rr < 0 || fill[rr as usize * Self::W + xx as usize] == 0)
            };
            for r in 0..H {
                for x in 0..Self::W {
                    let k = r * Self::W + x;
                    if fill[k] == 0 {
                        continue;
                    }
                    layer[k] = if i == NEAR && spire[k] == 0 {
                        FLOOR
                    } else {
                        i as i8
                    };
                    if i != NEAR {
                        tex[k] = 0.0;
                    }
                    below[k] = (r as f64 + 0.5 - f64::from(line[x])) as f32;
                    ground[k] = base[x];
                    // a rim where the sky (or a farther layer) shows beside or above
                    let toward = if (x as f64) < Self::L.sun[0] { 1 } else { -1 };
                    let (xi, ri) = (x as i64, r as i64);
                    edge[k] = if open(xi + toward, ri) || open(xi, ri - 1) {
                        1.0
                    } else if open(xi + 2 * toward, ri) {
                        0.5
                    } else {
                        0.0
                    };
                }
            }
        }

        // --- what stands in front: a giant pine cut by the left of the frame, and a
        // smaller one at the right edge, so the frame is not a symmetric curtain
        // (wide: and a young one between, off-centre, so the middle is not empty)
        let mut giant = vec![0u8; n];
        for &[gx, tip, spread, tier] in Self::L.giants {
            for r in tip.floor().max(0.0) as usize..H {
                let d = r as f64 + 0.5 - tip;
                let saw = (d % tier) / tier;
                // each tier of boughs sweeps out and droops, so the outline is a stack
                // of points rather than a straight edge where it meets the frame
                let half = (spread * (0.5 + 0.5 * saw)).min(d * 0.34 * (0.45 + 0.7 * saw) + 0.5);
                let x0 = (gx - half).floor().max(0.0) as i64;
                let x1 = ((Self::W - 1) as f64).min((gx + half).ceil()) as i64;
                for x in x0..=x1 {
                    let xf = x as f64;
                    let dx = (xf + 0.5 - gx).abs();
                    // the side toward the frame stays full, so no sliver of sky shows there
                    // (the young pine stands clear of the frame, so both its sides are ragged)
                    let framed = gx < 20.0 || gx > (Self::W - 20) as f64;
                    let outer = if framed && (xf + 0.5 - gx) * (gx - (Self::W / 2) as f64) > 0.0 {
                        1.6
                    } else {
                        1.0
                    };
                    let ragged = half * outer * (0.82 + 0.3 * noise(xf * 0.5, r as f64 * 0.4, 0.0));
                    if dx <= ragged || dx < 0.9 {
                        let k = r * Self::W + x as usize;
                        giant[k] = 1;
                        tex[k] = (smooth(0.66, 0.96, saw)
                            * smooth(0.25, 0.8, dx / ragged)
                            * (0.5 + 0.5 * hash(xf * 3.0, r as f64)))
                            as f32;
                    }
                }
            }
        }
        for k in 0..n {
            if giant[k] != 0 {
                layer[k] = GIANT;
                below[k] = 0.0;
            }
        }
        // a rim two cells deep on the side that faces the sun
        {
            let open = |x: i64, r: i64| {
                x >= 0 && x < Self::W as i64 && (r < 0 || giant[r as usize * Self::W + x as usize] == 0)
            };
            for r in 0..H {
                for x in 0..Self::W {
                    let k = r * Self::W + x;
                    if giant[k] == 0 {
                        continue;
                    }
                    let tx = if (x as f64) < Self::L.sun[0] { 1 } else { -1 };
                    let (xi, ri) = (x as i64, r as i64);
                    edge[k] = if open(xi + tx, ri) || open(xi, ri - 1) {
                        1.0
                    } else if open(xi + 2 * tx, ri) {
                        0.6
                    } else {
                        0.0
                    };
                }
            }
        }

        // --- static colour
        let mut sr = vec![0f32; n];
        let mut sg = vec![0f32; n];
        let mut sb = vec![0f32; n];
        let mut fog_amt = vec![0f32; n]; // how much drifting fog shows in front of a cell
        let mut fog_speed = vec![0f32; n];
        let mut ray_amt = vec![0f32; n]; // how much of a sunbeam the air in front of it holds
        let mut sun_s = vec![0f32; n]; // nearness to the sun, for the fog's warmth
        let mut floor_lit = vec![0f32; n]; // where a beam that reaches the floor lights it
        let mut lift = vec![0f32; n]; // the dot floor, so the darkest air still shows
        let mut cloud_amt = vec![0f32; n]; // where high cloud can show, in the sky only
        let mut abin = vec![0f32; n];
        let mut dist = vec![0f32; n];
        let ra = RA as f64;
        for r in 0..H {
            for xi in 0..Self::W {
                let k = r * Self::W + xi;
                let x = xi as f64;
                let y = r as f64 + 0.5;
                let l = layer[k];
                let (dx, dy) = (x + 0.5 - Self::L.sun[0], y - Self::L.sun[1]);
                let ang = dy.atan2(dx);
                abin[k] = (((ang / (PI * 2.0)) * ra + ra) % ra) as f32;
                let d = (dx * dx + dy * dy).sqrt();
                dist[k] = d as f32;
                let s = (-(dx * dx + dy * dy * 1.96).sqrt() / 30.0).exp();
                sun_s[k] = s as f32;
                // a soft warm bloom over a wide radius, in the air and the fog
                let bloom = (-d / 16.0).exp() * 0.25 + (-d / 6.0).exp() * 0.1;
                let (mut cr, mut cg, mut cb, mut ray);
                if l == SKY {
                    // sky: deep overhead, paling to the fog band low down, warm by the sun
                    let v = clamp(y / 52.0);
                    let p = v.powf(1.6);
                    let veil = 0.95 + 0.1 * fbm(x * 0.04, y * 0.08, 3, 0.0);
                    cr = mix(0.065, 0.36, p) * veil;
                    cg = mix(0.14, 0.58, p) * veil;
                    cb = mix(0.17, 0.62, p) * veil;
                    // the fog band the far ridge stands in
                    let band = 0.75
                        * smooth(28.0, 50.0, y)
                        * (0.55 + 0.75 * fbm(x * 0.025, y * 0.16, 3, 0.0));
                    cr = mix(cr, fog_r(0.0), band);
                    cg = mix(cg, fog_g(0.0), band);
                    cb = mix(cb, fog_b(0.0), band);
                    // warm in hue round the sun, but falling off in brightness, so the
                    // disc stands in a halo rather than a flat blaze
                    let (kw, kb) = (s * 0.55, 0.5 + 0.5 * (-d / 9.0).exp());
                    cr = mix(cr, fog_r(s) * kb, kw);
                    cg = mix(cg, fog_g(s) * kb, kw);
                    cb = mix(cb, fog_b(s) * kb, kw);
                    let glow = (-d / 3.0).exp() * 0.3 + (-d / 10.0).exp() * 0.18;
                    cr += glow + bloom;
                    cg += glow * 0.9 + bloom * 0.78;
                    cb += glow * 0.66 + bloom * 0.45;
                    // the disc itself, a little brighter in the middle
                    let disc = smooth(SUN_R + 0.6, SUN_R - 0.4, d);
                    cr = mix(cr, 1.3, disc);
                    cg = mix(cg, 1.24, disc);
                    cb = mix(cb, 1.08, disc);
                    fog_amt[k] = (0.3 * smooth(30.0, 50.0, y)) as f32;
                    fog_speed[k] = 0.4;
                    cloud_amt[k] = (0.75
                        * smooth(5.0, 15.0, y)
                        * smooth(44.0, 28.0, y)
                        * smooth(SUN_R + 2.0, SUN_R + 8.0, d))
                        as f32;
                    ray = 0.35 * smooth(26.0, 46.0, y);
                    lift[k] = 0.04;
                } else if l < NEAR as i8 {
                    let li = l as usize;
                    let [_, _, _, _, _, haze, speed, m, drift, beam] = LAYERS[li];
                    let lf = l as f64;
                    // pine: dark teal, paling with distance; mist pooled just under the
                    // tree line, and a sheet of it lying along the valley floor below
                    let bk = f64::from(below[k]);
                    let pool = smooth(2.5, 8.0 + lf * 1.5, bk) * smooth(20.0, 11.0, bk);
                    // the sheet follows the lie of the land, smoothly, so it never streaks
                    let sheet = (-((y - (f64::from(ground[k]) + 5.0)) / 3.0).powi(2)).exp()
                        * (0.75 + 0.5 * fbm(x * 0.035, lf * 7.3, 3, 0.0));
                    let mist = clamp((pool * 0.5).max(sheet) * m);
                    let h = clamp(haze + (1.0 - haze) * mist);
                    let veil = 0.9 + 0.2 * fbm(x * 0.06, y * 0.12, 3, 0.0);
                    let (pr, pg, pb) = (0.02, 0.075, 0.07);
                    cr = mix(pr, fog_r(s) * veil, h);
                    cg = mix(pg, fog_g(s) * veil, h);
                    cb = mix(pb, fog_b(s) * veil, h);
                    cr += bloom * h;
                    cg += bloom * 0.78 * h;
                    cb += bloom * 0.45 * h;
                    let rim = f64::from(edge[k]) * s * (0.2 + 0.6 * (1.0 - haze));
                    cr += rim * 0.95;
                    cg += rim * 0.8;
                    cb += rim * 0.45;
                    fog_amt[k] = drift as f32;
                    fog_speed[k] = speed as f32;
                    // the trees stop most of the light; the beams show in the mist between
                    // (far off there is more air in front of them to hold the light)
                    ray = beam
                        * if li < 3 {
                            0.45 + 0.55 * clamp(mist / m)
                        } else {
                            0.25 + 0.75 * clamp(mist / m)
                        };
                    lift[k] = 0.03;
                } else if l == NEAR as i8 {
                    // the nearest pines: near-black, a faint teal on each tier's lower edge
                    let g = f64::from(tex[k]) * 0.12;
                    // (the body itself stays black, or the dither would screen it evenly)
                    cr = 0.001 + g * 0.35;
                    cg = 0.004 + g;
                    cb = 0.004 + g * 0.9;
                    let rim = f64::from(edge[k]) * (0.12 + 0.6 * s);
                    cr += rim * 0.9;
                    cg += rim * 0.7;
                    cb += rim * 0.38;
                    fog_amt[k] = LAYERS[NEAR][8] as f32;
                    fog_speed[k] = LAYERS[NEAR][6] as f32;
                    ray = 0.12;
                    lift[k] = 0.0;
                } else if l == FLOOR {
                    // the forest floor: moss in clumps, sparse, fading out toward the frame
                    let clump = smooth(0.42, 0.72, fbm(x * 0.09, y * 0.4, 3, 0.0));
                    let speck = if hash(x * 7.0, r as f64 * 11.0) > 0.55 {
                        1.0
                    } else {
                        0.35
                    };
                    let m = (0.06 + 0.3 * clump) * speck * smooth(hf + 2.0, hf - 8.0, y);
                    cr = 0.02 + m * 0.6;
                    cg = 0.03 + m * 0.62;
                    cb = 0.02 + m * 0.3;
                    fog_amt[k] = 0.06;
                    fog_speed[k] = 3.4;
                    ray = 0.0;
                    // dappled patches the beams can land on
                    floor_lit[k] = (smooth(0.32, 0.56, fbm(x * 0.06 + 3.0, y * 0.24, 3, 0.0))
                        * smooth(hf + 3.0, hf - 6.0, y)) as f32;
                    lift[k] = 0.0;
                } else {
                    // the giants: near-black needles, the tiers drawn by a faint teal edge,
                    // a warm rim two cells deep toward the light
                    let g = f64::from(tex[k]) * 0.15;
                    cr = 0.001 + g * 0.35;
                    cg = 0.004 + g;
                    cb = 0.004 + g * 0.9;
                    // warm on the side near the sun, a cool fog-lit edge far from it
                    let warm = (-d / 50.0).exp();
                    let rim = f64::from(edge[k]) * (0.13 + 0.55 * warm);
                    cr += rim * mix(0.4, 0.95, warm);
                    cg += rim * mix(0.75, 0.66, warm);
                    cb += rim * mix(0.72, 0.32, warm);
                    fog_amt[k] = 0.04;
                    fog_speed[k] = 4.0;
                    ray = 0.04;
                    lift[k] = 0.0;
                }
                // the beams fan out mostly down and to the west, the way the gaps face
                ray *= 0.2 + 0.8 * smooth(1.25, 1.75, ang) * smooth(3.1, 2.6, ang);
                ray_amt[k] = ray as f32;
                sr[k] = cr as f32;
                sg[k] = cg as f32;
                sb[k] = cb as f32;
            }
        }

        // drifting fog banks: wide soft noise that wraps so it can slide forever
        let fw = FW as f64;
        let mut fog = vec![0f32; FW * H];
        for r in 0..H {
            for u in 0..FW {
                fog[r * FW + u] = smooth(
                    0.42,
                    0.75,
                    fbm(u as f64 * 0.022, r as f64 * 0.09, 4, fw * 0.022),
                ) as f32;
            }
        }

        // thin high cloud, long and flat, lit from below by the low sun
        let mut cloud = vec![0f32; FW * CH];
        for r in 0..CH {
            for u in 0..FW {
                let (uf, rf) = (u as f64, r as f64);
                let q = fbm(uf * 0.01, rf * 0.05, 2, fw * 0.01);
                cloud[r * FW + u] = smooth(
                    0.44,
                    0.68,
                    fbm(uf * 0.016 + q * 1.5, rf * 0.17, 4, fw * 0.016),
                ) as f32;
            }
        }

        // a faint grain along each shaft, and a slower pattern sliding across
        // them so they brighten and fade
        let mut ray_a = vec![0f32; RA];
        let mut ray_b = vec![0f32; RA];
        for i in 0..RA {
            let fi = i as f64;
            let a = (fi / ra) * PI * 2.0;
            let mut v: f64 = 0.0;
            for [c, w, st] in SHAFTS {
                v = v.max(st * smooth(w, w * 0.35, (a - c).abs()));
            }
            ray_a[i] = (v * (0.8 + 0.2 * fbm(fi * 0.4, 3.1, 2, ra * 0.4))) as f32;
            ray_b[i] = smooth(0.4, 0.7, fbm(fi * 0.03, 8.7, 2, ra * 0.03)) as f32;
        }

        let motes = (0..110)
            .map(|i| {
                let i = f64::from(i);
                [
                    hash(i, 1.0) * Self::W as f64,
                    50.0 + hash(i, 2.0) * 44.0,
                    0.3 + hash(i, 3.0) * 0.8,
                    hash(i, 4.0) * 6.28,
                    hash(i, 5.0),
                ]
            })
            .collect::<Vec<_>>();

        Self {
            dots: Dots::new(Self::PALETTE),
            sr,
            sg,
            sb,
            fog_amt,
            fog_speed,
            ray_amt,
            sun_s,
            floor_lit,
            lift,
            cloud_amt,
            abin,
            dist,
            fog,
            cloud,
            ray_a,
            ray_b,
            mote_cells: Vec::with_capacity(motes.len()),
            motes,
            mote: vec![0f32; n],
        }
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        let (wf, hf) = (Self::W as f64, H as f64);
        for &k in &self.mote_cells {
            self.mote[k] = 0.0;
        }
        self.mote_cells.clear();
        for &[mx, my, sp, ph, sz] in &self.motes {
            let x = (((mx + t * sp + 2.5 * (t * 0.4 + ph).sin()) % wf) + wf) % wf;
            let y = (my + 2.0 * (t * 0.3 + ph * 1.7).sin() - ((t * sp * 0.2) % 6.0)).floor();
            if y < 0.0 || y >= hf {
                continue;
            }
            let k = y as usize * Self::W + x.floor() as usize;
            self.mote[k] = (0.5 + 0.5 * sz) as f32;
            self.mote_cells.push(k);
        }
        // the beams hold their places (the gaps in the trees do not move) and
        // only sway a hair; a second, slower pattern drifts across them
        let shift_a = 2.0 * (t * 0.35).sin();
        let shift_b = -t * 1.6;
        let pulse = 0.88 + 0.12 * (t * 0.7).sin();
        let ray_at =
            |v: &[f32], a: f64| f64::from(v[(a.floor() as i64).rem_euclid(RA as i64) as usize]);

        for r in 0..H {
            for xi in 0..Self::W {
                let k = r * Self::W + xi;
                let x = xi as f64;
                let (mut cr, mut cg, mut cb) = (
                    f64::from(self.sr[k]),
                    f64::from(self.sg[k]),
                    f64::from(self.sb[k]),
                );
                let s = f64::from(self.sun_s[k]);
                let dist = f64::from(self.dist[k]);

                // the fog banks drift, nearer ones faster
                let fa = f64::from(self.fog_amt[k]);
                if fa > 0.01 {
                    let u = x + t * f64::from(self.fog_speed[k]);
                    let ui = u.floor();
                    let uf = u - ui;
                    let ui = ui as usize;
                    let f0 = f64::from(self.fog[r * FW + ui % FW]);
                    let f1 = f64::from(self.fog[r * FW + (ui + 1) % FW]);
                    let a = (f0 + (f1 - f0) * uf) * fa;
                    cr = mix(cr, fog_r(s), a);
                    cg = mix(cg, fog_g(s), a);
                    cb = mix(cb, fog_b(s), a);
                }

                let ca = f64::from(self.cloud_amt[k]);
                if ca > 0.01 {
                    let u = x + t * 0.6;
                    let ui = u.floor();
                    let uf = u - ui;
                    let ui = ui as usize;
                    let c0 = f64::from(self.cloud[r * FW + ui % FW]);
                    let c1 = f64::from(self.cloud[r * FW + (ui + 1) % FW]);
                    let a = (c0 + (c1 - c0) * uf) * ca;
                    if a > 0.005 {
                        // cool grey-teal, warming to gold on the undersides near the sun
                        let w = (-dist / 40.0).exp();
                        cr = mix(cr, mix(0.26, 0.95, w), a);
                        cg = mix(cg, mix(0.38, 0.72, w), a);
                        cb = mix(cb, mix(0.42, 0.45, w), a);
                    }
                }

                // the beams: brightest near the sun, fading with distance
                let (ra, fl) = (f64::from(self.ray_amt[k]), f64::from(self.floor_lit[k]));
                let ai = f64::from(self.abin[k]);
                let mut beam = 0.0;
                if ra > 0.01 || fl > 0.01 {
                    beam = ray_at(&self.ray_a, ai + shift_a)
                        * (0.6 + 0.4 * ray_at(&self.ray_b, ai + shift_b))
                        * pulse;
                }
                if ra > 0.01 && dist > SUN_R {
                    // lit shafts brighten the air, the shadows between them dim it
                    let fall = (-dist / 80.0).exp() * smooth(SUN_R, 12.0, dist);
                    let b = (beam - 0.3) * fall * ra * 2.2;
                    if b > 0.0 {
                        cr += b * 1.05;
                        cg += b * 0.8;
                        cb += b * 0.42;
                    } else {
                        let dim = 1.0 + b;
                        cr *= dim;
                        cg *= dim;
                        cb *= dim;
                    }
                }
                if fl > 0.01 {
                    // a patch of sun on the moss where a beam lands
                    let b = beam * fl * 1.1;
                    cr += b * 1.0;
                    cg += b * 0.78;
                    cb += b * 0.4;
                }

                let mut floor = f64::from(self.lift[k]);
                let mote = f64::from(self.mote[k]);
                if mote != 0.0 && dist > 6.0 {
                    // a mote shows up where a beam catches it
                    let lit = ray_at(&self.ray_a, ai + shift_a) * (-dist / 90.0).exp();
                    let v = mote * (0.1 + 1.6 * lit) * (ra * 2.0).min(1.0);
                    if v > 0.18 {
                        cr = cr.max(v * 1.05);
                        cg = cg.max(v * 0.97);
                        cb = cb.max(v * 0.7);
                        floor = 0.3;
                    }
                }

                let peak = cr.max(cg).max(cb).max(1e-4);
                let level = clamp(floor + (1.0 - floor) * peak.powf(0.9) * 0.95);
                let step = Dots::step(level, bayer(r, xi));
                out[k] = self.dots.ink(step, level, [cr, cg, cb], peak);
            }
        }
    }
}
