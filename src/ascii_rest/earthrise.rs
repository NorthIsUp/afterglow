//! earthrise: the Earth coming up over the lunar horizon. The sun is low on
//! the right, so every crater rim and boulder throws a long black shadow
//! across the grey ground, and the same light makes a gibbous Earth with a
//! clean line between day and night. The Earth turns, its clouds drift, it
//! climbs very slowly, and a few bright stars breathe.
//!
//! The ground is a heightfield of craters, rendered once column by column
//! from a camera standing on it, with real shadows marched toward the sun.
//! Each frame only the Earth and the few stars that twinkle are shaded again.
//!
//! Upstream's dot takes a `cap` on the colour boost, so the tail is inline
//! rather than [`Dots::ink`]. `Math.pow` with an integer exponent is
//! repeated squaring under JavaScriptCore, which is `powi`.

use std::f64::consts::PI;

use super::halftone::{bayer, Dots};
use super::math::{clamp, fbm, hash, js_hypot, js_round, mix, noise, smooth, unit};
use super::{Fit, Piece, hex};
use crate::grid::Cell;

const W: usize = 200;
const H: usize = 100;
const N: usize = W * H;
/// Screen row of eye level; the horizon dips below it.
const EYE: f64 = 37.0;
/// Focal length in cells.
const F: f64 = 112.0;
const CAM_H: f64 = 20.0;
/// The moon's radius, in ground units, for the falling horizon.
const RM: f64 = 1500.0;
const ZMAX: f64 = 520.0;
/// The Earth on screen, its lower edge still behind the horizon.
const EC: [f64; 2] = [141.0, 32.0];
const ER: f64 = 22.0;
/// Which face of the globe is turned to us at the start.
const LON0: f64 = 3.5;
/// It climbs RISE rows and settles back over `RISE_T` seconds.
const RISE: f64 = 5.0;
const RISE_T: f64 = 150.0;
/// The bright stars, which twinkle: [col, row, brightness, period in seconds].
const BRIGHT: [(usize, usize, f64, f64); 4] = [
    (24, 9, 1.0, 4.6),
    (67, 27, 0.85, 3.4),
    (99, 12, 0.95, 5.8),
    (189, 7, 0.8, 4.1),
];
/// A few big boulders in the near ground, [x, z, radius].
const ROCKS: [[f64; 3]; 6] = [
    [30.0, 50.0, 2.4],
    [62.0, 63.0, 1.8],
    [12.0, 70.0, 1.3],
    [-4.0, 45.0, 1.2],
    [90.0, 55.0, 1.6],
    [46.0, 90.0, 1.4],
];
const TW: usize = 192;
const TH: usize = 96;


/// Toward the sun: low, from the right and a little behind us (z is forward).
fn sun() -> [f64; 3] {
    unit([0.94, 0.14, -0.3])
}

/// A bowl with a raised rim, r in ground units, q its distance in radii.
fn bowl(q: f64, r: f64) -> f64 {
    r * ((if q < 1.0 { -0.36 * (1.0 - q * q) } else { 0.0 })
        + 0.14 * (-((q - 1.0) / 0.25).powi(2)).exp())
}

/// Craters scattered one to a cell.
fn craters(x: f64, z: f64, cell: f64, salt: f64, r0: f64, r1: f64, p: f64) -> f64 {
    let ci = (x / cell).floor();
    let cj = (z / cell).floor();
    let mut h = 0.0;
    for j in [cj - 1.0, cj, cj + 1.0] {
        for i in [ci - 1.0, ci, ci + 1.0] {
            if hash(i + salt, j - salt) > p {
                continue;
            }
            let cx = (i + hash(i, j + salt * 3.0)) * cell;
            let cz = (j + hash(i + salt * 5.0, j)) * cell;
            let e = hash(j + salt, i - salt * 7.0);
            let r = cell * (r0 + (r1 - r0) * e * e);
            let (dx, dz) = (x - cx, z - cz);
            let d2 = dx * dx + dz * dz;
            if d2 > 4.0 * r * r {
                continue;
            }
            h += bowl(d2.sqrt() / r, r) * 0.95;
        }
    }
    h
}

