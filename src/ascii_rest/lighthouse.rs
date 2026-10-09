//! lighthouse: a banded lighthouse on a heap of rocks at night. Its beam turns
//! round the lantern, long when it crosses the frame and a flash when it faces
//! us, lighting the haze and the waves beneath; surf bursts on the rocks.
//!
//! Drawn at the panel's size: the tower and its rocks scaled to the height
//! (or, on a narrow panel, to the width) and centred, the sea and the sky run
//! to every edge, and the beam reaches far enough to sweep the whole width.
//! The beam turns all the way round: in front of the tower on the half that
//! faces us, flaring over the lantern when square to us, behind it on the
//! other (`LIGHTHOUSE_BEAM_FRONT`, on by default; upstream only ever passes
//! behind). `LIGHTHOUSE_COLOR` (on) paints each part its own colour; with
//! both off, at upstream's 64x30, it is upstream's picture cell for cell.

use std::f64::consts::PI;
use std::ops::Range;

use super::math::{hash2, js_round, sign_or_one};
use super::{hex, text, Canvas};
use crate::grid::Cell;

const RAMP: &[u8] = b" .:-=+*#%@";
/// The beam's ramp in the haze.
const BEAM: &[u8] = b" .:-=+*#";
const COLS: usize = 64;
const ROWS: usize = 30;
/// Rows the beam carries into the haze at its longest.
const REACH: f64 = 22.0;
/// The lamp's row.
const LAMP: usize = 5;
const HORIZON: usize = 19;
/// Seconds a turn.
const PERIOD: f64 = 8.0;
/// The shaft's rows.
const TOP: usize = 9;
const FOOT: usize = 24;
/// Its half width at the top and the foot, in rows.
const WT: f64 = 2.1;
const WB: f64 = 3.1;
/// Rows to a band, dark and light in turn.
const BAND: usize = 3;
/// Boulders heaped round the foot, back to front, as [x, y, half width, half
/// height, waterline]: x in rows from the tower. The two behind it stand clear
/// of the sea.
const ROCKS: [[f64; 5]; 8] = [
    [-3.9, 21.1, 2.3, 1.7, 99.0],
    [3.9, 20.9, 2.4, 1.8, 99.0],
    [-6.6, 22.9, 2.3, 1.6, 23.9],
    [6.7, 22.7, 2.2, 1.5, 23.6],
    [-1.7, 23.5, 2.7, 1.8, 24.8],
    [3.1, 23.8, 2.5, 1.7, 25.1],
    [-9.3, 24.5, 1.5, 0.9, 25.0],
    [9.2, 24.2, 1.3, 0.8, 24.7],
];
/// The boulders the tower stands in front of.
const BEHIND: usize = 2;
/// Where surf strikes: the boulder, the side it breaks on (none for foam
/// alone), and when in the swell.
const SURF: [(usize, f64, f64); 6] = [
    (6, -1.0, 0.0),
    (7, 1.0, 0.45),
    (3, 1.0, 0.8),
    (2, -1.0, 0.3),
    (4, 0.0, 0.62),
    (5, 0.0, 0.15),
];
/// Seconds between breakers.
const SWELL: f64 = 3.2;

/// Lit from the upper left.
fn moon(u: f64, v: f64, nz: f64) -> f64 {
    (-0.5 * u - 0.45 * v + 0.75 * nz).max(0.0)
}

