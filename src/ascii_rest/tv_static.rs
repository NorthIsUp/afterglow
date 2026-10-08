//! tv-static: an old set showing snow, with a hum bar rolling through it. The
//! dial clicks over, a test card rolls into place and holds, then is lost.
//!
//! Drawn at the panel's size in one of two forms. By default the panel is the
//! screen: the tube's rounded corners meet its edges inside a thin bezel, the
//! snow, hum bar and card fill all of it, and the channel dial is set into the
//! bezel's foot. `TV_STATIC_SET=1` draws upstream's set instead, scaled to the
//! panel's height (or width) and centred, its screen as big as the set allows;
//! a panel too small for a legible set gets the screen form. Upstream's `set`
//! option is that form, always on.
//!
//! `TV_STATIC_COLOR` (on by default) paints either form: the card's bars in
//! their own colours, a grey cabinet or bezel and an amber dial. With the set
//! on and colour off, at upstream's 58x26, it is upstream's picture cell for
//! cell.

use super::math::{js_round, smooth};
use super::{hex, text, Canvas};
use crate::grid::Cell;

const COLS: usize = 58;
const ROWS: usize = 26;
const FPS: u32 = 20;
const RAMP: [Cell; 8] = text::cells([' ', '.', ':', '+', '░', '▒', '▓', '█']);
const DIAL: [Cell; 4] = text::cells(['╱', '─', '╲', '│']);
/// Seconds: snow, tuning in, the card, losing it, snow.
const LOOP: f64 = 10.0;
/// Ramp levels, brightest bar first.
const BARS: [i32; 7] = [7, 6, 5, 4, 3, 2, 1];
/// The reversed strip under the bars.
const CASTLE: [i32; 7] = [1, 0, 3, 0, 5, 0, 7];
const BOTTOM: [(f64, i32); 8] = [
    (0.17, 5),
    (0.34, 7),
    (0.51, 3),
    (0.68, 0),
    (0.73, 1),
    (0.78, 0),
    (0.83, 2),
    (1.0, 0),
];
/// Where the snow steps up a level.
const SNOW: [f64; 7] = [0.36, 0.48, 0.6, 0.7, 0.8, 0.9, 0.98];
const N: i32 = RAMP.len() as i32 - 1;

/// The picture area inside the set, and where its dial is.
struct Layout {
    cols: usize,
    x0: usize,
    y0: usize,
    sw: usize,
    sh: usize,
    dial: usize,
    /// Snow ids a row: wider than a row, so no two cells share one.
    stride: i32,
}

impl Layout {
    /// The whole grid inside a one-cell bezel.
    fn fit(cols: usize, rows: usize) -> Self {
        let (sw, sh) = (cols.saturating_sub(2).max(1), rows.saturating_sub(2).max(1));
        Self {
            cols,
            x0: usize::from(cols > 2),
            y0: usize::from(rows > 2),
            sw,
            sh,
            dial: (rows - 1) * cols + cols.saturating_sub(6),
            stride: 97.max(sw as i32),
        }
    }
}

/// This piece's own hash: three inputs and murmur-style finalising, unlike
/// `math::hash`.
#[inline]
fn hash(a: i32, b: i32, c: i32) -> f64 {
    let mut h = (a as u32).wrapping_mul(0x27d4_eb2d)
        ^ (b as u32).wrapping_mul(0x1656_67b1)
        ^ (c as u32).wrapping_mul(0x2545_f491);
    h = (h ^ (h >> 15)).wrapping_mul(0x85eb_ca6b);
    h = (h ^ (h >> 13)).wrapping_mul(0xc2b2_ae35);
    f64::from(h ^ (h >> 16)) / 4_294_967_296.0
}

