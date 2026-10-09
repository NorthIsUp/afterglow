//! ocean sunset: golden hour at sea. The sun rests on the horizon beyond a dark
//! pine headland, heaped cloud overhead lit from below, a glitter path running
//! across the water toward us, and long swells rolling in, their crests
//! catching the light.
//!
//! The land and clear sky are built once, the clouds are wrapping fields that
//! drift, and the sea is shaded each frame. A small sloop sits dark against
//! the glow just left of the sun.
//!
//! At any size the headland keeps the left and the sun sits on the horizon
//! right of centre: wider panels draw the headland larger to hold its weight
//! across the wider sea, move the sun and the sloop out with the frame and
//! raise a far, low island right of the sun; narrower ones draw the headland
//! smaller and bring the sun in; taller ones add sky above, where the cloud
//! deck climbs, and sea below.

use std::f64::consts::PI;

use super::halftone::{bayer, Dots};
use super::math::{clamp, fbm, hash, js_round, mix, noise, smooth};
use super::{hex, Piece};
use crate::grid::Cell;

/// The sun's radius.
const SR: f64 = 8.5;
/// Wind chop on the swell: [dir x, dir z, wavenumber, slope, speed, offset].
const CHOP: [[f64; 6]; 4] = [
    [-0.45, 0.89, 22.0, 0.07, 2.1, 0.4],
    [0.5, 0.87, 37.0, 0.06, 2.9, 2.1],
    [-0.15, 0.99, 61.0, 0.05, 3.7, 4.4],
    [0.3, 0.95, 97.0, 0.04, 4.6, 1.3],
];
/// Pines on the headland: [x, height, half width at the foot].
const PINES: [[f64; 3]; 8] = [
    [2.5, 10.0, 2.6],
    [8.0, 13.0, 3.0],
    [13.0, 18.0, 3.6],
    [18.5, 11.0, 2.8],
    [24.0, 20.0, 3.8],
    [30.0, 12.0, 2.9],
    [35.0, 8.0, 2.3],
    [39.5, 5.0, 1.8],
];
/// The sky's gradient, top to horizon, as [stop, r, g, b].
const STOPS: [[f64; 4]; 6] = [
    [0.0, 0.08, 0.1, 0.3],
    [0.3, 0.15, 0.1, 0.34],
    [0.52, 0.4, 0.15, 0.4],
    [0.72, 0.68, 0.27, 0.4],
    [0.88, 0.8, 0.32, 0.4],
    [1.0, 0.88, 0.4, 0.4],
];
const V: usize = 128;
/// The low bars' wrap.
const CW: usize = 720;

const ROCK: u8 = 1;
const PINE: u8 = 2;
const SLOOP: u8 = 3;
const ISLE: u8 = 4;

fn gradient(v: f64) -> [f64; 3] {
    let mut i = 1;
    while i < STOPS.len() - 1 && v > STOPS[i][0] {
        i += 1;
    }
    let (a, b) = (STOPS[i - 1], STOPS[i]);
    let k = clamp((v - a[0]) / (b[0] - a[0]));
    [mix(a[1], b[1], k), mix(a[2], b[2], k), mix(a[3], b[3], k)]
}

/// A far, low island spanning columns `span`: its skyline row at x.
fn isle(x: f64, span: [usize; 2], hzf: f64) -> f64 {
    let (a, b) = (span[0] as f64, span[1] as f64);
    if x > a && x < b {
        let u = (x - a) / (b - a);
        hzf + 0.8
            - 5.5 * (PI * u).sin().powf(0.7) * (0.75 + 0.5 * fbm(x * 0.12, 2.7, 3, 0.0))
            - 3.0 * (-((x - (a + 20.0)) / 7.0).powi(2)).exp()
    } else {
        1e9
    }
}

/// Long thin bars low over the horizon.
fn lower(x: f64, y: f64, hzf: f64) -> f64 {
    let cw = CW as f64;
    let b = fbm(x / 40.0, y * 0.3, 4, cw / 40.0);
    let gaps = fbm(x / 120.0, 5.0, 2, cw / 120.0);
    b + 0.01 + 0.5 * (gaps - 0.5)
        - 0.35 * smooth(hzf - 14.0, hzf - 20.0, y)
        - 0.25 * smooth(hzf - 4.0, hzf - 1.0, y)
}

