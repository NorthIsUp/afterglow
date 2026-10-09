//! desert night: a moonless desert under the milky way. The galaxy's bright
//! core sits low over a lone acacia on a dune crest and arches up across the
//! sky, split by its dark dust lane; a distant town warms the far horizon and
//! catches the dunes' faces. Stars twinkle, sand glints along the ridges, and
//! now and then a meteor crosses.
//!
//! The dunes are a heightfield raymarched once, keeping each cell's depth,
//! light and shadow. The sky is drawn with a random dither so its faint glow
//! reads as star dust, softened toward an ordered one inside the band; the sand
//! keeps an ordered dither so its slopes read as smooth surfaces. Each cell is
//! one dot, in the palette colour nearest its hue.
//!
//! At any size the camera keeps its scale and the frame decides how much dune
//! field it sees: wider panels widen the view, keeping the acacia and the
//! galaxy's core right of centre, push the town out left and sweep the milky
//! way further so its arch spans the sky; narrower ones pan the camera so the
//! acacia stays whole near the right edge; taller ones add sky above and
//! nearer sand below, and steepen the arch so it climbs the extra sky.

use super::halftone::{bayer, Dots};
use super::math::{clamp, hash, js_hypot, js_round, mix, noise, smooth, unit};
use super::{hex, Piece};
use crate::grid::Cell;

const K: f64 = 0.62;
const CAM: f64 = 6.0;
const HMAX: f64 = 16.0; // no dune is taller

const SKY: u8 = 0;
const SAND: u8 = 1;
const TREE: u8 = 2;

/// Upstream's own fbm: each octave is shifted 31.7 along x, unlike the shared one.
fn fbm(x: f64, y: f64, octaves: u32) -> f64 {
    let (mut s, mut n, mut amp, mut f) = (0.0, 0.0, 0.5, 1.0);
    for i in 0..octaves {
        s += amp * noise(x * f + f64::from(i) * 31.7, y * f, 0.0);
        n += amp;
        amp *= 0.5;
        f *= 2.0;
    }
    s / n
}


// Where a point sits across the dunes' wave: 0 at a trough, rising gently up
// the windward side to the crest at 0.72, then dropping down the slip face.
fn phase(x: f64, z: f64) -> f64 {
    let p = x * 0.9 + z * 0.42 + 26.0 * fbm(x * 0.014, z * 0.014, 3);
    let q = p / 19.0;
    q - q.floor()
}

// The tall dune the acacia stands on: a sharp crest that snakes toward us from
// its summit, a gentle face on the town's side and a steep one on the other.
fn crest_x(z: f64) -> f64 {
    let k = clamp((z - 22.0) / 52.0);
    -4.0 + 26.0 * k * k * (3.0 - 2.0 * k) + 5.0 * ((z - 22.0) * 0.09).sin()
}

fn ridge(x: f64, z: f64) -> f64 {
    if z < 8.0 {
        return 0.0;
    }
    let hc = if z <= 74.0 {
        0.5 + 13.0 * clamp((z - 12.0) / 62.0).powf(1.25)
    } else {
        13.5 * (1.0 - (z - 74.0) / 9.0)
    };
    let dx = x - crest_x(z);
    hc - if dx < 0.0 { -dx * 0.36 } else { dx * 0.8 }
}

fn dunes(x: f64, z: f64) -> f64 {
    let u = phase(x, z);
    let prof = if u < 0.72 {
        (u / 0.72).powf(1.5)
    } else {
        ((1.0 - u) / 0.28).powf(0.75)
    };
    let rg = ridge(x, z);
    // the field lies low around the big dune so its faces stay clean
    let amp =
        (1.0 + 4.5 * fbm(x * 0.008 + 5.0, z * 0.008, 2)) * (1.0 - 0.8 * smooth(-3.0, 3.0, rg));
    (prof * amp).max(rg) + 0.3 * fbm(x * 0.05, z * 0.05, 2)
}

