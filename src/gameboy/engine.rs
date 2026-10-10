//! The emulator and the one thread that runs it.
//!
//! The thread starts on first use, runs the Game Boy at its own 59.73 Hz and
//! parks whenever no `gameboy` saver is showing, keeping the cartridge where
//! it was. The render thread only ever `try_lock`s the last finished view, so
//! a slow frame here is a stale frame there, never a late one.
//!
//! A panic anywhere in a frame (the core, a pilot, a view) is caught at the
//! frame boundary: that cartridge is dropped and the next one boots.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use mizu_core::{GameBoy, GameBoyConfig};

use super::carts::{self, Cart, Pilot, DMG_GREYS};
use super::pilot::Driver;
use super::pokemon::Knobs;
use super::view::{Composer, Mode};
use crate::engine_slot::{self, pace, Slot, View};

pub const H: usize = 144;
pub const SCREEN_W: usize = 160;
/// The widest view: 5.3:1 glass at a square Game Boy pixel.
pub const MAX_W: usize = 768;
/// 70224 dots at 4.194304 MHz.
const FRAME: Duration = Duration::from_nanos(16_742_706);

/// What the showing saver wants. Written by the render thread on a switch,
/// read by the engine thread once a frame.
#[derive(Clone, PartialEq)]
pub struct Want {
    pub rom: String,
    pub sav: String,
    pub palette: String,
    /// View width in Game Boy pixels; the height is always [`H`].
    pub width: usize,
    pub mode: Mode,
    pub rotate: Option<Duration>,
    pub seed: u32,
    /// `saver::restarts("gameboy")` when built: a new value is a power-on reset.
    pub restart: u64,
    pub pokemon: Knobs,
}

impl Want {
    /// Same cartridges: a change in anything else keeps the game running.
    fn same_carts(&self, o: &Self) -> bool {
        (&self.rom, &self.sav, self.restart) == (&o.rom, &o.sav, o.restart)
    }
}

/// The latest finished view. A blank one has nothing to show (no cartridge
/// would boot) and the saver draws static.
pub struct Engine(Slot<Want, View<u16, ()>>);

impl Engine {
    pub fn get() -> &'static Self {
        static E: OnceLock<Engine> = OnceLock::new();
        E.get_or_init(|| {
            std::thread::Builder::new()
                .name("gameboy".into())
                .spawn(|| run(Engine::get()))
                .expect("spawning the gameboy thread");
            Engine(Slot::new(View::new(MAX_W, H), ()))
        })
    }

    /// Copy the view into `dst` if it moved on since `seen`, never waiting.
    /// Returns the new sequence number and the view's width, `None` for
    /// nothing to show.
    pub fn latest(&self, seen: u32, dst: &mut [u16]) -> Option<(u32, Option<usize>)> {
        let (seq, shown) = self.0.peek()?.read(seen, dst)?;
        Some((seq, shown.map(|(w, ())| w)))
    }
}

impl engine_slot::Engine for Engine {
    type Want = Want;

    fn claim(&self, id: u64, want: Want) {
        self.0.claim(id, want);
    }

    fn release(&self, id: u64) {
        self.0.release(id);
    }
}

/// The 32768-entry colour map a cartridge's frames go through: a monochrome
/// game's four greys become `shades`; a colour game gets the usual
/// GBC-panel correction, without which its palettes look garish on a modern
/// screen.
pub fn tone_map(color: bool, shades: carts::Shades) -> Box<[u16]> {
    (0..0x8000u32)
        .map(|c| {
            let c = c as u16;
            if !color {
                return DMG_GREYS
                    .iter()
                    .position(|&g| g == c)
                    .map_or(c, |i| shades[i]);
            }
            let (r, g, b) = (
                u32::from(c & 31),
                u32::from(c >> 5 & 31),
                u32::from(c >> 10 & 31),
            );
            let rr = ((r * 26 + g * 4 + b * 2).min(960) >> 5) as u16;
            let gg = ((g * 24 + b * 8).min(960) >> 5) as u16;
            let bb = ((r * 6 + g * 4 + b * 22).min(960) >> 5) as u16;
            rr | gg << 5 | bb << 10
        })
        .collect()
}

/// One cartridge, booted.
pub struct Session {
    pub gb: GameBoy,
    pub name: String,
    driver: Driver,
    tone: Box<[u16]>,
    color: bool,
    pub composer: Composer,
    pub frames: u64,
}

impl Session {
    pub fn boot(cart: &Cart, want: &Want, seed: u32) -> Result<Self, String> {
        // A monochrome game runs as one on a DMG: the core's GBC mode would
        // colour it with a palette of its own choosing.
        let color = cart.rom.get(0x143).is_some_and(|b| b & 0x80 != 0);
        let config = GameBoyConfig { is_dmg: !color };
        // The Pokémon bot plays a new game from power-on: a battery save
        // would put it somewhere its intro does not know.
        let sram = match cart.pilot {
            Pilot::Pokemon(_) => None,
            _ => cart.sram.as_deref(),
        };
        // The core asserts on headers and battery saves it cannot map: a
        // user's ROM or GAMEBOY_SAV, so a panic here is a cartridge that
        // will not boot, not a dead engine thread.
        let gb = catch_unwind(|| GameBoy::from_rom(cart.rom.clone(), sram, config))
            .map_err(|_| "the core panicked loading it".to_string())?
            .map_err(|e| e.to_string())?;
        Ok(Self {
            gb,
            name: cart.name.clone(),
            driver: Driver::new(cart.pilot, seed, want.pokemon),
            tone: tone_map(color, carts::shades(&want.palette, cart.pilot)),
            color,
            composer: Composer::new(want.width, want.mode, cart.pilot),
            frames: 0,
        })
    }

