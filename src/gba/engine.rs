//! The emulator and the one thread that runs it.
//!
//! The thread starts on first use, runs the GBA at `GBA_FPS` (59.73 Hz, its
//! own rate, by default) and parks whenever no `gba` saver is showing,
//! keeping the cartridge where it was. The render thread only ever
//! `try_lock`s the last finished view, so a slow frame here is a stale frame
//! there, never a late one.
//!
//! A core abort or segfault is caught in C (`gba/afterglow_gba.c`) and a
//! panic here at the frame boundary: either way that cartridge is dropped
//! and the next one boots. One that will not boot is skipped for the life of
//! the process.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use super::agb::{Core, H};
use super::carts::{self, Pokemon};
use super::pilot::Pilot;
use super::view;
use crate::cart_runner::{self, Deck};
use crate::engine_slot::{self, Slot, View};

/// The widest view: 5.4:1 glass at a square GBA pixel.
pub const MAX_W: usize = 864;
/// 280896 cycles at 16.777216 MHz.
const FRAME: Duration = Duration::from_nanos(16_742_706);

/// What the showing saver wants. Written by the render thread on a switch,
/// read by the engine thread once a frame.
#[derive(Clone, PartialEq)]
pub struct Want {
    pub rom: String,
    pub bios: String,
    /// View width in GBA pixels; the height is always [`H`].
    pub width: usize,
    /// Emulated frames a second; under 60 the game runs slow, and costs
    /// that much less.
    pub fps: u32,
    pub rotate: Option<Duration>,
    pub seed: u32,
    /// `saver::restarts("gba")` when built: a new value is a power-on reset.
    pub restart: u64,
}

impl Want {
    fn same_carts(&self, o: &Self) -> bool {
        (&self.rom, &self.bios, self.restart) == (&o.rom, &o.bios, o.restart)
    }

    fn period(&self) -> Duration {
        if self.fps >= 60 {
            FRAME
        } else {
            Duration::from_secs(1) / self.fps.max(1)
        }
    }
}

pub struct Engine(Slot<Want, View<u16, ()>>);

impl Engine {
    pub fn get() -> &'static Self {
        static E: OnceLock<Engine> = OnceLock::new();
        E.get_or_init(|| {
            std::thread::Builder::new()
                .name("gba".into())
                .spawn(|| cart_runner::run(&Engine::get().0, runner()))
                .expect("spawning the gba thread");
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

/// `gMain` in IWRAM: its first word, `callback1`, stays null through the
/// title and the professor's speech and is set once the overworld runs.
fn main_struct(p: Pokemon) -> u32 {
    match p {
        Pokemon::FireRed | Pokemon::LeafGreen => 0x0300_30F0,
        Pokemon::Emerald => 0x0300_22C0,
    }
}

/// One cartridge, booted.
pub struct Session {
    core: Core,
    pub name: String,
    pokemon: Option<Pokemon>,
    pilot: Pilot,
    keys: u32,
}

impl Session {
    pub fn boot(path: &Path, bios: &[u8], seed: u32) -> Result<Self, String> {
        let cart = carts::read(path).ok_or("unreadable")?;
        let core = Core::new(cart.rom.into_boxed_slice(), bios).ok_or("mGBA will not load it")?;
        Ok(Self {
            core,
            name: cart.name,
            pokemon: cart.pokemon,
            pilot: Pilot::new(cart.pokemon, seed),
            keys: 0,
        })
    }

    /// One frame of emulation under the pilot's keys, then the view. False
    /// if the core aborted.
    pub fn step(&mut self, width: usize, out: &mut [u16]) -> bool {
        #[cfg(test)]
        assert!(!PANIC_NEXT.replace(false), "test-injected panic");
        if !self.core.frame(self.keys) {
            return false;
        }
        let overworld = self
            .pokemon
            .is_some_and(|p| self.core.read32(main_struct(p)) != 0);
        self.keys = self.pilot.keys(self.core.pixels(), overworld);
        view::compose(self.core.pixels(), width, out);
        true
    }

    #[cfg(test)]
    pub fn pilot(&self) -> &Pilot {
        &self.pilot
    }
}

#[cfg(test)]
thread_local! {
    /// Makes this thread's next frame panic, for the crash test; per thread
    /// so a test stepping a session alongside it never takes the panic.
    pub static PANIC_NEXT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The ROM files `GBA_ROM` names, each read when it boots, and the BIOS.
pub struct Files {
    files: Vec<PathBuf>,
    bios: Vec<u8>,
}

pub type Runner = cart_runner::Runner<Files>;

pub fn runner() -> Runner {
    Runner::new(
        Files {
            files: Vec::new(),
            bios: Vec::new(),
        },
        MAX_W * H,
    )
}

impl Deck for Files {
    type Want = Want;
    type Session = Session;
    const NAME: &'static str = "gba";
    const RETIRE: bool = true;

    fn len(&self) -> usize {
        self.files.len()
    }

    fn label(&self, i: usize) -> String {
        self.files[i].display().to_string()
    }

    fn boot(&self, i: usize, _: &Want, seed: u32) -> Result<Session, String> {
        Session::boot(&self.files[i], &self.bios, seed)
    }

    fn name(s: &Session) -> &str {
        &s.name
    }

    fn step(s: &mut Session, want: &Want, view: &mut [u16]) -> bool {
        s.step(want.width, view)
    }

    fn width(want: &Want) -> usize {
        want.width
    }

    fn rotate(want: &Want) -> Option<Duration> {
        want.rotate
    }

    fn period(want: &Want) -> Duration {
        want.period()
    }

    /// Rescan the ROMs when they changed (or on a restart), else keep the
    /// game going under the new view.
    fn take(r: &mut Runner, old: Option<&Want>, w: &Want) {
        if old.is_some_and(|o| o.same_carts(w)) {
            return;
        }
        r.reload(
            || Files {
                files: carts::files(&w.rom),
                bios: read_bios(&w.bios),
            },
            w.seed,
        );
    }
}

/// `GBA_BIOS`, or nothing for mGBA's HLE BIOS.
fn read_bios(path: &str) -> Vec<u8> {
    if path.is_empty() {
        return Vec::new();
    }
    std::fs::read(path).unwrap_or_else(|e| {
        eprintln!("[screensaver] gba: GBA_BIOS {path}: {e}; using mGBA's HLE BIOS");
        Vec::new()
    })
}
