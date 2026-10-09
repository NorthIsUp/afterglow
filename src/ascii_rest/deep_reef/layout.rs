//! Where the dive's reefs, coral and kelp sit in a `w x h` frame.

use crate::ascii_rest::math::{clamp, fbm};

// both reefs are rounded masses, shouldering down toward the open sand
fn dome(u: f64) -> f64 {
    1.0 - (1.0 - u * u).max(0.0).sqrt()
}

/// Brain corals on the left reef: [x on upstream's reef, depth of the centre
/// below the crest, radius, kind, the narrowest panel that has it]. `x`
/// stretches with the reef, from the first.
const LEFT_DOMES: [[f64; 5]; 6] = [
    [16.0, 3.0, 7.0, 0.0, 0.0],
    [29.0, 2.5, 5.5, 2.0, 240.0],
    [41.0, 2.5, 6.5, 1.0, 0.0],
    [53.0, 2.0, 4.5, 3.0, 0.0],
    [65.0, 2.0, 5.0, 2.0, 0.0],
    [78.0, 2.0, 4.0, 1.0, 240.0],
];
/// The right reef's, `x` back from its end.
const RIGHT_DOMES: [[f64; 5]; 4] = [
    [38.0, 2.0, 4.0, 2.0, 0.0],
    [24.0, 2.5, 5.0, 0.0, 240.0],
    [10.0, 3.0, 5.0, 1.0, 0.0],
    [0.0, 3.0, 4.5, 3.0, 0.0],
];
/// Bommies out on the sand: [x from the middle of the open water, crest
/// row, half width, fog, coral kind]. The first is the big one; the rest
/// stand clear of it and of the reefs, or not at all.
const BOMMIES: [[f64; 5]; 5] = [
    [2.0, 69.0, 9.0, 0.18, 0.0],
    [-18.0, 63.6, 4.0, 0.5, 2.0],
    [27.0, 64.4, 4.0, 0.42, 1.0],
    [52.0, 65.2, 5.0, 0.36, 3.0],
    [-31.0, 66.8, 3.5, 0.3, 1.0],
];

/// Where things sit in a `w x h` frame. Upstream's 200x100 is the anchor
/// every value moves from, so there it is exact.
pub struct Layout {
    pub w: usize,
    pub h: usize,
    /// Rows of open water added above the reef, and of reef and sand below.
    pub top: f64,
    pub bot: f64,
    /// Where the sea floor would meet the haze.
    pub hz: usize,
    /// Where the sun shows through the surface.
    pub sunx: f64,
    /// The column the sand runs away from.
    pub cam: f64,
    /// Left of here a cell belongs to the left reef.
    pub split: f64,
    /// Which side of a kelp the sun lies on.
    pub sun_side: f64,
    /// The left reef's reach, and where its fog thins out.
    pub left: [f64; 2],
    /// The right reef: where it ends, its reach, and the dip by its end.
    pub right: [f64; 3],
    /// The dip in the far reef.
    pub far_dip: f64,
    /// The middle of the open water, and how far its haze and its light reach.
    pub mid: [f64; 3],
    /// Where the sides start to darken.
    pub mid_dark: f64,
    /// Where the sand brightens toward the sun.
    pub lit: [f64; 2],
    /// The open sand between the reefs.
    pub open_sand: [f64; 2],
    /// Brain corals, domes on the crests: [x, depth of the centre below the
    /// crest, radius, kind].
    pub domes: Vec<[f64; 4]>,
    /// Bommies on the sand: [x, crest row, half width, fog, coral kind].
    pub bommies: Vec<[f64; 5]>,
    /// The sea fan's column.
    pub fan: f64,
    /// Branching coral on each crest: [seed, count, from, span].
    pub branches: [(u32, usize, f64, f64); 2],
    /// A third, hazier kelp out on the sand, on a panel wide enough.
    pub far_kelp: bool,
    /// Bubble streams: [x, y, bubbles].
    pub streams: Vec<(f64, f64, usize)>,
    /// Specks of marine snow.
    pub snow: usize,
    /// The school's loop: its centre column, its sway and its row.
    pub school: [f64; 3],
}

/// The slots of `slots` a `w`-wide panel has room for, at `x_at` of a slot's
/// first value: on the reef between `reef`, and not crowding the dome before.
fn domes_at(
    slots: &[[f64; 5]],
    w: f64,
    reef: [f64; 2],
    x_at: impl Fn(f64) -> f64,
) -> Vec<[f64; 4]> {
    let mut out: Vec<[f64; 4]> = Vec::new();
    for &[x, d, r, kind, from] in slots {
        let x = x_at(x);
        let crowded = out.last().is_some_and(|p| (x - p[0]).abs() < 0.7 * (r + p[2]));
        let on = x - 0.5 * r >= reef[0] && x + 0.5 * r <= reef[1];
        if w >= from && on && !crowded {
            out.push([x, d, r, kind]);
        }
    }
    out
}

