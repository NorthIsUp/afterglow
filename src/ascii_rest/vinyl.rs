//! vinyl: a record turning on a turntable, seen from above. The label and a
//! few specks of dust turn at 33 1/3 rpm, the sheen on the grooves stays put,
//! and a J-shaped tonearm rests in the outer grooves.

use std::f64::consts::PI;

use super::math::js_round;
use super::{hex, text, Piece};
use crate::grid::Cell;

const COLS: usize = 64;
const ROWS: usize = 25;
const RAMP: [Cell; 11] = text::cells([' ', '.', '·', ':', '-', '=', '+', '*', '#', '%', '@']);
/// 33 1/3 rpm, in radians per second.
const SPIN: f64 = (2.0 * PI * 100.0) / 3.0 / 60.0;
/// The record, in rows.
const R: f64 = 9.0;
/// The platter's edge.
const RIM: f64 = 9.9;
const LABEL: f64 = 4.0;
/// The smooth bands between tracks.
const GAPS: [f64; 2] = [6.2, 7.5];
/// Radius, angle.
const DUST: [(f64, f64); 3] = [(6.8, 0.4), (8.2, 2.9), (5.4, 4.4)];
/// The spindle, in cells.
const CX: f64 = 22.5;
const CY: f64 = 12.5;

/// The label's print, in its own frame (rows, x across): a title band above
/// the spindle and a round mark off to one side below it.
fn printed(x: f64, y: f64) -> bool {
    (y > -2.9 && y < -1.45 && x.abs() < 2.7) || (x - 1.5).hypot(y - 1.9) < 0.75
}

/// Light bars: two opposite wedges where the grooves catch a lamp to the
/// upper left. They stay put while the record turns under them.
fn sheen(th: f64) -> f64 {
    let d = (((th + 0.8) % PI) + PI) % PI - PI / 2.0;
    (-(d * 2.4).powi(2)).exp()
}

/// A record cell: index, radius in rows, the ring half-depth, the sheen, and
/// its label sample points (an empty range off the label).
struct Disc {
    k: usize,
    d: f64,
    h: f64,
    s: f64,
    sub: std::ops::Range<usize>,
}

/// Rim tally: samples, summed height in the cell, summed |cos|, falling count.
type Tally = [f64; 4];

pub struct Vinyl {
    cells: Vec<Disc>,
    subs: Vec<(f64, f64)>,
    edge: Vec<(usize, Cell)>,
}

fn phi_of(&[n, _, c, _]: &Tally) -> f64 {
    (c / n).atan2((1.0 - (c / n).powi(2)).max(0.0).sqrt()) * 180.0 / PI
}

impl Piece for Vinyl {
    const NAME: &'static str = "vinyl";
    const COLS: usize = COLS;
    const ROWS: usize = ROWS;
    const FPS: u32 = 24;
    const CELL: usize = 2;
    const PALETTE: &'static [u32] = &[hex("#ffb347")];
    const GROUND: u32 = 0;

