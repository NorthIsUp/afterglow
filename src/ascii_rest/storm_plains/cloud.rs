//! The storm cloud as a height field: heaped domes for the towers, a flat
//! lens for the anvil and pouches hanging under it, all in the storm's own
//! columns.

use super::{BASE, SY};
use crate::ascii_rest::math::{fbm, hash, js_round, mix, noise, smooth};

fn dome(dx: f64, dy: f64, r: f64) -> f64 {
    let q = 1.0 - dx * dx - dy * dy;
    if q > 0.0 {
        r * q.sqrt()
    } else {
        0.0
    }
}

pub fn tower_c(y: f64) -> f64 {
    138.0 + (BASE - y) * 0.14
}

pub fn tower_hw(y: f64) -> f64 {
    23.0 - (BASE - y) * 0.08
}

fn anvil_top(x: f64) -> f64 {
    7.5 + 2.5 * smooth(150.0, 210.0, x) + 3.0 * smooth(112.0, 58.0, x)
}

pub fn anvil_bot(x: f64) -> f64 {
    21.0 - 9.0 * smooth(124.0, 62.0, x) - 7.0 * smooth(152.0, 210.0, x)
}

/// The shelf: a flat dark lip of cloud along the storm's leading edge.
pub fn shelf_top(x: f64) -> f64 {
    BASE - 2.4 + 1.2 * (noise(x * 0.2, 8.1, 0.0) - 0.5)
}

pub fn shelf_bot(x: f64) -> f64 {
    BASE + 2.4 + 1.8 * (fbm(x * 0.12, 8.7, 2, 0.0) - 0.5)
        - 2.0 * smooth(184.0, 199.0, x)
        - 2.0 * smooth(114.0, 104.0, x)
}

pub fn shelf_on(x: f64) -> f64 {
    smooth(103.0, 112.0, x) * smooth(199.0, 190.0, x)
}

/// The cloud's height, how much of it is anvil, and the anvil's sunlit top
/// edge, on a grid `sx` columns wide from frame column -2, for a storm `sd`
/// columns east of upstream's.
pub struct Cloud {
    pub sh: Vec<f32>,
    pub anv: Vec<f32>,
    pub lip: Vec<f32>,
}

