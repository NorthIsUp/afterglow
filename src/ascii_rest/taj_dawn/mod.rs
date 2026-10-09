//! taj dawn: the Taj Mahal at first light, seen down its long reflecting canal
//! between rows of cypress. The sun has just cleared the red sandstone mosque
//! on the left; the haze and the clouds drift, the canal's reflection ripples,
//! light glints along its far end, and a few birds cross.
//!
//! Its tail is its own: a per-cell jittered dither, a tone shoulder and a
//! capped sky lift, so it uses [`Dots::nearest`] rather than [`Dots::ink`].
//!
//! At any size the Taj keeps the canal's vanishing point and the mosque its
//! left, with the sun just over the mosque. Wider panels take in more garden,
//! and from about 260 columns the jawab, the mosque's twin, answering it on the
//! right; narrower ones draw the mosque in behind the Taj's left minaret;
//! taller ones add sky above and canal below. Everything vertical is in
//! upstream's rows, offset by the sky a tall panel adds on top.

mod taj;

use std::f64::consts::PI;

use super::halftone::{Dots, BAYER};
use super::math::{clamp, fbm, hash, js_round, mix, noise, smooth};
use super::stretch::Stretch;
use super::{hex, Piece};
use crate::grid::Cell;
use taj::{mosque, taj};

const BASE: f64 = 68.0; // where the Taj meets the garden, and the far end of the canal
const HZ: f64 = 60.0; // eye level
const S: f64 = 0.7; // cells per metre on the Taj
const MB: f64 = 67.0; // and its foot
const KR: f64 = 1.55; // the reflection is foreshortened so the dome reaches the canal
/// Cloud strip rows; the cloud banks wrap at `BANKS_W` columns.
const CH: usize = 50;
const BANKS_W: f64 = 400.0;
/// Upstream's cloud and haze strip width. A wider panel takes a whole
/// multiple of it, so the noise still wraps seamlessly.
const STRIP: usize = 400;

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

/// Where things sit in a `w x h` frame. Upstream's 200x100 is the anchor
/// every value moves from, so there it is exact.
struct Layout {
    w: usize,
    h: usize,
    /// Rows of sky above upstream's top row: frame row `r` is upstream's
    /// `r - top`.
    top: f64,
    /// Upstream's bottom row, in its own rows.
    hf: f64,
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
    /// Cloud and haze strip width.
    cw: usize,
}

impl Layout {
    fn new(w: usize, h: usize) -> Self {
        let s = Stretch::new(w, h);
        let Stretch { w: wf, narrow, tall, .. } = s;
        let grow = |at, by_wide, by_narrow| js_round(s.grow(at, by_wide, by_narrow));
        let top = js_round(0.6 * tall);
        let cx = grow(128.0, 48.0, 70.0);
        let mx = cx - grow(83.0, 0.0, 42.0);
        // on a square panel the mosque runs off the left edge, so the sun
        // climbs over its middle instead
        let sun = mx - js_round(9.0 * (1.0 - narrow));
        let jx = 2.0 * cx - mx;
        let shift = cx - 128.0;
        Self {
            w,
            h,
            top,
            hf: h as f64 - top,
            cx,
            sun: [sun, 40.0],
            mx,
            jawab: (jx + 24.0 < wf).then_some(jx),
            far: [sun + 14.0, sun + 164.0],
            banks: [
                [sun + 40.0, 28.0, 40.0, 7.0, 3.5, 1.4], // over the sun's right shoulder
                [cx + 24.0, 36.0, 52.0, 9.0, 4.0, 1.0],  // behind the dome
                [sun - 20.0, 13.0, 32.0, 4.0, 2.5, 0.8], // a high wisp
                [250.0 + shift, 24.0, 40.0, 8.0, 4.0, 1.0],
                [330.0 + shift, 33.0, 36.0, 7.0, 4.0, 0.95],
            ],
            birds: [sun + 22.0, wf + 60.0],
            stars: [cx - 32.0, cx + 12.0],
            cw: w.div_ceil(STRIP) * STRIP,
        }
    }