fn march(u: f64, v: f64) -> f64 {
    let (mut z, mut prev) = (3.0, 3.0);
    for _ in 0..300 {
        if z >= 600.0 {
            break;
        }
        let y = CAM + v * z;
        if v > 0.0 && y > HMAX {
            return 0.0;
        }
        let gap = y - dunes(u * z, z);
        if gap < 0.0 {
            let (mut a, mut b) = (prev, z);
            for _ in 0..7 {
                let m = (a + b) / 2.0;
                if CAM + v * m - dunes(u * m, m) < 0.0 {
                    b = m;
                } else {
                    a = m;
                }
            }
            return b;
        }
        prev = z;
        z += (gap * 0.5).max(0.25) + z * 0.004;
    }
    0.0
}

struct Star {
    k: usize,
    bright: f64,
    tint: [f64; 3],
    rate: f64,
    ph: f64,
}

/// Where things sit in a `w x h` frame. Upstream's 200x100 is the anchor
/// every value moves from, so there it is exact.
struct Layout {
    w: usize,
    h: usize,
    /// Eye level, in rows.
    hz: f64,
    /// The camera's centre column.
    cx: f64,
    /// The far glow, just under the horizon.
    town: [f64; 2],
    /// The galaxy's bright centre.
    core: [f64; 2],
    /// The milky way's arch: centre and radius.
    arc: [f64; 3],
    /// Its angle at the core, and the sweep to the west edge.
    arc_s: [f64; 2],
    acacia: usize,
    /// Where meteors start: the first one's column, and the span the rest start
    /// in from column 30.
    meteor_x: [f64; 2],
}

/// The arch's radius.
const ARC_R: f64 = 199.8;

/// How far round from `a0` an arch centred at `c` runs before it leaves a
/// frame by its west or top edge.
fn arc_sweep(c: [f64; 2], a0: f64) -> f64 {
    let west = -((-2.0 - c[0]) / ARC_R).clamp(-1.0, 1.0).acos();
    let top = ((-2.0 - c[1]) / ARC_R).clamp(-1.0, 1.0).asin();
    a0 - west.max(top)
}

impl Layout {
    fn new(w: usize, h: usize) -> Self {
        // `wide` is 1 at 3.2:1, the old `-wide` recomposition; `narrow` is 1
        // at square; `tall` is the rows past upstream's 100.
        let (wf, tall) = (w as f64, h as f64 - 100.0);
        let wide = (wf - 200.0) / 120.0;
        let narrow = clamp((200.0 - wf) / 100.0);
        let grow = |at: f64, by_wide: f64, by_narrow: f64| {
            if wf >= 200.0 {
                at + by_wide * wide
            } else {
                at - by_narrow * narrow
            }
        };
        // Two thirds of the extra rows are sky: the milky way is the subject.
        let sky = (tall * 0.6).round();
        // The acacia and the core hold 50 columns right of the camera, so
        // narrowing pans the camera left to keep the tree in frame.
        let cx = grow(100.0, 60.0, 78.0);
        let core = [cx + 52.0, 38.0 + sky];
        // A portrait frame turns the arch up so it climbs the extra sky.
        let a0 = -1.035 + 0.45 * clamp(tall / 78.0);
        let arc = [
            50.0 + ((core[0] - ARC_R * a0.cos()) - (152.0 - ARC_R * (-1.035f64).cos())),
            209.8 + ((core[1] - ARC_R * a0.sin()) - (38.0 - ARC_R * (-1.035f64).sin())),
            ARC_R,
        ];
        let sweep = arc_sweep([arc[0], arc[1]], a0) - arc_sweep([50.0, 209.8], -1.035);
        Self {
            w,
            h,
            hz: 64.0 + sky,
            cx,
            town: [grow(30.0, 50.0, 15.0), 66.0 + sky],
            core,
            arc,
            arc_s: [a0, 0.8 + sweep],
            acacia: (cx + 50.0) as usize,
            meteor_x: [wf - 1.0, wf - 60.0],
        }
    }

