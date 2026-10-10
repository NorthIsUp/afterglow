//! The Pokémon bot: plays Red, Blue or Yellow from the game's own RAM, laid
//! out by pret's pokered and pokeyellow disassemblies, along the story.
//!
//! `intro.rs` gets from power-on out of the house; `story.rs` names the
//! next thing the game needs (a starter, Oak's Parcel, the Pokédex, levels,
//! a badge) as a square to stand on; `nav.rs` routes there across maps;
//! `battle.rs` picks moves by expected damage and runs from wild battles it
//! has no use for. Text is answered `POKEMON_TEXT_MS` after it stops
//! printing, menus by reading their cursor. Held still for minutes, it
//! rewinds to a recent save state; held still across rewinds, it starts
//! the run over from the door.

mod battle;
mod input;
mod intro;
mod nav;
mod screen;
mod story;
#[cfg(test)]
mod tests;

use mizu_core::GameBoy;

use super::carts::Revision;
use super::kanto::Kanto;
use super::pilot::{A, B, START};
use battle::Battle;
use input::{Keys, GAP};
use intro::Intro;
use nav::Nav;
use story::{Starter, Story};

/// The WRAM addresses the bot and the world view read. Red and Blue share
/// one layout; Yellow's sits a byte lower from `wFontLoaded` up.
#[derive(Clone, Copy, Debug)]
pub struct Ram {
    pub font_loaded: u16,
    pub walk_counter: u16,
    pub in_battle: u16,
    pub cur_map: u16,
    pub y: u16,
    pub x: u16,
    pub tileset: u16,
    pub party_count: u16,
    pub sprite_data1: u16,
    pub sprite_data2: u16,
    pub player_name: u16,
    pub rival_name: u16,
    pub moving_direction: u16,
    pub options: u16,
    yellow: bool,
}

impl Ram {
    pub fn of(r: Revision) -> Self {
        let yellow = matches!(r, Revision::Yellow);
        let at = |red: u16| shift(yellow, red);
        Self {
            font_loaded: at(0xCFC4),
            walk_counter: at(0xCFC5),
            in_battle: at(0xD057),
            cur_map: at(0xD35E),
            y: at(0xD361),
            x: at(0xD362),
            tileset: at(0xD367),
            party_count: at(0xD163),
            sprite_data1: 0xC100,
            sprite_data2: 0xC200,
            player_name: at(0xD158),
            rival_name: at(0xD34A),
            moving_direction: at(0xD528),
            options: at(0xD355),
            yellow,
        }
    }

    /// A Red/Blue WRAM address in this revision.
    pub const fn at(self, red: u16) -> u16 {
        shift(self.yellow, red)
    }
}

const fn shift(yellow: bool, red: u16) -> u16 {
    if yellow && red >= 0xCFC4 {
        red - 1
    } else {
        red
    }
}

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
/// Rewinds in a row without a new square seen before starting over.
const REWINDS_TO_RESET: u32 = 4;
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
    snapshots: Vec<Vec<u8>>,
    rewinds: u32,
    stuck_rewinds: u32,
    last_goal: Option<story::Goal>,
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
        let mut nav = Nav::new(Kanto::new(rev), ram);
        // Yellow's Pikachu walks behind the player in the last sprite slot.
        if rev == Revision::Yellow {
            nav.skip_sprite = Some(15);
        }
        Self {
            rev,
            ram,
            rng,
            knobs,
            keys: Keys::new(knobs.text_ms),
            intro: Intro::new(rev),
            nav,
            story: Story::new(rev, starter),
            battle: Battle::new(ram),
            text_frames: 0,
            wander: 0,
            last_here: None,
            still: 0,
            snapshots: Vec::new(),
            rewinds: 0,
            stuck_rewinds: 0,
            last_goal: None,
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
            "map {:3} ({:2},{:2}) lv {} hp {}/{} badges {:08b} goal {:?} rewinds {} joyignore {:02x}",
            gb.peek(r.cur_map),
            gb.peek(r.x),
            gb.peek(r.y),
            lead.level,
            lead.hp,
            lead.max,
            self.story.badges(gb),
            self.story.goal(gb),
            self.rewinds,
            gb.peek(0xCD6B)
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
        if let Some(d) = self.nav.stepping(gb) {
            return d;
        }
        if let Some(b) = self.intro.buttons(gb, &mut self.keys, frame) {
            return b;
        }
        self.intro.watch(gb, frame);
        let r = self.ram;
        if gb.peek(r.in_battle) != 0 {
            self.nav.stop();
            self.text_frames = 0;
            let grind = self.story.grinding(gb);
            return self.battle.buttons(gb, &mut self.keys, grind);
        }
        if gb.peek(r.font_loaded) & 1 != 0 {
            self.nav.stop();
            return self.menus(gb, frame);
        }
        self.text_frames = 0;
        // Held in place with no text box the bot can see: some screen
        // (the Pokédex, a picture) is waiting on a button.
        if self.still > 60 * 3 && self.still % 120 < 40 {
            let key = if self.still % 240 < 120 { A } else { B };
            return self.keys.tap(key, 30);
        }
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
        if intro::naming_keyboard(gb) {
            return self.keys.tap(
                if (frame / 16).is_multiple_of(2) {
                    START
                } else {
                    A
                },
                8,
            );
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
            #[cfg(test)]
            if text {
                eprintln!(
                    "menus: backing out of {:?}\n{}",
                    screen::menu(gb),
                    screen::dump(gb)
                );
            }
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
    /// held still too long, it goes back to an older one and plays on with
    /// different luck. Rewinds that keep landing it in the same trap start
    /// the run over from the house door.
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
        let goal = self.story.goal(gb);
        if goal != self.last_goal {
            self.last_goal = goal;
            self.stuck_rewinds = 0;
        }
        if frame.is_multiple_of(SNAPSHOT_EVERY) && self.still < STILL_LIMIT / 4 {
            let mut state = Vec::new();
            if gb.save_state(&mut state).is_ok() {
                if self.snapshots.len() == SNAPSHOTS {
                    self.snapshots.remove(0);
                }
                self.snapshots.push(state);
            }
        }
        if self.still < STILL_LIMIT {
            return;
        }
        self.still = 0;
        self.stuck_rewinds += 1;
        if self.stuck_rewinds >= REWINDS_TO_RESET {
            self.stuck_rewinds = 0;
            self.snapshots.clear();
            self.intro = Intro::new(self.rev);
            self.intro.resume(gb);
        } else if let Some(state) = self.snapshots.first() {
            if gb.load_state(state.as_slice()).is_ok() {
                self.snapshots.truncate(1);
            }
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
