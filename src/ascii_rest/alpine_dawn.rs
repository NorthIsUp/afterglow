//! alpine dawn: jagged snow peaks catch the first pink light on their east
//! faces while their flanks stay in blue shadow. Mist pools along the far shore
//! and a still lake mirrors it all. The light warms toward gold as the sun
//! clears the ridge, the mist drifts, and slow ripples cross the water.
//!
//! The range is a heightfield, raymarched once from a camera just above the
//! water: each cell keeps its depth, height, sunlight (with cast shadows) and
//! snow cover, so a frame only re-tints it. The lake looks up the picture above
//! it along each cell's reflected ray.
//!
//! Upstream's `Float32Array`s stay `f32` here, as in `night_coast`. Its colour
//! tail dims shadowed dots further than [`Dots::ink`] does, so that is inline.
//!
//! At any size the camera keeps upstream's scale and the range stays centred:
//! wider panels see further along it, with more peaks rising at the flanks;
//! narrower ones draw the summits in toward the middle, so the sun keeps its
//! place east of centre; taller ones add sky above and lake below.

use std::f64::consts::PI;

use super::halftone::{bayer, Dots};
use super::math::{clamp, fbm, hash, js_hypot, js_round, mix, noise, smooth};
use super::{hex, Piece};
use crate::grid::Cell;

const K: f64 = 0.62; // tangent of half the field of view, across the width
const HZ: f64 = 56.5; // eye level, in rows
const CAM: f64 = 1.5; // camera height above the water
const SHORE_Z: f64 = 46.0; // distance to the far shore
const SHORE: usize = 62; // first row of open water, before a tall frame's extra sky
/// The mist sheet: wraps every `MW` columns, rows `M0..M0 + MR`.
const MW: usize = 480;
const M0: usize = 36;
const MR: usize = SHORE + 2 - M0;
/// The high cloud: wraps every `CW` columns, the top `CR` rows.
const CW: usize = 640;
const CR: usize = 34;

/// Sharp crests where plain noise crosses its middle: rock ribs and couloirs.
fn ridged(x: f64, y: f64, octaves: u32) -> f64 {
    let (mut s, mut n, mut amp, mut f) = (0.0, 0.0, 0.5, 1.0);
    for i in 0..octaves {
        let v = 1.0 - (2.0 * noise(x * f + f64::from(i) * 17.3, y * f, 0.0) - 1.0).abs();
        s += amp * v * v;
        n += amp;
        amp *= 0.5;
        f *= 2.1;
    }
    s / n
}

struct Terrain {
    /// [x, z, height, spread, cos, sin] per peak.
    peaks: Vec<[f64; 6]>,
}

impl Terrain {
    /// `specs` as [`Layout::peaks`], seen from a camera centred on column `cx`.
    fn new(specs: &[[f64; 5]], cx: f64) -> Self {
        Self {
            peaks: specs
                .iter()
                .map(|&[sx, row, z, f, a]| {
                let h = CAM + ((HZ - row) / 100.0) * K * z - 1.5;
                [
                    ((sx - cx) / 100.0) * K * z,
                    z,
                    h,
                    h * f,
                    a.cos(),
                    a.sin(),
                ]
            })
            .collect(),
        }
    }

    fn height(&self, x: f64, z: f64) -> f64 {
        let mut h = 0.0;
        for &[px, pz, ph, pr, c, s] in &self.peaks {
            let (dx, dz) = (x - px, z - pz);
            let (rx, rz) = (dx * c - dz * s, dx * s + dz * c);
            let v = ph * (1.0 - (rx.abs() + rz.abs()) / pr);
            if v > h {
                h = v;
            }
        }
        let hills =
            (1.0 + 3.0 * fbm(x * 0.04, z * 0.04, 3, 0.0)) * smooth(SHORE_Z, SHORE_Z + 15.0, z);
        if hills > h {
            h = hills;
        }
        // crags, deeper on the high ground
        h += ((ridged(x * 0.06, z * 0.06, 3) - 0.45) * 6.0
            + (ridged(x * 0.2, z * 0.2, 2) - 0.45) * 1.6)
            * smooth(4.0, 22.0, h);
        h
    }

    /// Distance along a ray from height `oy` with slope `v` and spread `u` to
    /// the terrain beyond the shore, or 0 when it reaches the sky.
    fn march(&self, u: f64, v: f64, oy: f64) -> f64 {
        let (mut z, mut prev) = (SHORE_Z, SHORE_Z);
        let mut i = 0;
        while i < 260 && z < 520.0 {
            let gap = oy + v * z - self.height(u * z, z);
            if gap < 0.0 {
                let (mut a, mut b) = (prev, z);
                for _ in 0..7 {
                    let m = (a + b) / 2.0;
                    if oy + v * m - self.height(u * m, m) < 0.0 {
                        b = m;
                    } else {
                        a = m;
                    }
                }
                return b;
            }
            prev = z;
            z += (gap * 0.45).max(0.35) + z * 0.002;
            i += 1;
        }
        0.0
    }
}

