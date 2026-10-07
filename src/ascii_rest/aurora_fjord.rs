//! aurora fjord: curtains of aurora ripple over a fjord between snowy
//! mountains. The still water holds a broken shimmer of them, and a red cabin
//! on the far shore keeps its lamps lit.
//!
//! The land is built once; each frame shades the sky, then mirrors it into the
//! water. Upstream's `Float32Array`s stay `f32` here, as in `night_coast`.

use std::f64::consts::PI;

use super::halftone::{bayer, Dots};
use super::math::{clamp, fbm, hash, js_round, mix, noise, sign_or_one, smooth};
use super::{hex, Piece};
use crate::grid::Cell;

const W: usize = 200;
const H: usize = 100;
const WL: usize = 62; // the waterline

const AIR: u8 = 0;
const NEAR: u8 = 1;
const FAR: u8 = 2;
const SHORE: u8 = 3;
const WALL: u8 = 4;
const ROOF: u8 = 5;
const PANE: u8 = 6;
const TREE: u8 = 7;
const DOOR: u8 = 8;
const CAB: [usize; 2] = [142, 161]; // the cabin's walls, x from and to
const PANES: [[usize; 2]; 2] = [[145, 148], [156, 159]];
const DOOR_X: [usize; 2] = [150, 152];
const LAMPS: [[f64; 2]; 2] = [[147.0, 1.0], [158.0, 0.8]]; // pane centres and their strength
const RAYS: usize = 4 * W;

// Ranges as tent peaks [x, height, slope], roughened.
const LEFT: [[f64; 3]; 4] = [
    [25.0, 38.0, 1.3],
    [6.0, 29.0, 0.9],
    [50.0, 25.0, 1.0],
    [72.0, 13.0, 0.6],
];
const RIGHT: [[f64; 3]; 4] = [
    [172.0, 30.0, 1.15],
    [194.0, 24.0, 0.85],
    [151.0, 16.0, 1.0],
    [212.0, 22.0, 0.6],
];
const DISTANT: [[f64; 3]; 4] = [
    [100.0, 10.0, 0.5],
    [119.0, 12.0, 0.55],
    [86.0, 7.0, 0.45],
    [134.0, 8.0, 0.5],
];

/// A range's height at `x` and the peak it belongs to.
fn range(x: f64, peaks: &[[f64; 3]], seed: f64, rough: f64) -> (f64, f64) {
    let (mut m, mut px) = (-99.0, 0.0);
    for &[cx, h, s] in peaks {
        let v = h - (x - cx).abs() * s;
        if v > m {
            m = v;
            px = cx;
        }
    }
    let j = rough * (fbm(x * 0.09, seed, 3, 0.0) - 0.5)
        + rough * 0.45 * (noise(x * 0.45, seed + 5.0, 0.0) - 0.5);
    (m + j * (m.max(0.0) / 6.0).min(1.0), px)
}

fn shore_top(x: f64) -> f64 {
    WL as f64
        - 2.2 * smooth(134.0, 140.0, x) * smooth(178.0, 168.0, x)
        - 0.6 * noise(x * 0.3, 2.0, 0.0)
}


/// The land layers, shared by the builders below.
struct Land {
    mat: Vec<u8>,
    sr: Vec<f32>,
    sg: Vec<f32>,
    sb: Vec<f32>,
    rec: Vec<f32>,
    rim: Vec<f32>,
}

impl Land {
    /// A spruce: tiered, lit on the aurora's side. `guard` keeps it off cells
    /// it must stand behind.
    fn spruce(&mut self, tx: f64, th: f64, tw: f64, tb: f64, guard: Option<fn(u8) -> bool>) {
        let r0 = (tb - th).floor().max(0.0) as usize;
        for r in r0..WL {
            let y = r as f64 + 0.5;
            let dy = y - (tb - th);
            if dy < 0.0 || y >= tb + 0.5 {
                continue;
            }
            let tier = (dy + th * 0.3) / 1.8;
            let w = (dy / th) * tw * (0.7 + 0.45 * (tier - tier.floor())) + 0.35;
            let x_hi = ((W - 1) as f64).min(tx + tw + 1.0);
            let mut x = (tx - tw - 1.0).floor().max(0.0) as usize;
            while x as f64 <= x_hi {
                let k = r * W + x;
                let ex = x as f64 + 0.5 - tx;
                if ex.abs() > w || guard.is_some_and(|g| g(self.mat[k])) {
                    x += 1;
                    continue;
                }
                self.mat[k] = TREE;
                let h = hash(x as f64 * 13.0 + r as f64, 5.0);
                let s = if ex < 0.0 { 0.7 } else { 0.3 }; // the aurora is up and left
                self.sr[k] = (0.02 + 0.03 * s * h) as f32;
                self.sg[k] = (0.04 + 0.05 * s) as f32;
                self.sb[k] = (0.055 + 0.045 * s) as f32;
                self.rec[k] = 0.05;
                let side = if ex < 0.0 && ex < -w + 1.0 {
                    0.45 * smooth(th, 0.0, dy)
                } else {
                    0.0
                };
                self.rim[k] = smooth(1.6, 0.2, dy).max(side) as f32;
                x += 1;
            }
        }
    }
}

