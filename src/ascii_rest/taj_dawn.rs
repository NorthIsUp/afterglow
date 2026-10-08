//! taj dawn: the Taj Mahal at first light, seen down its long reflecting canal
//! between rows of cypress. The sun has just cleared the red sandstone mosque
//! on the left; the haze and the clouds drift, the canal's reflection ripples,
//! light glints along its far end, and a few birds cross.
//!
//! Its tail is its own: a per-cell jittered dither, a tone shoulder and a
//! capped sky lift, so it uses [`Dots::nearest`] rather than [`Dots::ink`].
//!
//! `taj-dawn-wide` is the same dawn on a 3.2:1 canvas that takes in the whole
//! garden front: the mosque on the left and its twin, the jawab, answering it
//! on the right.

use std::f64::consts::PI;

use super::halftone::{Dots, BAYER};
use super::math::{clamp, fbm, hash, js_round, mix, noise, smooth};
use super::{Fit, Piece, hex};
use crate::grid::Cell;

const H: usize = 100;
const BASE: f64 = 68.0; // where the Taj meets the garden, and the far end of the canal
const HZ: f64 = 60.0; // eye level
const S: f64 = 0.7; // cells per metre on the Taj
const MB: f64 = 67.0; // and its foot
const KR: f64 = 1.55; // the reflection is foreshortened so the dome reaches the canal
/// Cloud strip, wrapping at `CW` columns, `CH` rows deep.
const CW: usize = 400;
const CH: usize = 50;
/// Haze strip width.
const HWD: usize = 400;

const SKY: u8 = 0;
const BGTREE: u8 = 1;
const MARBLE: u8 = 2;
const LAWN: u8 = 3;
const WALK: u8 = 4;
const POOL: u8 = 5;
const CYPRESS: u8 = 6;
const MOSQUE: u8 = 7;

struct Tree {
    x: f64,
    base: f64,
    h: f64,
    w: f64,
    s: f64,
}

/// Where things sit in the frame: upstream's, or the `-wide` recomposition's.
struct Layout {
    name: &'static str,
    w: usize,
    /// The Taj's axis, and the canal's vanishing point.
    cx: f64,
    sun: [f64; 2],
    /// The mosque's axis.
    mx: f64,
    /// The jawab's axis, the mosque's mirror across the canal.
    jawab: Option<f64>,
    /// Where the sky cools and dims away from the sun.
    far: [f64; 2],
    /// Dawn cloud banks: [x, y, half-width, height above, depth below, strength].
    banks: [[f64; 6]; 5],
    /// Where the birds start, and the span they wrap over.
    birds: [f64; 2],
    /// The last stars, in the sky away from the sun: from this column, fading in
    /// to the next.
    stars: [f64; 2],
}

const ORIGINAL: Layout = Layout {
    name: "taj-dawn",
    w: 200,
    cx: 128.0,
    sun: [36.0, 40.0],
    mx: 45.0,
    jawab: None,
    far: [50.0, 200.0],
    banks: [
        [76.0, 28.0, 40.0, 7.0, 3.5, 1.4],  // over the sun's right shoulder
        [152.0, 36.0, 52.0, 9.0, 4.0, 1.0], // behind the dome
        [16.0, 13.0, 32.0, 4.0, 2.5, 0.8],  // a high wisp
        [250.0, 24.0, 40.0, 8.0, 4.0, 1.0],
        [330.0, 33.0, 36.0, 7.0, 4.0, 0.95],
    ],
    birds: [58.0, 260.0],
    stars: [96.0, 140.0],
};

const WIDE: Layout = Layout {
    name: "taj-dawn-wide",
    w: 320,
    cx: 176.0,
    sun: [84.0, 40.0],
    mx: 93.0,
    jawab: Some(2.0 * 176.0 - 93.0),
    far: [98.0, 248.0],
    banks: [
        [124.0, 28.0, 40.0, 7.0, 3.5, 1.4],
        [200.0, 36.0, 52.0, 9.0, 4.0, 1.0],
        [64.0, 13.0, 32.0, 4.0, 2.5, 0.8],
        [298.0, 24.0, 40.0, 8.0, 4.0, 1.0],
        [378.0, 33.0, 36.0, 7.0, 4.0, 0.95],
    ],
    birds: [106.0, 380.0],
    stars: [144.0, 188.0],
};

pub type TajDawn = Scene<false>;
pub type TajDawnWide = Scene<true>;

pub struct Scene<const IS_WIDE: bool> {
    dots: Dots,
    mat: Vec<u8>,
    r: Vec<f32>,
    g: Vec<f32>,
    b: Vec<f32>,
    /// The reflection source: everything above the garden, trees included.
    rr: Vec<f32>,
    rg: Vec<f32>,
    rb: Vec<f32>,
    floor: Vec<f32>,
    /// How much drifting haze each cell takes.
    haze_w: Vec<f32>,
    /// How much of the sun's glow a sky cell holds, for its breathing.
    glow: Vec<f32>,
    cloud: Vec<f32>,
    cloud_lit: Vec<f32>,
    haze1: Vec<f32>,
    haze2: Vec<f32>,
    haze_row: Vec<f32>,
    birds: [[f64; 3]; 3],
    dith: Vec<f32>,
    jit: Vec<f32>,
    tone: Vec<f32>,
}

/// The onion dome's profile: swelling out past the drum, then drawn to a point.
fn onion(h: f64) -> f64 {
    if h < 0.3 {
        0.8 + 0.2 * ((h / 0.3) * PI * 0.5).sin()
    } else {
        (((h - 0.3) / 0.7) * PI * 0.5).cos().powf(1.45)
    }
}

/// Pointed arch: half-width at height h above the springline, for an arch of
/// half-width w; zero above the apex.
fn arch(w: f64, h: f64) -> f64 {
    if h <= 0.0 {
        return w;
    }
    let c = w * 0.5;
    let r = w + c;
    let q = r * r - h * h;
    if q > 0.0 {
        (q.sqrt() - c).max(0.0)
    } else {
        0.0
    }
}

