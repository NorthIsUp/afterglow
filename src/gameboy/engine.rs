//! The emulator and the one thread that runs it.
//!
//! The thread starts on first use, runs the Game Boy at its own 59.73 Hz and
//! parks whenever no `gameboy` saver is showing, keeping the cartridge where
//! it was. The render thread only ever `try_lock`s the last finished view, so
//! a slow frame here is a stale frame there, never a late one.
//!
//! A panic anywhere in a frame (the core, a pilot, a view) is caught at the
//! frame boundary: that cartridge is dropped and the next one boots.

use std::panic::catch_unwind;
use std::sync::OnceLock;
use std::time::Duration;

use mizu_core::{GameBoy, GameBoyConfig, SAVE_STATE_VERSION};

use super::carts::{self, Cart, Pilot, DMG_GREYS};
use super::pilot::Driver;
use super::pokemon::Knobs;
use super::view::{Composer, Mode};
use crate::cart_runner::{self, Deck};
use crate::engine_slot::{self, Slot, View};

pub const H: usize = 144;
pub const SCREEN_W: usize = 160;
/// The widest view: 5.3:1 glass at a square Game Boy pixel.
pub const MAX_W: usize = 768;
const _: () = assert!(
    SAVE_STATE_VERSION == 2,
    "a new mizu state: bump Carts::FORMAT"
);
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
    /// How often the playing cartridge is saved.
    pub snap_every: Option<Duration>,
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
                .spawn(|| cart_runner::run(&Engine::get().0, runner(Vec::new())))
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
    sha1: String,
    driver: Driver,
    tone: Box<[u16]>,
    color: bool,
    pub composer: Composer,
    pub frames: u64,
}

impl Session {
    pub fn boot(cart: &Cart, want: &Want, seed: u32) -> Result<Self, String> {
        let color = carts::color(&cart.rom);
        let config = GameBoyConfig { is_dmg: !color };
        // The Pokémon bot plays a new game from power-on, or its own save
        // state: a battery save would put it somewhere its intro does not
        // know.
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
            sha1: cart.sha1.clone(),
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

/// The cartridges `GAMEBOY_ROM` names, held in memory.
pub struct Carts(pub Vec<Cart>);

pub type Runner = cart_runner::Runner<Carts>;

pub fn runner(carts: Vec<Cart>) -> Runner {
    Runner::new(Carts(carts), MAX_W * H)
}

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

impl Deck for Carts {
    type Want = Want;
    type Session = Session;
    const NAME: &'static str = "gameboy";
    // A cartridge that will not boot is tried again on its next turn.
    const RETIRE: bool = false;

    fn len(&self) -> usize {
        self.0.len()
    }

    fn label(&self, i: usize) -> String {
        self.0[i].name.clone()
    }

    fn boot(&self, i: usize, want: &Want, seed: u32) -> Result<Session, String> {
        Session::boot(&self.0[i], want, seed)
    }

    fn name(s: &Session) -> &str {
        &s.name
    }

    fn step(s: &mut Session, _: &Want, view: &mut [u16]) -> bool {
        s.step(view);
        true
    }

    /// The Pokémon intro runs unseen at the core's own speed; the view
    /// shows the last frame until it is out of the house.
    fn fast(s: &Session) -> bool {
        s.fast()
    }

    fn width(want: &Want) -> usize {
        want.width
    }

    fn rotate(want: &Want) -> Option<Duration> {
        want.rotate
    }

    fn snap_every(want: &Want) -> Option<Duration> {
        want.snap_every
    }

    const FORMAT: &'static str = "mizu-1.3.0/2";

    fn rom_id(s: &Session) -> &str {
        &s.sha1
    }

    fn snap(s: &mut Session, out: &mut Vec<u8>) -> bool {
        s.driver.can_snap() && s.gb.save_state(out).is_ok()
    }

    fn restore(s: &mut Session, state: &[u8]) -> Result<(), String> {
        s.gb.load_state(state).map_err(|e| e.to_string())?;
        s.driver.resumed(&mut s.gb);
        Ok(())
    }

    fn period(_: &Want) -> Duration {
        FRAME
    }

    /// Reload the cartridges when they changed (or on a restart), else keep
    /// the game going under the new view.
    fn take(r: &mut Runner, old: Option<&Want>, w: &Want) {
        if old.is_none_or(|o| !o.same_carts(w)) {
            // A restart is a fresh cartridge: no battery save and no save
            // state, so the game starts from NEW GAME.
            let restarted = old.is_some_and(|o| o.restart != w.restart);
            if restarted {
                r.forget();
            }
            let sav = if restarted { "" } else { &w.sav };
            r.reload(|| Carts(carts::load(&w.rom, sav)), w.seed);
        } else if let Some(s) = &mut r.session {
            s.composer = Composer::new(w.width, w.mode, s.pilot());
            s.repaint(&w.palette);
        }
    }
}