pub struct AuroraFjord {
    dots: Dots,
    mat: Vec<u8>,
    sr: Vec<f32>,
    sg: Vec<f32>,
    sb: Vec<f32>,
    /// How much aurora light a cell picks up.
    rec: Vec<f32>,
    /// Aurora light caught on ridgelines and treetops.
    rim: Vec<f32>,
    lamp_land: Vec<f32>,
    haze: Vec<f32>,
    star: Vec<f32>,
    gap: Vec<u8>,
    eave: f64,
    base_a: Vec<f32>,
    tall_a: Vec<f32>,
    env_a: Vec<f32>,
    base_b: Vec<f32>,
    env_b: Vec<f32>,
    rays_a: Vec<f32>,
    rays_b: Vec<f32>,
    r: Vec<f32>,
    g: Vec<f32>,
    b: Vec<f32>,
    /// The aurora's light falling on the land below.
    light_x: Vec<f32>,
}

fn ray(arr: &[f32], u: f64) -> f64 {
    let s = u * 4.0;
    let i = s.floor();
    let f = s - i;
    let i = (i as i64).rem_euclid(RAYS as i64) as usize;
    let (a, b) = (f64::from(arr[i]), f64::from(arr[i + 1]));
    a + (b - a) * f
}

impl Piece for AuroraFjord {
    const NAME: &'static str = "aurora-fjord";
    const COLS: usize = W;
    const ROWS: usize = H;
    const FPS: u32 = 15;
    const CELL: usize = 1;
    const GROUND: u32 = hex("#05080f");
    #[rustfmt::skip]
    const PALETTE: &'static [u32] = &[
        hex("#0b1322"), hex("#101b30"), hex("#16243f"), hex("#1e3050"), hex("#2a3f63"),
        hex("#0e3a33"), hex("#11573f"), hex("#167a4c"), hex("#22a05c"), hex("#3ccb73"), hex("#7cf0a0"), hex("#c8ffdc"),
        hex("#0f5f5c"), hex("#16877f"), hex("#2cb5a6"), hex("#7fe6d6"),
        hex("#2a1f52"), hex("#432b78"), hex("#6a3c9f"), hex("#9558c6"), hex("#c48ae4"), hex("#363a72"), hex("#4f5596"), hex("#1b4f63"),
        hex("#1a2236"), hex("#283350"), hex("#3b4a6e"), hex("#566a92"), hex("#7d91b8"), hex("#a9bad9"), hex("#d6e0f2"), hex("#9fc9cf"),
        hex("#0f1a1c"), hex("#173128"), hex("#24493a"),
        hex("#4a1517"), hex("#7c2420"), hex("#b23a2a"), hex("#d65aa8"),
        hex("#ffd27c"), hex("#ffb04a"), hex("#fff2c4"),
        hex("#eef3ff"),
    ];

    fn new() -> Self {
        let n = W * H;
        let wl = WL as f64;
        let mut land = Land {
            mat: vec![AIR; n],
            sr: vec![0f32; n],
            sg: vec![0f32; n],
            sb: vec![0f32; n],
            rec: vec![0f32; n],
            rim: vec![0f32; n],
        };

        // --- the land, built once --------------------------------------------
        let mut near_top = vec![0f32; W];
        let mut far_top = vec![0f32; W];
        let mut peak_x = vec![0f32; W];
        for x in 0..W {
            let xc = x as f64 + 0.5;
            let (hl, pl) = range(xc, &LEFT, 3.1, 4.0);
            let (hr, pr) = range(xc, &RIGHT, 7.7, 4.0);
            let (hd, _) = range(xc, &DISTANT, 11.3, 2.2);
            near_top[x] = (wl - hl.max(hr).max(0.0)) as f32;
            peak_x[x] = (if hl > hr { pl } else { pr }) as f32;
            far_top[x] = (wl - hd.max(0.0)) as f32;
        }
        let cab_base = wl - 2.2;

        for r in 0..WL {
            for xi in 0..W {
                let k = r * W + xi;
                let (x, rf) = (xi as f64, r as f64);
                let y = rf + 0.5;
                let nt = f64::from(near_top[xi]);
                let ft = f64::from(far_top[xi]);
                if y >= nt {
                    land.mat[k] = NEAR;
                    let px = f64::from(peak_x[xi]);
                    let side = x + 0.5 - px;
                    let depth = y - nt;
                    let height = wl - nt;
                    // Faces turned toward the fjord catch the aurora; the ridge
                    // between the faces wanders as it comes down from the peak.
                    let ridge = px + (depth + 1.0) * 0.6 * (noise(y * 0.1, px, 0.0) - 0.5);
                    let inward = if px < 100.0 { 1.0 } else { -1.0 };
                    let face = smooth(-4.0, 4.0, (x + 0.5 - ridge) * inward);
                    // ribs and couloirs running down the fall line, each with a lit side
                    let u = x + depth * 0.45 * sign_or_one(side);
                    let rib = |v: f64| fbm(v * 0.13, px * 0.37, 3, 0.0);
                    let grad = (rib(u + 1.0) - rib(u - 1.0)) * 7.0 * inward;
                    // the snow reaches further down the ribs than the couloirs, in fingers
                    let reach = 3.0 + height * (0.62 + 0.42 * rib(u + 40.0));
                    let streak = smooth(0.56, 0.64, fbm(u * 0.32, y * 0.035 + px, 3, 0.0))
                        * smooth(1.0, 5.0, depth);
                    // a stand of spruce climbing the slope behind the cabin, jagged on top
                    let cx = x + 0.5 - 152.0;
                    let wood = wl - 18.5
                        + 9.0 * (cx / 19.0).powi(2)
                        + 1.5 * (noise(x * 0.7, 9.0, 0.0) - 0.5)
                        - 2.6 * hash(x, 77.0) * if xi & 1 != 0 { 1.0 } else { 0.3 };
                    let snow = smooth(reach + 1.6, reach - 1.6, depth)
                        * (1.0 - smooth(wl - 3.0, wl - 0.5, y))
                        * (1.0 - 0.5 * streak);
                    let lit = clamp(0.35 + 0.55 * face + grad * 0.5);
                    // blue-grey rock below the snow, a little lighter on the lit faces
                    let rv = 0.75 + 0.5 * fbm(x * 0.4, y * 0.4, 2, 0.0);
                    let (rr, rg, rb) = (
                        (0.05 + 0.03 * lit) * rv,
                        (0.065 + 0.035 * lit) * rv,
                        (0.11 + 0.05 * lit) * rv,
                    );
                    let s = 0.34 + 0.66 * lit.powf(1.5);
                    land.sr[k] = mix(rr, 0.62 * s + 0.04, snow) as f32;
                    land.sg[k] = mix(rg, 0.7 * s + 0.05, snow) as f32;
                    land.sb[k] = mix(rb, 0.86 * s + 0.09, snow) as f32;
                    land.rec[k] = (snow * (0.25 + 0.75 * lit) + 0.06) as f32;
                    land.rim[k] = (smooth(2.2, 0.3, depth) * 0.5) as f32;
                    if y > wood
                        && cx > -14.0 - 2.0 * hash(rf, 3.0)
                        && cx < 13.0 + 2.0 * hash(rf, 4.0)
                    {
                        land.mat[k] = TREE;
                        let h = hash(x * 13.0 + rf, 5.0);
                        land.sr[k] = (0.02 + 0.03 * h) as f32;
                        land.sg[k] = (0.045 + 0.045 * h) as f32;
                        land.sb[k] = (0.06 + 0.04 * h) as f32;
                        land.rec[k] = 0.05;
                        land.rim[k] = (smooth(wood + 1.6, wood + 0.2, y) * (0.7 + 0.3 * h)) as f32;
                    }
                } else if y >= ft {
                    land.mat[k] = FAR;
                    let depth = y - ft;
                    let snow = smooth(
                        0.5,
                        0.62,
                        fbm(x * 0.18, y * 0.25, 3, 0.0) * 0.6 + (1.0 - depth / 7.0) * 0.55,
                    );
                    let lit = 0.5 + 0.3 * (noise(x * 0.25, y * 0.1, 0.0) - 0.5);
                    land.sr[k] = mix(0.075, 0.22 * lit + 0.12, snow) as f32;
                    land.sg[k] = mix(0.1, 0.27 * lit + 0.15, snow) as f32;
                    land.sb[k] = mix(0.17, 0.36 * lit + 0.24, snow) as f32;
                    land.rec[k] = (0.2 + 0.3 * snow) as f32;
                }
                // the shelf the cabin stands on, snowed over
                if xi > 132 && xi < 180 && y >= shore_top(x) {
                    land.mat[k] = SHORE;
                    let f = 0.6 + 0.3 * fbm(x * 0.3, y * 0.5, 2, 0.0);
                    land.sr[k] = (0.26 * f) as f32;
                    land.sg[k] = (0.3 * f) as f32;
                    land.sb[k] = (0.42 * f) as f32;
                    land.rec[k] = 0.4;
                    land.rim[k] = 0.0;
                }
            }
        }

        // a spruce treeline along the foot of both ranges, open where the fjord runs in
        fn solid(m: u8) -> bool {
            m == WALL || m == ROOF || m == PANE || m == DOOR || m == SHORE
        }
        let mut x = -1.0f64;
        while x < W as f64 + 2.0 {
            let h = hash((x * 7.0).floor(), 21.0);
            let open = smooth(96.0, 80.0, x) + smooth(122.0, 136.0, x);
            let on_shelf = x > 132.0 && x < 180.0;
            if open > 0.05 && !(x > 136.0 && x < 166.0) {
                let th = (3.2
                    + hash((x * 3.0).floor(), 5.0) * 3.0
                    + if hash((x * 5.0).floor(), 8.0) > 0.7 {
                        2.4
                    } else {
                        0.0
                    })
                    * open.min(1.0)
                    * if on_shelf { 0.7 } else { 1.0 };
                if th > 1.5 {
                    land.spruce(
                        x + hash(x.floor(), 2.0),
                        th,
                        1.0 + hash(x.floor(), 4.0) * 0.6,
                        if on_shelf {
                            shore_top(x) - 0.4
                        } else {
                            wl + 0.3
                        },
                        if on_shelf { Some(solid) } else { None },
                    );
                }
            }
            x += 2.0 + 2.2 * h;
        }

        // The cabin: red boards, a snowed roof, two warm panes, a door, a chimney.
        let eave = cab_base - 7.0;
        let roof_top = eave - 8.0;
        let (chim, chim_top) = (CAB[1] - 5, eave - 8.5);
        let (c0, c1) = (CAB[0] as f64, CAB[1] as f64);
        let cx = (c0 + c1 + 1.0) / 2.0;
        for r in 0..WL {
            for xi in CAB[0] - 3..=CAB[1] + 3 {
                let k = r * W + xi;
                let x = xi as f64;
                let y = r as f64 + 0.5;
                if xi >= CAB[0] && xi <= CAB[1] && y >= eave && y < cab_base + 0.5 {
                    land.mat[k] = WALL;
                    let boards = if r & 1 != 0 { 0.82 } else { 1.0 };
                    let s = (0.5 + 0.5 * ((c1 - x) / (c1 - c0))) * boards;
                    land.sr[k] = (0.7 * s) as f32;
                    land.sg[k] = (0.14 * s) as f32;
                    land.sb[k] = (0.11 * s) as f32;
                    land.rim[k] = 0.0;
                    for &[a, b] in &PANES {
                        if xi >= a && xi <= b && y >= eave + 1.5 && y < cab_base - 2.0 {
                            land.mat[k] = PANE;
                        }
                    }
                    if xi >= DOOR_X[0] && xi <= DOOR_X[1] && y >= eave + 1.5 {
                        land.mat[k] = DOOR;
                        (land.sr[k], land.sg[k], land.sb[k]) = (0.16, 0.06, 0.05);
                    }
                }
                let half = 12.5 - (eave - y) * 1.45;
                if y >= roof_top && y < eave && (x + 0.5 - cx).abs() <= half {
                    land.mat[k] = ROOF;
                    let snowy = y < eave - 1.0;
                    let s = 0.78 + 0.22 * ((cx - x - 0.5) / 12.0);
                    if snowy {
                        land.sr[k] = (0.78 * s) as f32;
                        land.sg[k] = (0.84 * s) as f32;
                        land.sb[k] = (0.96 * s) as f32;
                    } else {
                        (land.sr[k], land.sg[k], land.sb[k]) = (0.12, 0.05, 0.06);
                    }
                    land.rec[k] = if snowy { 0.5 } else { 0.0 };
                    land.rim[k] = 0.0;
                }
                if xi >= chim && xi <= chim + 1 && y >= chim_top && y < eave - 4.0 {
                    land.mat[k] = WALL;
                    let s = if xi == chim { 1.0 } else { 0.55 };
                    land.sr[k] = (0.2 * s) as f32;
                    land.sg[k] = (0.21 * s) as f32;
                    land.sb[k] = (0.27 * s) as f32;
                    land.rim[k] = 0.0;
                }
            }
        }

        // a few spruce on the shelf, beside the cabin
        fn cabin(m: u8) -> bool {
            m == WALL || m == ROOF || m == PANE || m == DOOR
        }
        for [tx, th] in [
            [134.0, 7.0],
            [137.8, 10.0],
            [166.5, 9.0],
            [170.0, 6.0],
            [174.5, 8.0],
        ] {
            land.spruce(tx, th, 2.2, shore_top(tx) + 0.5, Some(cabin));
        }

        let Land {
            mat,
            sr,
            sg,
            sb,
            rec,
            rim,
        } = land;

        // the warm light the panes throw on the snow and the air around them
        let mut lamp_land = vec![0f32; WL * W];
        for r in 0..WL {
            for x in 0..W {
                let k = r * W + x;
                let m = mat[k];
                if cabin(m) {
                    continue;
                }
                let mut g = 0.0;
                for &[lx, s] in &LAMPS {
                    let (wx, wy) = (x as f64 + 0.5 - lx, r as f64 + 0.5 - (cab_base - 2.5));
                    let d = (wx * wx * 0.6 + wy * wy * 2.2).sqrt();
                    g += s * (-d / 5.5).exp() * 0.6;
                }
                let lift = match m {
                    AIR => 0.3,
                    TREE => 0.4,
                    _ => 0.75 + 0.5 * hash(x as f64, r as f64 + 31.0),
                };
                lamp_land[k] = (g * lift) as f32;
            }
        }

        // the sky's unevenness and its stars
        let haze = (0..WL * W)
            .map(|k| fbm((k % W) as f64 * 0.04, (k / W) as f64 * 0.07, 3, 0.0) as f32)
            .collect();
        let star = (0..WL * W)
            .map(|k| {
                let h = hash((k % W) as f64, (k / W) as f64 + 101.0);
                if h > 0.986 {
                    (0.35 + (h - 0.986) * 45.0) as f32
                } else {
                    0.0
                }
            })
            .collect();
        // rows of the water that break the reflection into strips
        let gap = (0..H)
            .map(|r| u8::from(r >= WL && hash(r as f64, 404.0) < 0.3))
            .collect();

        Self {
            dots: Dots::new(Self::PALETTE),
            mat,
            sr,
            sg,
            sb,
            rec,
            rim,
            lamp_land,
            haze,
            star,
            gap,
            eave,
            base_a: vec![0.0; W],
            tall_a: vec![0.0; W],
            env_a: vec![0.0; W],
            base_b: vec![0.0; W],
            env_b: vec![0.0; W],
            rays_a: vec![0.0; RAYS + 2],
            rays_b: vec![0.0; RAYS + 2],
            r: vec![0.0; WL * W],
            g: vec![0.0; WL * W],
            b: vec![0.0; WL * W],
            light_x: vec![0.0; W],
        }
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        let wl = WL as f64;
        let hf = H as f64;
        // --- the curtains ----------------------------------------------------
        for xi in 0..W {
            let x = xi as f64;
            let u = x / W as f64;
            // the main curtain sweeps down from the upper left, low over the
            // fjord, and lifts again to the right; ripples travel along it and
            // fold it
            self.base_a[xi] = (5.0 + 40.0 * ((u / 0.6).min(1.0) * PI / 2.0).sin().powf(1.5)
                - 13.0 * smooth(0.6, 0.95, u)
                + 2.6 * (x * 0.07 - t * 0.55).sin()
                + 1.4 * (x * 0.17 + t * 0.9 + 1.3).sin()
                + 2.2 * (x * 0.22 + t * 1.2).sin()
                + 4.0 * (fbm(x * 0.015 + t * 0.03, 4.2, 2, 0.0) - 0.5))
                as f32;
            self.tall_a[xi] = (11.0 + 8.0 * fbm(x * 0.03 - t * 0.05, 1.7, 2, 0.0)) as f32;
            self.env_a[xi] = (smooth(0.0, 0.2, u)
                * smooth(0.98, 0.72, u)
                * (0.4 + 0.8 * fbm(x * 0.022 - t * 0.07, 8.8, 3, 0.0)))
                as f32;
            // a fainter curtain behind, higher up, on the right
            self.base_b[xi] = (14.0
                + 4.0 * (x * 0.035 + t * 0.3 + 2.0).sin()
                + 1.6 * (x * 0.11 - t * 0.7).sin()) as f32;
            self.env_b[xi] = (smooth(0.45, 0.7, u)
                * smooth(1.05, 0.85, u)
                * (0.25 + 0.5 * fbm(x * 0.03 + t * 0.05, 3.3, 2, 0.0)))
                as f32;
        }
        for i in 0..=RAYS + 1 {
            let u = i as f64 / 4.0;
            self.rays_a[i] =
                (0.14 + fbm(u * 0.6 + t * 0.35, t * 0.12, 3, 0.0).powf(2.4) * 2.5) as f32;
            self.rays_b[i] =
                (0.1 + fbm(u * 0.45 - t * 0.2, 5.0 + t * 0.1, 3, 0.0).powf(2.2) * 2.0) as f32;
        }
        for x in 0..W {
            let mut s = 0.0;
            let mut d = -24i32;
            while d <= 24 {
                let xx = (x as i32 + d).clamp(0, W as i32 - 1) as usize;
                s += f64::from(self.env_a[xx]) + 0.4 * f64::from(self.env_b[xx]);
                d += 6;
            }
            self.light_x[x] = (s / 9.0) as f32;
        }
        let flick = 0.92 + 0.05 * (t * 2.3).sin() + 0.03 * (t * 7.1).sin();

        // --- sky and land ----------------------------------------------------
        for r in 0..WL {
            let rf = r as f64;
            let y = rf + 0.5;
            let v = y / wl;
            for xi in 0..W {
                let k = r * W + xi;
                let x = xi as f64;
                let m = self.mat[k];
                let light_x = f64::from(self.light_x[xi]);
                let (mut cr, mut cg, mut cb);
                if m == AIR {
                    let hz = 0.85 + 0.3 * f64::from(self.haze[k]);
                    cr = (0.025 + 0.03 * v * v) * hz;
                    cg = (0.04 + 0.07 * v * v) * hz;
                    cb = (0.1 + 0.11 * v * v) * hz;
                    let base_a = f64::from(self.base_a[xi]);
                    let env_a = f64::from(self.env_a[xi]);
                    // curtain A: a sharp lower hem, rays rising and fading to violet
                    let mut a = 0.0;
                    let d = base_a - y;
                    if d > -4.0 && d < 46.0 {
                        let bend = (f64::from(self.base_a[(xi + 1).min(W - 1)])
                            - f64::from(self.base_a[xi.saturating_sub(1)]))
                        .abs();
                        let hc = f64::from(self.tall_a[xi]);
                        let lean = ray(&self.rays_a, x + d * 0.22);
                        let prof = if d < 0.0 {
                            (-d * d * 0.9).exp()
                        } else {
                            (1.0 - (-(d + 0.4) * 1.1).exp()) * (-d / hc).exp()
                        };
                        // the rays, over a continuous bright band along the hem
                        let band = if d < 0.0 {
                            (-d * d).exp()
                        } else {
                            (-d / 3.0).exp()
                        };
                        a = (prof * lean + 0.3 * band * (0.55 + 0.45 * lean.min(1.4)))
                            * env_a
                            * (1.15 + 0.35 * bend);
                        let up = clamp(d / (hc * 1.6));
                        let (gk, vk) = (1.0 - smooth(0.0, 0.5, up), smooth(0.4, 0.9, up));
                        let tk = 1.0 - gk - vk;
                        cr += a * (0.18 * gk + 0.06 * tk + 0.42 * vk);
                        cg += a * (1.0 * gk + 0.78 * tk + 0.16 * vk);
                        cb += a * (0.48 * gk + 0.7 * tk + 0.75 * vk);
                        // pink at the very hem where it is brightest
                        let hem = (-(d + 0.6).powi(2) * 1.2).exp()
                            * smooth(0.3, 0.8, a)
                            * 0.8
                            * (0.45 + 0.55 * lean.min(1.0));
                        cr += hem * 0.95;
                        cg *= 1.0 - 0.55 * (hem * 1.6).min(1.0);
                        cb += hem * 0.4;
                    }
                    // curtain B, further away, thinning out toward the top of the frame
                    let db = f64::from(self.base_b[xi]) - y;
                    if db > -3.0 && db < 30.0 {
                        let prof = if db < 0.0 {
                            (-db * db).exp()
                        } else {
                            (1.0 - (-(db + 0.4)).exp()) * (-db / 9.0).exp()
                        };
                        let b = prof
                            * ray(&self.rays_b, x + db * 0.18)
                            * f64::from(self.env_b[xi])
                            * 0.85
                            * smooth(0.0, 8.0, y);
                        let up = clamp(db / 12.0);
                        cr += b * (0.1 + 0.4 * up);
                        cg += b * (0.75 - 0.5 * up);
                        cb += b * (0.7 + 0.2 * up);
                        a += b;
                    }
                    // a little glow round the curtain, kept tight under the hem
                    // so the hem reads as an edge over dark sky
                    let below = y - base_a;
                    let gl = env_a
                        * if below > 0.0 {
                            (-below / 6.0).exp() * 0.1
                        } else {
                            (below / 8.0).exp() * 0.18
                        };
                    cr += gl * 0.12;
                    cg += gl * 0.62;
                    cb += gl * 0.52;
                    // airglow on the horizon
                    let ag = (-(wl - y) / 10.0).exp() * (0.1 + 0.16 * light_x);
                    cg += ag * 0.7;
                    cb += ag * 0.75;
                    cr += ag * 0.2;
                    let st = f64::from(self.star[k]);
                    if st > 0.0 {
                        let tw =
                            0.7 + 0.3 * (t * (1.3 + 3.0 * hash(x, rf)) + 6.28 * hash(rf, x)).sin();
                        let s = st * tw * clamp(1.0 - a * 1.6) * smooth(wl - 2.0, 30.0, y);
                        cr = cr.max(s * 0.9);
                        cg = cg.max(s * 0.94);
                        cb = cb.max(s);
                    }
                } else {
                    cr = f64::from(self.sr[k]);
                    cg = f64::from(self.sg[k]);
                    cb = f64::from(self.sb[k]);
                    let l = light_x * f64::from(self.rec[k]);
                    cr += l * 0.03;
                    cg += l * 0.14;
                    cb += l * 0.1;
                    let e = f64::from(self.rim[k]);
                    if e > 0.0 {
                        let q = e * if m == TREE {
                            0.2 + 0.45 * light_x
                        } else {
                            0.05 + 0.22 * light_x
                        };
                        cr += q * 0.25;
                        cg += q * 0.95;
                        cb += q * 0.7;
                    }
                    if m == PANE {
                        let f = flick + 0.04 * (t * 5.3 + x).sin();
                        (cr, cg, cb) = (f, 0.76 * f, 0.38 * f);
                    } else if m == DOOR && xi == DOOR_X[1] && rf > self.eave + 1.0 {
                        // light through the crack of the door
                        (cr, cg, cb) = (0.55 * flick, 0.36 * flick, 0.14 * flick);
                    }
                }
                let lg = f64::from(self.lamp_land[k]) * flick;
                if lg > 0.0 {
                    cr += lg;
                    cg += lg * 0.62;
                    cb += lg * 0.26;
                }
                (self.r[k], self.g[k], self.b[k]) = (cr as f32, cg as f32, cb as f32);
            }
        }

        for r in 0..H {
            let rf = r as f64;
            let y = rf + 0.5;
            let mut fade = 1.0;
            let dw = y - wl;
            let deep = if r >= WL {
                mix(1.0, 0.45, dw / (hf - wl))
            } else {
                1.0
            };
            let ry = WL as i64
                - 1
                - (r as i64 - WL as i64)
                - js_round(0.4 * (rf * 0.9 + t * 0.6).sin()) as i64;
            let r0 = ry.max(0) as usize * W;
            let r1 = (ry - 1).max(0) as usize * W;
            let r2 = (ry - 2).max(0) as usize * W;
            for xi in 0..W {
                let k = r * W + xi;
                let x = xi as f64;
                let (mut cr, mut cg, mut cb, floor);
                if r < WL {
                    cr = f64::from(self.r[k]);
                    cg = f64::from(self.g[k]);
                    cb = f64::from(self.b[k]);
                    floor = match self.mat[k] {
                        AIR => 0.1,
                        NEAR => 0.05,
                        TREE => 0.04,
                        _ => 0.06,
                    };
                } else {
                    // still water: the mirror image, stretched and broken by slow ripples
                    let wave = noise(x * 0.04 + t * 0.05, rf * 0.55 - t * 0.35, 0.0);
                    let sx = x + (0.5 + dw * 0.12) * (rf * 1.3 + t * 1.6 + wave * 4.0).sin();
                    let ix = sx.floor();
                    let fx = sx - ix;
                    let ix = ix.clamp(0.0, (W - 2) as f64) as usize;
                    let (a0, a1, a2) = (r0 + ix, r1 + ix, r2 + ix);
                    let ms = self.mat[a0];
                    let sky = ms == AIR;
                    let mut kr = (if sky { 0.55 } else { 0.64 }) * (0.82 + 0.3 * wave) * deep;
                    if sky && self.gap[r] != 0 && wave < 0.45 {
                        kr *= 0.12;
                    }
                    // the cabin's own image is soft; the lamplight road below carries it
                    if ms == PANE {
                        kr *= 0.4;
                    } else if ms == WALL || ms == ROOF || ms == DOOR {
                        kr *= 0.7;
                    }
                    let gx = 1.0 - fx;
                    let blend = |c: &[f32]| {
                        let p = |i: usize| f64::from(c[i]);
                        ((p(a0) * gx + p(a0 + 1) * fx) * 0.5
                            + (p(a1) * gx + p(a1 + 1) * fx) * 0.3
                            + (p(a2) * gx + p(a2 + 1) * fx) * 0.2)
                            * kr
                    };
                    cr = blend(&self.r);
                    cg = blend(&self.g);
                    cb = blend(&self.b);
                    if ms != WALL && ms != DOOR {
                        cr += 0.02;
                        cg += 0.035;
                        cb += 0.065;
                    }
                    // a pale line where the water meets the shore
                    if r == WL {
                        let e =
                            0.3 * (0.3 + 0.7 * smooth(0.25, 0.75, noise(x * 0.3, t * 0.4, 0.0)));
                        cr += e * 0.6;
                        cg += e * 0.85;
                        cb += e;
                    }
                    // the lamplight laid on the water as a broken golden road
                    if dw < 14.0 && xi > 138 && xi < 166 {
                        let rip = noise(x * 0.5 - t * 0.2, rf * 1.4 - t * 1.5, 0.0);
                        for &[lx, s] in &LAMPS {
                            let lw = 1.0 + dw * 0.1;
                            let q = (x + 0.5 - lx) / lw;
                            let g = (-q * q).exp()
                                * (-dw / 8.0).exp()
                                * smooth(0.3, 0.65, rip)
                                * s
                                * 1.6
                                * flick;
                            cr += g;
                            cg += g * 0.68;
                            cb += g * 0.28;
                        }
                    }
                    floor = if sky { 0.07 * deep } else { 0.03 };
                    fade = smooth(hf + 3.0, hf - 12.0, y);
                }
                let peak = cr.max(cg).max(cb).max(1e-4);
                let level = clamp(floor + (1.0 - floor) * peak.powf(0.85) * 0.95) * fade;
                let step = Dots::step(level, bayer(r, xi));
                out[k] = self.dots.ink(step, level, [cr, cg, cb], peak);
            }
        }
    }
}