fn height(x: f64, z: f64) -> f64 {
    let mut h = 6.0 * fbm(x * 0.006 + 50.0, z * 0.006 + 50.0, 3, 0.0)
        + 0.6 * fbm(x * 0.04, z * 0.04, 2, 0.0);
    // old, worn highlands at the edge of sight, rising to the left, and a
    // lower ridge in front of them
    if z > 150.0 {
        let lift = smooth(150.0, 300.0, z);
        // rounded massifs: folded noise, worn smooth
        let m = 1.0 - (2.0 * fbm(x * 0.006 + 7.0, z * 0.006, 4, 0.0) - 1.0).abs();
        h += lift * (28.0 * (-((x + 260.0) / 230.0).powi(2)).exp() + 45.0 * (m * m - 0.35));
        let ridge = (-((z - 205.0) / 20.0).powi(2)).exp() * (-((x + 140.0) / 130.0).powi(2)).exp();
        h += ridge * 22.0 * (0.35 + fbm(x * 0.018 + 3.0, z * 0.01, 3, 0.0));
    }
    // big basins only out in the middle distance, so we do not stand in one
    if z > 90.0 {
        h += craters(x, z, 110.0, 11.0, 0.14, 0.34, 0.55) * smooth(90.0, 150.0, z);
    }
    if z < 300.0 {
        h += craters(x, z, 34.0, 23.0, 0.12, 0.36, 0.85);
    }
    if z < 170.0 {
        h += craters(x, z, 10.0, 37.0, 0.12, 0.34, 0.85) * smooth(170.0, 110.0, z);
    }
    // one big crater in the near ground, off to the left
    {
        let (dx, dz) = (x + 24.0, z - 58.0);
        let q = (dx * dx + dz * dz).sqrt() / 16.0;
        if q < 2.0 {
            h += bowl(q, 16.0);
        }
    }
    // boulders strewn close by, and a few big ones
    if z < 90.0 {
        let c = 5.0;
        let (ci, cj) = ((x / c).floor(), (z / c).floor());
        if hash(ci + 91.0, cj) < 0.14 {
            let bx = (ci + 0.2 + 0.6 * hash(ci, cj + 92.0)) * c;
            let bz = (cj + 0.2 + 0.6 * hash(ci + 93.0, cj)) * c;
            let br = 0.3 + 0.5 * hash(ci + 94.0, cj + 95.0).powi(2);
            let d2 = ((x - bx).powi(2) + (z - bz).powi(2)) / (br * br);
            if d2 < 4.0 {
                h += br * 0.9 * (-d2 * 1.4).exp();
            }
        }
        for [bx, bz, br] in ROCKS {
            let d2 = ((x - bx).powi(2) + (z - bz).powi(2)) / (br * br);
            // a squat, lumpy dome
            if d2 < 1.0 {
                h += br * (0.85 + 0.3 * noise(x * 1.3, z * 1.3, 0.0)) * (1.0 - d2).sqrt();
            }
        }
    }
    h
}

/// A bright star's cell: its index, column, row, brightness, period, phase,
/// and whether it is the core or one arm of its cross.
struct Twinkle {
    k: usize,
    x: usize,
    r: usize,
    s: f64,
    p: f64,
    ph: f64,
    core: bool,
}

pub struct Earthrise {
    dots: Dots,
    /// The static picture: the ground and the sky, drawn once.
    base: Vec<Cell>,
    sr: Vec<f32>,
    sg: Vec<f32>,
    sb: Vec<f32>,
    scap: Vec<f32>,
    tr: Vec<f32>,
    tg: Vec<f32>,
    tb: Vec<f32>,
    sea: Vec<f32>,
    cloud: Vec<f32>,
    /// The sky cells the Earth and its air can reach, as it rises and settles.
    bx: Vec<usize>,
    twinkle: Vec<Twinkle>,
    l: [f64; 3],
    hv: [f64; 3],
}

