//! vinyl: a record turning on a turntable, seen from above. The label and a
//! few specks of dust turn at 33 1/3 rpm, the sheen on the grooves stays put,
//! and a J-shaped tonearm rests in the outer grooves.
//!
//! `vinyl-wide` is a DJ's console across the panel: two decks, the right one
//! pitched a little up so the pair drift in and out of phase, either side of
//! a mixer whose meters jump to the beat, whose EQ knobs and faders get
//! nudged, and whose crossfader sweeps slowly from deck to deck. A single
//! turntable stretched to 3.2:1 would be a record and a lot of plinth; two
//! decks and a mixer is the shape that object really has at that width. The
//! decks are upstream's turntable at upstream's size on pine, scaled to the
//! panel's height (and whatever width two of them leave) elsewhere.

use std::f64::consts::PI;

use super::math::{hash2, js_round};
use super::{hex, text, Canvas, Piece};
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
/// The spindle, in cells from the deck's left edge; it sits half way down.
const CX: f64 = 22.5;
/// Columns a deck takes beyond its record's reach: the tonearm, the pitch
/// slider and the plinth's edge.
const ARM: f64 = 22.5;
/// The mixer's beat, a second.
const BEAT: f64 = 124.0 / 60.0;
const DIAL: [char; 4] = ['╱', '─', '╲', '│'];

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

/// The twin's colours: an amber plinth, a brass tonearm, a black record
/// whose grooves catch a grey sheen, a silver rim and spindle, each deck's
/// label in its own colour printed in cream, white dust; on the mixer green,
/// yellow and red meters, cyan knobs, white fader caps in a grey box.
const PLINTH: u16 = 0;
const BRASS: u16 = 1;
const GROOVE: u16 = 2;
const SHEEN: u16 = 3;
const SILVER: u16 = 4;
const RED_LABEL: u16 = 5;
const PRINT: u16 = 6;
const DUST_INK: u16 = 7;
const METER: [u16; 3] = [8, 9, 10];
const KNOB: u16 = 11;
const CAP: u16 = 12;
const PANEL: u16 = 13;
const BLUE_LABEL: u16 = 14;
const WIDE_PALETTE: &[u32] = &[
    hex("#ffb347"),
    hex("#e0b050"),
    hex("#4a4a54"),
    hex("#b0b0c0"),
    hex("#d8d8e0"),
    hex("#d83a3a"),
    hex("#ffe9c0"),
    hex("#ffffff"),
    hex("#50e070"),
    hex("#f0e040"),
    hex("#ff4a3a"),
    hex("#50d8f0"),
    hex("#f0f0f0"),
    hex("#8a8a96"),
    hex("#3a6ad8"),
];

/// Where one turntable sits on the grid and how its record turns.
struct Spec {
    /// The deck's left edge, in columns.
    ox: f64,
    /// Upstream's sizes to this deck's.
    s: f64,
    /// Turns a second, as a share of 33 1/3 rpm.
    rate: f64,
    /// Added to every speck of dust's angle.
    dust: f64,
    /// Rows the pitch slider's cap sits below its middle.
    pitch: i64,
    /// The twin's label colour.
    label: u16,
}

const ORIGINAL: Spec = Spec {
    ox: 0.0,
    s: 1.0,
    rate: 1.0,
    dust: 0.0,
    pitch: 0,
    label: 0,
};

/// One turntable: its record, traced once, and where its furniture goes.
struct Deck {
    cx: f64,
    cy: f64,
    s: f64,
    rate: f64,
    dust: f64,
    pitch: i64,
    label: u16,
    cells: Vec<Disc>,
    subs: Vec<(f64, f64)>,
    edge: Vec<(usize, Cell)>,
}

fn phi_of(&[n, _, c, _]: &Tally) -> f64 {
    (c / n).atan2((1.0 - (c / n).powi(2)).max(0.0).sqrt()) * 180.0 / PI
}