/// Upstream's set as it lands on a `cols x rows` grid: its own 58x26
/// drawing, every coordinate scaled to the largest set of that shape the grid
/// holds and the whole centred, with the picture area inside its screen.
/// None when that set would be too small to read.
fn draw_set(cols: usize, rows: usize) -> Option<(Vec<Vec<char>>, Layout)> {
    let h = rows.min(cols * ROWS / COLS);
    let w = cols.min((h as f64 * COLS as f64 / ROWS as f64).round() as usize);
    if w < COLS * 2 / 3 || h < ROWS * 2 / 3 {
        return None;
    }
    let (ox, oy) = ((cols - w) / 2, (rows - h) / 2);
    let (fx, fy) = ((w - 1) as f64 / (COLS - 1) as f64, (h - 1) as f64 / (ROWS - 1) as f64);
    let x = |v: usize| ox + (v as f64 * fx).round() as usize;
    let y = |v: usize| oy + (v as f64 * fy).round() as usize;
    let mut g = vec![vec![' '; cols]; rows];
    let mut boxed = |(x0, y0, x1, y1): (usize, usize, usize, usize), c: [char; 6]| {
        let (x0, y0, x1, y1) = (x(x0), y(y0), x(x1), y(y1));
        g[y0][x0 + 1..x1].fill(c[4]);
        g[y1][x0 + 1..x1].fill(c[4]);
        for row in &mut g[y0 + 1..y1] {
            row[x0] = c[5];
            row[x1] = c[5];
        }
        g[y0][x0] = c[0];
        g[y0][x1] = c[1];
        g[y1][x0] = c[2];
        g[y1][x1] = c[3];
    };
    let round = ['╭', '╮', '╰', '╯', '─', '│'];
    boxed((1, 5, 56, 23), round);
    boxed((3, 6, 43, 22), round);
    boxed((46, 8, 52, 10), round); // the channel knob
    boxed((46, 12, 52, 14), round); // the volume knob
    boxed((45, 16, 53, 21), ['┌', '┐', '└', '┘', '─', '│']); // the speaker
    for row in &mut g[y(17)..=y(20)] {
        row[x(46)..=x(52)].fill('═');
    }
    g[y(13)][x(49)] = '╲';
    // The antenna's base, as tall as the gap down to the cabinet.
    let foot: Vec<char> = "▄▄███▄▄".chars().collect();
    for (n, row) in g[y(4)..y(5)].iter_mut().enumerate() {
        for (i, c) in row[x(25)..=x(31)].iter_mut().enumerate() {
            *c = if n == 0 {
                foot[((i as f64 / fx).round() as usize).min(6)]
            } else {
                '█'
            };
        }
    }
    // Rods and legs: straight runs between upstream's ends, a cell a row,
    // reaching the base and the cabinet however far apart scaling sets them.
    let mut line = |(r0, c0): (usize, usize), (r1, c1): (usize, usize), ch: char| {
        let (c0, c1) = (c0 as f64, c1 as f64);
        let lo = r0.min(r1);
        for (n, row) in g[lo..=r0.max(r1)].iter_mut().enumerate() {
            let f = if r0 == r1 {
                0.0
            } else {
                ((lo + n) as f64 - r0 as f64) / (r1 as f64 - r0 as f64)
            };
            row[(c0 + (c1 - c0) * f).round() as usize] = ch;
        }
    };
    line((y(23) + 1, x(5)), (y(25), x(4)), '╱');
    line((y(23) + 1, x(52)), (y(25), x(53)), '╲');
    line((y(4) - 1, x(24)), (y(0), x(21)), '╲');
    line((y(4) - 1, x(32)), (y(0), x(35)), '╱');
    g[y(0)][x(21)] = 'o';
    g[y(0)][x(35)] = 'o';
    let (sw, sh) = (x(43) - x(3) - 1, y(22) - y(6) - 1);
    let lay = Layout {
        cols,
        x0: x(3) + 1,
        y0: y(6) + 1,
        sw,
        sh,
        dial: y(9) * cols + x(49),
        stride: 97.max(sw as i32),
    };
    Some((g, lay))
}

