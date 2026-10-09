//! Lightning: a fixed schedule of flashes, some carrying one of four bolts,
//! and the flicker inside the cloud between them.

use super::{BASE, FL, HZ, HZF, PERIOD};
use crate::ascii_rest::math::{clamp, hash};

/// Distance from (px, py) to the segment a-b.
fn seg_dist(px: f64, py: f64, ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
    let (dx, dy) = (bx - ax, by - ay);
    let l = dx * dx + dy * dy;
    let k = clamp(((px - ax) * dx + (py - ay) * dy) / if l != 0.0 { l } else { 1.0 });
    let (ex, ey) = (px - ax - dx * k, py - ay - dy * k);
    (ex * ex + ey * ey).sqrt()
}

/// One lightning segment: [ax, ay, bx, by, depth].
type Seg = [f64; 5];

fn walk(i: f64, mut x: f64, mut y: f64, len: f64, lean: f64, depth: f64, segs: &mut Vec<Seg>) {
    let mut s = 0.0;
    while s < len && y < HZF + 2.0 {
        let nx = x + (hash(i * 97.0 + s, depth * 13.0 + 41.0) - 0.5) * 3.2 + lean;
        let ny = y + 0.9 + hash(i * 31.0 + s, depth + 42.0) * 1.3;
        segs.push([x, y, nx, ny, depth]);
        if depth == 0.0 && hash(i * 7.0 + s, 43.0) > 0.84 {
            walk(
                i,
                nx,
                ny,
                3.0 + hash(s, i) * 5.0,
                (hash(s, i + 9.0) - 0.5) * 2.4,
                1.0,
                segs,
            );
        }
        (x, y) = (nx, ny);
        s += 1.0;
    }
}

pub struct Bolt {
    pub x: f64,
    pub field: Vec<f32>,
}

#[derive(Clone, Copy)]
pub struct Flash {
    pub i: f64,
    pub bolt: Option<usize>,
    pub cx: f64,
    pub cy: f64,
    pub bolt_on: f64,
    pub inside: f64,
}

/// A fixed schedule of flashes, some carrying a bolt, for a storm `sd`
/// columns east and `top` rows down from upstream's.
pub fn flash_at(t: f64, bolts: &[Bolt], sd: f64, top: f64) -> Option<Flash> {
    let n = (t / PERIOD).floor();
    if n > 0.0 && hash(n, 50.0) < 0.25 {
        return None; // some periods stay dark
    }
    let start = if n == 0.0 {
        -0.08
    } else {
        n * PERIOD + 0.2 + hash(n, 51.0) * 1.6
    };
    let l = t - start;
    if !(0.0..=0.9).contains(&l) {
        return None;
    }
    // a stroke and its restrikes, each a sharp rise and a fast decay
    let mut i = 0.0;
    let strikes = [0.0, 0.09 + hash(n, 52.0) * 0.06, 0.32 + hash(n, 53.0) * 0.2];
    for (j, &s) in strikes.iter().enumerate() {
        if l >= s {
            let (tau, a) = if j == 0 {
                (0.09, 1.0)
            } else {
                (0.06, 0.7 - j as f64 * 0.15)
            };
            i += (-(l - s) / tau).exp() * a;
        }
    }
    let bolt = (n == 0.0 || hash(n, 54.0) > 0.55).then_some((n % 4.0) as usize);
    let (cx, cy) = match bolt {
        Some(b) => (bolts[b].x, BASE - 8.0 + top),
        None => (
            115.0 + sd + hash(n, 55.0) * 45.0,
            14.0 + hash(n, 56.0) * 24.0 + top,
        ),
    };
    Some(Flash {
        i: i.min(1.3),
        bolt,
        cx,
        cy,
        bolt_on: if bolt.is_some() && l < 0.5 {
            (i * 1.4).min(1.0)
        } else {
            0.0
        },
        inside: 0.0,
    })
}

/// Between the big flashes, the storm keeps flickering inside itself.
pub fn flicker_at(t: f64, sd: f64, top: f64) -> Option<Flash> {
    let n0 = (t / FL).floor();
    let mut n = n0;
    while n >= n0 - 1.0 && n >= 0.0 {
        let l = t - (n * FL + 0.35 + hash(n, 70.0) * 0.75);
        if (0.0..=0.6).contains(&l) {
            let mut i = 0.0;
            let pulses = [0.0, 0.08 + hash(n, 71.0) * 0.1, 0.26 + hash(n, 72.0) * 0.18];
            for (j, &p) in pulses.iter().enumerate() {
                if l >= p {
                    let a = match j {
                        1 => 0.7,
                        2 => 0.45,
                        _ => 1.0,
                    };
                    i += (-(l - p) / 0.07).exp() * a;
                }
            }
            return Some(Flash {
                i: i.min(1.0) * (0.25 + 0.2 * hash(n, 73.0)),
                bolt: None,
                cx: 120.0 + sd + hash(n, 74.0) * 38.0,
                cy: 15.0 + hash(n, 75.0) * 25.0 + top,
                bolt_on: 0.0,
                inside: 1.0,
            });
        }
        n -= 1.0;
    }
    None
}

/// The four bolts, walked in upstream's rows and stored `top` rows down a
/// `w x h` frame, for a storm `sd` columns east of upstream's.
pub fn bolts(w: usize, h: usize, sd: f64, top: usize) -> Vec<Bolt> {
    (0..4)
        .map(|i| {
            let i = f64::from(i);
            let mut segs = Vec::new();
            walk(
                i,
                122.0 + sd + hash(i, 40.0) * 34.0,
                BASE - 4.0,
                40.0,
                (hash(i, 44.0) - 0.5) * 0.8,
                0.0,
                &mut segs,
            );
            let mut field = vec![99f32; w * h];
            for r in 25..HZ + 4 {
                for x2 in (80.0 + sd).max(0.0) as usize..w {
                    let x2f = x2 as f64;
                    let mut m: f64 = 99.0;
                    for &[ax, ay, bx, by, dep] in &segs {
                        if (x2f - ax).abs() > 14.0 && (x2f - bx).abs() > 14.0 {
                            continue;
                        }
                        let d = seg_dist(x2f + 0.5, r as f64 + 0.5, ax, ay, bx, by) + dep * 0.5;
                        if d < m {
                            m = d;
                        }
                    }
                    field[(r + top) * w + x2] = m as f32;
                }
            }
            Bolt {
                x: segs[0][0],
                field,
            }
        })
        .collect()
}