/// Upstream's peaks, as [`Layout::peaks`] around its centre column 100.
const RANGE: [[f64; 5]; 7] = [
    [66.0, 11.0, 140.0, 0.95, 0.3],
    [38.0, 26.0, 115.0, 1.1, -0.15],
    [116.0, 22.0, 175.0, 0.9, 0.2],
    [92.0, 33.0, 150.0, 1.0, 0.1],
    [96.0, 25.0, 340.0, 1.1, 0.4],
    [180.0, 38.0, 200.0, 1.5, -0.1],
    [8.0, 30.0, 160.0, 1.2, 0.25],
];
/// The flank peaks a wider frame raises, their columns from the nearer edge
/// (negative: the right one), as the old 3.2:1 recomposition placed them.
const FLANKS: [[f64; 5]; 3] = [
    [26.0, 17.0, 135.0, 1.0, -0.2],
    [-38.0, 21.0, 165.0, 1.05, 0.15],
    [-14.0, 31.0, 240.0, 1.3, -0.3],
];
/// Upstream's near pines, [column, tip row, size]; the last three hold the
/// right edge.
const PINES: [[f64; 3]; 6] = [
    [9.0, 5.0, 1.15],
    [20.0, 38.0, 0.85],
    [32.0, 66.0, 0.5],
    [192.0, 10.0, 1.15],
    [181.0, 40.0, 0.75],
    [204.0, 26.0, 1.0],
];

/// Where things sit in a `w x h` frame. Upstream's 200x100 is the anchor
/// every value moves from, so there it is exact. Rows in `peaks`, `pines` and
/// the sky's shading are upstream's; a tall frame's extra sky is `top` rows
/// above them.
struct Layout {
    w: usize,
    h: usize,
    top: usize,
    /// First row of open water.
    shore: usize,
    /// The camera's centre column.
    cx: f64,
    sun: [f64; 2],
    /// Peaks as pyramids, each turned a little, placed by where their summits
    /// should land on screen: [column, row, distance, spread, turn].
    peaks: Vec<[f64; 5]>,
    /// The near pines: [column, tip row, size].
    pines: [[f64; 3]; 6],
    /// Where the sky has turned fully toward the eastern light.
    east: f64,
    /// The western sky, where stars linger and the cloud thins: between these.
    west: [f64; 2],
    /// Where the valley mist reaches full strength.
    mist: f64,
}

impl Layout {
    fn new(w: usize, h: usize) -> Self {
        let wf = w as f64;
        let tall = h - 100;
        let top = tall / 2;
        let cx = wf / 2.0;
        // Narrower than 2:1 the summits close in rather than leave the frame;
        // wider, the camera's scale holds and the frame sees more of the range.
        let squeeze = (wf / 200.0).min(1.0);
        let at = |x: f64| cx + (x - 100.0) * squeeze;
        let mut peaks: Vec<[f64; 5]> = RANGE.iter().map(|&[x, r, z, f, a]| [at(x), r, z, f, a]).collect();
        // The flanks rise out of the lake as the frame widens, full by 3:1.
        let rise = clamp((wf - 200.0) / 100.0);
        if rise > 0.0 {
            for &[e, r, z, f, a] in &FLANKS {
                let x = if e > 0.0 { e } else { wf + e };
                peaks.push([x, HZ - (HZ - r) * rise, z, f, a]);
            }
            // Past 3.2:1 the gaps between the range and its flanks fill too.
            let gaps = [[26.0, at(8.0)], [at(180.0), wf - 38.0]];
            for (side, [a, b]) in gaps.into_iter().enumerate() {
                let n = ((b - a - 40.0) / 48.0).floor().max(0.0) as usize;
                for i in 1..=n {
                    let j = (side * 16 + i) as f64;
                    peaks.push([
                        a + (b - a) * i as f64 / (n + 1) as f64 + (hash(j, 1.0) - 0.5) * 12.0,
                        16.0 + hash(j, 2.0) * 16.0,
                        130.0 + hash(j, 3.0) * 110.0,
                        0.95 + hash(j, 4.0) * 0.35,
                        (hash(j, 5.0) - 0.5) * 0.6,
                    ]);
                }
            }
        }
        let pines = PINES.map(|[x, r, s]| {
            let x = if x < 100.0 { x * squeeze } else { wf - (200.0 - x) * squeeze };
            [x, r, s]
        });
        Self {
            w,
            h,
            top,
            shore: SHORE + top,
            cx,
            // closed in, the range hides the low notch the sun clears upstream,
            // so it stands higher over the ridge
            sun: [at(151.0), 50.0 - 14.0 * clamp((200.0 - wf) / 100.0) + top as f64],
            peaks,
            pines,
            east: at(190.0),
            west: [at(40.0), at(150.0)],
            mist: at(170.0),
        }
    }
}