/// Dot and colour for one shaded cell. Small dots are drawn brighter to make
/// up their size, but the darkest cells keep a dim colour, so the shadows stay
/// deep instead of glittering.
#[inline]
fn halftone(dots: &mut Dots, r: usize, x: usize, rgb: [f64; 3], floor: f64, fade: f64) -> Cell {
    let peak = rgb[0].max(rgb[1]).max(rgb[2]).max(1e-4);
    let level = clamp(floor + (1.0 - floor) * peak.powf(0.85) * 0.95) * fade;
    let step = Dots::step(level, bayer(r, x));
    let want = Dots::want(step, level, 0.06);
    let s = ((0.3 + 0.7 * want) * (0.4 + 0.6 * smooth(0.14, 0.5, level))) / peak;
    dots.dot(step, rgb, s)
}

/// Where things sit in a `w x h` frame. Upstream's 200x100 is the anchor
/// every value moves from, so there it is exact.
struct Layout {
    w: usize,
    h: usize,
    /// The horizon.
    hz: usize,
    sun: [f64; 2],
    /// The sloop's mast.
    boat: usize,
    /// The headland's scale, across and up: 1.0 for upstream's, where `x / 1.0`
    /// and `1.0 * v` are exact.
    hs: f64,
    vs: f64,
    /// How far the headland's grid reaches.
    landw: usize,
    /// The cloud deck plane's width.
    u: usize,
    /// Its noise periods: the deck wraps at [`Layout::u`].
    deck_periods: [f64; 3],
    /// The far island's span.
    isle: Option<[usize; 2]>,
}

impl Layout {
    fn new(w: usize, h: usize) -> Self {
        // `wide` is 1 at 3.2:1, the old `-wide` recomposition; `narrow` is 1
        // at square; `tall` is the rows past upstream's 100.
        let (wf, tall) = (w as f64, h - 100);
        let wide = (wf - 200.0) / 120.0;
        let narrow = clamp((200.0 - wf) / 100.0);
        let grow = |at: f64, by_wide: f64, by_narrow: f64| {
            if wf >= 200.0 {
                at + by_wide * wide
            } else {
                at - by_narrow * narrow
            }
        };
        let hz = 56 + tall / 2;
        let sun = grow(134.0, 78.0, 62.0);
        let hs = grow(1.0, 0.3, 0.15);
        // a deck at least twice the frame's width, so it never repeats in view
        let u = 640 * w.div_ceil(200).max(1);
        let scale = u as f64 / 640.0;
        // the island needs 40 columns of sea right of the sun to sit in
        let i0 = sun as usize + 34;
        let i1 = (w - 4).min(i0 + 90);
        Self {
            w,
            h,
            hz,
            sun: [sun, hz as f64 - 4.5],
            boat: grow(112.0, 76.0, 56.0) as usize,
            hs,
            vs: grow(1.0, 0.15, 0.15),
            landw: (60.0 * hs).ceil() as usize,
            u,
            deck_periods: [8.0 * scale, 4.0 * scale, 26.0 * scale],
            isle: (i1 >= i0 + 40).then_some([i0, i1]),
        }
    }

    /// The headland's skyline row at x.
    fn ridge(&self, x: f64) -> f64 {
        let x = x / self.hs;
        self.hz as f64 + 2.5
            - self.vs * 11.0 * smooth(54.0, 32.0, x).powf(0.7)
            - self.vs * 2.4 * (-((x - 19.0) / 9.0).powi(2)).exp()
            - 2.0 * fbm(x * 0.21, 3.3, 3, 0.0)
    }

    /// The row where the headland's foot meets the sea.
    fn shore(&self, x: f64) -> f64 {
        let x = x / self.hs;
        self.hz as f64 + 1.5 + 5.0 * smooth(54.0, 4.0, x)
    }

    /// A sea stack off the point.
    fn stack(&self, x: f64) -> f64 {
        let x = x / self.hs;
        if x > 52.0 && x < 58.0 {
            self.hz as f64 - 3.5 - 1.6 * fbm(x * 0.6, 8.1, 2, 0.0) + 2.5 * ((x - 55.0) / 3.0).powf(4.0)
        } else {
            1e9
        }
    }

    fn deck_at(&self, u: f64, v: f64) -> f64 {
        let [p0, p1, p2] = self.deck_periods;
        let q = fbm(u * 0.0125, v * 0.06, 2, p0);
        let patch = fbm(u * 0.00625, v * 0.03 + 7.0, 2, p1);
        fbm(u * 0.040625 + q * 1.6, v * 0.3, 5, p2) + 0.25 * (patch - 0.5)
    }
}

pub struct OceanSunset {
    l: Layout,
    dots: Dots,
    sky: Vec<f32>,
    land: Vec<u8>,
    lr: Vec<f32>,
    lg: Vec<f32>,
    lb: Vec<f32>,
    deck: Vec<f32>,
    deck_lit: Vec<f32>,
    thin_top: Vec<f32>,
    lo_cover: Vec<f32>,
    lo_lit: Vec<f32>,
    /// This frame's sky, finished, for the sea to mirror.
    skr: Vec<f32>,
    skg: Vec<f32>,
    skb: Vec<f32>,
}

