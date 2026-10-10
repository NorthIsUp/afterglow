//! Battles, through the game's own menus: the move with the best expected
//! damage (power, the cartridge's type chart, same-type bonus, accuracy,
//! the attack-to-defence ratio, PP left), RUN from wild battles the story
//! has no use for, and the weakest move forgotten when a new one comes.

use mizu_core::GameBoy;

use super::super::pilot::{A, B, DOWN, LEFT, RIGHT, UP};
use super::input::{Keys, GAP};
use super::{screen, Ram};

/// `wBattleMon` and `wEnemyMon`, `battle_struct` in pokered.
const BATTLE_MON: u16 = 0xD014;
const ENEMY_MON: u16 = 0xCFE5;
const TYPE1: u16 = 5;
const MOVES: u16 = 8;
const ATTACK: u16 = 17;
const DEFENSE: u16 = 19;
const SPECIAL: u16 = 23;
const PP: u16 = 25;
/// `wIsInBattle`: 1 wild, 2 trainer.
const WILD: u8 = 1;
/// `TypeEffects`' first entries (water on fire, fire on grass, both
/// double), and the moves table's first two rows (Pound, Karate Chop): how
/// both are found in any revision.
const TYPE_CHART: [u8; 6] = [0x15, 0x14, 0x14, 0x14, 0x16, 0x14];
const MOVE_TABLE: [u8; 12] = [1, 0, 40, 0, 255, 35, 2, 0, 50, 0, 255, 25];
/// Types from this one up hit with Special.
const FIRST_SPECIAL_TYPE: u8 = 0x14;
/// The move rows' first line on the FIGHT screen.
const MOVE_ROW: usize = 13;

pub struct Battle {
    ram: Ram,
    /// Attacking type, defending type, multiplier in tenths.
    chart: Vec<(u8, u8, u8)>,
    /// ROM offset of the moves table, 6 bytes per move from move 1.
    moves: Option<usize>,
}

impl Battle {
    pub fn new(ram: Ram) -> Self {
        Self {
            ram,
            chart: Vec::new(),
            moves: None,
        }
    }

    fn learn(&mut self, rom: &[u8]) {
        if self.moves.is_some() {
            return;
        }
        self.moves = rom.windows(MOVE_TABLE.len()).position(|w| w == MOVE_TABLE);
        if let Some(at) = rom.windows(TYPE_CHART.len()).position(|w| w == TYPE_CHART) {
            self.chart = rom[at..]
                .chunks(3)
                .take_while(|c| c[0] != 0xFF && c.len() == 3)
                .map(|c| (c[0], c[1], c[2]))
                .collect();
        }
    }

    fn effect(&self, attack: u8, defend: u8) -> u32 {
        self.chart
            .iter()
            .find(|&&(a, d, _)| a == attack && d == defend)
            .map_or(10, |&(.., m)| u32::from(m))
    }

    /// Move `id`'s power, type and accuracy (out of 255).
    fn move_data(&self, rom: &[u8], id: u8) -> (u32, u8, u32) {
        let Some(row) = self
            .moves
            .and_then(|at| rom.get(at + 6 * (usize::from(id).max(1) - 1)..)?.get(..6))
        else {
            return (0, 0, 0);
        };
        (u32::from(row[2]), row[3], u32::from(row[4]))
    }

    /// Expected damage of move `id` from `from`'s battle struct on `to`'s,
    /// up to a constant.
    fn score(&self, gb: &mut GameBoy, id: u8, from: u16, to: u16) -> u32 {
        let (power, ty, acc) = self.move_data(gb.rom(), id);
        if power == 0 {
            return 0;
        }
        let w = |gb: &mut GameBoy, a: u16| u32::from(gb.peek(a)) << 8 | u32::from(gb.peek(a + 1));
        let (atk, def) = if ty >= FIRST_SPECIAL_TYPE {
            (w(gb, from + SPECIAL), w(gb, to + SPECIAL))
        } else {
            (w(gb, from + ATTACK), w(gb, to + DEFENSE))
        };
        let (t1, t2) = (gb.peek(to + TYPE1), gb.peek(to + TYPE1 + 1));
        let mut eff = self.effect(ty, t1);
        if t2 != t1 {
            eff = eff * self.effect(ty, t2) / 10;
        }
        let mine = [gb.peek(from + TYPE1), gb.peek(from + TYPE1 + 1)];
        let stab = if mine.contains(&ty) { 3 } else { 2 };
        power * eff * stab * acc * atk.max(1) / def.max(1) / 8
    }

