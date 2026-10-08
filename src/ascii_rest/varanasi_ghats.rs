//! varanasi ghats: dusk on the ganga. Stepped ghats run along the bank and
//! away toward the afterglow, temple spires black against an indigo to amber
//! sky. Priests raise the aarti lamps on the steps, diyas drift downstream on
//! the dark water, and a boatman rows slowly across the bright reach.
//!
//! Its tail is its own: the dot's colour lift is capped per step and stars take
//! the last palette entry directly, so the nearest-colour search skips it.
//!
//! `varanasi-ghats-wide` is the same evening on a 3.2:1 canvas: the ghats run a
//! third further before they meet the far bank, and a broad reach of river
//! opens beyond the glow.

use std::f64::consts::PI;

use super::halftone::{bayer, Dots};
use super::math::{clamp, fbm, hash, mix, noise, smooth};
use super::{Fit, Piece, hex};
use crate::font;
use crate::grid::Cell;

const H: usize = 100;
const HF: f64 = H as f64;
const HZ: f64 = 60.0; // the far bank
/// How far a dot's colour may be brightened to make up for its size: small
/// dots in dark areas stay dark instead of turning into a pale speckle.
const LIFT: [f64; 4] = [0.0, 2.5, 1.7, 1.35];
const FLAME: [f64; 3] = [1.0, 0.62, 0.22];
const WASH: [f64; 3] = [1.0, 0.5, 0.2];
/// Starlight, the palette's last entry, set directly and never matched.
const STAR: usize = <VaranasiGhats as Piece>::PALETTE.len() - 1;
/// Cloud strip, wrapping at `CW` columns, `CH` rows deep.
const CW: usize = 800;
const CH: usize = HZ as usize - 6;
const BS: f64 = 0.85;

const SKY: u8 = 0;
const WATER: u8 = 1;
const BANK: u8 = 2;
const STEPS: u8 = 3;
const BLD: u8 = 4;
const SPIRE: u8 = 5;
const UMB: u8 = 6;
const FIG: u8 = 7;
const PLAT: u8 = 8;

/// Heat, then colour: indigo overhead, violet, plum, rose, coral, amber, gold.
const RAMP: [[f64; 4]; 12] = [
    [0.0, 0.085, 0.07, 0.24],
    [0.14, 0.13, 0.1, 0.34],
    [0.28, 0.22, 0.14, 0.44],
    [0.42, 0.34, 0.17, 0.48],
    [0.52, 0.47, 0.21, 0.47],
    [0.62, 0.6, 0.26, 0.42],
    [0.7, 0.7, 0.31, 0.36],
    [0.78, 0.8, 0.39, 0.3],
    [0.86, 0.88, 0.5, 0.26],
    [0.92, 0.95, 0.65, 0.32],
    [0.97, 1.0, 0.8, 0.48],
    [1.0, 1.0, 0.92, 0.7],
];
const RL: usize = 256;

/// The heat ramp's index for heat `h`.
fn ramp_at(h: f64) -> usize {
    (clamp(h) * RL as f64 + 0.5) as usize * 3
}

/// Stretched, warped noise, gathered into three loose bands of cloud.
fn density(x: f64, y: f64) -> f64 {
    let q = fbm(x * 0.005, y * 0.03, 3, 4.0);
    let d = fbm(x * 0.0125 + q * 1.8, y * 0.09 + q * 0.9, 5, 10.0);
    let env =
        0.03 * (-((y - 13.0) / 4.0).powi(2)).exp() + 0.1 * (-((y - 29.0) / 5.5).powi(2)).exp();
    // and one thin, broken streak low down, to cross the glow
    let c = 46.5 + 3.0 * (fbm(x * 0.01, 9.5, 2, 8.0) - 0.5);
    let th = 1.0 + 0.9 * fbm(x * 0.05, 3.5, 2, 40.0);
    let streak = (-((y - c) / th).powi(2)).exp()
        * smooth(0.42, 0.6, fbm(x * 0.0125, 21.5, 3, 10.0))
        * (0.75 + 0.5 * fbm(x * 0.05, y * 0.3, 3, 40.0));
    f64::max(d + env - 0.08, 0.36 + 0.3 * streak)
}

/// The boat, in local coordinates: lx along the boat, ly up from the waterline.
fn boat_at(lx: f64, ly: f64) -> f64 {
    const L: f64 = 9.0;
    if lx.abs() < L && ly > -0.6 && ly < 1.2 + 1.4 * (lx.abs() / L).powi(3) {
        return 1.0;
    }
    if lx > 4.6 && lx < 5.8 && ly > 0.0 && ly < 6.6 {
        return 1.0; // the boatman
    }
    if (lx - 5.2).abs() < 0.7 && (ly - 7.2).abs() < 0.7 {
        return 1.0;
    }
    let (ox, oy) = (lx - 5.6, ly - 5.4); // his oar, raked back into the water
    let along = ox * 0.42 - oy * 0.91;
    if along > 0.0 && along < 8.0 && (ox * 0.91 + oy * 0.42).abs() < 0.32 {
        return 1.0;
    }
    if lx > -4.0 && lx < -0.5 && ly > 0.0 && ly < 2.6 - 0.25 * (lx + 2.2).abs() {
        return 1.0; // a passenger
    }
    0.0
}

/// Where things sit in the frame: upstream's, or the `-wide` recomposition's.
struct Layout {
    name: &'static str,
    w: usize,
    /// Where the ghats meet the far bank.
    end: f64,
    /// The afterglow, sitting on the far bank.
    glow: [f64; 2],
    /// Temple spires: [centre x, height in rows, half width at the base].
    spires: &'static [[f64; 3]],
    /// Aarti stations on the near ghat, unevenly spaced: [x, streak width, streak
    /// length].
    aarti: &'static [[f64; 3]],
    /// Umbrellas on the far steps.
    umbs: &'static [f64],
    /// Where the buildings start to sit lower.
    low_from: f64,
    /// Where the far bank's tree clumps rise in.
    clumps: [f64; 2],
    /// The diyas on the steps: the column they stop at, and where they gather
    /// [centre, spread].
    step_lamps: (usize, [f64; 2]),
    /// Where the electric lamps along the far ghats start.
    lamps_from: f64,
    /// The floating diyas: how many, and the span they wrap over.
    diyas: (usize, f64),
    /// The cloud's drift at t = 0: the long streaks start over the glow.
    drift: f64,
    /// Where the boatman starts, and the span he wraps over.
    boat: [f64; 2],
}

