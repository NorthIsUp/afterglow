//! Drawing a fight: the board resampled through the zoom, the fighters posed
//! from the scene, and the effects over them.

use super::art::{
    self, ink, shadow, ACCENT, ADVANCE, ASH, BG, CLEAR, FIRE, FIRE_HOT, MAGIC, MAGIC_HOT, SPARK,
    SPRITE, STEEL, TEXT_H, WOOD,
};
use super::fight::{scene, smooth, weapon_line, Actor, Beat, Fight, Fx, Look};
use super::fighters::{leg_px, BODY, TORSO};
use super::paint::Painter;
use super::rules::{file, kind, rank, side, BISHOP, KING, KNIGHT, PAWN, QUEEN};

/// The zoom a fight settles at. The fighters have twice the board sprites'
/// resolution, so each of their pixels is half a zoomed board pixel.
const ZOOM: f32 = 3.0;

/// Cells `(x0, y0, x1, y1)`, the far edges exclusive.
pub type Span = (i32, i32, i32, i32);
const EMPTY: Span = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);

fn hash(a: u32, b: u32, c: u32) -> u32 {
    let mut h =
        a.wrapping_mul(0x9E37_79B1) ^ b.wrapping_mul(0x85EB_CA77) ^ c.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^ (h >> 12)
}

/// Board-local world pixels at the view's top-left, and its zoom.
struct View {
    z: f32,
    ox: f32,
    oy: f32,
}

/// Where scene coordinates land on screen: the victim's feet, the side the
/// attacker comes from, and cells per body pixel.
#[derive(Clone, Copy)]
struct Stage {
    x: f32,
    y: f32,
    dir: f32,
    k: f32,
}

impl Stage {
    fn at(&self, p: (f32, f32)) -> (f32, f32) {
        (self.x + self.dir * p.0 * self.k, self.y + p.1 * self.k)
    }
}

