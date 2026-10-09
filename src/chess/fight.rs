//! A capture played out as a fight: who fights, where, which choreography,
//! and the frame clock. `scene` turns a frame into poses and effects;
//! `fight_paint` draws them.
//!
//! Scene coordinates are body pixels (the fighters' design pixels), origin
//! the victim's feet, x toward the victim from the attacker's side, so every
//! choreography is written once whichever way the attacker comes from.

use super::art::{ACCENT, ASH, FIRE, FIRE_HOT, MAGIC, MAGIC_HOT, SPARK};
use super::rules::{file, kind, rank, side, Move, BISHOP, EN_PASSANT, KNIGHT, PAWN, QUEEN, ROOK};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    /// Pawn: two spear jabs, the victim topples.
    Jab,
    /// Knight: backs off, charges with a lance, the victim is sent flying.
    Charge,
    /// Bishop: a fireball from the staff, the victim burns to ash.
    Cast,
    /// Rook: leaps and lands on the victim, flattening it.
    Slam,
    /// Queen: lightning from the wand, the victim dissolves into sparks.
    Zap,
    /// King: sceptre blows, the victim shatters.
    Swing,
    /// Queen takes queen: two bolts meet, the attacker's wins.
    Duel,
    /// Knight takes rook: the tower crumbles to a pile of bricks.
    Crumble,
    /// Pawn takes queen: a poke, a startled hop, a faint.
    Comedy,
}