const ORIGINAL: Layout = Layout {
    name: "varanasi-ghats",
    w: 200,
    end: 150.0,
    glow: [156.0, HZ - 1.5],
    spires: &[
        [118.0, 33.0, 5.2],
        [64.0, 19.0, 4.0],
        [31.0, 15.0, 3.6],
        [139.0, 8.0, 1.6],
        [90.0, 11.0, 2.4],
        [129.0, 10.0, 2.0],
    ],
    aarti: &[
        [11.0, 1.15, 1.25],
        [24.0, 0.8, 0.85],
        [39.0, 1.25, 1.1],
        [54.0, 0.75, 0.75],
    ],
    umbs: &[70.0, 83.0, 98.0, 109.0],
    low_from: 110.0,
    clumps: [160.0, 172.0],
    step_lamps: (118, [33.0, 36.0]),
    lamps_from: 58.0,
    diyas: (34, 260.0),
    drift: 720.0,
    boat: [159.0, 250.0],
};

const WIDE: Layout = Layout {
    name: "varanasi-ghats-wide",
    w: 320,
    end: 200.0,
    glow: [206.0, HZ - 1.5],
    spires: &[
        [157.0, 33.0, 5.2],
        [85.0, 19.0, 4.0],
        [41.0, 15.0, 3.6],
        [185.0, 8.0, 1.6],
        [120.0, 11.0, 2.4],
        [172.0, 10.0, 2.0],
        [104.0, 14.0, 2.8],
        [62.0, 10.0, 2.6],
    ],
    aarti: &[
        [11.0, 1.15, 1.25],
        [25.0, 0.8, 0.85],
        [42.0, 1.25, 1.1],
        [58.0, 0.75, 0.75],
        [73.0, 0.9, 0.8],
    ],
    umbs: &[93.0, 111.0, 131.0, 145.0, 162.0, 176.0],
    low_from: 147.0,
    clumps: [210.0, 222.0],
    step_lamps: (157, [44.0, 48.0]),
    lamps_from: 77.0,
    diyas: (52, 380.0),
    drift: 670.0,
    boat: [209.0, 370.0],
};

pub type VaranasiGhats = Scene<false>;
pub type VaranasiGhatsWide = Scene<true>;

pub struct Scene<const IS_WIDE: bool> {
    dots: Dots,
    rt: Vec<f32>,
    mat: Vec<u8>,
    s: [Vec<f32>; 3],
    flo: Vec<f32>,
    /// How much the lamps' wash lights each cell.
    alb: Vec<f32>,
    warm: Vec<f32>,
    step_lamps: Vec<(usize, f64)>,
    sref: Vec<f32>,
    refl: [Vec<f32>; 3],
    /// The lit cell each water cell mirrors, -1 for none.
    mir_k: Vec<i32>,
    cover: Vec<f32>,
    clit: Vec<f32>,
    diyas: Vec<[f64; 4]>,
    dynl: [Vec<f32>; 3],
    /// Warm light to be reflected.
    dref: Vec<f32>,
    /// The priests' raised arms, this frame.
    arm: Vec<u8>,
}

impl<const IS_WIDE: bool> Scene<IS_WIDE> {
    const L: Layout = if IS_WIDE { WIDE } else { ORIGINAL };
    const W: usize = Self::L.w;
    const WF: f64 = Self::W as f64;

    /// The waterline along the ghats, near at the left and running away to the right.
    fn wl_f(x: f64) -> f64 {
        if x < Self::L.end {
            HZ + 0.5 + 20.0 * (1.0 - x / Self::L.end).powf(1.6)
        } else {
            HZ + 0.5
        }
    }

    fn sc_f(x: f64) -> f64 {
        0.45 + 1.75 * (1.0 - x / Self::L.end).max(0.0).powf(1.3)
    }

    fn step_top_f(x: f64) -> f64 {
        Self::wl_f(x) - 9.5 * Self::sc_f(x) * (0.3 + 0.7 * smooth(Self::L.end, Self::L.end - 16.0, x))
    }

    /// The dusk sky's heat: low overhead, rising toward the horizon, hottest at the glow.
    fn sky_heat(x: f64, y: f64) -> f64 {
        let v = clamp(y / HZ);
        let dx = (x - Self::L.glow[0]).abs();
        let dy = (Self::L.glow[1] - y).abs();
        // a wide warm band along the horizon, and a small hot core on the bank
        let wide = 0.46 * (-dx / 55.0 - dy / 23.0).exp();
        let core = 0.24 * (-((dx / 13.0).powi(2) + (dy / 4.5).powi(2)).sqrt()).exp();
        0.06 + 0.42 * v * v + wide + core
    }

    /// A lamp: a white-hot heart inside an orange halo; the priests are not lit.
    fn add_glow(
        dynl: &mut [Vec<f32>; 3],
        mat: &[u8],
        fx: f64,
        fy: f64,
        core: f64,
        halo: f64,
        amp: f64,
    ) {
        let rr = (halo * 3.2).ceil();
        let mut r = (fy - rr).floor().max(0.0);
        while r < HF.min(fy + rr) {
            let mut x = (fx - rr).floor().max(0.0);
            while x < Self::WF.min(fx + rr) {
                let k = r as usize * Self::W + x as usize;
                let dx = x + 0.5 - fx;
                let dy = (r + 0.5 - fy) * 0.85;
                let d = (dx * dx + dy * dy).sqrt();
                let am = if mat[k] == FIG { amp * 0.15 } else { amp };
                let c = (-(d / core).powi(2)).exp() * 1.6 * am;
                let v = (-d / halo).exp() * 0.35 * am;
                dynl[0][k] = (f64::from(dynl[0][k]) + (c + v * FLAME[0])) as f32;
                dynl[1][k] = (f64::from(dynl[1][k]) + (c * 0.86 + v * FLAME[1])) as f32;
                dynl[2][k] = (f64::from(dynl[2][k]) + (c * 0.55 + v * FLAME[2])) as f32;
                x += 1.0;
            }
            r += 1.0;
        }
    }