    /// Input, one frame of emulation, then the view.
    pub fn step(&mut self, view: &mut [u16]) {
        #[cfg(test)]
        assert!(
            !PANIC_NEXT.swap(false, std::sync::atomic::Ordering::Relaxed),
            "test-injected panic"
        );
        let buttons = self.driver.buttons(&mut self.gb, self.frames);
        self.gb.set_buttons(buttons);
        if let Err(e) = self.gb.clock_for_frame() {
            panic!("{e:?}");
        }
        self.frames += 1;
        self.composer.compose(&mut self.gb, &self.tone, view);
    }

    /// A new `GAMEBOY_PALETTE`, without restarting the game.
    fn repaint(&mut self, palette: &str) {
        self.tone = tone_map(self.color, carts::shades(palette, self.pilot()));
    }

    pub fn pilot(&self) -> Pilot {
        self.driver.pilot()
    }

    pub fn fast(&self) -> bool {
        self.driver.fast()
    }
}

/// Makes the next frame panic, for the crash test.
#[cfg(test)]
pub static PANIC_NEXT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
impl Runner {
    pub fn frames(&self) -> u64 {
        self.session.as_ref().map_or(0, |s| s.frames)
    }

    pub fn playing(&self) -> String {
        self.session
            .as_ref()
            .map_or_else(String::new, |s| s.name.clone())
    }
}

pub struct Runner {
    carts: Vec<Cart>,
    idx: usize,
    session: Option<Session>,
    started: Instant,
    seed: u32,
    view: Vec<u16>,
}

impl Runner {
    pub(super) fn new(carts: Vec<Cart>) -> Self {
        Self {
            carts,
            idx: 0,
            session: None,
            started: Instant::now(),
            seed: 1,
            view: vec![0; MAX_W * H],
        }
    }

    /// Boot the current cartridge, skipping any that will not load. False
    /// when none will.
    fn ensure(&mut self, want: &Want) -> bool {
        if self.session.is_some() {
            return true;
        }
        for _ in 0..self.carts.len() {
            let cart = &self.carts[self.idx];
            self.seed = self.seed.wrapping_mul(0x9E37_79B9).wrapping_add(1);
            match Session::boot(cart, want, self.seed) {
                Ok(s) => {
                    eprintln!("[screensaver] gameboy: playing {}", cart.name);
                    self.session = Some(s);
                    self.started = Instant::now();
                    return true;
                }
                Err(e) => {
                    eprintln!("[screensaver] gameboy: {} will not boot: {e}", cart.name);
                    self.idx = (self.idx + 1) % self.carts.len();
                }
            }
        }
        false
    }

    /// A new want from the saver: reload the cartridges when they changed
    /// (or on a restart), else keep the game going under the new view.
    pub(super) fn take(&mut self, old: Option<&Want>, w: &Want) {
        if old.is_none_or(|o| !o.same_carts(w)) {
            // A restart is a fresh cartridge: no battery save, so the game
            // starts from NEW GAME.
            let restarted = old.is_some_and(|o| o.restart != w.restart);
            self.carts = carts::load(&w.rom, if restarted { "" } else { &w.sav });
            self.idx = (w.seed as usize) % self.carts.len().max(1);
            self.seed = w.seed;
            self.session = None;
        } else if let Some(s) = &mut self.session {
            s.composer = Composer::new(w.width, w.mode, s.pilot());
            s.repaint(&w.palette);
        }
    }

    fn next(&mut self) {
        self.session = None;
        self.idx = (self.idx + 1) % self.carts.len().max(1);
    }

    /// One frame. False when there is nothing to show.
    pub(super) fn frame(&mut self, want: &Want) -> bool {
        if !self.ensure(want) {
            return false;
        }
        let rotate = want.rotate.filter(|_| self.carts.len() > 1);
        if rotate.is_some_and(|d| self.started.elapsed() >= d) {
            self.next();
            if !self.ensure(want) {
                return false;
            }
        }
        let s = self.session.as_mut().expect("ensured above");
        let view = &mut self.view;
        if catch_unwind(AssertUnwindSafe(|| s.step(view))).is_err() {
            eprintln!(
                "[screensaver] gameboy: {} crashed; moving to the next cartridge",
                s.name
            );
            self.next();
            return self.ensure(want);
        }
        true
    }
}

fn run(e: &Engine) {
    let mut r = Runner::new(Vec::new());
    let mut rev = 0;
    let mut want: Option<Want> = None;
    let mut next = Instant::now();
    loop {
        let mut claimed = None;
        {
            let (c, parked) = e.0.wait(|_| false);
            if parked {
                next = Instant::now();
            }
            c.take(&mut rev, &mut claimed);
        }
        if let Some(w) = claimed {
            r.take(want.as_ref(), &w);
            want = Some(w);
        }
        let w = want.as_ref().expect("set on the first claim");
        // The Pokémon intro runs unseen at the core's own speed; the view
        // shows the last frame until it is out of the house.
        if r.session.as_ref().is_some_and(Session::fast) {
            r.frame(w);
            next = Instant::now();
            continue;
        }
        if r.frame(w) {
            e.0.frame().publish(&r.view, w.width, ());
        } else {
            e.0.frame().blank();
        }
        pace(&mut next, FRAME);
    }
}
