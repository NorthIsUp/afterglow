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
/// The bits of `wOptions` the bot sets; Yellow keeps its sound setting in
/// the others.
const OPTION_BITS: u8 = 0xCF;
pub const PALLET: u8 = 0x00;
pub const BEDROOM: u8 = 0x26;
pub const DOWNSTAIRS: u8 = 0x25;
/// Where the house door puts the player in Pallet Town.
pub const DOOR: (u8, u8) = (5, 6);
/// Frames after which the intro is stuck somewhere: several times what it
/// takes.
const GIVE_UP: u64 = 60 * 60 * 8;
/// A, B, Select and Start together: the game's own soft reset.
const SOFT_RESET: u8 = A | B | 0x40 | START;

/// In the order a fresh game passes them; the discriminant indexes `passed`.
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
const _: () = {
    let mut i = 0;
    while i < STEPS.len() {
        assert!(STEPS[i] as usize == i, "STEPS out of declaration order");
        i += 1;
    }
};

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
    /// Frames in Pallet Town away from the door.
    away: u32,
    /// The frame past which the intro counts as stuck.
    deadline: u64,
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
            away: 0,
            deadline: GIVE_UP,
            done: false,
        }
    }

    /// Drop the cached door state, so the next intro runs from power-on.
    #[cfg(test)]
    pub fn forget() {
        AT_DOOR
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
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
        let i = step as usize;
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

    fn has(&self, step: Step) -> bool {
        self.passed[step as usize].is_some()
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
        if self.done || gb.peek(r.font_loaded) & 1 != 0 || screen::has_text(gb) {
            return;
        }
        let here = (gb.peek(r.x), gb.peek(r.y));
        match gb.peek(r.cur_map) {
            BEDROOM => self.pass(gb, Step::Bedroom, frame),
            DOWNSTAIRS => self.pass(gb, Step::Downstairs, frame),
            PALLET if self.next() == Some(Step::Outside) => {
                // The map changes a moment before the coordinates do.
                if here != DOOR {
                    self.away += 1;
                    if self.away < 120 {
                        return;
                    }
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
        if self.has(Step::Bedroom) || self.done {
            return None;
        }
        // A game already under way (a loaded state): nothing to do here.
        if gb.peek(r.party_count) != 0 {
            self.done = true;
            return None;
        }
        // Stuck before the house: the game's own soft reset, and the steps
        // checked again from the title. In the house, the bot's ordinary
        // walking takes over.
        if frame > self.deadline {
            self.fail(
                gb,
                self.next().unwrap_or(Step::Outside),
                "intro ran too long",
            );
            if !self.has(Step::Bedroom) {
                self.passed = [None; STEPS.len()];
                self.deadline = frame + GIVE_UP;
                return Some(keys.tap(SOFT_RESET, GAP));
            }
            self.done = true;
            return None;
        }
        // Text is checked every frame, so its delay counts real stillness.
        let text = keys.text_ready(gb);
        if self.has(Step::RivalName)
            && gb.peek(r.cur_map) == BEDROOM
            && gb.peek(r.font_loaded) & 1 == 0
            && !screen::has_text(gb)
        {
            self.watch(gb, frame);
            return None;
        }
        let m = screen::menu(gb);
        let cursor = screen::cursor(gb);
        if cursor.is_some() && screen::shows(gb, b"NEW GAME") {
            let first = u8::from(screen::shows(gb, b"CONTINUE"));
            let opts = gb.peek(r.options) & OPTION_BITS;
            let want = if opts == OPTIONS { first } else { first + 1 };
            if m.item == first && opts == OPTIONS {
                self.pass(gb, Step::TitleMenu, frame);
            }
            return Some(keys.tap(screen::toward(m.item, want), GAP));
        }
        if let Some(at) = cursor.filter(|_| screen::shows(gb, b"TEXT SPEED")) {
            return Some(keys.tap(options_key(gb, at), GAP));
        }
        if gb.peek(r.options) & OPTION_BITS == OPTIONS && self.has(Step::TitleMenu) {
            self.pass(gb, Step::Options, frame);
        }
        if let Some((_, y)) = cursor.filter(|_| screen::shows(gb, b"NEW NAME")) {
            let rival = self.has(Step::PlayerName);
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
                return Some(keys.tap(screen::toward(m.item, 1), GAP));
            }
            self.picked = preset(&screen::row(gb, y));
            return Some(keys.tap(A, GAP));
        }
        // The keyboard: the list was not answered with a preset.
        if let Some(key) = naming_key(gb, frame) {
            self.fail(
                gb,
                self.next().unwrap_or(Step::Bedroom),
                "naming keyboard opened",
            );
            return Some(keys.tap(key, 8));
        }
        if self.picked[0] != 0 {
            let (step, at) = if self.has(Step::PlayerName) {
                (Step::RivalName, r.rival_name)
            } else {
                (Step::PlayerName, r.player_name)
            };
            let got = screen::name(gb, at);
            if got == self.picked {
                self.picked = [0; 11];
                self.pass(gb, step, frame);
            } else if !screen::shows(gb, b"NEW NAME")
                && got[0] != 0
                && got[0] != b'.'
                // The title screen's placeholders, until the pick lands.
                && !got.starts_with(b"NINTEN")
                && !got.starts_with(b"SONY")
            {
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
/// screen says what a row is set to.
fn options_key(gb: &mut GameBoy, (x, y): (usize, usize)) -> u8 {
    const ROWS: [(&[u8], &[u8], u8); 3] = [
        (b"TEXT SPEED", b"FAST", LEFT),
        (b"ANIMATION", b"OFF", RIGHT),
        (b"STYLE", b"SET", RIGHT),
    ];
    let row = screen::row(gb, y);
    let has = |t: &[u8]| row.windows(t.len()).any(|w| w == t);
    // Yellow: each setting's value is on its own row, and SOUND and
    // PRINT stay as they are on the way down to CANCEL.
    if screen::shows(gb, b"SOUND") {
        for (title, want, toward) in ROWS {
            if has(title) {
                return if has(want) { DOWN } else { toward };
            }
        }
        return if has(b"CANCEL") { A } else { DOWN };
    }
    // Red and Blue: the values sit under the title, the cursor on one.
    for (title, want, toward) in ROWS {
        if screen::find(gb, title).is_some_and(|(_, ty)| ty + 2 == y) {
            let at = row.windows(want.len()).position(|w| w == want);
            return if at.is_some_and(|a| a == x + 1) {
                DOWN
            } else {
                toward
            };
        }
    }
    // CANCEL, or a row this revision adds below the three: B leaves.
    if row[x + 1..].starts_with(b"CANCEL") {
        A
    } else {
        B
    }
}

/// On the naming keyboard (its last line reads `UPPER CASE` or `lower
/// case`), START and A by turns: START jumps to END, A takes whatever is
/// typed, the default if nothing.
pub fn naming_key(gb: &mut GameBoy, frame: u64) -> Option<u8> {
    let up = screen::shows(gb, b"UPPER CASE") || screen::shows(gb, b"lower case");
    up.then_some(if (frame / 16).is_multiple_of(2) {
        START
    } else {
        A
    })
}