    /// Meteor 0 is already falling; meteor n starts somewhere in its 7 second
    /// slot. `[start, x0, y0, dx, dy, speed]`.
    fn meteor(&self, n: usize) -> [f64; 6] {
        let nf = n as f64;
        let start = if n == 0 {
            -0.35
        } else {
            7.0 * (nf - 1.0) + 2.5 + hash(nf, 71.0) * 2.5
        };
        let x0 = if n == 0 {
            self.meteor_x[0]
        } else {
            30.0 + hash(nf, 72.0) * self.meteor_x[1]
        };
        let y0 = if n == 0 {
            4.0
        } else {
            4.0 + hash(nf, 73.0) * 18.0
        };
        // each one heads across the sky rather than straight off its nearer edge
        let a = if n == 0 {
            2.65
        } else {
            (if x0 > self.w as f64 / 2.0 { 2.6 } else { 0.55 }) + (hash(nf, 75.0) - 0.5) * 0.4
        };
        [
            start,
            x0,
            y0,
            a.cos(),
            a.sin(),
            52.0 + hash(nf, 76.0) * 30.0,
        ]
    }
}

pub struct DesertNight {
    l: Layout,
    dots: Dots,
    mat: Vec<u8>,
    s_r: Vec<f32>,
    s_g: Vec<f32>,
    s_b: Vec<f32>,
    crest: Vec<f32>,
    bark: Vec<f32>,
    /// `[cell, phase, rate]`
    lamps: Vec<[f64; 3]>,
    k_r: Vec<f32>,
    k_g: Vec<f32>,
    k_b: Vec<f32>,
    air: Vec<f32>,
    dith: Vec<f32>,
    floor0: Vec<f32>,
    stars: Vec<Star>,
    f_r: Vec<f32>,
    f_g: Vec<f32>,
    f_b: Vec<f32>,
    floor: Vec<f32>,
}

