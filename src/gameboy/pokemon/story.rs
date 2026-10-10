//! The story route: the next thing the game needs done, read from its
//! RAM (party, bag, event flags, badges) every time the bot is free to
//! walk, as a place to stand and a direction to face. The walking is
//! `nav.rs`'s.

use mizu_core::GameBoy;

use super::super::carts::Revision;
use super::super::pilot::{A, DOWN, LEFT, RIGHT, UP};
use super::super::ram::word;
use super::battle;
use super::field::{self, Kind};
use super::input::{Keys, GAP};
use super::nav::{self, Grid, Nav, Square};
use super::Ram;

const EVENT_FLAGS: u16 = 0xD747;
pub const EVENT_GOT_POKEDEX: u16 = 0x25;
const BILL_ASKED: u16 = 0x55E;
const CELL_SEPARATOR: u16 = 0x55B;
pub const GOT_SS_TICKET: u16 = 0x55C;
pub const GOT_HM01: u16 = 0x5E0;
const FIRST_LOCK: u16 = 0x161;
const SECOND_LOCK: u16 = 0x160;
/// `wFirstLockTrashCanIndex`, the second lock's after it.
const TRASH_CAN: u16 = 0xD743;
pub const BADGES: u16 = 0xD356;
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
const ROUTE_6: u8 = 0x11;
const ROUTE_7: u8 = 0x12;
const ROUTE_24: u8 = 0x23;
const CERULEAN_MART: u8 = 0x43;
const BILLS_HOUSE: u8 = 0x58;
const VERMILION_GYM: u8 = 0x5C;
const SS_ANNE_CAPTAINS_ROOM: u8 = 0x65;
const CELADON_GYM: u8 = 0x86;
/// Each gym in badge order: where to grind for it, and its leader.
const GYMS: [(u8, Square); 4] = [
    (VIRIDIAN_FOREST, (PEWTER_GYM, 4, 1)),
    (ROUTE_3, (CERULEAN_GYM, 4, 2)),
    (ROUTE_6, (VERMILION_GYM, 5, 1)),
    (ROUTE_7, (CELADON_GYM, 4, 3)),
];
/// Lt. Surge's index in `GYMS`: Cut and two trash cans stand before him.
const SURGE: usize = 2;
/// Vermilion Gym's electric gate, the block the second lock clears.
const GATE: [(u8, u8); 4] = [(4, 4), (5, 4), (4, 5), (5, 5)];
/// Species that can learn Cut: the two starters that can, and the two
/// grass Pokémon on Route 24 in every revision, Oddish and Bellsprout.
const CUTTERS: [u8; 10] = [0x99, 0x09, 0x9A, 0xB0, 0xB2, 0xB4, 0xB9, 0xBA, 0xBC, 0xBD];
const CATCH: [u8; 2] = [0xB9, 0xBC];
const POKECENTER: u8 = 6;
/// Celadon's hotel: a Pokémon Center's tiles and counter, and no nurse.
const CELADON_HOTEL: u8 = 0x8C;
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

    /// The level the lead should reach before each of `GYMS`. A starter
    /// weak against one grinds longer.
    fn levels(self, rev: Revision) -> [u8; GYMS.len()] {
        if rev == Revision::Yellow {
            return [16, 18, 30, 40];
        }
        match self {
            Self::Bulbasaur => [14, 21, 26, 32],
            Self::Squirtle => [14, 24, 30, 40],
            Self::Charmander => [20, 28, 26, 28],
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
    /// Stand on this square, face this way and press A: something only
    /// answers from one side, or across a counter.
    Press(Square, u8),
    /// Poké Balls from Cerulean's mart.
    Buy,
    /// Teach Cut to this party slot.
    Teach(u8),
    /// Walk this map's grass with a Poké Ball for anything in `CATCH`.
    Catch(u8),
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
        hp: word(gb, p + battle::HP),
        max: word(gb, p + 0x22),
        level: gb.peek(p + 0x21),
    }
}

pub fn has_item(gb: &mut GameBoy, ram: Ram, item: u8) -> bool {
    field::bag_index(gb, ram, item).is_some()
}