    /// The move slot to use: best expected damage among moves with PP.
    fn best_move(&self, gb: &mut GameBoy) -> (usize, u32) {
        let (me, them) = (self.ram.at(BATTLE_MON), self.ram.at(ENEMY_MON));
        let mut best = (0, 0);
        let mut any = None;
        for slot in 0..4u16 {
            let id = gb.peek(me + MOVES + slot);
            if id == 0 || gb.peek(me + PP + slot) & 0x3F == 0 {
                continue;
            }
            any.get_or_insert(slot as usize);
            let s = self.score(gb, id, me, them);
            if s > best.1 {
                best = (slot as usize, s);
            }
        }
        if best.1 == 0 {
            best.0 = any.unwrap_or(0);
        }
        best
    }

    /// The slot holding the move least worth keeping, for "which move
    /// should be forgotten?": the one that does the least to a neutral
    /// target, status moves first.
    fn worst_move(&self, gb: &mut GameBoy) -> u8 {
        let me = self.ram.at(BATTLE_MON);
        (0..4u16)
            .min_by_key(|&slot| {
                let id = gb.peek(me + MOVES + slot);
                let (power, ty, acc) = self.move_data(gb.rom(), id);
                let stab = if [gb.peek(me + TYPE1), gb.peek(me + TYPE1 + 1)].contains(&ty) {
                    3
                } else {
                    2
                };
                power * stab * acc
            })
            .unwrap_or(0) as u8
    }

    pub fn buttons(&mut self, gb: &mut GameBoy, keys: &mut Keys, grind: bool) -> u8 {
        self.learn(gb.rom());
        let text = keys.text_ready(gb);
        let Some((cx, cy)) = screen::cursor(gb) else {
            return if text { keys.tap(A, GAP) } else { 0 };
        };
        let m = screen::menu(gb);
        if screen::shows(gb, b"FIGHT") && screen::shows(gb, b"RUN") {
            let wild = gb.peek(self.ram.in_battle) == WILD;
            let (_, damage) = self.best_move(gb);
            // A wild battle is worth it for the levels, and only when the
            // lead can hurt it.
            let run = wild && (!grind || damage == 0);
            let (want_x, want_item) = if run { (15, 1) } else { (9, 0) };
            let key = if m.x != want_x {
                if m.x < want_x {
                    RIGHT
                } else {
                    LEFT
                }
            } else if m.item != want_item {
                if m.item < want_item {
                    DOWN
                } else {
                    UP
                }
            } else {
                A
            };
            return keys.tap(key, GAP);
        }
        if screen::shows(gb, b"TYPE") {
            let (slot, _) = self.best_move(gb);
            let at = cy.saturating_sub(MOVE_ROW);
            let key = match at.cmp(&slot) {
                std::cmp::Ordering::Less => DOWN,
                std::cmp::Ordering::Greater => UP,
                std::cmp::Ordering::Equal => A,
            };
            return keys.tap(key, GAP);
        }
        if screen::shows(gb, b"YES") {
            // Keep the lead in, never nickname; learn every new move (the
            // weakest old one goes).
            let no = screen::shows(gb, b"change") || screen::shows(gb, b"nickname");
            return keys.tap(if no { B } else { A }, GAP);
        }
        if screen::shows(gb, b"forgotten") || (cx == 4 && m.max == 3 && !screen::shows(gb, b"ITEM"))
        {
            return keys.tap(screen::toward(m, self.worst_move(gb)), GAP);
        }
        // The bag, the party screen or anything else it did not open.
        #[cfg(test)]
        if text {
            eprintln!("battle: backing out of {m:?}\n{}", screen::dump(gb));
        }
        if text {
            return keys.tap(B, GAP);
        }
        0
    }
}
