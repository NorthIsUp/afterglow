//! Power-on to the first step out of the player's house, as fast as the
//! core runs. Every screen is handled by what it shows (the main menu, the
//! options, a preset-name list, text), and every step is checked in RAM in
//! the order a fresh game must pass them; a step out of order is logged and
//! the screen-driven handling carries on from there.
//!
//! The state outside the door is cached per revision, so the next run of
//! the same game starts there.

use std::sync::Mutex;

use mizu_core::GameBoy;

use super::super::carts::Revision;
use super::super::pilot::{A, B, DOWN, LEFT, RIGHT, START};
use super::input::{Keys, GAP};
use super::screen;
use super::Ram;

/// `wOptions`: fast text, battle animations off, battle style SET.
pub const OPTIONS: u8 = 0xC1;
pub const PALLET: u8 = 0x00;
pub const BEDROOM: u8 = 0x26;
pub const DOWNSTAIRS: u8 = 0x25;
/// Where the house door puts the player in Pallet Town.
pub const DOOR: (u8, u8) = (5, 6);
/// Frames after which the intro hands over to the ordinary bot: several
/// times what it takes.
const GIVE_UP: u64 = 60 * 60 * 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// The title menu is up with its cursor on NEW GAME.
    TitleMenu,
    /// The options read fast text, no animations, SET.
    Options,
    /// Oak's speech ran to the first preset-name list.
    PlayerList,
    /// `wPlayerName` is the preset picked.
    PlayerName,
    RivalList,
    RivalName,
    Bedroom,
    Downstairs,
    /// In Pallet Town at the house door.
    Outside,
}

pub const STEPS: [Step; 9] = [
    Step::TitleMenu,
    Step::Options,
    Step::PlayerList,
    Step::PlayerName,
    Step::RivalList,
    Step::RivalName,
    Step::Bedroom,
    Step::Downstairs,
    Step::Outside,
];

/// The state at the door, per revision, for the next run.
static AT_DOOR: Mutex<Vec<(Revision, Vec<u8>)>> = Mutex::new(Vec::new());

pub struct Intro {
    rev: Revision,
    ram: Ram,
    /// The frame each step passed at, in `STEPS` order.
    pub passed: [Option<u64>; STEPS.len()],
    /// The first step found out of order, and the RAM it found.
    pub failed: Option<(Step, String)>,
    /// The preset name the open list was answered with.
    picked: [u8; 11],
    pub done: bool,
}

impl Intro {
    pub fn new(rev: Revision) -> Self {
        Self {
            rev,
            ram: Ram::of(rev),
            passed: [None; STEPS.len()],
            failed: None,
            picked: [0; 11],
            done: false,
        }
    }

    /// Start from the cached door state, if an earlier run left one.
    pub fn resume(&mut self, gb: &mut GameBoy) -> bool {
        let cache = AT_DOOR
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some((_, state)) = cache.iter().find(|(r, _)| *r == self.rev) else {
            return false;
        };
        if gb.load_state(state.as_slice()).is_err() {
            return false;
        }
        self.done = true;
        true
    }

    fn next(&self) -> Option<Step> {
        self.passed
            .iter()
            .position(Option::is_none)
            .map(|i| STEPS[i])
    }

    /// Mark `step` passed if it is the next one due; out of order, log it
    /// once and record the first failure.
    fn pass(&mut self, gb: &mut GameBoy, step: Step, frame: u64) {
        let i = STEPS.iter().position(|&s| s == step).expect("a step");
        if self.passed[i].is_some() {
            return;
        }
        if self.next() != Some(step) && self.failed.is_none() {
            let r = self.ram;
            let why = format!(
                "expected {:?}, saw {step:?}: map {} ({},{}) menu {:?} options {:02x}",
                self.next(),
                gb.peek(r.cur_map),
                gb.peek(r.x),
                gb.peek(r.y),
                screen::menu(gb),
                gb.peek(r.options),
            );
            eprintln!("[screensaver] gameboy: intro step out of order: {why}");
            self.failed = Some((step, why));
        }
        self.passed[i] = Some(frame);
    }

    fn fail(&mut self, gb: &mut GameBoy, step: Step, what: &str) {
        if self.failed.is_none() {
            let why = format!("{what}; menu {:?}", screen::menu(gb));
            eprintln!("[screensaver] gameboy: intro step {step:?} failed: {why}");
            self.failed = Some((step, why));
        }
    }

    /// Checks the maps the house walk passes through; the walk itself is
    /// the bot's ordinary navigation.
    pub fn watch(&mut self, gb: &mut GameBoy, frame: u64) {
        let r = self.ram;
        if self.done || gb.peek(r.font_loaded) & 1 != 0 {
            return;
        }
        let here = (gb.peek(r.x), gb.peek(r.y));
        match gb.peek(r.cur_map) {
            BEDROOM => self.pass(gb, Step::Bedroom, frame),
            DOWNSTAIRS => self.pass(gb, Step::Downstairs, frame),
            PALLET if self.next() == Some(Step::Outside) => {
                if here != DOOR {
                    self.fail(gb, Step::Outside, &format!("outside at {here:?}"));
                }
                self.pass(gb, Step::Outside, frame);
                self.done = true;
                let mut state = Vec::new();
                if gb.save_state(&mut state).is_ok() {
                    let mut cache = AT_DOOR
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    cache.retain(|(rv, _)| *rv != self.rev);
                    cache.push((self.rev, state));
                }
            }
            _ => {}
        }
    }

