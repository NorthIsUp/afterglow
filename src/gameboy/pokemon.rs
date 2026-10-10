//! The Pokémon bot: plays Red, Blue or Yellow from the game's own RAM, laid
//! out by pret's pokered and pokeyellow disassemblies, along the story.
//!
//! `intro.rs` gets from power-on out of the house; `story.rs` names the
//! next thing the game needs (a starter, Oak's Parcel, the Pokédex, levels,
//! a badge) as a square to stand on; `nav.rs` routes there across maps;
//! `battle.rs` picks moves by expected damage and runs from wild battles it
//! has no use for. Text is answered `POKEMON_TEXT_MS` after it stops
//! printing, menus by reading their cursor. Held still for minutes, it
//! rewinds to a recent save state and never stands on that square again.

mod battle;
mod input;
mod intro;
mod nav;
mod screen;
mod story;
#[cfg(test)]
mod tests;

use std::collections::VecDeque;

use mizu_core::GameBoy;

use super::carts::Revision;
use super::pilot::{A, B, START};
use super::ram::Ram;
use battle::Battle;
use input::{Keys, GAP};
use intro::Intro;
use nav::Nav;
use story::{Starter, Story};

/// The `POKEMON_*` knobs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Knobs {
    /// How long printed text stays up before A.
    pub text_ms: u32,
    /// 0 picks one from the seed; 1 Bulbasaur, 2 Charmander, 3 Squirtle.
    pub starter: u32,
}

/// Save states kept for rewinding, one per this many frames of play.
const SNAPSHOTS: usize = 4;
const SNAPSHOT_EVERY: u64 = 60 * 60 * 2;
/// Frames on one square (battles and text boxes included) that count as
/// stuck: longer than any battle.
const STILL_LIMIT: u64 = 60 * 60 * 3;
/// A text box or menu open this long is one the bot is going round in.
const STUCK_TEXT: u32 = 60 * 20;

pub struct Bot {
    rev: Revision,
    ram: Ram,
    rng: u32,
    knobs: Knobs,
    keys: Keys,
    intro: Intro,
    nav: Nav,
    story: Story,
    battle: Battle,
    text_frames: u32,
    wander: u32,
    last_here: Option<nav::Square>,
    still: u64,
    snapshots: VecDeque<(Vec<u8>, nav::Square)>,
    rewinds: u32,
    started: bool,
}

impl Bot {
    pub fn new(rev: Revision, seed: u32, knobs: Knobs) -> Self {
        let ram = Ram::of(rev);
        let mut rng = seed | 1;
        let starter = Starter::of(match knobs.starter {
            0 => crate::next_rand(&mut rng),
            n => n - 1,
        });
        Self {
            rev,
            ram,
            rng,
            knobs,
            keys: Keys::new(knobs.text_ms),
            intro: Intro::new(rev),
            nav: Nav::new(rev),
            story: Story::new(rev, starter),
            battle: Battle::new(ram),
            text_frames: 0,
            wander: 0,
            last_here: None,
            still: 0,
            snapshots: VecDeque::new(),
            rewinds: 0,
            started: false,
        }
    }

    #[cfg(test)]
    pub fn intro(&self) -> &Intro {
        &self.intro
    }

    pub fn revision(&self) -> Revision {
        self.rev
    }

    #[cfg(test)]
    pub fn starter(&self) -> Starter {
        self.story.starter
    }

    /// Still in the intro: the engine runs it as fast as the core goes.
    pub fn fast(&self) -> bool {
        !self.intro.done
    }

    /// One line on where the bot is, for the bench log.
    #[cfg(test)]
    pub fn status(&self, gb: &mut GameBoy) -> String {
        let r = self.ram;
        let lead = story::lead(gb, r);
        format!(
            "map {:3} ({:2},{:2}) lv {} hp {}/{} badges {:08b} goal {:?} rewinds {} joyignore {:02x} options {:02x}",
            gb.peek(r.cur_map),
            gb.peek(r.x),
            gb.peek(r.y),
            lead.level,
            lead.hp,
            lead.max,
            self.story.badges(gb),
            self.story.goal(gb),
            self.rewinds,
            gb.peek(nav::JOY_IGNORE),
            gb.peek(r.options)
        )
    }

