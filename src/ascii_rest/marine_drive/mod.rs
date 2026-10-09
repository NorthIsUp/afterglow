//! marine drive: the queen's necklace seen from malabar hill at night. A string
//! of sodium lamps sweeps round the bay to the towers at nariman point, cars
//! trail light along the road beneath them, and the bay carries their long
//! wavering reflections. Ships ride at anchor on the horizon.
//!
//! Everything along the curve is placed by distance along it, so buildings,
//! lamps and cars shrink together toward the point. Upstream's
//! `Float32Array`s stay `f32` here: their rounding is part of the picture.
//!
//! At any size the bay runs from the hill at the left to the point, with open
//! sea beyond it: wider panels lengthen the bay, so the necklace carries more
//! lamps and blocks round to a point further east, more towers rise
//! mid-curve, and the open sea has room for a liner and a fishing boat beyond
//! the freighter; narrower ones shorten it to fewer blocks and a smaller
//! cluster at the point; taller ones add sky above, where the moon climbs, and
//! bay below.

mod layout;

use super::halftone::{bayer, Dots};
use super::math::{clamp, fbm, hash, js_round, mix, noise, smooth};
use super::{hex, Piece};
use crate::grid::Cell;
use layout::Layout;

const MOON_R: f64 = 4.5;
/// Darker seas on the moon's face: [dx, dy, radius].
const MARIA: [[f64; 3]; 3] = [[-1.4, -1.2, 1.6], [1.3, 0.8, 1.4], [-0.6, 2.0, 1.1]];
const SODIUM: [f64; 3] = [1.0, 0.56, 0.18];

const SKY: u8 = 0;
const WATER: u8 = 1;
const FAR: u8 = 2;
const TOWER: u8 = 3;
const FRONT: u8 = 4;
const ROAD: u8 = 5;
const WALL: u8 = 6;
const POLE: u8 = 7;
const SHIP: u8 = 8;

/// One lamp near us is failing.
const FLICK: usize = 6;
/// Cloud field width: the clouds wrap at this many columns.
const CW: usize = 800;
const BIN: f64 = 0.2;
const U0: f64 = -6.0;
const STEP: f64 = 0.25;

/// Distance along the drive at column x, from the f32 table.
fn u_of(utab: &[f32], x: f64) -> f64 {
    let f = 0.0f64.max((utab.len() as f64 - 1.001).min(x / STEP));
    let i = f as usize;
    let (a, b) = (f64::from(utab[i]), f64::from(utab[i + 1]));
    a + (b - a) * (f - i as f64)
}

struct Building {
    u0: f64,
    u1: f64,
    floors: f64,
    tone: f64,
    crown: bool,
}

/// A room whose light changes.
struct Room {
    k: usize,
    w: [f64; 3],
    tv: bool,
    rate: f64,
    ph: f64,
}

pub struct MarineDrive {
    l: Layout,
    dots: Dots,
    mat: Vec<u8>,
    sr: Vec<f32>,
    sg: Vec<f32>,
    sb: Vec<f32>,
    flo: Vec<f32>,
    /// 1 far lane, 2 near lane, 3 both.
    lane: Vec<u8>,
    sky_glow: Vec<f32>,
    rooms: Vec<Room>,
    /// Red lights on the tallest towers: [cell, phase].
    beacons: Vec<(isize, f64)>,
    lamp_i: Vec<f32>,
    lamp_w: Vec<f32>,
    flick_i: Vec<f32>,
    flick_w: Vec<f32>,
    refl: [Vec<f32>; 3],
    refl_fl: Vec<f32>,
    cover: Vec<f32>,
    clit: Vec<f32>,
    clear: Vec<f32>,
    uend: f64,
    lane_a: Vec<f32>,
    lane_b: Vec<f32>,
    /// [position, speed, direction].
    cars: Vec<[f64; 3]>,
    bin_lo: Vec<i32>,
    bin_hi: Vec<i32>,
    col_a: Vec<f32>,
    col_b: Vec<f32>,
}

/// Redraw one cell over the frame, unfaded.
fn paint(dots: &mut Dots, out: &mut [Cell], w: usize, k: usize, rgb: [f64; 3], floor: f64) {
    let [cr, cg, cb] = rgb.map(|v| v.max(0.0));
    let peak = cr.max(cg).max(cb).max(1e-4);
    let level = clamp(floor + (1.0 - floor) * peak.powf(0.85) * 0.95);
    let step = Dots::step(level, bayer(k / w, k % w));
    out[k] = dots.ink(step, level, [cr, cg, cb], peak);
}

