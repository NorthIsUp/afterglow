//! The hands on the Mac: boot, open Battle Chess from the Finder, hand both
//! sides to the Mac, and start a new game whenever one ends.
//!
//! Driven a sixtieth at a time from the screen alone. The only signals are
//! whether the menu bar is up and whether anything below it has moved, so
//! every wait is "quiet for long enough" and every action is checked by the
//! screen moving afterwards; anything that does not move in time is retried,
//! and in the end the Mac is reset. Times are emulated seconds: the boot runs
//! fast, the game at 1x.

use std::collections::VecDeque;

use super::proto::{Cmd, FRAME, W};

/// Mini vMac's speed while booting and launching (8x), and while playing.
const BOOT_SPEED: u8 = 3;
const PLAY_SPEED: u8 = 0;
const HZ: u32 = 60;
/// Battle Chess's Settings menu and its "Mac White" item. Mac Black is the
/// game's own default, so this one choice makes it computer against computer.
const SETTINGS: (u16, u16) = (178, 9);
const MAC_WHITE: (u16, u16) = (200, 75);
/// Where the pointer waits: the bottom-right corner, where all but a pixel of
/// the arrow is off the screen. The Plus draws its pointer into the screen,
/// so it must hold still while the board is being watched.
const PARK: (u16, u16) = (511, 341);
const RETRIES: u8 = 3;

const MKC_RETURN: u8 = 0x24;
const MKC_COMMAND: u8 = 0x37;
const MKC_N: u8 = 0x2D;
const MKC_O: u8 = 0x1F;
const MKC_SPACE: u8 = 0x31;
const MKC_LETTERS: [u8; 26] = [
    0x00, 0x0B, 0x08, 0x02, 0x0E, 0x03, 0x05, 0x04, 0x22, 0x26, 0x28, 0x25, 0x2E, 0x2D, 0x1F, 0x23,
    0x0C, 0x0F, 0x01, 0x11, 0x20, 0x09, 0x0D, 0x07, 0x10, 0x06,
];
const MKC_DIGITS: [u8; 10] = [0x1D, 0x12, 0x13, 0x14, 0x15, 0x17, 0x16, 0x1A, 0x1C, 0x19];

/// What a name types as for the Finder's type-to-select: letters, digits and
/// spaces, up to the first other character. A prefix selects as well.
pub fn keys_for(name: &str) -> Vec<u8> {
    name.chars()
        .map_while(|c| match c.to_ascii_lowercase() {
            c @ 'a'..='z' => Some(MKC_LETTERS[c as usize - 'a' as usize]),
            c @ '0'..='9' => Some(MKC_DIGITS[c as usize - '0' as usize]),
            ' ' => Some(MKC_SPACE),
            _ => None,
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for the Finder's desktop, then opening the game's disk.
    Boot,
    /// The disk's window is opening; then the game is opened from it.
    Finder,
    /// The game is loading.
    Launch,
    /// Both sides are being handed to the Mac.
    Arm,
    Play,
    /// A new game has been asked for.
    Restart,
}

/// One scripted input, held for `ticks` sixtieths.
#[derive(Clone, Copy)]
enum Act {
    Key(u8, bool),
    Mouse(u16, u16, bool),
}

pub struct Pilot {
    volume: Vec<u8>,
    phase: Phase,
    tries: u8,
    /// Games running that stalled within five minutes.
    short: u8,
    /// Emulated sixtieths: since the start, when the phase began, when the
    /// screen below the menu bar last changed, and when the script ran out.
    now: u64,
    since: u64,
    quiet_since: u64,
    acted: u64,
    board: u64,
    /// The Finder's menu titles, to tell when the game's have replaced them.
    finder_menu: u64,
    /// The board as it stood before an action that should move it.
    before: u64,
    speed: u8,
    mouse: (u16, u16, bool),
    script: VecDeque<(Act, u32)>,
    held: u32,
    reset: bool,
}

fn fnv(bytes: impl Iterator<Item = u8>) -> u64 {
    bytes.fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
    })
}

/// Everything below the menu bar, whose clock ticks every minute.
fn board_hash(screen: &[u8; FRAME]) -> u64 {
    fnv(screen[20 * W / 8..].iter().copied())
}

/// The menu titles: the left 360 pixels of the bar, short of the clock.
fn menu_hash(screen: &[u8; FRAME]) -> u64 {
    fnv((0..20).flat_map(|y| screen[y * W / 8..][..45].iter().copied()))
}

/// The Finder's and the game's menu bar end in a black line across row 19;
/// the boot screens have none.
fn menu_bar(screen: &[u8; FRAME]) -> bool {
    let row = &screen[19 * W / 8..20 * W / 8];
    row.iter().map(|b| b.count_ones()).sum::<u32>() >= 500
}

