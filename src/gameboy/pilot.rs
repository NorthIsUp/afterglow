//! Who holds the joypad: one small policy per bundled cartridge, a generic
//! one for anything else, and the Pokémon bot.

use mizu_core::GameBoy;

use super::carts::Pilot;
use super::pokemon::{Bot, Knobs};
use crate::next_rand;

pub const RIGHT: u8 = 0x01;
pub const LEFT: u8 = 0x02;
pub const UP: u8 = 0x04;
pub const DOWN: u8 = 0x08;
pub const A: u8 = 0x10;
pub const B: u8 = 0x20;
pub const START: u8 = 0x80;

pub enum Driver {
    Mash(u32),
    Tobu(Tobu),
    Rebound(u32),
    Pokemon(Box<Bot>),
}

impl Driver {
    pub fn new(p: Pilot, seed: u32, knobs: Knobs) -> Self {
        match p {
            Pilot::Mash => Self::Mash(seed),
            Pilot::Tobu => Self::Tobu(Tobu { rng: seed, dash: 0 }),
            Pilot::Rebound => Self::Rebound(seed),
            Pilot::Pokemon(r) => Self::Pokemon(Box::new(Bot::new(r, seed, knobs))),
        }
    }

    pub fn pilot(&self) -> Pilot {
        match self {
            Self::Mash(_) => Pilot::Mash,
            Self::Tobu(_) => Pilot::Tobu,
            Self::Rebound(_) => Pilot::Rebound,
            Self::Pokemon(b) => Pilot::Pokemon(b.revision()),
        }
    }

    /// Whether the engine should run this game as fast as it can, unseen.
    pub fn fast(&self) -> bool {
        matches!(self, Self::Pokemon(b) if b.fast())
    }

    /// The buttons to hold for the next frame.
    pub fn buttons(&mut self, gb: &mut GameBoy, frame: u64) -> u8 {
        match self {
            Self::Mash(rng) => mash(rng, frame),
            Self::Tobu(t) => t.buttons(gb, frame),
            Self::Rebound(rng) => rebound(rng, frame),
            Self::Pokemon(b) => b.buttons(gb, frame),
        }
    }
}

/// Start, then A, a few frames each about once a second and a half: past a
/// title screen and through most menus, and harmless once a game is going.
fn mash(rng: &mut u32, frame: u64) -> u8 {
    if frame < 120 || frame % 45 >= 4 {
        return 0;
    }
    match next_rand(rng) % 4 {
        0 => START,
        _ => A,
    }
}

/// Run right, jumping now and then: Rebound's levels scroll that way. Start
/// through the menus.
fn rebound(rng: &mut u32, frame: u64) -> u8 {
    if frame < 120 {
        return 0;
    }
    let menus = if frame % 90 < 4 { START | A } else { 0 };
    let jump = if (frame / 8).is_multiple_of(3 + u64::from(next_rand(rng) % 2)) {
        A
    } else {
        0
    };
    RIGHT | jump | menus
}

/// Tobu Tobu Girl Deluxe's own RAM (the game's `tobudx.sym`).
mod tobu {
    pub const GAMESTATE: u16 = 0xC0A5;
    pub const SCENE_STATE: u16 = 0xC0A6;
    pub const PLAYER_X: u16 = 0xC0C2;
    pub const PLAYER_Y: u16 = 0xC0C3;
    pub const PLAYER_YDIR: u16 = 0xC0C5;
    pub const ENTITY_X: u16 = 0xC154;
    pub const ENTITY_Y: u16 = 0xC15E;
    pub const ENTITY_TYPE: u16 = 0xC168;
    pub const ENTITIES: u16 = 10;
    pub const INGAME: u8 = 4;
    pub const DOWN: u8 = 3;
    pub const SPIKES: u8 = 1;
    pub const FIREBALL: u8 = 2;
    pub const PORTAL: u8 = 8;
    pub const CLOUD: u8 = 9;
}

pub struct Tobu {
    rng: u32,
    dash: u8,
}

impl Tobu {
    /// Steer under whatever she can bounce on next, clear of spikes and
    /// fireballs; dash up when the next bounce is far above. Menus: A.
    fn buttons(&mut self, gb: &mut GameBoy, frame: u64) -> u8 {
        if gb.peek(tobu::GAMESTATE) != tobu::INGAME || gb.peek(tobu::SCENE_STATE) != 0 {
            return match frame % 60 {
                0..3 => START,
                30..33 => A,
                _ => 0,
            };
        }
        let (px, py) = (
            i32::from(gb.peek(tobu::PLAYER_X)),
            i32::from(gb.peek(tobu::PLAYER_Y)),
        );
        let falling = gb.peek(tobu::PLAYER_YDIR) == tobu::DOWN;
        let mut best: Option<(i32, i32)> = None;
        let mut danger = 0i32;
        for i in 0..tobu::ENTITIES {
            let kind = gb.peek(tobu::ENTITY_TYPE + i);
            if kind == 0 || kind > tobu::CLOUD {
                continue;
            }
            let ex = i32::from(gb.peek(tobu::ENTITY_X + i));
            let ey = i32::from(gb.peek(tobu::ENTITY_Y + i));
            if kind == tobu::SPIKES || kind == tobu::FIREBALL {
                if (ey - py).abs() < 40 && (ex - px).abs() < 24 {
                    danger = if ex > px { -1 } else { 1 };
                }
                continue;
            }
            // Below her (larger y) while falling; anything near while rising.
            let dy = ey - py;
            if falling && dy < -4 {
                continue;
            }
            let score = (ex - px).abs() * 2 + dy.abs() - if kind == tobu::PORTAL { 40 } else { 0 };
            if best.is_none_or(|(s, _)| score < s) {
                best = Some((score, ex));
            }
        }
        let mut b = match (danger, best) {
            (d, _) if d < 0 => LEFT,
            (d, _) if d > 0 => RIGHT,
            (_, Some((_, ex))) if ex > px + 3 => RIGHT,
            (_, Some((_, ex))) if ex < px - 3 => LEFT,
            _ => 0,
        };
        if self.dash > 0 {
            self.dash -= 1;
        } else if !falling && best.is_none() && next_rand(&mut self.rng).is_multiple_of(40) {
            self.dash = 10;
            b |= UP | A;
        }
        b
    }
}