/// Halftone one cell: dot size from brightness, colour from hue, with the
/// colour making up what the dot size could not, up to `cap`.
#[allow(clippy::too_many_arguments)]
fn dot(
    dots: &mut Dots,
    x: usize,
    r: usize,
    cr: f64,
    cg: f64,
    cb: f64,
    floor: f64,
    cap: f64,
) -> Cell {
    let peak = cr.max(cg).max(cb).max(1e-4);
    let level = clamp(floor + (1.0 - floor) * peak.powf(0.85) * 0.95);
    let step = Dots::step(level, bayer(r, x));
    let want = Dots::want(step, level, 0.06);
    let s = cap.min(0.3 + 0.7 * want) / peak;
    dots.dot(step, [cr, cg, cb], s)
}

/// Bilinear sample of an equirectangular map, wrapping in longitude.
fn sample(a: &[f32], lon: f64, vy: f64) -> f64 {
    let fx = ((lon / (PI * 2.0)) % 1.0 + 1.0) % 1.0 * TW as f64;
    let x0f = fx.floor();
    let ax = fx - x0f;
    let x0 = x0f as usize;
    let x1 = (x0 + 1) % TW;
    let y0f = vy.floor().clamp(0.0, (TH - 2) as f64);
    let ay = clamp(vy - y0f);
    let y0 = y0f as usize;
    let v = |i: usize| f64::from(a[i]);
    let (a0, b0) = (v(y0 * TW + x0), v(y0 * TW + x1));
    let (c0, d0) = (v(y0 * TW + TW + x0), v(y0 * TW + TW + x1));
    a0 + (b0 - a0) * ax + (c0 - a0) * ay + (a0 - b0 - c0 + d0) * ax * ay
}

/// Thresholds by share of the globe.
fn quantile(a: &[f32], p: f64) -> f64 {
    let mut s = a.to_vec();
    s.sort_by(f32::total_cmp);
    f64::from(s[(p * (s.len() - 1) as f64).floor() as usize])
}