pub struct AlpineDawn {
    l: Layout,
    dots: Dots,
    depth: Vec<f32>,
    alt: Vec<f32>,
    sun: Vec<f32>,
    snow: Vec<f32>,
    up: Vec<f32>,
    rim: Vec<u8>,
    tree_top: Vec<f32>,
    src: Vec<f32>,
    fg: Vec<u8>,
    fg_shade: Vec<f32>,
    fg_rim: Vec<f32>,
    mist: Vec<f32>,
    cloud: Vec<f32>,
    hz: Vec<f32>,
    sky_r: Vec<f32>,
    sky_g: Vec<f32>,
    sky_b: Vec<f32>,
    glow_a: Vec<f32>,
    ar: Vec<f32>,
    ag: Vec<f32>,
    ab: Vec<f32>,
    fr: Vec<f32>,
    fgr: Vec<f32>,
    fb: Vec<f32>,
    floor: Vec<f32>,
    fade: Vec<f32>,
    wisp: Vec<f32>,
}

impl Piece for AlpineDawn {
    const NAME: &'static str = "alpine-dawn";
    const FPS: u32 = 15;
    const GROUND: u32 = hex("#090c18");
    #[rustfmt::skip]
    const PALETTE: &'static [u32] = &[
        hex("#0e1430"), hex("#151d40"), hex("#1d2752"), hex("#263365"), hex("#314179"), hex("#3e508c"), hex("#4f62a0"), hex("#6577b3"), hex("#8090c4"), hex("#9eaad3"), hex("#bec6e2"),
        hex("#4b3e6c"), hex("#6a5482"), hex("#8c6a92"), hex("#b0829c"), hex("#cf96a4"),
        hex("#e8a9a8"), hex("#f5bcaa"), hex("#ffd0b0"), hex("#ffe2c2"), hex("#fff1e0"), hex("#fdfaf6"),
        hex("#ffc887"), hex("#f7a965"), hex("#f2a08f"), hex("#e58a87"), hex("#f8b59d"), hex("#d97b7e"),
        hex("#7a4c4a"), hex("#a5654f"), hex("#523a4a"),
        hex("#1b2034"), hex("#262c45"), hex("#363c59"),
        hex("#0a1418"), hex("#0f1f24"), hex("#162a2f"), hex("#203a3c"),
        hex("#a3a7c6"), hex("#c6c3d8"), hex("#e0d4dc"),
        hex("#5a4f7e"), hex("#7b6c9c"), hex("#9a8cb6"), hex("#b8a8c8"), hex("#d8bccb"),
    ];

    fn new(cols: usize, rows: usize) -> Self {
        let lay = Layout::new(cols, rows);
        let (w, h, top, shore_r) = (lay.w, lay.h, lay.top as f64, lay.shore);
        let (sr, lr) = (shore_r, h - shore_r);
        let n = w * h;
        let terrain = Terrain::new(&lay.peaks, lay.cx);
        let l = {
            let v = [0.9, 0.3, 0.14];
            let n = js_hypot(&v);
            v.map(|c| c / n)
        };

        // --- the range, raymarched once --------------------------------------
        let mut depth = vec![0f32; sr * w];
        let mut alt = vec![0f32; sr * w];
        let mut sun = vec![0f32; sr * w];
        let mut snow = vec![0f32; sr * w];
        let mut up = vec![0f32; sr * w];
        for r in 0..sr {
            let v = ((HZ - (r as f64 + 0.5 - top)) / 100.0) * K;
            for x in 0..w {
                let u = ((x as f64 + 0.5 - lay.cx) / 100.0) * K;
                let z = terrain.march(u, v, CAM);
                let k = r * w + x;
                if z == 0.0 {
                    continue;
                }
                let (px, py) = (u * z, CAM + v * z);
                let e = 0.35;
                let hx = (terrain.height(px + e, z) - terrain.height(px - e, z)) / (2.0 * e);
                let hzz = (terrain.height(px, z + e) - terrain.height(px, z - e)) / (2.0 * e);
                let nl = js_hypot(&[hx, 1.0, hzz]);
                let (nx, ny, nz) = (-hx / nl, 1.0 / nl, -hzz / nl);
                let mut lit = (nx * l[0] + ny * l[1] + nz * l[2]).max(0.0);
                if lit > 0.0 {
                    // cast shadow: walk toward the sun
                    let mut s = 0.8;
                    while s < 160.0 {
                        let (qx, qy, qz) = (px + l[0] * s, py + l[1] * s + 0.15, z + l[2] * s);
                        if qz < SHORE_Z {
                            break;
                        }
                        if qy < terrain.height(qx, qz) {
                            lit = 0.0;
                            break;
                        }
                        s += 0.6 + s * 0.04;
                    }
                }
                depth[k] = z as f32;
                alt[k] = py as f32;
                sun[k] = lit as f32;
                up[k] = ny as f32;
                let grain = fbm(px * 0.4, py * 0.25, 2, 0.0);
                // rock shows through in couloirs running down the fall line
                let gully = ridged(px * 0.22 + z * 0.05, py * 0.045, 2);
                snow[k] = (smooth(0.36, 0.52, ny + 0.3 * (grain - 0.5))
                    * smooth(5.0, 11.0, py + 5.0 * grain)
                    * (1.0 - 0.75 * smooth(0.62, 0.85, gully))) as f32;
            }
        }

        // sunlit terrain with open sky directly above: the crest line
        let mut rim = vec![0u8; sr * w];
        for k in w..sr * w {
            rim[k] = u8::from(depth[k] != 0.0 && depth[k - w] == 0.0 && sun[k] > 0.0);
        }

        // --- the far shore's treeline ----------------------------------------
        let shore = shore_r as f64;
        let mut tree_top: Vec<f32> = (0..w)
            .map(|x| (shore - 0.6 - 1.2 * fbm(x as f64 * 0.06, 2.3, 2, 0.0)) as f32)
            .collect();
        let mut tx = -2.0;
        while tx < w as f64 + 2.0 {
            let tip = shore - 2.6 - hash(tx * 3.0, 8.0) * 4.0 - 1.5 * smooth(60.0, 0.0, tx);
            let slope = 1.1 + hash(tx * 5.0, 9.0) * 0.5;
            let mut x = (tx - 6.0).floor().max(0.0) as usize;
            while (x as f64) < (w as f64).min(tx + 6.0) {
                let v = tip + (x as f64 + 0.5 - tx).abs() * slope;
                tree_top[x] = f64::from(tree_top[x]).min(v) as f32;
                x += 1;
            }
            tx += 1.6 + hash(tx * 9.0, 7.0) * 2.2;
        }

        // --- the lake: where each cell's reflected ray lands in the picture above
        let mut src = vec![0f32; lr * w];
        for r in shore_r..h {
            let v = ((HZ - (r as f64 + 0.5 - top)) / 100.0) * K;
            for x in 0..w {
                let u = ((x as f64 + 0.5 - lay.cx) / 100.0) * K;
                let z = terrain.march(u, -v, -CAM);
                let vs = -v - if z != 0.0 { (2.0 * CAM) / z } else { 0.0 };
                let mut row = HZ - (vs * 100.0) / K - 0.5 + top;
                let mirror = (2 * shore_r - 1 - r) as f64; // the treeline stands on the shore
                if mirror >= f64::from(tree_top[x]) {
                    row = mirror;
                }
                src[(r - shore_r) * w + x] = row as f32;
            }
        }

        // --- the near pines and the bank they stand on -----------------------
        // the bank holds the bottom edge however tall the frame
        let lift = (h - 100) as f64;
        let mut fg = vec![0u8; n];
        let mut fg_shade = vec![0f32; n];
        let mut fg_rim = vec![0f32; n];
        for r in 0..h {
            for xi in 0..w {
                let k = r * w + xi;
                let (x, rf) = (xi as f64, r as f64);
                let y = rf + 0.5;
                let bank_l = 88.0 + lift + 14.0 * smooth(0.0, 52.0, x) + 2.0 * fbm(x * 0.2, 1.0, 2, 0.0);
                let bank_r = 90.0 + lift
                    + 12.0 * smooth(w as f64, w as f64 - 40.0, x)
                    + 2.0 * fbm(x * 0.2, 4.0, 2, 0.0);
                if y > bank_l || y > bank_r {
                    fg[k] = 1;
                    fg_shade[k] = (0.15 * hash(x, rf)) as f32;
                }
                for &[px, tip, s] in &lay.pines {
                    let d = y - top - tip;
                    if d < 0.0 {
                        continue;
                    }
                    let tier = 3.4 * s;
                    let f = d / tier - (d / tier).floor();
                    let hw = (0.4 + d * 0.2)
                        * (0.5 + 0.5 * f)
                        * (1.0 + 0.6 * (hash(rf, px.floor() * 7.0) - 0.5) * smooth(0.0, 8.0, d));
                    let dx = x + 0.5 - px;
                    if dx.abs() <= hw {
                        fg[k] = 2;
                        // the outline catches the dawn sky: a warm rim on the side
                        // facing the sun, a dim cool one on the other, the inside
                        // stays black
                        let edge = hw - dx.abs() < 1.0;
                        let sunward = if px < lay.sun[0] { dx > 0.0 } else { dx < 0.0 };
                        fg_shade[k] = if edge {
                            if sunward {
                                1.0
                            } else {
                                0.5
                            }
                        } else {
                            (0.3 * hash(x * 3.0, rf * 5.0)) as f32
                        };
                        // the rim is light from the sky behind, so it fades out
                        // below the shore
                        fg_rim[k] = if edge {
                            (smooth(70.0, 44.0, y - top) * (0.75 + 0.25 * hash(x, rf * 7.0))) as f32
                        } else {
                            0.0
                        };
                    }
                }
            }
        }

        // --- mist, a wrapping sheet that drifts along the valley -------------
        let mw = MW as f64;
        let mut mist = vec![0f32; MW * MR];
        for r in 0..MR {
            for x in 0..MW {
                let (xf, y) = (x as f64, (r + M0) as f64);
                let q = fbm(xf * 0.0125, y * 0.1, 2, mw * 0.0125);
                mist[r * MW + x] = fbm(xf * 0.025 + q * 1.4, y * 0.22 + q, 4, mw * 0.025) as f32;
            }
        }

        // --- thin high cloud, streaked and lit from below by the sun ---------
        let mut cloud = vec![0f32; CW * CR];
        for r in 0..CR {
            for x in 0..CW {
                let (xf, y) = (x as f64, r as f64 + 0.5);
                let q = fbm(xf * 0.0125, y * 0.12, 2, 8.0);
                let c = fbm(xf * 0.025 + q * 2.0, y * 0.2 + q * 0.8, 4, 16.0);
                cloud[r * CW + x] = (smooth(0.52, 0.7, c - 0.06 * (y - 18.0).abs() / 10.0)
                    * smooth(5.0, 13.0, y)
                    * smooth(33.0, 24.0, y)) as f32;
            }
        }

        // a faint large-scale unevenness, so the open sky and the deep water
        // are never one flat halftone screen
        let hz: Vec<f32> = (0..n)
            .map(|k| fbm((k % w) as f64 * 0.03, (k / w) as f64 * 0.06, 3, 0.0) as f32)
            .collect();

        // the sky's colour at a point, for the sky itself and as haze on the peaks
        let mut sky_r = vec![0f32; sr * w];
        let mut sky_g = vec![0f32; sr * w];
        let mut sky_b = vec![0f32; sr * w];
        let mut glow_a = vec![0f32; sr * w];
        for r in 0..sr {
            for xi in 0..w {
                let k = r * w + xi;
                let x = xi as f64;
                let y = r as f64 + 0.5;
                let v = clamp((y - top) / 52.0);
                let east = smooth(20.0, lay.east, x);
                let (dx, dy) = (x + 0.5 - lay.sun[0], (y - lay.sun[1]) * 2.2);
                let ds = (dx * dx + dy * dy).sqrt();
                let low = v.powf(1.9);
                // indigo overhead, a soft unevenness in it, then a pale lilac
                // and rose band behind the range, lighter than the mountains'
                // shadowed flanks
                let veil = (f64::from(hz[k]) - 0.5) * 0.1 * (1.0 - v);
                sky_r[k] = (0.03 + veil + low * (0.56 + 0.2 * east)) as f32;
                sky_g[k] = (0.04 + veil + low * (0.48 + 0.02 * east)) as f32;
                sky_b[k] = (0.13 + veil * 1.6 + low * (0.62 - 0.12 * east)) as f32;
                // the sun's glow, kept apart so it can breathe
                glow_a[k] = ((-ds / 6.0).exp() * 0.65
                    + (-ds / 15.0).exp() * 0.2
                    + (-ds / 50.0).exp() * 0.1) as f32;
            }
        }

        Self {
            l: lay,
            dots: Dots::new(Self::PALETTE),
            depth,
            alt,
            sun,
            snow,
            up,
            rim,
            tree_top,
            src,
            fg,
            fg_shade,
            fg_rim,
            mist,
            cloud,
            hz,
            sky_r,
            sky_g,
            sky_b,
            glow_a,
            ar: vec![0.0; sr * w],
            ag: vec![0.0; sr * w],
            ab: vec![0.0; sr * w],
            fr: vec![0.0; n],
            fgr: vec![0.0; n],
            fb: vec![0.0; n],
            floor: vec![0.0; n],
            fade: vec![1.0; n],
            wisp: vec![0.0; w],
        }
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        let warm = 0.75 - 0.5 * (-t / 60.0).exp(); // rose first light warming toward gold
        let line = 6.0 + 4.0 * (-t / 70.0).exp(); // the sunlit line creeps down the slopes
        let drift = t * 1.1;
        let pulse = 1.0 + 0.06 * ((t / 8.0) * PI * 2.0).sin(); // the sun's glow breathes
        for (x, w) in self.wisp.iter_mut().enumerate() {
            *w = noise((x as f64 + drift * 0.6) * 0.06, 3.7, 0.0) as f32;
        }
        let l = &self.l;
        let (w, h, top) = (l.w, l.h, l.top);
        let (shore_r, topf) = (l.shore, top as f64);
        let shore = shore_r as f64;
        let hf = h as f64;

        // the warm light, from rose toward gold
        let (lr, lg, lb) = (1.0, mix(0.6, 0.8, warm), mix(0.55, 0.4, warm));

        for r in 0..shore_r {
            let rf = r as f64;
            let y = rf + 0.5;
            for xi in 0..w {
                let k = r * w + xi;
                let x = xi as f64;
                let gl = f64::from(self.glow_a[k]) * pulse;
                let gg = 0.62 + 0.28 * smooth(0.15, 0.7, gl); // gold at the core, rose further out
                let (sky_r, sky_g, sky_b) = (
                    f64::from(self.sky_r[k]),
                    f64::from(self.sky_g[k]),
                    f64::from(self.sky_b[k]),
                );
                let mut cr = sky_r + gl;
                let mut cg = sky_g + gl * gg;
                let mut cb = sky_b + gl * (gg - 0.22);
                let mut fl = 0.21;
                let z = f64::from(self.depth[k]);
                if z != 0.0 {
                    let alt = f64::from(self.alt[k]);
                    let sn = f64::from(self.snow[k]);
                    let lit = f64::from(self.sun[k]) * smooth(line, line + 7.0, alt);
                    let amb = (0.55 + 0.45 * f64::from(self.up[k]))
                        * (0.55 + 0.5 * smooth(4.0, 34.0, alt));
                    // snow: deep blue in shadow, rose to gold in the sun, ending sharply
                    let sl = smooth(0.08, 0.24, lit);
                    // full on faces and summits glow gold, glancing light stays rose
                    let gold = clamp(0.6 * smooth(0.25, 0.8, lit) + 0.5 * smooth(14.0, 36.0, alt));
                    let br = 0.78 + 0.3 * lit;
                    let sr = mix(0.13 * amb, lr * br, sl);
                    let sg = mix(0.17 * amb, mix(lg - 0.12, lg + 0.14, gold) * br, sl);
                    let sb = mix(0.36 * amb, mix(lb + 0.02, lb + 0.12, gold) * br, sl);
                    // rock: slate in shadow, warm umber in the sun
                    let (rr, rg, rb) = (mix(0.06, 0.4, sl), mix(0.07, 0.2, sl), mix(0.14, 0.2, sl));
                    cr = mix(rr, sr, sn);
                    cg = mix(rg, sg, sn);
                    cb = mix(rb, sb, sn);
                    // forested foothills
                    let wood = smooth(9.0, 4.0, alt) * smooth(110.0, 75.0, z);
                    cr = mix(cr, 0.07, wood);
                    cg = mix(cg, 0.09, wood);
                    cb = mix(cb, 0.18, wood);
                    // distance hazes toward the sky behind, and haze settles in
                    // the far valleys so each ridge stands clear of the one
                    // behind it
                    let fog = smooth(16.0, 3.0, alt) * smooth(70.0, 150.0, z) * 0.6;
                    let haze = fog.max(clamp(1.0 - (-(z - SHORE_Z) / 260.0).exp()) * 0.45);
                    cr = mix(cr, sky_r + gl, haze);
                    cg = mix(cg, sky_g + gl * gg, haze);
                    cb = mix(cb, sky_b + gl * (gg - 0.22), haze);
                    // the first light catches the crest itself in a bright line
                    if self.rim[k] != 0 && sl > 0.3 {
                        let a = f64::from(self.rim[k]) * sl;
                        cr = mix(cr, 1.0, a);
                        cg = mix(cg, mix(0.89, 0.95, warm), a);
                        cb = mix(cb, mix(0.76, 0.88, warm), a);
                    }
                    // the shadowed range still carries a dim blue screen; the
                    // wooded foothills in front of it drop away to near black
                    fl = mix(mix(0.42, 0.3, wood), 0.04, sl);
                } else {
                    // a few stars still out in the west
                    if y - topf < 34.0 && hash(x, rf * 3.0 + 11.0) > 0.985 {
                        let tw =
                            0.6 + 0.4 * (t * (1.5 + hash(x, rf) * 3.0) + hash(rf, x) * 6.28).sin();
                        let s = tw * smooth(l.west[1], l.west[0], x) * smooth(34.0, 6.0, y - topf) * 0.75;
                        cr = cr.max(s * 0.9);
                        cg = cg.max(s * 0.92);
                        cb = cb.max(s);
                    }
                    // high cloud, rose-gold toward the sun and mauve away from it
                    if (top..top + CR).contains(&r) {
                        let r = r - top;
                        let sx = x + t * 0.8;
                        let ix = sx.floor();
                        let fx = sx - ix;
                        let ix = ix as usize;
                        let c0 = f64::from(self.cloud[r * CW + ix % CW]);
                        let c1 = f64::from(self.cloud[r * CW + (ix + 1) % CW]);
                        let c = (c0 + (c1 - c0) * fx) * (0.35 + 0.65 * smooth(l.west[0], l.west[1], x));
                        if c > 0.01 {
                            let g =
                                (-js_hypot(&[x + 0.5 - l.sun[0], (y - l.sun[1]) * 1.6]) / 55.0).exp();
                            let b = clamp(0.25 + 0.9 * g);
                            let (kr, kg, kb) = (
                                mix(0.32, 1.0, b),
                                mix(0.24, mix(0.62, 0.74, warm), b),
                                mix(0.4, 0.5, b),
                            );
                            cr = mix(cr, kr, c * 0.75);
                            cg = mix(cg, kg, c * 0.75);
                            cb = mix(cb, kb, c * 0.75);
                        }
                    }
                    // the sun, just clearing the ridge
                    let (dx, dy) = (x + 0.5 - l.sun[0], y - l.sun[1]);
                    let ds = (dx * dx + dy * dy).sqrt();
                    if ds < 4.5 {
                        let a = smooth(4.5, 3.3, ds);
                        cr = mix(cr, 1.0, a);
                        cg = mix(cg, 0.96, a);
                        cb = mix(cb, 0.86, a);
                    }
                }
                // mist pooled in the valley behind the shore, lit rose toward the sun
                let e = smooth(20.0, l.mist, x) * (0.6 + 0.4 * warm);
                let near = (-(x + 0.5 - l.sun[0]).abs() / 22.0).exp() * 0.3;
                let (mr, mg, mb) = (
                    mix(0.48, 0.9, e) + near,
                    mix(0.46, 0.7, e) + near * 0.75,
                    mix(0.7, 0.7, e) + near * 0.5,
                );
                if r >= M0 + top {
                    let m = f64::from(self.mist[(r - M0 - top) * MW + (x + drift).floor() as usize % MW]);
                    // a ragged top edge: the sheet heaves in long swells and small tufts
                    let edge = 5.0 * (m - 0.5) + 4.0 * (f64::from(self.wisp[xi]) - 0.5);
                    let band = smooth((M0 + 14) as f64, (SHORE - 3) as f64, y - topf + edge);
                    let a = (0.3 + 0.7 * smooth(0.32, 0.64, m)) * band * 0.6;
                    cr = mix(cr, mr, a);
                    cg = mix(cg, mg, a);
                    cb = mix(cb, mb, a);
                    if a > 0.05 {
                        fl = f64::max(fl, 0.2);
                    }
                }
                let tt = f64::from(self.tree_top[xi]);
                if y >= tt {
                    // the far shore's pines, dark against the mist
                    let s = 0.4 + 0.6 * hash(x * 7.0, rf * 3.0);
                    cr = 0.04 + 0.03 * s;
                    cg = 0.06 + 0.04 * s;
                    cb = 0.1 + 0.05 * s;
                    fl = 0.0;
                    // and low wisps drifting across their feet
                    let mi = (x * 0.7 + drift * 1.9 + 211.0).floor() as usize % MW;
                    let m = f64::from(self.mist[(r - M0 - top) * MW + mi]);
                    let a = smooth(0.45, 0.72, m) * smooth(tt + 1.0, shore, y) * 0.6;
                    cr = mix(cr, mr, a);
                    cg = mix(cg, mg, a);
                    cb = mix(cb, mb, a);
                }
                (self.ar[k], self.ag[k], self.ab[k]) = (cr as f32, cg as f32, cb as f32);
                (self.fr[k], self.fgr[k], self.fb[k]) = (cr as f32, cg as f32, cb as f32);
                self.floor[k] = fl as f32;
            }
        }

        // the lake: the picture above, shaken a little by slow ripples
        for r in shore_r..h {
            let y = r as f64 + 0.5;
            let d = (y - shore) / (h - shore_r) as f64;
            for xi in 0..w {
                let k = r * w + xi;
                let x = xi as f64;
                let w1 = noise(x * 0.045 + t * 0.06, y * 0.5 - t * 0.35, 0.0);
                let w2 = noise(x * 0.12 - t * 0.1, y * 1.1 - t * 0.7, 0.0);
                let sway = (w1 - 0.5) * (0.4 + 1.4 * d) + (w2 - 0.5) * 0.5;
                let sx = js_round(x + sway).clamp(0.0, (w - 1) as f64) as usize;
                let sr = js_round(f64::from(self.src[(r - shore_r) * w + xi]) + (w2 - 0.5) * 0.6 * d)
                    .clamp(0.0, (shore_r - 1) as f64) as usize;
                let sk = sr * w + sx;
                let refl = 0.68 - 0.32 * d;
                let w3 = noise(x * 0.03 + t * 0.04, y * 1.9 - t * 0.45, 0.0);
                let lift = 1.0 + (w3 - 0.5) * (0.4 + 0.5 * d); // long, faint ripple lines
                                                               // the water gives back a little less colour than it was sent
                let (ar, ag, ab) = (
                    f64::from(self.ar[sk]),
                    f64::from(self.ag[sk]),
                    f64::from(self.ab[sk]),
                );
                let grey = (ar + ag + ab) / 3.0;
                let deep = (f64::from(self.hz[k]) - 0.5) * 0.1 * d; // slow unevenness in the dark water
                let mut cr = 0.02 + deep + mix(grey, ar, 0.75) * refl * lift;
                let mut cg = 0.035 + deep + mix(grey, ag, 0.75) * refl * lift;
                let mut cb = 0.07 + deep * 1.6 + mix(grey, ab, 0.75) * refl * lift;
                // the sun's road
                let road_w = 1.5 + (y - shore) * 0.45;
                let road = (-((x + 0.5 - l.sun[0]) / road_w).powi(2)).exp();
                let glint = smooth(0.55, 0.85, w2) * road * (0.5 + 0.5 * warm);
                cr += glint;
                cg += glint * 0.8;
                cb += glint * 0.6;
                (self.fr[k], self.fgr[k], self.fb[k]) = (cr as f32, cg as f32, cb as f32);
                self.floor[k] = 0.26;
                self.fade[k] = smooth(hf + 2.0, hf - 22.0, y) as f32;
                if r == shore_r {
                    // a dark seam where the shore meets the water
                    (self.fr[k], self.fgr[k], self.fb[k]) = (0.06, 0.12, 0.14);
                    self.floor[k] = 0.0;
                }
            }
        }

        // the near pines and the bank, black against it all, rimmed on the sun side
        for k in 0..w * h {
            if self.fg[k] == 0 {
                continue;
            }
            let s = f64::from(self.fg_shade[k]);
            self.fr[k] = (0.02 + 0.05 * s) as f32;
            self.fgr[k] = (0.04 + 0.06 * s) as f32;
            self.fb[k] = (0.05 + 0.06 * s) as f32;
            self.floor[k] = 0.0;
            let e = f64::from(self.fg_rim[k]);
            let (fr, fgr, fb) = (
                f64::from(self.fr[k]),
                f64::from(self.fgr[k]),
                f64::from(self.fb[k]),
            );
            if e > 0.0 && s == 1.0 {
                // warm on the side facing the sun
                self.fr[k] = mix(fr, 0.62, e) as f32;
                self.fgr[k] = mix(fgr, 0.4, e) as f32;
                self.fb[k] = mix(fb, 0.38, e) as f32;
                self.floor[k] = (0.12 * e) as f32;
            } else if e > 0.0 && s == 0.5 {
                // cool lilac from the sky on the other
                self.fr[k] = mix(fr, 0.2, e) as f32;
                self.fgr[k] = mix(fgr, 0.22, e) as f32;
                self.fb[k] = mix(fb, 0.36, e) as f32;
                self.floor[k] = (0.22 * e) as f32;
            }
            self.fade[k] = 1.0;
        }

        for r in 0..h {
            for x in 0..w {
                let k = r * w + x;
                let (cr, cg, cb) = (
                    f64::from(self.fr[k]),
                    f64::from(self.fgr[k]),
                    f64::from(self.fb[k]),
                );
                let fl = f64::from(self.floor[k]);
                let peak = cr.max(cg).max(cb).max(1e-4);
                let level = clamp(fl + (1.0 - fl) * peak.powf(1.1)) * f64::from(self.fade[k]);
                let step = Dots::step(level, bayer(r, x));
                let want = Dots::want(step, level, 0.06);
                // dim cells keep some of their darkness in the colour too, so
                // the shadows sit back in deep blues rather than as a bright
                // fine screen
                let s = ((0.3 + 0.7 * want) * mix(0.5, 1.0, smooth(0.08, 0.5, peak))) / peak;
                out[k] = self.dots.dot(step, [cr, cg, cb], s);
            }
        }
    }
}