pub fn style(attacker: u8, victim: u8) -> Style {
    match (kind(attacker), kind(victim)) {
        (QUEEN, QUEEN) => Style::Duel,
        (KNIGHT, ROOK) => Style::Crumble,
        (PAWN, QUEEN) => Style::Comedy,
        (PAWN, _) => Style::Jab,
        (KNIGHT, _) => Style::Charge,
        (BISHOP, _) => Style::Cast,
        (ROOK, _) => Style::Slam,
        (QUEEN, _) => Style::Zap,
        _ => Style::Swing,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Flourish {
    None,
    Check,
    Mate,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Beat {
    ZoomIn,
    Approach,
    Attack,
    Defeat,
    Step,
    ZoomOut,
}

/// Where each beat ends, as a share of the fight.
const BEATS: [(Beat, f32); 6] = [
    (Beat::ZoomIn, 0.10),
    (Beat::Approach, 0.26),
    (Beat::Attack, 0.54),
    (Beat::Defeat, 0.74),
    (Beat::Step, 0.90),
    (Beat::ZoomOut, 1.0),
];

#[derive(Clone, Copy)]
pub struct Fight {
    /// The board before the capture, for the zoom in and the fight itself.
    pub before: [u8; 64],
    pub attacker: u8,
    pub victim: u8,
    pub from: u8,
    pub to: u8,
    /// Where the victim stands: `to`, except en passant.
    pub at: u8,
    /// The kind promoted to, or 0.
    pub promo: u8,
    /// +1 when the attacker comes from the left of the victim.
    pub dir: i32,
    pub style: Style,
    pub flourish: Flourish,
    pub t: u32,
    pub len: u32,
}

impl Fight {
    pub fn new(board: &[u8; 64], m: Move, flourish: Flourish, len: u32) -> Self {
        let attacker = board[m.from as usize];
        let at = if m.flag == EN_PASSANT {
            (m.from & !7) | (m.to & 7)
        } else {
            m.to
        };
        let victim = board[at as usize];
        let (fa, ff) = (file(at), file(m.from));
        let mut dir = match ff.cmp(&fa) {
            std::cmp::Ordering::Less => 1,
            std::cmp::Ordering::Greater => -1,
            std::cmp::Ordering::Equal => 1 - 2 * side(attacker) as i32,
        };
        // The attacker stands most of a square beside the victim: keep it
        // on the board.
        if fa == 0 {
            dir = -1;
        } else if fa == 7 {
            dir = 1;
        }
        Self {
            before: *board,
            attacker,
            victim,
            from: m.from,
            to: m.to,
            at,
            promo: m.promo,
            dir,
            style: style(attacker, victim),
            flourish,
            t: 0,
            len: len.max(12),
        }
    }

    pub fn done(&self) -> bool {
        self.t >= self.len
    }

    /// The beat this frame is in and how far through it, 0..1.
    pub fn beat(&self) -> (Beat, f32) {
        let x = self.t as f32 / self.len as f32;
        let mut start = 0.0;
        for (b, end) in BEATS {
            if x < end {
                return (b, ((x - start) / (end - start)).clamp(0.0, 1.0));
            }
            start = end;
        }
        (Beat::ZoomOut, 1.0)
    }

    /// Squares from the victim to `s`, in scene axes.
    fn rel(&self, s: u8) -> (f32, f32) {
        let dx = f32::from(file(s) - file(self.at)) * self.dir as f32;
        let dy = f32::from(rank(self.at) - rank(s));
        (dx, dy)
    }
}

/// How a body is drawn this frame, beyond where it stands.
#[derive(Clone, Copy, Default)]
pub struct Look {
    /// Body pixels the head leans forward, feet fixed.
    pub lean: f32,
    /// Quarter turns backward about the body's middle.
    pub rot: u8,
    /// 0 upright .. 1 pancake.
    pub flat: f32,
    /// A palette index the fill is drawn in, or 0.
    pub tint: u16,
    /// 0 solid .. 1 gone, dithered.
    pub fade: f32,
    /// Share of the body, from the feet up, burnt to ash.
    pub burn: f32,
    /// 0 whole, then how far its shards have flown, 0..1.
    pub shards: f32,
    /// The shards drop where they are rather than burst.
    pub crumble: bool,
}

#[derive(Clone, Copy, Default)]
pub struct Actor {
    pub kind: u8,
    pub side: usize,
    pub show: bool,
    pub x: f32,
    pub y: f32,
    /// +1 faces the victim's side, -1 the attacker's.
    pub face: f32,
    /// Walk frame, 0 standing.
    pub step: u8,
    /// Radians above forward.
    pub arm: f32,
    /// Body pixels the weapon is thrust out.
    pub reach: f32,
    pub look: Look,
}

#[derive(Clone, Copy, Default)]
pub enum Fx {
    #[default]
    None,
    Bolt {
        a: (f32, f32),
        b: (f32, f32),
    },
    Ball {
        at: (f32, f32),
        r: f32,
        core: u16,
        rim: u16,
    },
    /// A shock ring on the floor.
    Ring {
        at: (f32, f32),
        r: f32,
    },
    Burst {
        at: (f32, f32),
        p: f32,
        colour: u16,
    },
    /// Dizzy stars circling a fallen head.
    Stars {
        at: (f32, f32),
        p: f32,
    },
    Word {
        at: (f32, f32),
        text: &'static [u8],
        colour: u16,
    },
}

pub const FX: usize = 4;

#[derive(Clone, Copy, Default)]
pub struct Scene {
    /// Drawn first.
    pub victim: Actor,
    pub attacker: Actor,
    pub fx: [Fx; FX],
    pub tick: u32,
}

impl Scene {
    fn push(&mut self, f: Fx) {
        if let Some(slot) = self.fx.iter_mut().find(|s| matches!(s, Fx::None)) {
            *slot = f;
        }
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// 0..1..0 over `t` in 0..1, `n` times.
fn pulse(t: f32, n: f32) -> f32 {
    let x = (t * n).fract();
    1.0 - (2.0 * x - 1.0).abs()
}

/// Share of the way through the window `a..b` of `t`.
fn win(t: f32, a: f32, b: f32) -> f32 {
    ((t - a) / (b - a)).clamp(0.0, 1.0)
}

/// Where the attacker squares up, in squares before the victim.
const CONTACT: f32 = 0.8;

/// An arm and what it holds, in body pixels from the feet, v up, facing
/// right.
pub struct Grip {
    pub shoulder: (f32, f32),
    pub hand: (f32, f32),
    pub butt: (f32, f32),
    pub tip: (f32, f32),
}

impl Grip {
    /// The point `d` pixels back from the tip toward the butt.
    pub fn towards(&self, d: f32) -> (f32, f32) {
        let (dx, dy) = (self.butt.0 - self.tip.0, self.butt.1 - self.tip.1);
        let n = dx.hypot(dy).max(1.0);
        (self.tip.0 + dx / n * d, self.tip.1 + dy / n * d)
    }
}

pub fn weapon_line(kind: u8, arm: f32, reach: f32) -> Grip {
    let (sx, sy, _, _) = super::fighters::JOINTS[kind as usize - 1];
    let shoulder = (sx as f32 - 15.5, 31.5 - sy as f32);
    let d = (arm.cos(), arm.sin());
    let along = |p: (f32, f32), n: f32| (p.0 + d.0 * n, p.1 + d.1 * n);
    let hand = along(shoulder, if kind == KNIGHT { 2.0 } else { 7.0 });
    let (butt, tip) = match kind {
        PAWN => (along(hand, -5.0), along(hand, 12.0 + reach)),
        KNIGHT => (along(hand, -6.0), along(hand, 16.0 + reach)),
        BISHOP => {
            // The staff stands up from the fist, whichever way the arm points.
            let s = ((arm + 1.2).cos(), (arm + 1.2).sin());
            (
                (hand.0 - s.0 * 9.0, hand.1 - s.1 * 9.0),
                (hand.0 + s.0 * 11.0, hand.1 + s.1 * 11.0),
            )
        }
        ROOK => (hand, hand),
        QUEEN => (hand, along(hand, 6.0 + reach)),
        _ => (along(hand, -2.0), along(hand, 10.0 + reach)),
    };
    Grip {
        shoulder,
        hand,
        butt,
        tip,
    }
}

/// Where an actor's weapon ends, in scene coordinates.
pub fn weapon_tip(a: &Actor) -> (f32, f32) {
    let g = weapon_line(a.kind, a.arm, a.reach);
    (a.x + g.tip.0 * a.face, a.y - g.tip.1)
}

/// The poses and effects of frame `f.t`. `per_sq` is body pixels per
/// board square.
pub fn scene(f: &Fight, per_sq: f32) -> Scene {
    let (beat, p) = f.beat();
    let mut sc = Scene {
        tick: f.t,
        ..Scene::default()
    };
    let walk = ((f.t / 3) % 4) as u8;
    let contact = -CONTACT * per_sq;
    let (fx, fy) = f.rel(f.from);
    let (fx, fy) = (fx * per_sq, fy * per_sq);
    let mut a = Actor {
        kind: kind(f.attacker),
        side: side(f.attacker),
        show: true,
        x: contact,
        face: 1.0,
        arm: -0.5,
        ..Actor::default()
    };
    let mut v = Actor {
        kind: kind(f.victim),
        side: side(f.victim),
        show: true,
        face: -1.0,
        arm: -0.9,
        ..Actor::default()
    };
    match beat {
        Beat::ZoomIn | Beat::ZoomOut => {
            a.show = false;
            v.show = false;
        }
        Beat::Approach => {
            let e = smooth(p);
            a.x = lerp(fx, contact, e);
            a.y = lerp(fy, 0.0, e);
            a.step = if p < 0.95 { walk } else { 0 };
            if f.style == Style::Comedy {
                // On tiptoe.
                a.step = ((f.t / 5) % 4) as u8;
                a.y -= pulse(p, 6.0) * 2.0;
            }
            v.y = -pulse(p, 2.0) * 1.0;
        }
        Beat::Attack => attack(f.style, p, f.t, per_sq, &mut a, &mut v, &mut sc),
        Beat::Defeat => defeat(f.style, p, per_sq, &mut a, &mut v, &mut sc),
        Beat::Step => {
            v.show = false;
            let from_x = end_of_defeat(f.style, per_sq);
            let (tx, ty) = f.rel(f.to);
            let e = smooth(win(p, 0.0, 0.6));
            a.x = lerp(from_x, tx * per_sq, e);
            a.y = lerp(0.0, ty * per_sq, e) - pulse(e, 1.0) * 3.0;
            a.step = if e < 1.0 { walk } else { 0 };
            a.arm = lerp(-0.5, 1.3, win(p, 0.5, 0.8));
            if f.promo != 0 && p > 0.6 {
                a.kind = f.promo;
                sc.push(Fx::Burst {
                    at: (a.x, a.y - 16.0),
                    p: win(p, 0.6, 1.0),
                    colour: SPARK,
                });
            }
            let word: &'static [u8] = match f.flourish {
                Flourish::None => b"",
                Flourish::Check => b"CHECK",
                Flourish::Mate => b"MATE",
            };
            if !word.is_empty() && p > 0.15 {
                sc.push(Fx::Burst {
                    at: (a.x, a.y - 20.0),
                    p: win(p, 0.15, 1.0),
                    colour: ACCENT,
                });
                sc.push(Fx::Word {
                    at: (a.x, a.y - 40.0),
                    text: word,
                    colour: ACCENT,
                });
            }
        }
    }
    sc.attacker = a;
    sc.victim = v;
    sc
}

/// Where the attacker stands when the victim is gone.
fn end_of_defeat(style: Style, per_sq: f32) -> f32 {
    match style {
        Style::Slam => 0.0,
        Style::Charge | Style::Crumble => -0.45 * per_sq,
        _ => -CONTACT * per_sq,
    }
}

fn hit(v: &mut Actor, on: bool) {
    if on {
        v.look.tint = SPARK;
        v.look.lean = -3.0;
    }
}

fn attack(style: Style, p: f32, t: u32, per_sq: f32, a: &mut Actor, v: &mut Actor, sc: &mut Scene) {
    let contact = a.x;
    match style {
        Style::Jab => {
            let thrust = pulse(win(p, 0.1, 0.9), 2.0);
            a.arm = 0.05;
            a.reach = thrust * 9.0;
            a.look.lean = thrust * 3.0;
            hit(v, thrust > 0.85);
            if thrust > 0.85 {
                sc.push(Fx::Burst {
                    at: weapon_tip(a),
                    p: thrust,
                    colour: SPARK,
                });
            }
        }
        Style::Charge | Style::Crumble => {
            let back = smooth(win(p, 0.0, 0.35));
            let run = win(p, 0.45, 0.8);
            a.x = contact - back * 0.4 * per_sq + run * run * (0.4 + 0.35) * per_sq;
            a.step = ((t / 2) % 4) as u8;
            a.arm = 0.0;
            a.reach = 4.0 * run;
            a.look.lean = if run > 0.0 { 3.0 } else { -2.0 * back };
            a.y = -pulse(p, 5.0) * 2.0;
            let struck = p > 0.8;
            hit(v, struck);
            if struck {
                sc.push(Fx::Burst {
                    at: (v.x - 8.0, v.y - 14.0),
                    p: win(p, 0.8, 1.0),
                    colour: SPARK,
                });
            }
        }
        Style::Cast => {
            a.arm = lerp(-0.5, 0.9, smooth(win(p, 0.0, 0.3)));
            let (tx, ty) = weapon_tip(a);
            let glow = win(p, 0.15, 0.4);
            if p < 0.45 {
                sc.push(Fx::Ball {
                    at: (tx, ty - 3.0),
                    r: 1.5 + glow * 3.0,
                    core: FIRE_HOT,
                    rim: FIRE,
                });
            } else {
                let q = win(p, 0.45, 1.0);
                let (bx, by) = (v.x - 4.0, v.y - 14.0);
                let x = lerp(tx, bx, q);
                let y = lerp(ty - 3.0, by, q) - (q * std::f32::consts::PI).sin() * 10.0;
                sc.push(Fx::Ball {
                    at: (x, y),
                    r: 4.0,
                    core: FIRE_HOT,
                    rim: FIRE,
                });
                sc.push(Fx::Burst {
                    at: (x - 4.0, y),
                    p: (q * 4.0).fract(),
                    colour: FIRE,
                });
                hit(v, q > 0.9);
            }
        }
        Style::Slam => {
            let crouch = smooth(win(p, 0.0, 0.3));
            let jump = win(p, 0.3, 1.0);
            a.look.flat = 0.25 * crouch * (1.0 - jump);
            if jump > 0.0 {
                a.x = lerp(contact, 0.0, smooth(jump));
                a.y = -(jump * std::f32::consts::PI).sin() * per_sq * 1.1;
            }
            a.arm = 1.4 * jump;
            v.look.lean = -2.0 * jump;
            v.y = 0.0;
        }
        Style::Zap => {
            a.arm = lerp(-0.5, 0.6, smooth(win(p, 0.0, 0.25)));
            if p > 0.3 {
                sc.push(Fx::Bolt {
                    a: weapon_tip(a),
                    b: (v.x, v.y - 14.0),
                });
                hit(v, t % 4 < 2);
                if v.look.tint != 0 {
                    v.look.tint = if t % 8 < 4 { SPARK } else { MAGIC };
                }
            }
        }
        Style::Swing => {
            let s = win(p, 0.05, 0.95);
            let blow = pulse(s, 2.0);
            a.arm = lerp(1.6, -0.6, blow);
            a.look.lean = blow * 3.0;
            let struck = blow > 0.85;
            hit(v, struck);
            if struck {
                sc.push(Fx::Burst {
                    at: weapon_tip(a),
                    p: blow,
                    colour: ACCENT,
                });
            }
        }
        Style::Duel => {
            a.arm = lerp(-0.5, 0.6, smooth(win(p, 0.0, 0.2)));
            v.arm = a.arm;
            v.reach = 0.0;
            let (ta, tv) = (weapon_tip(a), weapon_tip(v));
            if p > 0.2 {
                // The clash point drifts toward the loser as the bolts push.
                let push = 0.5 + 0.45 * smooth(win(p, 0.55, 1.0));
                let mid = (lerp(ta.0, tv.0, push), lerp(ta.1, tv.1, push));
                sc.push(Fx::Bolt { a: ta, b: mid });
                sc.push(Fx::Bolt { a: tv, b: mid });
                sc.push(Fx::Ball {
                    at: mid,
                    r: 3.0 + pulse(p, 6.0) * 2.5,
                    core: MAGIC_HOT,
                    rim: MAGIC,
                });
                hit(v, p > 0.9);
            }
        }
        Style::Comedy => {
            let poke = pulse(win(p, 0.15, 0.4), 1.0);
            a.arm = 0.1;
            a.reach = poke * 5.0;
            let jump = win(p, 0.35, 0.75);
            v.y = -(jump * std::f32::consts::PI).sin() * 18.0;
            v.arm = if jump > 0.0 { 1.4 } else { -0.9 };
            v.step = if jump > 0.0 { 1 } else { 0 };
            if p > 0.35 {
                sc.push(Fx::Word {
                    at: (v.x, v.y - 42.0),
                    text: b"!",
                    colour: ACCENT,
                });
            }
        }
    }
}

fn defeat(style: Style, p: f32, per_sq: f32, a: &mut Actor, v: &mut Actor, sc: &mut Scene) {
    a.x = end_of_defeat(style, per_sq);
    let fade = win(p, 0.75, 1.0);
    match style {
        Style::Jab | Style::Comedy => {
            let fall = smooth(win(p, 0.0, 0.45));
            if fall < 1.0 {
                v.look.lean = -fall * 22.0;
            } else {
                v.look.rot = 1;
            }
            v.arm = 1.2;
            if style == Style::Comedy {
                v.look.fade = 0.0;
                if p > 0.45 {
                    sc.push(Fx::Stars {
                        at: (v.x + 22.0, v.y - 12.0),
                        p,
                    });
                }
                a.y = -pulse(p, 3.0) * 6.0;
                a.arm = 1.2;
            } else {
                v.look.fade = fade;
                a.arm = lerp(0.05, 1.0, win(p, 0.3, 0.6));
            }
        }
        Style::Charge => {
            let q = smooth(p);
            v.x = q * per_sq * 1.8;
            v.y = -(q * std::f32::consts::PI).sin() * per_sq * 0.9;
            v.look.rot = (p * 7.0) as u8 % 4;
            v.look.fade = fade;
            a.arm = 0.0;
        }
        Style::Crumble => {
            v.look.shards = smooth(win(p, 0.05, 0.7));
            v.look.crumble = true;
            v.look.fade = fade;
            sc.push(Fx::Burst {
                at: (v.x, v.y - 4.0),
                p: win(p, 0.0, 0.8),
                colour: ASH,
            });
        }
        Style::Cast => {
            v.look.burn = smooth(win(p, 0.0, 0.7));
            v.look.fade = fade;
            v.look.lean = -2.0;
            a.arm = lerp(0.9, -0.5, win(p, 0.5, 0.9));
        }
        Style::Slam => {
            v.look.flat = 0.92;
            v.look.fade = fade;
            a.look.flat = 0.25 * (1.0 - win(p, 0.0, 0.25));
            a.arm = lerp(1.4, -0.5, win(p, 0.0, 0.4));
            sc.push(Fx::Ring {
                at: (0.0, 0.0),
                r: 6.0 + smooth(win(p, 0.0, 0.7)) * per_sq * 0.9,
            });
        }
        Style::Zap | Style::Duel => {
            v.look.tint = MAGIC_HOT;
            v.look.fade = smooth(win(p, 0.0, 0.85));
            sc.push(Fx::Burst {
                at: (v.x, v.y - 14.0),
                p: win(p, 0.0, 0.9),
                colour: MAGIC,
            });
            a.arm = lerp(0.6, -0.5, win(p, 0.4, 0.9));
            if style == Style::Duel {
                v.arm = -0.9;
            }
        }
        Style::Swing => {
            v.look.shards = smooth(win(p, 0.0, 0.8));
            v.look.fade = fade;
            a.arm = lerp(-0.6, 1.2, win(p, 0.2, 0.6));
        }
    }
}
