//! The story route: the next thing the game needs done, read from its
//! RAM (party, bag, event flags, badges) every time the bot is free to
//! walk, as a place to stand and a direction to face. The walking is
//! `nav.rs`'s.

use mizu_core::GameBoy;

use super::super::carts::Revision;
use super::super::pilot::{A, DOWN, LEFT, RIGHT, UP};
use super::super::ram::word;
use super::input::{Keys, GAP};
use super::nav::{self, Grid, Nav, Square};
use super::Ram;

const EVENT_FLAGS: u16 = 0xD747;
pub const EVENT_GOT_POKEDEX: u16 = 0x25;
const BAG_COUNT: u16 = 0xD31D;
const BAG: u16 = 0xD31E;
pub const BADGES: u16 = 0xD356;
/// `wSpritePlayerStateData1FacingDirection`.
const FACING: u16 = 0xC109;
pub const OAKS_PARCEL: u8 = 0x46;
const DOME_FOSSIL: u8 = 0x29;
const HELIX_FOSSIL: u8 = 0x2A;
const MT_MOON_B2F: u8 = 0x3D;

const ROUTE_1: u8 = 0x0C;
const ROUTE_3: u8 = 0x0E;
const OAKS_LAB: u8 = 0x28;
const VIRIDIAN_MART: u8 = 0x2A;
const VIRIDIAN_FOREST: u8 = 0x33;
const PEWTER_GYM: u8 = 0x36;
const CERULEAN_GYM: u8 = 0x41;
const POKECENTER: u8 = 6;
const CAVERN: u8 = 17;

/// Oak's three Poké Balls, Bulbasaur's last as in pokered's
/// `OaksLab.asm`: Charmander, Squirtle, Bulbasaur.
const BALLS: [u8; 3] = [8, 6, 7];

pub fn event(gb: &mut GameBoy, ram: Ram, e: u16) -> bool {
    gb.peek(ram.at(EVENT_FLAGS) + e / 8) & 1 << (e % 8) != 0
}

/// Starter choices, in `POKEMON_STARTER`'s order after "random".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Starter {
    Bulbasaur,
    Charmander,
    Squirtle,
}

impl Starter {
    pub fn of(n: u32) -> Self {
        [Self::Bulbasaur, Self::Charmander, Self::Squirtle][n as usize % 3]
    }

    /// The level the lead should reach before Brock and before Misty: a
    /// starter weak against them grinds longer.
    fn levels(self, rev: Revision) -> [u8; 2] {
        if rev == Revision::Yellow {
            return [16, 18];
        }
        match self {
            Self::Bulbasaur => [14, 21],
            Self::Squirtle => [14, 24],
            Self::Charmander => [20, 28],
        }
    }
}

/// Where to go and what to do there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Goal {
    /// Anywhere on this map.
    Map(u8),
    /// Stand next to whoever (or whatever) is on this square, face them
    /// and press A.
    Talk(Square),
    /// A Pokémon Center's nurse, whichever is nearest.
    Heal,
    /// Walk the grass (or a cave's floor) of this map until the lead levels.
    Grind(u8),
}

/// The party's lead: current HP, max HP and level.
pub struct Lead {
    pub hp: u16,
    pub max: u16,
    pub level: u8,
}

pub fn lead(gb: &mut GameBoy, ram: Ram) -> Lead {
    let p = ram.party_mons;
    Lead {
        hp: word(gb, p + 1),
        max: word(gb, p + 0x22),
        level: gb.peek(p + 0x21),
    }
}

pub fn has_item(gb: &mut GameBoy, ram: Ram, item: u8) -> bool {
    let n = gb.peek(ram.at(BAG_COUNT)).min(20);
    (0..n).any(|i| gb.peek(ram.at(BAG) + 2 * u16::from(i)) == item)
}

pub struct Story {
    rev: Revision,
    ram: Ram,
    pub starter: Starter,
    goal: Option<Goal>,
    pace: u8,
    /// The lead has a damaging move with PP left: kept by the bot, which
    /// knows the moves table.
    pub can_attack: bool,
}

impl Story {
    pub fn new(rev: Revision, starter: Starter) -> Self {
        Self {
            rev,
            ram: Ram::of(rev),
            starter,
            goal: None,
            pace: 0,
            can_attack: true,
        }
    }

    pub fn badges(&self, gb: &mut GameBoy) -> u8 {
        gb.peek(self.ram.at(BADGES))
    }

    /// Whether wild battles are worth fighting now: the lead is behind the
    /// level the next gym wants.
    pub fn grinding(&self, gb: &mut GameBoy) -> bool {
        matches!(self.goal, Some(Goal::Grind(_))) || {
            let b = self.badges(gb);
            let [brock, misty] = self.starter.levels(self.rev);
            let want = if b & 1 == 0 { brock } else { misty };
            lead(gb, self.ram).level < want
        }
    }