/// The beam in the haze through a cell: a cone seen side on, so the more it
/// turns toward or away from us the shorter, wider and brighter it looks.
/// `round` gives the two halves of the turn their own depth: brighter
/// swinging toward us, narrower and dimmer going away behind, where its far
/// end draws in to the tower and out the other side as `span * |c|` rather
/// than sitting clipped at the frame's edge until the last moment and
/// jumping across.
fn beam_at(dx: f64, dy: f64, c: f64, s: f64, reach: f64, round: bool, span: f64) -> f64 {
    let fore = c.abs().max(0.1);
    let ax = sign_or_one(c);
    let ay = (0.02 / fore).min(0.35);
    let m = ax.hypot(ay);
    let a = (dx * ax + dy * ay) / m;
    if a <= 0.0 {
        return 0.0;
    }
    let across = (dx * ay - dy * ax).abs() / m;
    let (wide, gain) = match (round, s < 0.0) {
        (false, true) => (1.0, 0.75),
        (false, false) => (1.0, 1.0),
        (true, true) => (1.0 + 0.4 * s, 0.55 + 0.45 * c.abs()),
        (true, false) => (1.0, 1.0 + 0.25 * s),
    };
    let tip = if round && s < 0.0 {
        let x = ((span * c.abs() - a) / (0.2 * span)).clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    } else {
        1.0
    };
    let half = 0.5 + a * (0.11 / fore) * wide;
    tip * (-(across / half).powf(4.0)).exp()
        * (-a / (reach * fore)).exp()
        * (1.0 / fore.powf(0.3)).min(1.6)
        * gain
}

/// Where the picture sits on the grid.
struct Layout {
    cols: usize,
    rows: usize,
    /// The tower's column.
    tc: usize,
    /// Rows here to upstream's.
    s: f64,
    /// Upstream's rows of sky added above its top, on a grid taller than the
    /// scaled picture.
    oy: f64,
    reach: f64,
    stars: i64,
    /// Colour by part (`PALETTE`) rather than upstream's one ink.
    colour: bool,
    /// The beam crosses in front of the tower on the half of its turn that
    /// faces us, and flares over the lantern when square to us. Upstream's
    /// beam always passes behind.
    round: bool,
}

impl Layout {
    /// Upstream's picture scaled to the grid's height and centred, its sea
    /// and sky run out to the sides and the beam's reach with them. A grid
    /// too narrow for the rocks at that scale scales to its width instead,
    /// and the rows left over go a little more to sky than to sea. At
    /// upstream's 64x30 this is upstream's layout exactly.
    fn fit(cols: usize, rows: usize, colour: bool, round: bool) -> Self {
        let s = (rows as f64 / ROWS as f64).min(cols as f64 / ROCKS_WIDE);
        let spare = rows as f64 / s - ROWS as f64;
        let half = cols as f64 / 4.0 / s;
        Self {
            cols,
            rows,
            tc: cols / 2,
            s,
            oy: (spare * 0.55).max(0.0),
            reach: REACH * (half / (COLS / 4) as f64).max(1.0),
            stars: (24 * cols * rows / (COLS * ROWS)) as i64,
            colour,
            round,
        }
    }

    /// Upstream's y (in its rows, from its top) at the middle of row `r`.
    #[inline]
    fn y(&self, r: usize) -> f64 {
        (r as f64 + 0.5) / self.s - self.oy
    }

    /// Upstream's row under row `r`; the added sky is its row 0.
    #[inline]
    fn row(&self, r: usize) -> usize {
        self.y(r).floor().max(0.0) as usize
    }
}

/// Columns the heap of rocks needs at upstream's scale, with a little sea
/// either side.
const ROCKS_WIDE: f64 = 44.0;

/// The colours, one per part: an amber beam fading to bronze in the
/// thin haze, a pale-gold lamp, the sea in two blues with the beam's road on
/// it in amber, a white tower banded red, grey-brown rocks, iron gallery and
/// roof, white surf, blue-white stars.
const HAZE: u8 = 0;
const BEAM_INK: u8 = 1;
const LIGHT: u8 = 2;
const DEEP: u8 = 3;
const SWELL_INK: u8 = 4;
const ROAD: u8 = 5;
const WHITE: u8 = 6;
const RED: u8 = 7;
const ROCK: u8 = 8;
const SURF_INK: u8 = 9;
const STAR: u8 = 10;
const IRON: u8 = 11;
const PALETTE: &[u32] = &[
    hex("#a8743a"),
    hex("#ffc860"),
    hex("#fff2c0"),
    hex("#2f5f9a"),
    hex("#6a9ad8"),
    hex("#ffd27a"),
    hex("#ece6dc"),
    hex("#d8443a"),
    hex("#8a8070"),
    hex("#f0f8ff"),
    hex("#c8d4ff"),
    hex("#9098a8"),
];