impl Piece for OceanSunset {
    const NAME: &'static str = "ocean-sunset";
    const FPS: u32 = 15;
    const GROUND: u32 = hex("#0b0817");
    #[rustfmt::skip]
    const PALETTE: &'static [u32] = &[
        hex("#140f2c"), hex("#1e1640"), hex("#2b1c52"), hex("#3d2266"), hex("#552a74"),
        hex("#6e2c78"), hex("#90327c"), hex("#b23c7c"), hex("#d24f7a"), hex("#ea6a78"),
        hex("#c2306a"), hex("#ff5d8f"), hex("#ff8a9a"), hex("#7a2456"), hex("#f05d5e"),
        hex("#f2804f"), hex("#f99a43"), hex("#ffb44a"), hex("#ffcd62"), hex("#ffe39a"), hex("#fff5d8"),
        hex("#a8323c"), hex("#d2453c"), hex("#e8603e"),
        hex("#2f1a3e"), hex("#47234f"), hex("#63305c"), hex("#874266"), hex("#ac5b70"),
        hex("#ff9e86"), hex("#ffc4a2"), hex("#ffdcc0"),
        hex("#0e1230"), hex("#171a46"), hex("#232059"), hex("#33286a"), hex("#4a2f78"),
        hex("#1c2a5c"), hex("#2e4078"),
        hex("#7d3f8f"), hex("#a24f98"), hex("#c8649e"), hex("#de8bb5"),
    ];

    fn new(cols: usize, rows: usize) -> Self {
        let l = Layout::new(cols, rows);
        let (w, hz) = (l.w, l.hz);
        let hzf = hz as f64;
        let n = w * l.h;
        let (hs, vs, landw, boat) = (l.hs, l.vs, l.landw, l.boat);
        let boatf = boat as f64;
        let landh = hz + 10;

        // the clear sky, built once
        let mut sky = vec![0f32; hz * w * 3];
        for r in 0..hz {
            for xi in 0..w {
                let x = xi as f64;
                let y = r as f64 + 0.5;
                let (dx, dy) = (x + 0.5 - l.sun[0], (y - l.sun[1]) * 1.6);
                let d = (dx * dx + dy * dy).sqrt();
                // rose and violet everywhere, burning gold only toward the sun
                let near = (-dx.abs() / 62.0).exp();
                let v = y / hzf;
                let mut g = gradient(v.powf(1.0 + 0.12 * (1.0 - near)));
                let warm = (-dx.abs() / 36.0).exp() * smooth(0.45, 1.0, v) * 0.9;
                g[0] = mix(g[0], 1.0 * mix(0.6, 1.0, v), warm);
                g[1] = mix(g[1], 0.64 * mix(0.6, 1.0, v), warm);
                g[2] = mix(g[2], 0.3, warm);
                let fall = mix(1.0, 0.86 + 0.14 * near, smooth(0.6, 1.0, v));
                let glow = (-d / 40.0).exp() * 0.15 + (-d / 9.0).exp() * 0.1;
                // high haze in long thin bands, so no stretch of sky is one flat tone
                let haze = 0.82 + 0.36 * fbm(x * 0.03, y * 0.12, 3, 0.0);
                let k = (r * w + xi) * 3;
                let vig = (1.0 - 0.08 * ((x + 0.5 - w as f64 / 2.0).abs() / (w as f64 / 2.0)).powf(2.0)) * haze * fall;
                sky[k] = (g[0] * (0.85 + 0.15 * near) * vig + glow) as f32;
                sky[k + 1] = (g[1] * (0.65 + 0.35 * near) * vig + glow * 0.7) as f32;
                sky[k + 2] = (g[2] * (1.1 - 0.2 * near) * vig + glow * 0.3) as f32;
            }
        }

        // the headland and its pines, built once
        let mut land = vec![0u8; n];
        let (mut lr, mut lg, mut lb) = (vec![0f32; n], vec![0f32; n], vec![0f32; n]);
        // the rock's seaward edge on each row, for the light on the cliff face
        let mut edge = vec![-1f32; landh];
        for (r, e) in edge.iter_mut().enumerate() {
            let y = r as f64 + 0.5;
            for x in 0..landw {
                let xc = x as f64 + 0.5;
                if y >= l.ridge(xc) && y < l.shore(xc) {
                    *e = x as f32;
                }
            }
        }
        let pine_foot = PINES.map(|p| l.ridge(p[0] * hs));
        for (r, &er) in edge.iter().enumerate() {
            let y = r as f64 + 0.5;
            for xi in 0..landw {
                let k = r * w + xi;
                let x = xi as f64;
                let xc = x + 0.5;
                let (t0, st) = (l.ridge(xc), l.stack(xc));
                let (mut cr, mut cg, mut cb) = (0.0, 0.0, 0.0);
                let main = y >= t0 && y < l.shore(xc);
                if main || (y >= st && y < hzf + 1.2) {
                    land[k] = ROCK;
                    let is_stack = !main;
                    let top = if is_stack { st } else { t0 };
                    // dark violet rock in rough strata
                    let s = 0.6 + 0.8 * fbm(x * 0.22, y * 0.7, 3, 0.0);
                    (cr, cg, cb) = (0.06 * s, 0.042 * s, 0.15 * s);
                    // the sun is low and to the right: slopes that drop toward it catch it
                    let facing = clamp(
                        0.4 + if is_stack {
                            0.5
                        } else {
                            (l.ridge(xc + 1.2) - l.ridge(xc - 1.2)) * 0.45
                        },
                    );
                    let rim = smooth(top + 2.4, top + 0.4, y) * facing;
                    // the cliff face, warm where it turns to the sun, broken by crags
                    let crag = smooth(0.35, 0.75, fbm(x * 0.4 + 3.0, y * 0.5, 3, 0.0));
                    let er = f64::from(er);
                    let face = if is_stack {
                        smooth(53.5 * hs, 57.5 * hs, xc) * 0.7
                    } else if er > 30.0 * hs {
                        (-(er - x) / 4.0).exp() * smooth(hzf + 5.0, hzf - 4.0, y)
                    } else {
                        0.0
                    };
                    let lit = clamp((rim * 0.9).max(face * (0.3 + 0.7 * crag) * 0.75));
                    cr = mix(cr, 0.95, lit);
                    cg = mix(cg, 0.38, lit * 0.95);
                    cb = mix(cb, 0.3, lit * 0.9);
                }
                for (&[tx, th, tw], &tb) in PINES.iter().zip(&pine_foot) {
                    let (tx, th, tw) = (tx * hs, th * vs, tw * hs);
                    let dy = y - (tb - th);
                    // a conifer: a spire that widens in tiers of branches
                    let tier = (dy + th * 0.3) / 2.4;
                    let half = (dy / th) * tw * (0.5 + 0.75 * (tier - tier.floor())) + 0.35;
                    let ex = xc - tx;
                    if dy >= 0.0 && y < tb + 1.5 && ex.abs() <= half {
                        land[k] = PINE;
                        let s = 0.7 + 0.6 * hash(x * 13.0 + r as f64, 5.0);
                        (cr, cg, cb) = (0.045 * s, 0.03 * s, 0.1 * s);
                        // the right flank faces the sun
                        let e = if ex > 0.0 && ex > half - 1.1 {
                            0.3 + 0.3 * smooth(th, 0.0, dy)
                        } else {
                            0.0
                        };
                        cr = mix(cr, 0.85, e);
                        cg = mix(cg, 0.3, e);
                        cb = mix(cb, 0.3, e);
                    }
                }
                if land[k] != 0 {
                    (lr[k], lg[k], lb[k]) = (cr as f32, cg as f32, cb as f32);
                }
            }
        }
        // a small sloop out on the water, dark against the glow left of the sun
        for r in hz - 10..hz + 4 {
            let y = r as f64 + 0.5;
            for x in boat - 6..=boat + 6 {
                let k = r * w + x;
                let ex = x as f64 + 0.5 - boatf;
                let hull = (hzf + 1.0..hzf + 3.0).contains(&y)
                    && (ex - 0.3).abs() < 4.6 - (y - hzf - 1.0) * 1.3;
                let mast = ex.abs() < 0.5 && (hzf - 9.0..hzf + 1.0).contains(&y);
                let main = ex > 0.0
                    && (hzf - 8.5..hzf + 0.5).contains(&y)
                    && ex < 0.6 + (y - (hzf - 8.5)) * 0.42;
                let jib = ex < 0.0
                    && (hzf - 7.0..hzf + 0.5).contains(&y)
                    && -ex < (y - (hzf - 7.0)) * 0.36;
                if hull || mast || main || jib {
                    land[k] = SLOOP;
                    // the sails are thin enough to glow a little with the sun behind them
                    let glow = if main {
                        0.12 + 0.18 * smooth(0.0, 3.5, ex)
                    } else {
                        0.0
                    };
                    lr[k] = (0.05 + glow) as f32;
                    lg[k] = (0.03 + glow * 0.45) as f32;
                    lb[k] = (0.09 + glow * 0.35) as f32;
                }
            }
        }

        // the far island, hazy with distance, its sunward slopes rimmed
        if let Some([i0, i1]) = l.isle {
            let isle = |x: f64| isle(x, [i0, i1], hzf);
            for r in hz - 12..hz + 1 {
                let y = r as f64 + 0.5;
                for xi in i0..i1 {
                    let xc = xi as f64 + 0.5;
                    let top = isle(xc);
                    if y >= top && y < hzf + 0.8 {
                        let k = r * w + xi;
                        land[k] = ISLE;
                        let s = 0.8 + 0.4 * fbm(xc * 0.3, y * 0.6, 2, 0.0);
                        let facing = clamp(0.3 + (isle(xc - 1.2) - isle(xc + 1.2)) * 0.5);
                        let rim = smooth(top + 1.8, top + 0.3, y) * facing;
                        lr[k] = mix(0.15 * s, 0.85, rim * 0.7) as f32;
                        lg[k] = mix(0.07 * s, 0.34, rim * 0.6) as f32;
                        lb[k] = mix(0.2 * s, 0.34, rim * 0.6) as f32;
                    }
                }
            }
        }

        // The cloud deck: a sheet of heaped cloud seen from below, laid out on
        // its own plane so it shrinks and flattens toward the horizon.
        let mut deck = vec![0f32; l.u * V];
        let mut deck_lit = vec![0f32; l.u * V];
        for v in 0..V {
            for u in 0..l.u {
                let (uf, vf) = (u as f64, v as f64);
                let d = l.deck_at(uf, vf);
                deck[v * l.u + u] = d as f32;
                // the side of each heap that faces the horizon, and the sun, is lit
                deck_lit[v * l.u + u] = clamp(0.5 + (d - l.deck_at(uf, vf + 2.5)) * 9.0) as f32;
            }
        }
        // thinning toward the zenith, ragged rather than a ruled line
        let thin_top = (0..w * hz)
            .map(|k| fbm((k % w) as f64 * 0.05 + 11.0, (k / w) as f64 * 0.15, 2, 0.0) as f32)
            .collect();

        // low bars over the horizon: a wrapping field that drifts
        let mut lo_cover = vec![0f32; CW * hz];
        let mut lo_lit = vec![0f32; CW * hz];
        for r in 0..hz {
            for x in 0..CW {
                let (xf, y) = (x as f64, r as f64 + 0.5);
                let k = r * CW + x;
                let dl = lower(xf, y, hzf);
                lo_cover[k] = smooth(0.58, 0.66, dl) as f32;
                lo_lit[k] = clamp(0.5 + (dl - lower(xf, y + 1.2, hzf)) * 6.0) as f32;
            }
        }

        Self {
            dots: Dots::new(Self::PALETTE),
            sky,
            land,
            lr,
            lg,
            lb,
            deck,
            deck_lit,
            thin_top,
            lo_cover,
            lo_lit,
            skr: vec![0f32; hz * w],
            skg: vec![0f32; hz * w],
            skb: vec![0f32; hz * w],
            l,
        }
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        let (d_up, d_lo) = (t * 0.9, t * 1.7);
        let (w, h, hz) = (self.l.w, self.l.h, self.l.hz);
        let (hzf, hf) = (hz as f64, h as f64);
        let (hs, landw, boat, u) = (self.l.hs, self.l.landw, self.l.boat, self.l.u);
        let sun = self.l.sun;
        let landh = hz + 10;
        // a taller sky holds the deck and the first stars in proportion
        let deck_d = 58.0 * (hzf / 56.0);
        let stars = 22.0 * (hzf / 56.0);
        let lrgb =
            |s: &Self, k: usize| [f64::from(s.lr[k]), f64::from(s.lg[k]), f64::from(s.lb[k])];

        for r in 0..hz {
            let y = r as f64 + 0.5;
            for xi in 0..w {
                let x = xi as f64;
                let k = r * w + xi;
                let s = k * 3;
                let (mut cr, mut cg, mut cb) = (
                    f64::from(self.sky[s]),
                    f64::from(self.sky[s + 1]),
                    f64::from(self.sky[s + 2]),
                );
                let dx = x + 0.5 - sun[0];
                let dy = y - sun[1];
                let mut disc = 0.0;
                let ds = (dx * dx + (dy / 0.9).powi(2)).sqrt();
                if ds < SR + 0.6 {
                    // the disc, a little flattened, hot at the core and redder at the rim
                    let e = ds / SR;
                    disc = smooth(SR + 0.45, SR - 0.45, ds);
                    let e4 = e * e * e * e;
                    cr = mix(cr, 1.0, disc);
                    cg = mix(cg, 0.95 - 0.2 * e4, disc);
                    cb = mix(cb, 0.78 - 0.38 * e4, disc);
                }
                let near = (-(dx * dx + dy * dy * 2.5).sqrt() / 45.0).exp();
                let v = y / hzf;
                // the cloud deck, found on its plane: far rows are far away
                let dd_ = deck_d / (hzf - y + 2.5);
                // a band of heaped cloud mid-sky: clear above and clear in the low glow
                let far = smooth(4.8, 2.7, dd_) * smooth(1.08, 1.7, dd_);
                let mut cloud = 0.0;
                if far > 0.0 {
                    let su = dx * dd_ + d_up + 64200.0;
                    let sv = (dd_ - 0.9) * 10.0;
                    let iuf = su.floor();
                    let (fu, ivf) = (su - iuf, sv.floor());
                    let fv = sv - ivf;
                    let iu = iuf as usize % u;
                    let iu1 = (iu + 1) % u;
                    let a0 = ivf as usize * u;
                    let a1 = a0 + u;
                    let dk = |i: usize| f64::from(self.deck[i]);
                    let d0 = dk(a0 + iu) + (dk(a0 + iu1) - dk(a0 + iu)) * fu;
                    let d1 = dk(a1 + iu) + (dk(a1 + iu1) - dk(a1 + iu)) * fu;
                    let dd = d0 + (d1 - d0) * fv - 0.22 * (1.0 - far) * f64::from(self.thin_top[k]);
                    let c = smooth(0.56, 0.68, dd) * far.sqrt();
                    if c > 0.01 {
                        cloud = c;
                        let dl = |i: usize| f64::from(self.deck_lit[i]);
                        let l0 = dl(a0 + iu) + (dl(a0 + iu1) - dl(a0 + iu)) * fu;
                        let l1 = dl(a1 + iu) + (dl(a1 + iu1) - dl(a1 + iu)) * fu;
                        let l = l0 + (l1 - l0) * fv;
                        // undersides glow gold toward the sun and rose away from
                        // it, and more the lower they sit; thick cores stay dusk violet
                        let az = (-dx.abs() / 60.0).exp();
                        let low = v * v;
                        let thin = 1.0 - smooth(0.6, 0.76, dd);
                        let b = clamp(
                            smooth(0.45, 0.85, l).powf(1.5) * (0.55 + 0.25 * low + 0.35 * az)
                                + 0.3 * thin * az * low,
                        );
                        let warm = clamp(az * (0.1 + 1.2 * low));
                        let (hr, hg, hb) = (1.0, mix(0.45, 0.72, warm), mix(0.52, 0.38, warm));
                        let (sr, sg, sb) =
                            (mix(0.1, 0.2, v), mix(0.065, 0.07, v), mix(0.23, 0.27, v));
                        cr = mix(cr, mix(sr, hr, b), c);
                        cg = mix(cg, mix(sg, hg, b), c);
                        cb = mix(cb, mix(sb, hb, b), c);
                    }
                }
                // the low bars, dark against the glow with burning edges
                let sx = x + d_lo;
                let ixf = sx.floor();
                let fx = sx - ixf;
                let ix = ixf as usize;
                let i0 = r * CW + ix % CW;
                let i1 = r * CW + (ix + 1) % CW;
                let (c0, c1) = (f64::from(self.lo_cover[i0]), f64::from(self.lo_cover[i1]));
                // thinned over the sun and kept off the sky behind the headland
                let c = (c0 + (c1 - c0) * fx)
                    * (1.0 - 0.75 * (-(dx / 13.0).powi(2)).exp())
                    * (1.0 - 0.85 * smooth(76.0 * hs, 50.0 * hs, x));
                if c > 0.01 {
                    cloud = cloud.max(c);
                    let (l0, l1) = (f64::from(self.lo_lit[i0]), f64::from(self.lo_lit[i1]));
                    let l = l0 + (l1 - l0) * fx;
                    let rim = l.powf(2.0) * (0.35 + 0.9 * near);
                    let a = (c * 1.2).min(1.0) * 0.92;
                    cr = mix(cr, 0.3 + 0.7 * rim, a);
                    cg = mix(cg, 0.09 + 0.55 * rim, a);
                    cb = mix(cb, 0.24 + 0.18 * rim, a);
                }
                // the first stars, high up where the sky has gone to indigo
                let rf = r as f64;
                if rf < stars && cloud < 0.05 && hash(x, rf * 5.0 + 3.0) > 0.993 {
                    let tw =
                        0.55 + 0.45 * (t * (0.8 + hash(rf, x) * 1.6) + hash(x, rf) * 6.28).sin();
                    let st = tw * smooth(stars, 4.0, rf) * 0.75;
                    cr = cr.max(st * 0.95);
                    cg = cg.max(st * 0.85);
                    cb = cb.max(st);
                }
                // the sea is too rough to mirror the disc whole: it gives back glitter
                let keep = 1.0 - 0.75 * disc;
                self.skr[k] = (cr * keep) as f32;
                self.skg[k] = (cg * keep) as f32;
                self.skb[k] = (cb * keep) as f32;
                out[k] = if self.land[k] != 0 {
                    {
                        let c = lrgb(self, k);
                        halftone(&mut self.dots, r, xi, c, 0.02, 1.0)
                    }
                } else {
                    halftone(&mut self.dots, r, xi, [cr, cg, cb], 0.05, 1.0)
                };
            }
        }

        for r in hz..h {
            let y = r as f64 + 0.5;
            // the sea: each cell is a facet of water that mirrors whatever part
            // of the sky its tilt points it at
            let dz = y - hzf;
            let z = 36.0 / dz; // distance out
            let rows = 36.0 / (dz * dz); // how much sea one row spans
            let swell_amt = smooth(8.0, 26.0, dz);
            let hot = (-dz / 9.0).exp();
            let pw = 3.0 + dz * 0.8; // the glitter path, wider as it comes toward us
            let (fx, fy) = ((0.9 / (1.0 + dz * 0.06)) * 0.35, 1.6 / (1.0 + dz * 0.05));
            let fres = 0.24 + 0.46 * (-dz / 8.0).exp();
            let depth = smooth(hzf, hf, y);
            let fade = smooth(hf + 6.0, hf - 4.0, y);
            for xi in 0..w {
                let x = xi as f64;
                let k = r * w + xi;
                if self.land[k] != 0 {
                    out[k] = {
                        let c = lrgb(self, k);
                        halftone(&mut self.dots, r, xi, c, 0.02, 1.0)
                    };
                    continue;
                }
                let dx = x + 0.5 - sun[0];
                let xp = (x + 0.5 - w as f64 / 2.0) / dz;
                // the long swell rolls toward us; shorter chop rides on it
                let p =
                    13.0 * (z + 0.05 * xp) + 2.2 * fbm(xp * 0.35 + 3.0, z * 0.4, 2, 0.0) + t * 0.8;
                let (wv, slope) = (p.sin(), p.cos());
                let mut sz = 0.11 * slope * clamp(1.4 / (13.0 * rows));
                let mut sxl = 0.015 * slope;
                for &[kx, kz, kk, a, w, f] in &CHOP {
                    let ph = kk * (kx * xp + kz * z) - w * t + f;
                    let c = ph.cos() * a * clamp(1.6 / (kk * rows));
                    sz += c * kz;
                    sxl += c * kx;
                }
                let ry = js_round(hzf - 1.0 - dz * 0.85 - 70.0 * sz).clamp(0.0, hzf - 1.0);
                let rx = js_round(x - 60.0 * sxl).clamp(0.0, w as f64 - 1.0);
                let q = ry as usize * w + rx as usize;
                // violet near the horizon, deepening to indigo toward us,
                // broken into long horizontal ripples, each giving back a
                // little more or less of the sky
                let rip = noise(x * fx * 1.4 + 13.0 - t * 0.15, y * fy * 1.1 + t * 0.25, 0.0);
                let rf = fres * (0.5 + 1.1 * rip * rip);
                let mut cr = f64::from(self.skr[q]) * rf * 0.8 + 0.03 - 0.01 * depth;
                let mut cg = f64::from(self.skg[q]) * rf * 0.75 + 0.022;
                let mut cb = f64::from(self.skb[q]) * rf + 0.095 + 0.03 * depth;
                // ripple facets that catch the sun: short dashes far out, longer
                // close in, each turning toward the sun and away again
                let n = 0.55 * noise(x * fx + t * 0.3, y * fy * 1.3 - t * 0.4, 0.0)
                    + 0.45 * noise(x * fx * 1.7 - t * 0.4, y * fy * 2.0 + t * 0.3 + 40.0, 0.0);
                let wave = (p / (2.0 * PI)).floor();
                let crest = (0.5 + 0.5 * wv).powf(7.0)
                    * swell_amt
                    * smooth(0.4, 0.65, noise(xp * 2.5 + 7.0, wave * 3.7, 0.0));
                let path = (-(dx / pw).powi(2)).exp();
                let soft = (-(dx / (pw * 1.6)).powi(2)).exp();
                // the near face of a swell is in shadow
                if slope > 0.0 {
                    let d = 1.0 - 0.7 * swell_amt * slope;
                    cr *= d;
                    cg *= d;
                    cb *= d;
                }
                let th = 0.84 - 0.3 * path * (0.45 + 0.55 * hot);
                let glint = smooth(th, th + 0.08, n) * (0.25 + 0.75 * path) * soft;
                let white = path * path * smooth(th + 0.04, th + 0.2, n);
                cr += 0.2 * soft * (0.4 + hot) + glint * 1.1;
                cg += 0.1 * soft * (0.4 + hot) + glint * (0.55 + 0.3 * hot + 0.35 * white);
                cb += 0.04 * soft + glint * (0.2 + 0.25 * hot + 0.45 * white);
                // the crests catch it: gold in the path, rose out to the sides
                let catch = crest * (0.15 + 0.85 * soft) * (0.6 + 0.6 * n);
                cr += catch * 0.95;
                cg += catch * mix(0.3, 0.7, soft);
                cb += catch * mix(0.5, 0.3, soft);
                // the sun's own column: a gap under the disc, then broken dashes
                let dash = smooth(0.4, 0.7, noise(x * 0.18 + t * 0.25, y * 0.9 - t * 0.5, 0.0));
                let col = (-(dx / (1.0 + dz * 0.22)).powi(2)).exp()
                    * (-dz / 5.0).exp()
                    * (0.6 + 0.4 * n)
                    * smooth(0.0, 3.0, dz)
                    * dash;
                cr += col;
                cg += col * 0.8;
                cb += col * 0.45;
                // the headland upside down in the water, broken by ripples
                if xi < landw + 4 && dz < 26.0 {
                    let xs = x + 0.5 + 1.3 * (y * 1.1 + t * 1.2 + x * 0.05).sin();
                    let ix = (xs as i32).clamp(0, landw as i32 - 1) as usize;
                    let ft = self.l.shore(ix as f64 + 0.5);
                    if y > ft {
                        let my = (2.0 * ft - y).floor();
                        if my >= 0.0 && my < landh as f64 {
                            let m = my as usize * w + ix;
                            if self.land[m] != 0 {
                                let a = 0.8 * smooth(ft + 24.0, ft + 4.0, y);
                                cr = mix(cr, f64::from(self.lr[m]) * 0.8 + 0.02, a);
                                cg = mix(cg, f64::from(self.lg[m]) * 0.7 + 0.015, a);
                                cb = mix(cb, f64::from(self.lb[m]) * 0.85 + 0.05, a);
                            }
                        }
                        // a line of surf where rock meets water
                        let foam = smooth(ft + 1.6, ft + 0.3, y)
                            * smooth(0.45, 0.8, noise(x * 0.5 - t * 0.4, t * 0.3, 0.0));
                        cr += 0.5 * foam;
                        cg += 0.3 * foam;
                        cb += 0.35 * foam;
                    }
                }
                // and the sloop's, shorter and more broken
                if dz > 2.5 && dz < 14.0 && xi > boat - 8 && xi < boat + 8 {
                    let ix = (x + 0.5 + 0.9 * (y * 1.3 + t * 1.6).sin()) as usize;
                    let m = (2.0 * (hzf + 3.0) - y).floor() as usize * w + ix;
                    if self.land[m] == SLOOP {
                        let a = 0.7 * smooth(hzf + 14.0, hzf + 4.0, y) * (0.6 + 0.4 * rip);
                        cr = mix(cr, 0.04, a);
                        cg = mix(cg, 0.03, a);
                        cb = mix(cb, 0.1, a);
                    }
                }
                // the island's, short and dim
                if let Some([i0, i1]) = self.l.isle {
                    if dz < 7.0 && xi >= i0 && xi < i1 {
                        let ix = (x + 0.5 + 0.7 * (y * 1.4 + t * 1.4).sin()) as usize;
                        let m = (2.0 * (hzf + 0.8) - y).floor() as usize * w
                            + ix.min(w - 1);
                        if self.land[m] == ISLE {
                            let a = 0.6 * smooth(hzf + 7.0, hzf + 1.0, y) * (0.6 + 0.4 * rip);
                            cr = mix(cr, 0.1, a);
                            cg = mix(cg, 0.05, a);
                            cb = mix(cb, 0.16, a);
                        }
                    }
                }
                // a crisp dark line at the horizon for the sun to sit on
                if dz < 1.5 {
                    cr *= 0.55;
                    cg *= 0.55;
                    cb *= 0.55;
                }
                out[k] = halftone(&mut self.dots, r, xi, [cr, cg, cb], 0.18, fade);
            }
        }
    }
}