/// The screen form's set: a bezel round the panel's edge, the dial in its foot.
fn draw_bezel(cols: usize, rows: usize) -> Vec<Vec<char>> {
    let mut g = vec![vec![' '; cols]; rows];
    if cols < 3 || rows < 3 {
        return g;
    }
    let (r1, c1) = (rows - 1, cols - 1);
    g[0][1..c1].fill('─');
    g[r1][1..c1].fill('─');
    for row in &mut g[1..r1] {
        row[0] = '│';
        row[c1] = '│';
    }
    (g[0][0], g[0][c1], g[r1][0], g[r1][c1]) = ('╭', '╮', '╰', '╯');
    if cols >= 10 {
        g[r1][cols - 7] = '(';
        g[r1][cols - 5] = ')';
    }
    g
}

#[derive(Clone, Copy)]
enum Px {
    Level(i32),
    Glyph(Cell),
}

/// The test card: bars over a reversed strip and a bottom strip of hard-edged
/// blocks, with a circle and a crosshair drawn across the middle.
fn draw_card(sw: usize, sh: usize) -> Vec<Px> {
    let mut card: Vec<Px> = (0..sw * sh)
        .map(|i| {
            let (r, c) = (i / sw, i % sw);
            let u = (c as f64 + 0.5) / sw as f64;
            let v = (r as f64 + 0.5) / sh as f64;
            let bar = ((u * 7.0).floor() as usize).min(6);
            Px::Level(if v < 0.67 {
                BARS[bar]
            } else if v < 0.75 {
                CASTLE[bar]
            } else {
                BOTTOM.iter().find(|&&(end, _)| u < end).unwrap().1
            })
        })
        .collect();
    let mut set = |r: usize, c: i64, ch: char| {
        if (0..sw as i64).contains(&c) {
            card[r * sw + c as usize] = Px::Glyph(text::cell(ch));
        }
    };
    // The circle, row by row: its span in each row, outlined in box drawing.
    let rad = (sh as f64 * 0.43).min(sw as f64 * 0.22) * 2.0;
    let (cx, cy) = (sw as f64 / 2.0, sh as f64 / 2.0);
    let span: Vec<Option<(i64, i64)>> = (0..sh)
        .map(|r| {
            let y = (r as f64 + 0.5 - cy) * 2.0;
            let w = rad * rad - y * y;
            (w >= 0.0).then(|| {
                (
                    (cx - w.sqrt() - 0.5).ceil() as i64,
                    (cx + w.sqrt() - 0.5).floor() as i64,
                )
            })
        })
        .collect();
    let (mid, mc) = (sh / 2, (sw / 2) as i64);
    for r in 0..sh {
        let Some((a, b)) = span[r] else { continue };
        let up = r < mid;
        let near = if up {
            r.checked_sub(1).and_then(|i| span[i])
        } else {
            span.get(r + 1).copied().flatten()
        };
        // The run out to the row nearer the middle's edge, stepped with corners.
        let (na, nb) = near.unwrap_or((mc + 1, mc - 1));
        for c in a..=b {
            if (c > a && c < na && c < nb) || (c < b && c > nb && c > na) {
                set(r, c, '─');
            }
        }
        if a < na {
            set(r, a, if up { '╭' } else { '╰' });
            set(r, na, if up { '╯' } else { '╮' });
            set(r, b, if up { '╮' } else { '╯' });
            set(r, nb, if up { '╰' } else { '╭' });
        } else {
            set(r, a, '│');
            set(r, b, '│');
        }
        if near.is_none() {
            for c in a + 1..b {
                set(r, c, '─');
            }
        }
    }
    // The crosshair, meeting the circle in tees.
    let (a, b) = span[mid].unwrap();
    for c in a + 1..b {
        set(mid, c, '─');
    }
    for (r, sp) in span.iter().enumerate() {
        if sp.is_some() {
            set(r, mc, '│');
        }
    }
    set(mid, a, '├');
    set(mid, b, '┤');
    set(mid, mc, '┼');
    set(span.iter().position(Option::is_some).unwrap(), mc, '┬');
    set(span.iter().rposition(Option::is_some).unwrap(), mc, '┴');
    card
}

