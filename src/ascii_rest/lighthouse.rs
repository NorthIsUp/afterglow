//! lighthouse: a banded lighthouse on a heap of rocks at night. Its beam turns
//! round the lantern, long when it crosses the frame and a flash when it faces
//! us, lighting the haze and the waves beneath; surf bursts on the rocks.

use std::f64::consts::PI;

use super::math::{hash2, js_round, sign_or_one};
use super::{hex, text, Piece};
use crate::grid::Cell;

const RAMP: &[u8] = b" .:-=+*#%@";
/// The beam's ramp in the haze.
const BEAM: &[u8] = b" .:-=+*#";
/// The tower's column.
const TC: usize = 32;
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
fn beam_at(dx: f64, dy: f64, c: f64, s: f64) -> f64 {
    let fore = c.abs().max(0.1);
    let ax = sign_or_one(c);
    let ay = (0.02 / fore).min(0.35);
    let m = ax.hypot(ay);
    let a = (dx * ax + dy * ay) / m;
    if a <= 0.0 {
        return 0.0;
    }
    let across = (dx * ay - dy * ax).abs() / m;
    let half = 0.5 + a * (0.11 / fore);
    (-(across / half).powf(4.0)).exp()
        * (-a / (22.0 * fore)).exp()
        * (1.0 / fore.powf(0.3)).min(1.6)
        * if s < 0.0 { 0.75 } else { 1.0 }
}

pub struct Lighthouse {
    /// The lighthouse and its rocks never move: light as 0..1 where they are,
    /// -1 where they are not.
    still: Vec<f32>,
    rock_at: Vec<i8>,
    stars: Vec<(usize, u8)>,
    /// Where each breaker strikes: x, y, side, phase.
    surf: [[f64; 4]; 6],
    out: Vec<u8>,
    light: Vec<f32>,
    cells: [Cell; 128],
}

impl Piece for Lighthouse {
    const NAME: &'static str = "lighthouse";
    const COLS: usize = 64;
    const ROWS: usize = 30;
    const FPS: u32 = 20;
    const CELL: usize = 2;
    const PALETTE: &'static [u32] = &[hex("#ffd27a")];
    const GROUND: u32 = 0;