impl Deck {
    fn new(cols: usize, rows: usize, spec: &Spec) -> Self {
        let s = spec.s;
        let (cx, cy) = (spec.ox + CX * s, rows as f64 / 2.0);
        let (r_, rim) = (R * s, RIM * s);
        // Per cell: radius in rows, angle, and the cell's radial depth, so a ring
        // drawn within half of it is one cell thick all the way round. Label cells
        // keep sixteen sample points so the print keeps its shape as it turns.
        let mut cells = Vec::new();
        let mut subs = Vec::new();
        for r in 0..rows {
            for c in 0..cols {
                let dx = (c as f64 + 0.5 - cx) / 2.0;
                let dy = r as f64 + 0.5 - cy;
                let d = dx.hypot(dy);
                if d > r_ + 0.1 * s {
                    continue;
                }
                let th = dy.atan2(dx);
                let h = 0.5 * th.cos().abs() + th.sin().abs();
                let start = subs.len();
                if d < LABEL * s {
                    for j in 0..4 {
                        for i in 0..4 {
                            subs.push((
                                (dx + (f64::from(i) - 1.5) / 8.0) / s,
                                (dy + (f64::from(j) - 1.5) / 4.0) / s,
                            ));
                        }
                    }
                }
                cells.push(Disc {
                    k: r * cols + c,
                    d: d / s,
                    h: h / s,
                    s: sheen(th),
                    sub: start..subs.len(),
                });
            }
        }

        // The platter's edge, traced once as an outline, each cell taking the
        // glyph for the slope and height of the curve inside it. Its upright
        // sides take one cell a row, and so does each slant. A Vec keeps the
        // insertion order upstream's Map iterates in, which breaks `most` ties.
        let mut rim_at: Vec<(i64, Tally)> = Vec::new();
        let (icols, irows) = (cols as i64, rows as i64);
        let on = |r: f64, c: f64| r >= 0.0 && (r as i64) < irows && c >= 0.0 && (c as i64) < icols;
        let side = rim * 0.3;
        let mut r = (cy - side - 0.5).ceil();
        while r + 0.5 < cy + side {
            let w = 2.0 * (rim * rim - (r + 0.5 - cy).powi(2)).sqrt();
            for c in [cx - w, cx + w] {
                if !on(r, c.floor()) {
                    continue;
                }
                let k = r as i64 * icols + c.floor() as i64;
                match rim_at.iter_mut().find(|e| e.0 == k) {
                    Some(e) => e.1 = [1.0, 0.5, 1.0, 0.0],
                    None => rim_at.push((k, [1.0, 0.5, 1.0, 0.0])),
                }
            }
            r += 1.0;
        }
        for i in 0..2000 {
            let p = (f64::from(i) / 2000.0) * 2.0 * PI;
            let x = cx + 2.0 * rim * p.cos();
            let y = cy + rim * p.sin();
            if (y.floor() + 0.5 - cy).abs() < side || !on(y.floor(), x.floor()) {
                continue;
            }
            let k = y.floor() as i64 * icols + x.floor() as i64;
            let at = match rim_at.iter().position(|e| e.0 == k) {
                Some(at) => at,
                None => {
                    rim_at.push((k, [0.0; 4]));
                    rim_at.len() - 1
                }
            };
            let e = &mut rim_at[at].1;
            e[0] += 1.0;
            e[1] += y - y.floor();
            e[2] += p.cos().abs();
            e[3] += if p.sin() * p.cos() < 0.0 { 1.0 } else { 0.0 };
        }
        let id = |k: i64| (k / icols) * 2 + i64::from((k % icols) as f64 >= cx);
        // The fullest slanted cell on each row and side.
        let mut most: Vec<(i64, i64)> = Vec::new();
        for &(k, e) in &rim_at {
            let phi = phi_of(&e);
            let best = most
                .iter()
                .find(|m| m.0 == id(k))
                .and_then(|m| rim_at.iter().find(|r| r.0 == m.1))
                .map_or(0.0, |r| r.1[0]);
            if phi > 50.0 && phi <= 72.0 && e[0] > best {
                match most.iter_mut().find(|m| m.0 == id(k)) {
                    Some(m) => m.1 = k,
                    None => most.push((id(k), k)),
                }
            }
        }
        let edge = rim_at
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
            cx,
            cy,
            s,
            rate: spec.rate,
            dust: spec.dust,
            pitch: spec.pitch,
            label: spec.label,
            cells,
            subs,
            edge,
        }
    }

    /// The record turning, its spindle and its dust.
    fn record(&self, t: f64, pen: &mut Pen<'_>) {
        let a = SPIN * t * self.rate;
        let (ca, sa) = (a.cos(), a.sin());
        for &(k, g) in &self.edge {
            pen.set(k, g, SILVER);
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
            let tone = match b {
                5 => PRINT,
                9 => self.label,
                6.. => SHEEN,
                _ => GROOVE,
            };
            pen.set(cell.k, RAMP[b], tone);
        }
        pen.ink(SILVER);
        pen.put(self.cx, self.cy, 'o');
        pen.ink(DUST_INK);
        // Dust riding round with the record.
        for (r, p) in DUST {
            let r = r * self.s;
            pen.put(
                self.cx + 2.0 * r * (p + self.dust + a).cos(),
                self.cy + r * (p + self.dust + a).sin(),
                '°',
            );
        }
    }

    /// The start and speed buttons, the pitch slider and the tonearm. The
    /// buttons and the slider's foot keep to the record, not to a taller
    /// panel's bottom edge.
    fn furniture(&self, ox: f64, pen: &mut Pen<'_>) {
        let s = self.s;
        let bottom = (self.cy + 11.5 * s).round().min((pen.rows - 1) as f64);
        pen.ink(PLINTH);
        let pc = (self.cx + 2.0 * R * s + 10.5).round();
        let pr = (self.cy - R * s + 1.5).round();
        let ar = (self.cy + 3.5 * s).round();
        let hc = (self.cx + 13.5 * s).round();
        pen.words(ox + 2.0, bottom - 3.0, "┌──┐");
        pen.words(ox + 2.0, bottom - 2.0, "└──┘");
        pen.words(pc - 4.0, bottom - 3.0, "┌┐┌┐");
        pen.words(pc - 4.0, bottom - 2.0, "└┘└┘");
        let top = (self.cy - 0.5).round();
        let cap = ((top + bottom - 2.0) / 2.0).floor() + self.pitch as f64;
        let mut r = top;
        while r <= bottom - 2.0 {
            pen.ink(if r == cap { CAP } else { PLINTH });
            pen.put(pc + 8.0, r, if r == cap { '═' } else { '┊' });
            r += 1.0;
        }

        // The tonearm: counterweight behind the pivot, the pivot in its ring,
        // a tube down and round to the headshell, and the cue lever beside it.
        pen.ink(BRASS);
        pen.words(pc - 2.0, pr - 3.0, "▗▄▄▄▖");
        pen.words(pc - 2.0, pr - 2.0, "▝▀█▀▘");
        pen.words(pc - 3.0, pr - 1.0, "╭──╨──╮");
        pen.words(pc - 3.0, pr, "│  O  │");
        pen.words(pc - 3.0, pr + 1.0, "╰──╥──╯");
        let mut r = pr + 2.0;
        while r < ar {
            pen.put(pc, r, '║');
            r += 1.0;
        }
        let mut c = hc + 5.0;
        while c < pc {
            pen.put(c, ar, '═');
            c += 1.0;
        }
        pen.put(pc, ar, '╝');
        pen.words(hc, ar, "▐███▌");
        pen.words(pc + 3.0, pr + 4.0, "╭╮");
        pen.words(pc + 3.0, pr + 5.0, "││");
        pen.words(pc + 3.0, pr + 6.0, "╰╯");
    }
}