impl Layout {
    pub fn new(w: usize, h: usize) -> Self {
        // `wide` is 1 at 3.2:1, the old `-wide` recomposition; `narrow` is 1
        // at square.
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
        // a portrait panel looks up through more water, and a little nearer
        let top = (tall * 0.75).round();
        let bot = tall - top;
        let left = [grow(86.0, 44.0, 50.0), grow(80.0, 40.0, 46.0)];
        let right = [wf - 4.0, grow(48.0, 32.0, 22.0), wf - 9.0];
        let open_sand = [left[0] + 1.0, right[0] - right[1] - 1.0];
        // the reefs' furniture stretches with them
        let (ls, rs) = (left[0] / 86.0, right[1] / 48.0);
        let mid = [grow(112.0, 60.0, 56.0), grow(100.0, 60.0, 50.0), grow(104.0, 60.0, 52.0)];
        let mut domes = domes_at(&LEFT_DOMES, wf, [0.0, left[0]], |x| 16.0 + (x - 16.0) * ls);
        let reef = [right[0] - right[1], wf];
        domes.extend(domes_at(&RIGHT_DOMES, wf, reef, |x| right[0] - x * rs));
        let squeeze = ((open_sand[1] - open_sand[0]) / 60.0).min(1.0);
        let big = mid[0] + BOMMIES[0][0];
        let mut bommies = vec![[big, BOMMIES[0][1] + top, BOMMIES[0][2], BOMMIES[0][3], BOMMIES[0][4]]];
        for &[dx, crest, hw, fog, kind] in &BOMMIES[1..] {
            let x = mid[0] + dx * squeeze;
            if x - hw > open_sand[0] && x + hw < open_sand[1] && (x - big).abs() >= hw + BOMMIES[0][2] {
                bommies.push([x, crest + top, hw, fog, kind]);
            }
        }
        let far_kelp = wf >= 260.0;
        let mut streams = vec![
            (44.0 * ls, 52.0 + top, 8),
            (right[0] - 28.0 * rs, 66.0 + top, 7),
            (mid[0] + 4.0, h as f64 - 4.0, 6),
        ];
        if far_kelp {
            streams.push((left[0] - 18.0, 74.0 + top, 5));
        }
        Self {
            w,
            h,
            top,
            bot,
            hz: 61 + top as usize,
            sunx: grow(136.0, 60.0, 72.0),
            cam: wf / 2.0,
            split: grow(100.0, 80.0, 47.0),
            sun_side: grow(100.0, 96.0, 50.0),
            left,
            right,
            far_dip: grow(120.0, 60.0, 60.0),
            mid,
            mid_dark: grow(50.0, 30.0, 25.0),
            lit: [grow(100.0, 60.0, 50.0), grow(130.0, 60.0, 65.0)],
            open_sand,
            domes,
            bommies,
            fan: right[0] - 29.0 * rs,
            branches: [
                (31, (10.0 * ls).round() as usize, 3.0, 60.0 * ls),
                (53, (3.0 * rs).round() as usize, right[0] - 18.0 * rs, 20.0 * rs),
            ],
            far_kelp,
            streams,
            snow: 90 * w * h / 20_000,
            school: [grow(95.0, 60.0, 48.0), grow(24.0, 10.0, 10.0), 44.0 + 0.5 * top],
        }
    }

    pub fn left_top(&self, x: f64) -> f64 {
        47.0 + self.top + (56.0 + self.bot) * dome((x / self.left[0]).min(1.0))
            - 8.0 * fbm(x * 0.06, 3.1, 4, 0.0)
            + 3.0 * ((10.0 - x) / 10.0).max(0.0)
    }

    pub fn right_top(&self, x: f64) -> f64 {
        69.0 + self.top + (34.0 + self.bot) * dome(((self.right[0] - x) / self.right[1]).min(1.0))
            - 5.0 * fbm(x * 0.08, 8.3, 3, 0.0)
            - 3.0 * (-((x - self.right[2]) / 6.0).powi(2)).exp()
    }

    pub fn far_top(&self, x: f64) -> f64 {
        self.hz as f64
            - 1.0
            - 6.0 * fbm(x * 0.035 + 2.0, 1.7, 3, 0.0)
            - 3.0 * (-((x - self.far_dip) / 18.0).powi(2)).exp()
    }

    pub fn top_at(&self, x: f64) -> f64 {
        if x < self.split {
            self.left_top(x)
        } else {
            self.right_top(x)
        }
    }
}