/// The lantern's cells on `rows`: index, upstream's column off the tower, and
/// the row's weight.
fn lantern_cells(lay: &Layout, rows: Range<usize>) -> impl Iterator<Item = (usize, i64, f64)> + '_ {
    let (cols, tc, sc) = (lay.cols, lay.tc, lay.s);
    let wide = (3.0 * sc).ceil() as i64;
    rows.flat_map(move |r| {
        let row = if lay.row(r) == LAMP { 1.0 } else { 0.85 };
        (-wide..=wide).filter_map(move |dc| {
            let x = js_round(dc as f64 / sc) as i64;
            (x.abs() <= 3).then(|| ((r * cols + tc).wrapping_add_signed(dc as isize), x, row))
        })
    })
}

struct Scene {
    lay: Layout,
    /// The lighthouse and its rocks never move: light as 0..1 where they are,
    /// -1 where they are not.
    still: Vec<f32>,
    /// What each still cell is, for the palette.
    still_tone: Vec<u8>,
    /// What each cell of `out` is this frame, likewise.
    tone: Vec<u8>,
    rock_at: Vec<i8>,
    stars: Vec<(usize, u8)>,
    /// Where each breaker strikes: x, y, side, phase.
    surf: [[f64; 4]; 6],
    out: Vec<u8>,
    /// The cells above the horizon the tower and lantern cover, which the
    /// beam crosses on the half of its turn that faces us; empty unless
    /// `lay.round`.
    cover: Vec<usize>,
    light: Vec<f32>,
    cells: [Cell; 128],
    /// The rows upstream's lantern rows land on, and the lamp's.
    lantern: Range<usize>,
    lamp: usize,
}