impl Painter<'_> {
    /// One frame of `f` over the board. `drawn` carries the overlay's bounds
    /// to the next frame, which restores only those; `None` repaints the
    /// whole board.
    pub fn fight(&mut self, f: &Fight, drawn: &mut Option<Span>) {
        let (beat, p) = f.beat();
        let zooming = matches!(beat, Beat::ZoomIn | Beat::ZoomOut);
        let e = match beat {
            Beat::ZoomIn => smooth(p),
            Beat::ZoomOut => 1.0 - smooth(p),
            _ => 1.0,
        };
        let b = self.l.sq * 8;
        let (fx, fy) = self.focus(f);
        let view = View {
            z: 1.0 + e * (ZOOM - 1.0),
            ox: fx as f32 * e,
            oy: fy as f32 * e,
        };
        let (board, hide) = match beat {
            Beat::ZoomIn => (&f.before, [64, 64]),
            Beat::ZoomOut => (&self.g.pos.sq, [64, 64]),
            _ => (&f.before, [f.from, f.at]),
        };
        let region = match (zooming, *drawn) {
            (false, Some(r)) => r,
            _ => (0, 0, b, b),
        };
        self.resample(&view, board, hide, region);
        if zooming {
            *drawn = None;
            return;
        }
        let (bx, by) = (self.l.bx, self.l.by);
        self.c.clip_to(bx, by, b, b);
        let k = self.cell() as f32;
        let sq = self.l.sq as f32;
        let (vcx, vfy) = self.feet(f.at);
        let stage = Stage {
            x: bx as f32 + (vcx - view.ox) * view.z,
            y: by as f32 + (vfy - view.oy) * view.z,
            dir: f.dir as f32,
            k,
        };
        let sc = scene(f, sq * view.z / k);
        let mut span = EMPTY;
        for a in [&sc.victim, &sc.attacker] {
            if a.show {
                self.actor(a, stage, sc.tick, &mut span);
            }
        }
        for fx in &sc.fx {
            self.effect(fx, stage, sc.tick, &mut span);
        }
        self.c.unclip();
        let clamp = |s: Span| (s.0.max(0), s.1.max(0), s.2.min(b), s.3.min(b));
        *drawn = Some(if span.0 > span.2 {
            (0, 0, 0, 0)
        } else {
            clamp((span.0 - bx, span.1 - by, span.2 - bx, span.3 - by))
        });
    }

    /// Cells per fighter pixel.
    fn cell(&self) -> i32 {
        ((self.l.k as f32 * ZOOM / 2.0).round() as i32).max(1)
    }

    /// The settled view's top-left in board pixels: the two fighters in the
    /// middle, the view kept on the board.
    fn focus(&self, f: &Fight) -> (i32, i32) {
        let sq = self.l.sq as f32;
        let b = sq * 8.0;
        let (cx, cy) = self.centre(f.at);
        let cx = cx - f.dir as f32 * 0.4 * sq;
        let cy = cy - 0.2 * sq;
        let half = b / ZOOM / 2.0;
        let max = b - b / ZOOM;
        (
            (cx - half).clamp(0.0, max).round() as i32,
            (cy - half).clamp(0.0, max).round() as i32,
        )
    }

    fn centre(&self, s: u8) -> (f32, f32) {
        let sq = self.l.sq as f32;
        (
            (f32::from(file(s)) + 0.5) * sq,
            (7.0 - f32::from(rank(s)) + 0.5) * sq,
        )
    }

    /// Board pixels under the middle of a piece's base on `s`.
    fn feet(&self, s: u8) -> (f32, f32) {
        let (sq, k) = (self.l.sq, self.l.k);
        let top = (7 - i32::from(rank(s))) * sq + (sq - SPRITE as i32 * k + 1) / 2;
        (self.centre(s).0, (top + 15 * k) as f32)
    }

    /// The board through `view`, over `r` in board-local cells, with the
    /// pieces on `hide` left off.
    fn resample(&mut self, view: &View, board: &[u8; 64], hide: [u8; 2], r: Span) {
        let (bx, by, sq, k) = (self.l.bx, self.l.by, self.l.sq, self.l.k);
        let b = sq * 8;
        let p = SPRITE as i32 * k;
        let (inset_x, inset_y) = ((sq - p) / 2, (sq - p + 1) / 2);
        for sy in r.1..r.3 {
            let wy = ((view.oy + sy as f32 / view.z) as i32).clamp(0, b - 1);
            let row = wy / sq;
            let ly = wy - row * sq - inset_y;
            for sx in r.0..r.2 {
                let wx = ((view.ox + sx as f32 / view.z) as i32).clamp(0, b - 1);
                let col = wx / sq;
                let s = ((7 - row) * 8 + col) as u8;
                let base = self.colours[s as usize];
                let pc = board[s as usize];
                let lx = wx - col * sq - inset_x;
                let mut c = base;
                if pc != 0 && !hide.contains(&s) {
                    let spr = &self.art.big[kind(pc) as usize - 1];
                    let at = |x: i32, y: i32| -> u8 {
                        if x < 0 || y < 0 || x >= p || y >= p {
                            CLEAR
                        } else {
                            spr[(y / k) as usize][(x / k) as usize]
                        }
                    };
                    let px = at(lx, ly);
                    if px != CLEAR {
                        c = ink(side(pc), px);
                    } else if at(lx - k, ly - k) != CLEAR {
                        c = shadow(base);
                    }
                }
                self.c.put(bx + sx, by + sy, c);
            }
        }
    }

    fn plot(&mut self, x: f32, y: f32, w: i32, c: u16, span: &mut Span) {
        let (x, y) = (x.round() as i32, y.round() as i32);
        self.c.rect(x, y, w, w, c);
        *span = (
            span.0.min(x),
            span.1.min(y),
            span.2.max(x + w),
            span.3.max(y + w),
        );
    }

    fn actor(&mut self, a: &Actor, st: Stage, tick: u32, span: &mut Span) {
        let body = &self.art.bodies[a.kind as usize - 1];
        let lk = a.look;
        let face = a.face * st.dir;
        let (ax, ay) = st.at((a.x, a.y));
        let k = st.k;
        let wide = (k * (1.0 + 0.4 * lk.flat)).ceil() as i32;
        let place = |u: f32, v: f32| -> (f32, f32) {
            let (mut u, mut v) = (u * (1.0 + 0.4 * lk.flat), v * (1.0 - 0.85 * lk.flat));
            u += lk.lean * v / BODY as f32;
            let mut w = v - 16.0;
            for _ in 0..lk.rot % 4 {
                (u, w) = (-w, u);
            }
            v = w + if lk.rot % 2 == 1 { 10.0 } else { 16.0 };
            (ax + face * u * k - k / 2.0, ay - v * k - k / 2.0)
        };
        let step = if lk.rot == 0 { a.step } else { 0 };
        let burn_line = BODY as f32 * (1.0 - lk.burn);
        for (py, row) in body.iter().enumerate() {
            for (px, &drawn) in row.iter().enumerate() {
                let cls = if py < TORSO {
                    drawn
                } else {
                    leg_px(step, px as i32, py as i32)
                };
                if cls == CLEAR {
                    continue;
                }
                let h = hash(px as u32, py as u32, a.kind.into());
                if (h % 100) as f32 >= 100.0 * (1.0 - lk.fade) {
                    continue;
                }
                let mut c = ink(a.side, cls);
                if lk.tint != 0 && cls != 1 {
                    c = lk.tint;
                }
                if lk.burn > 0.0 {
                    let y = py as f32;
                    if y >= burn_line {
                        c = ASH;
                    } else if y >= burn_line - 4.0 && lk.burn < 1.0 {
                        c = if hash(px as u32, py as u32, tick / 2).is_multiple_of(3) {
                            FIRE_HOT
                        } else {
                            FIRE
                        };
                    }
                }
                let (mut u, mut v) = (px as f32 - 15.5, 31.5 - py as f32);
                if lk.shards > 0.0 {
                    (u, v) = shard(u, v, px, py, lk);
                }
                let (x, y) = place(u, v);
                self.plot(x, y, if lk.flat > 0.0 { wide } else { k as i32 }, c, span);
            }
        }
        let intact = lk.rot == 0 && lk.shards == 0.0 && lk.flat < 0.5 && lk.burn == 0.0;
        if intact && lk.fade < 0.5 {
            self.weapon(a, &place, span);
        }
    }

    /// The arm, and what it holds, in the actor's own frame.
    fn weapon(&mut self, a: &Actor, place: &dyn Fn(f32, f32) -> (f32, f32), span: &mut Span) {
        let k = self.cell();
        let w = weapon_line(a.kind, a.arm, a.reach);
        let line = art::INK[a.side][0];
        let fill = art::INK[a.side][1];
        let mut stroke = |s: &mut Self, p0: (f32, f32), p1: (f32, f32), c: u16, thick: i32| {
            let n = ((p1.0 - p0.0).hypot(p1.1 - p0.1) * 2.0).ceil().max(1.0) as i32;
            for i in 0..=n {
                let t = i as f32 / n as f32;
                let (u, v) = (p0.0 + (p1.0 - p0.0) * t, p0.1 + (p1.1 - p0.1) * t);
                let (x, y) = place(u, v);
                s.plot(x, y, k * thick, c, span);
            }
        };
        if a.kind != KNIGHT {
            stroke(self, w.shoulder, w.hand, line, 2);
        }
        match a.kind {
            PAWN => {
                stroke(self, w.butt, w.tip, WOOD, 1);
                let back = w.towards(3.0);
                stroke(self, back, w.tip, STEEL, 2);
            }
            KNIGHT => {
                stroke(self, w.butt, w.tip, WOOD, 2);
                let back = w.towards(4.0);
                stroke(self, back, w.tip, STEEL, 2);
            }
            BISHOP => {
                stroke(self, w.butt, w.tip, WOOD, 1);
                self.blob(place, w.tip, 2.0, MAGIC, MAGIC_HOT, span);
            }
            QUEEN => {
                stroke(self, w.hand, w.tip, STEEL, 1);
                self.blob(place, w.tip, 1.6, MAGIC, MAGIC_HOT, span);
            }
            KING => {
                stroke(self, w.butt, w.tip, ACCENT, 1);
                self.blob(place, w.tip, 2.2, ACCENT, SPARK, span);
            }
            // The rook fights with its fists.
            _ => {}
        }
        if a.kind != KNIGHT {
            let (x, y) = place(w.hand.0 - 1.0, w.hand.1 + 1.0);
            self.plot(x, y, 3 * k, line, span);
            let (x, y) = place(w.hand.0, w.hand.1);
            self.plot(x, y, k, fill, span);
        }
    }

    /// A round knob of radius `r` body pixels with a bright middle.
    fn blob(
        &mut self,
        place: &dyn Fn(f32, f32) -> (f32, f32),
        at: (f32, f32),
        r: f32,
        rim: u16,
        core: u16,
        span: &mut Span,
    ) {
        let k = self.cell();
        let n = r.ceil() as i32;
        for dy in -n..=n {
            for dx in -n..=n {
                let d = ((dx * dx + dy * dy) as f32).sqrt();
                if d <= r {
                    let c = if d <= r * 0.45 { core } else { rim };
                    let (x, y) = place(at.0 + dx as f32, at.1 + dy as f32);
                    self.plot(x, y, k, c, span);
                }
            }
        }
    }

    fn effect(&mut self, fx: &Fx, st: Stage, tick: u32, span: &mut Span) {
        let k = st.k;
        let ki = k as i32;
        let dot = |s: &mut Self, p: (f32, f32), w: i32, c: u16, span: &mut Span| {
            let (x, y) = st.at(p);
            s.plot(x - k / 2.0, y - k / 2.0, ki * w, c, span);
        };
        match *fx {
            Fx::None => {}
            Fx::Bolt { a, b } => {
                const SEGS: u32 = 7;
                let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                let len = dx.hypot(dy).max(1.0);
                let (nx, ny) = (-dy / len, dx / len);
                let mut prev = a;
                for i in 1..=SEGS {
                    let t = i as f32 / SEGS as f32;
                    let j = if i == SEGS {
                        0.0
                    } else {
                        (hash(i, tick / 2, 7) % 9) as f32 - 4.0
                    };
                    let next = (a.0 + dx * t + nx * j, a.1 + dy * t + ny * j);
                    let n = ((next.0 - prev.0).hypot(next.1 - prev.1) * 2.0).ceil() as i32;
                    for s in 0..=n.max(1) {
                        let u = s as f32 / n.max(1) as f32;
                        let p = (
                            prev.0 + (next.0 - prev.0) * u,
                            prev.1 + (next.1 - prev.1) * u,
                        );
                        dot(self, (p.0 - 0.5, p.1 - 0.5), 2, MAGIC, span);
                        dot(
                            self,
                            p,
                            1,
                            if tick % 4 < 2 { SPARK } else { MAGIC_HOT },
                            span,
                        );
                    }
                    prev = next;
                }
            }
            Fx::Ball { at, r, core, rim } => {
                let n = r.ceil() as i32;
                for dy in -n..=n {
                    for dx in -n..=n {
                        let d = ((dx * dx + dy * dy) as f32).sqrt();
                        if d <= r {
                            let c = if d <= r * 0.55 { core } else { rim };
                            dot(self, (at.0 + dx as f32, at.1 + dy as f32), 1, c, span);
                        }
                    }
                }
            }
            Fx::Ring { at, r } => {
                let n = (r * 3.0) as u32 + 8;
                for i in 0..n {
                    let a = i as f32 / n as f32 * std::f32::consts::TAU;
                    let p = (at.0 + a.cos() * r, at.1 + a.sin() * r * 0.3);
                    dot(self, p, 1, if i % 3 == 0 { SPARK } else { ASH }, span);
                }
            }
            Fx::Burst { at, p, colour } => {
                if p >= 1.0 {
                    return;
                }
                for i in 0..10u32 {
                    let h = hash(i, 3, colour.into());
                    let a = i as f32 * 0.628 + (h % 50) as f32 / 100.0;
                    let d = p * (10.0 + (h % 9) as f32);
                    let w = if p < 0.5 { 2 } else { 1 };
                    dot(
                        self,
                        (at.0 + a.cos() * d, at.1 - a.sin() * d),
                        w,
                        colour,
                        span,
                    );
                }
            }
            Fx::Stars { at, p } => {
                for i in 0..3 {
                    let a = p * 12.0 + i as f32 * std::f32::consts::TAU / 3.0;
                    let c = (at.0 + a.cos() * 9.0, at.1 + a.sin() * 3.0);
                    for (dx, dy) in [(0.0, 0.0), (1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                        dot(self, (c.0 + dx, c.1 + dy), 1, ACCENT, span);
                    }
                }
            }
            Fx::Word { at, text, colour } => {
                let (x, mut y) = st.at(at);
                let h = TEXT_H as i32 * ki;
                // Over a piece on the back rank it would leave the view: under
                // the feet instead.
                if (y as i32) - h / 2 < self.l.by + ki {
                    y = st.at((at.0, at.1 + 52.0)).1;
                }
                let w = (text.len() * ADVANCE) as i32 * ki;
                let (b, bx, by) = (self.l.sq * 8, self.l.bx, self.l.by);
                let x0 = (x as i32 - w / 2).clamp(bx + ki, (bx + b - w - 2 * ki).max(bx + ki));
                let y0 = (y as i32 - h / 2).clamp(by + ki, (by + b - h - 2 * ki).max(by + ki));
                for (dx, c) in [(ki, BG), (0, colour)] {
                    art::each_text_px(text, |gx, gy| {
                        let (px, py) = (x0 + dx + gx * ki, y0 + dx + gy * ki);
                        self.plot(px as f32, py as f32, ki, c, span);
                    });
                }
            }
        }
    }
}

/// Where a body pixel has got to once the body breaks apart, in 4x4 chunks.
fn shard(u: f32, v: f32, px: usize, py: usize, lk: Look) -> (f32, f32) {
    let q = lk.shards;
    let h = hash((px / 4) as u32, (py / 4) as u32, 11);
    if lk.crumble {
        let pile = (h % 5) as f32 + (h >> 8) as f32 % 3.0;
        let jx = ((h >> 4) % 9) as f32 - 4.0;
        return (u + jx * q, v + (pile - v) * q);
    }
    let vx = ((h % 61) as f32 - 30.0) * 0.8;
    let vy = 15.0 + ((h >> 6) % 30) as f32;
    (u + vx * q, (v + vy * q - 70.0 * q * q).max(0.5))
}
