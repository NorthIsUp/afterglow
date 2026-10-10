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
use super::grid::Grid;
use super::input::{Keys, GAP};
use super::nav::{self, Nav, Square};
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
/// `wToggleableObjectFlags`: a set bit hides that object.
const TOGGLED_OFF: u16 = 0xD5A6;
const GAME_CORNER_ROCKET: u16 = 0x46;
const FOUND_ROCKET_HIDEOUT: u16 = 0x1B9;
/// The Rocket in B1F's elevator room, whose defeat opens its door.
const BEAT_LIFT_ROCKET: u16 = 0x675;
const BEAT_DOOR_ROCKET: u16 = 0x6A2;
const HIDEOUT_DOOR: u16 = 0x6A5;
const LIFT_KEY_DROPPED: u16 = 0x6A6;
pub const BEAT_HIDEOUT_GIOVANNI: u16 = 0x6A7;
const RESCUED_FUJI: u16 = 0x4CF;
pub const ROUTE_12_SNORLAX: u16 = 0x48F;
const ROUTE_16_SNORLAX: u16 = 0x4C9;
pub const SILPH_SCOPE: u8 = 0x48;
pub const POKE_FLUTE: u8 = 0x49;
const LIFT_KEY: u8 = 0x4A;
const CARD_KEY: u8 = 0x30;
/// Fresh Water, Soda Pop, Lemonade: any one gets a Saffron guard to let
/// the player by.
const DRINKS: [u8; 3] = [0x3C, 0x3D, 0x3E];
/// `wStatusFlags1`, and its bit for the guards having had their drink.
const STATUS_FLAGS: u16 = 0xD728;
const GAVE_DRINK: u16 = 6;
pub const BEAT_SILPH_GIOVANNI: u16 = 0x78F;
const BIKE_VOUCHER: u8 = 0x2D;
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
const ROUTE_12: u8 = 0x17;
const ROUTE_15: u8 = 0x1A;
const ROUTE_16: u8 = 0x1B;
const GAME_CORNER: u8 = 0x87;
const HIDEOUT_B4F: u8 = 0xCA;
const HIDEOUT_B1F: u8 = 0xC7;
pub const TOWER_6F: u8 = 0x93;
const TOWER_7F: u8 = 0x94;
const FUJIS_HOUSE: u8 = 0x95;
const FUCHSIA_GYM: u8 = 0x9D;
const MART_ROOF: u8 = 0x7E;
const BIKE_SHOP: u8 = 0x42;
const FAN_CLUB: u8 = 0x5A;
const SAFFRON_GYM: u8 = 0xB2;
const SILPH_5F: u8 = 0xD2;
const SILPH_11F: u8 = 0xEB;
const SNORLAX: Square = (ROUTE_12, 10, 62);
/// Each gym in badge order: where to grind for it, and its leader.
const GYMS: [(u8, Square); 6] = [
    (VIRIDIAN_FOREST, (PEWTER_GYM, 4, 1)),
    (ROUTE_3, (CERULEAN_GYM, 4, 2)),
    (ROUTE_6, (VERMILION_GYM, 5, 1)),
    (ROUTE_7, (CELADON_GYM, 4, 3)),
    (ROUTE_15, (FUCHSIA_GYM, 4, 10)),
    (ROUTE_7, (SAFFRON_GYM, 9, 8)),
];
/// Koga's index in `GYMS`: the Poké Flute stands before him.
const KOGA: usize = 4;
/// Sabrina's: Team Rocket holds Silph Co., and a guard who wants a drink
/// stands at every gate into Saffron.
const SABRINA: usize = 5;
/// Silph Co.'s card-key doors, each a block a script shuts until its
/// event: the event, the floor, and the block (x, y).
const DOORS: [(u16, u8, (u8, u8)); 19] = [
    (0x6FD, 0xCF, (2, 2)),
    (0x6FE, 0xCF, (2, 5)),
    (0x708, 0xD0, (4, 4)),
    (0x709, 0xD0, (8, 4)),
    (0x718, 0xD1, (2, 6)),
    (0x719, 0xD1, (6, 4)),
    (0x728, SILPH_5F, (3, 2)),
    (0x729, SILPH_5F, (3, 6)),
    (0x72A, SILPH_5F, (7, 5)),
    (0x73F, 0xD3, (2, 6)),
    (0x74C, 0xD4, (5, 3)),
    (0x74D, 0xD4, (10, 2)),
    (0x74E, 0xD4, (10, 6)),
    (0x758, 0xD5, (3, 4)),
    (0x768, 0xE9, (1, 4)),
    (0x769, 0xE9, (9, 2)),
    (0x76A, 0xE9, (9, 5)),
    (0x76B, 0xE9, (5, 6)),
    (0x778, 0xEA, (5, 4)),
];
type Squares = &'static [(u8, u8)];
/// Squares the cartridge's maps show one way and a script keeps the other
/// until an event: open once it is set, shut until then.
const LOCKS: [(u16, u8, Squares); 6] = [
    (SECOND_LOCK, VERMILION_GYM, &GATE),
    (FOUND_ROCKET_HIDEOUT, GAME_CORNER, &[(17, 4)]),
    (
        BEAT_LIFT_ROCKET,
        HIDEOUT_B1F,
        &[(24, 16), (25, 16), (24, 17), (25, 17)],
    ),
    (
        HIDEOUT_DOOR,
        HIDEOUT_B4F,
        &[(24, 10), (25, 10), (24, 11), (25, 11)],
    ),
    (ROUTE_12_SNORLAX, ROUTE_12, &[(SNORLAX.1, SNORLAX.2)]),
    (ROUTE_16_SNORLAX, ROUTE_16, &[(26, 10)]),
];
/// Lt. Surge's index in `GYMS`: Cut and two trash cans stand before him.
const SURGE: usize = 2;
/// Erika's: the Bicycle first, on the way back through Cerulean.
const ERIKA: usize = 3;
/// Vermilion Gym's electric gate, the block the second lock clears.
const GATE: [(u8, u8); 4] = [(4, 4), (5, 4), (4, 5), (5, 5)];
/// Species that can learn Cut: the two starters that can, and the two
/// grass Pokémon on Route 24 in every revision, Oddish and Bellsprout.
const CUTTERS: [u8; 10] = [0x99, 0x09, 0x9A, 0xB0, 0xB2, 0xB4, 0xB9, 0xBA, 0xBC, 0xBD];
const CATCH: [u8; 2] = [0xB9, 0xBC];
const POKECENTER: u8 = 6;
/// Every mart but the department store: its clerk across the counter from
/// (2, 5).
const MART: u8 = 2;
/// `wPlayerMoney`, three BCD bytes.
const MONEY: u16 = 0xD347;
/// Money enough to spend on potions without going short.
const SHOPPING_MONEY: u32 = 3000;
/// Celadon's hotel: a Pokémon Center's tiles and counter, and no nurse.
const CELADON_HOTEL: u8 = 0x8C;
const CAVERN: u8 = 17;
const OVERWORLD: u8 = 0;