impl Cloud {
    pub fn new(sx: usize, sd: f64) -> Self {
            // The towers are heaps of puffs: [x, y, radius, bulge toward us, flat].
            // Each puff is a dome in the height field, so it gets its own lit side
            // and shadow.
            let mut puffs: Vec<[f64; 5]> = Vec::new();
            let mut seed = 1.0;
            let mut rnd = || {
                let v = hash(seed, 911.0);
                seed += 1.0;
                v
            };
            let mut y = BASE - 4.5;
            while y > 14.0 {
                let (cx, hw) = (tower_c(y), tower_hw(y));
                let count = js_round((hw * 2.0) / 11.0).max(2.0);
                let mut i = 0.0;
                while i < count {
                    let u = ((i + 0.5) / count) * 2.0 - 1.0;
                    let rr = 7.0 + rnd() * 4.0;
                    let px = cx + u * (hw - rr * 0.55) + (rnd() - 0.5) * 3.0;
                    let py = y + (rnd() - 0.5) * 2.0;
                    puffs.push([px, py, rr, 4.0 * (1.0 - u * u * 0.85).sqrt(), 1.0]);
                    i += 1.0;
                }
                y -= 4.4;
            }
            // the overshooting top, bulging out of the anvil
            puffs.extend([
                [141.0, 8.5, 9.0, 2.0, 0.0],
                [133.0, 9.5, 6.0, 1.0, 0.0],
                [149.0, 10.0, 6.0, 1.0, 0.0],
            ]);
            // the flanking line: younger towers stepping down to the west
            for [cx, top, w] in [[104.0, 27.0, 9.0], [89.0, 34.0, 6.5], [77.0, 40.5, 3.8]] {
                let mut y = BASE - w * 0.4;
                while y > top + w * 0.6 {
                    let px = cx + (rnd() - 0.5) * w * 0.6;
                    puffs.push([px, y, w * (0.75 + rnd() * 0.2), 1.0, 1.0]);
                    y -= w * 0.75;
                }
                puffs.extend([
                    [cx - w * 0.35, top + w * 0.75, w * 0.62, 1.0, 1.0],
                    [cx + w * 0.3, top + w * 0.6, w * 0.7, 1.5, 1.0],
                    [cx, top + w * 0.45, w * 0.55, 2.0, 1.0],
                ]);
            }
            // one continuous low base under the line, so the towers stand on something
            let mut x = 66.0;
            while x < 126.0 {
                let py = BASE - 1.8 - rnd() * 1.2;
                let pr = 3.0 + rnd() * 1.6 + 1.6 * smooth(70.0, 110.0, x);
                puffs.push([x, py, pr, 0.4, 1.0]);
                x += 3.2 + rnd() * 1.6;
            }

            let mut sh = vec![0f32; sx * SY];
            let mut anv = vec![0f32; sx * SY]; // how much of the height is anvil
            let mut lip = vec![0f32; sx * SY]; // the anvil's sunlit top edge
            for r in 0..SY {
                for i in 0..sx {
                    let x = i as f64 - 2.0 + 0.5 - sd;
                    let y = r as f64 + 0.5;
                    // the towers, cut flat along the base
                    let mut tower: f64 = 0.0;
                    let cut = smooth(
                        BASE + 1.0,
                        BASE - 1.5,
                        y + 1.2 * (noise(x * 0.15, 3.3, 0.0) - 0.5),
                    );
                    for &[px, py, pr, pz, flat] in &puffs {
                        let (dx, dy) = ((x - px) / pr, (y - py) / pr);
                        if dx * dx + dy * dy >= 1.0 {
                            continue;
                        }
                        let v = (pr * 0.6 + pz)
                            * (1.0 - dx * dx - dy * dy).sqrt()
                            * if flat != 0.0 { cut } else { 1.0 };
                        if v > tower {
                            tower = v;
                        }
                    }
                    // the anvil: flat on top, a lens in section, combed by the wind
                    let fib = fbm(x * 0.035, y * 0.2, 3, 0.0) - 0.5;
                    let top = anvil_top(x) + 2.2 * fib;
                    let bot = anvil_bot(x) - 1.5 * fib - 3.0 * (fbm(x * 0.2, 7.0, 2, 0.0) - 0.5);
                    let (mid, half) = ((top + bot) / 2.0, ((bot - top) / 2.0).max(0.5));
                    let mut anvil = dome(0.0, (y - mid) / half, (half * 0.9).min(5.0))
                        * smooth(58.0, 72.0, x + 6.0 * fib);
                    let on_top = if anvil > 0.0 {
                        smooth(top + 3.2, top + 0.6, y)
                    } else {
                        0.0
                    };
                    // pouches of mammatus hanging under its western half
                    if x > 66.0 && x < 128.0 && y > bot - 2.0 && y < bot + 5.0 {
                        let row = ((x - 66.0) / 5.2).floor();
                        let off = row * 5.2 + 66.0 + 2.6 + (hash(row, 3.0) - 0.5) * 1.2;
                        let py = bot + 0.6 + hash(row, 4.0) * 1.2;
                        anvil = anvil.max(
                            dome((x - off) / 2.7, (y - py) / 2.3, 2.2)
                                * smooth(64.0, 76.0, x)
                                * smooth(130.0, 116.0, x),
                        );
                    }
                    let solid = tower;
                    let mut h = solid.max(anvil);
                    let a = if anvil > solid {
                        smooth(0.0, 2.0, anvil - solid)
                    } else {
                        0.0
                    };
                    // small billows on the towers; long streaks on the anvil, combed
                    // at a slight slant so they do not line up with the rows
                    let billow = fbm(x * 0.22, y * 0.24, 3, 0.0) - 0.5;
                    let sy = y * 0.18 + 0.9 * (noise(x * 0.04, 5.5, 0.0) - 0.5) + x * 0.012;
                    let streaky = fbm(x * 0.03 + 0.6 * noise(x * 0.08, y * 0.1, 0.0), sy, 3, 0.0) - 0.5;
                    h += (h / 2.5).min(1.0) * mix(3.0 * billow, 2.2 * streaky, a);
                    sh[r * sx + i] = h.max(0.0) as f32;
                    anv[r * sx + i] = a as f32;
                    lip[r * sx + i] = (on_top * a) as f32;
                }
            }
        Self { sh, anv, lip }
    }
}
