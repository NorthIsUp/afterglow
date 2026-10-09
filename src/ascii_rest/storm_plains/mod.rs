//! storm plains: an anvil thunderhead at dusk over open wheat country. The
//! sun has just set behind the farmhouse, so the storm's top and western flank
//! still catch its light while the base sinks into slate shadow. Lightning
//! flickers inside the cloud, now and then a bolt reaches the ground, rain
//! curtains drift under the base and the wheat moves in gusts.
//!
//! The cloud is built as a height field (heaped domes, a flat anvil, pouches
//! hanging under it) and lit from its surface normals.
//!
//! At any size the farm keeps the western corner and the storm stands whole
//! east of it: wider panels move the storm east and stream the anvil on across
//! the extra sky, trailing thinner rain; narrower ones draw the storm in over
//! the farm and the farm toward the edge; taller ones add sky above and wheat
//! below.

mod cloud;
mod lightning;

use super::halftone::{bayer, Dots};
use super::math::{clamp, fbm, hash, js_round, mix, noise, smooth};
use super::{hex, Piece};
use crate::font;
use crate::grid::Cell;
use cloud::{anvil_bot, shelf_bot, shelf_on, shelf_top, tower_c, tower_hw, Cloud};
use lightning::{bolts, flash_at, flicker_at, Bolt};

/// The horizon, in upstream's rows.
const HZ: usize = 64;
const HZF: f64 = HZ as f64;
/// Just below the horizon, behind the farmhouse.
const SUN: [f64; 2] = [30.0, 70.0];
/// The storm's flat base.
const BASE: f64 = 45.0;

const SKY: u8 = 0;
const PLAIN: u8 = 1;
const FIELD: u8 = 2;
const ROAD: u8 = 3;
const HOUSE: u8 = 4;
const ROOF: u8 = 5;
const PANE: u8 = 6;
const TREE: u8 = 7;
const PUMP: u8 = 8;
const BELT: u8 = 9;

/// The height-field grid's rows.
const SY: usize = BASE as usize + 8;
/// Toward the light: west, a touch high, out of the page.
const LX: f64 = -0.72;
const LY: f64 = -0.3;
const LZ: f64 = 0.62;
/// Farmhouse walls from x 40 to 54, eaves at row 58.
const HX0: i64 = 40;
const HX1: i64 = 54;
const HW0: f64 = 58.0;
const PUMP_X: f64 = 66.0;
/// The wheat's vanishing column.
const VX: f64 = 57.0;
const TW: usize = 512;
const GW: usize = 512;
const CW: usize = 400;
const RW: usize = 256;
const PERIOD: f64 = 3.1;
const FL: f64 = 1.6;
/// `meta.palette.indexOf("#8c6230")`.
const STALK: u16 = 24;

fn glow_sun(x: f64, y: f64) -> f64 {
    let (dx, dy) = (x - SUN[0], y - SUN[1]);
    (-(dx / 48.0).powi(2) - (dy / 10.0).powi(2)).exp() * 0.9
        + (-(dx / 95.0).powi(2) - (dy / 20.0).powi(2)).exp() * 0.26
}

fn ground(x: f64) -> f64 {
    HZF + 3.0 - 1.2 * smooth(20.0, 70.0, x) * smooth(110.0, 70.0, x)
}

/// The road: from the yard to the bottom edge, a leading line.
/// `reach`: how far east it runs by the bottom edge.
fn road_c(p: f64, reach: f64) -> f64 {
    57.0 + reach * p.powf(1.25)
}

fn road_w(p: f64) -> f64 {
    0.6 + 12.0 * p
}

/// Where things sit in a `w x h` frame. Upstream's 200x100 is the anchor
/// every value moves from, so there it is exact. The sky and the land are
/// drawn in upstream's rows, `top` rows down; the storm in its own columns,
/// `x - sd`; the farm in its own, `x - fd`. Shifts of zero leave `x` exact.
struct Layout {
    w: usize,
    h: usize,
    top: usize,
    /// The horizon's row in the frame.
    hz: usize,
    /// How far east the storm stands from upstream's.
    sd: f64,
    /// How far east the farm stands from upstream's, in whole columns so its
    /// walls and panes stay on the cells upstream draws them on.
    fd: i64,
    /// Where the shelterbelt on the horizon gives out, in the storm's columns.
    belt_end: [f64; 2],
    /// The stars: the columns they show in, and where they would fade out.
    stars: (usize, f64),
    /// How far east the road runs from the yard to the bottom edge.
    road: f64,
    /// Thinner rain trailing from the anvil across a wider frame's extra
    /// sky, at this strength.
    trail: f64,
}

impl Layout {
    fn new(w: usize, h: usize) -> Self {
        let wf = w as f64;
        let wide = (wf - 200.0) / 120.0;
        let narrow = clamp((200.0 - wf) / 100.0);
        // the old 3.2:1 recomposition stood the storm 50 columns east; a
        // square one brings it in over the farm, which steps toward the edge
        let sd = if wf >= 200.0 { 50.0 * wide } else { -69.0 * narrow };
        let top = (h - 100) / 2;
        Self {
            w,
            h,
            top,
            hz: HZ + top,
            sd,
            fd: -(14.0 * narrow).round() as i64,
            belt_end: [wf - sd - 4.0, wf - sd - 14.0],
            stars: ((100.0 + sd).max(0.0) as usize, 130.0 + 1.3 * sd),
            road: 62.0 - 20.0 * narrow,
            trail: 0.5 * clamp(wide),
        }
    }
}