impl Piece for Earthrise {
    const NAME: &'static str = "earthrise";
    const COLS: usize = W;
    const ROWS: usize = H;
    const FPS: u32 = 15;
    const CELL: usize = 1;
    const FIT: Fit = Fit::Cover { anchor: 0.2 };
    const GROUND: u32 = hex("#030408");
    #[rustfmt::skip]
    const PALETTE: &'static [u32] = &[
        hex("#18181b"), hex("#232326"), hex("#303033"), hex("#414143"), hex("#555556"), hex("#6b6a69"), hex("#83817d"), hex("#9c9993"),
        hex("#b6b2aa"), hex("#cfcac1"), hex("#e6e1d8"), hex("#f7f4ee"),
        hex("#0c1120"), hex("#131a2e"), hex("#1b2540"), hex("#262f4a"),
        hex("#dfe9ff"),
        hex("#0a2259"), hex("#0f2f72"), hex("#15408c"), hex("#1d53a6"), hex("#2a69bf"), hex("#4386d3"),
        hex("#6eaeea"), hex("#a8d3f6"),
        hex("#e8f0fa"), hex("#c2d0e3"), hex("#8b9fbc"),
        hex("#2f4a26"), hex("#3b5a2c"), hex("#5b7238"), hex("#7a9150"), hex("#77783f"), hex("#8f8550"), hex("#a8955e"), hex("#6b5634"),
        hex("#b9774a"), hex("#8a4838"),
    ];

    fn new() -> Self {
        let sun = sun();
        let mut dots = Dots::new(Self::PALETTE);

        // the ground: march each column from near to far
        let mut ground = vec![false; N];
        let mut top = vec![H as i32; W];
        let mut gx = vec![0f32; N];
        let mut gz = vec![0f32; N];
        let mut gy = vec![0f32; N];
        let cam_y = height(0.0, 7.0) + CAM_H;
        for (c, top_c) in top.iter_mut().enumerate() {
            let dir = (c as f64 + 0.5 - W as f64 / 2.0) / F;
            let mut top_r = H as i32;
            let (mut z, mut prev_y, mut prev_z, mut prev_f) = (26.0, 0.0, 0.0, 1e9);
            while z < ZMAX && top_r > 0 {
                let x = dir * z;
                let hy = height(x, z);
                let y = hy - (z * z) / (2.0 * RM);
                let yf = EYE - (F * (y - cam_y)) / z;
                let r0 = (yf - 0.5).ceil().max(0.0) as i32;
                for r in r0..top_r {
                    // place the cell between this sample and the last, by where its row falls
                    let a = if prev_f > yf + 1e-6 {
                        clamp((prev_f - (f64::from(r) + 0.5)) / (prev_f - yf))
                    } else {
                        1.0
                    };
                    let k = r as usize * W + c;
                    ground[k] = true;
                    gz[k] = (if prev_z != 0.0 { mix(prev_z, z, a) } else { z }) as f32;
                    gx[k] = (dir * f64::from(gz[k])) as f32;
                    gy[k] = (if prev_z != 0.0 {
                        mix(prev_y, hy, a)
                    } else {
                        hy
                    }) as f32;
                }
                if r0 < top_r {
                    top_r = r0;
                }
                (prev_f, prev_y, prev_z) = (yf, hy, z);
                z += 0.03 + z * 0.012;
            }
            *top_c = top_r;
        }

        // Static light: the ground and the sky, everything but the Earth's disc.
        let mut sr = vec![0f32; N];
        let mut sg = vec![0f32; N];
        let mut sb = vec![0f32; N];
        let mut scap = vec![1f32; N];
        for r in 0..H {
            for (xi, &top_x) in top.iter().enumerate() {
                let k = r * W + xi;
                if !ground[k] {
                    continue;
                }
                let (x, z, y) = (f64::from(gx[k]), f64::from(gz[k]), f64::from(gy[k]));
                let e = 0.1 + z * 0.004;
                let hx = (height(x + e, z) - height(x - e, z)) / (2.0 * e);
                let hz = (height(x, z + e) - height(x, z - e)) / (2.0 * e);
                let nl = js_hypot(&[hx, 1.0, hz]);
                let lam = (-hx * sun[0] + sun[1] - hz * sun[2]) / nl;
                // toward the eye, for the moon's own way of reflecting (Lommel-Seeliger)
                let (vx, vy, vz) = (-x, cam_y - y, -z);
                let vl = js_hypot(&[vx, vy, vz]);
                let mu = ((-hx * vx + vy - hz * vz) / (nl * vl)).max(0.02);
                let mut lit = 0.0;
                if lam > 0.0 {
                    // march toward the sun; a soft edge for the sun's own width
                    lit = 1.0;
                    let mut s = 0.1 + z * 0.003;
                    while s < 120.0 {
                        let (px, pz, py) = (x + sun[0] * s, z + sun[2] * s, y + sun[1] * s);
                        let d = py - height(px, pz);
                        if d < 0.0 {
                            lit = 0.0;
                            break;
                        }
                        lit = f64::min(lit, (d * 30.0) / s);
                        s += 0.05 + s * 0.2;
                    }
                    lit = smooth(0.0, 1.0, lit);
                }
                // regolith: patchy, the maria darker
                let albedo = 0.5
                    + 0.9 * fbm(x * 0.025 + 3.0, z * 0.025, 3, 0.0)
                    + 0.3 * (fbm(x * 0.35, z * 0.35, 2, 0.0) - 0.5)
                    - 0.22 * smooth(0.46, 0.64, fbm(x * 0.0035, z * 0.0035 + 20.0, 3, 0.0));
                let ls = if lam > 0.0 {
                    ((0.2 * lam) / (lam + mu) + 2.6 * lam) * lit
                } else {
                    0.0
                };
                // the near corners fall off a little, to frame the view
                let vig = 1.0
                    - 0.3
                        * smooth(84.0, 102.0, r as f64)
                        * smooth(30.0, 100.0, (xi as f64 - 100.0).abs());
                let mut b = (1.0 - (-ls * 1.5).exp()) * albedo * vig;
                // the far crest catches the sun along its whole length
                let crest = r as i32 - top_x;
                if crest < 2 && lit > 0.2 {
                    b = b.max(if crest != 0 { 0.55 } else { 0.85 } * albedo);
                }
                // shadow is black, but for a breath of earthshine on what faces us
                let fill = if lit > 0.02 || b > 0.03 {
                    0.008 + 0.007 * clamp(-hz / nl + 0.5)
                } else {
                    0.0
                };
                sr[k] = (b + fill * 0.75) as f32;
                sg[k] = (b * 0.95 + fill * 0.85) as f32;
                sb[k] = (b * 0.86 + fill * 1.2) as f32;
                scap[k] = (0.42 + 0.62 * b) as f32; // grey stays grey: dim ground draws in darker ink
            }
        }

        // the sky: a soft band of the galaxy, and stars that hold still
        let near =
            |x: f64, r: f64| (x + 0.5 - EC[0]).hypot(r + 0.5 - (EC[1] - RISE / 2.0)) < 2.0 * ER;
        for r in 0..H {
            for xi in 0..W {
                let k = r * W + xi;
                if ground[k] {
                    continue;
                }
                let (x, rf) = (xi as f64, r as f64);
                // a black sky. Faint stars, thicker along a diagonal where the
                // galaxy runs, none round the Earth
                let band_d = (rf - (4.0 + x * 0.32)) / 1.05;
                let band = (-(band_d / 10.0).powi(2)).exp()
                    * smooth(0.35, 0.65, fbm(x * 0.05, rf * 0.08, 3, 0.0))
                    * smooth(120.0, 70.0, x);
                let (mut cr, mut cg, mut cb) = (0.0, 0.0, 0.0);
                let hs = hash(x * 3.0 + 1.0, rf * 7.0 + 2.0);
                if hs > 0.994 - 0.05 * band && !near(x, rf) {
                    let m = hash(x + 17.0, rf + 29.0).powi(3);
                    let s = 0.2 + 0.5 * m;
                    let tint = hash(x + 5.0, rf + 77.0);
                    let [tr, tg, tb] = if tint < 0.3 {
                        [0.84, 0.9, 1.0]
                    } else if tint > 0.88 {
                        [1.0, 0.93, 0.84]
                    } else {
                        [0.96, 0.96, 0.98]
                    };
                    (cr, cg, cb) = (s * tr, s * tg, s * tb);
                }
                sr[k] = cr as f32;
                sg[k] = cg as f32;
                sb[k] = cb as f32;
                scap[k] = 1.0;
            }
        }
        let base = (0..N)
            .map(|k| {
                let f = |v: &[f32]| f64::from(v[k]);
                dot(
                    &mut dots,
                    k % W,
                    k / W,
                    f(&sr),
                    f(&sg),
                    f(&sb),
                    0.0,
                    f(&scap),
                )
            })
            .collect();

        // the Earth: equirectangular maps, wrapping in longitude, of surface
        // colour and cloud
        let tn = TW * TH;
        let mut tr = vec![0f32; tn];
        let mut tg = vec![0f32; tn];
        let mut tb = vec![0f32; tn];
        let mut sea = vec![0f32; tn];
        let mut cloud = vec![0f32; tn];
        let mut elev = vec![0f32; tn];
        let storms: [[f64; 3]; 5] = [
            [0.9, 0.8, 1.0],
            [3.1, -0.85, -1.0],
            [4.6, 0.62, 1.0],
            [1.9, 0.25, 1.0],
            [5.6, -0.55, -1.0],
        ];
        for j in 0..TH {
            let v = (j as f64 + 0.5) / TH as f64;
            let lat = (0.5 - v) * PI;
            let al = lat.abs();
            for i in 0..TW {
                let u = i as f64 / TW as f64;
                let lon = u * PI * 2.0;
                let t = j * TW + i;
                let wx = fbm(u * 6.0, v * 3.0 + 9.0, 3, 6.0);
                elev[t] =
                    (fbm(u * 8.0 + 1.6 * wx, v * 4.0, 5, 8.0) - 0.04 * smooth(1.2, 1.5, al)) as f32;
                // clouds: warped noise, wound into spirals around a few storms
                let (mut cx, mut cy) = (u * 14.0, v * 7.0);
                for [slon, slat, spin] in storms {
                    let mut dl = lon - slon;
                    dl -= js_round(dl / (PI * 2.0)) * PI * 2.0;
                    let (lx, ly) = (dl * lat.cos(), lat - slat);
                    let dd = lx.hypot(ly);
                    let a = spin * 5.0 * (-dd / 0.2).exp();
                    if a * spin > 0.02 {
                        let (ca, sa) = (a.cos(), a.sin());
                        cx += ((lx * ca - ly * sa - lx) / (PI * 2.0)) * 14.0;
                        cy -= ((lx * sa + ly * ca - ly) / PI) * 7.0;
                    }
                }
                // streaked along the winds, east to west
                let q = fbm(cx * 0.5 + 3.0, cy * 1.2, 3, 7.0);
                let n0 = fbm(cx + 1.6 * q, cy * 1.3 + 0.5 * q, 5, 14.0);
                // folded into filaments, the way weather fronts string out
                let n = 0.4 * n0
                    + 0.6
                        * (1.0
                            - (2.0 * fbm(cx * 1.5 + 2.2 * q, cy * 1.6 + 9.0, 4, 21.0) - 1.0).abs());
                // cloudy at the equator and in the storm belts, clearer in the subtropics
                let belt = 0.05 * (-(lat / 0.12).powi(2)).exp()
                    - 0.07 * (-((al - 0.42) / 0.16).powi(2)).exp()
                    + 0.05 * (-((al - 0.95) / 0.25).powi(2)).exp();
                cloud[t] = (n + belt) as f32;
            }
        }
        // about three tenths land, a third cloud
        let shore = quantile(&elev, 0.7);
        let c0 = quantile(&cloud, 0.6);
        let c1 = quantile(&cloud, 0.86);
        for j in 0..TH {
            let v = (j as f64 + 0.5) / TH as f64;
            let lat = (0.5 - v) * PI;
            let al = lat.abs();
            for i in 0..TW {
                let u = i as f64 / TW as f64;
                let t = j * TW + i;
                let e = f64::from(elev[t]);
                let land = smooth(shore - 0.004, shore + 0.006, e);
                let ice = smooth(1.22, 1.32, al + 0.12 * fbm(u * 12.0, v * 6.0, 2, 12.0));
                // desert in the subtropics and on high ground, forest and scrub
                // elsewhere, broken up finely so a continent is never one flat tone
                let grain = fbm(u * 36.0 + 2.0, v * 18.0, 3, 36.0) - 0.5;
                let arid = clamp(
                    (-((al - 0.4) / 0.22).powi(2)).exp()
                        * (0.1 + 1.2 * fbm(u * 10.0 + 4.0, v * 5.0, 3, 10.0))
                        + (e - shore - 0.04) * 4.0
                        + 0.8 * grain,
                );
                let relief = 1.0 + 1.2 * grain - 2.5 * (e - shore - 0.08).max(0.0);
                let mut r = mix(0.13, 0.4, arid) * relief;
                let mut g = mix(0.25, 0.34, arid) * relief;
                let mut b = mix(0.08, 0.19, arid) * relief;
                // ocean, lighter over the shelves near the coasts
                let shelf = smooth(shore - 0.06, shore, e);
                let (or, og, ob) = (
                    mix(0.025, 0.07, shelf),
                    mix(0.11, 0.3, shelf),
                    mix(0.38, 0.62, shelf),
                );
                (r, g, b) = (mix(or, r, land), mix(og, g, land), mix(ob, b, land));
                (r, g, b) = (mix(r, 0.92, ice), mix(g, 0.95, ice), mix(b, 0.99, ice));
                tr[t] = r as f32;
                tg[t] = g as f32;
                tb[t] = b as f32;
                sea[t] = ((1.0 - land) * (1.0 - ice)) as f32;
                cloud[t] = smooth(c0, c1, f64::from(cloud[t])) as f32;
            }
        }

        let l = [sun[0], sun[1], -sun[2]]; // into screen space, z toward us
        let hv = unit([l[0], l[1], l[2] + 1.0]);
        let mut bx = Vec::new();
        let (ec0, ec1, er) = (EC[0] as i32, EC[1] as i32, ER as i32);
        for r in (ec1 - RISE as i32 - er - 13).max(0)..=(ec1 + er + 2).min(H as i32 - 1) {
            for x in ec0 - er - 13..=ec0 + er + 13 {
                let k = r as usize * W + x as usize;
                if !ground[k] {
                    bx.push(k);
                }
            }
        }
        // the bright stars and the faint cross each one carries
        let mut twinkle = Vec::new();
        for (x, r, s, p) in BRIGHT {
            for (ox, oy, w) in [
                (0, 0, 1.0),
                (-1, 0, 0.2),
                (1, 0, 0.2),
                (0, -1, 0.2),
                (0, 1, 0.2),
            ] {
                let (cx, cr) = ((x as i32 + ox) as usize, (r as i32 + oy) as usize);
                let k = cr * W + cx;
                if !ground[k] {
                    twinkle.push(Twinkle {
                        k,
                        x: cx,
                        r: cr,
                        s: s * w,
                        p,
                        ph: hash(x as f64, r as f64) * 6.28,
                        core: w == 1.0,
                    });
                }
            }
        }

        Self {
            dots,
            base,
            sr,
            sg,
            sb,
            scap,
            tr,
            tg,
            tb,
            sea,
            cloud,
            bx,
            twinkle,
            l,
            hv,
        }
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        out.copy_from_slice(&self.base);
        let (l, hv) = (self.l, self.hv);
        let (tilt, nod): (f64, f64) = (0.4, 0.22);
        let (ct, st, cn, sn) = (tilt.cos(), tilt.sin(), nod.cos(), nod.sin());
        let spin = t * 0.045;
        let drift = t * 0.012; // clouds run a little ahead of the ground
        let ey = EC[1] - RISE * (0.5 - 0.5 * ((t / RISE_T) * PI * 2.0).cos());
        for &k in &self.bx {
            let (x, r) = (k % W, k / W);
            let mut cr = f64::from(self.sr[k]);
            let mut cg = f64::from(self.sg[k]);
            let mut cb = f64::from(self.sb[k]);
            let mut cap = f64::from(self.scap[k]);
            let mut floor = 0.0;
            let dx = x as f64 + 0.5 - EC[0];
            let dy = r as f64 + 0.5 - ey;
            let d = dx.hypot(dy);
            // the Earth's air, a thin blue rim on its sunlit side
            if (ER - 1.0..ER + 12.0).contains(&d) {
                let side = smooth(-0.3, 0.75, (dx * l[0] - dy * l[1]) / d);
                let mut g = (-(d - ER).max(0.0) / 1.2).exp() * 0.55 * side;
                if g < 0.05 {
                    g = 0.0; // no stray haze dots out in space
                }
                cr += 0.25 * g;
                cg += 0.52 * g;
                cb += g;
                if g > 0.0 {
                    cap = 1.0;
                }
            }
            if d < ER {
                let (nx, ny) = (dx / ER, -dy / ER);
                let q2 = nx * nx + ny * ny;
                let nz = (1.0 - q2).sqrt();
                // into the globe's own frame: tip the pole toward us, then lean it
                let ax = nx * ct + ny * st;
                let ay0 = -nx * st + ny * ct;
                let ay = ay0 * cn - nz * sn;
                let az = ay0 * sn + nz * cn;
                let lat = ay.clamp(-1.0, 1.0).asin();
                let lon = ax.atan2(az) + LON0 + spin;
                let vy = (0.5 - lat / PI) * TH as f64 - 0.5;
                let ndl = nx * l[0] + ny * l[1] + nz * l[2];
                // full sun at the right limb, dimming toward the terminator, so
                // the disc reads as a ball
                let day = smooth(-0.005, 0.06, ndl) * (0.45 + 0.8 * clamp(ndl).sqrt());
                let dusk = (-((ndl - 0.02) / 0.035).powi(2)).exp();
                let cl = sample(&self.cloud, lon + drift, vy);
                // the cloud's own shadow, offset away from the sun
                let sh = sample(&self.cloud, lon + drift - 0.03, vy + 0.4);
                let sw = sample(&self.sea, lon, vy);
                let mut er = sample(&self.tr, lon, vy);
                let mut eg = sample(&self.tg, lon, vy);
                let mut eb = sample(&self.tb, lon, vy);
                let shade = 1.0 - 0.45 * sh * (1.0 - cl);
                er *= shade;
                eg *= shade;
                eb *= shade;
                (er, eg, eb) = (mix(er, 0.95, cl), mix(eg, 0.97, cl), mix(eb, 1.0, cl));
                er *= day * (1.0 + 0.06 * dusk);
                eg *= day * (1.0 - 0.03 * dusk);
                eb *= day * (1.0 - 0.1 * dusk);
                let glint = (nx * hv[0] + ny * hv[1] + nz * hv[2]).max(0.0).powi(70)
                    * 0.8
                    * sw
                    * (1.0 - cl);
                let rim = (1.0 - nz).powf(2.2) * smooth(0.0, 0.3, ndl);
                er += glint * 0.95 + rim * 0.2;
                eg += glint * 0.92 + rim * 0.42;
                eb += glint * 0.85 + rim * 0.85;
                // a crisp edge where the disc meets space; the night side is a void
                let edge = smooth(1.0, 0.95, q2.sqrt());
                (cr, cg, cb) = (mix(cr, er, edge), mix(cg, eg, edge), mix(cb, eb, edge));
                // land keeps its earth tones instead of washing out to cream
                cap = mix(
                    1.0,
                    0.66,
                    (1.0 - sw) * (1.0 - cl) * smooth(0.95, 0.85, ay.abs()),
                );
                // the day side is the brightest thing in the sky: full, round dots
                floor = 0.15 * day.min(1.0) * edge;
            }
            out[k] = dot(&mut self.dots, x, r, cr, cg, cb, floor, cap);
        }
        for tw in &self.twinkle {
            let v = tw.s * (0.86 + 0.14 * ((t / tw.p) * PI * 2.0 + tw.ph).sin());
            out[tw.k] = if tw.core {
                dot(&mut self.dots, tw.x, tw.r, v * 0.97, v * 0.98, v, 0.0, 1.0)
            } else {
                let k = tw.k;
                dot(
                    &mut self.dots,
                    tw.x,
                    tw.r,
                    f64::from(self.sr[k]).max(v * 0.95),
                    f64::from(self.sg[k]).max(v),
                    f64::from(self.sb[k]).max(v * 1.15),
                    0.0,
                    0.6,
                )
            };
        }
    }
}
