//! The autopilot: buttons from a seeded roll and what the screen is doing,
//! never from a game's RAM except to tell a Pokémon intro from its overworld.
//!
//! `Boot` taps A, with START every few taps: through title screens, menus
//! and, in Pokémon, the professor's speech and the naming screen (START jumps
//! to OK there). `Explore` then walks: a direction held a while, A to talk
//! or pick, B to back out of whatever A opened. START comes back only when
//! the screen has sat still a long time (a pause, a game-over card), and
//! never in Pokémon, whose START is a menu B already closes.

use super::agb::keys::{A, B, DOWN, LEFT, RIGHT, START, UP};
use super::carts::Pokemon;
use crate::next_rand;

/// How long a generic game gets to reach play before the walk starts:
/// 90 s at 59.73 Hz.
const BOOT_FRAMES: u64 = 5400;
/// A Pokémon intro that has not reached the overworld by now (ten
/// minutes) never will under `Boot`; walk instead.
const POKEMON_BOOT_CAP: u64 = 36_000;
/// A tap is held this long, one every `TAP_EVERY` frames in `Boot`.
const TAP_HOLD: u32 = 4;
const TAP_EVERY: u64 = 16;
/// Frames of an unchanged screen before `Explore` presses START: ~10 s.
const STILL: u32 = 600;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Boot,
    Explore,
}

pub struct Pilot {
    pokemon: Option<Pokemon>,
    rng: u32,
    frame: u64,
    phase: Phase,
    /// The direction held and the frames left of it.
    dir: u32,
    dir_left: u32,
    /// Frames left of the button being tapped, and which.
    tap: u32,
    tap_keys: u32,
    still: u32,
    last: u64,
}

impl Pilot {
    pub fn new(pokemon: Option<Pokemon>, seed: u32) -> Self {
        Self {
            pokemon,
            rng: seed | 1,
            frame: 0,
            phase: Phase::Boot,
            dir: 0,
            dir_left: 0,
            tap: 0,
            tap_keys: 0,
            still: 0,
            last: 0,
        }
    }

    #[cfg(test)]
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The game was loaded from a save state, past its title screens.
    pub fn resumed(&mut self) {
        self.phase = Phase::Explore;
    }

    fn roll(&mut self, n: u32) -> u32 {
        next_rand(&mut self.rng) % n
    }

    /// The keys for the next frame, given the last frame's screen and, for a
    /// Pokémon game, whether its RAM says the player is in the overworld.
    pub fn keys(&mut self, screen: &[u32], overworld: bool) -> u32 {
        self.frame += 1;
        let h = fingerprint(screen);
        self.still = if h == self.last { self.still + 1 } else { 0 };
        self.last = h;
        let done = match self.pokemon {
            Some(_) => overworld || self.frame >= POKEMON_BOOT_CAP,
            None => self.frame >= BOOT_FRAMES,
        };
        if self.phase == Phase::Boot && done {
            self.phase = Phase::Explore;
        }
        match self.phase {
            Phase::Boot => self.boot(),
            Phase::Explore => self.explore(),
        }
    }

    fn boot(&mut self) -> u32 {
        if self.frame.is_multiple_of(TAP_EVERY) {
            let start = self.frame / TAP_EVERY % 4 == 3;
            self.tap(if start { START } else { A });
        }
        self.held()
    }

    fn explore(&mut self) -> u32 {
        if self.dir_left == 0 {
            const DIRS: [u32; 5] = [UP, DOWN, LEFT, RIGHT, 0];
            self.dir = DIRS[self.roll(5) as usize];
            self.dir_left = 15 + self.roll(75);
        }
        self.dir_left -= 1;
        if self.tap == 0 {
            if self.still >= STILL && self.pokemon.is_none() {
                self.still = 0;
                self.tap(START);
            } else {
                match self.roll(48) {
                    0 => self.tap(A),
                    1 | 2 => self.tap(B),
                    _ => {}
                }
            }
        }
        self.dir | self.held()
    }

    fn tap(&mut self, k: u32) {
        (self.tap, self.tap_keys) = (TAP_HOLD * 2, k);
    }

    /// A tap is held for its first half and released for the second, so two
    /// in a row read as two presses.
    fn held(&mut self) -> u32 {
        if self.tap == 0 {
            return 0;
        }
        self.tap -= 1;
        if self.tap >= TAP_HOLD {
            self.tap_keys
        } else {
            0
        }
    }
}

/// Every 61st pixel, folded: enough to tell a still screen from a moving one.
fn fingerprint(screen: &[u32]) -> u64 {
    screen
        .iter()
        .step_by(61)
        .fold(0xcbf2_9ce4_8422_2325, |h, &p| {
            (h ^ u64::from(p)).wrapping_mul(0x100_0000_01b3)
        })
}