pub struct StormPlains {
    l: Layout,
    dots: Dots,
    sr: Vec<f32>,
    sg: Vec<f32>,
    sb: Vec<f32>,
    mat: Vec<u8>,
    cloud: Vec<f32>,
    dark: Vec<f32>,
    floor_of: Vec<f32>,
    jit: Vec<f32>,
    pump_y: f64,
    wheat: Vec<f32>,
    persp: Vec<f32>,
    field_light: Vec<f32>,
    gust: Vec<f32>,
    streak: Vec<f32>,
    /// [cell, phase, speed, amplitude].
    stars: Vec<[f64; 4]>,
    star: Vec<f32>,
    /// [x, stalk height, phase, foot below the frame, head length].
    ears: Vec<[f64; 5]>,
    /// > 0: a lit grain head; < 0: a dark stalk.
    ear: Vec<f32>,
    touched: Vec<usize>,
    shafts: Vec<f32>,
    col_phase: Vec<f32>,
    col_speed: Vec<f32>,
    rain_top: Vec<f32>,
    rain_on: Vec<f32>,
    bolts: Vec<Bolt>,
}

impl Piece for StormPlains {
    const NAME: &'static str = "storm-plains";
    const FPS: u32 = 15;
    const GROUND: u32 = hex("#0b0912");
    #[rustfmt::skip]
    const PALETTE: &'static [u32] = &[
        // night violets, zenith to dusk
        hex("#0f0c20"), hex("#141029"), hex("#1c1638"), hex("#261c4a"), hex("#32235a"), hex("#40306c"), hex("#53407e"), hex("#6a5590"), hex("#8670a6"),
        // dusty mauve, the shaded flank
        hex("#7a6288"), hex("#9a7890"), hex("#b98e94"),
        // peach and gold, the sunlit tops
        hex("#e6a890"), hex("#f2bca8"), hex("#f7d6a8"), hex("#fbe8c6"),
        // amber afterglow
        hex("#f8b070"), hex("#e88a4c"), hex("#c8643a"), hex("#9a4630"), hex("#6a3226"),
        // wheat, gold to umber
        hex("#f0c868"), hex("#d8a447"), hex("#b7843a"), hex("#8c6230"), hex("#5e4226"), hex("#3a2a1c"), hex("#261c14"),
        // rain and slate
        hex("#181a2c"), hex("#2a3048"), hex("#3a4260"), hex("#4f5878"), hex("#6a7392"), hex("#8890ae"), hex("#a2aabd"), hex("#5d5878"), hex("#7c7598"),
        // lightning
        hex("#a6a2c8"), hex("#d6d2fa"), hex("#f4f2ff"),
        // lamplight
        hex("#ffe08a"), hex("#ffb84a"),
    ];

    fn new(cols: usize, rows: usize) -> Self {
        let l = Layout::new(cols, rows);
        let (w, h, top, hz, sd) = (l.w, l.h, l.top, l.hz, l.sd);
        let (topf, hzf, hf, fdf) = (top as f64, hz as f64, h as f64, l.fd as f64);
        let n = w * h;
        let sx = w + 4;
        let Cloud { sh, anv, lip } = Cloud::new(sx, sd);

        // static colour of every cell: sky, storm, land
        let (mut sr, mut sg, mut sb) = (vec![0f32; n], vec![0f32; n], vec![0f32; n]);
        let mut mat = vec![SKY; n];
        let mut cloud = vec![0f32; n]; // storm cover, for the lightning
        let mut dark = vec![0f32; n]; // base and shelf, lit from behind by a flash
        let mut dark_sky = vec![0f32; n]; // the slot under the storm
        let mut floor_of = vec![0f32; n];
        // per-cell dither jitter, against contour lines
        let jit: Vec<f32> = (0..n)
            .map(|k| {
                let row = k / w;
                ((hash((k % w) as f64, row as f64 + 517.0) - 0.5)
                    * if row < hz { 0.16 } else { 0.06 }) as f32
            })
            .collect();

        // the farmhouse, its tree and a windpump, standing on the horizon
        let hb = ground(47.0);
        let pump_y = hb - 15.0;

        let shf = |v: f32| f64::from(v);
        for r in 0..h {
            // upstream's row: negative in a tall frame's extra sky
            let rs = r as i64 - top as i64;
            for xi in 0..w {
                let x = xi as f64;
                let k = r * w + xi;
                let y = r as f64 + 0.5 - topf;
                // the farm's column, and its cell for the walls and panes
                let (xf, xfi) = (x - fdf, xi as i64 - l.fd);
                if y < ground(xf) {
                    // sky: indigo overhead, dusky rose lower down, amber where the sun went
                    let v = (y / HZF).max(0.0);
                    let g = glow_sun(xf, y);
                    // everything after the sun's own glow is the storm's
                    let x = x - sd;
                    let east = smooth(60.0, 190.0, x); // the storm side is cooler and darker
                    let v2 = v.powf(2.2);
                    let veil = 0.9 + 0.2 * fbm(x * 0.05, y * 0.09, 3, 0.0);
                    let mut cr = (0.045 + 0.2 * v2 * (1.0 - 0.6 * east)) * veil + 1.0 * g;
                    let mut cg = (0.045 + 0.09 * v2 * (1.0 - 0.5 * east)) * veil + 0.56 * g;
                    let mut cb = (0.16 + 0.13 * v2 * (1.0 - 0.35 * east)) * veil + 0.22 * g;
                    // the last light along the horizon, gone where the storm stands
                    let be = smooth(45.0, 118.0, x);
                    let band =
                        (-(HZF + 2.0 - y) / 11.0).exp() * (1.0 - 0.85 * be) * (1.0 - 0.6 * g);
                    cr += band * mix(0.5, 0.34, be);
                    cg += band * mix(0.26, 0.21, be);
                    cb += band * mix(0.26, 0.3, be);
                    // under the storm the air itself goes dark slate, with only a
                    // thread of light left along the ground behind the rain
                    let under = smooth(98.0, 128.0, x + 8.0 * (noise(y * 0.15, 4.4, 0.0) - 0.5))
                        * smooth(BASE - 14.0, BASE - 1.0, y);
                    // black under the base, opening to a dim violet glow at the
                    // ground: the clear sky far beyond the storm, for the rain
                    let slot = smooth(BASE + 1.5, HZF - 6.0, y).powf(0.5)
                        * (0.85 + 0.3 * noise(x * 0.06, 6.6, 0.0));
                    cr = mix(cr, mix(0.035, 0.38, slot) * veil, under * 0.92);
                    cg = mix(cg, mix(0.032, 0.3, slot) * veil, under * 0.92);
                    cb = mix(cb, mix(0.075, 0.56, slot) * veil, under * 0.92);
                    dark_sky[k] = under as f32;
                    // the storm
                    if (0..SY as i64 - 1).contains(&rs) {
                        let i = xi + 2;
                        let j = rs as usize * sx + i;
                        let h = shf(sh[j]);
                        let c = smooth(0.15, 1.6, h);
                        if c > 0.0 {
                            let up = if rs > 0 { shf(sh[j - sx]) } else { 0.0 };
                            let dn = shf(sh[j + sx]);
                            let (gx, gy) =
                                ((shf(sh[j + 1]) - shf(sh[j - 1])) / 2.0, (dn - up) / 2.0);
                            let nl = (gx * gx + gy * gy + 1.0).sqrt();
                            let (nx, ny, nz) = (-gx / nl, -gy / nl, 1.0 / nl);
                            let an = shf(anv[j]);
                            let lam = clamp(nx * LX + ny * LY + nz * LZ).powf(1.5);
                            // earth's shadow climbing the tower: the tops still
                            // lit, the lower bulk already in dusk
                            let vis = 1.0
                                - 0.4
                                    * smooth(
                                        22.0,
                                        BASE - 4.0,
                                        y + 5.0 * (noise(x * 0.12, 2.2, 0.0) - 0.5),
                                    );
                            let a = smooth(8.0, BASE, y);
                            let near = (-((x - SUN[0]) / 110.0).powi(2)).exp();
                            let flank = smooth(122.0, 108.0, x) * (1.0 - an); // the younger towers to the west
                                                                              // sunlight: cream-gold up high, peach lower down
                            let (s_r, s_g, s_b) = (1.0, mix(0.84, 0.58, a), mix(0.58, 0.45, a));
                            // the tower's own bulk shades its eastern side, along a ragged edge
                            let ej = 8.0 * (fbm(y * 0.14 + x * 0.02, 21.3, 3, 0.0) - 0.5);
                            let lee = 1.0
                                - 0.65
                                    * smooth(
                                        tower_c(y) - 14.0 + ej,
                                        tower_c(y) + tower_hw(y) + 16.0 + ej,
                                        x,
                                    )
                                    * (1.0 - an)
                                    * smooth(
                                        anvil_bot(x) - 3.0,
                                        anvil_bot(x) + 9.0,
                                        y + 5.0 * (noise(x * 0.15, 21.0, 0.0) - 0.5),
                                    )
                                    * if y < BASE { 1.0 } else { 0.0 };
                            let sun = lam
                                * vis
                                * lee
                                * (0.95 + 0.4 * near)
                                * (1.0 - 0.3 * flank)
                                * (1.0 - 0.2 * an);
                            // skylight from above, violet; afterglow bounced up from the west
                            let sky = 0.55 + 0.45 * (-ny).max(0.0);
                            let bounce = ny.max(0.0)
                                * (0.2 + 0.45 * near)
                                * (1.0 - 0.6 * flank)
                                * (1.0 - 0.5 * an);
                            let occ = 1.0 - 0.35 * smooth(4.0, 22.0, h) * (1.0 - lam);
                            let mut kr = (0.27 * sky + s_r * sun + 0.36 * bounce) * occ;
                            let mut kg = (0.2 * sky + s_g * sun + 0.16 * bounce) * occ;
                            let mut kb = (0.45 * sky + s_b * sun + 0.16 * bounce) * occ;
                            // the anvil's underside sinks into mauve shadow; its
                            // top edge catches the last direct light, cream toward the west
                            let below = clamp(ny * 2.2) * an;
                            kr *= 1.0 - 0.42 * below;
                            kg *= 1.0 - 0.46 * below;
                            kb *= 1.0 - 0.3 * below;
                            let rim = shf(lip[j]) * smooth(196.0, 120.0, x) * (0.6 + 0.4 * lam);
                            kr = mix(kr, 0.98, rim * 0.75);
                            kg = mix(kg, 0.86, rim * 0.75);
                            kb = mix(kb, 0.64, rim * 0.75);
                            // the flankers' lower halves turn violet-grey
                            let low = clamp(ny * 1.6 + 0.2) * flank;
                            kr = mix(kr, 0.14, low * 0.65);
                            kg = mix(kg, 0.11, low * 0.65);
                            kb = mix(kb, 0.24, low * 0.65);
                            // the base: dark slate where the rain hangs
                            let base =
                                smooth(BASE - 4.5, BASE - 0.5, y) * (1.0 - an) * (1.0 - 0.3 * near);
                            kr = mix(kr, 0.03, base * 0.9);
                            kg = mix(kg, 0.026, base * 0.9);
                            kb = mix(kb, 0.06, base * 0.9);
                            cr = mix(cr, kr, c);
                            cg = mix(cg, kg, c);
                            cb = mix(cb, kb, c);
                            cloud[k] = c as f32;
                            dark[k] = (base * c) as f32;
                        }
                    }
                    // the shelf, hung along the base, a touch lighter on its leading lip
                    let on = shelf_on(x);
                    if on > 0.0 && y > BASE - 5.0 && y < BASE + 5.0 {
                        let (st, sbt) = (shelf_top(x), shelf_bot(x));
                        let s =
                            smooth(st - 1.0, st + 0.6, y) * smooth(sbt + 0.6, sbt - 0.8, y) * on;
                        if s > 0.0 {
                            let nn = fbm(x * 0.15, y * 0.5, 2, 0.0);
                            // striations along its face, and a faint lip of light underneath
                            let lit = 0.55
                                + 0.9 * nn * (0.7 + 0.6 * noise(x * 0.05, y * 1.3, 0.0))
                                + 0.9 * smooth(sbt - 1.6, sbt - 0.2, y);
                            cr = mix(cr, 0.06 * lit, s);
                            cg = mix(cg, 0.056 * lit, s);
                            cb = mix(cb, 0.12 * lit, s);
                            cloud[k] = shf(cloud[k]).max(s) as f32;
                            dark[k] = shf(dark[k]).max(s) as f32;
                        }
                    }
                    (sr[k], sg[k], sb[k]) = (cr as f32, cg as f32, cb as f32);
                    mat[k] = SKY;
                    floor_of[k] = (0.06 - 0.06 * shf(dark_sky[k]).max(shf(dark[k]))) as f32;
                } else {
                    // land: the afterglow caught only in the far rows near the sun
                    let g = glow_sun(xf, HZF + 1.0) * (-(y - HZF) / 3.2).exp();
                    mat[k] = if y < HZF + 4.0 { PLAIN } else { FIELD };
                    sr[k] = (0.06 + 0.55 * g) as f32;
                    sg[k] = (0.045 + 0.3 * g) as f32;
                    sb[k] = (0.08 + 0.1 * g) as f32;
                    floor_of[k] = 0.06;
                }

                // a shelterbelt of trees on the far horizon, half lost in the rain
                let xs = x - sd;
                let belt = HZF + 2.0
                    - (1.5 + 2.2 * fbm(xs * 0.3, 2.0, 2, 0.0))
                        * smooth(140.0, 150.0, xs)
                        * smooth(l.belt_end[0], l.belt_end[1], xs);
                let belt2 = HZF + 2.0
                    - (1.0 + 1.8 * fbm(x * 0.35, 9.0, 2, 0.0))
                        * smooth(0.0, 4.0, x)
                        * smooth(18.0, 10.0, x);
                if y >= belt.min(belt2) && y < HZF + 3.0 {
                    mat[k] = BELT;
                }

                // the cottonwood by the house
                let (tx, ty) = ((xf + 0.5 - 31.0) / 8.5, (y - (hb - 10.0)) / 7.0);
                let crown = tx * tx + ty * ty < 1.0 + 0.35 * (fbm(xf * 0.4, y * 0.4, 2, 0.0) - 0.5);
                if crown || ((xf + 0.5 - 31.5).abs() < 1.0 && y > hb - 6.0 && y < hb + 1.0) {
                    mat[k] = TREE;
                }

                // the farmhouse: two storeys, a gable roof, a chimney, a porch
                if (HX0..=HX1).contains(&xfi) && y >= HW0 && y < hb + 1.0 {
                    mat[k] = HOUSE;
                }
                if (HW0 - 7.0..HW0).contains(&y) && (xf + 0.5 - 47.5).abs() <= 8.6 - (HW0 - y) * 1.15
                {
                    mat[k] = ROOF;
                }
                if (50..=51).contains(&xfi) && (HW0 - 8.0..HW0 - 3.0).contains(&y) {
                    mat[k] = ROOF;
                }
                if (36..HX0).contains(&xfi) && y >= hb - 4.0 && y < hb - 3.0 {
                    mat[k] = ROOF;
                }
                if xfi == 36 && y >= hb - 3.0 && y < hb {
                    mat[k] = HOUSE;
                }
                if (49..=51).contains(&xfi) && (HW0 + 2.0..HW0 + 5.0).contains(&y) {
                    mat[k] = PANE;
                }
                if (43..=44).contains(&xfi) && (HW0 + 2.0..HW0 + 4.0).contains(&y) {
                    mat[k] = PANE;
                }

                // the windpump: a tapering lattice tower
                let py = y - pump_y;
                if py > 2.0 && y < hb + 0.5 {
                    let half = 0.4 + py * 0.11;
                    let dx = xf + 0.5 - PUMP_X;
                    if (dx.abs() - half).abs() < 0.55 {
                        mat[k] = PUMP;
                    }
                    if dx.abs() < half && (py as i32) % 4 == 0 {
                        mat[k] = PUMP;
                    }
                }
            }
        }

        for r in hz + 3..h {
            let p = (r as f64 + 0.5 - hzf) / (hf - hzf);
            for xi in 0..w {
                let k = r * w + xi;
                if mat[k] != FIELD {
                    continue;
                }
                let x = xi as f64;
                if (x - fdf + 0.5 - road_c(p, l.road)).abs()
                    < road_w(p) + 0.6 * (noise(x * 0.3, r as f64 * 0.3, 0.0) - 0.5)
                {
                    mat[k] = ROAD;
                }
            }
        }

        // wheat: stalks in perspective, their columns converging on the
        // farmhouse, tall strokes up close and a fine grain toward the horizon
        let mut wheat = vec![0f32; h * TW];
        let mut persp = vec![0f32; n]; // each cell's column in the wheat texture
        for r in hz..h {
            let rf = r as f64;
            let p = (rf + 0.5 - hzf) / (hf - hzf);
            for u in 0..TW {
                wheat[r * TW + u] =
                    fbm(u as f64 * 0.55, rf * (0.5 - 0.42 * p), 3, TW as f64 * 0.55) as f32;
            }
            for x in 0..w {
                persp[r * w + x] =
                    (((x as f64 - fdf + 0.5 - VX) * 24.0) / (rf + 2.0 - hzf) + 256.0) as f32;
            }
        }
        // how much light the field holds: the sun side, the far rows, not the
        // east or the foreground, which sink into the storm's shadow
        let mut field_light = vec![0f32; n];
        for r in hz..h {
            let y = r as f64 + 0.5;
            let p = (y - hzf) / (hf - hzf);
            for xi in 0..w {
                let x = xi as f64;
                let sun_w = (-((x - fdf - SUN[0]) / 100.0).powi(2)).exp();
                let east = smooth(80.0, 130.0, x - sd + 10.0 * p);
                let fore = 1.0 - 0.6 * smooth(hf - 22.0, hf - 2.0, y);
                field_light[r * w + xi] =
                    ((0.72 + 0.45 * sun_w) * (1.0 - 0.35 * east) * fore) as f32;
            }
        }
        // gusts: soft patches of bent, brighter wheat rolling across the field
        let mut gust = vec![0f32; GW * (h - hz)];
        for r in hz..h {
            for u in 0..GW {
                gust[(r - hz) * GW + u] = smooth(
                    0.4,
                    0.7,
                    fbm(u as f64 * 0.025, r as f64 * 0.16, 3, GW as f64 * 0.025),
                ) as f32;
            }
        }

        // thin glowing streaks of altostratus in the clear western sky
        let mut streak = vec![0f32; CW * HZ];
        for r in 0..HZ {
            let rf = r as f64;
            for u in 0..CW {
                let nn = fbm(u as f64 * 0.02, rf * 0.2, 4, CW as f64 * 0.02);
                streak[r * CW + u] = (smooth(0.64, 0.8, nn)
                    * smooth(40.0, 46.0, rf)
                    * smooth(HZF - 3.0, 52.0, rf)) as f32;
            }
        }

        // stars in the darkest part of the sky
        let mut stars = Vec::new();
        for r in 0..28 + top {
            for x in 0..l.stars.0.min(w) {
                // upstream's row, so a tall frame's extra sky is starry too
                let (xf, rf) = (x as f64, r as f64 - topf);
                if hash(xf, rf * 5.0 + 3.0) > 0.986 && cloud[r * w + x] < 0.05 {
                    stars.push([
                        (r * w + x) as f64,
                        hash(xf, rf) * 6.28,
                        1.0 + hash(rf, xf) * 2.5,
                        0.85 * (1.0 - rf / 28.0).min(1.0) * (1.0 - xf / l.stars.1),
                    ]);
                }
            }
        }

        // wheat ears right in front of us, bowing with the wind
        let mut ears = Vec::new();
        let mut x = 2.0;
        while x < w as f64 {
            ears.push([
                x,
                8.0 + hash(x * 3.0, 61.0) * 12.0,
                hash(x * 7.0, 62.0) * 6.28,
                hash(x * 5.0, 63.0) * 6.0,
                5.0 + if hash(x, 64.0) > 0.5 { 1.0 } else { 0.0 },
            ]);
            x += 5.0 + hash(x, 60.0) * 4.0;
        }
        // every ear row can touch three cells
        let touched_max: usize = ears.iter().map(|e| (e[1] + e[4]).ceil() as usize * 3).sum();

        // rain: one envelope of shafts that drifts, and falling streaks inside it
        let shafts = (0..RW)
            .map(|u| smooth(0.47, 0.57, fbm(u as f64 * 0.09375, 0.5, 2, 24.0)) as f32)
            .collect();
        let col_phase = (0..w)
            .map(|x| (hash(x as f64, 77.0) * 40.0) as f32)
            .collect();
        let col_speed = (0..w)
            .map(|x| (22.0 + hash(x as f64, 78.0) * 14.0) as f32)
            .collect();
        // in the frame's rows
        let rain_top = (0..w)
            .map(|x| {
                let x = x as f64 - sd;
                let top = shelf_bot(x) - 1.5;
                if l.trail > 0.0 {
                    (mix(top, BASE - 7.0, smooth(200.0, 214.0, x)) + topf) as f32
                } else {
                    (top + topf) as f32
                }
            })
            .collect();
        let rain_on = (0..w)
            .map(|x| {
                let x = x as f64 - sd;
                // the main curtains, then thinner rain trailing from the anvil
                let on = smooth(106.0, 118.0, x) * smooth(198.0, 186.0, x);
                if l.trail > 0.0 {
                    (on + l.trail * smooth(214.0, 226.0, x) * smooth(266.0, 252.0, x)) as f32
                } else {
                    on as f32
                }
            })
            .collect();

        let bolts = bolts(w, h, sd, top);

        Self {
            l,
            dots: Dots::new(Self::PALETTE),
            sr,
            sg,
            sb,
            mat,
            cloud,
            dark,
            floor_of,
            jit,
            pump_y: pump_y + topf,
            wheat,
            persp,
            field_light,
            gust,
            streak,
            stars,
            star: vec![0f32; n],
            ears,
            ear: vec![0f32; n],
            touched: Vec::with_capacity(touched_max),
            shafts,
            col_phase,
            col_speed,
            rain_top,
            rain_on,
            bolts,
        }
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        let l = &self.l;
        let (w, h, top, hz) = (l.w, l.h, l.top, l.hz);
        let (topf, hzf, hf, fdf) = (top as f64, hz as f64, h as f64, l.fd as f64);
        let base = BASE + topf;
        let mut f = flash_at(t, &self.bolts, l.sd, topf);
        let fl = flicker_at(t, l.sd, topf);
        if let Some(fl) = fl {
            if f.is_none_or(|f| f.i < fl.i) {
                f = Some(fl);
            }
        }
        let big_i = f.map_or(0.0, |f| f.i);
        let inside = f.map_or(0.0, |f| f.inside);
        let drift = t * 1.2;
        let shaft_off = t * 0.9;
        let g_off = t * 9.0;

        for &[k, ph, sp, a] in &self.stars {
            self.star[k as usize] = (a * (0.55 + 0.45 * (t * sp + ph).sin())) as f32;
        }

        // the near ears: each stalk bends from its foot, more at the tip, and
        // the gusts that roll across the field push them further
        for &k in &self.touched {
            self.ear[k] = 0.0;
        }
        self.touched.clear();
        let (ear, touched) = (&mut self.ear, &mut self.touched);
        for &[ex, eh, ph, foot, hl] in &self.ears {
            let gi = ((ex as i64) - (g_off * 1.5).floor() as i64).rem_euclid(GW as i64) as usize;
            let gu = f64::from(self.gust[(h - 1 - hz) * GW + gi]);
            // the inflow blows toward the storm, so every ear bows a little east
            let lean = 0.7 + 0.7 * (t * 1.7 + ph).sin() + 1.8 * gu;
            let tall = eh + hl;
            let mut i = 0.0;
            while i < tall {
                let y = js_round(hf + foot - i);
                let q = i / tall;
                let x = js_round(ex + lean * q * q);
                if y >= hf || x < 0.0 || x + 1.0 >= w as f64 {
                    i += 1.0;
                    continue;
                }
                let k = y as usize * w + x as usize;
                if i < eh {
                    // the stalk: a thin dark line against the field
                    if ear[k] == 0.0 {
                        ear[k] = -1.0;
                        touched.push(k);
                    }
                } else {
                    // the head: plump, two grains wide in the middle, tapering
                    // at both ends, the grains alternating side to side
                    let j = i - eh;
                    let tip = j == hl - 1.0;
                    let v = if tip {
                        0.55
                    } else if j == 0.0 {
                        0.7
                    } else {
                        1.0 - 0.18 * f64::from(j as i32 & 1)
                    };
                    if v > f64::from(ear[k]) {
                        ear[k] = v as f32;
                        touched.push(k);
                    }
                    if !tip && j > 0.0 && v * 0.82 > f64::from(ear[k + 1]) {
                        ear[k + 1] = (v * 0.82) as f32;
                        touched.push(k + 1);
                    }
                    // a whisker of awns past the tip
                    if tip && y > 0.0 {
                        let a = k - w + usize::from(lean > 0.5);
                        if ear[a] < 0.3 {
                            ear[a] = 0.3;
                            touched.push(a);
                        }
                    }
                }
                i += 1.0;
            }
        }

        for r in 0..h {
            let y = r as f64 + 0.5;
            let p = (y - hzf) / (hf - hzf);
            for xi in 0..w {
                let x = xi as f64;
                let xf = x - fdf;
                let k = r * w + xi;
                let m = self.mat[k];
                let (mut cr, mut cg, mut cb) = (
                    f64::from(self.sr[k]),
                    f64::from(self.sg[k]),
                    f64::from(self.sb[k]),
                );
                let (mut floor, mut fade, mut rain) = (f64::from(self.floor_of[k]), 1.0, 0.0);

                if m == SKY {
                    let c = f64::from(self.cloud[k]);
                    if c < 0.98 && (top..hz).contains(&r) {
                        // the altostratus, lit gold toward the sun
                        let sx = x + drift;
                        let ixf = sx.floor();
                        let fx = sx - ixf;
                        let ix = ixf as usize;
                        let row = (r - top) * CW;
                        let s0 = f64::from(self.streak[row + ix % CW]);
                        let s1 = f64::from(self.streak[row + (ix + 1) % CW]);
                        let s = (s0 + (s1 - s0) * fx) * (1.0 - c) * smooth(105.0, 55.0, x - l.sd) * 0.75;
                        if s > 0.005 {
                            let sun = (-((xf - SUN[0]) / 80.0).powi(2)).exp();
                            cr = mix(cr, 0.5 + 0.5 * sun, s);
                            cg = mix(cg, 0.24 + 0.36 * sun, s);
                            cb = mix(cb, 0.34 + 0.06 * sun, s);
                        }
                        if self.star[k] != 0.0 {
                            let v = f64::from(self.star[k]) * (1.0 - c);
                            cr = cr.max(v * 0.9);
                            cg = cg.max(v * 0.88);
                            cb = cb.max(v);
                        }
                    }
                    // rain curtains hanging from the shelf to the ground
                    let on = f64::from(self.rain_on[xi]);
                    let top = f64::from(self.rain_top[xi]);
                    if on > 0.0 && y > top {
                        let u = x + (y - base) * 0.42 - shaft_off;
                        let ui = (u.floor() as i64).rem_euclid(RW as i64) as usize;
                        let env = f64::from(self.shafts[ui])
                            * on
                            * smooth(top, top + 3.0, y)
                            * (1.0 - c * 0.7);
                        if env > 0.01 {
                            // falling streaks, slanted with the shafts
                            let col = (x + (y - base) * 0.42).floor() as usize % w;
                            let fall = (y * 0.4 - t * f64::from(self.col_speed[col]) * 0.1
                                + f64::from(self.col_phase[col])
                                + 1000.0)
                                % 4.0;
                            let lit = if fall < 1.4 { 1.0 } else { 0.0 };
                            rain = env;
                            cr = mix(cr, 0.06 + 0.07 * lit, env * 0.88);
                            cg = mix(cg, 0.065 + 0.075 * lit, env * 0.88);
                            cb = mix(cb, 0.12 + 0.1 * lit, env * 0.88);
                        }
                    }
                } else if m == FIELD || m == PLAIN {
                    let gx = (xi as i64 - (g_off * (0.5 + p)).floor() as i64).rem_euclid(GW as i64);
                    let gu = f64::from(self.gust[(r - hz) * GW + gx as usize]);
                    let g = f64::from(self.sr[k]) - 0.06; // the gold glint just under the horizon
                    if m == FIELD {
                        // stalks lean with the gust and spring back
                        let sway = 0.5 * (t * 2.1 + x * 0.05 + r as f64 * 0.17).sin() + 2.2 * gu;
                        let u = f64::from(self.persp[k]) + sway;
                        let uif = u.floor();
                        let uf = u - uif;
                        let ui = uif as i32;
                        let a0 = f64::from(self.wheat[r * TW + (ui & 511) as usize]);
                        let a1 = f64::from(self.wheat[r * TW + ((ui + 1) & 511) as usize]);
                        let tex = a0 + (a1 - a0) * uf;
                        let v = clamp(0.55 + (tex - 0.5) * (0.9 + 1.4 * p));
                        let l = (v * 0.7 + 0.4 * gu) * f64::from(self.field_light[k]);
                        cr = 0.04 + 0.7 * l + 0.9 * g;
                        cg = 0.03 + 0.48 * l + 0.48 * g;
                        cb = 0.035 + 0.18 * l + 0.12 * g;
                        fade = smooth(hf + 10.0, hf - 8.0, y);
                    } else {
                        let v = 0.75 + 0.45 * gu;
                        cr *= v;
                        cg *= v;
                        cb *= v;
                    }
                    floor = 0.05;
                } else if m == ROAD {
                    let sun = 0.5 + 0.5 * (-((xf - SUN[0]) / 110.0).powi(2)).exp();
                    let rut =
                        if ((xf + 0.5 - road_c(p, l.road)).abs() - road_w(p) * 0.45).abs() < 0.4 + p * 0.8 {
                            0.6
                        } else {
                            1.0
                        };
                    let l = (0.6 + 0.3 * noise(x * 0.5, r as f64 * 0.5, 0.0))
                        * sun
                        * rut
                        * (1.0 - 0.65 * p);
                    (cr, cg, cb) = (0.08 + 0.42 * l, 0.06 + 0.3 * l, 0.07 + 0.26 * l);
                    fade = smooth(hf + 10.0, hf - 8.0, y);
                    floor = 0.05;
                } else if m == BELT {
                    // far trees: dark, hazed toward the rain
                    let h = smooth(130.0, 190.0, x - l.sd);
                    (cr, cg, cb) = (mix(0.07, 0.1, h), mix(0.05, 0.1, h), mix(0.1, 0.17, h));
                    floor = 0.06;
                } else if m == TREE {
                    let rim = noise(x * 0.6, y * 0.6, 0.0);
                    (cr, cg, cb) = (0.06 + 0.06 * rim, 0.04 + 0.03 * rim, 0.07 + 0.04 * rim);
                    floor = 0.03;
                } else if m == HOUSE || m == PUMP {
                    // back-lit by the afterglow: a dark silhouette with a warm western rim
                    let rim = if m == HOUSE && xi as i64 - l.fd == HX0 { 0.18 } else { 0.0 };
                    (cr, cg, cb) = (0.07 + rim, 0.05 + rim * 0.55, 0.09 + rim * 0.3);
                    floor = 0.03;
                } else if m == ROOF {
                    (cr, cg, cb) = (0.1, 0.06, 0.1);
                    floor = 0.03;
                } else if m == PANE {
                    let g = 0.88 + 0.12 * (t * 2.3 + x).sin() * (t * 3.7).sin();
                    let small = if (xi as i64 - l.fd) < 46 { 0.6 } else { 1.0 };
                    (cr, cg, cb) = (g * small, 0.72 * g * small, 0.3 * g * small);
                    floor = 0.3;
                }

                let e = f64::from(self.ear[k]);
                if e > 0.0 {
                    // a grain head, warm and brightest on its sunward edge
                    let sun_w = (-((xf - SUN[0]) / 120.0).powi(2)).exp();
                    let l = e * (0.5 + 0.5 * sun_w);
                    (cr, cg, cb, fade, floor) = (0.95 * l, 0.68 * l, 0.3 * l, 1.0, 0.1);
                }

                // the windpump's wheel, turning slowly, and its tail vane
                let (wx, wy) = (xf + 0.5 - PUMP_X, y - self.pump_y);
                if wx > -4.0 && wx < 7.0 && wy > -4.0 && wy < 4.0 {
                    let wd = (wx * wx + wy * wy).sqrt();
                    if wd < 3.6
                        && wd > 0.4
                        && (((wy.atan2(wx) - t * 1.6) * 8.0).cos() > 0.2 || wd < 1.0)
                    {
                        (cr, cg, cb, floor) = (0.07, 0.05, 0.09, 0.03);
                    }
                    if wy > -1.0 && wy < 0.6 && wx > 2.0 {
                        (cr, cg, cb, floor) = (0.07, 0.05, 0.09, 0.03);
                    }
                }

                // the lit pane's glow on the yard and the wall around it
                let (lx, ly) = (xf + 0.5 - 50.5, y - (HW0 + 3.5 + topf));
                if m != PANE && lx * lx + ly * ly < 200.0 {
                    let lg = (-(lx * lx + ly * ly * 2.0).sqrt() / 3.0).exp() * 0.5;
                    cr += lg;
                    cg += lg * 0.66;
                    cb += lg * 0.25;
                }

                // lightning: the cloud glows from inside; the base and the rain
                // are lit from behind; a full flash reaches the land as well
                if let Some(f) = f.filter(|_| big_i > 0.01) {
                    let (dx, dy) = (x - f.cx, (y - f.cy) * 1.3);
                    let dist = (dx * dx + dy * dy).sqrt();
                    let c = if m == SKY {
                        f64::from(self.cloud[k])
                    } else {
                        0.0
                    };
                    let inner = if inside != 0.0 {
                        (-dist / 14.0).exp() * c * big_i * 2.0
                    } else {
                        (-dist / 16.0).exp() * (0.3 + 0.7 * c) * big_i * 1.3
                    };
                    let back = if m == SKY {
                        (f64::from(self.dark[k]) * 0.5 + rain * 0.8)
                            * (-dx.abs() / 34.0).exp()
                            * big_i
                            * if inside != 0.0 { 0.5 } else { 1.0 }
                    } else {
                        0.0
                    };
                    let amb = if inside != 0.0 {
                        0.0
                    } else {
                        big_i * if m == SKY { 0.08 } else { 0.05 }
                    };
                    let lw = inner + back;
                    cr += 0.75 * lw + amb * 0.8;
                    cg += 0.72 * lw + amb * 0.8;
                    cb += 1.0 * lw + amb;
                    if let (Some(b), true) = (f.bolt, f.bolt_on != 0.0) {
                        let d = f64::from(self.bolts[b].field[k]);
                        if d < 12.0 && (c < 0.6 || y > base - 1.0) {
                            let core = smooth(1.1, 0.35, d) * f.bolt_on;
                            let glow =
                                ((-d / 2.0).exp() * 0.55 + (-d / 7.0).exp() * 0.2) * f.bolt_on;
                            cr += core + glow * 0.75;
                            cg += core + glow * 0.72;
                            cb += core + glow;
                        }
                    }
                }

                if e < 0.0 {
                    // a stalk: one unbroken line of the smallest dots in dark umber
                    out[k] = Cell::new(font::HALFTONE[1], STALK);
                    continue;
                }

                let peak = cr.max(cg).max(cb).max(1e-4);
                let level =
                    clamp(floor + (1.0 - floor) * peak * 0.97 + f64::from(self.jit[k])) * fade;
                let step = Dots::step(level, bayer(r, xi));
                // a small dot is drawn brighter, a large one dimmer, so a
                // gradient stays smooth across the steps; the darkest stay dark
                let want = Dots::want(step, level, 0.04);
                let s = (0.14 + 0.86 * want).min(0.45 + 0.8 * level) / peak;
                out[k] = self.dots.dot(step, [cr, cg, cb], s);
            }
        }
    }
}