    pub fn buttons(&mut self, gb: &mut GameBoy, frame: u64) -> u8 {
        if !self.started {
            self.started = true;
            self.intro.resume(gb);
        }
        self.watchdog(gb, frame);
        if let Some(b) = self.keys.busy() {
            return b;
        }
        if let Some(b) = self.intro.buttons(gb, &mut self.keys, frame) {
            return b;
        }
        self.intro.watch(gb, frame);
        let r = self.ram;
        if gb.peek(r.in_battle) != 0 {
            // A step cut short by a battle is no wall, and the people it
            // walked around have moved by the end.
            self.nav.reset();
            self.text_frames = 0;
            let grind = self.story.grinding(gb);
            return self.battle.buttons(gb, &mut self.keys, grind);
        }
        if gb.peek(r.font_loaded) & 1 != 0 {
            self.nav.stop();
            return self.menus(gb, frame);
        }
        if let Some(d) = self.nav.stepping(gb) {
            return d;
        }
        // Checked between steps only: it reads the whole text box.
        if screen::has_text(gb) {
            return self.menus(gb, frame);
        }
        self.text_frames = 0;
        // Held in place with no text box the bot can see: some screen
        // (the Pokédex, a picture) is waiting on a button. B, not A: A
        // would talk to whoever it faces, again and again.
        if self.still > 60 * 3 && self.still % 120 < 20 {
            return self.keys.tap(B, 30);
        }
        self.story.can_attack = self.battle.can_attack(gb);
        if let Some(b) = self.story.buttons(gb, &mut self.nav, &mut self.keys) {
            return b;
        }
        self.roam(gb)
    }

    /// Text and menus outside battle: yes to everything but a nickname,
    /// out of any menu the bot did not open, A once text has printed.
    fn menus(&mut self, gb: &mut GameBoy, frame: u64) -> u8 {
        let text = self.keys.text_ready(gb);
        self.text_frames += 1;
        if let Some(key) = intro::naming_key(gb, frame) {
            return self.keys.tap(key, 8);
        }
        if self.text_frames > STUCK_TEXT {
            let key = if (frame / 12).is_multiple_of(3) {
                START
            } else {
                B
            };
            return self.keys.tap(key, 8);
        }
        if screen::cursor(gb).is_some() {
            if screen::shows(gb, b">HEAL") {
                return self.keys.tap(A, GAP);
            }
            if screen::shows(gb, b"YES") {
                let no = screen::shows(gb, b"nickname");
                return self.keys.tap(if no { B } else { A }, GAP);
            }
            // The start menu, a shop, a PC: nothing the route asks for.
            if text {
                return self.keys.tap(B, GAP);
            }
            return 0;
        }
        if text {
            return self.keys.tap(A, GAP);
        }
        0
    }

    /// No goal, or none it can reach: to the least-trodden neighbour, now
    /// and then pressing A at whatever it faces.
    fn roam(&mut self, gb: &mut GameBoy) -> u8 {
        self.wander += 1;
        if self.wander.is_multiple_of(7) {
            return self.keys.tap(A, GAP);
        }
        let dir = nav::DIRS[(crate::next_rand(&mut self.rng) % 4) as usize];
        self.nav.hold(gb, dir)
    }

    /// Some scripted moment it does not understand can hold it in place
    /// for good. Every couple of minutes of progress it keeps a save state;
    /// held still too long, it marks the square a trap and goes back to
    /// the newest one taken somewhere else.
    fn watchdog(&mut self, gb: &mut GameBoy, frame: u64) {
        if gb.peek(self.ram.party_count) == 0 {
            return;
        }
        let here = self.nav.here(gb);
        if self.last_here == Some(here) {
            self.still += 1;
        } else {
            self.last_here = Some(here);
            self.still = 0;
        }
        if frame.is_multiple_of(SNAPSHOT_EVERY) && self.still < STILL_LIMIT / 4 {
            let mut state = Vec::new();
            if gb.save_state(&mut state).is_ok() {
                if self.snapshots.len() == SNAPSHOTS {
                    self.snapshots.pop_front();
                }
                self.snapshots.push_back((state, here));
            }
        }
        if self.still < STILL_LIMIT {
            return;
        }
        self.still = 0;
        // Whatever holds it here, it is not walking back into it.
        self.nav.trap(here);
        // The newest state taken somewhere else; the ones after it are in
        // the trap.
        while self.snapshots.back().is_some_and(|(_, at)| *at == here) {
            self.snapshots.pop_back();
        }
        if let Some((state, _)) = self.snapshots.back() {
            let _ = gb.load_state(state.as_slice());
        }
        self.rewinds += 1;
        self.rng = self
            .rng
            .wrapping_mul(0x9E37_79B9)
            .wrapping_add(self.rewinds);
        self.nav.reset();
        self.keys = Keys::new(self.knobs.text_ms);
    }
}