    fn new() -> Self {
        // Per cell: radius in rows, angle, and the cell's radial depth, so a ring
        // drawn within half of it is one cell thick all the way round. Label cells
        // keep sixteen sample points so the print keeps its shape as it turns.
        let mut cells = Vec::new();
        let mut subs = Vec::new();
        for r in 0..ROWS {
            for c in 0..COLS {
                let dx = (c as f64 + 0.5 - CX) / 2.0;
                let dy = r as f64 + 0.5 - CY;
                let d = dx.hypot(dy);
                if d > R + 0.1 {
                    continue;
                }
                let th = dy.atan2(dx);
                let h = 0.5 * th.cos().abs() + th.sin().abs();
                let start = subs.len();
                if d < LABEL {
                    for j in 0..4 {
                        for i in 0..4 {
                            subs.push((
                                dx + (f64::from(i) - 1.5) / 8.0,
                                dy + (f64::from(j) - 1.5) / 4.0,
                            ));
                        }
                    }
                }
                cells.push(Disc {
                    k: r * COLS + c,
                    d,
                    h,
                    s: sheen(th),
                    sub: start..subs.len(),
                });
            }
        }

        // The platter's edge, traced once as an outline, each cell taking the
        // glyph for the slope and height of the curve inside it. Its upright
        // sides take one cell a row, and so does each slant. A Vec keeps the
        // insertion order upstream's Map iterates in, which breaks `most` ties.
        let mut rim: Vec<(i64, Tally)> = Vec::new();
        let cols = COLS as i64;
        let side = RIM * 0.3;
        let mut r = (CY - side - 0.5).ceil();
        while r + 0.5 < CY + side {
            let w = 2.0 * (RIM * RIM - (r + 0.5 - CY).powi(2)).sqrt();
            for c in [CX - w, CX + w] {
                let k = r as i64 * cols + c.floor() as i64;
                match rim.iter_mut().find(|e| e.0 == k) {
                    Some(e) => e.1 = [1.0, 0.5, 1.0, 0.0],
                    None => rim.push((k, [1.0, 0.5, 1.0, 0.0])),
                }
            }
            r += 1.0;
        }
        for i in 0..2000 {
            let p = (f64::from(i) / 2000.0) * 2.0 * PI;
            let x = CX + 2.0 * RIM * p.cos();
            let y = CY + RIM * p.sin();
            if (y.floor() + 0.5 - CY).abs() < side {
                continue;
            }
            let k = y.floor() as i64 * cols + x.floor() as i64;
            let at = match rim.iter().position(|e| e.0 == k) {
                Some(at) => at,
                None => {
                    rim.push((k, [0.0; 4]));
                    rim.len() - 1
                }
            };
            let e = &mut rim[at].1;
            e[0] += 1.0;
            e[1] += y - y.floor();
            e[2] += p.cos().abs();
            e[3] += if p.sin() * p.cos() < 0.0 { 1.0 } else { 0.0 };
        }
        let id = |k: i64| (k / cols) * 2 + i64::from((k % cols) as f64 >= CX);
        // The fullest slanted cell on each row and side.
        let mut most: Vec<(i64, i64)> = Vec::new();
        for &(k, e) in &rim {
            let phi = phi_of(&e);
            let best = most
                .iter()
                .find(|m| m.0 == id(k))
                .and_then(|m| rim.iter().find(|r| r.0 == m.1))
                .map_or(0.0, |r| r.1[0]);
            if phi > 50.0 && phi <= 72.0 && e[0] > best {
                match most.iter_mut().find(|m| m.0 == id(k)) {
                    Some(m) => m.1 = k,
                    None => most.push((id(k), k)),
                }
            }
        }
        let edge = rim
            .iter()
            .map(|&(k, e)| {
                let [n, fy, _, fall] = e;
                let phi = phi_of(&e);
                let slant = most.iter().any(|m| m.0 == id(k) && m.1 == k);
                let g = if phi > 72.0 {
                    '|'
                } else if slant {
                    if fall / n > 0.5 {
                        '\\'
                    } else {
                        '/'
                    }
                } else if fy / n < 0.42 {
                    '\''
                } else if fy / n > 0.58 {
                    if phi > 10.0 {
                        '.'
                    } else {
                        '_'
                    }
                } else {
                    '-'
                };
                (k as usize, text::cell(g))
            })
            .collect();

        Self {
            cells,
            subs,
            edge,
        }
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        let put = |out: &mut [Cell], c: f64, r: f64, g: char| {
            let (c, r) = (c.floor(), r.floor());
            if c >= 0.0 && c < COLS as f64 && r >= 0.0 && r < ROWS as f64 {
                out[r as usize * COLS + c as usize] = text::cell(g);
            }
        };
        let words = |out: &mut [Cell], c: usize, r: usize, s: &str| {
            for (i, g) in s.chars().enumerate() {
                if g != ' ' {
                    put(out, (c + i) as f64, r as f64, g);
                }
            }
        };

        let a = SPIN * t;
        let (ca, sa) = (a.cos(), a.sin());
        out.fill(RAMP[0]);
        for &(k, g) in &self.edge {
            out[k] = g;
        }
        for cell in &self.cells {
            let r = cell.d;
            let s = cell.s;
            let ring = |g: &f64| (r - g).abs() < cell.h / 2.0;
            if r < 0.6 {
                continue;
            }
            let b = if r < LABEL {
                // Turn each sample back into the label's frame and count the print.
                let sub = &self.subs[cell.sub.clone()];
                let ink = sub
                    .iter()
                    .filter(|&&(x, y)| printed(x * ca + y * sa, y * ca - x * sa))
                    .count();
                if ink > sub.len() / 2 {
                    5
                } else {
                    9
                }
            } else if r > R - 0.45 {
                4 + js_round(2.0 * s) as usize
            } else if r < LABEL + 1.0 {
                2 + js_round(2.0 * s) as usize
            } else if GAPS.iter().any(ring) {
                2 + js_round(3.0 * s) as usize
            } else {
                3 + js_round(4.0 * s) as usize
            };
            out[cell.k] = RAMP[b];
        }
        put(out, CX, CY, 'o');
        // Dust riding round with the record.
        for (r, p) in DUST {
            put(
                out,
                CX + 2.0 * r * (p + a).cos(),
                CY + r * (p + a).sin(),
                '°',
            );
        }

        // The plinth, its start and speed buttons, and the pitch slider.
        for c in 1..COLS - 1 {
            put(out, c as f64, 0.0, '─');
            put(out, c as f64, (ROWS - 1) as f64, '─');
        }
        for r in 1..ROWS - 1 {
            put(out, 0.0, r as f64, '│');
            put(out, (COLS - 1) as f64, r as f64, '│');
        }
        put(out, 0.0, 0.0, '╭');
        put(out, (COLS - 1) as f64, 0.0, '╮');
        put(out, 0.0, (ROWS - 1) as f64, '╰');
        put(out, (COLS - 1) as f64, (ROWS - 1) as f64, '╯');
        words(out, 2, 21, "┌──┐");
        words(out, 2, 22, "└──┘");
        words(out, 47, 21, "┌┐┌┐");
        words(out, 47, 22, "└┘└┘");
        for r in 12..=22 {
            put(out, 59.0, r as f64, if r == 17 { '═' } else { '┊' });
        }

        // The tonearm: counterweight behind the pivot, the pivot in its ring,
        // a tube down and round to the headshell, and the cue lever beside it.
        words(out, 49, 2, "▗▄▄▄▖");
        words(out, 49, 3, "▝▀█▀▘");
        words(out, 48, 4, "╭──╨──╮");
        words(out, 48, 5, "│  O  │");
        words(out, 48, 6, "╰──╥──╯");
        for r in 7..16 {
            put(out, 51.0, r as f64, '║');
        }
        words(out, 41, 16, "══════════╝");
        words(out, 36, 16, "▐███▌");
        words(out, 54, 9, "╭╮");
        words(out, 54, 10, "││");
        words(out, 54, 11, "╰╯");
    }
}