/// The Taj in metres: X across from the axis, Y up from the garden. Returns
/// (lit, recess) for marble, or None for air. lit runs 0 (shadow, facing away
/// from the sun) to 1 (facing it); recess darkens arches and niches.
fn taj(x: f64, y: f64) -> Option<(f64, f64)> {
    let ax = x.abs();
    let left = if x < 0.0 { 1.0 } else { -1.0 }; // +1 on the sun's side
                                                 // finial
    if (72.0..80.5).contains(&y)
        && ax
            <= 0.75
                + if (y - 74.5).abs() < 0.9 { 0.6 } else { 0.0 }
                + if (y - 77.0).abs() < 0.7 { 0.4 } else { 0.0 }
    {
        return Some((0.75 + 0.2 * left, 0.0));
    }
    // the great dome
    if (47.5..72.0).contains(&y) {
        let h = (y - 47.5) / 24.5;
        let hw = 15.0 * onion(h);
        if ax <= hw {
            // a sphere lit from the left and a little above
            let nx = x / (hw + 0.01);
            let nz = (1.0 - nx * nx).max(0.0).sqrt();
            return Some((clamp(0.3 + 0.5 * (-0.8 * nx + 0.4 * nz) + 0.2 * h), 0.0));
        }
    }
    // the four chhatris on the roof, two in view
    let cx = ax - 17.5;
    let sx = if x < 0.0 { -cx } else { cx };
    if cx.abs() <= 4.4 && (39.5..55.5).contains(&y) {
        let nx = sx / 4.4;
        if y < 41.0 {
            return Some((clamp(0.5 - 0.4 * nx * left), 0.0));
        }
        if y < 46.5 {
            if cx.abs() > 3.6 {
                return None;
            }
            let open = cx.abs() < 2.6 && cx.abs() > 0.6 && y < 45.5;
            return Some((
                clamp(0.5 - 0.45 * sx / 4.0 * left),
                if open { 0.75 } else { 0.0 },
            ));
        }
        if y < 47.3 {
            return Some((clamp(0.55 - 0.4 * nx * left), 0.0));
        }
        let h = (y - 47.3) / 6.5;
        if h < 1.0 && cx.abs() <= 4.0 * onion(h) {
            return Some((
                clamp(0.55 - 0.6 * (sx / (4.0 * onion(h) + 0.01)) * left + 0.1 * h),
                0.0,
            ));
        }
        if h >= 1.0 && cx.abs() < 0.6 {
            return Some((0.6, 0.0));
        }
    }
    // the drum under the dome
    if (40.0..47.5).contains(&y) && ax <= 12.2 {
        let nx = x / 12.2;
        let band = if y > 45.8 { 0.15 } else { 0.0 };
        return Some((clamp(0.45 - 0.55 * nx + band), 0.0));
    }
    // slender pinnacles at the corners of the portal and of the building
    if ((ax - 8.8).abs() < 0.75 && (40.0..47.5).contains(&y))
        || ((ax - 28.2).abs() < 0.75 && (38.0..44.5).contains(&y))
    {
        return Some((0.55 + 0.25 * left, 0.0));
    }
    // the main building
    if ax <= 28.5 && (7.0..40.0).contains(&y) {
        // the portal rises a little above the parapet
        if y >= 38.5 && ax > 9.0 && ((ax + 0.5) / 1.5).floor() as i64 & 1 != 0 {
            return None;
        }
        if ax <= 9.0 {
            // central portal: a calligraphy band round a deep pointed arch
            let hw = arch(6.0, y - 25.0);
            if ax <= hw && y < 25.0 + 9.0 {
                let door = ax <= 3.2 && ax <= arch(3.2, y - 15.5);
                return Some((
                    0.35 + 0.15 * left,
                    if door {
                        0.82
                    } else {
                        0.6 + 0.12 * (1.0 - smooth(25.0, 33.0, y))
                    },
                ));
            }
            if ax <= hw + 0.8 && y < 25.0 + 10.2 {
                return Some((0.42, 0.35));
            }
            if ax > 7.6 && ax <= 9.0 && y < 40.0 {
                return Some((0.5 + 0.12 * left, 0.08));
            }
            return Some((0.5 + 0.08 * left, 0.0));
        }
        if ax <= 21.0 {
            // front face either side of the portal: two storeys of arched niches
            let nx = ax - 15.0;
            for (y0, y1) in [(9.0, 21.5), (24.5, 37.0)] {
                if y >= y0 && y < y1 && nx.abs() <= arch(3.6, y - (y1 - 4.5)) {
                    return Some((0.4 + 0.1 * left, 0.5));
                }
                if y >= y0 - 0.6
                    && y < y1 + 0.6
                    && nx.abs() <= arch(4.3, y - (y1 - 4.2))
                    && nx.abs() > 3.6
                {
                    return Some((0.45, 0.22));
                }
            }
            return Some((0.5 + 0.1 * left, 0.0));
        }
        // chamfered corners, turned toward the sun on the left and away on the right
        let lit = 0.5 + 0.48 * left;
        let nx = ax - 24.7;
        for (y0, y1) in [(9.0, 21.5), (24.5, 37.0)] {
            if y >= y0 && y < y1 && nx.abs() <= arch(2.2, y - (y1 - 3.0)) {
                return Some((lit * 0.7, 0.45));
            }
        }
        return Some((lit, 0.0));
    }
    // minarets at the corners of the plinth, tapering, with three galleries
    let mx = ax - 44.0;
    if (7.0..57.0).contains(&y) {
        let s = if x < 0.0 { -mx } else { mx }; // across the shaft, toward the sun negative
        let hw = 2.9 - 0.6 * (y - 7.0) / 40.0;
        for g in [19.5, 32.0, 44.5] {
            if y >= g && y < g + 1.4 && mx.abs() <= hw + 1.1 {
                return Some((
                    clamp(0.5 - 0.42 * s / (hw + 1.1) * left),
                    if y < g + 0.5 { 0.35 } else { 0.0 },
                ));
            }
        }
        if y < 46.0 && mx.abs() <= hw {
            return Some((clamp(0.5 - 0.48 * (s / hw) * left), 0.0));
        }
        if (45.9..49.5).contains(&y) && mx.abs() <= 2.2 {
            return Some((
                clamp(0.5 - 0.4 * s / 2.2 * left),
                if mx.abs() < 1.4 && mx.abs() > 0.3 {
                    0.7
                } else {
                    0.0
                },
            ));
        }
        if y >= 49.5 {
            let h = (y - 49.5) / 4.5;
            if h < 1.0 && mx.abs() <= 2.5 * onion(h) {
                return Some((clamp(0.55 - 0.55 * s / (2.5 * onion(h) + 0.01) * left), 0.0));
            }
            if h >= 1.0 && y < 56.0 && mx.abs() < 0.5 {
                return Some((0.6, 0.0));
            }
        }
    }
    // the plinth, with a row of shallow niches
    if ax <= 47.5 && (0.0..7.0).contains(&y) {
        if y > 6.2 {
            return Some((0.62, 0.0));
        }
        let k = (ax % 5.2) - 2.6;
        if y > 1.5 && y < 5.2 && k.abs() < 1.1 {
            return Some((0.42, 0.3));
        }
        return Some((0.52, 0.0));
    }
    None
}