    fn new() -> Self {
        let (cols, rows) = (Self::COLS, Self::ROWS);
        let n = cols * rows;
        let mut still = vec![-1.0f32; n];
        let mut rock_at = vec![-1i8; n];
        for r in 0..rows {
            for c in 0..cols {
                let k = r * cols + c;
                let x = (c as f64 - TC as f64) / 2.0;
                let y = r as f64 + 0.5;
                // The rocks behind the tower first, then the tower, then the rocks before it.
                let boulder = |i: usize, still: &mut [f32], rock_at: &mut [i8]| {
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
                };
                for i in 0..BEHIND {
                    boulder(i, &mut still, &mut rock_at);
                }
                // The shaft: banded, round, a door at its foot and a slit of a light in a band.
                if (TOP..=FOOT).contains(&r) {
                    let w = WT + ((WB - WT) * (r - TOP) as f64) / (FOOT - TOP) as f64;
                    if x.abs() <= w {
                        let u = x / (w + 0.25);
                        let nz = (1.0 - u * u).sqrt();
                        let dark = ((r - TOP) / BAND) % 2 == 1;
                        let mut v =
                            (if dark { 0.24 } else { 0.92 }) * (0.2 + 0.8 * moon(u, 0.0, nz));
                        if (FOOT - 3..FOOT).contains(&r) && x.abs() < 0.6 {
                            v = 0.03;
                        }
                        if r == TOP + BAND + 1 && x == 0.0 {
                            v = 0.03;
                        }
                        still[k] = v as f32;
                        rock_at[k] = -1;
                    }
                }
                // The gallery: a rail of posts, and the deck under it in shadow.
                if r == TOP - 2 && x.abs() <= 2.9 {
                    still[k] = if x.abs() > 2.6 {
                        0.55
                    } else if (c as i64 - TC as i64) % 2 != 0 {
                        0.2
                    } else {
                        (0.6 - 0.25 * (x / 3.0)) as f32
                    };
                }
                if r == TOP - 1 && x.abs() <= 3.0 {
                    still[k] = (0.45
                        * (0.25 + 0.75 * moon(x / 3.2, 0.3, (1.0 - (x / 3.2).powf(2.0)).sqrt())))
                        as f32;
                }
                // The roof: a dome with a vent on top, moonlit on the left.
                if r + 3 >= LAMP && r + 2 <= LAMP {
                    let rw = if r + 2 == LAMP { 1.85 } else { 1.15 };
                    if x.abs() <= rw {
                        let u = x / (rw + 0.3);
                        still[k] = (0.12 + 0.6 * moon(u, -0.4, (1.0 - u * u).sqrt())) as f32;
                    }
                }
                if r + 4 == LAMP && x == 0.0 {
                    still[k] = 0.5;
                }
                for i in BEHIND..ROCKS.len() {
                    boulder(i, &mut still, &mut rock_at);
                }
            }
        }
        // Stars, a few and faint.
        let mut stars = Vec::new();
        for i in 0..24 {
            let c = (hash2(i, 1) * cols as f64).floor() as usize;
            let r = (hash2(i, 2) * (HORIZON - 2) as f64).floor() as usize;
            if still[r * cols + c] < 0.0 && (c as i64 - TC as i64).abs() > 4 {
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
                TC as f64 + 2.0 * (bx + rx * side * 0.95),
                wl - 0.2,
                side,
                ph,
            ]
        });
        let mut cells = [Cell::CLEAR; 128];
        for (b, cell) in cells.iter_mut().enumerate().take(0x7f).skip(0x20) {
            *cell = text::cell(b as u8 as char);
        }
        cells[MIDDOT as usize] = text::cell('·');
        Self {
            still,
            rock_at,
            stars,
            surf,
            out: vec![b' '; n],
            light: vec![0.0; cols],
            cells,
        }
    }

    fn frame(&mut self, t: f64, cells: &mut [Cell]) {
        let (cols, rows) = (Self::COLS, Self::ROWS);
        let n = cols * rows;
        let th = -0.35 + (2.0 * PI * t) / PERIOD;
        let (c, s) = (th.cos(), th.sin());
        let face = s.max(0.0).powf(10.0); // the lens turned square to us
        let (out, still, rock_at) = (&mut self.out, &self.still, &self.rock_at);
        out.fill(b' ');
        self.light.fill(0.0);
        for r in 0..rows {
            for cc in 0..cols {
                let k = r * cols + cc;
                let dx = (cc as f64 - TC as f64) / 2.0;
                let dy = r as f64 + 0.5 - (LAMP as f64 + 0.5);
                let glare =
                    (-(dx.hypot(dy) / (1.2 + 2.2 * face)).powf(2.0)).exp() * (0.5 + 1.2 * face);
                if r < HORIZON {
                    let b = beam_at(dx, dy, c, s) + glare;
                    if r == LAMP {
                        self.light[cc] = b as f32;
                    }
                    if still[k] >= 0.0 {
                        continue;
                    }
                    let i = (b * 6.0 + 0.35 * hash2(cc as i64, r as i64))
                        .floor()
                        .min(7.0);
                    if i > 0.0 {
                        out[k] = BEAM[i as usize];
                    }
                    continue;
                }
                // The sea: crests rolling in, finer toward the horizon, lit under the beam.
                let d = (r - HORIZON + 1) as f64;
                let x = cc as f64 / (3.2 / d + 0.6);
                let wave = (x * 0.9 + d * 1.7 - t * 2.2 + (x * 0.31 + d).sin() * 1.5).sin()
                    + 0.6 * (x * 0.43 - d * 0.9 + t * 1.3).sin();
                let lit = (f64::from(self.light[cc]) * 0.9 + glare * 0.5).min(1.0)
                    * (-(d - 1.0) / 6.0).exp();
                if wave > 0.9 - 0.5 * lit {
                    out[k] = if lit > 0.5 {
                        b'='
                    } else if lit > 0.2 || d >= 3.0 {
                        b'~'
                    } else {
                        b'-'
                    };
                } else if wave > 0.4 - 0.4 * lit && (d < 3.0 || lit > 0.15) {
                    out[k] = if d < 3.0 { b'.' } else { b'-' };
                } else if d == 1.0 {
                    out[k] = b'_';
                }
            }
        }
        for &(k, ch) in &self.stars {
            if out[k] == b' ' {
                out[k] = ch;
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
            if rock_at[k] >= 0 && (k / cols) as f64 + 0.5 > swell[rock_at[k] as usize] {
                continue;
            }
            let i = js_round(v * 9.0).clamp(1.0, 9.0);
            out[k] = RAMP[i as usize];
        }
        let glow = 0.75 + 0.25 * face;
        for r in LAMP - 1..=LAMP + 1 {
            for x in -3i64..=3 {
                let (edge, bar) = (x.abs() == 3, x.abs() == 2);
                let v = if edge {
                    0.34
                } else if bar {
                    0.2
                } else {
                    glow * (1.0 - 0.1 * x.abs() as f64) * if r == LAMP { 1.0 } else { 0.85 }
                };
                let i = js_round(v * 9.0).clamp(1.0, 9.0);
                out[(r * cols + TC).wrapping_add_signed(x as isize)] = RAMP[i as usize];
            }
        }
        // The lamp, and the rays when it faces us.
        let l = LAMP * cols + TC;
        out[l] = if face > 0.3 {
            b'@'
        } else if s > -0.2 {
            b'*'
        } else {
            b'o'
        };
        if face > 0.5 {
            out[l - cols] = b'|';
            out[l + cols] = b'|';
            out[l - 1] = b'=';
            out[l + 1] = b'=';
        }
        // Surf: a breaker bursts on a boulder, its spray thrown up in a fan
        // that falls back as drops.
        for (n, &[x0, y0, side, ph]) in self.surf.iter().enumerate() {
            let u = (((t / SWELL + ph) % 1.0) + 1.0) % 1.0;
            let tt = u * SWELL;
            if tt > 1.6 {
                continue;
            }
            // Foam spreading along the waterline where it broke.
            let fr = (y0 + 0.7).floor();
            for j in -4i64..=4 {
                let cx = js_round(x0) as i64 + j;
                let jf = j.abs() as f64;
                if tt < 1.3
                    && jf <= 1.0 + tt * 3.0
                    && cx >= 0
                    && cx < cols as i64
                    && fr < rows as f64
                {
                    out[fr as usize * cols + cx as usize] =
                        if tt < 0.5 && jf < 2.0 { b'=' } else { b'~' };
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
                    let cx = js_round(x0 + vx * ts);
                    let cy = js_round(y0 - vy * ts + 9.0 * ts * ts);
                    if ts < 0.0
                        || cy < 0.0
                        || cy >= rows as f64
                        || cx < 0.0
                        || cx >= cols as f64
                        || cy > y0 + 0.3
                    {
                        continue;
                    }
                    let k = cy as usize * cols + cx as usize;
                    if rock_at[k] >= 0 && tt > 0.2 {
                        continue;
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
        for (cell, &b) in cells.iter_mut().zip(out.iter()) {
            *cell = self.cells[b as usize];
        }
    }
}

/// A byte for upstream's `·` star in the ASCII-only buffer.
const MIDDOT: u8 = 0x7f;
