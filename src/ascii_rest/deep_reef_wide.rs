//! deep reef, wide: [`super::deep_reef`] recomposed for a 3.2:1 panel. Both
//! reefs broaden with more coral heads on them, the open sand between them
//! widens with two more bommies, the sun moves 60 columns right with the
//! view's centre, and a third, hazier kelp stands out on the sand.
//!
//! deep reef: looking along a coral reef from a few metres down. The sun is a
//! bright blaze in the rippled surface, shafts of light fan down from it,
//! kelp sways in the swell, a school of fish wheels through the dark water,
//! bubbles rise and caustics crawl over the sand.
//!
//! The water, the sand and the reef are shaded once; each frame adds the light
//! that moves (shafts, ripples, caustics) and draws what swims or sways on top.
//! Every cell is then a halftone dot sized by its brightness, ordered-dithered,
//! in the palette colour nearest its hue.

use super::halftone::{bayer, Dots};
use super::math::{clamp, fbm, hash, js_hypot, js_round, mix, noise, smooth, Mulberry32};
use super::{Fit, Piece, hex};
use crate::grid::Cell;

const W: usize = 320;
const H: usize = 100;
const N: usize = W * H;
const SURF: f64 = 15.0; // the surface band
const HZ: usize = 61; // where the sea floor would meet the haze
const SUNX: f64 = 196.0; // where the sun shows through the surface
const SUNY: f64 = -12.0;
const RAYS: usize = 320;
const RAY0: f64 = 160.0; // the shaft straight down from the sun
/// Caustic tile size, and its Voronoi cells per side.
const CT: usize = 64;
const CC: usize = 6;

const NONE: u8 = 0;
const SAND: u8 = 1;
const REEF: u8 = 2;
const FAR: u8 = 3;
const FAN: u8 = 4;

const CORAL: [[f64; 3]; 4] = [
    [0.86, 0.36, 0.44],
    [0.93, 0.55, 0.3],
    [0.6, 0.36, 0.72],
    [0.78, 0.7, 0.35],
];
const ROCK: [f64; 3] = [0.016, 0.03, 0.042];
const MID: f64 = 172.0; // the middle of the open water
const CAM: f64 = 160.0; // the column the sand runs away from
const SPLIT: f64 = 180.0; // left of here a cell belongs to the left reef
const LW: f64 = 130.0; // the left reef's reach
const RW: f64 = 80.0; // the right reef's reach

// both reefs are rounded masses, shouldering down toward the open sand
fn dome(u: f64) -> f64 {
    1.0 - (1.0 - u * u).max(0.0).sqrt()
}

fn left_top(x: f64) -> f64 {
    47.0 + 56.0 * dome((x / LW).min(1.0)) - 8.0 * fbm(x * 0.06, 3.1, 4, 0.0)
        + 3.0 * ((10.0 - x) / 10.0).max(0.0)
}

fn right_top(x: f64) -> f64 {
    69.0 + 34.0 * dome(((316.0 - x) / RW).min(1.0))
        - 5.0 * fbm(x * 0.08, 8.3, 3, 0.0)
        - 3.0 * (-((x - 311.0) / 6.0).powi(2)).exp()
}

fn far_top(x: f64) -> f64 {
    HZ as f64
        - 1.0
        - 6.0 * fbm(x * 0.035 + 2.0, 1.7, 3, 0.0)
        - 3.0 * (-((x - 180.0) / 18.0).powi(2)).exp()
}

/// A reef's crest row at a column.
type TopAt = fn(f64) -> f64;

struct Fish {
    a: f64,
    b: f64,
    ph: f64,
    sp: f64,
}

struct Kelp {
    bx: f64,
    base: usize,
    len: usize,
    sz: f64,
    ph: f64,
    haze: f64,
}

struct Bubble {
    x0: f64,
    y0: f64,
    ph: f64,
    sp: f64,
    w: f64,
}

struct Speck {
    x: f64,
    y: f64,
    sp: f64,
    ph: f64,
    b: f64,
}

pub struct DeepReefWide {
    dots: Dots,
    caus: Vec<f32>,
    wr: Vec<f32>,
    wg: Vec<f32>,
    wb: Vec<f32>,
    ray_at: Vec<u16>,
    mat: Vec<u8>,
    fog: Vec<f32>,
    cu: Vec<f32>,
    cv: Vec<f32>,
    cw: Vec<f32>,
    br: Vec<f32>,
    bg: Vec<f32>,
    bb: Vec<f32>,
    fish: Vec<Fish>,
    kelp: Vec<Kelp>,
    bubbles: Vec<Bubble>,
    snow: Vec<Speck>,
    cr: Vec<f32>,
    cg: Vec<f32>,
    cb: Vec<f32>,
    floor: Vec<f32>,
    rays: Vec<f32>,
}

