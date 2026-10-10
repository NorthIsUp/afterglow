//! The emulator and the one thread that runs it.
//!
//! The thread starts on first use, runs the GBA at `GBA_FPS` (59.73 Hz, its
//! own rate, by default) and parks whenever no `gba` saver is showing,
//! keeping the cartridge where it was. The render thread only ever
//! `try_lock`s the last finished view, so a slow frame here is a stale frame
//! there, never a late one.
//!
//! A core abort is caught in C (`gba/afterglow_gba.c`) and a panic here at
//! the frame boundary: either way that cartridge is dropped and the next one
//! boots. One that will not boot is skipped for the life of the process.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use super::agb::{Core, H};
use super::carts::{self, Pokemon};
use super::pilot::Pilot;
use super::view;
use crate::engine_slot::{self, pace, Slot, View};

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
                .spawn(|| run(Engine::get()))
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
    pub frames: u64,
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
            frames: 0,
        })
    }

    /// One frame of emulation under the pilot's keys, then the view. False
    /// if the core aborted.
    pub fn step(&mut self, width: usize, out: &mut [u16]) -> bool {
        #[cfg(test)]
        assert!(
            !PANIC_NEXT.swap(false, std::sync::atomic::Ordering::Relaxed),
            "test-injected panic"
        );
        if !self.core.frame(self.keys) {
            return false;
        }
        self.frames += 1;
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

/// Makes the next frame panic, for the crash test.
#[cfg(test)]
pub static PANIC_NEXT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub struct Runner {
    files: Vec<PathBuf>,
    /// Files that would not boot, never tried again.
    bad: Vec<bool>,
    bios: Vec<u8>,
    idx: usize,
    pub session: Option<Session>,
    started: Instant,
    seed: u32,
    pub view: Vec<u16>,
}

impl Runner {
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
            bad: Vec::new(),
            bios: Vec::new(),
            idx: 0,
            session: None,
            started: Instant::now(),
            seed: 1,
            view: vec![0; MAX_W * H],
        }
    }

    /// Boot the current cartridge, skipping any that will not. False when
    /// none will.
    fn ensure(&mut self) -> bool {
        if self.session.is_some() {
            return true;
        }
        for _ in 0..self.files.len() {
            let i = self.idx;
            self.idx = (i + 1) % self.files.len();
            if self.bad[i] {
                continue;
            }
            self.seed = self.seed.wrapping_mul(0x9E37_79B9).wrapping_add(1);
            match Session::boot(&self.files[i], &self.bios, self.seed) {
                Ok(s) => {
                    eprintln!("[screensaver] gba: playing {}", s.name);
                    self.session = Some(s);
                    self.started = Instant::now();
                    // Rotation and a crash move on from the one playing.
                    self.idx = i;
                    return true;
                }
                Err(e) => {
                    eprintln!(
                        "[screensaver] gba: {} will not boot: {e}",
                        self.files[i].display()
                    );
                    self.bad[i] = true;
                }
            }
        }
        false
    }

    /// A new want from the saver: rescan the ROMs when they changed (or on
    /// a restart), else keep the game going under the new view.
    pub fn take(&mut self, old: Option<&Want>, w: &Want) {
        if old.is_some_and(|o| o.same_carts(w)) {
            return;
        }
        self.session = None;
        self.files = carts::files(&w.rom);
        self.bad = vec![false; self.files.len()];
        self.bios = if w.bios.is_empty() {
            Vec::new()
        } else {
            std::fs::read(&w.bios).unwrap_or_else(|e| {
                eprintln!(
                    "[screensaver] gba: GBA_BIOS {}: {e}; using mGBA's HLE BIOS",
                    w.bios
                );
                Vec::new()
            })
        };
        self.idx = (w.seed as usize) % self.files.len().max(1);
        self.seed = w.seed;
    }

    /// Drop the playing cartridge before the next is read, so two ROMs are
    /// never in memory at once.
    fn next(&mut self) {
        self.session = None;
        self.idx = (self.idx + 1) % self.files.len().max(1);
    }

    /// One frame. False when there is nothing to show.
    pub fn frame(&mut self, want: &Want) -> bool {
        if !self.ensure() {
            return false;
        }
        let rotate = want.rotate.filter(|_| self.files.len() > 1);
        if rotate.is_some_and(|d| self.started.elapsed() >= d) {
            self.next();
            if !self.ensure() {
                return false;
            }
        }
        let s = self.session.as_mut().expect("ensured above");
        let view = &mut self.view;
        let ok = catch_unwind(AssertUnwindSafe(|| s.step(want.width, view))).unwrap_or(false);
        if !ok {
            eprintln!(
                "[screensaver] gba: {} crashed; moving to the next cartridge",
                s.name
            );
            self.bad[self.idx] = true;
            self.next();
            return self.ensure();
        }
        true
    }
}

fn run(e: &Engine) {
    let mut r = Runner::new();
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
        if r.frame(w) {
            e.0.frame().publish(&r.view, w.width, ());
        } else {
            e.0.frame().blank();
        }
        pace(&mut next, w.period());
    }
}