impl Scene {
    fn new(lay: Layout) -> Self {
        let (cols, rows, tc, sc) = (lay.cols, lay.rows, lay.tc, lay.s);
        let n = cols * rows;
        let mut still = vec![-1.0f32; n];
        let mut rock_at = vec![-1i8; n];
        let mut still_tone = vec![0u8; n];
        for r in 0..rows {
            let ro = lay.row(r);
            for c in 0..cols {
                let k = r * cols + c;
                let x = (c as f64 - tc as f64) / 2.0 / sc;
                let y = lay.y(r);
                // The rocks behind the tower first, then the tower, then the rocks before it.
                let boulder = |i: usize, still: &mut [f32], rock_at: &mut [i8], still_tone: &mut [u8]| {
                    let [bx, by, rx, ry, wl] = ROCKS[i];
                    let (u, v) = ((x - bx) / rx, (y - by) / ry);
                    let q = u * u + v * v;
                    if q >= 1.0 || y > wl + 0.5 {
                        return;
                    }
                    let nz = (1.0 - q).sqrt();
                    // A dark rim on each, so heaped boulders keep apart; a little grain.
                    still[k] = ((0.06 + 0.86 * moon(u, v, nz)) * (0.15 + nz * 1.8).min(1.0)
                        + 0.06 * (hash2(c as i64, r as i64) - 0.5))
                        as f32;
                    rock_at[k] = i as i8;
                    still_tone[k] = ROCK;
                };
                for i in 0..BEHIND {
                    boulder(i, &mut still, &mut rock_at, &mut still_tone);
                }
                // The shaft: banded, round, a door at its foot and a slit of a light in a band.
                if (TOP..=FOOT).contains(&ro) {
                    let w = WT + ((WB - WT) * (ro - TOP) as f64) / (FOOT - TOP) as f64;
                    if x.abs() <= w {
                        let u = x / (w + 0.25);
                        let nz = (1.0 - u * u).sqrt();
                        let dark = ((ro - TOP) / BAND) % 2 == 1;
                        let mut v =
                            (if dark { 0.24 } else { 0.92 }) * (0.2 + 0.8 * moon(u, 0.0, nz));
                        if (FOOT - 3..FOOT).contains(&ro) && x.abs() < 0.6 {
                            v = 0.03;
                        }
                        if ro == TOP + BAND + 1 && c == tc {
                            v = 0.03;
                        }
                        still[k] = v as f32;
                        rock_at[k] = -1;
                        still_tone[k] = if dark { RED } else { WHITE };
                    }
                }
                // The gallery: a rail of posts, and the deck under it in shadow.
                if ro == TOP - 2 && x.abs() <= 2.9 {
                    still_tone[k] = IRON;
                    still[k] = if x.abs() > 2.6 {
                        0.55
                    } else if (c as i64 - tc as i64) % 2 != 0 {
                        0.2
                    } else {
                        (0.6 - 0.25 * (x / 3.0)) as f32
                    };
                }
                if ro == TOP - 1 && x.abs() <= 3.0 {
                    still_tone[k] = IRON;
                    still[k] = (0.45
                        * (0.25 + 0.75 * moon(x / 3.2, 0.3, (1.0 - (x / 3.2).powf(2.0)).sqrt())))
                        as f32;
                }
                // The roof: a dome with a vent on top, moonlit on the left.
                if ro + 3 >= LAMP && ro + 2 <= LAMP {
                    let rw = if ro + 2 == LAMP { 1.85 } else { 1.15 };
                    if x.abs() <= rw {
                        let u = x / (rw + 0.3);
                        still[k] = (0.12 + 0.6 * moon(u, -0.4, (1.0 - u * u).sqrt())) as f32;
                        still_tone[k] = IRON;
                    }
                }
                if ro + 4 == LAMP && c == tc {
                    still[k] = 0.5;
                    still_tone[k] = IRON;
                }
                for i in BEHIND..ROCKS.len() {
                    boulder(i, &mut still, &mut rock_at, &mut still_tone);
                }
            }
        }
        // Stars, a few and faint.
        let hz = (0..rows).find(|&r| lay.row(r) >= HORIZON).unwrap_or(rows);
        let mut stars = Vec::new();
        for i in 0..lay.stars {
            let c = (hash2(i, 1) * cols as f64).floor() as usize;
            let r = (hash2(i, 2) * hz.saturating_sub(2) as f64).floor() as usize;
            if still[r * cols + c] < 0.0 && ((c as i64 - tc as i64).abs() as f64) > 4.0 * sc {
                let h = hash2(i, 3);
                let ch = if h < 0.2 {
                    b'*'
                } else if h < 0.6 {
                    b'.'
                } else {
                    MIDDOT
                };
                stars.push((r * cols + c, ch));
            }
        }
        // The surf: where each breaker strikes, at the waterline on its boulder's outer flank.
        let surf = SURF.map(|(i, side, ph)| {
            let [bx, _, rx, _, wl] = ROCKS[i];
            [
                tc as f64 + 2.0 * sc * (bx + rx * side * 0.95),
                (wl - 0.2 + lay.oy) * sc,
                side,
                ph,
            ]
        });
        let mut cells = [Cell::CLEAR; 128];
        for (b, cell) in cells.iter_mut().enumerate().take(0x7f).skip(0x20) {
            *cell = text::cell(b as u8 as char);
        }
        cells[MIDDOT as usize] = text::cell('·');
        let at = &lay;
        let lit = |want: usize| (0..rows).filter(move |&r| at.row(r) == want);
        let first = lit(LAMP - 1).chain(lit(LAMP)).next().unwrap_or(0);
        let last = lit(LAMP).chain(lit(LAMP + 1)).last().unwrap_or(first);
        let lamp = lit(LAMP).next().unwrap_or(first);
        let mut scene = Self {
            still,
            rock_at,
            stars,
            surf,
            out: vec![b' '; n],
            cover: Vec::new(),
            tone: vec![0; n],
            still_tone,
            light: vec![0.0; cols],
            cells,
            lantern: first..last + 1,
            lamp,
            lay,
        };
        if scene.lay.round {
            let mut over: Vec<bool> = (0..n)
                .map(|k| scene.still[k] >= 0.0 && scene.lay.row(k / cols) < HORIZON)
                .collect();
            for (k, _, _) in lantern_cells(&scene.lay, scene.lantern.clone()) {
                over[k] = true;
            }
            scene.cover = (0..n).filter(|&k| over[k]).collect();
        }
        scene
    }