const fn secs(s: u64) -> u64 {
    s * HZ as u64
}

impl Pilot {
    /// `volume` is the game disk's name, which the Finder is typed.
    pub fn new(volume: &str) -> Self {
        Self {
            volume: keys_for(volume),
            phase: Phase::Boot,
            tries: 0,
            short: 0,
            now: 0,
            since: 0,
            quiet_since: 0,
            acted: 0,
            board: 0,
            finder_menu: 0,
            before: 0,
            speed: BOOT_SPEED,
            mouse: (PARK.0, PARK.1, false),
            script: VecDeque::new(),
            held: 0,
            reset: false,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The command for the next sixtieth, given the screen as it stands and
    /// whether it changed since the last call.
    pub fn step(&mut self, screen: &[u8; FRAME], changed: bool) -> Cmd {
        self.now += 1 << self.speed;
        if changed {
            let h = board_hash(screen);
            if h != self.board {
                self.board = h;
                self.quiet_since = self.now;
            }
        }
        let key = if self.script.is_empty() {
            self.decide(screen);
            None
        } else {
            self.act()
        };
        Cmd {
            reset: std::mem::take(&mut self.reset),
            speed: self.speed,
            h: self.mouse.0,
            v: self.mouse.1,
            button: self.mouse.2,
            key,
        }
    }

    fn act(&mut self) -> Option<(u8, bool)> {
        let (act, ticks) = self.script[0];
        let first = self.held == 0;
        self.held += 1;
        if self.held >= ticks {
            self.script.pop_front();
            self.held = 0;
            if self.script.is_empty() {
                self.acted = self.now;
            }
        }
        match act {
            Act::Key(k, down) => first.then_some((k, down)),
            Act::Mouse(h, v, b) => {
                self.mouse = (h, v, b);
                None
            }
        }
    }

    fn quiet(&self) -> u64 {
        self.now - self.quiet_since
    }

    /// Sixtieths since the last script ended, or the phase began.
    fn waited(&self) -> u64 {
        self.now - self.acted.max(self.since)
    }

    fn moved(&self) -> bool {
        self.board != self.before
    }

    fn enter(&mut self, phase: Phase) {
        if phase != self.phase {
            self.tries = 0;
        }
        self.phase = phase;
        self.since = self.now;
        self.before = self.board;
    }

    fn restart(&mut self) {
        self.reset = true;
        self.short = 0;
        self.quiet_since = self.now;
        self.speed = BOOT_SPEED;
        self.script.clear();
        self.held = 0;
        self.enter(Phase::Boot);
    }

    fn retry_or_reset(&mut self, phase: Phase, again: impl FnOnce(&mut Self)) {
        if self.tries >= RETRIES {
            self.restart();
        } else {
            self.tries += 1;
            again(self);
            self.since = self.now;
            self.phase = phase;
        }
    }

    fn decide(&mut self, screen: &[u8; FRAME]) {
        if self.quiet() >= secs(300) {
            return self.restart();
        }
        let (quiet, waited) = (self.quiet(), self.waited());
        match self.phase {
            Phase::Boot if self.now - self.since > secs(300) => self.restart(),
            Phase::Boot if menu_bar(screen) && quiet >= secs(25) => {
                self.finder_menu = menu_hash(screen);
                let keys = self.volume.clone();
                self.type_then_open(&keys);
                self.enter(Phase::Finder);
            }
            Phase::Finder if !self.moved() && waited > secs(30) => {
                self.retry_or_reset(Phase::Finder, |p| {
                    let keys = p.volume.clone();
                    p.type_then_open(&keys);
                });
            }
            Phase::Finder if self.moved() && quiet >= secs(3) => {
                self.type_then_open(&keys_for("battle chess"));
                self.enter(Phase::Launch);
            }
            Phase::Launch if waited > secs(90) => self.restart(),
            // The game's title holds still for a dozen seconds while it
            // loads, so the board is only taken as up after twenty.
            Phase::Launch
                if menu_bar(screen)
                    && menu_hash(screen) != self.finder_menu
                    && quiet >= secs(20) =>
            {
                self.speed = PLAY_SPEED;
                self.choose(MAC_WHITE);
                self.enter(Phase::Arm);
            }
            Phase::Arm | Phase::Restart if self.moved() && waited > secs(1) => {
                self.enter(Phase::Play);
            }
            Phase::Arm if waited > secs(20) => {
                self.retry_or_reset(Phase::Arm, |p| p.choose(MAC_WHITE));
            }
            Phase::Restart if waited > secs(30) => self.restart(),
            // Three games running that stall within five minutes are a Mac
            // that is not playing itself, whatever the screen moved for.
            Phase::Play if quiet >= secs(60) && self.short >= RETRIES => self.restart(),
            // A finished game waits on its "Check and mate" alert; a new one
            // asks "OK to start New Game?" while a game is on. Mac White
            // again, in case it never took.
            Phase::Play if quiet >= secs(60) => {
                self.short = if self.now - self.since < secs(300) {
                    self.short + 1
                } else {
                    0
                };
                self.press(&[MKC_RETURN], 2);
                self.pause(HZ);
                self.press(&[MKC_COMMAND, MKC_N], 2);
                self.pause(HZ);
                self.press(&[MKC_RETURN], 2);
                self.pause(HZ);
                self.choose(MAC_WHITE);
                self.enter(Phase::Restart);
            }
            _ => {}
        }
    }

    fn pause(&mut self, ticks: u32) {
        self.script
            .push_back((Act::Mouse(self.mouse.0, self.mouse.1, false), ticks));
    }

    /// Keys down in order, then up in reverse: a chord, or one key.
    fn press(&mut self, keys: &[u8], ticks: u32) {
        for &k in keys {
            self.script.push_back((Act::Key(k, true), ticks));
        }
        for &k in keys.iter().rev() {
            self.script.push_back((Act::Key(k, false), ticks));
        }
    }

    /// Type-select a name in the frontmost Finder window, then Open.
    fn type_then_open(&mut self, keys: &[u8]) {
        for &k in keys {
            self.press(&[k], 2);
        }
        self.pause(30);
        self.press(&[MKC_COMMAND, MKC_O], 2);
    }

    /// Drag down Settings to `item` and let go, then park the pointer again.
    fn choose(&mut self, item: (u16, u16)) {
        let (sh, sv) = SETTINGS;
        let s = &mut self.script;
        s.push_back((Act::Mouse(sh, sv, false), 10));
        s.push_back((Act::Mouse(sh, sv, true), 20));
        s.push_back((Act::Mouse(sh + 12, sv + 30, true), 10));
        s.push_back((Act::Mouse(item.0, item.1, true), 30));
        s.push_back((Act::Mouse(item.0, item.1, false), 10));
        s.push_back((Act::Mouse(PARK.0, PARK.1, false), 10));
    }
}

#[cfg(test)]
mod tests {
    use super::super::proto::blank;
    use super::*;

    fn screen(menu: bool, board: u8) -> Box<[u8; FRAME]> {
        let mut s = blank();
        s.fill(board);
        s[19 * W / 8..20 * W / 8].fill(if menu { 0xFF } else { 0x55 });
        s
    }

    /// Steps until `done` or `max` sixtieths, collecting the keys sent.
    fn run(
        p: &mut Pilot,
        s: &[u8; FRAME],
        max: u32,
        done: impl Fn(&Pilot) -> bool,
    ) -> (Vec<(u8, bool)>, bool) {
        let mut keys = vec![];
        let mut reset = false;
        for _ in 0..max {
            let c = p.step(s, false);
            keys.extend(c.key);
            reset |= c.reset;
            if done(p) {
                break;
            }
        }
        (keys, reset)
    }

    #[test]
    fn names_type_as_their_keys_up_to_the_first_odd_character() {
        assert_eq!(keys_for("BattleChess"), keys_for("battlechess"));
        assert_eq!(keys_for("Bat 1"), vec![0x0B, 0x00, 0x11, MKC_SPACE, 0x12]);
        assert_eq!(keys_for("System7_5_3"), keys_for("system7"));
    }

    /// No keys before the menu bar is up and quiet; then the disk's name and
    /// Command-O, at boot speed.
    #[test]
    fn the_desktop_gets_the_disk_name_then_open() {
        let mut p = Pilot::new("Bat");
        let boot = screen(false, 1);
        p.step(&boot, true);
        let (keys, _) = run(&mut p, &boot, 600, |_| false);
        assert_eq!(keys, vec![]);
        let desk = screen(true, 2);
        p.step(&desk, true);
        let (keys, reset) = run(&mut p, &desk, 2000, |p| {
            p.phase == Phase::Finder && p.script.is_empty()
        });
        assert!(!reset);
        let downs: Vec<u8> = keys.iter().filter(|k| k.1).map(|k| k.0).collect();
        assert_eq!(downs, vec![0x0B, 0x00, 0x11, MKC_COMMAND, MKC_O]);
        assert_eq!(p.speed, BOOT_SPEED);
    }

    /// The window opens (the screen moves), and the game is typed and opened.
    #[test]
    fn an_opened_window_gets_the_game_name() {
        let mut p = Pilot::new("Bat");
        p.enter(Phase::Finder);
        p.step(&screen(true, 3), true);
        let (keys, _) = run(&mut p, &screen(true, 3), 2000, |p| {
            p.phase == Phase::Launch && p.script.is_empty()
        });
        let downs: Vec<u8> = keys.iter().filter(|k| k.1).map(|k| k.0).collect();
        let mut want = keys_for("battle chess");
        want.extend([MKC_COMMAND, MKC_O]);
        assert_eq!(downs, want);
    }

    /// A window that never opens is typed for again, then the Mac is reset.
    #[test]
    fn a_window_that_never_opens_is_retried_then_reset() {
        let mut p = Pilot::new("Bat");
        p.enter(Phase::Finder);
        let (_, reset) = run(&mut p, &screen(true, 0), 40_000, |p| p.phase == Phase::Boot);
        assert!(reset);
        assert_eq!(p.phase, Phase::Boot);
    }

    /// The game's menus up and quiet: 1x, then the drag to Mac White; a
    /// board that then moves is a game being played.
    #[test]
    fn a_loaded_game_is_handed_to_the_mac() {
        let mut p = Pilot::new("Bat");
        p.finder_menu = menu_hash(&screen(true, 3));
        p.enter(Phase::Launch);
        // Still the Finder's menus: no matter how quiet, nothing happens.
        p.step(&screen(true, 3), true);
        run(&mut p, &screen(true, 3), 200, |_| false);
        assert_eq!(p.phase, Phase::Launch);
        p.step(&screen(true, 4), true);
        run(&mut p, &screen(true, 4), 4000, |p| p.phase == Phase::Arm);
        assert_eq!((p.phase, p.speed), (Phase::Arm, PLAY_SPEED));
        let mut downs = 0;
        while !p.script.is_empty() {
            let c = p.step(&screen(true, 4), false);
            if c.button && (c.h, c.v) == MAC_WHITE {
                downs += 1;
            }
        }
        assert!(downs > 0, "never dragged to Mac White");
        assert_eq!((p.mouse.0, p.mouse.1), PARK);
        p.step(&screen(true, 5), true);
        run(&mut p, &screen(true, 5), 200, |p| p.phase == Phase::Play);
        assert_eq!(p.phase, Phase::Play);
    }

    /// A finished game sits still: Return for its alert, then Command-N.
    #[test]
    fn a_still_board_gets_return_then_a_new_game() {
        let mut p = Pilot::new("Bat");
        p.speed = PLAY_SPEED;
        p.enter(Phase::Play);
        let s = screen(true, 6);
        p.step(&s, true);
        let (keys, reset) = run(&mut p, &s, 5000, |p| {
            p.phase == Phase::Restart && p.script.is_empty()
        });
        assert!(!reset);
        let downs: Vec<u8> = keys.iter().filter(|k| k.1).map(|k| k.0).collect();
        assert_eq!(downs, vec![MKC_RETURN, MKC_COMMAND, MKC_N, MKC_RETURN]);
        assert_eq!((p.mouse.0, p.mouse.1), PARK, "Mac White again, then parked");
        // ...and a new game that never starts is a reset.
        let (_, reset) = run(&mut p, &s, 5000, |p| p.phase == Phase::Boot);
        assert!(reset);
    }

    /// A reset is one press, at boot speed, and the next one waits for the
    /// rebooted Mac to sit still for five minutes all over again.
    #[test]
    fn a_reset_is_sent_once_and_boots_fast() {
        let mut p = Pilot::new("Bat");
        p.speed = PLAY_SPEED;
        p.enter(Phase::Play);
        p.quiet_since = 0;
        p.now = secs(300);
        assert!(p.step(&screen(false, 7), false).reset);
        assert_eq!((p.phase, p.speed), (Phase::Boot, BOOT_SPEED));
        let (_, again) = run(&mut p, &screen(false, 7), 600, |_| false);
        assert!(!again);
    }

    /// A game that moves after every new game but stalls again within five
    /// minutes, three times, is a Mac not playing itself: reset it.
    #[test]
    fn games_that_keep_stalling_reset_the_mac() {
        let mut p = Pilot::new("Bat");
        p.speed = PLAY_SPEED;
        p.enter(Phase::Play);
        let mut board = 10;
        let mut resets = 0;
        for _ in 0..4 {
            p.step(&screen(true, board), true);
            let (_, reset) = run(&mut p, &screen(true, board), 10_000, |p| {
                p.reset || p.phase == Phase::Restart && p.script.is_empty()
            });
            resets += usize::from(reset);
            board += 1;
            p.step(&screen(true, board), true);
            run(&mut p, &screen(true, board), 200, |p| {
                p.phase == Phase::Play
            });
        }
        assert_eq!(resets, 1);
    }
}
