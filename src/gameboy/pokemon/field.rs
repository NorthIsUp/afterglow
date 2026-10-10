//! Errands through the game's own menus outside battle: cutting a tree
//! from the party menu, teaching Cut from the bag, buying Poké Balls at a
//! mart. Each step is read off the screen, so a menu that opens late or a
//! text box in between is just waited out.

use mizu_core::GameBoy;

use super::super::pilot::{A, B, DOWN, START};
use super::battle::{Battle, MOVES};
use super::input::{Keys, GAP};
use super::{nav, screen, Ram};

pub const CUT: u8 = 0x0F;
pub const HM01: u8 = 0xC4;
pub const POKE_BALL: u8 = 0x04;
/// Hyper Potion, Super Potion, Potion: the best first.
pub const POTIONS: [u8; 3] = [0x12, 0x13, 0x14];
/// Potions bought in one visit.
const STOCK: u8 = 4;
const FRESH_WATER: u8 = 0x3C;
pub const BICYCLE: u8 = 0x06;
/// `wWalkBikeSurfState`: 0 walking.
const WALK_BIKE_SURF: u16 = 0xD700;
/// Balls bought in one visit.
const BALLS: u8 = 5;
const PARTY_MON: u16 = 44;
const BAG_COUNT: u16 = 0xD31D;
const BAG: u16 = 0xD31E;
/// `wListScrollOffset`, below `wFontLoaded` so the same in every revision.
const LIST_SCROLL: u16 = 0xCC36;
/// Long enough for any errand; past it something unexpected is open.
const GIVE_UP: u32 = 60 * 90;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Cut the faced tree with this party slot.
    Cut(u8),
    /// Teach HM01 to this party slot.
    Teach(u8),
    /// Buy Poké Balls from the clerk the player faces.
    Buy,
    /// Use this item from the bag, on nobody.
    Use(u8),
    /// Pick this entry from the elevator panel the player faces, to send
    /// it to this map.
    Floor(u8, u8),
    /// A drink from the vending machine the player faces.
    Drink,
    /// Use this potion on the lead.
    Potion(u8),
    /// Buy potions from the clerk the player faces, the first the shop
    /// lists.
    Stock,
}

pub struct Errand {
    pub kind: Kind,
    frames: u32,
    used: bool,
}