    fn sun_glow(&self, x: f64, y: f64) -> f64 {
        let dx = x - self.sun[0];
        let dy = (y - self.sun[1]) * 1.25;
        let d = (dx * dx + dy * dy).sqrt();
        (-d / 4.0).exp() * 0.9 + (-d / 13.0).exp() * 0.42 + (-d / 40.0).exp() * 0.3
    }

    fn sky_at(&self, x: f64, y: f64) -> [f64; 3] {
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
        let far = smooth(self.far[0], self.far[1], x) * 0.18;
        r *= 1.0 - far;
        g *= 1.0 - far * 0.8;
        b *= 1.0 - far * 0.3;
        let glow = self.sun_glow(x, y)
            + (-(y - 57.0).abs() / 5.0).exp() * 0.25 * (-(x - self.sun[0]).abs() / 50.0).exp();
        r += glow * 1.0;
        g += glow * 0.78;
        b += glow * 0.48;
        // faint shafts of light fanning up from the sun through the haze
        let ang = (y - self.sun[1]).atan2(x - self.sun[0]);
        let ray = fbm(ang * 9.0 + 3.0, 1.7, 2, 0.0);
        let rd = (x - self.sun[0]).hypot(y - self.sun[1]);
        let shaft = smooth(0.5, 0.75, ray) * (-rd / 45.0).exp() * smooth(4.0, 14.0, rd) * 0.12;
        r += shaft;
        g += shaft * 0.8;
        b += shaft * 0.55;
        [r, g, b]
    }