/// Writes clipped to the grid.
struct Pen<'a> {
    out: &'a mut [Cell],
    cols: usize,
    rows: usize,
    /// The twin colours by part; upstream's one ink stays entry 0.
    colour: bool,
    ink: u16,
}

impl Pen<'_> {
    /// Draw in `WIDE_PALETTE` entry `k` from here on, in the twin.
    fn ink(&mut self, k: u16) {
        if self.colour {
            self.ink = k;
        }
    }

    /// Cell `k` is `c`, in entry `tone` in the twin.
    fn set(&mut self, k: usize, c: Cell, tone: u16) {
        self.out[k] = if self.colour { text::tint(c, tone) } else { c };
    }

    fn put(&mut self, c: f64, r: f64, g: char) {
        let (c, r) = (c.floor(), r.floor());
        if c >= 0.0 && c < self.cols as f64 && r >= 0.0 && r < self.rows as f64 {
            self.out[r as usize * self.cols + c as usize] = text::tint(text::cell(g), self.ink);
        }
    }

    fn words(&mut self, c: f64, r: f64, s: &str) {
        for (i, g) in s.chars().enumerate() {
            if g != ' ' {
                self.put(c + i as f64, r, g);
            }
        }
    }

    /// The plinth's edge round the whole grid.
    fn plinth(&mut self) {
        let (w, h) = ((self.cols - 1) as f64, (self.rows - 1) as f64);
        for c in 1..self.cols - 1 {
            self.put(c as f64, 0.0, '─');
            self.put(c as f64, h, '─');
        }
        for r in 1..self.rows - 1 {
            self.put(0.0, r as f64, '│');
            self.put(w, r as f64, '│');
        }
        self.put(0.0, 0.0, '╭');
        self.put(w, 0.0, '╮');
        self.put(0.0, h, '╰');
        self.put(w, h, '╯');
    }
}