impl Piece for DesertNight {
    const NAME: &'static str = "desert-night";
    const FPS: u32 = 15;
    const GROUND: u32 = hex("#04060c");
    #[rustfmt::skip]
    const PALETTE: &'static [u32] = &[
        hex("#0a1024"), hex("#101830"), hex("#16213f"), hex("#1e2b50"), hex("#283864"), hex("#34477a"), hex("#45598f"), hex("#5a6fa6"), hex("#7488bd"),
        hex("#93a5d2"), hex("#b6c3e4"), hex("#d8e0f2"), hex("#f4f6fb"),
        hex("#fff3dc"), hex("#ffe2b4"), hex("#f5c98e"), hex("#e2a86e"), hex("#c4864f"), hex("#9c6440"), hex("#74492f"),
        hex("#2a2230"), hex("#3d2f3a"), hex("#56404a"), hex("#735358"), hex("#946a62"), hex("#b6836c"),
        hex("#4a3b48"), hex("#6b5562"), hex("#8a6f7a"), hex("#a88590"), hex("#c9a3a3"), hex("#e2bfb4"),
        hex("#2a2448"), hex("#3d3466"), hex("#57498a"), hex("#7a68a8"),
        hex("#ffd8a8"), hex("#cfe0ff"), hex("#a9c4ff"),
        hex("#ff9a52"), hex("#e07a3e"),
        hex("#0d0f1a"), hex("#151827"),
    ];

    fn new(cols: usize, rows: usize) -> Self {
        let lay = Layout::new(cols, rows);
        let (w, h, hz) = (lay.w, lay.h, lay.hz);
        let n = w * h;
        // the town's light comes in low from ahead and to the left
        let l = unit([-0.8, 0.32, 0.5]);
        // the galaxy's cool fill, from up and to the right
        let g = unit([0.6, 0.5, 0.4]);

        // --- the dunes, raymarched once --------------------------------------
        let mut mat = vec![SKY; n];
        let mut depth = vec![0f32; n];
        let (mut s_r, mut s_g, mut s_b) = (vec![0f32; n], vec![0f32; n], vec![0f32; n]);
        let mut crest = vec![0f32; n];
        let mut top = vec![h as i16; w];
        for r in 0..h {
            let v = ((hz - (r as f64 + 0.5)) / 100.0) * K;
            for (x, col_top) in top.iter_mut().enumerate() {
                let xf = x as f64;
                let u = ((xf + 0.5 - lay.cx) / 100.0) * K;
                let z = march(u, v);
                if z == 0.0 {
                    continue;
                }
                let k = r * w + x;
                mat[k] = SAND;
                depth[k] = z as f32;
                if (r as i16) < *col_top {
                    *col_top = r as i16;
                }
                let (px, py) = (u * z, CAM + v * z);
                let e = 0.2;
                let hx = (dunes(px + e, z) - dunes(px - e, z)) / (2.0 * e);
                let gz = (dunes(px, z + e) - dunes(px, z - e)) / (2.0 * e);
                let nl = js_hypot(&[hx, 1.0, gz]);
                let (nx, ny, nz) = (-hx / nl, 1.0 / nl, -gz / nl);
                let mut lit = (nx * l[0] + ny * l[1] + nz * l[2]).max(0.0);
                if lit > 0.0 {
                    // a soft shadow: how close the ray to the light passes over the sand
                    let mut sh: f64 = 1.0;
                    let mut s = 0.6;
                    while s < 90.0 {
                        let (qx, qy, qz) = (px + l[0] * s, py + l[1] * s + 0.05, z + l[2] * s);
                        if qy > HMAX {
                            break;
                        }
                        sh = sh.min((qy - dunes(qx, qz)) / (0.08 * s));
                        if sh <= 0.0 {
                            break;
                        }
                        s += 0.5 + s * 0.05;
                    }
                    lit *= clamp(sh);
                }
                lit = (lit.powf(1.4) * 1.45).min(1.0); // faces square to the light stand out
                                                       // brightest along the crests, shading down each face into the trough
                let ph = phase(px, z);
                let on_ridge = ridge(px, z) >= dunes(px, z) - 0.35;
                let dxc = px - crest_x(z);
                let fall = if on_ridge {
                    0.5 + 0.5 * (-dxc.abs() / 7.0).exp()
                } else {
                    0.55 + 0.45 * smooth(0.2, 0.72, ph)
                };
                let near = 0.72 + 0.28 * smooth(5.0, 25.0, z);
                let reach = (0.55 + 0.45 * (-(xf - lay.town[0]).abs() / 90.0).exp())
                    * fall
                    * near
                    * smooth(300.0, 120.0, z);
                // starlight from the galaxy's side, and wind ripples across every slope
                let fill = 0.045 * (nx * g[0] + ny * g[1] + nz * g[2]).max(0.0);
                let ripple = (fbm(px * 0.6 + z * 0.2, z * 0.6, 2) - 0.5) * 0.05;
                let amb = (0.02 + 0.03 * ny + ripple) * near;
                let mut cr = amb * 0.7 + fill * 0.45 + lit * 1.0 * reach;
                let mut cg = amb * 0.62 + fill * 0.65 + lit * 0.63 * reach;
                let mut cb = amb * 1.35 + fill * 1.2 + lit * 0.53 * reach;
                // the big dune's shadowed brink catches a line of starlight
                if on_ridge && z < 76.0 && dxc > 0.0 {
                    let cell_w = z * K * 0.01;
                    let rim = smooth(2.2 * cell_w, 0.6 * cell_w, dxc) * smooth(10.0, 20.0, z);
                    cr += 0.1 * rim;
                    cg += 0.14 * rim;
                    cb += 0.24 * rim;
                }
                // distance thins the light into the horizon's haze
                let haze =
                    clamp(1.0 - (-z / 260.0).exp()) * mix(0.7, 0.92, smooth(120.0, 300.0, z));
                let hg = (-(xf - lay.town[0]).abs() / 40.0).exp() * 0.18;
                cr = mix(cr, 0.07 + hg, haze);
                cg = mix(cg, 0.085 + hg * 0.6, haze);
                cb = mix(cb, 0.15 + hg * 0.3, haze);
                s_r[k] = cr as f32;
                s_g[k] = cg as f32;
                s_b[k] = cb as f32;
                let big = if ridge(px, z) > 0.5 && z < 76.0 {
                    smooth(0.9, 0.0, dxc.abs())
                } else {
                    0.0
                };
                crest[k] = ((smooth(0.07, 0.0, (ph - 0.72).abs()) * smooth(260.0, 30.0, z))
                    .max(big)
                    * (0.3 + 0.7 * smooth(0.0, 0.2, lit))) as f32;
            }
        }

        // --- the acacia, on the crest under the galaxy's core -----------------
        let mut bark = vec![0f32; n]; // warm light from the core caught on the canopy's top
        {
            let ac = lay.acacia as f64;
            let base = f64::from(top[lay.acacia]);
            let canopy_top = base - 16.0;
            // an umbrella: a low lumpy dome on top, thinning to the tips, tufts below
            const SPAN: f64 = 17.0;
            let canopy = |x: f64, px: f64, py: f64| {
                let q = (px - ac) / SPAN;
                if q.abs() > 1.08 {
                    return false;
                }
                let top_y =
                    canopy_top + 0.4 + 2.6 * q * q + 1.3 * (noise(px * 0.3, 7.1, 0.0) - 0.5);
                let bot_y = canopy_top + 3.8 + 1.4 * (1.0 - q * q)
                    - 1.4 * smooth(0.8, 1.08, q.abs())
                    + 1.2 * (noise(px * 0.45, 3.3, 0.0) - 0.5)
                    + if hash(x, 51.0) < 0.2 { 0.9 } else { 0.0 };
                py > top_y && py < bot_y
            };
            // limbs: from the fork up and out to the canopy
            let fork = [ac + 0.3, base - 5.0];
            let limbs: [([f64; 2], [f64; 2], f64); 7] = [
                ([ac, base + 1.0], fork, 1.3),
                (fork, [ac - 9.0, canopy_top + 3.0], 0.8),
                (fork, [ac + 1.5, canopy_top + 2.0], 0.75),
                (fork, [ac + 10.0, canopy_top + 3.2], 0.8),
                (fork, [ac - 4.0, canopy_top + 2.6], 0.55),
                ([ac - 4.0, base - 9.0], [ac - 14.0, canopy_top + 3.5], 0.55),
                ([ac + 3.0, base - 8.0], [ac + 15.0, canopy_top + 3.6], 0.5),
            ];
            let r0 = (canopy_top - 3.0).max(0.0) as usize;
            // upstream's typed arrays drop writes past the last row
            let r1 = (base as usize + 1).min(h - 1);
            for r in r0..=r1 {
                for x in lay.acacia.saturating_sub(26)..=(lay.acacia + 26).min(w - 1) {
                    let (px, py) = (x as f64 + 0.5, r as f64 + 0.5);
                    let mut on = canopy(x as f64, px, py);
                    for &([ax, ay], [bx, by], w) in &limbs {
                        let (lx, ly) = (bx - ax, by - ay);
                        let tt = clamp(((px - ax) * lx + (py - ay) * ly) / (lx * lx + ly * ly));
                        let (dx, dy) = (px - (ax + lx * tt), py - (ay + ly * tt));
                        if dx * dx + dy * dy < (w * (1.0 - 0.4 * tt)).powi(2) {
                            on = true;
                        }
                    }
                    if on {
                        mat[r * w + x] = TREE;
                    }
                }
            }
            // the canopy's upper edge, rimmed by the bulge behind it
            for r in 1..h {
                for x in 0..w {
                    let k = r * w + x;
                    if mat[k] != TREE || mat[k - w] != SKY {
                        continue;
                    }
                    let (dx, dy) = (x as f64 + 0.5 - lay.core[0], (r as f64 - lay.core[1]) * 1.3);
                    bark[k] = (0.25 + 0.75 * (-(dx * dx + dy * dy).sqrt() / 18.0).exp()) as f32;
                }
            }
        }

        // --- the town's lights, a few pinpricks along the far horizon ---------
        let mut lamps: Vec<[f64; 3]> = Vec::with_capacity(5);
        let town = lay.town[0] as usize;
        for x in town.saturating_sub(9)..=(town + 9).min(w - 1) {
            if lamps.len() >= 5 {
                break;
            }
            let xf = x as f64;
            let r = top[x] as usize;
            if r >= h || depth[r * w + x] < 140.0 || hash(xf, 91.0) > 0.45 {
                continue;
            }
            if lamps.iter().any(|l| (l[0] as usize % w).abs_diff(x) < 2) {
                continue;
            }
            lamps.push([
                (r * w + x) as f64,
                hash(xf, 92.0) * 6.28,
                0.7 + hash(xf, 93.0) * 1.4,
            ]);
        }

        // --- the sky: gradient, the town's glow and the milky way ------------
        let (mut k_r, mut k_g, mut k_b) = (vec![0f32; n], vec![0f32; n], vec![0f32; n]);
        let mut air = vec![0f32; n];
        let mut band = vec![0f32; n];
        for r in 0..h {
            for xi in 0..w {
                let x = xi as f64;
                let k = r * w + xi;
                let y = r as f64 + 0.5;
                let v = clamp(y / hz);
                // a saturated navy, so the dither's specks read as colour
                let mut cr = 0.02 + 0.03 * v * v;
                let mut cg = 0.035 + 0.045 * v * v;
                let mut cb = 0.1 + 0.08 * v * v;
                // the town: an amber dome rising from below the horizon, its warmth
                // taking over from the navy rather than greying it
                let (tx, ty) = (x + 0.5 - lay.town[0], (y - lay.town[1]) * 1.9);
                let td = (tx * tx + ty * ty).sqrt();
                let glow = (-td / 16.0).exp() * 0.4 + (-td / 60.0).exp() * 0.22;
                let cool = 1.0 - 0.85 * clamp((-td / 22.0).exp() * 1.5);
                cr = cr * cool + glow;
                cg = cg * cool + glow * 0.5;
                cb = cb * cool + glow * 0.16;
                // airglow, a faint blue sheen low down, kept off the town; it drifts each frame
                // it peaks a little above the horizon, where the air below thins it, and
                // varies along the skyline so it never lies as one flat strip
                let along = 0.72 + 0.5 * fbm(x * 0.025 + 3.0, y * 0.04, 2);
                air[k] = (smooth(0.3, 0.94, v).powf(3.0)
                    * (1.0 - 0.3 * smooth(0.92, 1.02, v))
                    * 0.32
                    * cool
                    * along) as f32;
                // the milky way, along an arc from the core up and over to the west
                let (ax, ay) = (x + 0.5 - lay.arc[0], y - lay.arc[1]);
                let d = (ax * ax + ay * ay).sqrt() - lay.arc[2];
                let s_raw = (lay.arc_s[0] - ay.atan2(ax)) / lay.arc_s[1]; // 0 at the core end
                let s = clamp(s_raw);
                // the band ends at the core
                let end = if s_raw < 0.0 {
                    (-(s_raw / 0.05).powi(2)).exp()
                } else {
                    1.0
                };
                let lane_end = if s_raw < 0.0 {
                    (-(s_raw / 0.11).powi(2)).exp()
                } else {
                    1.0
                };
                let w = 9.0 + 8.0 * (1.0 - s).powf(1.4);
                // the lanes stretch with a longer sweep
                let lane_s = s * (9.0 * (lay.arc_s[1] / 0.8));
                let lane0 = (noise(lane_s, 3.3, 0.0) - 0.5) * w * 0.5 * smooth(0.0, 0.25, s);
                let mut b = (-(d / w).powi(2)).exp();
                let clump = fbm(x * 0.09, y * 0.09, 4);
                b *= 0.35 + 1.1 * smooth(0.3, 0.75, clump);
                b *= (0.55 + 0.55 * (1.0 - s)) * end;
                // the core's bulge
                let (cx, cy) = (x + 0.5 - lay.core[0], (y - lay.core[1]) * 1.3);
                let cd = (cx * cx + cy * cy).sqrt();
                let core = (-cd / 6.0).exp() * 0.5 + (-cd / 22.0).exp() * 0.25;
                b += core;
                // the dust lane, a crisp rift through the bulge, and its filaments
                let near_core = (-cd / 16.0).exp();
                let lane = (-((d - lane0) / (w * 0.12)).powi(2)).exp()
                    * mix(
                        (0.55 + 0.6 * fbm(x * 0.12, y * 0.12, 3)) * (0.55 + 0.45 * (1.0 - s)),
                        1.0,
                        near_core,
                    )
                    * lane_end;
                let fil = smooth(0.58, 0.78, fbm(x * 0.16 + 9.0, y * 0.16, 3)) * 0.6;
                b *= clamp(1.0 - mix(0.92, 0.97, near_core) * lane)
                    * (1.0 - fil * (-(d / (w * 1.3)).powi(2)).exp());
                band[k] = ((-(d / (w * 1.2)).powi(2)).exp() * end + core) as f32;
                // warm toward the core, cool along the arm
                let warm = clamp((-cd / 26.0).exp() * 1.2 + (1.0 - s) * 0.25);
                cr += b * mix(0.62, 1.0, warm) * 0.75;
                cg += b * mix(0.68, 0.9, warm) * 0.75;
                cb += b * mix(0.95, 0.74, warm) * 0.75;
                k_r[k] = cr as f32;
                k_g[k] = cg as f32;
                k_b[k] = cb as f32;
            }
        }

        // per-cell dither and floor: random in open sky, half ordered in the band
        let mut dith = vec![0f32; n];
        let mut floor0 = vec![0f32; n];
        for r in 0..h {
            for x in 0..w {
                let k = r * w + x;
                let bay = bayer(r, x);
                if mat[k] == SKY {
                    // the band and the glow low down get a wider, half-ordered dither, so
                    // their gradients fade out instead of stopping at an edge
                    let inb = clamp(f64::from(band[k]));
                    let soft = inb.max(clamp(f64::from(air[k]) / 0.2));
                    let rnd = (hash(x as f64 * 3.0 + 1.0, r as f64 * 5.0 + 2.0) - 0.5)
                        * mix(0.6, 0.94, soft);
                    dith[k] = mix(rnd, bay, 0.5 * soft) as f32;
                    floor0[k] = (0.1 * inb) as f32;
                } else {
                    dith[k] = bay as f32;
                    floor0[k] = if mat[k] == SAND { 0.02 } else { 0.0 };
                }
            }
        }

        // --- stars: thick in the band, a few bright ones everywhere -----------
        let mut stars = Vec::new();
        for r in 0..hz as usize + 2 {
            for xi in 0..w {
                let k = r * w + xi;
                if mat[k] != SKY {
                    continue;
                }
                let (x, rf) = (xi as f64, r as f64);
                let p = 0.015 + 0.12 * clamp(f64::from(band[k]));
                if hash(x * 7.0 + 3.0, rf * 13.0 + 1.0) > p {
                    continue;
                }
                let m = hash(x * 5.0 + 1.0, rf * 3.0 + 7.0).powf(12.0);
                let bright = (0.2 + 0.95 * m) * smooth(hz + 1.0, hz - 6.0, rf + 0.5);
                let c = hash(x * 11.0, rf * 17.0 + 5.0);
                let tint = if c < 0.2 {
                    [0.78, 0.86, 1.0]
                } else if c < 0.9 {
                    [1.0, 1.0, 1.0]
                } else {
                    [1.0, 0.82, 0.6]
                };
                stars.push(Star {
                    k,
                    bright,
                    tint,
                    rate: 1.5 + hash(x, rf * 9.0) * 4.0,
                    ph: hash(rf, x * 3.0) * 6.28,
                });
            }
        }

        Self {
            l: lay,
            dots: Dots::new(Self::PALETTE),
            mat,
            s_r,
            s_g,
            s_b,
            crest,
            bark,
            lamps,
            k_r,
            k_g,
            k_b,
            air,
            dith,
            floor0,
            stars,
            f_r: vec![0f32; n],
            f_g: vec![0f32; n],
            f_b: vec![0f32; n],
            floor: vec![0f32; n],
        }
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        let (w, h, hz) = (self.l.w, self.l.h, self.l.hz);
        self.f_r.copy_from_slice(&self.k_r);
        self.f_g.copy_from_slice(&self.k_g);
        self.f_b.copy_from_slice(&self.k_b);
        self.floor.copy_from_slice(&self.floor0);
        let (fr, fg, fb, floor) = (&mut self.f_r, &mut self.f_g, &mut self.f_b, &mut self.floor);
        let bump = |v: &mut f32, d: f64| *v = (f64::from(*v) + d) as f32;
        // the airglow drifts slowly along the horizon
        let air0 = (hz * 0.3).floor() as usize;
        for r in air0..hz as usize + 2 {
            for x in 0..w {
                let k = r * w + x;
                if self.mat[k] != SKY {
                    continue;
                }
                let a = f64::from(self.air[k])
                    * (0.86 + 0.28 * noise((x as f64 - t * 0.3) * 0.03, r as f64 * 0.08, 0.0));
                bump(&mut fr[k], a * 0.3);
                bump(&mut fg[k], a * 0.5);
                bump(&mut fb[k], a * 1.2);
            }
        }
        for k in 0..w * h {
            let m = self.mat[k];
            if m == SAND {
                fr[k] = self.s_r[k];
                fg[k] = self.s_g[k];
                fb[k] = self.s_b[k];
                let c = f64::from(self.crest[k]);
                if c > 0.0 {
                    // sand glinting along the brink of each slip face
                    let h = hash(k as f64, 5.0);
                    let g = c * (t * (0.8 + 2.2 * h) + h * 40.0).sin().max(0.0).powf(8.0) * 0.5;
                    bump(&mut fr[k], g);
                    bump(&mut fg[k], g * 0.85);
                    bump(&mut fb[k], g * 0.7);
                }
            } else if m == TREE {
                let e = f64::from(self.bark[k]);
                fr[k] = (e * 0.42) as f32;
                fg[k] = (e * 0.3) as f32;
                fb[k] = (e * 0.2) as f32;
                floor[k] = if e > 0.0 { 0.1 } else { 0.0 };
            }
        }
        for s in &self.stars {
            let tw = if s.bright > 0.35 {
                0.72 + 0.28 * (t * s.rate + s.ph).sin()
            } else {
                0.6 + 0.4 * (t * s.rate + s.ph).sin()
            };
            let v = s.bright * tw;
            bump(&mut fr[s.k], v * s.tint[0]);
            bump(&mut fg[s.k], v * s.tint[1]);
            bump(&mut fb[s.k], v * s.tint[2]);
            floor[s.k] = 0.28;
        }
        for &[k, ph, rate] in &self.lamps {
            let k = k as usize;
            let g = 0.42 + 0.14 * (t * rate + ph).sin() * (t * rate * 2.3 + ph * 3.0).sin();
            fr[k] = g as f32;
            fg[k] = (0.85 * g) as f32;
            fb[k] = (0.66 * g) as f32;
            floor[k] = 0.12;
        }
        // a meteor, if one is crossing
        let n = ((t - 2.5) / 7.0).floor() + 1.0;
        let first = (n - 1.0).max(0.0) as usize;
        for i in first..=(n + 1.0) as usize {
            let [start, x0, y0, dx, dy, speed] = self.l.meteor(i);
            let age = t - start;
            if !(0.0..=0.9).contains(&age) {
                continue;
            }
            let (fade_in, fade_out) = (smooth(0.0, 0.12, age), smooth(0.9, 0.6, age));
            let (hx, hy) = (x0 + dx * speed * age, y0 + dy * speed * age * 0.75);
            let len = (speed * age).min(28.0);
            let mut j = 0.0;
            while j < len * 2.0 {
                let f = j / (len * 2.0);
                j += 1.0;
                let px = js_round(hx - dx * f * len);
                let py = js_round(hy - dy * f * len * 0.75);
                if px < 0.0 || px >= w as f64 || py < 0.0 || py >= h as f64 {
                    continue;
                }
                let k = py as usize * w + px as usize;
                if self.mat[k] != SKY {
                    continue;
                }
                let s = (1.0 - f) * fade_in * fade_out * 1.25;
                fr[k] = f64::from(fr[k]).max(s * 0.9) as f32;
                fg[k] = f64::from(fg[k]).max(s * 0.96) as f32;
                fb[k] = f64::from(fb[k]).max(s) as f32;
                floor[k] = f64::from(floor[k]).max(if s > 0.12 { 0.4 } else { 0.0 }) as f32;
            }
        }

        let hf = h as f64;
        for r in 0..h {
            let edge = smooth(hf + 1.0, hf - 12.0, r as f64 + 0.5);
            for x in 0..w {
                let k = r * w + x;
                let rgb = [f64::from(fr[k]), f64::from(fg[k]), f64::from(fb[k])];
                let fl = f64::from(floor[k]);
                let peak = rgb[0].max(rgb[1]).max(rgb[2]).max(1e-4);
                let gamma = if self.mat[k] == SKY { 0.92 } else { 0.75 };
                let level = clamp(fl + (1.0 - fl) * peak.powf(gamma)) * edge;
                let step = Dots::step(level, f64::from(self.dith[k]));
                out[k] = self.dots.ink(step, level, rgb, peak);
            }
        }
    }
}