    /// Dawn cloud in a wrapping strip that drifts: broken noise, gathered into a
    /// bank up and right of the sun and a lower one behind the dome.
    fn cdens(&self, x: f64, y: f64) -> f64 {
        let cw = self.cw as f64;
        let q = fbm(x * 0.01, y * 0.04, 2, cw * 0.01);
        let n = fbm(x * 0.025 + q * 1.6, y * 0.075 + q * 0.6, 5, cw * 0.025);
        let mut m = 0.0;
        for [bx, by, rx, up, down, a] in self.banks {
            let mut dx = x - bx;
            dx -= js_round(dx / BANKS_W) * BANKS_W;
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

pub struct TajDawn {
    l: Layout,
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

impl Piece for TajDawn {
    const NAME: &'static str = "taj-dawn";
    const FPS: u32 = 15;
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

    fn new(cols: usize, rows: usize) -> Self {
        let l = Layout::new(cols, rows);
        let (w, h, top, hf) = (l.w, l.h, l.top, l.hf);
        let n = w * h;
        let mut mat = vec![SKY; n];
        let (mut rv, mut gv, mut bv) = (vec![0f32; n], vec![0f32; n], vec![0f32; n]);
        let mut floor = vec![0.12f32; n];
        let mut haze_w = vec![0f32; n];
        let mut glow = vec![0f32; n];

        let cw = l.cw;
        let cwf = cw as f64;
        let ch = CH + top as usize;
        let mut cloud = vec![0f32; cw * ch];
        let mut cloud_lit = vec![0f32; cw * ch];
        for r in 0..ch {
            for x in 0..cw {
                let (xf, y) = (x as f64, (r as f64 - top) + 0.5);
                let d = l.cdens(xf, y);
                cloud[r * cw + x] = smooth(0.46, 0.66, d) as f32;
                // the side toward the sun (down and left) catches the light
                let toward = l.cdens(xf - 2.0, y + 2.5);
                cloud_lit[r * cw + x] = clamp(
                    0.56 + (d - toward) * 0.75 - (d - 0.6) * 0.2
                        + 0.7 * (fbm(xf * 0.09, y * 0.2, 2, cwf * 0.09) - 0.5),
                ) as f32;
            }
        }

        // the far tree line: rounded crowns, lower behind the Taj
        let mut crowns: Vec<[f64; 3]> = Vec::new();
        let (mut x, mut i) = (-12.0, 0.0);
        while x < w as f64 + 12.0 {
            let rad = 3.0 + hash(i, 61.0) * 4.5;
            crowns.push([
                x,
                60.5 - hash(i, 62.0) * 2.5 - 2.5 * fbm(x * 0.03, 2.0, 2, 0.0) + rad * 0.35,
                rad,
            ]);
            x += 4.0 + hash(i, 60.0) * 5.0;
            i += 1.0;
        }
        let mut tops = vec![0f32; w];
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
        let tree_top = |x: f64| f64::from(tops[(x.floor().max(0.0) as usize).min(w - 1)]);
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
                    let cxp = l.cx + side * off * s;
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
        for ru in (MB - 27.0) as usize..MB as usize {
            let (r, y) = (ru + top as usize, ru as f64 + 0.5);
            for x in (l.mx - 24.0).max(0.0) as usize..=(l.mx + 24.0) as usize {
                mq[r * w + x] = mosque(x as f64 + 0.5, y, l.mx, MB);
            }
            if let Some(jx) = l.jawab {
                for x in (jx - 24.0) as usize..=(jx + 24.0) as usize {
                    mq[r * w + x] = mosque(x as f64 + 0.5 - (jx - l.mx), y, l.mx, MB);
                }
            }
        }

        for r in 0..h {
            let ru = r as f64 - top;
            for x in 0..w {
                let k = r * w + x;
                let (y, xc) = (ru + 0.5, x as f64 + 0.5);
                let mut m = SKY;
                let (mut cr, mut cg, mut cb) = (0.0, 0.0, 0.0);
                let mut fl;
                let mut hz = 0.0;
                let tt = tree_top(xc);
                if y < BASE && y >= tt {
                    m = BGTREE;
                } else if y >= BASE {
                    let dx = (xc - l.cx).abs();
                    m = if dx < pool_half(y) {
                        POOL
                    } else if dx < walk_half(y) {
                        WALK
                    } else {
                        LAWN
                    };
                }
                if m == SKY {
                    [cr, cg, cb] = l.sky_at(xc, y);
                    // never one flat tone: a faint unevenness in the dawn air
                    let veil = 0.88 + 0.24 * fbm(xc * 0.05, y * 0.09, 3, 0.0);
                    cr *= veil;
                    cg *= veil;
                    cb *= veil;
                    glow[k] = l.sun_glow(xc, y) as f32;
                    hz = 0.5 + 0.5 * smooth(20.0, 58.0, y);
                    fl = 0.05 + 0.13 * smooth(18.0, 42.0, y);
                } else if m == BGTREE {
                    // distant trees, flattened by the haze; rimmed where the sun is close
                    let tex = fbm(xc * 0.25, y * 0.3, 3, 0.0);
                    let depth = smooth(tt, BASE, y);
                    let sky = l.sky_at(xc, tt);
                    let a = 0.46 - 0.12 * depth + 0.12 * tex;
                    (cr, cg, cb) = (
                        mix(0.1, sky[0], a),
                        mix(0.09, sky[1], a),
                        mix(0.15, sky[2], a),
                    );
                    let rim = smooth(tt + 1.8, tt, y) * (-(xc - l.sun[0]).abs() / 18.0).exp();
                    cr += 0.4 * rim;
                    cg += 0.27 * rim;
                    cb += 0.14 * rim;
                    // their feet lost in a bank of ground mist, broken into soft patches
                    let patch =
                        0.45 + 0.75 * smooth(0.3, 0.7, fbm(xc * 0.035 + 11.0, y * 0.18, 3, 0.0));
                    let mist = smooth(tt + 1.0, BASE + 1.0, y) * 0.75 * patch.min(1.0);
                    let mw = (-(xc - l.sun[0]).abs() / 90.0).exp();
                    cr = mix(cr, 0.62 + 0.25 * mw, mist);
                    cg = mix(cg, 0.46 + 0.16 * mw, mist);
                    cb = mix(cb, 0.55 + 0.02 * mw, mist);
                    fl = 0.1;
                    hz = 0.8;
                } else if m == LAWN {
                    let v = (y - BASE) / (hf - BASE);
                    let u = (xc - l.cx) / (y - HZ); // across the ground plane
                    let tex = fbm(u * 3.0, 40.0 / (y - HZ), 3, 0.0);
                    // mown bands that run toward the Taj
                    let stripe = if (u * 1.4).floor() as i64 & 1 != 0 {
                        1.0
                    } else {
                        0.55
                    };
                    // low sun raking across the grass from the left: gold where it
                    // lands, violet sky-light in the shade
                    let mut lit = (0.6 + 0.4 * (-(xc - l.sun[0]).abs() / 60.0).exp())
                        * stripe
                        * (0.85 + 0.3 * tex);
                    lit *= shadow_at(xc, y);
                    let vig = 1.0
                        - 0.45
                            * smooth(0.5, 1.0, v)
                            * (0.6 + 0.4 * smooth(40.0, 0.0, xc.min(w as f64 - xc)));
                    let fall = 1.0 - 0.35 * v;
                    // sunlit grass turns from gold near the sun to rose further off
                    let warm = (-(xc - l.sun[0]).abs() / 80.0).exp();
                    // the shade holds the violet of the sky overhead, brighter in the open
                    let amb = 0.75 + 0.5 * stripe - 0.25 * v;
                    cr = (0.12 * amb + 0.56 * lit * fall) * vig;
                    cg = (0.09 * amb + (0.36 + 0.14 * warm) * lit * fall) * vig;
                    cb = (0.24 * amb + (0.26 - 0.1 * warm) * lit * fall) * vig;
                    // the far lawn sits in the haze
                    let far = smooth(BASE + 10.0, BASE, y) * 0.7;
                    let mw = (-(xc - l.sun[0]).abs() / 90.0).exp();
                    cr = mix(cr, 0.62 + 0.25 * mw, far);
                    cg = mix(cg, 0.46 + 0.16 * mw, far);
                    cb = mix(cb, 0.55 + 0.02 * mw, far);
                    fl = 0.1;
                    hz = 0.7 * (1.0 - v);
                } else if m == WALK {
                    let v = (y - BASE) / (hf - BASE);
                    let lit = if xc < l.cx { 1.0 } else { 0.86 };
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
                    let sg = l.sun_glow(xc, y);
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
                    let up = if r > 0 { mq[k - w] } else { -1 };
                    let toward = if xc < l.sun[0] {
                        if x < w - 1 {
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
                    if let Some((lit, rec)) = taj((xc - l.cx) / S, (BASE - y) / S) {
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
                        let g = 0.94 + 0.08 * hash(x as f64 * 3.0 + 1.0, ru * 5.0 + 2.0);
                        cr *= g;
                        cg *= g;
                        cb *= g;
                        let low = smooth(40.0, BASE, y) * 0.2;
                        let sky = l.sky_at(xc, y);
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
            let sun_side = if tr.x < l.cx { 0.9 } else { 0.6 };
            let r0 = (tr.base - tr.h + top).floor().max(0.0) as usize;
            let r1 = (tr.base + 0.6 + top).ceil().min(h as f64) as usize;
            for r in r0..r1 {
                let y = (r as f64 - top) + 0.5;
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
                let x1 = (tr.x + tr.w + 2.0).ceil().min(w as f64) as usize;
                for x in x0..x1 {
                    let xc = x as f64 + 0.5;
                    let nz = noise(xc * 0.9, y * 0.55 + tr.s, 0.0);
                    let hw = tr.w * p * (0.88 + 0.26 * nz) + 0.35;
                    let dx = (xc - tr.x) / hw;
                    if dx.abs() > 1.0 {
                        continue;
                    }
                    let k = r * w + x;
                    let leaf = fbm(xc * 0.6, y * 0.35 + tr.s * 3.0, 2, 0.0);
                    let rim = smooth(-0.2, -0.9, dx)
                        * (0.4 + 0.6 * leaf)
                        * (0.5 + 0.5 * (-(tr.x - l.sun[0]).abs() / 80.0).exp());
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
        let mut haze1 = vec![0f32; cw * h];
        let mut haze2 = vec![0f32; cw * h];
        for r in 0..h {
            let rf = r as f64 - top;
            for x in 0..cw {
                let xf = x as f64;
                haze1[r * cw + x] =
                    smooth(0.4, 0.75, fbm(xf * 0.018, rf * 0.09, 4, cwf * 0.018)) as f32;
                haze2[r * cw + x] = smooth(
                    0.42,
                    0.8,
                    fbm(xf * 0.04 + 7.0, rf * 0.16 + 3.0, 3, cwf * 0.04),
                ) as f32;
            }
        }
        // haze is thickest just above the garden
        let haze_row = (0..h)
            .map(|r| (0.03 + 0.42 * (-((r as f64 - top) + 0.5 - 63.0).abs() / 6.0).exp()) as f32)
            .collect();

        let birds = std::array::from_fn(|i| {
            let i = i as f64;
            [
                l.birds[0] + i * 8.0 + hash(i, 40.0) * 4.0,
                30.0 - i * 2.5 + hash(i, 41.0) * 2.0,
                hash(i, 42.0) * 6.28,
            ]
        });
        // ordered dither, nudged per cell so its grid does not show in flat light
        let dith = (0..n)
            .map(|k| (BAYER[((k / w) & 3) * 4 + ((k % w) & 3)] * 0.85) as f32)
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
            l,
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
        let l = &self.l;
        let (w, h, top, hf) = (l.w, l.h, l.top, l.hf);
        let cw = l.cw;
        let hwd = cw as f64;
        // where the birds are this frame
        let (mut bx, mut by, mut bu) = ([0.0; 3], [0.0; 3], [false; 3]);
        for (i, &[x0, y0, ph]) in self.birds.iter().enumerate() {
            bx[i] = ((x0 + t * 3.2) % l.birds[1] + l.birds[1]) % l.birds[1] - 30.0;
            by[i] = y0 + (t * 0.4 + ph).sin() * 1.3;
            bu[i] = (t * 7.0 + ph).sin() > 0.0;
        }

        for r in 0..h {
            let rf = r as f64 - top;
            let y = rf + 0.5;
            let hr = f64::from(self.haze_row[r]);
            for xi in 0..w {
                let x = xi as f64;
                let k = r * w + xi;
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
                    let sx = js_round(x + wob).clamp(0.0, (w - 1) as f64);
                    let flick = if (x * 0.3 + t * 1.3 + y).sin() > 0.6 {
                        1.0
                    } else {
                        0.0
                    };
                    let sr = js_round(BASE - 0.5 - (y - BASE) * KR - flick).clamp(-top, BASE - 1.0);
                    let j = (sr + top) as usize * w + sx as usize;
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
                        let i0 = r * cw + ix % cw;
                        let i1 = r * cw + (ix + 1) % cw;
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
                    if m == SKY && rf < 24.0 && xi > l.stars[0] as usize && hash(x, rf * 3.0 + 11.0) > 0.988 {
                        // the last few stars, fading where the dawn reaches
                        let tw =
                            0.7 + 0.3 * (t * (1.2 + hash(x, rf) * 2.0) + hash(rf, x) * 6.28).sin();
                        let s = tw * 0.62 * smooth(24.0, 8.0, rf) * smooth(l.stars[0], l.stars[1], x);
                        cr = cr.max(s * 0.85);
                        cg = cg.max(s * 0.85);
                        cb = cb.max(s);
                    }
                    if m == SKY {
                        // the sun's disc, softened by the haze
                        let (dx, dy) = (x + 0.5 - l.sun[0], y - l.sun[1]);
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
                    let i1 = r * cw + sx1 as usize;
                    let i2 = r * cw + sx2 as usize;
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