impl<const IS_WIDE: bool> Scene<IS_WIDE> {
    const L: Layout = if IS_WIDE { WIDE } else { ORIGINAL };
    const W: usize = Self::L.w;

    /// The red sandstone mosque that flanks the Taj, in cells: three domes over a
    /// five-bay front with a tall central portal. Returns 0 for wall, 1 for dome,
    /// 2 for a recess, or -1 for air. It stands against the sun, so it is mostly
    /// silhouette.
    fn mosque(xc: f64, y: f64) -> i8 {
        let dx = xc - Self::L.mx;
        let ax = dx.abs();
        let odd = |v: f64| v.floor() as i64 & 1 != 0;
        // plinth
        if (MB - 2.5..MB).contains(&y) && ax <= 22.0 {
            return 0;
        }
        // end towers, each with a small kiosk on top
        let tx = (ax - 19.5).abs();
        if tx <= 1.3 && (MB - 13.0..MB - 2.5).contains(&y) {
            return 0;
        }
        if (MB - 15.5..MB - 13.0).contains(&y) {
            let h = (MB - 13.0 - y) / 2.5;
            if tx <= 1.8 * onion(h) {
                return 1;
            }
        }
        if tx < 0.35 && (MB - 16.5..MB - 15.5).contains(&y) {
            return 1;
        }
        // central portal, rising above the front
        if ax <= 5.5 && (MB - 15.0..MB - 2.5).contains(&y) {
            if y < MB - 14.3 && odd(xc) {
                return -1;
            }
            if ax <= arch(3.4, MB - 9.5 - y) && y >= MB - 13.5 {
                return 2;
            }
            return 0;
        }
        // the five-bay front and its parapet
        if ax <= 18.0 && (MB - 10.0..MB - 2.5).contains(&y) {
            let bay = ((ax - 5.5) % 4.2) - 2.1;
            if ax > 6.0 && y >= MB - 8.5 && bay.abs() <= arch(1.4, MB - 6.2 - y) {
                return 2;
            }
            return 0;
        }
        if ax <= 18.0 && (MB - 10.8..MB - 10.0).contains(&y) && odd(xc * 0.75) {
            return 0;
        }
        // side domes on drums
        let sx = (ax - 11.5).abs();
        if sx <= 2.6 && (MB - 12.0..MB - 10.0).contains(&y) {
            return 0;
        }
        if (MB - 17.0..MB - 12.0).contains(&y) {
            let h = (MB - 12.0 - y) / 5.0;
            if sx <= 3.6 * onion(h) {
                return 1;
            }
        }
        if sx < 0.35 && (MB - 18.5..MB - 17.0).contains(&y) {
            return 1;
        }
        // the great central dome
        if ax <= 4.0 && (MB - 16.5..MB - 15.0).contains(&y) {
            return 0;
        }
        if (MB - 23.5..MB - 16.5).contains(&y) {
            let h = (MB - 16.5 - y) / 7.0;
            if ax <= 5.2 * onion(h) {
                return 1;
            }
        }
        if ax < 0.4 && (MB - 25.5..MB - 23.5).contains(&y) {
            return 1;
        }
        -1
    }

    fn sun_glow(x: f64, y: f64) -> f64 {
        let dx = x - Self::L.sun[0];
        let dy = (y - Self::L.sun[1]) * 1.25;
        let d = (dx * dx + dy * dy).sqrt();
        (-d / 4.0).exp() * 0.9 + (-d / 13.0).exp() * 0.42 + (-d / 40.0).exp() * 0.3
    }

    fn sky_at(x: f64, y: f64) -> [f64; 3] {
        let v = clamp(y / HZ);
        // deep violet overhead, mauve, then rose and peach toward the horizon
        let up = smooth(0.05, 0.8, v);
        let (mut r, mut g, mut b) = (
            mix(0.055, 0.3, up),
            mix(0.05, 0.19, up),
            mix(0.17, 0.36, up),
        );
        let low = smooth(0.62, 1.0, v);
        (r, g, b) = (mix(r, 0.66, low), mix(g, 0.4, low), mix(b, 0.42, low));
        // cooler and dimmer on the side away from the sun
        let far = smooth(Self::L.far[0], Self::L.far[1], x) * 0.18;
        r *= 1.0 - far;
        g *= 1.0 - far * 0.8;
        b *= 1.0 - far * 0.3;
        let glow = Self::sun_glow(x, y)
            + (-(y - 57.0).abs() / 5.0).exp() * 0.25 * (-(x - Self::L.sun[0]).abs() / 50.0).exp();
        r += glow * 1.0;
        g += glow * 0.78;
        b += glow * 0.48;
        // faint shafts of light fanning up from the sun through the haze
        let ang = (y - Self::L.sun[1]).atan2(x - Self::L.sun[0]);
        let ray = fbm(ang * 9.0 + 3.0, 1.7, 2, 0.0);
        let rd = (x - Self::L.sun[0]).hypot(y - Self::L.sun[1]);
        let shaft = smooth(0.5, 0.75, ray) * (-rd / 45.0).exp() * smooth(4.0, 14.0, rd) * 0.12;
        r += shaft;
        g += shaft * 0.8;
        b += shaft * 0.55;
        [r, g, b]
    }