    /// The menus and text before the player has a body: `None` once the
    /// game is in the bedroom.
    pub fn buttons(&mut self, gb: &mut GameBoy, keys: &mut Keys, frame: u64) -> Option<u8> {
        let r = self.ram;
        if self.passed[6].is_some() || self.done {
            return None;
        }
        // A game already under way (a loaded state): nothing to do here.
        if gb.peek(r.party_count) != 0 {
            self.done = true;
            return None;
        }
        // Whatever went wrong, the house is the bot's ordinary walking.
        if frame > GIVE_UP {
            self.fail(
                gb,
                self.next().unwrap_or(Step::Outside),
                "intro ran too long",
            );
            self.done = true;
            return None;
        }
        // Text is checked every frame, so its delay counts real stillness.
        let text = keys.text_ready(gb);
        if self.passed[5].is_some()
            && gb.peek(r.cur_map) == BEDROOM
            && gb.peek(r.font_loaded) & 1 == 0
        {
            self.watch(gb, frame);
            return None;
        }
        let m = screen::menu(gb);
        let cursor = screen::cursor(gb);
        if cursor.is_some() && screen::shows(gb, b"NEW GAME") {
            let first = u8::from(screen::shows(gb, b"CONTINUE"));
            let opts = gb.peek(r.options);
            let want = if opts == OPTIONS { first } else { first + 1 };
            if m.item == first && opts == OPTIONS {
                self.pass(gb, Step::TitleMenu, frame);
            }
            return Some(keys.tap(screen::toward(m, want), GAP));
        }
        if let Some(at) = cursor.filter(|_| screen::shows(gb, b"TEXT SPEED")) {
            return Some(keys.tap(options_key(gb, at), GAP));
        }
        if gb.peek(r.options) == OPTIONS && self.passed[0].is_some() {
            self.pass(gb, Step::Options, frame);
        }
        if let Some((_, y)) = cursor.filter(|_| screen::shows(gb, b"NEW NAME")) {
            let rival = self.passed[3].is_some();
            self.pass(
                gb,
                if rival {
                    Step::RivalList
                } else {
                    Step::PlayerList
                },
                frame,
            );
            if m.item != 1 {
                return Some(keys.tap(screen::toward(m, 1), GAP));
            }
            self.picked = preset(&screen::row(gb, y));
            return Some(keys.tap(A, GAP));
        }
        // The keyboard: the list was not answered with a preset. START
        // jumps to END and A takes whatever is typed, the default if
        // nothing.
        if naming_keyboard(gb) {
            self.fail(
                gb,
                self.next().unwrap_or(Step::Bedroom),
                "naming keyboard opened",
            );
            return Some(keys.tap(
                if (frame / 16).is_multiple_of(2) {
                    START
                } else {
                    A
                },
                8,
            ));
        }
        if self.picked[0] != 0 {
            let (step, at) = if self.passed[3].is_some() {
                (Step::RivalName, r.rival_name)
            } else {
                (Step::PlayerName, r.player_name)
            };
            let got = screen::name(gb, at);
            if got == self.picked {
                self.picked = [0; 11];
                self.pass(gb, step, frame);
            } else if !screen::shows(gb, b"NEW NAME") && got[0] != 0 && got[0] != b'.' {
                let (g, p) = (
                    String::from_utf8_lossy(&got),
                    String::from_utf8_lossy(&self.picked),
                );
                self.fail(gb, step, &format!("name {g:?}, picked {p:?}"));
                self.picked = [0; 11];
                self.pass(gb, step, frame);
            }
        }
        // Everything else is text, the intro movie and the title screen:
        // A once it holds still.
        Some(if text { keys.tap(A, GAP) } else { 0 })
    }
}

/// The preset on the cursor's row: the letters after it, padded the way
/// `screen::name` pads.
fn preset(row: &[u8; screen::COLS]) -> [u8; 11] {
    let mut out = [0; 11];
    let start = row.iter().position(|&c| c == b'>').map_or(0, |i| i + 1);
    for (o, &c) in out.iter_mut().zip(
        row[start..]
            .iter()
            .take_while(|c| c.is_ascii_alphanumeric()),
    ) {
        *o = c;
    }
    out
}

/// The options screen: each row toward fast text, animations off and SET,
/// then CANCEL. The game writes `wOptions` only on the way out, so the
/// cursor's column says what a row is set to.
fn options_key(gb: &mut GameBoy, (x, y): (usize, usize)) -> u8 {
    for (title, want, toward) in [
        (&b"TEXT SPEED"[..], &b"FAST"[..], LEFT),
        (b"ANIMATION", b"OFF", RIGHT),
        (b"STYLE", b"SET", RIGHT),
    ] {
        if screen::find(gb, title).is_some_and(|(_, ty)| ty + 2 == y) {
            let at = screen::row(gb, y)
                .windows(want.len())
                .position(|w| w == want);
            return if at.is_some_and(|a| a == x + 1) {
                DOWN
            } else {
                toward
            };
        }
    }
    // CANCEL, or a row this revision adds below the three: B leaves.
    if screen::row(gb, y)[x + 1..].starts_with(b"CANCEL") {
        A
    } else {
        B
    }
}

/// The naming keyboard is up: its last line reads `UPPER CASE` or `lower
/// case`.
pub fn naming_keyboard(gb: &mut GameBoy) -> bool {
    screen::shows(gb, b"UPPER CASE") || screen::shows(gb, b"lower case")
}