/// The party slot that knows Cut.
pub fn cutter(gb: &mut GameBoy, ram: Ram) -> Option<u8> {
    field::slot(gb, ram, |gb, s| field::knows(gb, ram, s, field::CUT))
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
    /// An errand the bot should run through the menus.
    pub errand: Option<Kind>,
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
            errand: None,
        }
    }

    pub fn badges(&self, gb: &mut GameBoy) -> u8 {
        gb.peek(self.ram.at(BADGES))
    }

    /// Species worth a Poké Ball now.
    pub fn catching(&self) -> &'static [u8] {
        if matches!(self.goal, Some(Goal::Catch(_))) {
            &CATCH
        } else {
            &[]
        }
    }

    /// Whether wild battles are worth fighting now: the lead is behind the
    /// level the next gym wants.
    pub fn grinding(&self, gb: &mut GameBoy) -> bool {
        matches!(self.goal, Some(Goal::Grind(_))) || {
            let b = self.badges(gb);
            let levels = self.starter.levels(self.rev);
            let want = levels[(b.trailing_ones() as usize).min(levels.len() - 1)];
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
        let next = badges.trailing_ones() as usize;
        let &(grind, leader) = GYMS.get(next)?;
        if next == SURGE {
            if let Some(g) = self.to_vermilion(gb) {
                return Some(g);
            }
        }
        if me.level < self.starter.levels(self.rev)[next] {
            return Some(Goal::Grind(grind));
        }
        // Then on to it, through the cave: the Helix Fossil.
        if fossil {
            return Some(Goal::Talk((MT_MOON_B2F, 13, 6)));
        }
        if me.hp < me.max {
            return Some(Goal::Heal);
        }
        Some(if next == SURGE {
            self.surge(gb)
        } else {
            Goal::Talk(leader)
        })
    }

    /// Cut, the way speedrun routes get it: a Pokémon that can learn it
    /// (caught on Route 24 if the starter cannot), Bill's S.S. Ticket, and
    /// HM01 from the S.S. Anne's captain.
    fn to_vermilion(&self, gb: &mut GameBoy) -> Option<Goal> {
        let r = self.ram;
        let Some(learner) =
            field::slot(gb, r, |gb, s| CUTTERS.contains(&gb.peek(field::mon(r, s))))
        else {
            return Some(if has_item(gb, r, field::POKE_BALL) {
                Goal::Catch(ROUTE_24)
            } else {
                Goal::Buy
            });
        };
        if !event(gb, r, BILL_ASKED) {
            return Some(Goal::Talk((BILLS_HOUSE, 6, 5)));
        }
        if !event(gb, r, CELL_SEPARATOR) {
            return Some(Goal::Press((BILLS_HOUSE, 1, 5), UP));
        }
        if !event(gb, r, GOT_SS_TICKET) {
            return Some(Goal::Talk((BILLS_HOUSE, 4, 4)));
        }
        if !event(gb, r, GOT_HM01) {
            return Some(Goal::Talk((SS_ANNE_CAPTAINS_ROOM, 4, 2)));
        }
        cutter(gb, r).is_none().then_some(Goal::Teach(learner))
    }

    /// Lt. Surge, behind a gate two trash cans open: the first can's
    /// number is in RAM from the start, the second's once the first opens.
    fn surge(&self, gb: &mut GameBoy) -> Goal {
        let r = self.ram;
        if event(gb, r, SECOND_LOCK) {
            return Goal::Talk(GYMS[SURGE].1);
        }
        let second = u16::from(event(gb, r, FIRST_LOCK));
        let (x, y) = trash_can(gb.peek(r.at(TRASH_CAN) + second));
        Goal::Talk((VERMILION_GYM, x, y))
    }

    /// The buttons toward the current goal; `None` when there is no goal
    /// or it cannot be reached from here.
    pub fn buttons(&mut self, gb: &mut GameBoy, nav: &mut Nav, keys: &mut Keys) -> Option<u8> {
        let goal = self.goal(gb)?;
        if self.goal != Some(goal) {
            nav.reset();
            self.goal = Some(goal);
        }
        if event(gb, self.ram, SECOND_LOCK) {
            nav.open(gb.rom(), VERMILION_GYM, &GATE);
        }
        match goal {
            Goal::Map(m) => nav.toward(gb, &move |_, s| s.0 == m),
            Goal::Press(at, dir) => Self::talk(gb, nav, keys, &move |_, s| s == at, &move |_| dir),
            Goal::Buy => {
                let at = (CERULEAN_MART, 2, 5);
                match nav.toward(gb, &move |_, s| s == at)? {
                    0 if nav::facing(gb.peek(nav::FACING)) == LEFT => {
                        self.errand = Some(Kind::Buy);
                        Some(0)
                    }
                    0 => Some(keys.tap(LEFT, GAP * 3)),
                    b => Some(b),
                }
            }
            Goal::Teach(slot) => {
                self.errand = Some(Kind::Teach(slot));
                Some(0)
            }
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
                &|g, s| g.tileset == POKECENTER && s.0 != CELADON_HOTEL && (s.1, s.2) == (3, 3),
                &|_| UP,
            ),
            Goal::Grind(m) | Goal::Catch(m) => {
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
                let facing = nav::facing(gb.peek(nav::FACING));
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
        let dirs = if (self.pace / 4).is_multiple_of(2) {
            [LEFT, RIGHT, UP, DOWN]
        } else {
            [RIGHT, LEFT, DOWN, UP]
        };
        let free = dirs.map(|d| !nav.person(Nav::ahead(here, d)));
        let rom = gb.rom();
        let Some(g) = nav.grid(rom, here.0) else {
            return 0;
        };
        let dir = dirs.into_iter().zip(free).find_map(|(d, free)| {
            let n = Nav::ahead(here, d);
            (free && g.walkable(n.1.into(), n.2.into()) && grind(g, n)).then_some(d)
        });
        dir.map_or(0, |d| nav.hold(gb, d))
    }
}

/// Vermilion Gym's trash can `i`: three to a column, five columns, as
/// pokered's hidden events number them.
pub fn trash_can(i: u8) -> (u8, u8) {
    (1 + 2 * (i / 3), 7 + 2 * (i % 3))
}

/// The direction from `s` to the square beside it, `to`.
fn toward(s: Square, to: Square) -> Option<u8> {
    nav::DIRS.into_iter().find(|&d| Nav::ahead(s, d) == to)
}