    /// The next thing the story needs.
    pub fn goal(&self, gb: &mut GameBoy) -> Option<Goal> {
        let r = self.ram;
        if gb.peek(r.party_count) == 0 {
            if gb.peek(r.cur_map) == OAKS_LAB {
                let i = self.starter as usize;
                let x = if self.rev == Revision::Yellow {
                    7
                } else {
                    BALLS[i]
                };
                return Some(Goal::Talk((OAKS_LAB, x, 3)));
            }
            // Oak stops the player at the edge of the grass and walks them
            // to his lab.
            return Some(Goal::Map(ROUTE_1));
        }
        let me = lead(gb, r);
        let badges = self.badges(gb);
        // The Super Nerd beside Mt. Moon's fossils keeps the player there
        // until one is taken, so on that floor it comes before healing.
        let fossil =
            badges & 1 != 0 && !has_item(gb, r, DOME_FOSSIL) && !has_item(gb, r, HELIX_FOSSIL);
        if fossil && gb.peek(r.cur_map) == MT_MOON_B2F {
            return Some(Goal::Talk((MT_MOON_B2F, 13, 6)));
        }
        if me.hp * 3 < me.max || !self.can_attack {
            return Some(Goal::Heal);
        }
        let dex = event(gb, r, EVENT_GOT_POKEDEX);
        if !dex {
            return Some(if has_item(gb, r, OAKS_PARCEL) {
                Goal::Talk((OAKS_LAB, 5, 2))
            } else {
                Goal::Map(VIRIDIAN_MART)
            });
        }
        let [brock, misty] = self.starter.levels(self.rev);
        let (want, grind, gym) = if badges & 1 == 0 {
            (brock, VIRIDIAN_FOREST, Goal::Talk((PEWTER_GYM, 4, 1)))
        } else if badges & 2 == 0 {
            (misty, ROUTE_3, Goal::Talk((CERULEAN_GYM, 4, 2)))
        } else {
            return None;
        };
        if me.level < want {
            return Some(Goal::Grind(grind));
        }
        // Then on to it, through the cave: the Helix Fossil.
        if fossil {
            return Some(Goal::Talk((MT_MOON_B2F, 13, 6)));
        }
        if me.hp < me.max {
            return Some(Goal::Heal);
        }
        Some(gym)
    }

    /// The buttons toward the current goal; `None` when there is no goal
    /// or it cannot be reached from here.
    pub fn buttons(&mut self, gb: &mut GameBoy, nav: &mut Nav, keys: &mut Keys) -> Option<u8> {
        let goal = self.goal(gb)?;
        if self.goal != Some(goal) {
            nav.reset();
            self.goal = Some(goal);
        }
        match goal {
            Goal::Map(m) => nav.toward(gb, &move |_, s| s.0 == m),
            Goal::Talk(npc) => Self::talk(
                gb,
                nav,
                keys,
                &move |g, s| {
                    s.0 == npc.0 && toward(s, npc).is_some() && g.walkable(s.1.into(), s.2.into())
                },
                &move |s| toward(s, npc).unwrap_or(UP),
            ),
            Goal::Heal => Self::talk(
                gb,
                nav,
                keys,
                &|g, s| g.tileset == POKECENTER && (s.1, s.2) == (3, 3),
                &|_| UP,
            ),
            Goal::Grind(m) => {
                let grind = move |g: &Grid, s: Square| {
                    s.0 == m
                        && (g.tileset == CAVERN
                            || g.tile[usize::from(s.2) * g.w + usize::from(s.1)] == g.grass)
                };
                match nav.toward(gb, &grind)? {
                    0 => Some(self.pace(gb, nav, &grind)),
                    b => Some(b),
                }
            }
        }
    }

    /// Stand where `at` accepts, turn to `face`, press A.
    fn talk(
        gb: &mut GameBoy,
        nav: &mut Nav,
        keys: &mut Keys,
        at: &dyn Fn(&Grid, Square) -> bool,
        face: &dyn Fn(Square) -> u8,
    ) -> Option<u8> {
        match nav.toward(gb, at)? {
            0 => {
                let face = face(nav.here(gb));
                let facing = nav::facing(gb.peek(FACING));
                Some(keys.tap(if facing == face { A } else { face }, GAP * 3))
            }
            b => Some(b),
        }
    }

    /// Back and forth across the grass, so wild Pokémon come.
    fn pace(
        &mut self,
        gb: &mut GameBoy,
        nav: &mut Nav,
        grind: &dyn Fn(&Grid, Square) -> bool,
    ) -> u8 {
        let here = nav.here(gb);
        self.pace = self.pace.wrapping_add(1);
        let rom = gb.rom();
        let Some(g) = nav.grid(rom, here.0) else {
            return 0;
        };
        let dirs = if (self.pace / 4).is_multiple_of(2) {
            [LEFT, RIGHT, UP, DOWN]
        } else {
            [RIGHT, LEFT, DOWN, UP]
        };
        let dir = dirs.into_iter().find(|&d| {
            let n = Nav::ahead(here, d);
            g.walkable(n.1.into(), n.2.into()) && grind(g, n)
        });
        dir.map_or(0, |d| nav.hold(gb, d))
    }
}

/// The direction from `s` to the square beside it, `to`.
fn toward(s: Square, to: Square) -> Option<u8> {
    nav::DIRS.into_iter().find(|&d| Nav::ahead(s, d) == to)
}