impl DeepReefWide {
    fn caus_at(&self, u: f64, v: f64) -> f64 {
        let ct = CT as f64;
        let u = ((u % ct) + ct) % ct;
        let v = ((v % ct) + ct) % ct;
        let (x0, y0) = (u as usize, v as usize);
        let (ax, ay) = (u - x0 as f64, v - y0 as f64);
        let (x1, y1) = ((x0 + 1) % CT, (y0 + 1) % CT);
        let a = f64::from(self.caus[y0 * CT + x0]);
        let b = f64::from(self.caus[y0 * CT + x1]);
        let c = f64::from(self.caus[y1 * CT + x0]);
        let d = f64::from(self.caus[y1 * CT + x1]);
        a + (b - a) * ax + (c - a) * ay + (a - b - c + d) * ax * ay
    }

    fn add(&mut self, x: f64, r: f64, rgb: [f64; 3]) {
        if x < 0.0 || x >= W as f64 || r < 0.0 || r >= H as f64 {
            return;
        }
        let k = r as usize * W + x as usize;
        self.cr[k] = (f64::from(self.cr[k]) + rgb[0]) as f32;
        self.cg[k] = (f64::from(self.cg[k]) + rgb[1]) as f32;
        self.cb[k] = (f64::from(self.cb[k]) + rgb[2]) as f32;
    }

    fn put(&mut self, x: f64, r: f64, rgb: [f64; 3], a: f64) {
        if x < 0.0 || x >= W as f64 || r < 0.0 || r >= H as f64 {
            return;
        }
        let k = r as usize * W + x as usize;
        self.cr[k] = mix(f64::from(self.cr[k]), rgb[0], a) as f32;
        self.cg[k] = mix(f64::from(self.cg[k]), rgb[1], a) as f32;
        self.cb[k] = mix(f64::from(self.cb[k]), rgb[2], a) as f32;
    }
}

