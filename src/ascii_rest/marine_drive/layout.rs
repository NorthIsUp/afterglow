//! Where marine drive's things sit in a `w x h` frame. Upstream's 200x100 is
//! the anchor every value moves from, so there it is exact.

use super::super::math::{clamp, smooth};

/// Towers at the point, by distance back from it: [offset, half width, top
/// row, warm light]. Upstream's cluster.
#[rustfmt::skip]
const CLUSTER: [[f64; 4]; 11] = [
    [-79.0, 2.5, 27.0, 1.0], [-69.0, 2.0, 22.0, 0.0], [-58.0, 3.0, 30.0, 0.0], [-50.0, 2.0, 25.0, 1.0],
    [-40.0, 3.0, 23.0, 0.0], [-33.5, 2.0, 15.0, 1.0], [-27.0, 3.5, 27.0, 0.0], [-20.0, 2.5, 19.0, 0.0],
    [-14.0, 3.0, 12.0, 1.0], [-7.5, 2.5, 21.0, 0.0], [-3.0, 2.0, 28.0, 1.0],
];

/// More towers mid-curve, further back than the cluster reaches upstream,
/// rising as a longer bay brings them into reach; furthest first.
#[rustfmt::skip]
const MID_CURVE: [[f64; 4]; 5] = [
    [-150.0, 3.0, 31.0, 1.0], [-136.0, 2.5, 24.0, 0.0],
    [-122.0, 3.0, 29.0, 1.0], [-109.0, 2.0, 32.0, 0.0], [-97.0, 2.5, 26.0, 0.0],
];

/// The tower on the hill at the near end.
const HILL_TOWER: [f64; 4] = [24.0, 4.5, 21.0, 0.0];

/// A ship: its lights `[column, row, r, g, b]` and its dark hull's rectangles
/// `[first row, last row, first column, last column]`, by offset from an
/// anchor column and the horizon.
type Lights = [(i32, i32, f64, f64, f64)];
type Hulls = [[i32; 4]];

/// A long freighter with its bridge aft, and a smaller boat further out,
/// anchored at the point: white mastheads, warm deck and cabin lights, a red
/// port light.
#[rustfmt::skip]
const FREIGHTER: (&Lights, &Hulls) = (
    &[
        (8, -5, 1.0, 1.0, 1.0), (14, -6, 1.0, 1.0, 1.0), (9, -3, 1.0, 0.78, 0.45), (11, -3, 1.0, 0.8, 0.5),
        (13, -4, 1.0, 0.82, 0.5), (15, -4, 1.0, 0.8, 0.5),
        (21, -4, 1.0, 1.0, 1.0), (20, -2, 1.0, 0.78, 0.45), (23, -2, 0.95, 0.22, 0.18),
    ],
    &[[-2, -1, 6, 16], [-3, -3, 7, 16], [-5, -4, 13, 15], [-5, -4, 8, 8], [-2, -1, 19, 23], [-3, -3, 20, 21]],
);

/// A fishing boat's lone lamp, past the freighter.
const FISHING: (&Lights, &Hulls) = (&[(36, -2, 1.0, 0.86, 0.6)], &[[-2, -1, 35, 37]]);

/// A liner lit end to end, anchored under the moon.
#[rustfmt::skip]
const LINER: (&Lights, &Hulls) = (
    &[
        (9, -6, 1.0, 1.0, 1.0), (17, -6, 1.0, 1.0, 1.0),
        (7, -3, 1.0, 0.84, 0.55), (9, -3, 1.0, 0.8, 0.5), (11, -3, 1.0, 0.86, 0.6), (13, -3, 1.0, 0.82, 0.5),
        (15, -3, 1.0, 0.84, 0.55), (17, -3, 1.0, 0.8, 0.5), (19, -3, 1.0, 0.86, 0.6),
        (10, -4, 1.0, 0.82, 0.55), (13, -4, 0.85, 0.9, 1.0), (16, -4, 1.0, 0.82, 0.55),
        (5, -2, 0.2, 1.0, 0.35), (22, -2, 0.95, 0.22, 0.18),
    ],
    &[[-2, -1, 4, 23], [-3, -3, 6, 20], [-4, -4, 9, 17], [-5, -5, 12, 14], [-6, -1, 9, 9], [-6, -1, 17, 17]],
);

