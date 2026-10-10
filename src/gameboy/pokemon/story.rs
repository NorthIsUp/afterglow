//! The story route: the next thing the game needs done, read from its
//! RAM (party, bag, event flags, badges) every time the bot is free to
//! walk, as a place to stand and a direction to face. The walking is
//! `nav.rs`'s.

use mizu_core::GameBoy;

use super::super::carts::Revision;
use super::super::pilot::{A, DOWN, LEFT, RIGHT, UP};
use super::input::{Keys, GAP};
use super::nav::{Grid, Nav, Square};
use super::Ram;

const EVENT_FLAGS: u16 = 0xD747;
pub const EVENT_GOT_POKEDEX: u16 = 0x25;
const BAG_COUNT: u16 = 0xD31D;
const BAG: u16 = 0xD31E;
const BADGES: u16 = 0xD356;
const PARTY_MONS: u16 = 0xD16B;
/// `wSpritePlayerStateData1FacingDirection`.
const FACING: u16 = 0xC109;
const OAKS_PARCEL: u8 = 0x46;

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
    /// Stand on this square, face this way and press A.
    Talk(Square, u8),
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
    let p = ram.at(PARTY_MONS);
    let w = |gb: &mut GameBoy, a: u16| u16::from(gb.peek(a)) << 8 | u16::from(gb.peek(a + 1));
    Lead {
        hp: w(gb, p + 1),
        max: w(gb, p + 0x22),
        level: gb.peek(p + 0x21),
    }
}

fn has_item(gb: &mut GameBoy, ram: Ram, item: u8) -> bool {
    let n = gb.peek(ram.at(BAG_COUNT)).min(20);
    (0..n).any(|i| gb.peek(ram.at(BAG) + 2 * u16::from(i)) == item)
}

pub struct Story {
    rev: Revision,
    ram: Ram,
    pub starter: Starter,
    goal: Option<Goal>,
    pace: u8,
}

impl Story {
    pub fn new(rev: Revision, starter: Starter) -> Self {
        Self {
            rev,
            ram: Ram::of(rev),
            starter,
            goal: None,
            pace: 0,
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
                return Some(Goal::Talk((OAKS_LAB, x, 4), UP));
            }
            // Oak stops the player at the edge of the grass and walks them
            // to his lab.
            return Some(Goal::Map(ROUTE_1));
        }
        let me = lead(gb, r);
        if me.hp * 3 < me.max {
            return Some(Goal::Heal);
        }
        let dex = event(gb, r, EVENT_GOT_POKEDEX);
        if !dex {
            return Some(if has_item(gb, r, OAKS_PARCEL) {
                Goal::Talk((OAKS_LAB, 5, 3), UP)
            } else {
                Goal::Map(VIRIDIAN_MART)
            });
        }
        let badges = self.badges(gb);
        let [brock, misty] = self.starter.levels(self.rev);
        let (want, grind, gym) = if badges & 1 == 0 {
            (brock, VIRIDIAN_FOREST, Goal::Talk((PEWTER_GYM, 4, 2), UP))
        } else if badges & 2 == 0 {
            (misty, ROUTE_3, Goal::Talk((CERULEAN_GYM, 4, 3), UP))
        } else {
            return None;
        };
        if me.level < want {
            return Some(Goal::Grind(grind));
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
            Goal::Talk(at, face) => Self::talk(gb, nav, keys, &move |_, s| s == at, face),
            Goal::Heal => Self::talk(
                gb,
                nav,
                keys,
                &|g, s| g.tileset == POKECENTER && (s.1, s.2) == (3, 3),
                UP,
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
        face: u8,
    ) -> Option<u8> {
        match nav.toward(gb, at)? {
            0 => {
                let facing = match gb.peek(FACING) {
                    0x04 => UP,
                    0x08 => LEFT,
                    0x0C => RIGHT,
                    _ => DOWN,
                };
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