/// The mixer between the decks: a box spanning columns `x0..=x1`, two
/// channel strips of EQ knobs and a fader, level meters between them, and
/// the crossfader along the foot.
struct Mixer {
    x0: f64,
    x1: f64,
    y1: f64,
}

impl Mixer {
    fn draw(&self, t: f64, pen: &mut Pen<'_>) {
        let (x0, x1, y0, y1) = (self.x0, self.x1, 1.0, self.y1);
        pen.ink(PANEL);
        pen.words(x0, y0, "┌");
        pen.words(x1, y0, "┐");
        pen.words(x0, y1, "└");
        pen.words(x1, y1, "┘");
        let mut c = x0 + 1.0;
        while c < x1 {
            pen.put(c, y0, '─');
            pen.put(c, y1, '─');
            c += 1.0;
        }
        let mut r = y0 + 1.0;
        while r < y1 {
            pen.put(x0, r, '│');
            pen.put(x1, r, '│');
            r += 1.0;
        }
        let quarter = ((x1 - x0) / 4.0).floor();
        let (a, b, m) = (x0 + quarter, x1 - quarter, ((x0 + x1) / 2.0).floor());
        pen.put(a, y0 + 1.0, 'A');
        pen.put(b, y0 + 1.0, 'B');
        // Hi, mid and low on each channel, each turned now and then.
        pen.ink(KNOB);
        for (ch, x) in [a, b].into_iter().enumerate() {
            for k in 0..3 {
                let ph = (ch * 3 + k) as f64;
                let turn = 1.5 + 1.2 * (t * (0.07 + 0.03 * ph) + ph * 1.7).sin();
                let g = DIAL[(turn.rem_euclid(4.0)) as usize % 4];
                let r = y0 + 3.0 + 2.0 * k as f64;
                pen.put(x - 1.0, r, '(');
                pen.put(x, r, g);
                pen.put(x + 1.0, r, ')');
            }
        }
        // The channel faders: near the top, eased down and back up now and then.
        let (f0, f1) = (y0 + 9.0, y1 - 4.0);
        for (ch, x) in [a, b].into_iter().enumerate() {
            let dip = (t * 0.11 + ch as f64 * 2.4).sin().max(0.0).powi(4);
            let cap = (f0 + (f1 - f0) * (0.12 + 0.6 * dip)).round();
            let mut r = f0;
            pen.ink(PANEL);
            while r <= f1 {
                pen.put(x, r, '┊');
                r += 1.0;
            }
            pen.ink(CAP);
            pen.words(x - 1.0, cap, "═══");
        }
        // The meters, one a channel, kicking on the beat.
        let beat = t * BEAT;
        let (n, f) = (beat.floor(), beat - beat.floor());
        let (m0, m1) = (y0 + 3.0, f1);
        let span = m1 - m0 + 1.0;
        for (ch, x) in [m - 1.0, m + 1.0].into_iter().enumerate() {
            let jitter = hash2(n as i64, ch as i64);
            let shimmer = hash2((t * 24.0) as i64, ch as i64 + 7);
            let level = (0.55 + 0.35 * jitter) * (-3.0 * f).exp() + 0.12 + 0.1 * shimmer;
            let lit = level * span * 2.0;
            let mut r = m1;
            let mut i = 0.0;
            while r >= m0 {
                let g = if lit >= 2.0 * i + 2.0 {
                    '█'
                } else if lit >= 2.0 * i + 1.0 {
                    '▄'
                } else {
                    '·'
                };
                let up = i / span;
                pen.ink(if g == '·' {
                    PANEL
                } else if up < 0.6 {
                    METER[0]
                } else if up < 0.85 {
                    METER[1]
                } else {
                    METER[2]
                });
                pen.put(x, r, g);
                r -= 1.0;
                i += 1.0;
            }
        }
        // The crossfader, swept from deck to deck.
        let cr = y1 - 2.0;
        pen.ink(PANEL);
        let mut c = a;
        while c <= b {
            pen.put(c, cr, '─');
            c += 1.0;
        }
        let at = (a + (b - a) * (0.5 + 0.45 * (t * TAU_XF).sin())).round();
        pen.ink(PLINTH);
        pen.words(at - 1.0, cr, "▐█▌");
    }
}