/// The colours: snow in the set's blue-white (entry 0, which the
/// snow's cells already carry), the card's bars in their real colours, a
/// grey bezel and an amber dial.
const WHITE: u16 = 1;
/// White, yellow, cyan, green, magenta, red, blue, as the bars run.
const BAR_TONES: [u16; 7] = [1, 2, 3, 4, 5, 6, 7];
/// Under the bars, reversed: blue, magenta, cyan and white between black.
const CASTLE_TONES: [u16; 7] = [7, 0, 5, 0, 3, 0, 1];
const BEZEL: u16 = 8;
const DIAL_INK: u16 = 9;
/// Upstream's ink first, so colour off draws exactly what upstream does.
const PALETTE: &[u32] = &[
    hex("#cfe6ff"),
    hex("#e8e8e8"),
    hex("#f0e040"),
    hex("#40e0e8"),
    hex("#50e050"),
    hex("#e050e0"),
    hex("#f04040"),
    hex("#4c6cff"),
    hex("#6a7080"),
    hex("#ffb347"),
];

/// Each card cell's colour: its bar's, or white for the
/// circle, the crosshair and the bottom strip.
fn card_tones(card: &[Px], sw: usize, sh: usize) -> Vec<u16> {
    (0..sw * sh)
        .map(|i| {
            let (r, c) = (i / sw, i % sw);
            let v = (r as f64 + 0.5) / sh as f64;
            let bar = (((c as f64 + 0.5) / sw as f64 * 7.0).floor() as usize).min(6);
            match card[i] {
                Px::Level(_) if v < 0.67 => BAR_TONES[bar],
                Px::Level(_) if v < 0.75 => CASTLE_TONES[bar],
                Px::Glyph(_) | Px::Level(_) => WHITE,
            }
        })
        .collect()
}

struct Scene {
    lay: Layout,
    set: Vec<Cell>,
    card: Vec<Px>,
    /// `card_tones`; empty with colour off, the card in one ink.
    tone: Vec<u16>,
    /// Which picture cells the curved tube leaves in.
    inside: Vec<bool>,
}