    /// The lamps' wide warm wash on the stone, by how much each surface takes it.
    fn add_wash(dynl: &mut [Vec<f32>; 3], alb: &[f32], fx: f64, fy: f64, halo: f64, amp: f64) {
        let rr = (halo * 2.6).ceil();
        let mut r = (fy - rr).floor().max(0.0);
        while r < HF.min(fy + rr) {
            let mut x = (fx - rr).floor().max(0.0);
            while x < Self::WF.min(fx + rr) {
                let k = r as usize * Self::W + x as usize;
                let a = f64::from(alb[k]);
                if a != 0.0 {
                    let (dx, dy) = (x + 0.5 - fx, r + 0.5 - fy);
                    let v = (-(dx * dx + dy * dy).sqrt() / halo).exp() * amp * a;
                    for c in 0..3 {
                        dynl[c][k] = (f64::from(dynl[c][k]) + v * WASH[c]) as f32;
                    }
                }
                x += 1.0;
            }
            r += 1.0;
        }
    }

    fn add_streak(dref: &mut [f32], mat: &[u8], fx: f64, wl: f64, width: f64, len: f64, amp: f64) {
        let mut r = wl.floor().max(0.0);
        while r < HF {
            let dy = r + 0.5 - wl;
            let a = (-dy / len).exp() * amp * smooth(-0.5, 1.0, dy);
            if a < 0.01 {
                break;
            }
            let mut x = (fx - 3.0 * width - 1.0).floor().max(0.0);
            while x < Self::WF.min(fx + 3.0 * width + 1.0) {
                let k = r as usize * Self::W + x as usize;
                if mat[k] == WATER {
                    dref[k] =
                        (f64::from(dref[k]) + a * (-((x + 0.5 - fx) / width).powi(2)).exp()) as f32;
                }
                x += 1.0;
            }
            r += 1.0;
        }
    }
}