impl Piece for DeepReefWide {
    const NAME: &'static str = "deep-reef-wide";
    const COLS: usize = W;
    const ROWS: usize = H;
    const FPS: u32 = 15;
    const CELL: usize = 1;
    const FIT: Fit = Fit::Cover { anchor: 0.5 };
    const UPSTREAM: bool = false;
    const GROUND: u32 = hex("#03101a");
    // A dot is never drawn darker than about half brightness (dot size carries
    // the darkness), so the palette starts at mid tones.
    #[rustfmt::skip]
    const PALETTE: &'static [u32] = &[
        hex("#114a66"), hex("#165c78"), hex("#1d708a"), hex("#27869b"), hex("#3a9fae"), hex("#58b9c0"), hex("#80d2d2"), hex("#b0e8e2"), hex("#e2fbf5"),
        hex("#5e6b62"), hex("#87927f"), hex("#aab39a"), hex("#cfd2b4"),
        hex("#38461a"), hex("#5a6420"), hex("#857f2a"), hex("#b0a03c"), hex("#d6c25a"), hex("#efe08e"),
        hex("#8e3355"), hex("#c4506a"), hex("#e0786e"), hex("#f0a070"),
        hex("#6a3a78"), hex("#9a5aa8"), hex("#c88ad0"),
        hex("#6f8a98"), hex("#9fb8c6"), hex("#cfe3ea"),
    ];

    fn new() -> Self {
        // --- caustics: a tiling web of bright lines (cell edges of a Voronoi)
        let ct = CT as f64;
        let cs = ct / CC as f64;
        let mut caus = vec![0f32; CT * CT];
        {
            let mut rnd = Mulberry32(7);
            let mut pts = Vec::with_capacity(CC * CC);
            for j in 0..CC {
                for i in 0..CC {
                    let px = (i as f64 + 0.15 + 0.7 * rnd.next()) * cs;
                    let py = (j as f64 + 0.15 + 0.7 * rnd.next()) * cs;
                    pts.push([px, py]);
                }
            }
            for y in 0..CT {
                for x in 0..CT {
                    let (mut f1, mut f2) = (1e9, 1e9);
                    for &[px, py] in &pts {
                        for oy in [-ct, 0.0, ct] {
                            for ox in [-ct, 0.0, ct] {
                                let dx = x as f64 + 0.5 - px - ox;
                                let dy = y as f64 + 0.5 - py - oy;
                                let d = (dx * dx + dy * dy).sqrt();
                                if d < f1 {
                                    f2 = f1;
                                    f1 = d;
                                } else if d < f2 {
                                    f2 = d;
                                }
                            }
                        }
                    }
                    caus[y * CT + x] = (1.0 - smooth(0.0, 0.42 * cs, f2 - f1)).powf(2.2) as f32;
                }
            }
        }

        // --- the water: deep navy, lit from the surface and the sun ---------
        let (mut wr, mut wg, mut wb) = (vec![0f32; N], vec![0f32; N], vec![0f32; N]);
        // which shaft each cell sits in: shafts fan out from the sun, above the frame
        let mut ray_at = vec![0u16; N];
        for r in 0..H {
            let rf = r as f64;
            for xi in 0..W {
                let x = xi as f64;
                let k = r * W + xi;
                let v = (rf + 0.5) / H as f64;
                let up = (1.0 - v).powf(2.2);
                let sun = (-((x - SUNX) / 52.0).powi(2) - ((rf - 4.0) / 34.0).powi(2)).exp();
                // looking level, the distance is a lit blue haze the reefs stand against
                let haze = (-((rf - 54.0) / 15.0).powi(2)).exp()
                    * (1.0 - 0.6 * smooth(30.0, 160.0, (x - MID).abs()));
                // darker toward the sides and the floor, but the upper water stays lit
                // so the kelp stands dark against it
                let edge = 1.0
                    - 0.4 * smooth(80.0, 164.0, (x - MID).abs()) * smooth(8.0, 70.0, rf)
                    - 0.3 * smooth(66.0, 100.0, rf);
                // soft clouds of plankton haze, so open water is never one flat tone
                let veil = 0.78 + 0.44 * fbm(x * 0.022 + 5.0, rf * 0.04, 4, 0.0);
                wr[k] = ((0.012 + 0.09 * up + 0.05 * sun + 0.04 * haze) * edge * veil) as f32;
                wg[k] = ((0.1 + 0.32 * up + 0.14 * sun + 0.15 * haze) * edge * veil) as f32;
                wb[k] = ((0.15 + 0.27 * up + 0.12 * sun + 0.17 * haze) * edge * veil) as f32;
                let a = (x + 0.5 - SUNX).atan2(rf + 0.5 - SUNY);
                ray_at[k] = js_round((a + 1.6) * 100.0).clamp(0.0, (RAYS - 1) as f64) as u16;
            }
        }

        // --- the static scene: sand, the reef, a far reef in the haze, a sea fan
        let mut mat = vec![NONE; N];
        // lit colour
        let (mut ar, mut ag, mut ab) = (vec![0f32; N], vec![0f32; N], vec![0f32; N]);
        let mut fog = vec![0f32; N]; // how much water stands between us and it
                                     // caustic coords, weight
        let (mut cu, mut cv, mut cw) = (vec![0f32; N], vec![0f32; N], vec![0f32; N]);
        // brain corals: domes on the crests
        // [x, depth of the centre below the crest, radius, kind]
        let domes: [[f64; 4]; 10] = [
            [24.0, 3.0, 7.0, 0.0],
            [44.0, 2.5, 5.5, 2.0],
            [62.0, 2.5, 6.5, 1.0],
            [80.0, 2.0, 4.5, 3.0],
            [98.0, 2.0, 5.0, 2.0],
            [118.0, 2.0, 4.0, 1.0],
            [253.0, 2.0, 4.0, 2.0],
            [276.0, 2.5, 5.0, 0.0],
            [299.0, 3.0, 5.0, 1.0],
            [314.0, 3.0, 4.5, 3.0],
        ]
        .map(|[x, d, r, kind]| {
            let top = if x < SPLIT { left_top(x) } else { right_top(x) };
            [x, top + d, r, kind]
        });
        // [x, crest row, half width, fog, coral kind]
        const BOMMIES: [[f64; 5]; 5] = [
            [174.0, 69.0, 9.0, 0.18, 0.0],
            [154.0, 63.6, 4.0, 0.5, 2.0],
            [199.0, 64.4, 4.0, 0.42, 1.0],
            [224.0, 65.2, 5.0, 0.36, 3.0],
            [141.0, 66.8, 3.5, 0.3, 1.0],
        ];
        let fan_c = [268.0, right_top(268.0) + 1.5];
        const FAN_R: f64 = 27.0;
        let hz = HZ as f64;
        for r in 0..H {
            let rf = r as f64;
            let y = rf + 0.5;
            for xi in 0..W {
                let x = xi as f64;
                let k = r * W + xi;
                // the sea floor, a plane running off into the haze
                if r >= HZ + 2 {
                    let d = y - hz;
                    let z = 300.0 / d; // distance
                                       // the floor is seen at a low angle, so its pattern squeezes toward the
                                       // haze; spacing grows as it comes nearer, and fades out before it
                                       // gets too fine to draw
                    let near = 0.4 + 0.6 * (d / 38.0);
                    let sx = (x + 0.5 - CAM) / near;
                    let sy = 30.0 * d.ln();
                    let aa = smooth(2.2, 1.4, 30.0 / d);
                    // ripples in the sand run across our view, bending a little: thin
                    // bright crests over darker troughs
                    let ph = sy + 7.0 * fbm(sx * 0.07, sy * 0.06, 2, 0.0) + sx * 0.05;
                    let crest = (0.5 + 0.5 * ph.sin()).powf(3.0)
                        * smooth(0.3, 0.6, fbm(sx * 0.09 + 7.0, sy * 0.12, 2, 0.0));
                    let rip = 0.74
                        + aa * (0.5 * crest - 0.12)
                        + 0.12 * (fbm(sx * 0.08, sy * 0.15, 3, 0.0) - 0.5);
                    mat[k] = SAND;
                    let lit = 0.6 + 0.15 * smooth(160.0, 190.0, x) - 0.3 * smooth(84.0, 100.0, rf);
                    ar[k] = (0.38 * rip * lit) as f32;
                    ag[k] = (0.37 * rip * lit) as f32;
                    ab[k] = (0.3 * rip * lit) as f32;
                    fog[k] = (1.0 - (-z / 40.0).exp()) as f32;
                    cu[k] = (sx * 0.9) as f32;
                    cv[k] = (sy * 2.3) as f32;
                    cw[k] = (0.5 * smooth(14.0, 26.0, d)) as f32;
                }
                if y >= far_top(x) && r < HZ + 6 {
                    mat[k] = FAR;
                    // lit along its crest, dim below, half lost in the blue
                    let s = (0.45 + 0.55 * fbm(x * 0.15, y * 0.15, 2, 0.0))
                        * (0.5 + 0.8 * (-(y - far_top(x)) / 2.0).exp());
                    ar[k] = (0.06 * s) as f32;
                    ag[k] = (0.11 * s) as f32;
                    ab[k] = (0.14 * s) as f32;
                    fog[k] = 0.68;
                    cw[k] = 0.0;
                }
                // the sea fan: a thin lattice of veins spreading up from its root
                {
                    let (dx, dy) = (x + 0.5 - fan_c[0], fan_c[1] - y);
                    let (d, a) = (js_hypot(&[dx, dy]), dx.atan2(dy));
                    let reach = FAN_R * (0.68 + 0.36 * fbm(a * 2.6 + 5.0, 1.0, 3, 0.0));
                    if dy > -1.0 && a.abs() < 1.2 && d < reach {
                        let aw = a + 0.08 * (fbm(d * 0.2, a * 2.0, 2, 0.0) - 0.5) * 4.0;
                        let ray = (((aw * 7.5) / std::f64::consts::PI + 100.0) % 1.0 - 0.5).abs();
                        let ring = ((d / 3.2 + 100.0) % 1.0 - 0.5).abs();
                        let mesh = hash(x * 3.0, rf * 5.0) < 0.08;
                        let vein = ray > 0.42
                            || (ring > 0.45 && d > 5.0)
                            || mesh
                            || (d < 4.0 && dx.abs() < 1.2)
                            || d > reach - 1.2;
                        mat[k] = FAN;
                        // dark at the root, catching the light toward its rim; between the
                        // veins a thin dim web the water shows through
                        let o = smooth(2.0, reach, d);
                        let s = (0.35 + 0.75 * o) * if vein { 1.0 } else { 0.22 };
                        ar[k] = (mix(0.32, 0.72, o) * s) as f32;
                        ag[k] = (mix(0.12, 0.38, o) * s) as f32;
                        ab[k] = (mix(0.38, 0.8, o) * s) as f32;
                        fog[k] = if vein { 0.2 } else { 0.4 };
                        cw[k] = 0.0;
                    }
                }
                // small coral heads out on the sand, half lost in the blue
                for &[bx, crest, hw, f0, kind] in &BOMMIES {
                    let u = (x + 0.5 - bx) / hw;
                    let top = crest + u * u * u * u * hw * 0.5 - 1.6 * fbm(x * 0.35, crest, 2, 0.0);
                    if u.abs() < 1.1
                        && y >= top
                        && y < crest + hw * 0.5 + 1.5 * fbm(x * 0.3, crest + 5.0, 2, 0.0)
                            - 0.6 * u * u
                    {
                        mat[k] = REEF;
                        let below = y - top;
                        let living = smooth(2.2, 0.6, below)
                            * smooth(0.45, 0.6, fbm(x * 0.25 + bx, y * 0.3, 2, 0.0));
                        let lit = 0.3 + 0.6 * (-below / 1.5).exp();
                        let [cr0, cg0, cb0] = CORAL[kind as usize];
                        ar[k] = (mix(ROCK[0], cr0, living) * lit) as f32;
                        ag[k] = (mix(ROCK[1], cg0, living) * lit) as f32;
                        ab[k] = (mix(ROCK[2], cb0, living) * lit) as f32;
                        fog[k] = f0 as f32;
                        cu[k] = (x * 0.5) as f32;
                        cv[k] = (y * 0.9) as f32;
                        cw[k] = (0.5 * (-below / 1.5).exp()) as f32;
                    }
                }
                // reef masses, left and right: dark rock, rimmed with lit coral
                let top = if x < SPLIT { left_top(x) } else { right_top(x) };
                if y >= top && !(131.0..=235.0).contains(&x) {
                    mat[k] = REEF;
                    let below = y - top;
                    let n = fbm(x * 0.18, y * 0.22, 4, 0.0);
                    // lumps of rock and coral heads, each lit on its upper side
                    let lump = fbm(x * 0.07, y * 0.1, 3, 0.0);
                    let lump_up = fbm(x * 0.07, (y - 1.5) * 0.1, 3, 0.0);
                    let face = clamp(0.5 + (lump_up - lump) * 14.0);
                    let lit = (0.25 + 0.5 * face + 0.6 * (-below / 5.0).exp() + 0.14 * (n - 0.5))
                        * (0.55 + 0.45 * smooth(100.0, 40.0, rf));
                    // patches of living coral, thick along the crest, a few sponges below
                    let kind = (fbm(x * 0.06 + 11.0, y * 0.09, 3, 0.0) * 7.0).floor() as usize % 4;
                    let patch = fbm(x * 0.12 + 4.0, y * 0.12, 3, 0.0);
                    let living = (smooth(0.44, 0.56, patch) * smooth(7.0, 1.5, below))
                        .max(smooth(0.66, 0.72, patch) * 0.35 * smooth(24.0, 6.0, below));
                    let [cr0, cg0, cb0] = CORAL[kind];
                    // a cool rim of light along the bare rock of the crest
                    let rim = smooth(2.4, 0.3, below) * 0.6;
                    // and the upper lips of ledges down the face catch a little of it
                    let ledge = smooth(0.66, 0.9, face) * smooth(3.0, 8.0, below) * 0.55;
                    let rr = mix(mix(ROCK[0], 0.05, ledge), 0.12, rim);
                    let rg = mix(mix(ROCK[1], 0.11, ledge), 0.24, rim);
                    let rb = mix(mix(ROCK[2], 0.13, ledge), 0.25, rim);
                    ar[k] = (mix(rr, cr0, living) * lit) as f32;
                    ag[k] = (mix(rg, cg0, living) * lit) as f32;
                    ab[k] = (mix(rb, cb0, living) * lit) as f32;
                    fog[k] = if x < SPLIT {
                        0.04 + 0.04 * smooth(0.0, LW - 10.0, x)
                    } else {
                        0.08
                    } as f32;
                    cu[k] = (x * 0.5) as f32;
                    cv[k] = (y * 0.9) as f32;
                    cw[k] = (0.8 * (-below / 1.5).exp()) as f32;
                }
                for &[dx0, dy0, dr, kind] in &domes {
                    let (dx, dy) = (x + 0.5 - dx0, y - dy0);
                    let d = js_hypot(&[dx, dy * 1.25]);
                    if d < dr && dy < dr * 0.3 {
                        mat[k] = REEF;
                        let nz = (1.0 - (d / dr).powi(2)).max(0.0).sqrt();
                        let lamb =
                            clamp(0.15 + 0.85 * (nz * 0.6 - (dy / dr) * 0.55 + (dx / dr) * 0.25));
                        let groove = 0.75
                            + 0.25 * (d * 2.4 + 2.0 * fbm(x * 0.3, y * 0.3, 2, 0.0) * 3.0).sin();
                        let [cr0, cg0, cb0] = CORAL[(kind as usize + 1) % 4];
                        let s = (0.15 + 0.9 * lamb) * groove;
                        ar[k] = (cr0 * s) as f32;
                        ag[k] = (cg0 * s) as f32;
                        ab[k] = (cb0 * s) as f32;
                        fog[k] = if dx0 < SPLIT {
                            0.04 + 0.04 * smooth(0.0, LW - 10.0, dx0)
                        } else {
                            0.08
                        } as f32;
                        cu[k] = (x * 0.5) as f32;
                        cv[k] = (y * 0.9) as f32;
                        cw[k] = (0.5 * clamp(-dy / dr + 0.6)) as f32;
                    }
                }
            }
        }
        // branching coral standing up off both crests
        let groups: [(u32, usize, f64, f64, TopAt); 2] = [
            (31, 15, 4.0, 92.0, left_top),
            (53, 5, 280.0, 34.0, right_top),
        ];
        for (seed, count, x0, span, top_at) in groups {
            let mut rnd = Mulberry32(seed);
            for i in 0..count {
                let bx = x0 + rnd.next() * span;
                if (bx - fan_c[0]).abs() < 3.0 {
                    continue; // leave the fan's root clear
                }
                let base = top_at(bx);
                let h = 2.0 + rnd.next() * 3.5;
                let lean = (rnd.next() - 0.5) * 0.5;
                let kind = if rnd.next() < 0.5 { 1 } else { 3 };
                let mut s = 0.0;
                while s < h {
                    let branch = if s > h * 0.55 {
                        (if i % 2 == 1 { 1.0 } else { -1.0 }) * (s - h * 0.55) * 0.6
                    } else {
                        0.0
                    };
                    let x = js_round(bx + lean * s + branch);
                    let r = js_round(base - s);
                    // upstream's typed arrays drop writes past the end
                    if x < 0.0 || x >= W as f64 || r < 0.0 || r >= H as f64 {
                        s += 0.5;
                        continue;
                    }
                    let k = r as usize * W + x as usize;
                    mat[k] = REEF;
                    let tip = s / h;
                    let [cr0, cg0, cb0] = CORAL[kind];
                    let sh = 0.4 + 0.6 * tip;
                    ar[k] = (cr0 * sh) as f32;
                    ag[k] = (cg0 * sh) as f32;
                    ab[k] = (cb0 * sh) as f32;
                    fog[k] = 0.06;
                    cu[k] = (x * 0.5) as f32;
                    cv[k] = (r * 0.9) as f32;
                    cw[k] = 0.4;
                    s += 0.5;
                }
            }
        }
        // the water's colour filters what is behind it: reds go first
        let (mut br, mut bg, mut bb) = (vec![0f32; N], vec![0f32; N], vec![0f32; N]);
        for k in 0..N {
            if mat[k] == NONE {
                (br[k], bg[k], bb[k]) = (wr[k], wg[k], wb[k]);
                continue;
            }
            let f = f64::from(fog[k]);
            let tint = 1.0 - f;
            let ex = 1.9; // shallow water: the reef takes plenty of light
            br[k] = mix(
                f64::from(ar[k]) * ex * (0.55 + 0.45 * tint),
                f64::from(wr[k]),
                f,
            ) as f32;
            bg[k] = mix(
                f64::from(ag[k]) * ex * (0.85 + 0.15 * tint),
                f64::from(wg[k]),
                f,
            ) as f32;
            bb[k] = mix(f64::from(ab[k]) * ex, f64::from(wb[k]), f) as f32;
        }

        // --- the moving things, laid out once --------------------------------
        let mut rnd = Mulberry32(1234);
        let gauss = |rnd: &mut Mulberry32| {
            let mut s = 0.0;
            for _ in 0..4 {
                s += rnd.next();
            }
            (s - 2.0) * 1.7
        };
        let fish = (0..150)
            .map(|_| Fish {
                a: gauss(&mut rnd) * 10.0,
                b: gauss(&mut rnd) * 3.8,
                ph: rnd.next() * 6.28,
                sp: 0.8 + rnd.next() * 0.6,
            })
            .collect();
        // kelp: x, base row, length, width, phase, and how much water hides it
        let kelp = vec![
            Kelp {
                bx: 14.0,
                base: 100,
                len: 96,
                sz: 3.5,
                ph: 0.0,
                haze: 0.0,
            },
            Kelp {
                bx: 309.0,
                base: 100,
                len: 96,
                sz: 3.5,
                ph: 5.2,
                haze: 0.0,
            },
            // further off, out on the sand, half lost in the blue
            Kelp {
                bx: 238.0,
                base: 94,
                len: 70,
                sz: 2.4,
                ph: 2.6,
                haze: 0.5,
            },
        ];
        let streams = [
            (66.0, 53.0, 8),
            (262.0, 70.0, 7),
            (176.0, 96.0, 6),
            (112.0, 74.0, 5),
        ];
        let mut bubbles = Vec::new();
        for (x0, y0, n) in streams {
            for _ in 0..n {
                bubbles.push(Bubble {
                    x0,
                    y0,
                    ph: rnd.next(),
                    sp: 4.0 + rnd.next() * 3.0,
                    w: rnd.next() * 6.28,
                });
            }
        }
        let snow = (0..144)
            .map(|_| Speck {
                x: rnd.next() * W as f64,
                y: rnd.next() * H as f64,
                sp: 0.3 + rnd.next() * 0.7,
                ph: rnd.next() * 6.28,
                b: 0.12 + rnd.next() * 0.22,
            })
            .collect();

        Self {
            dots: Dots::new(Self::PALETTE),
            caus,
            wr,
            wg,
            wb,
            ray_at,
            mat,
            fog,
            cu,
            cv,
            cw,
            br,
            bg,
            bb,
            fish,
            kelp,
            bubbles,
            snow,
            cr: vec![0f32; N],
            cg: vec![0f32; N],
            cb: vec![0f32; N],
            floor: vec![0f32; N],
            rays: vec![0f32; RAYS],
        }
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        // shafts: a slow pattern across the surface, shimmering as the waves pass,
        // strongest straight under the sun
        for u in 0..RAYS {
            let uf = u as f64;
            let s = smooth(0.5, 0.62, fbm(uf * 0.06 + t * 0.025, 3.3, 3, 0.0)).powf(1.2);
            let shimmer = 0.6 + 0.4 * noise(uf * 0.18 - t * 0.7, t * 0.3, 0.0);
            self.rays[u] =
                (s * shimmer * (0.35 + 0.65 * (-((uf - RAY0) / 60.0).powi(2)).exp())) as f32;
        }
        let (cx0, cy0, cx1, cy1) = (t * 0.9, t * 0.35, -t * 0.6 + 21.0, t * 0.5 + 9.0);
        // the sun's blaze wobbles as the swell passes over it
        let hx = SUNX + 1.6 * (t * 0.6).sin() + 0.8 * (t * 1.7 + 1.0).sin();

        for r in 0..H {
            let rf = r as f64;
            let y = rf + 0.5;
            let depth_fade = (-rf / 30.0).exp() * smooth(0.0, 14.0, rf + 6.0);
            for xi in 0..W {
                let x = xi as f64;
                let k = r * W + xi;
                let (mut cr, mut cg, mut cb) = (
                    f64::from(self.br[k]),
                    f64::from(self.bg[k]),
                    f64::from(self.bb[k]),
                );
                let m = self.mat[k];
                let ray = f64::from(self.rays[self.ray_at[k] as usize]) * depth_fade;
                if m == NONE {
                    if y < SURF + 6.0 {
                        // the underside of the surface: a rippling ceiling, pressed flat toward the haze
                        let dist = SURF + 7.0 - y;
                        let q = 30.0 / dist;
                        let a =
                            self.caus_at(((x - CAM) / dist) * 1.6 + t * 1.6, q * 6.0 + t * 0.8);
                        let b2 = self.caus_at(
                            ((x - CAM) / dist) * 1.2 - t * 1.1 + 31.0,
                            q * 4.5 - t * 0.6 + 17.0,
                        );
                        let lit = smooth(SURF + 6.0, 0.0, y).powf(1.3);
                        let c = a.max(b2).powf(1.6);
                        let near = (-((x - hx) / 34.0).powi(2)).exp();
                        // troughs darken the water under them, crests focus light into lines
                        let dim = 1.0 - lit * 0.7 * (1.0 - c);
                        cr *= dim;
                        cg *= dim;
                        cb *= dim;
                        let v = lit * c * (0.1 + 0.42 * near);
                        // the sun: a compact blaze through the surface, broken by the ripples
                        let dx = x + 0.5 - hx;
                        let hot = (-(dx / 8.0).powi(2) - ((y - 6.0) / 3.6).powi(2)).exp()
                            * (1.0 + 0.3 * c);
                        let halo = (-(dx / 15.0).powi(2) - ((y - 6.0) / 7.0).powi(2)).exp()
                            * 0.3
                            * (0.6 + 0.6 * c);
                        cr += 0.55 * v + 0.9 * hot + 0.35 * halo;
                        cg += 0.85 * v + 1.0 * hot + 0.6 * halo;
                        cb += 0.8 * v + 0.97 * hot + 0.58 * halo;
                    }
                    cr += 0.42 * ray;
                    cg += 0.86 * ray;
                    cb += 0.78 * ray;
                    self.floor[k] = 0.0;
                } else {
                    let f = f64::from(self.fog[k]);
                    let cwk = f64::from(self.cw[k]);
                    if cwk > 0.0 {
                        let (cuk, cvk) = (f64::from(self.cu[k]), f64::from(self.cv[k]));
                        let c = self.caus_at(cuk + cx0, cvk + cy0).min(1.0) * 0.6
                            + self.caus_at(cuk * 0.8 + cx1, cvk * 0.8 + cy1) * 0.6;
                        let s = cwk * c * c * (1.0 - f) * (0.6 + 0.6 * ray + 0.3 * depth_fade);
                        // on the sand the light comes back warm, on the reef cool
                        if m == SAND {
                            cr += 0.62 * s;
                            cg += 0.66 * s;
                            cb += 0.5 * s;
                        } else {
                            cr += 0.55 * s;
                            cg += 0.72 * s;
                            cb += 0.62 * s;
                        }
                    }
                    cr += 0.16 * ray * f;
                    cg += 0.3 * ray * f;
                    cb += 0.28 * ray * f;
                    self.floor[k] = 0.02;
                }
                self.cr[k] = cr as f32;
                self.cg[k] = cg as f32;
                self.cb[k] = cb as f32;
            }
        }

        // the school: one body turning along a slow loop, each fish a beat behind
        let path_x = |s: f64| 155.0 + 34.0 * (0.11 * s + 0.4).sin();
        let path_y = |s: f64| 44.0 + 8.0 * (0.17 * s + 2.2).sin();
        for i in 0..self.fish.len() {
            let Fish { a, b, ph, sp } = self.fish[i];
            let s = t - a * 0.08;
            let vx = 34.0 * 0.11 * (0.11 * s + 0.4).cos();
            let vy = 8.0 * 0.17 * (0.17 * s + 2.2).cos();
            let th = vy.atan2(vx);
            let (ct, st) = (th.cos(), th.sin());
            let wob = (t * 1.3 * sp + ph).sin();
            let px = path_x(s) + a * ct - b * st + wob * 0.6;
            let py = path_y(s) + a * st + b * ct + (t * sp + ph).cos() * 0.4;
            let h = th + 0.15 * wob;
            let (dx, dy) = (h.cos(), h.sin());
            // dark shapes against the light, until a fish turns its silver flank to it
            let flash = smooth(
                0.72,
                0.97,
                (th * 1.6 + a * 0.35 + b * 0.4 - t * 0.5 + 0.6).sin().abs(),
            );
            let rgb = [
                mix(0.008, 0.85, flash),
                mix(0.018, 0.97, flash),
                mix(0.026, 1.0, flash),
            ];
            for j in 0..2 {
                let jf = f64::from(j);
                let gx = js_round(px - dx * jf * 0.9);
                let gy = js_round(py - dy * jf * 0.9);
                self.put(gx, gy, rgb, if j == 0 { 1.0 } else { 0.85 });
            }
        }

        // kelp: dark fronds framing the view, a gold edge only where a shaft hits
        for i in 0..self.kelp.len() {
            let Kelp {
                bx,
                base,
                len,
                sz,
                ph,
                haze,
            } = self.kelp[i];
            let side = if bx < SUNX { 1.0 } else { -1.0 }; // which way the sun lies
            for r in (base.saturating_sub(len)..base).rev() {
                let rf = r as f64;
                let s01 = (base - r) as f64 / len as f64;
                let lean = side * 2.5 * sz * s01 * s01;
                // the swell runs up the frond, so it bends in an S rather than tipping like a stick
                let bend = (t * 0.55 + ph - s01 * 4.2).sin() * 2.2 * sz * s01.powf(1.2)
                    + (t * 0.21 + ph).sin() * 1.5 * s01
                    + lean;
                let x = bx + bend;
                // the frond: a stipe with blades off alternate sides, each blade a lobe
                // that swells and tapers, trailing a little behind the stipe's sway
                let stem = 0.8 + 0.4 * sz * (1.0 - s01);
                let beat = s01 * len as f64 * 0.32 + ph;
                let lobe_l = beat.sin().max(0.0).powf(1.2) * sz * 2.3 * (1.0 - 0.35 * s01);
                let lobe_r = (-beat.sin()).max(0.0).powf(1.2) * sz * 2.3 * (1.0 - 0.35 * s01);
                let drag = (t * 0.55 + ph - s01 * 4.2).cos() * 0.8 * sz;
                let x0 = x - stem - lobe_l + drag.min(0.0);
                let x1 = x + stem + lobe_r + drag.max(0.0);
                let col = js_round(x).clamp(0.0, (W - 1) as f64) as usize;
                let ray =
                    f64::from(self.rays[self.ray_at[r * W + col] as usize]) * (-rf / 30.0).exp();
                let lit = smooth(0.3, 0.6, ray);
                let (xa, xb) = (js_round(x0) as i64, js_round(x1) as i64);
                for xx in xa..=xb {
                    if xx < 0 || xx >= W as i64 {
                        continue;
                    }
                    let xf = xx as f64;
                    let k = r * W + xx as usize;
                    // backlit: dark through the middle, the edges glowing, the sun-facing
                    // edge most of all and gold where a shaft catches it
                    let sun = if side > 0.0 { xx == xb } else { xx == xa };
                    let rim = xx == xa || xx == xb;
                    let blade = if (xf - x).abs() > stem { 1.0 } else { 0.0 };
                    // the stipe is black, the blades let a little olive light through
                    let through = blade
                        * 0.12
                        * smooth(0.0, 1.0, (xf - x).abs() - stem)
                        * (0.5 + 0.5 * hash(xf * 7.0, rf * 3.0));
                    let g = 0.02
                        + through
                        + if rim { 0.16 } else { 0.0 }
                        + if sun {
                            0.34 + 0.66 * lit * (0.4 + 0.6 * blade)
                        } else {
                            0.0
                        };
                    let rgb = [
                        mix(0.62 * g, f64::from(self.wr[k]), haze),
                        mix(0.58 * g, f64::from(self.wg[k]), haze),
                        mix(0.18 * g, f64::from(self.wb[k]), haze),
                    ];
                    self.put(xf, rf, rgb, 1.0);
                    self.floor[k] = 0.02;
                }
            }
        }

        // bubbles: wobbling up to the surface, growing as they rise
        for i in 0..self.bubbles.len() {
            let Bubble { x0, y0, ph, sp, w } = self.bubbles[i];
            let span = y0 - SURF;
            let p = ((t * sp) / span + ph) % 1.0;
            let y = y0 - p * span;
            let x = x0 + (y * 0.35 + w).sin() * 1.2 + p * 3.0;
            let v = 0.55 + 0.45 * p;
            self.add(js_round(x), js_round(y), [0.7 * v, 0.95 * v, 1.0 * v]);
        }
        // specks drifting in the water
        let (wf, hf) = (W as f64, H as f64);
        for i in 0..self.snow.len() {
            let Speck { x, y, sp, ph, b } = self.snow[i];
            let x = (((x + t * sp + 2.0 * (t * 0.3 + ph).sin()) % wf) + wf) % wf;
            let y = (((y + t * sp * 0.4) % hf) + hf) % hf;
            self.add(x.floor(), y.floor(), [b * 0.7, b * 0.9, b]);
        }

        for r in 0..H {
            for xi in 0..W {
                let k = r * W + xi;
                let rgb = [
                    f64::from(self.cr[k]),
                    f64::from(self.cg[k]),
                    f64::from(self.cb[k]),
                ];
                let floor = f64::from(self.floor[k]);
                let peak = rgb[0].max(rgb[1]).max(rgb[2]).max(1e-4);
                let level = clamp(floor + (1.0 - floor) * peak.powf(0.85) * 0.95);
                let step = Dots::step(level, bayer(r, xi));
                out[k] = self.dots.ink(step, level, rgb, peak);
            }
        }
    }
}