/// Oak's three Poké Balls, Bulbasaur's last as in pokered's
/// `OaksLab.asm`: Charmander, Squirtle, Bulbasaur.
const BALLS: [u8; 3] = [8, 6, 7];

pub fn event(gb: &mut GameBoy, ram: Ram, e: u16) -> bool {
    flag(gb, ram.at(EVENT_FLAGS), e)
}

fn flag(gb: &mut GameBoy, at: u16, i: u16) -> bool {
    gb.peek(at + i / 8) & 1 << (i % 8) != 0
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
            return [16, 18, 30, 40, 48, 0];
        }
        match self {
            Self::Bulbasaur => [14, 21, 26, 32, 44, 50],
            Self::Squirtle => [14, 24, 30, 40, 44, 48],
            Self::Charmander => [20, 28, 26, 28, 42, 47],
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
    /// Stand on this square, face this way and run this errand through
    /// the menus: a clerk, a vending machine.
    Errand(Square, u8, Kind),
    /// Teach Cut to this party slot.
    Teach(u8),
    /// Walk this map's grass with a Poké Ball for anything in `CATCH`.
    Catch(u8),
    /// A Pokémon Center's nurse, whichever is nearest.
    Heal,
    /// Drink this potion.
    Potion(u8),
    /// Potions from whichever mart is nearest.
    Shop,
    /// Walk the grass (or a cave's floor) of this map until the lead levels.
    Grind(u8),
    /// Stand next to this square and use this item from the bag.
    Use(u8, Square),
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
        // The ghost on Pokémon Tower's sixth floor turns back anyone who runs.
        matches!(self.goal, Some(Goal::Grind(_))) || gb.peek(self.ram.cur_map) == TOWER_6F || {
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
            let potion = field::POTIONS.into_iter().find(|&p| has_item(gb, r, p));
            // A potion cannot raise a lead who fainted.
            return Some(match potion.filter(|_| self.can_attack && me.hp > 0) {
                Some(p) => Goal::Potion(p),
                None => Goal::Heal,
            });
        }
        let dex = event(gb, r, EVENT_GOT_POKEDEX);
        if !dex {
            return Some(if has_item(gb, r, OAKS_PARCEL) {
                Goal::Talk((OAKS_LAB, 5, 2))
            } else {
                Goal::Map(VIRIDIAN_MART)
            });
        }
        if field::potions(gb, r) < 2 && money(gb, r) >= SHOPPING_MONEY {
            return Some(Goal::Shop);
        }
        let next = badges.trailing_ones() as usize;
        // Yellow's Pikachu keeps only electric moves, and Silph Co.'s
        // Giovanni leads with ground types: no Marsh Badge there.
        if next == SABRINA && self.rev == Revision::Yellow {
            return None;
        }
        let &(grind, leader) = GYMS.get(next)?;
        let errand = match next {
            SURGE => self.to_vermilion(gb),
            ERIKA => self.bicycle(gb),
            KOGA => self.to_fuchsia(gb),
            SABRINA => self.to_saffron(gb),
            _ => None,
        };
        if let Some(g) = errand {
            return Some(g);
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
                Goal::Errand((CERULEAN_MART, 2, 5), LEFT, Kind::Buy)
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

    /// The Bicycle, twice as fast as walking: the Fan Club chairman's
    /// voucher in Vermilion, traded at Cerulean's bike shop across its
    /// counter.
    fn bicycle(&self, gb: &mut GameBoy) -> Option<Goal> {
        let r = self.ram;
        if has_item(gb, r, field::BICYCLE) {
            return None;
        }
        Some(if has_item(gb, r, BIKE_VOUCHER) {
            Goal::Press((BIKE_SHOP, 6, 4), UP)
        } else {
            Goal::Talk((FAN_CLUB, 3, 1))
        })
    }

    /// The Poké Flute, to wake the Snorlax on Route 12: the Silph Scope
    /// from Giovanni under the Game Corner unmasks the ghost at the top of
    /// Pokémon Tower, where Mr. Fuji waits.
    fn to_fuchsia(&self, gb: &mut GameBoy) -> Option<Goal> {
        let r = self.ram;
        if event(gb, r, ROUTE_12_SNORLAX) {
            return None;
        }
        Some(if has_item(gb, r, POKE_FLUTE) {
            Goal::Use(POKE_FLUTE, SNORLAX)
        } else if event(gb, r, RESCUED_FUJI) {
            Goal::Talk((FUJIS_HOUSE, 3, 1))
        } else if has_item(gb, r, SILPH_SCOPE) {
            Goal::Talk((TOWER_7F, 10, 3))
        } else {
            self.hideout(gb)
        })
    }

    /// Giovanni again, on Silph Co.'s top floor: a drink from the Celadon
    /// Mart roof for Saffron's gate guards, and the Card Key from the fifth
    /// floor for the doors.
    fn to_saffron(&self, gb: &mut GameBoy) -> Option<Goal> {
        let r = self.ram;
        if event(gb, r, BEAT_SILPH_GIOVANNI) {
            return None;
        }
        Some(if !self.saffron_open(gb) {
            Goal::Errand((MART_ROOF, 10, 2), UP, Kind::Drink)
        } else if !has_item(gb, r, CARD_KEY) {
            Goal::Talk((SILPH_5F, 21, 16))
        } else {
            Goal::Talk((SILPH_11F, 6, 9))
        })
    }

    /// The guards let the player into Saffron: they have had their drink,
    /// or the bag holds one for them.
    fn saffron_open(&self, gb: &mut GameBoy) -> bool {
        flag(gb, self.ram.at(STATUS_FLAGS), GAVE_DRINK)
            || DRINKS.iter().any(|&d| has_item(gb, self.ram, d))
    }

    /// The Rocket Hideout: a switch behind the poster a Rocket guards, two
    /// spinner floors down to the Rocket who drops the Lift Key, and its
    /// elevator back down to Giovanni's half of B4F.
    fn hideout(&self, gb: &mut GameBoy) -> Goal {
        let r = self.ram;
        if !event(gb, r, FOUND_ROCKET_HIDEOUT) {
            return if flag(gb, r.at(TOGGLED_OFF), GAME_CORNER_ROCKET) {
                Goal::Press((GAME_CORNER, 9, 5), UP)
            } else {
                Goal::Talk((GAME_CORNER, 9, 5))
            };
        }
        if !has_item(gb, r, LIFT_KEY) {
            // He drops it when spoken to after the battle.
            let x = if event(gb, r, LIFT_KEY_DROPPED) {
                10
            } else {
                11
            };
            return Goal::Talk((HIDEOUT_B4F, x, 2));
        }
        // Yellow's Jessie and James step out instead, and there is no door.
        if self.rev != Revision::Yellow && !event(gb, r, HIDEOUT_DOOR) {
            let x = if event(gb, r, BEAT_DOOR_ROCKET) {
                26
            } else {
                23
            };
            return Goal::Talk((HIDEOUT_B4F, x, 12));
        }
        let y = if event(gb, r, BEAT_HIDEOUT_GIOVANNI) {
            2
        } else {
            3
        };
        Goal::Talk((HIDEOUT_B4F, 25, y))
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
        for (e, map, squares) in LOCKS {
            let open = event(gb, self.ram, e) || e == HIDEOUT_DOOR && self.rev == Revision::Yellow;
            nav.set_open(gb.rom(), map, squares, open);
        }
        self.doors(gb, nav);
        nav.saffron = self.saffron_open(gb);
        // On the Bicycle wherever it can be ridden: outdoors.
        let r = self.ram;
        if gb.peek(r.tileset) == OVERWORLD
            && !field::riding(gb, r)
            && has_item(gb, r, field::BICYCLE)
        {
            self.errand = Some(Kind::Use(field::BICYCLE));
            return Some(0);
        }
        match goal {
            Goal::Map(m) => nav.toward(gb, &move |_, s| s.0 == m),
            Goal::Press(at, dir) => Self::talk(gb, nav, keys, &move |_, s| s == at, &move |_| dir),
            Goal::Errand(at, dir, kind) => match nav.toward(gb, &move |_, s| s == at)? {
                0 if nav::facing(gb.peek(nav::FACING)) == dir => {
                    self.errand = Some(kind);
                    Some(0)
                }
                0 => Some(keys.tap(dir, GAP * 3)),
                b => Some(b),
            },
            Goal::Teach(slot) => {
                self.errand = Some(Kind::Teach(slot));
                Some(0)
            }
            Goal::Potion(p) => {
                self.errand = Some(Kind::Potion(p));
                Some(0)
            }
            Goal::Shop => {
                let at = |g: &Grid, s: Square| {
                    g.tileset == MART && s.0 != VIRIDIAN_MART && (s.1, s.2) == (2, 5)
                };
                match nav.toward(gb, &at)? {
                    0 if nav::facing(gb.peek(nav::FACING)) == LEFT => {
                        self.errand = Some(Kind::Stock);
                        Some(0)
                    }
                    0 => Some(keys.tap(LEFT, GAP * 3)),
                    b => Some(b),
                }
            }
            Goal::Use(item, at) => match nav.toward(gb, &move |_, s| toward(s, at).is_some())? {
                0 => {
                    self.errand = Some(Kind::Use(item));
                    Some(0)
                }
                b => Some(b),
            },
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

    /// Silph Co.'s doors: shut until opened, and the Card Key opens any
    /// the player faces.
    fn doors(&self, gb: &mut GameBoy, nav: &mut Nav) {
        let r = self.ram;
        let key = has_item(gb, r, CARD_KEY);
        let here = gb.peek(r.cur_map);
        nav.doors.clear();
        for (e, map, (bx, by)) in DOORS {
            let (x, y) = (bx * 2, by * 2);
            let squares = [(x, y), (x + 1, y), (x, y + 1), (x + 1, y + 1)];
            let open = event(gb, r, e);
            nav.set_open(gb.rom(), map, &squares, open || key);
            if key && !open && map == here {
                nav.doors.extend(squares.map(|(x, y)| (map, x, y)));
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

fn money(gb: &mut GameBoy, ram: Ram) -> u32 {
    (0..3).fold(0, |n, i| {
        let b = u32::from(gb.peek(ram.at(MONEY) + i));
        n * 100 + (b >> 4) * 10 + (b & 15)
    })
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