impl<const IS_WIDE: bool> Piece for Scene<IS_WIDE> {
    const NAME: &'static str = Self::L.name;
    const COLS: usize = Self::W;
    const ROWS: usize = H;
    const FPS: u32 = 15;
    const CELL: usize = 1;
    const FIT: Fit = Fit::Cover { anchor: 0.3 };
    const GROUND: u32 = hex("#0b0812");
    #[rustfmt::skip]
    const PALETTE: &'static [u32] = &[
        // the sky, indigo through violet
        hex("#141230"), hex("#1d1a42"), hex("#272257"), hex("#332b6a"), hex("#45357a"), hex("#5a4388"), hex("#7259a8"),
        // plum, orchid, mauve and rose
        hex("#6d3a73"), hex("#8a4677"), hex("#a8527a"), hex("#c4607a"), hex("#d97a92"), hex("#b866a0"), hex("#9a68c8"), hex("#c868b0"), hex("#e8707a"),
        // coral to gold
        hex("#d9705f"), hex("#e88552"), hex("#f29f4a"), hex("#f8b85a"), hex("#fcd07a"), hex("#ffe3a6"), hex("#fff2d2"),
        // stone in shadow, and the violet treads
        hex("#1a1220"), hex("#24182a"), hex("#301f33"), hex("#3f2838"), hex("#523340"), hex("#3a2c4c"), hex("#4d3a64"),
        // stone in lamplight
        hex("#5e3524"), hex("#7d4527"), hex("#a0582a"), hex("#c06e2e"), hex("#d98a46"),
        // flame
        hex("#ffffff"), hex("#fff6d8"), hex("#ffd860"), hex("#ff9f2a"), hex("#f06a1e"), hex("#c8401a"), hex("#8a2a16"),
        // dark water
        hex("#0f1a2c"), hex("#162540"), hex("#20325a"),
        // starlight, set directly and never matched
        hex("#d6d4ee"),
    ];

    fn new() -> Self {
        let n = Self::W * H;

        // The heat ramp, tabulated.
        let mut rt = vec![0f32; (RL + 1) * 3];
        for i in 0..=RL {
            let h = i as f64 / RL as f64;
            let mut j = 0;
            while j < RAMP.len() - 2 && h > RAMP[j + 1][0] {
                j += 1;
            }
            let [h0, r0, g0, b0] = RAMP[j];
            let [h1, r1, g1, b1] = RAMP[j + 1];
            let k = clamp((h - h0) / (h1 - h0));
            rt[i * 3] = mix(r0, r1, k) as f32;
            rt[i * 3 + 1] = mix(g0, g1, k) as f32;
            rt[i * 3 + 2] = mix(b0, b1, k) as f32;
        }
        let rgb = |i: usize| [f64::from(rt[i]), f64::from(rt[i + 1]), f64::from(rt[i + 2])];

        // --- the bank ---
        // Buildings along the ghats: [x0, x1, height above the steps in scale units, seed]
        let mut blds: Vec<[f64; 4]> = Vec::new();
        let mut x = -6.0;
        while x < Self::L.end - 10.0 {
            let s = Self::sc_f(f64::max(0.0, x));
            let w = (8.0 + 10.0 * hash(x, 3.0)) * s;
            let low = (if x > Self::L.low_from { 0.8 } else { 1.0 })
                * (0.45 + 0.55 * smooth(Self::L.end - 8.0, Self::L.end - 34.0, x));
            blds.push([x, x + w, (7.0 + 9.0 * hash(x, 4.0)) * low, hash(x, 5.0)]);
            x += w;
        }

        let mut mat = vec![SKY; n];
        let mut s = [vec![0f32; n], vec![0f32; n], vec![0f32; n]];
        let mut flo = vec![0f32; n];
        let mut alb = vec![0f32; n];
        let mut warm = vec![0f32; n];

        let roof_of = |x: f64| {
            for &[x0, x1, h, _] in &blds {
                if x >= x0 && x < x1 {
                    return Self::step_top_f(x) - h * Self::sc_f(x);
                }
            }
            Self::step_top_f(x)
        };
        let spire_base: Vec<f64> = Self::L.spires.iter().map(|&[sx, _, _]| roof_of(sx) + 1.0).collect();

        for r in 0..H {
            for xi in 0..Self::W {
                let k = r * Self::W + xi;
                let (xc, y) = (xi as f64 + 0.5, r as f64 + 0.5);
                let wl = Self::wl_f(xc);
                if y >= wl {
                    mat[k] = WATER;
                    alb[k] = 0.25;
                    continue;
                }
                let mut m = SKY;
                let (mut cr, mut cg, mut cb) = (0.0, 0.0, 0.0);
                let (mut fl, mut al) = (0.06, 0.0);
                let sc = Self::sc_f(xc);

                // the far bank: an embankment and clumps of trees against the glow
                let clump =
                    smooth(0.42, 0.72, fbm(xc * 0.11, 3.0, 3, 0.0)) * smooth(Self::L.clumps[0], Self::L.clumps[1], xc);
                let bank_top = HZ - 2.2 - 0.6 * fbm(xc * 0.4, 7.0, 2, 0.0) - 4.0 * clump;
                if xc >= Self::L.end - 4.0 && y >= bank_top {
                    (m, cr, cg, cb, fl) = (BANK, 0.1, 0.07, 0.125, 0.03);
                }

                if xc < Self::L.end {
                    let st = Self::step_top_f(xc);
                    // buildings and their rooftop pavilions
                    let (mut roof, mut bi) = (st, None);
                    for (i, b) in blds.iter().enumerate() {
                        if xc >= b[0] && xc < b[1] {
                            roof = st - b[2] * sc;
                            bi = Some(i);
                        }
                    }
                    let mut top = roof;
                    if let Some(bi) = bi {
                        let [x0, x1, _, hb] = blds[bi];
                        let (mid, half) = ((x0 + x1) / 2.0, (x1 - x0) / 2.0);
                        if hb > 0.45 {
                            // a chhatri: a small dome on four posts
                            let cx = if hb > 0.75 { x0 + half * 0.4 } else { mid };
                            let dx = (xc - cx).abs() / (1.6 * sc);
                            if dx < 1.0 {
                                top = top.min(roof - 1.4 * sc - 1.5 * sc * (1.0 - dx * dx).sqrt());
                            }
                            if dx < 1.1 && dx > 0.7 {
                                top = top.min(roof - 1.4 * sc);
                            }
                        }
                        if (xc - x0).abs() < 0.6 * sc || (xc - x1).abs() < 0.6 * sc {
                            top = top.min(roof - 0.6 * sc); // parapet ends
                        }
                    }
                    if y >= top && y < st {
                        m = BLD;
                        // backlit stone, a little violet from the sky, darker low down
                        (cr, cg, cb) = (0.075, 0.05, 0.09);
                        let fy = (y - top) / f64::max(1.0, st - top);
                        cr *= 1.1 - 0.4 * fy;
                        cg *= 1.1 - 0.4 * fy;
                        cb *= 1.1 - 0.3 * fy;
                        al = 0.5;
                        // rows of arched openings, a fair number lit on the near ghats
                        if let Some(bi) = bi.filter(|_| sc > 0.6) {
                            let fx = (xc - blds[bi][0]) / (2.4 * sc);
                            let fz = (st - y) / (3.0 * sc);
                            let (ix, iz) = (fx.floor(), fz.floor());
                            if fx - ix > 0.35
                                && fx - ix < 0.7
                                && fz - iz > 0.25
                                && fz - iz < 0.75
                                && iz >= 1.0
                                && st - y < blds[bi][2] * sc - 1.5 * sc
                            {
                                let lit = hash(bi as f64 * 31.0 + ix, iz * 7.0)
                                    < if sc > 1.2 { 0.28 } else { 0.2 };
                                if lit {
                                    cr += 0.5;
                                    cg += 0.28;
                                    cb += 0.08;
                                } else {
                                    cr *= 0.55;
                                    cg *= 0.55;
                                    cb *= 0.65;
                                    al = 0.2;
                                }
                            }
                        }
                        fl = 0.14;
                    }
                    if y >= st {
                        // the steps: treads catch the violet sky, risers fall in shadow
                        m = STEPS;
                        let sp = 1.6 * sc;
                        let near = (wl - y) / (wl - st);
                        if sp < 2.2 {
                            // too far to count the steps: a faint alternating tread
                            let odd = (wl - y).floor() as i64 & 1 != 0;
                            (cr, cg, cb) = (0.19, 0.13, 0.25);
                            if odd {
                                cr *= 0.55;
                                cg *= 0.55;
                                cb *= 0.58;
                            }
                            al = if odd { 0.4 } else { 0.75 };
                        } else {
                            let ph = ((wl - y) / sp) % 1.0;
                            if ph < 0.6 {
                                (cr, cg, cb, al) = (0.24 + 0.05 * (1.0 - near), 0.15, 0.2, 1.0);
                            } else {
                                (cr, cg, cb, al) = (0.05, 0.035, 0.06, 0.3);
                            }
                        }
                        // the wet bottom steps darker
                        if wl - y < 1.2 * sc {
                            cr *= 0.6;
                            cg *= 0.6;
                            cb *= 0.75;
                            al *= 0.6;
                        }
                        fl = 0.12;
                    }
                }

                // spires
                for (&[sx, sh, sw], &base) in Self::L.spires.iter().zip(&spire_base) {
                    let h = base - y;
                    if h < -1.0 || h > sh + 2.5 {
                        continue;
                    }
                    let dx = (xc - sx).abs();
                    let f = h / sh;
                    // the curved shikhara, an amalaka disc and the kalash on top
                    let mut hw = if f <= 1.0 {
                        sw * f64::max(0.0, 1.0 - f.powf(1.8)).powf(0.6)
                    } else {
                        0.0
                    };
                    if f > 0.92 && f < 1.02 {
                        hw = hw.max(sw * 0.32);
                    }
                    if f >= 1.02 && h < sh + 2.2 {
                        hw = hw.max(0.35 + 0.2 * (((h - sh) / 2.2) * PI).sin());
                    }
                    if (-1.0..0.5).contains(&h) {
                        hw = sw * 1.15;
                    }
                    if dx <= hw {
                        m = SPIRE;
                        let band = if (h / 1.6).floor() % 2.0 != 0.0 {
                            0.8
                        } else {
                            1.0
                        };
                        (cr, cg, cb) = (0.085 * band, 0.055 * band, 0.095 * band);
                        (fl, al) = (0.12, 0.35);
                    }
                }

                // the big umbrellas on the far steps
                for &ux in Self::L.umbs {
                    let (s2, wl2) = (Self::sc_f(ux), Self::wl_f(ux));
                    let cy = wl2 - (wl2 - Self::step_top_f(ux)) * 0.55;
                    let dx = (xc - ux) / (3.0 * s2);
                    let dy = (cy - y) / (1.4 * s2);
                    if dy > 0.0 && dy < 1.0 && dx.abs() < (1.0 - dy * dy).sqrt() {
                        (m, cr, cg, cb, al) = (UMB, 0.07, 0.045, 0.075, 0.4);
                    }
                    if dy <= 0.0 && dy > -0.25 && dx.abs() < 1.0 {
                        (m, cr, cg, cb, al) = (UMB, 0.05, 0.03, 0.05, 0.2);
                    }
                    if (xc - ux).abs() < 0.5 && y > cy && y < cy + 2.2 * s2 {
                        (m, cr, cg, cb, al) = (UMB, 0.04, 0.03, 0.04, 0.1);
                    }
                }

                // the priests on their platforms, dark against the lit steps
                for &[ax, _, _] in Self::L.aarti {
                    let s2 = Self::sc_f(ax);
                    let u = 1.3 * s2;
                    let base = Self::wl_f(ax) - 2.6 * s2;
                    let dx = (xc - ax) / u;
                    let fy = (base - y) / u;
                    // a dhoti, a broad-shouldered torso, a neck and the head
                    let bw = if fy < 0.0 {
                        -1.0
                    } else if fy < 2.2 {
                        0.66 - 0.04 * fy
                    } else if fy < 4.1 {
                        0.58 + 0.32 * smooth(2.2, 3.7, fy)
                    } else if fy < 4.5 {
                        0.28
                    } else {
                        -1.0
                    };
                    let head = (dx * 1.05).hypot(fy - 5.05) < 0.62;
                    if dx.abs() < bw || head {
                        (m, cr, cg, cb, fl, al) = (FIG, 0.035, 0.022, 0.035, 0.02, 0.04);
                    }
                    if y >= base && y < base + 0.7 * s2 && (xc - ax).abs() < 2.3 * u {
                        (m, cr, cg, cb, fl, al) = (PLAT, 0.12, 0.07, 0.06, 0.04, 0.8);
                    }
                }

                if m == SKY {
                    let (xf, rf) = (xi as f64, r as f64);
                    let veil = 0.92 + 0.16 * fbm(xf * 0.05, rf * 0.1, 3, 0.0);
                    let hh = Self::sky_heat(xc, y)
                        + 0.05 * (fbm(xf * 0.07 + 11.0, rf * 0.16, 2, 0.0) - 0.5)
                        + 0.1 * (fbm(xf * 0.025 + 3.0, rf * 0.09 + 5.0, 3, 0.0) - 0.5);
                    warm[k] = smooth(0.3, 0.9, hh) as f32;
                    let [a, b, c] = rgb(ramp_at(hh));
                    (cr, cg, cb) = (a * veil, b * veil, c * veil);
                    fl = 0.2;
                }
                mat[k] = m;
                s[0][k] = cr as f32;
                s[1][k] = cg as f32;
                s[2][k] = cb as f32;
                flo[k] = fl as f32;
                alb[k] = al as f32;
            }
        }

        // rim light: silhouette edges facing the glow, and their tops, take the sky's colour
        let mut rim = [vec![0f32; n], vec![0f32; n], vec![0f32; n]];
        for r in 1..H - 1 {
            for x in 2..Self::W - 2 {
                let k = r * Self::W + x;
                let m = mat[k];
                if m == SKY || m == WATER {
                    continue;
                }
                // the edge facing the glow takes its orange light, the top the sky's colour
                let toward = |o: usize| if (x as f64) < Self::L.glow[0] { k + o } else { k - o };
                let strong = 0.75 * (-(x as f64 - Self::L.glow[0]).abs() / 42.0).exp();
                let mut a = 0.0;
                if mat[toward(1)] == SKY {
                    a = strong;
                } else if mat[toward(2)] == SKY {
                    a = strong * 0.4;
                }
                if a > 0.0 {
                    rim[0][k] = (0.85 * a) as f32;
                    rim[1][k] = (0.45 * a) as f32;
                    rim[2][k] = (0.26 * a) as f32;
                }
                if mat[k - Self::W] == SKY {
                    for c in 0..3 {
                        rim[c][k] = (f64::from(rim[c][k]) + f64::from(s[c][k - Self::W]) * 0.3) as f32;
                    }
                }
            }
        }
        for c in 0..3 {
            for k in 0..n {
                s[c][k] = (f64::from(s[c][k]) + f64::from(rim[c][k])) as f32;
            }
        }

        // small diyas lining the steps near the aarti, twinkling
        let mut step_lamps = Vec::new();
        for xi in 2..Self::L.step_lamps.0 {
            let x = xi as f64;
            let sc = Self::sc_f(x + 0.5);
            let mut r = Self::step_top_f(x + 0.5).floor();
            while r < Self::wl_f(x + 0.5) - 1.0 {
                let k = r as usize * Self::W + xi;
                if mat[k] == STEPS {
                    let [nc, ns] = Self::L.step_lamps.1;
                    let near = (-((x - nc) / ns).powi(2)).exp();
                    if hash(x * 3.0 + 7.0, r * 5.0) < 0.035 * near + 0.006
                        && ((Self::wl_f(x + 0.5) - r - 0.5) / (1.6 * sc)) % 1.0 < 0.3
                    {
                        step_lamps.push((k, hash(x, r) * 40.0));
                    }
                }
                r += 1.0;
            }
        }

        // electric lamps along the far ghats, each with its thin road on the water
        let mut sref = vec![0f32; n];
        let mut x = Self::L.lamps_from;
        while x < Self::L.end - 3.0 {
            let sc = Self::sc_f(x);
            let ly =
                Self::step_top_f(x) - 0.4 * sc + (Self::wl_f(x) - Self::step_top_f(x)) * 0.55 * hash(x, 78.0).powi(2);
            let halo = 1.0 + 3.2 * sc;
            let rr = (halo * 3.0).ceil();
            let mut r = (ly - rr).floor().max(0.0);
            while r < HF.min(ly + rr) {
                let mut xx = (x - rr).floor().max(0.0);
                while xx < Self::WF.min(x + rr) {
                    let k = r as usize * Self::W + xx as usize;
                    if mat[k] != WATER && mat[k] != SKY {
                        let (dx, dy) = (xx + 0.5 - x, r + 0.5 - ly);
                        let d = (dx * dx + dy * dy).sqrt();
                        // the lamp, and the pool of light it throws on the stone round it
                        let v = (-(d / 0.6).powi(2)).exp() * 1.3 + (-d / 1.4).exp() * 0.12;
                        let p = (-d / halo).exp() * 0.3 * f64::from(alb[k]);
                        s[0][k] = (f64::from(s[0][k]) + (v + p * WASH[0])) as f32;
                        s[1][k] = (f64::from(s[1][k]) + (v * 0.76 + p * WASH[1])) as f32;
                        s[2][k] = (f64::from(s[2][k]) + (v * 0.42 + p * WASH[2])) as f32;
                    }
                    xx += 1.0;
                }
                r += 1.0;
            }
            let wl = Self::wl_f(x);
            let (width, len) = (0.35 + 0.35 * sc, 9.0 * sc);
            let mut r = wl.floor();
            while r < HF {
                let a = (-(r + 0.5 - wl) / len).exp() * 0.5;
                if a < 0.01 {
                    break;
                }
                let mut xx = (x - 3.0).floor();
                while xx < x + 3.0 {
                    let k = r as usize * Self::W + xx as usize;
                    sref[k] =
                        (f64::from(sref[k]) + a * (-((xx + 0.5 - x) / width).powi(2)).exp()) as f32;
                    xx += 1.0;
                }
                r += 1.0;
            }
            x += (5.0 + 10.0 * hash(x, 77.0)) * f64::max(0.8, sc);
        }

        // --- the reflection: the bank and sky mirrored at the waterline ---
        let mut refl = [vec![0f32; n], vec![0f32; n], vec![0f32; n]];
        let mut mir_k = vec![-1i32; n];
        for r in 0..H {
            for xi in 0..Self::W {
                let k = r * Self::W + xi;
                if mat[k] != WATER {
                    continue;
                }
                let x = xi as f64;
                let wl = Self::wl_f(x + 0.5);
                let d = r as f64 + 0.5 - wl;
                let (mut ar, mut ag, mut ab) = (0.0, 0.0, 0.0);
                for o in 0..2 {
                    let of = f64::from(o);
                    let ym = wl - d * 0.85 - 0.4 - of * 0.8;
                    let q = if ym < 0.0 {
                        -1
                    } else {
                        ym.floor() as i64 * Self::W as i64 + xi as i64
                    };
                    // the rippled water stretches the sky's light down toward us
                    let sky = rgb(ramp_at(Self::sky_heat(
                        x + 0.5,
                        f64::max(0.0, wl - d * 0.36 - 1.0 - of * 0.6),
                    )));
                    let (cr, cg, cb);
                    if q < 0 || mat[q as usize] == WATER || mat[q as usize] == SKY {
                        [cr, cg, cb] = sky;
                    } else {
                        // tilted ripples under the bank still catch a little sky
                        let q = q as usize;
                        cr = f64::from(s[0][q]) + sky[0] * 0.22;
                        cg = f64::from(s[1][q]) + sky[1] * 0.22;
                        cb = f64::from(s[2][q]) + sky[2] * 0.22;
                        if o == 0 {
                            mir_k[k] = q as i32;
                        }
                    }
                    ar += cr;
                    ag += cg;
                    ab += cb;
                }
                let a = 0.8 - 0.4 * smooth(HZ, HF, r as f64);
                refl[0][k] = ((ar / 2.0) * a) as f32;
                refl[1][k] = ((ag / 2.0) * a) as f32;
                refl[2][k] = ((ab / 2.0) * a) as f32;
            }
        }

        // --- clouds: long dusk streaks, dark on top and lit from below ---
        let mut cover = vec![0f32; CW * CH];
        let mut clit = vec![0f32; CW * CH];
        for r in 0..CH {
            for x in 0..CW {
                let (xf, y) = (x as f64, r as f64 + 0.5);
                let d = density(xf, y);
                cover[r * CW + x] = smooth(0.47, 0.62, d) as f32;
                // the undersides catch the light from below the horizon, the tops go dark
                clit[r * CW + x] = clamp(
                    0.4 + (d - density(xf, y + 2.0)) * 5.5 + (density(xf, y - 2.5) - d) * 1.5,
                ) as f32;
            }
        }

        // --- the floating diyas ---
        let diyas = (0..Self::L.diyas.0)
            .map(|i| {
            let i = i as f64;
            let q = hash(i, 61.0);
            let y = HZ + 5.0 + 34.0 * q.powf(1.3);
            [
                hash(i, 62.0) * Self::L.diyas.1 - 30.0,
                y,
                hash(i, 63.0) * 50.0,
                0.6 + 0.6 * hash(i, 64.0),
            ]
        })
            .collect();

        Self {
            dots: Dots::new(&Self::PALETTE[..STAR]),
            rt,
            mat,
            s,
            flo,
            alb,
            warm,
            step_lamps,
            sref,
            refl,
            mir_k,
            cover,
            clit,
            diyas,
            dynl: [vec![0f32; n], vec![0f32; n], vec![0f32; n]],
            dref: vec![0f32; n],
            arm: vec![0u8; n],
        }
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        for c in &mut self.dynl {
            c.fill(0.0);
        }
        self.dref.fill(0.0);

        // aarti: each lamp is raised and turned in slow circles
        self.arm.fill(0);
        for (i, &[ax, sw, sl]) in Self::L.aarti.iter().enumerate() {
            let fi = i as f64;
            let s = Self::sc_f(ax);
            let u = 1.3 * s;
            let base = Self::wl_f(ax) - 2.6 * s;
            let ph = t * 1.1 + fi * 1.3;
            let fx = ax + 0.9 * u + ph.cos() * 1.1 * u;
            let fy = base - 7.2 * u + ph.sin() * 0.6 * u;
            // the arm from the shoulder to the lamp
            let (sx0, sy0) = (ax + 0.62 * u, base - 3.8 * u);
            let (ex, ey) = (fx - sx0, fy + 0.6 * u - sy0);
            let el = ex * ex + ey * ey;
            let mut r = (sy0.min(fy) - 1.0).floor();
            while r <= sy0.max(fy) + 1.0 {
                let mut x = (sx0.min(fx) - 1.0).floor();
                while x <= sx0.max(fx) + 1.0 {
                    if (0.0..Self::WF).contains(&x) && (0.0..HF).contains(&r) {
                        let (px, py) = (x + 0.5 - sx0, r + 0.5 - sy0);
                        let q = clamp((px * ex + py * ey) / el);
                        if (px - q * ex).hypot(py - q * ey) < 0.28 * u {
                            self.arm[r as usize * Self::W + x as usize] = 1;
                        }
                    }
                    x += 1.0;
                }
                r += 1.0;
            }
            let fl = 0.85 + 0.15 * (t * 13.0 + fi * 5.0).sin() * (t * 7.3 + fi).sin();
            Self::add_glow(&mut self.dynl, &self.mat, fx, fy, 0.6 * u, 1.7 * u, fl);
            Self::add_wash(
                &mut self.dynl,
                &self.alb,
                fx,
                fy + 2.0 * u,
                7.5 * s,
                0.34 * fl,
            );
            Self::add_streak(
                &mut self.dref,
                &self.mat,
                fx,
                Self::wl_f(fx),
                0.8 * s * sw,
                15.0 * s * sl,
                0.75 * fl,
            );
        }
        // the step lamps
        for &(k, ph) in &self.step_lamps {
            let v = 0.7 + 0.3 * (t * 5.0 + ph).sin();
            let d = &mut self.dynl;
            d[0][k] = (f64::from(d[0][k]) + v * FLAME[0]) as f32;
            d[1][k] = (f64::from(d[1][k]) + v * FLAME[1] * 0.95) as f32;
            d[2][k] = (f64::from(d[2][k]) + v * FLAME[2]) as f32;
        }
        // diyas drifting downstream, nearer ones faster
        for &[x0, y, ph, sz] in &self.diyas {
            let near = (y - HZ) / (HF - HZ);
            let x = ((x0 + t * (0.25 + 0.9 * near)) % Self::L.diyas.1 + Self::L.diyas.1) % Self::L.diyas.1 - 30.0;
            if !(-3.0..=Self::WF + 3.0).contains(&x) || y < Self::wl_f(x) + 0.8 {
                continue;
            }
            let fl = 0.8 + 0.2 * (t * 9.0 + ph).sin();
            let s = (0.35 + 0.9 * near) * sz;
            Self::add_glow(
                &mut self.dynl,
                &self.mat,
                x,
                y - 0.3,
                0.45 + 0.3 * s,
                0.6 + 1.2 * s,
                fl * 0.8,
            );
            Self::add_streak(
                &mut self.dref,
                &self.mat,
                x,
                y + 0.2,
                0.3 + 0.35 * s,
                2.0 + 5.0 * s,
                0.6 * fl,
            );
        }

        let drift = t * 0.9 + Self::L.drift; // starts with the long streaks over the glow
        let [b0, bw] = Self::L.boat;
        let bx = ((b0 - t * 0.1 + 24.0) % bw + bw) % bw - 24.0;
        let by = 71.0 + (t * 0.9).sin() * 0.15;
        let rt = &self.rt;
        let rgb = |i: usize| [f64::from(rt[i]), f64::from(rt[i + 1]), f64::from(rt[i + 2])];

        for r in 0..H {
            let rf = r as f64;
            let y = rf + 0.5;
            for xi in 0..Self::W {
                let x = xi as f64;
                let k = r * Self::W + xi;
                let m = self.mat[k];
                let mut cr = f64::from(self.s[0][k]);
                let mut cg = f64::from(self.s[1][k]);
                let mut cb = f64::from(self.s[2][k]);
                let mut floor = f64::from(self.flo[k]);
                let mut fade = 1.0;
                let mut star = false;

                if m == SKY {
                    if r < CH {
                        let sx = x + drift;
                        let ix = sx.floor();
                        let fx = sx - ix;
                        let ix = ix as usize;
                        let i0 = r * CW + ix % CW;
                        let i1 = r * CW + (ix + 1) % CW;
                        let (c0, c1) = (f64::from(self.cover[i0]), f64::from(self.cover[i1]));
                        let c = c0 + (c1 - c0) * fx;
                        if c > 0.01 {
                            // high streaks glow rose from below; low ones stand dark
                            // against the afterglow with only their undersides lit,
                            // gold near the glow
                            let (l0, l1) = (f64::from(self.clit[i0]), f64::from(self.clit[i1]));
                            let l = l0 + (l1 - l0) * fx;
                            let lo = smooth(12.0, 30.0, y);
                            let lit = mix(0.3 + 0.7 * l, 0.9 * l * l, lo);
                            let ramp =
                                rgb(ramp_at(0.56 + 0.38 * f64::from(self.warm[k]) + 0.08 * l));
                            let kr = mix(0.075, ramp[0], lit);
                            let kg = mix(0.05, ramp[1], lit);
                            let kb = mix(0.15, ramp[2], lit);
                            cr = mix(cr, kr, c);
                            cg = mix(cg, kg, c);
                            cb = mix(cb, kb, c);
                            floor = mix(floor, 0.06, c * (1.0 - lit));
                        } else if y < 24.0 && hash(x, rf * 3.0 + 11.0) > 0.993 {
                            let tw = 0.4
                                + 0.25 * (t * (1.3 + hash(x, rf) * 2.0) + hash(rf, x) * 6.28).sin();
                            cr = cr.max(tw * 0.9);
                            cg = cg.max(tw * 0.88);
                            cb = cb.max(tw);
                            star = true;
                        }
                    }
                } else if m == WATER {
                    let v = (y - HZ) / (HF - HZ);
                    let w = 0.6 * noise(x * 0.05 + t * 0.08, y * 0.45 - t * 0.45, 0.0)
                        + 0.4 * noise(x * 0.16 - t * 0.15, y * 0.95 - t * 0.9, 0.0);
                    let swell = 0.55 + 0.9 * w;
                    cr = 0.035 * swell;
                    cg = 0.045 * swell;
                    cb = 0.12 * swell;
                    let wob =
                        (noise(x * 0.04 + 7.0, y * 0.3 - t * 0.6, 0.0) - 0.5) * (1.2 + 4.0 * v);
                    let sx = (x + wob).clamp(0.0, Self::WF - 1.001);
                    let i0 = r * Self::W + sx as usize;
                    let fx = sx - sx.trunc();
                    let dash = smooth(0.25, 0.75, noise(x * 0.12 + 3.0, y * 0.8 - t * 1.1, 0.0));
                    let rf_at = |a: &[f32]| {
                        let (a0, a1) = (f64::from(a[i0]), f64::from(a[i0 + 1]));
                        a0 + (a1 - a0) * fx
                    };
                    // broken bands, except in the bright reach under the glow
                    let reach = (-((x + 0.5 - Self::L.glow[0]) / 15.0).powi(2)).exp()
                        * smooth(HZ + 2.5, HZ + 5.5, y);
                    let da = mix(0.15 + 1.1 * dash, 0.7 + 0.5 * dash, reach);
                    cr += rf_at(&self.refl[0]) * da;
                    cg += rf_at(&self.refl[1]) * da;
                    cb += rf_at(&self.refl[2]) * da;
                    // the lamplit stone above, given back in broken bands
                    let q = self.mir_k[k];
                    if q >= 0 {
                        let q = q as usize;
                        cr += f64::from(self.dynl[0][q]) * 0.4 * da;
                        cg += f64::from(self.dynl[1][q]) * 0.4 * da;
                        cb += f64::from(self.dynl[2][q]) * 0.4 * da;
                    }
                    let fl = (rf_at(&self.dref) + rf_at(&self.sref)) * (0.2 + 1.1 * dash);
                    cr += fl * FLAME[0];
                    cg += fl * FLAME[1];
                    cb += fl * FLAME[2];
                    // a narrow road of glitter straight under the glow
                    let gw = 1.3 + (y - HZ) * 0.15;
                    let gx = (x + 0.5 - Self::L.glow[0]) / gw;
                    if gx > -3.0 && gx < 3.0 {
                        let road = (-gx * gx).exp() * (-(y - HZ) / 24.0).exp();
                        let rip = noise(x * 0.45 + y * 0.1 - t * 0.3, y * 1.3 - t * 1.6, 0.0);
                        let glint = smooth(0.42, 0.78, 0.45 * w + 0.55 * rip) * road;
                        cr += 1.0 * glint + 0.12 * road;
                        cg += 0.78 * glint + 0.07 * road;
                        cb += 0.42 * glint + 0.04 * road;
                    }
                    floor = 0.12;
                    fade = smooth(HF + 4.0, HF - 16.0, y);
                }

                // the boat and its dark reflection
                let lx = x + 0.5 - bx;
                if lx > -14.0 && lx < 15.0 && rf > by - 11.0 && rf < by + 11.0 {
                    let (mut cov, mut rc) = (0.0, 0.0);
                    for sy2 in 0..2 {
                        for sx2 in 0..2 {
                            let px = (lx - 0.25 + f64::from(sx2) * 0.5) / BS;
                            let py = (by - (rf + 0.25 + f64::from(sy2) * 0.5)) / BS;
                            cov += boat_at(px, py);
                            if m == WATER {
                                rc += boat_at(px, -py * 0.9);
                            }
                        }
                    }
                    cov /= 4.0;
                    rc /= 4.0;
                    if cov > 0.0 {
                        cr = mix(cr, 0.035, cov);
                        cg = mix(cg, 0.022, cov);
                        cb = mix(cb, 0.035, cov);
                        floor = mix(floor, 0.02, cov);
                        star = false;
                    } else if rc > 0.0 {
                        cr *= 1.0 - 0.8 * rc;
                        cg *= 1.0 - 0.8 * rc;
                        cb *= 1.0 - 0.75 * rc;
                    }
                }

                let d = [
                    f64::from(self.dynl[0][k]),
                    f64::from(self.dynl[1][k]),
                    f64::from(self.dynl[2][k]),
                ];
                if self.arm[k] != 0 {
                    cr = 0.035 + d[0] * 0.12;
                    cg = 0.022 + d[1] * 0.12;
                    cb = 0.035 + d[2] * 0.12;
                    floor = 0.02;
                } else {
                    cr += d[0];
                    cg += d[1];
                    cb += d[2];
                }

                let peak = cr.max(cg).max(cb).max(1e-4);
                let level = clamp(floor + (1.0 - floor) * peak.powf(0.72) * 0.95) * fade;
                let step = Dots::step(level, bayer(r, xi));
                let c = if star {
                    STAR as u8
                } else {
                    let want = Dots::want(step, level, 0.06);
                    let s = f64::min(LIFT[step], (0.3 + 0.7 * want) / peak);
                    self.dots
                        .nearest(clamp(cr * s), clamp(cg * s), clamp(cb * s))
                };
                out[k] = Cell::new(font::HALFTONE[step], u16::from(c));
            }
        }
    }
}