    fn frame(&mut self, t: f64, cells: &mut [Cell]) {
        let (cols, rows, tc, sc) = (self.lay.cols, self.lay.rows, self.lay.tc, self.lay.s);
        let n = cols * rows;
        let th = -0.35 + (2.0 * PI * t) / PERIOD;
        let (c, s) = (th.cos(), th.sin());
        let face = s.max(0.0).powf(10.0); // the lens turned square to us
        let round = self.lay.round;
        let reach = self.lay.reach;
        // Lamp to the frame's edge and a little past, in upstream's units.
        let span = 1.1 * cols as f64 / 4.0 / sc;
        // Square to us, the cone is seen end on: a round bloom on the lamp
        // rather than a shaft to one side.
        let (bloom, end_on) = if round {
            (1.2 + 5.2 * face, 1.0 - 0.7 * face)
        } else {
            (1.2 + 2.2 * face, 1.0)
        };
        let offset = |r: usize, cc: usize| {
            let dx = (cc as f64 - tc as f64) / 2.0 / sc;
            (dx, self.lay.y(r) - (LAMP as f64 + 0.5))
        };
        let glare_at =
            |dx: f64, dy: f64| (-(dx.hypot(dy) / bloom).powf(2.0)).exp() * (0.5 + 1.2 * face);
        let beam = |dx: f64, dy: f64| beam_at(dx, dy, c, s, reach, round, span) * end_on + glare_at(dx, dy);
        let haze = |b: f64, r: usize, cc: usize| {
            (b * 6.0 + 0.35 * hash2(cc as i64, r as i64)).floor().min(7.0)
        };
        let (out, still, rock_at) = (&mut self.out, &self.still, &self.rock_at);
        let tone = &mut self.tone;
        out.fill(b' ');
        tone.fill(0);
        self.light.fill(0.0);
        let mut hz = rows;
        for r in 0..rows {
            let ro = self.lay.row(r);
            if ro >= HORIZON && hz == rows {
                hz = r;
            }
            for cc in 0..cols {
                let k = r * cols + cc;
                let (dx, dy) = offset(r, cc);
                let glare = glare_at(dx, dy);
                if ro < HORIZON {
                    let b = beam(dx, dy);
                    if r == self.lamp {
                        self.light[cc] = b as f32;
                    }
                    if still[k] >= 0.0 {
                        continue;
                    }
                    let i = haze(b, r, cc);
                    if i > 0.0 {
                        out[k] = BEAM[i as usize];
                        tone[k] = if i > 2.0 { BEAM_INK } else { HAZE };
                    }
                    continue;
                }
                // The sea: crests rolling in, finer toward the horizon, lit under the beam.
                let d = (r - hz + 1) as f64 / sc;
                let x = cc as f64 / (3.2 / d + 0.6);
                let wave = (x * 0.9 + d * 1.7 - t * 2.2 + (x * 0.31 + d).sin() * 1.5).sin()
                    + 0.6 * (x * 0.43 - d * 0.9 + t * 1.3).sin();
                let lit = (f64::from(self.light[cc]) * 0.9 + glare * 0.5).min(1.0)
                    * (-(d - 1.0) / 6.0).exp();
                if wave > 0.9 - 0.5 * lit {
                    (out[k], tone[k]) = if lit > 0.5 {
                        (b'=', ROAD)
                    } else if lit > 0.2 || d >= 3.0 {
                        (b'~', SWELL_INK)
                    } else {
                        (b'-', DEEP)
                    };
                } else if wave > 0.4 - 0.4 * lit && (d < 3.0 || lit > 0.15) {
                    out[k] = if d < 3.0 { b'.' } else { b'-' };
                    tone[k] = DEEP;
                } else if r == hz {
                    out[k] = b'_';
                    tone[k] = DEEP;
                }
            }
        }
        for &(k, ch) in &self.stars {
            if out[k] == b' ' {
                out[k] = ch;
                tone[k] = STAR;
            }
        }
        // The lighthouse and rocks over the rest, each boulder washed by the
        // swell at its own waterline; the lantern glows from within.
        let mut swell = [0.0; ROCKS.len()];
        for (i, w) in swell.iter_mut().enumerate() {
            let fi = i as f64;
            *w = ROCKS[i][4]
                + 0.45 * ((2.0 * PI * t) / SWELL + fi * 1.9).sin()
                + 0.25 * (t * 1.3 + fi).sin();
        }
        for k in 0..n {
            let v = f64::from(still[k]);
            if v < 0.0 {
                continue;
            }
            if rock_at[k] >= 0 && self.lay.y(k / cols) > swell[rock_at[k] as usize] {
                continue;
            }
            let i = js_round(v * 9.0).clamp(1.0, 9.0);
            out[k] = RAMP[i as usize];
            tone[k] = self.still_tone[k];
        }
        let glow = 0.75 + 0.25 * face;
        for (k, x, row) in lantern_cells(&self.lay, self.lantern.clone()) {
            let (edge, bar) = (x.abs() == 3, x.abs() == 2);
            let v = if edge {
                0.34
            } else if bar {
                0.2
            } else {
                glow * (1.0 - 0.1 * x.abs() as f64) * row
            };
            let i = js_round(v * 9.0).clamp(1.0, 9.0);
            out[k] = RAMP[i as usize];
            tone[k] = if edge || bar { IRON } else { LIGHT };
        }
        // Swinging toward us, the beam crosses the lantern and the tower top.
        if s > 0.0 {
            for &k in &self.cover {
                let (r, cc) = (k / cols, k % cols);
                let (dx, dy) = offset(r, cc);
                let i = haze(beam(dx, dy), r, cc);
                if i >= 3.0 {
                    out[k] = BEAM[i as usize];
                    tone[k] = if i >= 6.0 { LIGHT } else { BEAM_INK };
                }
            }
        }
        // The lamp, and the rays when it faces us.
        let l = self.lamp * cols + tc;
        out[l] = if face > 0.3 {
            b'@'
        } else if s > -0.2 {
            b'*'
        } else {
            b'o'
        };
        tone[l] = LIGHT;
        if face > 0.5 && self.lamp > 0 && self.lamp + 1 < rows {
            out[l - cols] = b'|';
            out[l + cols] = b'|';
            out[l - 1] = b'=';
            out[l + 1] = b'=';
            for k in [l - cols, l + cols, l - 1, l + 1] {
                tone[k] = LIGHT;
            }
            // Square to us, the lens flares: a star of rays over all of it.
            if round {
                let len = (face * 5.0 * sc).round() as i64;
                let (lr, lc) = (self.lamp as i64, tc as i64);
                for (dr, dc, ch) in [
                    (-1, 0, b'|'),
                    (1, 0, b'|'),
                    (0, -2, b'='),
                    (0, 2, b'='),
                    (-1, -2, b'\\'),
                    (1, 2, b'\\'),
                    (-1, 2, b'/'),
                    (1, -2, b'/'),
                ] {
                    for j in 1..=len {
                        let (rr, cc) = (lr + dr * j, lc + dc * j);
                        if rr < 0 || rr >= rows as i64 || cc < 0 || cc >= cols as i64 {
                            break;
                        }
                        let k = rr as usize * cols + cc as usize;
                        out[k] = if j * 3 > len * 2 && ch != b'=' { b'.' } else { ch };
                        tone[k] = LIGHT;
                    }
                }
            }
        }
        // Surf: a breaker bursts on a boulder, its spray thrown up in a fan
        // that falls back as drops.
        let reach = (4.0 * sc).ceil() as i64;
        for (n, &[x0, y0, side, ph]) in self.surf.iter().enumerate() {
            let u = (((t / SWELL + ph) % 1.0) + 1.0) % 1.0;
            let tt = u * SWELL;
            if tt > 1.6 {
                continue;
            }
            // Foam spreading along the waterline where it broke.
            let fr = (y0 + 0.7 * sc).floor();
            for j in -reach..=reach {
                let cx = js_round(x0) as i64 + j;
                let jf = j.abs() as f64;
                if tt < 1.3
                    && jf <= (1.0 + tt * 3.0) * sc
                    && cx >= 0
                    && cx < cols as i64
                    && fr < rows as f64
                {
                    let k = fr as usize * cols + cx as usize;
                    out[k] = if tt < 0.5 && jf < 2.0 * sc { b'=' } else { b'~' };
                    tone[k] = SURF_INK;
                }
            }
            if side == 0.0 {
                continue;
            }
            // The spray, a fan of drops thrown up and out off the rock, thick
            // at first, falling back as it thins.
            for j in 0..18i64 {
                let f = j as f64 / 17.0;
                let (h1, h2) = (hash2(j, n as i64 + 7), hash2(j, n as i64 + 19));
                let vx = side * (1.0 + 13.0 * f * f) + (h1 - 0.5) * 3.0;
                let vy = (11.0 - 6.0 * f) * (0.8 + 0.4 * h2);
                let up = vy - 18.0 * tt > 0.0;
                for back in 0..=u8::from(up) {
                    let ts = tt - f64::from(back) * 0.06;
                    let cx = js_round(x0 + vx * sc * ts);
                    let cy = js_round(y0 - vy * sc * ts + 9.0 * sc * ts * ts);
                    if ts < 0.0
                        || cy < 0.0
                        || cy >= rows as f64
                        || cx < 0.0
                        || cx >= cols as f64
                        || cy > y0 + 0.3 * sc
                    {
                        continue;
                    }
                    let k = cy as usize * cols + cx as usize;
                    if rock_at[k] >= 0 && tt > 0.2 {
                        continue;
                    }
                    let spray = back == 0 || out[k] == b' ' || out[k] == b'~';
                    if spray {
                        tone[k] = SURF_INK;
                    }
                    out[k] = if back == 1 {
                        if out[k] == b' ' || out[k] == b'~' {
                            b'\''
                        } else {
                            out[k]
                        }
                    } else if tt < 0.22 {
                        if f < 0.4 {
                            b'#'
                        } else {
                            b'*'
                        }
                    } else if up {
                        b'*'
                    } else if tt < 1.15 {
                        b':'
                    } else {
                        b'.'
                    };
                }
            }
        }
        if self.lay.colour {
            for ((cell, &b), &k) in cells.iter_mut().zip(out.iter()).zip(tone.iter()) {
                *cell = text::tint(self.cells[b as usize], u16::from(k));
            }
        } else {
            for (cell, &b) in cells.iter_mut().zip(out.iter()) {
                *cell = self.cells[b as usize];
            }
        }
    }
}

pub struct Lighthouse(Scene);

impl Canvas for Lighthouse {
    const NAME: &'static str = "lighthouse";
    #[cfg(test)]
    const COLS: usize = COLS;
    #[cfg(test)]
    const ROWS: usize = ROWS;
    const FPS: u32 = 20;
    const COLOR: &'static str = "LIGHTHOUSE_COLOR";
    const PALETTE: &'static [u32] = PALETTE;
    const INK: u32 = hex("#ffd27a");
    /// A night of deep navy.
    const GROUND: u32 = hex("#050b1a");
    #[cfg(test)]
    const UPSTREAM: &'static [(&'static str, &'static str)] = &[("LIGHTHOUSE_BEAM_FRONT", "0")];

    fn new(cols: usize, rows: usize, colour: bool) -> Self {
        let round = crate::env_num(&["LIGHTHOUSE_BEAM_FRONT"], 1, 0, 1) == 1;
        Self(Scene::new(Layout::fit(cols, rows, colour, round)))
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        self.0.frame(t, out);
    }
}

/// A byte for upstream's `·` star in the ASCII-only buffer.
const MIDDOT: u8 = 0x7f;
