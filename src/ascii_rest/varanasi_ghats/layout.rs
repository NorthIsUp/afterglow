//! Where the ghats, their temples, priests and lamps sit in a `w x h` frame.

use crate::ascii_rest::math::{clamp, smooth};
use crate::ascii_rest::stretch::Stretch;

/// Temple spires: [centre x on upstream's ghats, height in rows, half width
/// at the base, the narrowest panel that has it]. `x` stretches with the
/// ghats.
const SPIRES: [[f64; 4]; 8] = [
    [118.0, 33.0, 5.2, 0.0],
    [64.0, 19.0, 4.0, 0.0],
    [31.0, 15.0, 3.6, 0.0],
    [139.0, 8.0, 1.6, 0.0],
    [90.0, 11.0, 2.4, 0.0],
    [129.0, 10.0, 2.0, 0.0],
    [78.0, 14.0, 2.8, 260.0],
    [46.5, 10.0, 2.6, 260.0],
];
/// Aarti stations on the near ghat, unevenly spaced: [x, streak width,
/// streak length, the narrowest panel that has it]. `x` stretches from the
/// first.
const AARTI: [[f64; 4]; 5] = [
    [11.0, 1.15, 1.25, 0.0],
    [24.0, 0.8, 0.85, 0.0],
    [39.0, 1.25, 1.1, 0.0],
    [54.0, 0.75, 0.75, 0.0],
    [66.0, 0.9, 0.8, 260.0],
];
/// Umbrellas on the far steps: [x, the narrowest panel that has it].
const UMBS: [[f64; 2]; 6] = [
    [70.0, 0.0],
    [83.0, 0.0],
    [98.0, 0.0],
    [109.0, 0.0],
    [121.5, 260.0],
    [132.0, 260.0],
];

/// Where things sit in a `w x h` frame. Upstream's 200x100 is the anchor
/// every value moves from, so there it is exact.
pub struct Layout {
    pub w: usize,
    pub h: usize,
    /// Rows of sky added above the far bank.
    pub sky: f64,
    /// The far bank.
    pub hz: f64,
    /// Where the ghats meet the far bank.
    pub end: f64,
    /// The afterglow, sitting on the far bank.
    pub glow: [f64; 2],
    /// Temple spires: [centre x, height in rows, half width at the base].
    pub spires: Vec<[f64; 3]>,
    /// Aarti stations on the near ghat: [x, streak width, streak length].
    pub aarti: Vec<[f64; 3]>,
    /// Umbrellas on the far steps.
    pub umbs: Vec<f64>,
    /// Where the buildings start to sit lower.
    pub low_from: f64,
    /// Where the far bank's tree clumps rise in.
    pub clumps: [f64; 2],
    /// The diyas on the steps: the column they stop at, and where they gather
    /// [centre, spread].
    pub step_lamps: (usize, [f64; 2]),
    /// Where the electric lamps along the far ghats start.
    pub lamps_from: f64,
    /// The floating diyas: how many, and the span they wrap over.
    pub diyas: (usize, f64),
    /// The cloud's drift at t = 0: the long streaks start over the glow.
    pub drift: f64,
    /// Where the boatman starts, the span he wraps over, and his waterline.
    pub boat: [f64; 3],
}

impl Layout {
    pub fn new(w: usize, h: usize) -> Self {
        let s = Stretch::new(w, h);
        let Stretch { w: wf, tall, .. } = s;
        let end = s.grow(150.0, 50.0, 80.0);
        // the ghats' furniture stretches with them
        let k = end / 150.0;
        let sky = (tall * 0.55).round();
        let hz = 60.0 + sky;
        let glow = [end + 6.0, hz - 1.5];
        let mut l = Self {
            w,
            h,
            sky,
            hz,
            end,
            glow,
            spires: Vec::new(),
            aarti: Vec::new(),
            umbs: Vec::new(),
            low_from: 110.0 * k,
            clumps: [end + 10.0, end + 22.0],
            step_lamps: ((118.0 * k) as usize, [33.0 * k, 36.0 * k]),
            lamps_from: 58.0 * k,
            diyas: (34 * (w + 60) * (h - hz as usize) / 10_400, wf + 60.0),
            drift: 720.0 - (glow[0] - 156.0),
            boat: [glow[0] + 3.0, wf + 50.0, 71.0 + sky + 0.5 * (tall - sky)],
        };
        // a narrow panel has no room for the slots that would crowd another
        for &[x, sh, sw, from] in &SPIRES {
            let x = x * k;
            let clear = l.spires.iter().all(|s| (x - s[0]).abs() >= 0.8 * (sw + s[2]));
            if wf >= from && clear {
                l.spires.push([x, sh, sw]);
            }
        }
        for &[x, sw, sl, from] in &AARTI {
            let x = 11.0 + (x - 11.0) * k;
            let u = 1.3 * l.sc_f(x);
            if wf >= from && l.aarti.last().is_none_or(|a| x - a[0] >= 3.5 * u) {
                l.aarti.push([x, sw, sl]);
            }
        }
        for &[x, from] in &UMBS {
            let x = x * k;
            let r = 3.0 * l.sc_f(x);
            if wf >= from && l.umbs.last().is_none_or(|&u| x - u >= 2.0 * r) {
                l.umbs.push(x);
            }
        }
        l
    }

    /// The waterline along the ghats, near at the left and running away to the right.
    pub fn wl_f(&self, x: f64) -> f64 {
        if x < self.end {
            self.hz + 0.5 + 20.0 * (1.0 - x / self.end).powf(1.6)
        } else {
            self.hz + 0.5
        }
    }

    pub fn sc_f(&self, x: f64) -> f64 {
        0.45 + 1.75 * (1.0 - x / self.end).max(0.0).powf(1.3)
    }

    pub fn step_top_f(&self, x: f64) -> f64 {
        self.wl_f(x)
            - 9.5 * self.sc_f(x) * (0.3 + 0.7 * smooth(self.end, self.end - 16.0, x))
    }

    /// The dusk sky's heat: low overhead, rising toward the horizon, hottest at the glow.
    pub fn sky_heat(&self, x: f64, y: f64) -> f64 {
        let v = clamp(y / self.hz);
        let dx = (x - self.glow[0]).abs();
        let dy = (self.glow[1] - y).abs();
        // a wide warm band along the horizon, and a small hot core on the bank
        let wide = 0.46 * (-dx / 55.0 - dy / 23.0).exp();
        let core = 0.24 * (-((dx / 13.0).powi(2) + (dy / 4.5).powi(2)).sqrt()).exp();
        0.06 + 0.42 * v * v + wide + core
    }
}
