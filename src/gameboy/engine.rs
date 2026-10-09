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
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use mizu_core::{GameBoy, GameBoyConfig};

use super::carts::{self, Cart, Pilot, DMG_GREYS};
use super::pilot::Driver;
use super::view::{Composer, Mode};

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
}

impl Want {
    /// Same cartridges: a change in anything else keeps the game running.
    fn same_carts(&self, o: &Self) -> bool {
        (&self.rom, &self.sav, &self.palette) == (&o.rom, &o.sav, &o.palette)
    }
}

/// The latest finished view. `blank` frames have nothing to show (no
/// cartridge would boot) and the saver draws static.
struct Frame {
    pix: Box<[u16]>,
    w: usize,
    seq: u32,
    blank: bool,
}

struct Ctl {
    owner: u64,
    rev: u64,
    want: Option<Want>,
}

pub struct Engine {
    ctl: Mutex<Ctl>,
    wake: Condvar,
    frame: Mutex<Frame>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Engine {
    pub fn get() -> &'static Self {
        static E: OnceLock<Engine> = OnceLock::new();
        E.get_or_init(|| {
            std::thread::Builder::new()
                .name("gameboy".into())
                .spawn(|| run(Engine::get()))
                .expect("spawning the gameboy thread");
            Engine {
                ctl: Mutex::new(Ctl {
                    owner: 0,
                    rev: 0,
                    want: None,
                }),
                wake: Condvar::new(),
                frame: Mutex::new(Frame {
                    pix: vec![0; MAX_W * H].into_boxed_slice(),
                    w: 0,
                    seq: 0,
                    blank: true,
                }),
            }
        })
    }

    pub fn claim(&self, id: u64, want: Want) {
        let mut c = lock(&self.ctl);
        c.owner = id;
        c.rev += 1;
        c.want = Some(want);
        self.wake.notify_one();
    }

    /// Park the engine, unless another saver has claimed it since.
    pub fn release(&self, id: u64) {
        let mut c = lock(&self.ctl);
        if c.owner == id {
            c.owner = 0;
        }
    }

    /// Copy the view into `dst` if it moved on since `seen`, never waiting.
    /// Returns the new sequence number and the view's width, `None` for
    /// nothing to show.
    pub fn latest(&self, seen: u32, dst: &mut [u16]) -> Option<(u32, Option<usize>)> {
        let f = self.frame.try_lock().ok()?;
        if f.seq == seen {
            return None;
        }
        if f.blank {
            return Some((f.seq, None));
        }
        let n = f.w * H;
        dst[..n].copy_from_slice(&f.pix[..n]);
        Some((f.seq, Some(f.w)))
    }

    fn publish(&self, view: &[u16], w: usize) {
        let mut f = lock(&self.frame);
        f.pix[..w * H].copy_from_slice(&view[..w * H]);
        f.w = w;
        f.blank = false;
        f.seq = f.seq.wrapping_add(1);
    }

    fn blank(&self) {
        let mut f = lock(&self.frame);
        f.blank = true;
        f.seq = f.seq.wrapping_add(1);
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
    pub composer: Composer,
    pub frames: u64,
}

impl Session {
    pub fn boot(cart: &Cart, want: &Want, seed: u32) -> Result<Self, String> {
        // A monochrome game runs as one on a DMG: the core's GBC mode would
        // colour it with a palette of its own choosing.
        let color = cart.rom.get(0x143).is_some_and(|b| b & 0x80 != 0);
        let config = GameBoyConfig { is_dmg: !color };
        let gb = GameBoy::from_rom(cart.rom.clone(), cart.sram.as_deref(), config)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            gb,
            name: cart.name.clone(),
            driver: Driver::new(cart.pilot, seed),
            tone: tone_map(color, carts::shades(&want.palette, cart.pilot)),
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

    pub fn pilot(&self) -> Pilot {
        self.driver.pilot()
    }
}

/// Makes the next frame panic, for the crash test.
#[cfg(test)]
pub static PANIC_NEXT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
pub fn tests_runner(carts: Vec<Cart>) -> Runner {
    Runner {
        carts,
        idx: 0,
        session: None,
        started: Instant::now(),
        seed: 1,
        view: vec![0; MAX_W * H],
    }
}

#[cfg(test)]
impl Runner {
    pub fn frame_for_test(&mut self, want: &Want) -> bool {
        self.frame(want)
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

    fn next(&mut self) {
        self.session = None;
        self.idx = (self.idx + 1) % self.carts.len().max(1);
    }

    /// One frame. False when there is nothing to show.
    fn frame(&mut self, want: &Want) -> bool {
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
    let mut r = Runner {
        carts: Vec::new(),
        idx: 0,
        session: None,
        started: Instant::now(),
        seed: 1,
        view: vec![0; MAX_W * H],
    };
    let mut rev = 0;
    let mut want: Option<Want> = None;
    let mut next = Instant::now();
    loop {
        let w = {
            let mut c = lock(&e.ctl);
            while c.owner == 0 || c.want.is_none() {
                c = e
                    .wake
                    .wait(c)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                next = Instant::now();
            }
            let changed = c.rev != rev;
            rev = c.rev;
            c.want.clone().filter(|_| changed)
        };
        if let Some(w) = w {
            let reload = want.as_ref().is_none_or(|o| !o.same_carts(&w));
            if reload {
                r.carts = carts::load(&w.rom, &w.sav);
                r.idx = (w.seed as usize) % r.carts.len().max(1);
                r.seed = w.seed;
                r.session = None;
            } else if let Some(s) = &mut r.session {
                s.composer = Composer::new(w.width, w.mode, s.pilot());
            }
            want = Some(w);
        }
        let w = want.as_ref().expect("set on the first claim");
        if r.frame(w) {
            e.publish(&r.view, w.width);
        } else {
            e.blank();
        }
        next += FRAME;
        let now = Instant::now();
        match next.checked_duration_since(now) {
            Some(d) => std::thread::sleep(d),
            None if now - next > Duration::from_millis(250) => next = now,
            None => {}
        }
    }
}