pub(super) struct Layout {
    pub w: usize,
    pub h: usize,
    /// The sea horizon.
    pub hz: f64,
    /// Rows of sky added above upstream's picture: everything above the sea
    /// moves down by this.
    pub sky: f64,
    /// The point, where the drive ends.
    pub tip: f64,
    pub moon: [f64; 2],
    /// [centre, half width, top row, warm light].
    pub towers: Vec<[f64; 4]>,
    /// [column, row, r, g, b].
    pub ship_lights: Vec<(usize, usize, f64, f64, f64)>,
    /// [first row, last row, first column, last column].
    pub hulls: Vec<[usize; 4]>,
    /// The sky over the point: where it brightens [rise from, to, fall from, to],
    pub point_glow: [f64; 4],
    /// where the city's haze lifts behind its towers,
    pub point_haze: [f64; 4],
    /// and where low cloud is kept clear of them.
    pub point_clear: [f64; 4],
    /// Where the pale haze over the open sea rises in.
    pub open_sea: [f64; 2],
}

impl Layout {
    pub fn new(w: usize, h: usize) -> Self {
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
        // Whole rows and columns, so tower floors and ship lights stay on the grid.
        let sky = (tall * 0.55).round();
        let hz = 40.0 + sky;
        let tip = grow(176.0, 64.0, 104.0).round();
        let moon = [grow(189.0, 98.0, 104.0).round(), 13.0 + (sky * 0.6).round()];
        let shift = tip - 176.0;
        // How far back from the point the towers reach.
        let reach = grow(79.0, 43.0, 39.0);
        let mut towers = vec![HILL_TOWER];
        for &[off, hw, top, warm] in MID_CURVE.iter().chain(&CLUSTER) {
            if -off <= reach {
                towers.push([tip + off, hw, top + sky, warm]);
            }
        }

        let (mut ship_lights, mut hulls) = (Vec::new(), Vec::new());
        let mut moor = |(lights, hull): (&Lights, &Hulls), at: f64| {
            let (x0, r0) = (at as i32, hz as i32);
            for &(dx, dy, r, g, b) in lights {
                ship_lights.push(((x0 + dx) as usize, (r0 + dy) as usize, r, g, b));
            }
            for &[a, z, c0, c1] in hull {
                hulls.push([(r0 + a) as usize, (r0 + z) as usize, (x0 + c0) as usize, (x0 + c1) as usize]);
            }
        };
        moor(FREIGHTER, tip);
        // The liner wants open water under the moon, clear of the freighter.
        let liner = moon[0] + 4.0 > tip + 28.0 && moon[0] + 23.0 < wf;
        if liner {
            moor(LINER, moon[0]);
        }
        if tip + 37.0 < wf && (!liner || tip + 39.0 < moon[0] + 4.0) {
            moor(FISHING, tip);
        }

        Self {
            w,
            h,
            hz,
            sky,
            tip,
            moon,
            towers,
            ship_lights,
            hulls,
            point_glow: [grow(84.0, 24.0, 65.0), grow(98.0, 30.0, 65.0), 186.0 + shift, 174.0 + shift],
            point_haze: [grow(84.0, 24.0, 65.0), grow(104.0, 30.0, 65.0), 188.0 + shift, 172.0 + shift],
            point_clear: [grow(84.0, 24.0, 65.0), grow(96.0, 24.0, 65.0), 186.0 + shift, 176.0 + shift],
            open_sea: [172.0 + shift, 186.0 + shift],
        }
    }

    /// The shoreline: steep and near at the left, flat and far toward the point.
    pub fn shore(&self, x: f64) -> f64 {
        (50.0 + self.sky) + 34.0 * (1.0 - x / self.tip).max(0.0).powf(2.2)
    }

    /// How large one unit of distance along the drive looks at column x.
    pub fn sc(&self, x: f64) -> f64 {
        0.5 + 1.9 * (1.0 - x / self.tip).max(0.0).powf(1.5)
    }

    pub fn sea_top(&self, x: f64) -> f64 {
        if x <= self.tip {
            self.shore(x)
        } else {
            self.hz.max((50.0 + self.sky) - (x - self.tip) * 1.6)
        }
    }

    pub fn wall_top(&self, x: f64) -> f64 {
        self.shore(x) - self.sc(x).max(1.0)
    }

    pub fn road_top(&self, x: f64) -> f64 {
        self.wall_top(x) - (1.5 * self.sc(x)).max(1.0)
    }

    /// The city's sodium glow in the sky, weaker out over the open sea.
    pub fn glow(&self, x: f64, y: f64) -> f64 {
        let over = 0.5 + 0.5 * smooth(self.tip + 24.0, self.tip - 16.0, x);
        let h = (self.hz - y).max(0.0);
        over * ((-h / 4.0).exp() * 0.5 + (-h / 12.0).exp() * 0.12)
    }
}