/// Radians a second of the crossfader's sweep: once across and back in 24 s.
const TAU_XF: f64 = 2.0 * PI / 24.0;

struct Scene {
    cols: usize,
    rows: usize,
    decks: Vec<(f64, Deck)>,
    mixer: Option<Mixer>,
    colour: bool,
}

impl Scene {
    fn frame(&self, t: f64, out: &mut [Cell]) {
        out.fill(RAMP[0]);
        let mut pen = Pen {
            out,
            cols: self.cols,
            rows: self.rows,
            colour: self.colour,
            ink: 0,
        };
        for (_, deck) in &self.decks {
            deck.record(t, &mut pen);
        }

        // The plinth, its start and speed buttons, and the pitch slider.
        pen.ink(PLINTH);
        pen.plinth();
        for (ox, deck) in &self.decks {
            deck.furniture(*ox, &mut pen);
        }
        if let Some(m) = &self.mixer {
            m.draw(t, &mut pen);
        }
    }
}

pub struct Vinyl(Scene);

impl Piece for Vinyl {
    const NAME: &'static str = "vinyl";
    const COLS: usize = COLS;
    const ROWS: usize = ROWS;
    const FPS: u32 = 24;
    const CELL: usize = 2;
    const PALETTE: &'static [u32] = &[hex("#ffb347")];
    const GROUND: u32 = 0;

    fn new() -> Self {
        Self(Scene {
            cols: COLS,
            rows: ROWS,
            decks: vec![(0.0, Deck::new(COLS, ROWS, &ORIGINAL))],
            mixer: None,
            colour: false,
        })
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        self.0.frame(t, out);
    }
}

pub struct VinylWide(Scene);

impl Canvas for VinylWide {
    const NAME: &'static str = "vinyl-wide";
    const FPS: u32 = Vinyl::FPS;
    const PALETTE: &'static [u32] = WIDE_PALETTE;

    /// Two decks as tall as the panel allows, so long as two of them and a
    /// mixer at least `MIX` wide still fit across it.
    fn new(cols: usize, rows: usize) -> Self {
        const MIX: f64 = 22.0;
        let (w, h) = (cols as f64, rows as f64);
        let s = (h / ROWS as f64)
            .min((w - MIX - 2.0 * (ARM + 1.0)) / (2.0 * (CX + 2.0 * R)))
            .max(0.25);
        let dw = (CX + 2.0 * R) * s + ARM + 1.0;
        let right = (w - dw).floor();
        let deck = |ox: f64, rate: f64, dust: f64, pitch: i64, label: u16| {
            let spec = Spec {
                ox,
                s,
                rate,
                dust,
                pitch,
                label,
            };
            (ox, Deck::new(cols, rows, &spec))
        };
        let (x0, x1) = (dw.floor(), right - 1.0);
        Self(Scene {
            cols,
            rows,
            decks: vec![
                deck(0.0, 1.0, 0.0, 0, RED_LABEL),
                deck(right, 1.02, PI, 1, BLUE_LABEL),
            ],
            colour: true,
            mixer: (x1 - x0 >= 10.0 && rows >= 16).then_some(Mixer {
                x0,
                x1,
                y1: h - 2.0,
            }),
        })
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        self.0.frame(t, out);
    }
}