impl Scene {
    fn new(lay: Layout, set: Vec<Vec<char>>, colour: bool) -> Self {
        let (sw, sh) = (lay.sw, lay.sh);
        let inside = (0..sw * sh)
            .map(|i| {
                let (r, c) = ((i / sw) as f64, (i % sw) as f64);
                let half = (sw as f64 / 2.0, sh as f64 / 2.0);
                let x = (c + 0.5 - half.0) / half.0;
                let y = (r + 0.5 - half.1) / half.1;
                x.powi(6) + y.powi(6) <= 1.02
            })
            .collect();
        let ink = if colour { BEZEL } else { 0 };
        let card = draw_card(sw, sh);
        Self {
            set: set
                .into_iter()
                .flatten()
                .map(|c| text::tint(text::cell(c), ink))
                .collect(),
            tone: if colour {
                card_tones(&card, sw, sh)
            } else {
                Vec::new()
            },
            card,
            inside,
            lay,
        }
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        let Layout {
            cols,
            x0,
            y0,
            sw,
            sh,
            dial: dial_at,
            stride,
        } = self.lay;
        let u = ((t % LOOP) + LOOP) % LOOP;
        // The snow is new every frame.
        let k = (t * f64::from(FPS)).floor() as i32;
        // How firmly the set holds the signal. Just before it locks, and just
        // before it is lost, the dial clicks through three stops.
        let lock = smooth(1.6, 3.4, u) * (1.0 - smooth(6.6, 7.8, u));
        let dial = ((u - 1.1) * 8.0)
            .floor()
            .min(3.0 - ((u - 6.4) * 8.0).floor())
            .clamp(0.0, 3.0);
        let snow = 1.0 - 0.97 * lock;
        // Unlocked, the picture rolls (with its blanking bar) and its lines tear.
        let roll = (1.0 - lock).powi(2) * (sh + 2) as f64 * 2.5;
        let tear = (1.0 - lock) * 9.0;
        // The hum bar's middle row, once a loop.
        let hum = ((u / LOOP + 0.45) % 1.0) * (sh + 6) as f64 - 3.0;
        let lock6 = lock.powi(6);

        out.copy_from_slice(&self.set);
        out[dial_at] = DIAL[dial as usize];
        if !self.tone.is_empty() {
            out[dial_at] = text::tint(out[dial_at], DIAL_INK);
        }
        for r in 0..sh {
            let (ri, rf) = (r as i32, r as f64);
            let dim = 1.0 - 0.45 * (-((rf - hum) / 2.2).powi(2)).exp();
            // Each scan line a little brighter or darker.
            let line = 0.9 + 0.2 * hash(k, ri, 9);
            let shift = js_round(tear * (rf * 0.8 + u * 9.0).sin() * hash(k >> 2, ri, 3)) as i64;
            // The card row shown here.
            let pr = ((rf + roll) % (sh + 2) as f64).floor() as usize;
            let mut prev = hash(k, ri, 1);
            for c in 0..sw {
                let o = (y0 + r) * cols + x0 + c;
                if !self.inside[r * sw + c] {
                    out[o] = RAMP[0];
                    continue;
                }
                // Snow is fine grain streaked along the line, as it is on a real
                // tube, mostly dots with now and then a brighter fleck.
                let id = ri * stride + c as i32;
                let n = hash(k, id, 2);
                prev = 0.6 * n + 0.4 * prev;
                let noisy = hash(k, id, 5) < snow;
                out[o] = if noisy && hash(k, id, 6) >= lock6 {
                    let v = prev * line * dim;
                    let mut level = 0;
                    while level < SNOW.len() && v > SNOW[level] {
                        level += 1;
                    }
                    RAMP[level]
                } else {
                    let sw = sw as i64;
                    let pc = (((c as i64 + shift) % sw) + sw) % sw;
                    let mut x = if pr < sh {
                        self.card[pr * sw as usize + pc as usize]
                    } else {
                        Px::Level(0)
                    };
                    // Once locked, what noise is left only nudges the card a shade.
                    if let (true, Px::Level(l)) = (noisy, x) {
                        x = Px::Level((l + if n < 0.5 { -1 } else { 1 }).clamp(0, N));
                    }
                    let cell = match x {
                        Px::Level(l) => RAMP[l as usize],
                        Px::Glyph(g) => g,
                    };
                    match self.tone.get(pr * sw as usize + pc as usize) {
                        Some(&tone) if pr < sh => text::tint(cell, tone),
                        _ => cell,
                    }
                };
            }
        }
    }
}

pub struct TvStatic(Scene);

impl Canvas for TvStatic {
    const NAME: &'static str = "tv-static";
    #[cfg(test)]
    const COLS: usize = COLS;
    #[cfg(test)]
    const ROWS: usize = ROWS;
    const FPS: u32 = FPS;
    #[cfg(test)]
    const UPSTREAM: &'static [(&'static str, &'static str)] =
        &[("TV_STATIC_SET", "1"), ("TV_STATIC_COLOR", "0")];

    fn new(cols: usize, rows: usize) -> Self {
        let set = crate::env_num(&["TV_STATIC_SET"], 0, 0, 1) == 1;
        let colour = crate::env_num(&["TV_STATIC_COLOR"], 1, 0, 1) == 1;
        let (chars, lay) = set
            .then(|| draw_set(cols, rows))
            .flatten()
            .unwrap_or_else(|| (draw_bezel(cols, rows), Layout::fit(cols, rows)));
        Self(Scene::new(lay, chars, colour))
    }

    fn palette(&self) -> &'static [u32] {
        if self.0.tone.is_empty() {
            &PALETTE[..1]
        } else {
            PALETTE
        }
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        self.0.frame(t, out);
    }
}