    /// Dawn cloud in a wrapping strip that drifts: broken noise, gathered into a
    /// bank up and right of the sun and a lower one behind the dome.
    fn cdens(x: f64, y: f64) -> f64 {
        let cw = CW as f64;
        let q = fbm(x * 0.01, y * 0.04, 2, cw * 0.01);
        let n = fbm(x * 0.025 + q * 1.6, y * 0.075 + q * 0.6, 5, cw * 0.025);
        let mut m = 0.0;
        for [bx, by, rx, up, down, a] in Self::L.banks {
            let mut dx = x - bx;
            dx -= js_round(dx / cw) * cw;
            let ex = dx / rx;
            let ey = (y - by) / if y < by { up } else { down };
            let e = (-ex * ex * ex * ex - ey * ey).exp() * a;
            if e > m {
                m = e;
            }
        }
        0.62 * m + 1.4 * (n - 0.5) + 0.02
    }
}

impl<const IS_WIDE: bool> Piece for Scene<IS_WIDE> {
    const NAME: &'static str = Self::L.name;
    const COLS: usize = Self::W;
    const ROWS: usize = H;
    const FPS: u32 = 15;
    const CELL: usize = 1;
    const FIT: Fit = Fit::Cover { anchor: 0.25 };
    const GROUND: u32 = hex("#0d0a13");
    #[rustfmt::skip]
    const PALETTE: &'static [u32] = &[
        hex("#2e2a55"), hex("#3d3870"), hex("#514a8a"), hex("#6a62a3"), hex("#8a83bd"),
        hex("#5a3a5e"), hex("#7a4d72"), hex("#9a6284"), hex("#b97a92"), hex("#d495a2"),
        hex("#b8705a"), hex("#d98d66"), hex("#efab78"), hex("#f8c98e"), hex("#ffe2ae"), hex("#fff3d8"), hex("#fffcf2"),
        hex("#8e7f9e"), hex("#ad9cb4"), hex("#cbb6c4"), hex("#e3cdd2"), hex("#f3e0dc"),
        hex("#2a4a3e"), hex("#36584a"), hex("#4a6c58"), hex("#66845f"), hex("#8a9a68"),
        hex("#8a4a3e"), hex("#b0624a"), hex("#3a2f4a"), hex("#4e3d5a"), hex("#f6d6c0"), hex("#e9bfae"),
        hex("#5e6a40"), hex("#7d8a52"), hex("#a3a064"), hex("#c2b274"),
    ];

    fn new() -> Self {
        let n = Self::W * H;
        let hf = H as f64;
        let mut mat = vec![SKY; n];
        let (mut rv, mut gv, mut bv) = (vec![0f32; n], vec![0f32; n], vec![0f32; n]);
        let mut floor = vec![0.12f32; n];
        let mut haze_w = vec![0f32; n];
        let mut glow = vec![0f32; n];

        let cwf = CW as f64;
        let mut cloud = vec![0f32; CW * CH];
        let mut cloud_lit = vec![0f32; CW * CH];
        for r in 0..CH {
            for x in 0..CW {
                let (xf, y) = (x as f64, r as f64 + 0.5);
                let d = Self::cdens(xf, y);
                cloud[r * CW + x] = smooth(0.46, 0.66, d) as f32;
                // the side toward the sun (down and left) catches the light
                let toward = Self::cdens(xf - 2.0, y + 2.5);
                cloud_lit[r * CW + x] = clamp(
                    0.56 + (d - toward) * 0.75 - (d - 0.6) * 0.2
                        + 0.7 * (fbm(xf * 0.09, y * 0.2, 2, cwf * 0.09) - 0.5),
                ) as f32;
            }
        }

        // the far tree line: rounded crowns, lower behind the Taj
        let mut crowns: Vec<[f64; 3]> = Vec::new();
        let (mut x, mut i) = (-12.0, 0.0);
        while x < Self::W as f64 + 12.0 {
            let rad = 3.0 + hash(i, 61.0) * 4.5;
            crowns.push([
                x,
                60.5 - hash(i, 62.0) * 2.5 - 2.5 * fbm(x * 0.03, 2.0, 2, 0.0) + rad * 0.35,
                rad,
            ]);
            x += 4.0 + hash(i, 60.0) * 5.0;
            i += 1.0;
        }
        let mut tops = vec![0f32; Self::W];
        for (x, top) in tops.iter_mut().enumerate() {
            let xf = x as f64;
            let mut t0: f64 = 64.0;
            for &[cx, cy, rad] in &crowns {
                let dx = xf + 0.5 - cx;
                if dx.abs() < rad {
                    t0 = t0.min(cy - (rad * rad - dx * dx).sqrt() * 0.75);
                }
            }
            *top = (t0 - 0.6 * fbm(xf * 0.5, 7.0, 2, 0.0)) as f32;
        }
        let tree_top = |x: f64| f64::from(tops[(x.floor().max(0.0) as usize).min(Self::W - 1)]);
        let pool_half = |y: f64| 0.5 * (y - HZ) + 1.0;
        let walk_half = |y: f64| 0.7 * (y - HZ) + 1.5;

        // cypress rows either side of the canal, near ones last
        let mut trees: Vec<Tree> = Vec::new();
        for s in [8.5, 10.4, 13.0, 16.8, 22.5, 31.0] {
            for side in [-1.0, 1.0] {
                // an inner row along the walks, and a lower outer row further back
                for (off, tall) in [(4.3, 0.8), (2.45, 1.0)] {
                    if off > 3.0 && (s > 20.0 || side > 0.0) {
                        continue;
                    }
                    if side < 0.0 && s > 30.0 {
                        continue; // leave the sun clear
                    }
                    let cxp = Self::L.cx + side * off * s;
                    trees.push(Tree {
                        x: cxp + (hash(s * 10.0, side + off) - 0.5) * 0.6,
                        base: HZ + s,
                        h: 1.6 * tall * s * (0.94 + 0.12 * hash(s * 7.0, side + 3.0 + off)),
                        w: 0.26 * s,
                        s,
                    });
                }
            }
        }
        // long shadows of the cypresses, cast toward us and to the right
        let shadow_at = |xc: f64, y: f64| {
            let mut lit = 1.0;
            for tr in &trees {
                let (dx, dy) = (xc - tr.x, y - tr.base);
                if dy < -0.5 {
                    continue;
                }
                let along = (dx * 0.6 + dy * 0.8) / tr.s;
                let across = (dx * 0.8 - dy * 0.6) / tr.s;
                if along > 0.0 && along < 3.0 && across.abs() < 0.17 * (1.0 - along / 3.4) + 0.03 {
                    lit *= 0.08 + 0.92 * smooth(1.6, 3.0, along);
                }
            }
            lit
        };

        // the mosque's silhouette, and its edges toward the sun
        let mut mq = vec![-1i8; n];
        for r in (MB - 27.0) as usize..MB as usize {
            for x in (Self::L.mx - 24.0) as usize..=(Self::L.mx + 24.0) as usize {
                mq[r * Self::W + x] = Self::mosque(x as f64 + 0.5, r as f64 + 0.5);
            }
            if let Some(jx) = Self::L.jawab {
                for x in (jx - 24.0) as usize..=(jx + 24.0) as usize {
                    mq[r * Self::W + x] = Self::mosque(x as f64 + 0.5 - (jx - Self::L.mx), r as f64 + 0.5);
                }
            }
        }

        for r in 0..H {
            for x in 0..Self::W {
                let k = r * Self::W + x;
                let (y, xc) = (r as f64 + 0.5, x as f64 + 0.5);
                let mut m = SKY;
                let (mut cr, mut cg, mut cb) = (0.0, 0.0, 0.0);
                let mut fl;
                let mut hz = 0.0;
                let tt = tree_top(xc);
                if y < BASE && y >= tt {
                    m = BGTREE;
                } else if y >= BASE {
                    let dx = (xc - Self::L.cx).abs();
                    m = if dx < pool_half(y) {
                        POOL
                    } else if dx < walk_half(y) {
                        WALK
                    } else {
                        LAWN
                    };
                }
                if m == SKY {
                    [cr, cg, cb] = Self::sky_at(xc, y);
                    // never one flat tone: a faint unevenness in the dawn air
                    let veil = 0.88 + 0.24 * fbm(xc * 0.05, y * 0.09, 3, 0.0);
                    cr *= veil;
                    cg *= veil;
                    cb *= veil;
                    glow[k] = Self::sun_glow(xc, y) as f32;
                    hz = 0.5 + 0.5 * smooth(20.0, 58.0, y);
                    fl = 0.05 + 0.13 * smooth(18.0, 42.0, y);
                } else if m == BGTREE {
                    // distant trees, flattened by the haze; rimmed where the sun is close
                    let tex = fbm(xc * 0.25, y * 0.3, 3, 0.0);
                    let depth = smooth(tt, BASE, y);
                    let sky = Self::sky_at(xc, tt);
                    let a = 0.46 - 0.12 * depth + 0.12 * tex;
                    (cr, cg, cb) = (
                        mix(0.1, sky[0], a),
                        mix(0.09, sky[1], a),
                        mix(0.15, sky[2], a),
                    );
                    let rim = smooth(tt + 1.8, tt, y) * (-(xc - Self::L.sun[0]).abs() / 18.0).exp();
                    cr += 0.4 * rim;
                    cg += 0.27 * rim;
                    cb += 0.14 * rim;
                    // their feet lost in a bank of ground mist, broken into soft patches
                    let patch =
                        0.45 + 0.75 * smooth(0.3, 0.7, fbm(xc * 0.035 + 11.0, y * 0.18, 3, 0.0));
                    let mist = smooth(tt + 1.0, BASE + 1.0, y) * 0.75 * patch.min(1.0);
                    let mw = (-(xc - Self::L.sun[0]).abs() / 90.0).exp();
                    cr = mix(cr, 0.62 + 0.25 * mw, mist);
                    cg = mix(cg, 0.46 + 0.16 * mw, mist);
                    cb = mix(cb, 0.55 + 0.02 * mw, mist);
                    fl = 0.1;
                    hz = 0.8;
                } else if m == LAWN {
                    let v = (y - BASE) / (hf - BASE);
                    let u = (xc - Self::L.cx) / (y - HZ); // across the ground plane
                    let tex = fbm(u * 3.0, 40.0 / (y - HZ), 3, 0.0);
                    // mown bands that run toward the Taj
                    let stripe = if (u * 1.4).floor() as i64 & 1 != 0 {
                        1.0
                    } else {
                        0.55
                    };
                    // low sun raking across the grass from the left: gold where it
                    // lands, violet sky-light in the shade
                    let mut lit = (0.6 + 0.4 * (-(xc - Self::L.sun[0]).abs() / 60.0).exp())
                        * stripe
                        * (0.85 + 0.3 * tex);
                    lit *= shadow_at(xc, y);
                    let vig = 1.0
                        - 0.45
                            * smooth(0.5, 1.0, v)
                            * (0.6 + 0.4 * smooth(40.0, 0.0, xc.min(Self::W as f64 - xc)));
                    let fall = 1.0 - 0.35 * v;
                    // sunlit grass turns from gold near the sun to rose further off
                    let warm = (-(xc - Self::L.sun[0]).abs() / 80.0).exp();
                    // the shade holds the violet of the sky overhead, brighter in the open
                    let amb = 0.75 + 0.5 * stripe - 0.25 * v;
                    cr = (0.12 * amb + 0.56 * lit * fall) * vig;
                    cg = (0.09 * amb + (0.36 + 0.14 * warm) * lit * fall) * vig;
                    cb = (0.24 * amb + (0.26 - 0.1 * warm) * lit * fall) * vig;
                    // the far lawn sits in the haze
                    let far = smooth(BASE + 10.0, BASE, y) * 0.7;
                    let mw = (-(xc - Self::L.sun[0]).abs() / 90.0).exp();
                    cr = mix(cr, 0.62 + 0.25 * mw, far);
                    cg = mix(cg, 0.46 + 0.16 * mw, far);
                    cb = mix(cb, 0.55 + 0.02 * mw, far);
                    fl = 0.1;
                    hz = 0.7 * (1.0 - v);
                } else if m == WALK {
                    let v = (y - BASE) / (hf - BASE);
                    let lit = if xc < Self::L.cx { 1.0 } else { 0.86 };
                    // paving joints, closer together into the distance
                    let joint = (if (90.0 / (y - HZ)) % 1.0 < 0.14 {
                        0.72
                    } else {
                        1.0
                    }) * (0.5 + 0.5 * shadow_at(xc, y));
                    cr = (0.78 - 0.22 * v) * lit * joint;
                    cg = (0.57 - 0.16 * v) * lit * joint;
                    cb = (0.53 - 0.12 * v) * lit * joint;
                    fl = 0.15;
                    hz = 0.3;
                } else {
                    // the pool: shaded each frame from the reflection source
                    fl = 0.15;
                }
                // the mosque, dark against the sun, its edges caught by it
                let q = mq[k];
                if q >= 0 {
                    m = MOSQUE;
                    let sg = Self::sun_glow(xc, y);
                    let mut a = if q == 1 {
                        [0.2, 0.11, 0.19]
                    } else {
                        [0.31, 0.13, 0.15]
                    };
                    if q == 2 {
                        a = [0.07, 0.04, 0.07];
                    }
                    let air = 0.06 + 0.3 * smooth(MB - 8.0, MB, y); // the foot sinks into the mist
                    (cr, cg, cb) = (
                        mix(a[0], 0.62, air),
                        mix(a[1], 0.44, air),
                        mix(a[2], 0.5, air),
                    );
                    // rim light where the cell faces open sky toward the sun
                    let lft = if x > 0 { mq[k - 1] } else { -1 };
                    let up = if r > 0 { mq[k - Self::W] } else { -1 };
                    let toward = if xc < Self::L.sun[0] {
                        if x < Self::W - 1 {
                            mq[k + 1]
                        } else {
                            -1
                        }
                    } else {
                        lft
                    };
                    let mut rim = (if up < 0 { 0.75 } else { 0.0 })
                        + (if toward < 0 { 0.9 } else { 0.0 })
                        + (if lft < 0 && up < 0 { 0.2 } else { 0.0 });
                    rim = f64::min(1.0, rim) * f64::min(1.0, 0.25 + sg * 1.6);
                    cr = mix(cr, 1.0, rim * 0.8);
                    cg = mix(cg, 0.7, rim * 0.8);
                    cb = mix(cb, 0.48, rim * 0.8);
                    fl = 0.04;
                    hz = 0.15;
                }
                // the Taj in front of the sky and the trees
                if y < BASE + 0.01 {
                    if let Some((lit, rec)) = taj((xc - Self::L.cx) / S, (BASE - y) / S) {
                        m = MARBLE;
                        // lavender shadow, pink-white front, gold where it faces the sun
                        const SH: [f64; 3] = [0.42, 0.36, 0.56];
                        const FR: [f64; 3] = [0.9, 0.76, 0.75];
                        const GD: [f64; 3] = [1.0, 0.88, 0.72];
                        let (a, b2, kk) = if lit < 0.5 {
                            (SH, FR, (lit * 2.0).powf(1.4))
                        } else {
                            (FR, GD, (lit - 0.5) * 2.0)
                        };
                        (cr, cg, cb) = (
                            mix(a[0], b2[0], kk),
                            mix(a[1], b2[1], kk),
                            mix(a[2], b2[2], kk),
                        );
                        if rec > 0.0 {
                            let d = 1.0 - rec * 0.78;
                            cr *= d * 0.95;
                            cg *= d * 0.92;
                            cb *= d;
                        }
                        // a little grain in the stone, and the morning haze over the
                        // lower storeys
                        let g = 0.94 + 0.08 * hash(x as f64 * 3.0 + 1.0, r as f64 * 5.0 + 2.0);
                        cr *= g;
                        cg *= g;
                        cb *= g;
                        let low = smooth(40.0, BASE, y) * 0.2;
                        let sky = Self::sky_at(xc, y);
                        cr = mix(cr, sky[0], low);
                        cg = mix(cg, sky[1], low);
                        cb = mix(cb, sky[2], low);
                        fl = 0.12;
                        hz = 0.55;
                    }
                }
                mat[k] = m;
                rv[k] = cr as f32;
                gv[k] = cg as f32;
                bv[k] = cb as f32;
                floor[k] = fl as f32;
                haze_w[k] = hz as f32;
            }
        }

        // cypresses, far to near: dark, with a warm rim on the side toward the sun
        for tr in &trees {
            let top = tr.base - tr.h;
            let sun_side = if tr.x < Self::L.cx { 0.9 } else { 0.6 };
            let r0 = top.floor().max(0.0) as usize;
            let r1 = (tr.base + 0.6).ceil().min(hf) as usize;
            for r in r0..r1 {
                let y = r as f64 + 0.5;
                let u = (tr.base - y) / tr.h; // 0 at the foot, 1 at the tip
                if u > 1.0 {
                    continue;
                }
                let p = if u < 0.06 {
                    0.18
                } else if u < 0.3 {
                    0.8 + 0.2 * (u / 0.3)
                } else {
                    ((1.0 - u) / 0.7).powf(0.8)
                };
                let x0 = (tr.x - tr.w - 2.0).floor().max(0.0) as usize;
                let x1 = (tr.x + tr.w + 2.0).ceil().min(Self::W as f64) as usize;
                for x in x0..x1 {
                    let xc = x as f64 + 0.5;
                    let nz = noise(xc * 0.9, y * 0.55 + tr.s, 0.0);
                    let hw = tr.w * p * (0.88 + 0.26 * nz) + 0.35;
                    let dx = (xc - tr.x) / hw;
                    if dx.abs() > 1.0 {
                        continue;
                    }
                    let k = r * Self::W + x;
                    let leaf = fbm(xc * 0.6, y * 0.35 + tr.s * 3.0, 2, 0.0);
                    let rim = smooth(-0.2, -0.9, dx)
                        * (0.4 + 0.6 * leaf)
                        * (0.5 + 0.5 * (-(tr.x - Self::L.sun[0]).abs() / 80.0).exp());
                    // clumps of foliage, each a little lit on top
                    let clump = smooth(0.45, 0.7, fbm(xc * 0.45, y * 0.3 + tr.s * 3.0, 3, 0.0));
                    let base = 0.5 + 0.5 * leaf + 0.5 * clump;
                    let dist = smooth(30.0, 8.0, tr.s); // far trees take more haze
                    let mut cr = 0.045 * base + sun_side * rim;
                    let mut cg = 0.1 * base + sun_side * 0.63 * rim;
                    let mut cb = 0.08 * base + sun_side * 0.27 * rim;
                    cr = mix(cr, 0.7, dist * 0.45);
                    cg = mix(cg, 0.5, dist * 0.45);
                    cb = mix(cb, 0.56, dist * 0.45);
                    mat[k] = CYPRESS;
                    rv[k] = cr as f32;
                    gv[k] = cg as f32;
                    bv[k] = cb as f32;
                    floor[k] = 0.05;
                    haze_w[k] =
                        ((0.3 + 0.4 * dist) * smooth(tr.base - tr.h * 0.7, tr.base, y)) as f32;
                }
            }
        }

        // drifting haze, two layers in a strip that wraps
        let hwd = HWD as f64;
        let mut haze1 = vec![0f32; HWD * H];
        let mut haze2 = vec![0f32; HWD * H];
        for r in 0..H {
            let rf = r as f64;
            for x in 0..HWD {
                let xf = x as f64;
                haze1[r * HWD + x] =
                    smooth(0.4, 0.75, fbm(xf * 0.018, rf * 0.09, 4, hwd * 0.018)) as f32;
                haze2[r * HWD + x] = smooth(
                    0.42,
                    0.8,
                    fbm(xf * 0.04 + 7.0, rf * 0.16 + 3.0, 3, hwd * 0.04),
                ) as f32;
            }
        }
        // haze is thickest just above the garden
        let haze_row = (0..H)
            .map(|r| (0.03 + 0.42 * (-(r as f64 + 0.5 - 63.0).abs() / 6.0).exp()) as f32)
            .collect();

        let birds = std::array::from_fn(|i| {
            let i = i as f64;
            [
                Self::L.birds[0] + i * 8.0 + hash(i, 40.0) * 4.0,
                30.0 - i * 2.5 + hash(i, 41.0) * 2.0,
                hash(i, 42.0) * 6.28,
            ]
        });
        // ordered dither, nudged per cell so its grid does not show in flat light
        let dith = (0..n)
            .map(|k| (BAYER[((k / Self::W) & 3) * 4 + ((k % Self::W) & 3)] * 0.85) as f32)
            .collect();
        let jit = (0..n)
            .map(|k| (hash(k as f64, 77.0) - 0.5) as f32)
            .collect();
        // a gentle shoulder so the brightest light keeps its gradient
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

        Self {
            dots: Dots::new(Self::PALETTE),
            mat,
            rr: rv.clone(),
            rg: gv.clone(),
            rb: bv.clone(),
            r: rv,
            g: gv,
            b: bv,
            floor,
            haze_w,
            glow,
            cloud,
            cloud_lit,
            haze1,
            haze2,
            haze_row,
            birds,
            dith,
            jit,
            tone,
        }
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        let (d1, d2) = (t * 1.1, t * 2.3);
        let cd = t * 0.5;
        let breathe = 0.035 * ((t / 6.0) * PI * 2.0).sin();
        let hf = H as f64;
        let hwd = HWD as f64;
        // where the birds are this frame
        let (mut bx, mut by, mut bu) = ([0.0; 3], [0.0; 3], [false; 3]);
        for (i, &[x0, y0, ph]) in self.birds.iter().enumerate() {
            bx[i] = ((x0 + t * 3.2) % Self::L.birds[1] + Self::L.birds[1]) % Self::L.birds[1] - 30.0;
            by[i] = y0 + (t * 0.4 + ph).sin() * 1.3;
            bu[i] = (t * 7.0 + ph).sin() > 0.0;
        }

        for r in 0..H {
            let rf = r as f64;
            let y = rf + 0.5;
            let hr = f64::from(self.haze_row[r]);
            for xi in 0..Self::W {
                let x = xi as f64;
                let k = r * Self::W + xi;
                let m = self.mat[k];
                let (mut cr, mut cg, mut cb);
                let mut fl = f64::from(self.floor[k]);
                let mut fade = 1.0;

                if m == POOL {
                    // the mirror: the scene above, flipped about the far end,
                    // shortened and shaken
                    let depth = (y - BASE) / (hf - BASE);
                    let wob = (y * 1.9 - t * 2.4 + (x * 0.11 + t * 0.7).sin() * 1.5).sin()
                        * (0.2 + 0.7 * depth);
                    let sx = js_round(x + wob).clamp(0.0, (Self::W - 1) as f64);
                    let flick = if (x * 0.3 + t * 1.3 + y).sin() > 0.6 {
                        1.0
                    } else {
                        0.0
                    };
                    let sr = js_round(BASE - 0.5 - (y - BASE) * KR - flick).clamp(0.0, BASE - 1.0);
                    let j = sr as usize * Self::W + sx as usize;
                    // stone reflects brighter than the sky does, so the Taj holds its shape
                    let src = if self.mat[j] == MARBLE { 1.0 } else { 0.85 };
                    let dim = src - 0.1 * depth;
                    cr = f64::from(self.rr[j]) * dim * 0.9;
                    cg = f64::from(self.rg[j]) * dim * 0.94;
                    cb = f64::from(self.rb[j]) * dim + 0.05;
                    // ripples catching the sky
                    let w = noise(x * 0.12 + t * 0.15, y * 0.9 - t * 0.9, 0.0);
                    let glint = smooth(0.75, 0.95, w) * 0.14;
                    cr += glint;
                    cg += glint * 0.85;
                    cb += glint * 0.75;
                    // a sparse line of light drifting across the far end
                    if rf > BASE && rf <= BASE + 4.0 {
                        let gl = smooth(
                            0.62,
                            0.85,
                            noise(x * 0.55 - t * 0.9, rf * 2.7 + t * 0.2, 0.0),
                        ) * (0.5 - 0.08 * (rf - BASE));
                        cr += gl;
                        cg += gl * 0.85;
                        cb += gl * 0.6;
                    }
                    // a thin line of light where the far edge meets the plinth
                    if rf == BASE {
                        cr += 0.2;
                        cg += 0.16;
                        cb += 0.12;
                    }
                    fade = 0.8 + 0.2 * smooth(hf + 2.0, hf - 6.0, y);
                } else {
                    cr = f64::from(self.r[k]);
                    cg = f64::from(self.g[k]);
                    cb = f64::from(self.b[k]);
                    if m == SKY {
                        let ga = f64::from(self.glow[k]) * breathe;
                        cr += ga;
                        cg += ga * 0.78;
                        cb += ga * 0.48;
                    }
                    if m == SKY && y < CH as f64 {
                        // drifting dawn cloud: gold and rose underneath near the sun,
                        // violet-grey elsewhere
                        let sx = x + cd;
                        let ix = sx.floor();
                        let fx = sx - ix;
                        let ix = ix as usize;
                        let i0 = r * CW + ix % CW;
                        let i1 = r * CW + (ix + 1) % CW;
                        let (c0, c1) = (f64::from(self.cloud[i0]), f64::from(self.cloud[i1]));
                        let c = c0 + (c1 - c0) * fx;
                        if c > 0.01 {
                            let (l0, l1) =
                                (f64::from(self.cloud_lit[i0]), f64::from(self.cloud_lit[i1]));
                            let l = l0 + (l1 - l0) * fx;
                            let near = f64::min(1.0, f64::from(self.glow[k]) * 1.7);
                            // a mauve body, rose underneath, gold where the sun is close
                            let b2 = clamp(0.2 + 0.6 * l + near * 0.55 * l);
                            let (kr, kg, kb) = if b2 < 0.5 {
                                let q = b2 * 2.0;
                                (mix(0.16, 0.6, q), mix(0.12, 0.34, q), mix(0.3, 0.5, q))
                            } else {
                                let q = (b2 - 0.5) * 2.0;
                                (mix(0.6, 1.05, q), mix(0.34, 0.8, q), mix(0.5, 0.55, q))
                            };
                            let a = f64::min(1.0, c) * 0.9;
                            cr = mix(cr, kr, a);
                            cg = mix(cg, kg, a);
                            cb = mix(cb, kb, a);
                        }
                    }
                    if m == SKY && r < 24 && xi > Self::L.stars[0] as usize && hash(x, rf * 3.0 + 11.0) > 0.988 {
                        // the last few stars, fading where the dawn reaches
                        let tw =
                            0.7 + 0.3 * (t * (1.2 + hash(x, rf) * 2.0) + hash(rf, x) * 6.28).sin();
                        let s = tw * 0.62 * smooth(24.0, 8.0, rf) * smooth(Self::L.stars[0], Self::L.stars[1], x);
                        cr = cr.max(s * 0.85);
                        cg = cg.max(s * 0.85);
                        cb = cb.max(s);
                    }
                    if m == SKY {
                        // the sun's disc, softened by the haze
                        let (dx, dy) = (x + 0.5 - Self::L.sun[0], y - Self::L.sun[1]);
                        let dd = dx * dx + dy * dy;
                        if dd < 22.0 {
                            let a = smooth(22.0, 9.0, dd);
                            cr = mix(cr, 1.05, a);
                            cg = mix(cg, 0.98, a);
                            cb = mix(cb, 0.86, a);
                        }
                        // birds, dark against the light
                        for i in 0..3 {
                            let ox = js_round(bx[i]) - x;
                            let oy = js_round(by[i]) - rf;
                            if !(-2.0..=2.0).contains(&ox) {
                                continue;
                            }
                            let ax = ox.abs();
                            let hit = if bu[i] {
                                (ax == 0.0 && oy == 0.0)
                                    || (ax == 1.0 && oy == 1.0)
                                    || (ax == 2.0 && oy == 1.0)
                            } else {
                                (oy == 0.0 && (ax == 0.0 || ax == 1.0)) || (ax == 2.0 && oy == -1.0)
                            };
                            if hit {
                                (cr, cg, cb, fl) = (0.16, 0.1, 0.17, 0.0);
                            }
                        }
                    }
                }

                // the morning haze, drifting
                let hw = f64::from(self.haze_w[k]);
                if hw > 0.0 || m == POOL {
                    let a = (if m == POOL { 0.12 } else { hw }) * hr;
                    let sx1 = (x + d1) % hwd;
                    let sx2 = (x + d2) % hwd;
                    let i1 = r * HWD + sx1 as usize;
                    let i2 = r * HWD + sx2 as usize;
                    let hz = a
                        * (0.35
                            + 0.75 * f64::from(self.haze1[i1])
                            + 0.45 * f64::from(self.haze2[i2]));
                    cr = mix(cr, 0.9, hz);
                    cg = mix(cg, 0.64, hz);
                    cb = mix(cb, 0.62, hz);
                }

                let tone =
                    |v: f64| f64::from(self.tone[((v * 256.0) as i32).clamp(0, 1024) as usize]);
                let (cr, cg, cb) = (tone(cr), tone(cg), tone(cb));
                let peak = cr.max(cg).max(cb).max(1e-4);
                let level = clamp(fl + (1.0 - fl) * peak.powf(1.1) * 1.02) * fade;
                let jit = f64::from(self.jit[k]) * if level < 0.3 { 0.12 } else { 0.3 };
                let step =
                    js_round(level * 3.0 + f64::from(self.dith[k]) + jit).clamp(0.0, 3.0) as usize;
                let want = Dots::want(step, level, 0.06);
                let mut s = (0.3 + 0.7 * want) / peak;
                if m == SKY && s > 1.6 {
                    s = 1.6;
                }
                out[k] = self.dots.dot(step, [cr, cg, cb], s);
            }
        }
    }
}