impl Piece for MarineDrive {
    const NAME: &'static str = "marine-drive";
    const FPS: u32 = 15;
    const GROUND: u32 = hex("#07080f");
    #[rustfmt::skip]
    const PALETTE: &'static [u32] = &[
        hex("#18213f"), hex("#202c55"), hex("#2a396d"), hex("#374a88"), hex("#4a60a2"), hex("#6880bc"),
        hex("#2c2140"), hex("#3f2d58"), hex("#56396b"), hex("#6e4a7a"), hex("#8a5f88"),
        hex("#5a2e3c"), hex("#7b404c"), hex("#9e5858"),
        hex("#55300f"), hex("#7d4515"), hex("#a95c1b"), hex("#d27a24"), hex("#ee9631"), hex("#ffb246"), hex("#ffcd6a"), hex("#ffe39c"), hex("#fff2cf"),
        hex("#ffffff"), hex("#dce6ff"), hex("#a9bde8"),
        hex("#6a181b"), hex("#b22a28"), hex("#ff4b3e"),
        hex("#132c38"), hex("#1c3f4f"), hex("#2a5868"),
        hex("#29283a"), hex("#3c3a4c"), hex("#565266"), hex("#79738a"),
    ];

    fn new(cols: usize, rows: usize) -> Self {
        let l = Layout::new(cols, rows);
        let (w, h, hz) = (l.w, l.h, l.hz);
        let n = w * h;
        let hf = h as f64;

        // --- distance along the drive
        let nu = ((l.tip + 4.0) / STEP).ceil() as usize + 2;
        let mut utab = vec![0f32; nu];
        for i in 1..nu {
            let x = (i as f64 - 0.5) * STEP;
            let sl = (l.shore(x + 0.05) - l.shore(x - 0.05)) / 0.1;
            utab[i] = (f64::from(utab[i - 1]) + ((1.0 + sl * sl).sqrt() * STEP) / l.sc(x)) as f32;
        }
        let uof = |x: f64| u_of(&utab, x);
        let x_of_u = |u: f64| {
            let (mut lo, mut hi) = (0.0, l.tip);
            for _ in 0..30 {
                let m = (lo + hi) / 2.0;
                if uof(m) < u {
                    lo = m;
                } else {
                    hi = m;
                }
            }
            (lo + hi) / 2.0
        };
        let uend = uof(l.tip);

        // --- the lamps
        let mut lamps: Vec<[f64; 3]> = Vec::new();
        let mut u = 0.6;
        while u < uend - 0.4 {
            let lx = x_of_u(u);
            let s = l.sc(lx);
            lamps.push([lx, l.wall_top(lx) - 2.4 * s, s]);
            u += 3.75;
        }

        // --- the front row: art deco blocks along the drive
        let mut blds: Vec<Building> = Vec::new();
        let mut u = -4.0;
        while u < uend {
            let i = blds.len() as f64;
            let span = 7.0 + 7.0 * hash(i, 21.0);
            // art deco blocks of five to eight storeys, and towers at the near, hilly end
            let tall = js_round(7.0 * smooth(26.0, 2.0, u) * (0.6 + 0.4 * hash(i, 26.0)));
            blds.push(Building {
                u0: u,
                u1: u + span,
                floors: 5.0 + (hash(i, 22.0) * 4.0).floor() + tall,
                tone: 0.75 + 0.5 * hash(i, 23.0),
                crown: hash(i, 24.0) > 0.45,
            });
            u += span + 0.4 + 1.8 * hash(i, 25.0);
        }
        let mut col_u = vec![0f32; w];
        let mut bld_at = vec![-1isize; w];
        for x in 0..w {
            col_u[x] = uof(x as f64 + 0.5) as f32;
            if x as f64 + 0.5 > l.tip {
                continue;
            }
            let cu = f64::from(col_u[x]);
            for (i, b) in blds.iter().enumerate() {
                if cu >= b.u0 && cu < b.u1 {
                    bld_at[x] = i as isize;
                }
            }
        }

        // --- the static picture
        let mut mat = vec![SKY; n];
        let mut sr = vec![0f32; n];
        let mut sg = vec![0f32; n];
        let mut sb = vec![0f32; n];
        let mut flo = vec![0f32; n];
        let mut lane = vec![0u8; n];
        let mut sky_glow = vec![0f32; n];
        let mut rooms: Vec<Room> = Vec::new();
        let mut beacons: Vec<(isize, f64)> = Vec::new();

        for r in 0..h {
            for x in 0..w {
                let k = r * w + x;
                let (xf, rf) = (x as f64, r as f64);
                let (xc, y) = (xf + 0.5, rf + 0.5);
                if y >= l.sea_top(xc) {
                    mat[k] = WATER;
                    continue;
                }
                let (mut m, mut cr, mut cg, mut cb, mut fl) = (SKY, 0.0, 0.0, 0.0, 0.15);

                let blk = ((xf + 40.0 * fbm(xf * 0.02, 5.0, 2, 0.0)) / 3.0).floor();
                let far_top = hz
                    - 0.4
                    - (0.4 + 5.0 * hash(blk, 7.0).powi(3))
                        * (0.5 + 0.5 * fbm(xf * 0.05, 2.0, 2, 0.0));
                if xc < l.tip + 7.0 && y >= far_top {
                    // the far city: low blocks in the haze, pricked with tiny lights
                    m = FAR;
                    let g = l.glow(xc, hz - 2.0);
                    cr = 0.03 + 0.05 * g;
                    cg = 0.04 + 0.03 * g;
                    cb = 0.08 + 0.02 * g;
                    let hl = hash(xf * 3.0 + 1.0, rf * 7.0 + 2.0);
                    if hl < 0.035 {
                        let v = 0.25 + 0.45 * hash(xf, rf * 5.0 + 9.0);
                        let cool = hash(xf * 5.0, rf) < 0.3;
                        cr += v * if cool { 0.8 } else { 1.0 };
                        cg += v * if cool { 0.85 } else { 0.7 };
                        cb += v * if cool { 1.0 } else { 0.4 };
                    }
                    fl = 0.08;
                }

                for (ti, &[tx, hw, top, warm]) in l.towers.iter().enumerate() {
                    let dx = xc - tx;
                    if dx.abs() > hw || y < top {
                        continue;
                    }
                    let tf = ti as f64;
                    m = TOWER;
                    // dark glass, its moonward edge catching a little light
                    (cr, cg, cb) = (0.015, 0.018, 0.035);
                    if dx > hw - 1.0 {
                        cr += 0.04;
                        cg += 0.055;
                        cb += 0.1;
                    }
                    let g = l.glow(xc, y) * 0.2;
                    cr += g * 0.42;
                    cg += g * 0.21;
                    cb += g * 0.1;
                    let fr = rf - top;
                    if fr == 0.0 {
                        // the rooftop's edge
                        cr += 0.08;
                        cg += 0.1;
                        cb += 0.16;
                    } else if fr > 0.0 && fr % 2.0 == 1.0 && dx.abs() < hw - 0.5 {
                        let seg = ((xc - tx + hw) / 3.0).floor();
                        let h = hash(tf * 17.0 + seg, rf * 13.0 + 3.0);
                        if h < 0.5 {
                            let v = 0.4 + 0.3 * hash(tf * 7.0 + seg, rf);
                            let cool = if warm != 0.0 { h < 0.1 } else { h > 0.1 };
                            cr += v * if cool { 0.72 } else { 1.0 };
                            cg += v * if cool { 0.84 } else { 0.72 };
                            cb += v * if cool { 1.0 } else { 0.42 };
                        }
                    }
                    if fr == 0.0 && dx.abs() < 0.6 && top < 20.0 + l.sky {
                        beacons.push((k as isize - w as isize, tf * 1.7));
                    }
                    fl = 0.03;
                }

                if xc <= l.tip {
                    let (s, wt, rt) = (l.sc(xc), l.wall_top(xc), l.road_top(xc));
                    let bi = bld_at[x];
                    if bi >= 0 && y < rt {
                        let bu = bi as usize;
                        let bf = bi as f64;
                        let b = &blds[bu];
                        let (uu, bw) = (f64::from(col_u[x]) - b.u0, b.u1 - b.u0);
                        let f = (rt - y) / s; // height above the road, in storeys of 1.7
                        let mut hgt = b.floors * 1.7 + 1.1;
                        if b.crown && uu > bw * 0.32 && uu < bw * 0.68 {
                            hgt += 1.6;
                        }
                        if f < hgt {
                            m = FRONT;
                            let edge = uu < 0.5 || uu > bw - 0.5;
                            let t0 = b.tone * if edge { 1.35 } else { 1.0 };
                            // pale art deco plaster, warm where the street lamps reach it
                            (cr, cg, cb) = (0.03 * t0, 0.035 * t0, 0.06 * t0);
                            let up = (-f / 2.0).exp() * 0.16;
                            cr += up * SODIUM[0];
                            cg += up * SODIUM[1];
                            cb += up * SODIUM[2];
                            if f > hgt - 0.6 / s
                                || (f > b.floors * 1.7 + 0.6 && f < b.floors * 1.7 + 0.6 + 0.5 / s)
                            {
                                // cornice, moonlit
                                cr += 0.1;
                                cg += 0.13;
                                cb += 0.22;
                            }
                            if uu > bw - 0.7 / s - 0.2 {
                                // the corner toward the moon
                                cr += 0.04;
                                cg += 0.06;
                                cb += 0.12;
                            }
                            let fi = ((f - 0.9) / 1.7).floor();
                            let ff = (f - 0.9) - fi * 1.7;
                            let col = ((uu - 0.3) / 1.5).floor();
                            let cu = uu - 0.3 - col * 1.5;
                            let fine = s * 1.7 >= 2.6;
                            let in_win = fi >= 0.0
                                && fi < b.floors
                                && uu > 0.7
                                && uu < bw - 0.7
                                && (!fine || (ff > 0.45 && ff < 1.35 && cu > 0.35 && cu < 1.15));
                            if in_win && !fine {
                                // too far to count rooms: each storey reads as a band of light,
                                // broken where rooms are dark
                                let v = 0.04
                                    + 0.26
                                        * hash(bf * 13.0 + (uu / 3.2).floor(), fi * 7.0 + 1.0)
                                            .powi(2);
                                cr += v;
                                cg += v * 0.8;
                                cb += v * 0.55;
                                if hash(xf * 7.0 + 3.0, rf * 11.0) < 0.035 {
                                    cr += 0.4;
                                    cg += 0.36;
                                    cb += 0.28;
                                }
                            } else if in_win {
                                let h = hash(bf * 97.0 + col, fi * 31.0 + 5.0);
                                let lit = h < if fine { 0.3 } else { 0.2 };
                                if lit {
                                    let v = (0.35 + 0.3 * hash(bf * 13.0 + col, fi))
                                        * if fine { 1.0 } else { 0.7 };
                                    let kind = hash(bf * 7.0 + col, fi * 3.0 + 1.0);
                                    // tungsten, paler than the lamps; or a tube light
                                    let (wr, wg, wb) = if kind < 0.25 {
                                        (0.82, 0.9, 1.0)
                                    } else {
                                        (1.0, 0.84, 0.58)
                                    };
                                    cr += v * wr;
                                    cg += v * wg;
                                    cb += v * wb;
                                    let fk = hash(bf * 5.0 + col, fi * 11.0 + 7.0);
                                    if fk < 0.07 {
                                        rooms.push(Room {
                                            k,
                                            w: [v * wr, v * wg, v * wb],
                                            tv: fk < 0.025,
                                            rate: 0.3 + fk * 9.0,
                                            ph: fk * 400.0,
                                        });
                                    }
                                } else if fine {
                                    cr *= 0.7;
                                    cg *= 0.7;
                                    cb *= 0.8;
                                }
                            }
                            fl = 0.03;
                        }
                    }
                    if y >= rt && y < wt {
                        m = ROAD;
                        (cr, cg, cb) = (0.07, 0.05, 0.05);
                        fl = 0.1;
                        let lf = (y - rt) / (wt - rt);
                        lane[k] = if wt - rt < 1.8 {
                            3
                        } else if lf < 0.5 {
                            1
                        } else {
                            2
                        };
                    } else if y >= wt {
                        m = WALL; // the sea wall and the promenade along it
                        (cr, cg, cb) = (0.1, 0.09, 0.1);
                        fl = 0.1;
                    }
                    for &[lx, ly, ls] in &lamps {
                        if ls > 1.15 && (xc - lx).abs() < 0.5 && y > ly && y < wt {
                            m = POLE;
                            (cr, cg, cb) = (0.09, 0.08, 0.09);
                            fl = 0.1;
                        }
                    }
                }

                if m == SKY {
                    // navy overhead to a sodium haze on the horizon
                    let v = y / hz;
                    // brighter over the point, so its towers stand dark against the haze
                    let (pg, ph) = (l.point_glow, l.point_haze);
                    let g = l.glow(xc, y)
                        * (1.0 + 0.6 * smooth(pg[0], pg[1], xc) * smooth(pg[2], pg[3], xc))
                        + 0.2
                            * (-(hz - y).max(0.0) / 14.0).exp()
                            * smooth(ph[0], ph[1], xc)
                            * smooth(ph[2], ph[3], xc);
                    let veil = 0.88 + 0.24 * fbm(xf * 0.05, rf * 0.08, 3, 0.0);
                    // the last of the blue hour, deepening overhead
                    let vv = v.powf(1.4);
                    cr = (0.03 + 0.07 * vv) * veil + g * 0.42;
                    cg = (0.05 + 0.12 * vv) * veil + g * 0.22;
                    cb = (0.15 + 0.24 * vv) * veil + g * 0.1;
                    // a pale sea haze on the open horizon, for the ships to sit against
                    let hzn = (-(hz - y).max(0.0) / 3.5).exp() * smooth(l.open_sea[0], l.open_sea[1], xc) * 0.16;
                    cr += hzn * 0.6;
                    cg += hzn * 0.7;
                    cb += hzn * 0.95;
                    // a hazy moon over the open sea
                    let (dmx, dmy) = (xc - l.moon[0], y - l.moon[1]);
                    let dm = (dmx * dmx + dmy * dmy).sqrt();
                    let halo = (-dm / 22.0).exp() * 0.13 + (-dm / 6.0).exp() * 0.26;
                    cr += halo * 0.9;
                    cg += halo * 0.86;
                    cb += halo * 0.8;
                    // a darker ring just off the limb, so the disc's edge pops
                    let ring = 0.03 * smooth(4.6, 5.6, dm) * smooth(8.5, 6.5, dm);
                    cr -= ring;
                    cg -= ring;
                    cb -= ring * 0.8;
                    if dm < MOON_R {
                        let mut face = 0.9 + 0.1 * fbm(xf * 0.6, rf * 0.6, 2, 0.0);
                        for [mx, my, mr] in MARIA {
                            let d2 = ((dmx - mx).powi(2) + (dmy - my).powi(2)) / (mr * mr);
                            if d2 < 1.0 {
                                face -= 0.14 * (1.0 - d2);
                            }
                        }
                        let a = smooth(MOON_R, MOON_R - 0.8, dm);
                        cr = mix(cr, face, a);
                        cg = mix(cg, face * 0.95, a);
                        cb = mix(cb, face * 0.86, a);
                    }
                    sky_glow[k] = g as f32;
                }
                mat[k] = m;
                sr[k] = cr as f32;
                sg[k] = cg as f32;
                sb[k] = cb as f32;
                flo[k] = fl as f32;
            }
        }

        let hull = |x: usize, r: usize| {
            l
                .hulls
                .iter()
                .any(|&[r0, r1, x0, x1]| (r0..=r1).contains(&r) && (x0..=x1).contains(&x))
        };
        for r in hz as usize - 6..hz as usize {
            for x in 0..w {
                if !hull(x, r) {
                    continue;
                }
                let k = r * w + x;
                mat[k] = SHIP;
                (sr[k], sg[k], sb[k], flo[k]) = (0.002, 0.002, 0.004, 0.0);
            }
        }
        for &(x, r, cr, cg, cb) in &l.ship_lights {
            let k = r * w + x;
            mat[k] = SHIP;
            (sr[k], sg[k], sb[k], flo[k]) = (cr as f32, cg as f32, cb as f32, 0.1);
        }

        // --- lamp light: a hot core, a halo, and a wide warm spill
        let mut lamp_i = vec![0f32; n];
        let mut lamp_w = vec![0f32; n]; // the white-hot cores
        let mut flick_i = vec![0f32; n];
        let mut flick_w = vec![0f32; n];
        let mut refl = [vec![0f32; n], vec![0f32; n], vec![0f32; n]];
        let mut refl_fl = vec![0f32; n];
        let add = |v: &mut f32, d: f64| *v = (f64::from(*v) + d) as f32;
        for (j, &[lx, ly, s]) in lamps.iter().enumerate() {
            let (core, halo, spill) = (0.34 + 0.3 * s, 0.4 + 0.4 * s, 1.2 + 1.5 * s);
            let rr = (spill * 3.2).ceil();
            let (into, hot) = if j == FLICK {
                (&mut flick_i, &mut flick_w)
            } else {
                (&mut lamp_i, &mut lamp_w)
            };
            let mut r = (ly - rr).floor().max(0.0);
            while r < hf.min(ly + rr) {
                let mut x = (lx - rr).floor().max(0.0);
                while x < (w as f64).min(lx + rr) {
                    let k = r as usize * w + x as usize;
                    if mat[k] != WATER {
                        let (dx, dy) = (x + 0.5 - lx, r + 0.5 - ly);
                        let d = (dx * dx + dy * dy).sqrt();
                        let c = (-(d / core).powi(2)).exp() * 2.2;
                        add(
                            &mut into[k],
                            c * 0.7 + (-d / halo).exp() * 0.24 + (-d / spill).exp() * 0.02,
                        );
                        add(&mut hot[k], c * 0.3);
                    }
                    x += 1.0;
                }
                r += 1.0;
            }
            // its reflection: a long column broken by the swell, reaching toward us
            let sy = l.shore(lx);
            let (wj, len) = (0.4 + 0.7 * s, 2.0 + 8.0 * s);
            let mut r = sy.floor();
            while r < hf {
                let dy = r + 0.5 - sy;
                if dy >= 0.0 {
                    let a = (-dy / (len * 1.7)).exp() * (0.3 + 0.15 * s) * smooth(-0.5, 1.5, dy);
                    if a < 0.004 {
                        break;
                    }
                    let mut x = (lx - 3.0 * wj - 1.0).floor().max(0.0);
                    while x < (w as f64).min(lx + 3.0 * wj + 1.0) {
                        let k = r as usize * w + x as usize;
                        if mat[k] == WATER {
                            let g = a * (-((x + 0.5 - lx) / wj).powi(2)).exp();
                            if j == FLICK {
                                add(&mut refl_fl[k], g);
                            } else {
                                for (c, rf) in refl.iter_mut().enumerate() {
                                    add(&mut rf[k], g * SODIUM[c]);
                                }
                            }
                        }
                        x += 1.0;
                    }
                }
                r += 1.0;
            }
        }
        for k in 0..n {
            let m = mat[k];
            if m == WATER || m == SKY {
                continue;
            }
            let (li, lw) = (f64::from(lamp_i[k]), f64::from(lamp_w[k]));
            add(&mut sr[k], li * SODIUM[0] + lw);
            add(&mut sg[k], li * SODIUM[1] + lw * 0.95);
            add(&mut sb[k], li * SODIUM[2] + lw * 0.8);
        }

        // the ships' lights stretch down the water too
        let hzu = hz as usize;
        for &(x, _, cr, cg, cb) in &l.ship_lights {
            for r in hzu..hzu + 14 {
                let a = (-((r - hzu) as f64) / 5.0).exp() * 0.35;
                for dx in -1i64..=1 {
                    let xx = x as i64 + dx;
                    if xx >= w as i64 {
                        continue;
                    }
                    let k = r * w + xx as usize;
                    if mat[k] != WATER {
                        continue;
                    }
                    let g = a * (-(dx * dx) as f64 * 2.5).exp();
                    add(&mut refl[0][k], g * cr);
                    add(&mut refl[1][k], g * cg);
                    add(&mut refl[2][k], g * cb);
                }
            }
        }

        // The mirror: each water cell sees the picture above the shore, flipped and
        // stretched toward us, dimmed.
        for r in 0..h {
            for x in 0..w {
                let k = r * w + x;
                if mat[k] != WATER {
                    continue;
                }
                let sy = l.sea_top(x as f64 + 0.5);
                let d = r as f64 + 0.5 - sy;
                let (mut ar, mut ag, mut ab, mut cnt) = (0.0, 0.0, 0.0, 0u32);
                // only the lights carry across: the sky, the towers' lit floors, ships
                for o in 0..3 {
                    let ym = (sy - d * 0.55 - 0.3 - f64::from(o) * 0.7).floor();
                    if ym < 0.0 {
                        continue;
                    }
                    let q = ym as usize * w + x;
                    if mat[q] == WATER {
                        continue;
                    }
                    cnt += 1;
                    let (qr, qg, qb) = (f64::from(sr[q]), f64::from(sg[q]), f64::from(sb[q]));
                    if mat[q] == SKY {
                        ar += qr;
                        ag += qg;
                        ab += qb;
                    } else if mat[q] == TOWER || mat[q] == SHIP {
                        ar += (qr - 0.25).max(0.0) * 0.6;
                        ag += (qg - 0.25).max(0.0) * 0.6;
                        ab += (qb - 0.25).max(0.0) * 0.6;
                    }
                }
                let a = 0.32 * (-d / 30.0).exp();
                if cnt > 0 {
                    let nf = f64::from(cnt);
                    add(&mut refl[0][k], (ar / nf) * a);
                    add(&mut refl[1][k], (ag / nf) * a);
                    add(&mut refl[2][k], (ab / nf) * a);
                }
                // and a soft warm sheen off the whole lit shore
                if x as f64 + 0.5 <= l.tip + 4.0 {
                    let sh =
                        0.025 * (-d / 5.0).exp() * smooth(l.tip + 4.0, l.tip - 6.0, x as f64 + 0.5);
                    for (c, rf) in refl.iter_mut().enumerate() {
                        add(&mut rf[k], sh * SODIUM[c]);
                    }
                }
            }
        }

        // --- clouds: low stratus lit from beneath by the city, wrapping
        let density = |x: f64, y: f64| {
            let q = fbm(x * 0.01, y * 0.05, 3, 8.0);
            let d = fbm(x * 0.025 + q * 2.6, y * 0.07 + q * 1.3, 5, 20.0);
            let yb = y + 14.0 * (fbm(x * 0.005 + 3.0, 7.0, 2, 4.0) - 0.5); // the bank's edge rises and falls
            d + 0.05 * smooth(2.0, 14.0, yb)
                - 0.1 * smooth(6.0, 0.0, y)
                - 0.1 * smooth(22.0, hz - 4.0, yb)
        };
        let mut cover = vec![0f32; CW * hzu];
        let mut clit = vec![0f32; CW * hzu];
        for r in 0..hzu {
            for x in 0..CW {
                let (xf, y) = (x as f64, r as f64 + 0.5);
                let d = density(xf, y);
                cover[r * CW + x] = (smooth(0.47, 0.62, d) * 0.9) as f32;
                clit[r * CW + x] =
                    clamp(0.5 + (d - density(xf - 1.0, y + 2.5)) * 9.0 - (d - 0.6) * 1.4) as f32;
            }
        }

        // the sky behind the towers at the point is kept clear of low cloud, so the
        // dark glass reads against the city's haze
        let clear = (0..n)
            .map(|k| {
                let x = (k % w) as f64 + 0.5;
                let y = (k / w) as f64 + 0.5;
                (1.0 - 0.9
                    * smooth(l.point_clear[0], l.point_clear[1], x)
                    * smooth(l.point_clear[2], l.point_clear[3], x)
                    * smooth(20.0 + l.sky, 28.0 + l.sky, y)) as f32
            })
            .collect();

        // --- cars
        let nb = ((uend + 12.0) / BIN).ceil() as i32;
        let cars = (0..26)
            .map(|i| {
                let f = f64::from(i);
                [
                    hash(f, 51.0) * (uend + 12.0),
                    4.2 + 1.6 * hash(f, 52.0),
                    f64::from(i & 1),
                ]
            })
            .collect();
        let mut bin_lo = vec![0i32; w];
        let mut bin_hi = vec![0i32; w];
        for x in 0..w {
            let xf = x as f64;
            bin_lo[x] = 0.max(((uof(xf) - U0) / BIN).floor() as i32);
            bin_hi[x] = (nb - 1).min(bin_lo[x].max(((uof(xf + 1.0) - U0) / BIN).floor() as i32));
        }

        Self {
            l,
            dots: Dots::new(Self::PALETTE),
            mat,
            sr,
            sg,
            sb,
            flo,
            lane,
            sky_glow,
            rooms,
            beacons,
            lamp_i,
            lamp_w,
            flick_i,
            flick_w,
            refl,
            refl_fl,
            cover,
            clit,
            clear,
            uend,
            lane_a: vec![0f32; nb as usize],
            lane_b: vec![0f32; nb as usize],
            cars,
            bin_lo,
            bin_hi,
            col_a: vec![0f32; w],
            col_b: vec![0f32; w],
        }
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        let (w, h, hz) = (self.l.w, self.l.h, self.l.hz);
        let (wf, hf) = (w as f64, h as f64);
        let moon = self.l.moon;
        let drift = t * 1.3;
        // cars: tail lights heading out to the point, headlights coming home
        self.lane_a.fill(0.0);
        self.lane_b.fill(0.0);
        let span = self.uend + 12.0;
        let nb = self.lane_a.len() as i64;
        for &[p0, v, dir] in &self.cars {
            let back = dir != 0.0;
            let p = (((p0 + (if back { -v } else { v }) * t) % span) + span) % span;
            let into = if back {
                &mut self.lane_b
            } else {
                &mut self.lane_a
            };
            let head = (p / BIN).floor() as i64;
            for i in 0..26i64 {
                let b = if back { head + i } else { head - i };
                if b < 0 || b >= nb {
                    continue;
                }
                let e = (-(i as f64) / 7.0).exp() * if i < 2 { 1.3 } else { 0.85 };
                if e > f64::from(into[b as usize]) {
                    into[b as usize] = e as f32;
                }
            }
        }
        for x in 0..w {
            let (mut a, mut b) = (0f32, 0f32);
            for i in self.bin_lo[x]..=self.bin_hi[x] {
                a = a.max(self.lane_a[i as usize]);
                b = b.max(self.lane_b[i as usize]);
            }
            self.col_a[x] = a;
            self.col_b[x] = b;
        }
        // the failing lamp
        let sputter = (t * 0.7).sin() > 0.45;
        let lamp_on = if sputter && hash((t * 9.0).floor(), 77.0) <= 0.45 {
            0.15
        } else {
            1.0
        };

        for r in 0..h {
            let rf = r as f64;
            let y = rf + 0.5;
            for xi in 0..w {
                let k = r * w + xi;
                let x = xi as f64;
                let m = self.mat[k];
                let (mut cr, mut cg, mut cb) = (
                    f64::from(self.sr[k]),
                    f64::from(self.sg[k]),
                    f64::from(self.sb[k]),
                );
                let (mut floor, mut fade) = (f64::from(self.flo[k]), 1.0);

                if m == SKY {
                    let sx = x + drift;
                    let ix = sx.floor();
                    let fx = sx - ix;
                    let ix = ix as usize;
                    let i0 = r * CW + ix % CW;
                    let i1 = r * CW + (ix + 1) % CW;
                    let (c0, c1) = (f64::from(self.cover[i0]), f64::from(self.cover[i1]));
                    let c = (c0 + (c1 - c0) * fx) * f64::from(self.clear[k]);
                    if c > 0.01 {
                        let (l0, l1) = (f64::from(self.clit[i0]), f64::from(self.clit[i1]));
                        let l = l0 + (l1 - l0) * fx;
                        let g = f64::from(self.sky_glow[k]);
                        // grey-violet, warmed underneath by the city
                        let (dmx, dmy) = (x + 0.5 - moon[0], y - moon[1]);
                        let dm = (dmx * dmx + dmy * dmy).sqrt();
                        let near = (-dm / 16.0).exp();
                        let b = clamp(0.1 + l * (0.42 + 0.9 * g + 0.5 * near));
                        let ramp = |lo: f64, mid: f64, hi: f64| {
                            if b < 0.5 {
                                mix(lo, mid, b * 2.0)
                            } else {
                                mix(mid, hi, b * 2.0 - 1.0)
                            }
                        };
                        let (kr, kg, kb) = (
                            ramp(0.05, 0.22, 0.9),
                            ramp(0.06, 0.26, 0.88),
                            ramp(0.15, 0.44, 0.92),
                        );
                        // low cloud thins into the haze, so the horizon and the ships stay clear
                        let a = (c * 1.1).min(1.0)
                            * smooth(MOON_R, MOON_R + 4.0, dm)
                            * (1.0 - 0.8 * smooth(27.0 + self.l.sky, 37.0 + self.l.sky, y));
                        cr = mix(cr, kr, a);
                        cg = mix(cg, kg, a);
                        cb = mix(cb, kb, a);
                    } else if y < hz - 14.0 && hash(x, rf * 3.0 + 11.0) > 0.993 {
                        let tw =
                            0.3 + 0.2 * (t * (1.3 + hash(x, rf) * 2.5) + hash(rf, x) * 6.28).sin();
                        cr = cr.max(tw * 0.9);
                        cg = cg.max(tw * 0.9);
                        cb = cb.max(tw);
                    }
                    let (li, lw) = (f64::from(self.lamp_i[k]), f64::from(self.lamp_w[k]));
                    cr += li * SODIUM[0] + lw;
                    cg += li * SODIUM[1] + lw * 0.95;
                    cb += li * SODIUM[2] + lw * 0.8;
                    floor = 0.14;
                } else if m == WATER {
                    let v = (y - hz) / (hf - hz);
                    let wave = 0.6 * noise(x * 0.06 + t * 0.1, y * 0.5 - t * 0.5, 0.0)
                        + 0.4 * noise(x * 0.18 - t * 0.2, y * 1.0 - t * 1.0, 0.0);
                    let swell = 0.45 + 0.95 * wave;
                    // brighter toward the open sea and the moon
                    let open = 0.85 + 0.3 * (x / wf);
                    cr = (0.05 - 0.01 * v) * swell * open;
                    cg = (0.095 - 0.015 * v) * swell * open;
                    cb = (0.23 - 0.03 * v) * swell * open;
                    {
                        // the moon's road on the open water
                        let road = (-((x + 0.5 - moon[0]) / (0.8 + (y - hz) * 0.12)).powi(2)).exp();
                        let glint = smooth(
                            0.52,
                            0.8,
                            0.45 * wave + 0.55 * noise(x * 0.3 + t * 0.3, y * 1.4 - t * 1.4, 0.0),
                        ) * road;
                        cr += 0.9 * glint + 0.03 * road;
                        cg += 0.88 * glint + 0.035 * road;
                        cb += 0.8 * glint + 0.05 * road;
                    }
                    // the reflections, pushed sideways by the swell and broken into dashes
                    let wob =
                        (noise(x * 0.05 + 7.0, y * 0.32 - t * 0.7, 0.0) - 0.5) * (1.4 + 4.5 * v);
                    let sx = (x + wob).max(0.0).min(wf - 1.001);
                    let i0 = r * w + sx as usize;
                    let fx = sx - sx.trunc();
                    let dash = 0.45
                        + 0.9 * smooth(0.2, 0.55, noise(x * 0.16 + 3.0, y * 0.85 - t * 1.3, 0.0));
                    let rf_at = |a: &[f32]| {
                        let (a0, a1) = (f64::from(a[i0]), f64::from(a[i0 + 1]));
                        (a0 + (a1 - a0) * fx) * dash
                    };
                    let fl = rf_at(&self.refl_fl) * lamp_on;
                    let rr = rf_at(&self.refl[0]) + fl * SODIUM[0];
                    let rg = rf_at(&self.refl[1]) + fl * SODIUM[1];
                    let rb = rf_at(&self.refl[2]) + fl * SODIUM[2];
                    // where the warm light lies on the water it replaces the blue, so the
                    // streaks stay gold instead of mixing to pink
                    let kill = 1.0 - 0.9 * clamp((rr - rb) * 3.0);
                    cr = cr * kill + rr;
                    cg = cg * kill + rg;
                    cb = cb * kill + rb;
                    floor = 0.16;
                    fade = smooth(hf + 6.0, hf - 8.0, y);
                } else if m == ROAD {
                    let l = self.lane[k];
                    let a = if l & 1 != 0 {
                        f64::from(self.col_a[xi])
                    } else {
                        0.0
                    };
                    let b = if l & 2 != 0 {
                        f64::from(self.col_b[xi])
                    } else {
                        0.0
                    };
                    cr += a * 0.8 + b * 1.0;
                    cg += a * 0.1 + b * 0.9;
                    cb += a * 0.08 + b * 0.72;
                }
                if m != WATER {
                    let f = f64::from(self.flick_i[k]) * lamp_on;
                    let fw = f64::from(self.flick_w[k]) * lamp_on;
                    if f != 0.0 {
                        cr += f * SODIUM[0] + fw;
                        cg += f * SODIUM[1] + fw * 0.95;
                        cb += f * SODIUM[2] + fw * 0.8;
                    }
                }

                let peak = cr.max(cg).max(cb).max(1e-4);
                let level = clamp(floor + (1.0 - floor) * peak.powf(0.85) * 0.95) * fade;
                let step = Dots::step(level, bayer(r, xi));
                out[k] = self.dots.ink(step, level, [cr, cg, cb], peak);
            }
        }

        // rooms whose light changes, and the aviation lights
        for room in &self.rooms {
            let k = room.k;
            let [wr, wg, wb] = room.w;
            let (sr, sg, sb) = (
                f64::from(self.sr[k]),
                f64::from(self.sg[k]),
                f64::from(self.sb[k]),
            );
            if room.tv {
                // a television: cold light that jumps
                let f = 0.5 + 0.5 * (t * 11.0 + room.ph).sin() * (t * 4.3 + room.ph * 2.0).sin();
                paint(
                    &mut self.dots,
                    out,
                    w,
                    k,
                    [
                        sr - wr + f * wr * 0.45,
                        sg - wg + f * wr * 0.6,
                        sb - wb + f * wr,
                    ],
                    0.08,
                );
            } else {
                // a light switched off for a while, then on again
                let f = if (t * room.rate * 0.5 + room.ph).sin() > -0.6 {
                    1.0
                } else {
                    0.0
                };
                paint(
                    &mut self.dots,
                    out,
                    w,
                    k,
                    [
                        sr - wr * (1.0 - f),
                        sg - wg * (1.0 - f),
                        sb - wb * (1.0 - f),
                    ],
                    0.08,
                );
            }
        }
        for &(k, ph) in &self.beacons {
            if k < 0 {
                continue;
            }
            let on = (t * 2.4 + ph).sin() > 0.3;
            let rgb = if on {
                [1.0, 0.2, 0.15]
            } else {
                [0.3, 0.06, 0.05]
            };
            paint(&mut self.dots, out, w, k as usize, rgb, 0.1);
        }
    }
}