impl Errand {
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            frames: 0,
            used: false,
        }
    }

    /// The next button; `None` once done (or given up) with every menu shut.
    pub fn buttons(
        &mut self,
        gb: &mut GameBoy,
        ram: Ram,
        keys: &mut Keys,
        battle: &Battle,
    ) -> Option<u8> {
        self.frames += 1;
        let open = gb.peek(ram.font_loaded) & 1 != 0
            || screen::has_text(gb)
            || screen::cursor(gb).is_some();
        let done = self.frames > GIVE_UP
            || self.used
            || match self.kind {
                Kind::Teach(slot) => knows(gb, ram, slot, CUT),
                Kind::Buy => count(gb, ram, POKE_BALL) >= BALLS,
                Kind::Floor(_, map) => nav::lift(gb, ram) == map,
                Kind::Drink => count(gb, ram, FRESH_WATER) > 0,
                Kind::Stock => potions(gb, ram) >= STOCK,
                // The bag uses the Bicycle as soon as it is picked: no USE.
                Kind::Use(BICYCLE) => riding(gb, ram),
                Kind::Cut(_) | Kind::Use(_) | Kind::Potion(_) => false,
            };
        if done && !open {
            return None;
        }
        if !keys.text_ready(gb) {
            return Some(0);
        }
        if done {
            return Some(keys.tap(B, GAP));
        }
        if !open {
            let talk = matches!(
                self.kind,
                Kind::Buy | Kind::Floor(..) | Kind::Drink | Kind::Stock
            );
            return Some(keys.tap(if talk { A } else { START }, GAP));
        }
        if screen::shows(gb, b"hacked")
            || screen::shows(gb, b"anything to CUT")
            || screen::shows(gb, b"enough money")
        {
            self.used = true;
        }
        let Some((cx, cy)) = screen::cursor(gb) else {
            return Some(keys.tap(A, GAP));
        };
        let m = screen::menu(gb);
        let on = |gb: &mut GameBoy, label: &[u8]| {
            screen::row(gb, cy)[cx..]
                .windows(label.len())
                .any(|w| w == label)
        };
        let key = if screen::shows(gb, b"OPTION") {
            let want: &[u8] = match self.kind {
                Kind::Teach(_) | Kind::Use(_) | Kind::Potion(_) => b"ITEM",
                Kind::Cut(_) => b"POKeMON",
                Kind::Buy | Kind::Floor(..) | Kind::Drink | Kind::Stock => {
                    return Some(keys.tap(B, GAP))
                }
            };
            if on(gb, want) {
                A
            } else {
                DOWN
            }
        } else if screen::shows(gb, b"BUY") && screen::shows(gb, b"SELL") {
            screen::toward(m.item, 0)
        } else if cx == 0 && (screen::shows(gb, b"which") || screen::shows(gb, b"Choose")) {
            match self.kind {
                Kind::Cut(slot) | Kind::Teach(slot) => screen::toward(m.item, slot),
                Kind::Potion(_) => {
                    self.used = m.item == 0;
                    screen::toward(m.item, 0)
                }
                _ => B,
            }
        } else if screen::shows(gb, b"SWITCH") && screen::shows(gb, b"STATS") {
            if on(gb, b"CUT") {
                A
            } else {
                DOWN
            }
        } else if screen::shows(gb, b"TOSS") {
            // USE: an item used on nobody is done once it is chosen.
            self.used |= matches!(self.kind, Kind::Use(_)) && m.item == 0;
            screen::toward(m.item, 0)
        } else if screen::shows(gb, b"forgotten") {
            let Kind::Teach(slot) = self.kind else {
                return Some(keys.tap(B, GAP));
            };
            screen::toward(m.item, battle.worst_move(gb, mon(ram, slot)))
        } else if screen::shows(gb, b"YES") {
            A
        } else {
            match self.kind {
                Kind::Teach(_) | Kind::Use(_) | Kind::Potion(_) => {
                    let item = match self.kind {
                        Kind::Use(item) | Kind::Potion(item) => item,
                        _ => HM01,
                    };
                    match bag_index(gb, ram, item) {
                        Some(i) => toward_item(gb, m.item, i),
                        None => B,
                    }
                }
                Kind::Floor(i, _) => toward_item(gb, m.item, i),
                // Fresh Water, the cheapest, tops the machine's list.
                Kind::Drink => screen::toward(m.item, 0),
                Kind::Buy if on(gb, b"POKe BALL") => A,
                Kind::Stock if on(gb, b"POTION") => A,
                // The end of the list, and no potion on it.
                Kind::Stock if on(gb, b"CANCEL") => {
                    self.used = true;
                    B
                }
                Kind::Buy | Kind::Stock => DOWN,
                Kind::Cut(_) => B,
            }
        };
        Some(keys.tap(key, GAP))
    }
}

/// `screen::toward` for a scrolled list: the cursor on visible item `at`,
/// heading for entry `i`.
pub fn toward_item(gb: &mut GameBoy, at: u8, i: u8) -> u8 {
    screen::toward(at.wrapping_add(gb.peek(LIST_SCROLL)), i)
}

/// `wPartyMons`' entry for this slot.
pub fn mon(ram: Ram, slot: u8) -> u16 {
    ram.party_mons + PARTY_MON * u16::from(slot)
}

pub fn knows(gb: &mut GameBoy, ram: Ram, slot: u8, id: u8) -> bool {
    (0..4).any(|i| gb.peek(mon(ram, slot) + MOVES + i) == id)
}

/// The first party slot that passes `f`.
pub fn slot(gb: &mut GameBoy, ram: Ram, f: impl Fn(&mut GameBoy, u8) -> bool) -> Option<u8> {
    (0..gb.peek(ram.party_count).min(6)).find(|&s| f(gb, s))
}

pub fn bag_index(gb: &mut GameBoy, ram: Ram, item: u8) -> Option<u8> {
    let n = gb.peek(ram.at(BAG_COUNT)).min(20);
    (0..n).find(|&i| gb.peek(ram.at(BAG) + 2 * u16::from(i)) == item)
}

pub fn riding(gb: &mut GameBoy, ram: Ram) -> bool {
    gb.peek(ram.at(WALK_BIKE_SURF)) != 0
}

/// Potions of any kind in the bag.
pub fn potions(gb: &mut GameBoy, ram: Ram) -> u8 {
    POTIONS.iter().map(|&p| count(gb, ram, p)).sum()
}

pub fn count(gb: &mut GameBoy, ram: Ram, item: u8) -> u8 {
    bag_index(gb, ram, item).map_or(0, |i| gb.peek(ram.at(BAG) + 2 * u16::from(i) + 1))
}
