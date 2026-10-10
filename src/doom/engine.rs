//! The Doom engine and the one thread that runs it.
//!
//! Doom keeps its whole world in C globals, so exactly one thread ever touches
//! them: the engine thread here, started on first use and parked whenever no
//! `doom` saver is showing. The render thread only ever `try_lock`s a
//! finished frame — a level load or a slow tic on this thread is a stale frame
//! there, never a late one.
//!
//! An engine that hits `I_Error` (or any other `exit`) unwinds to the C
//! boundary and reports failure. Its globals are then unusable for the life of
//! the process, so the engine is marked dead and the saver shows static.

use std::ffi::{c_char, c_int, CString};
#[cfg(test)]
use std::sync::atomic::Ordering;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::engine_slot::{self, pace, Slot, View};
use crate::next_rand;

pub const H: usize = 200;
/// Must match `MAXSCREENWIDTH` in `doom/doomgeneric/i_video.h`.
pub const MAX_W: usize = 1280;
const TICRATE: u64 = 35;
/// A claim this soon after a release is the same saver rebuilt for a knob
/// change, which keeps its map; anything later is a switch to `doom`.
const REBUILD: Duration = Duration::from_secs(1);

extern "C" {
    fn dgx_init(wad: *const c_char, seed: u32) -> c_int;
    fn dgx_view(width: c_int, pct: c_int, fov: c_int, hud: c_int, skill: c_int, god: c_int);
    fn dgx_warp(seed: u32, brightness: c_int) -> c_int;
    fn dgx_tick(ms: u32) -> c_int;
    fn dgx_frame(width: *mut c_int, palette: *mut c_int) -> *const u8;
    fn dgx_fault() -> c_int;
}

/// What the showing saver wants. Written by the render thread on a switch,
/// read by the engine thread once a tic.
#[derive(Clone, PartialEq)]
pub struct Want {
    pub seed: u32,
    pub wad: CString,
    /// Screen width in Doom pixels; the height is always [`H`].
    pub width: usize,
    pub view_pct: c_int,
    pub fov: c_int,
    pub hud: c_int,
    /// 1..=5, I'm Too Young To Die to Nightmare; from the next map on.
    pub skill: c_int,
    pub god: c_int,
    pub map_every: Option<Duration>,
    pub light: c_int,
}

/// A blank view has nothing to show and the saver draws static; the tag is
/// the palette.
type Frame = View<u8, u8>;

/// `x` is set by a claim that should start a new map.
pub struct Engine(Slot<Want, Frame, bool>);

impl Engine {
    pub fn get() -> &'static Self {
        static E: OnceLock<Engine> = OnceLock::new();
        E.get_or_init(|| {
            std::thread::Builder::new()
                .name("doom".into())
                .spawn(|| run(Engine::get()))
                .expect("spawning the doom thread");
            Engine(Slot::new(View::new(MAX_W, H), false))
        })
    }

    /// Copy the frame into `dst` if it moved on since `seen`. Never waits: a
    /// frame the engine thread is mid-write on is skipped. Returns the new
    /// sequence number, and the frame's width and palette, or `None` when
    /// there is nothing to show.
    pub fn latest(&self, seen: u32, dst: &mut [u8]) -> Option<(u32, Option<(usize, u8)>)> {
        self.0.peek()?.read(seen, dst)
    }

    pub fn dead(&self) -> bool {
        self.0.dead()
    }

    /// Make the engine hit `I_Error` on its next tic.
    #[cfg(test)]
    pub fn fault(&self) {
        self.0.fault();
    }
}

impl engine_slot::Engine for Engine {
    type Want = Want;

    /// A switch to `doom` starts a new map and blanks the frame so the last
    /// showing's map does not flash up; the same saver rebuilt for a knob
    /// change keeps playing.
    fn claim(&self, id: u64, want: Want) {
        self.0.claim_with(id, want, |c| {
            if c.released.is_none_or(|t| t.elapsed() >= REBUILD) {
                c.x = true;
                self.0.frame().blank();
            }
        });
    }

    fn release(&self, id: u64) {
        self.0.release(id);
    }
}

struct Runner<'a> {
    e: &'a Engine,
    started: bool,
    rng: u32,
}

impl Runner<'_> {
    /// An engine call that failed killed the engine; say so once and blank.
    fn check(&self, rc: c_int) -> bool {
        if rc < 0 {
            eprintln!("[screensaver] doom: the engine hit an error; showing static");
            self.e.0.kill();
            self.e.0.frame().blank();
        }
        rc >= 0
    }

    fn apply(&mut self, w: &Want, fresh: bool) -> bool {
        // SAFETY (every call below): only this thread enters the engine, and
        // the pointers handed in outlive the call.
        unsafe { dgx_view(w.width as c_int, w.view_pct, w.fov, w.hud, w.skill, w.god) };
        if !self.started {
            self.started = true;
            self.rng = w.seed;
            if !self.check(unsafe { dgx_init(w.wad.as_ptr(), w.seed) }) {
                return false;
            }
        }
        if !fresh {
            return true;
        }
        let seed = next_rand(&mut self.rng);
        self.check(unsafe { dgx_warp(seed, w.light) })
    }

    /// One tic. Returns the count of levels started so far, or `None` if the
    /// engine died.
    fn tick(&self, ms: u32) -> Option<c_int> {
        #[cfg(test)]
        let t0 = Instant::now();
        let level = unsafe { dgx_tick(ms) };
        if !self.check(level) {
            return None;
        }
        let (mut w, mut pal): (c_int, c_int) = (0, 0);
        let src = unsafe { dgx_frame(&raw mut w, &raw mut pal) };
        let w = (w.max(0) as usize).min(MAX_W);
        // SAFETY: the video buffer is MAXSCREENWIDTH x 200, allocated at init
        // and freed only on the error path, which returned above.
        let src = unsafe { std::slice::from_raw_parts(src, w * H) };
        self.e.0.frame().publish(src, w, pal.clamp(0, 13) as u8);
        #[cfg(test)]
        {
            TICK_NS.fetch_add(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);
            TICKS.fetch_add(1, Ordering::Relaxed);
        }
        Some(level)
    }
}

/// Engine time per tic, for the bench: the tic, the render and the copy out.
#[cfg(test)]
pub static TICK_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
#[cfg(test)]
pub static TICKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn run(e: &Engine) {
    let mut r = Runner {
        e,
        started: false,
        rng: 1,
    };
    let mut rev = 0;
    let mut want: Option<Want> = None;
    let mut tics: u64 = 0;
    let mut next_tic = Instant::now();
    let mut next_map = None;
    let mut level = -1;
    while !e.dead() {
        // Engine calls (an init is a WAD parse, a warp a level load) happen
        // with the lock released: `claim` on the render thread takes it.
        let (fresh, changed, fault) = {
            let (mut c, parked) = e.0.wait(|_| false);
            if parked {
                next_tic = Instant::now();
            }
            let changed = c.take(&mut rev, &mut want);
            (
                std::mem::take(&mut c.x),
                changed,
                std::mem::take(&mut c.fault),
            )
        };
        let w = want.as_ref().expect("set on the first claim");
        let now = Instant::now();
        let timed = next_map.is_some_and(|t| now >= t);
        if (changed || timed) && !r.apply(w, fresh || timed) {
            break;
        }
        if fresh || timed {
            next_map = w.map_every.map(|d| now + d);
        }
        // SAFETY: as in `Runner::apply`.
        if fault && !r.check(unsafe { dgx_fault() }) {
            break;
        }
        // Doom's clock is whole milliseconds; stepping by the exact share of
        // each tic keeps 35 tics a second without drift.
        let ms = ((tics + 1) * 1000 / TICRATE - tics * 1000 / TICRATE) as u32;
        tics += 1;
        let Some(now_level) = r.tick(ms) else {
            break;
        };
        // A map the autopilot finished, or died on, was followed by a new
        // one: its time runs from its own start.
        if now_level != level {
            level = now_level;
            next_map = w.map_every.map(|d| Instant::now() + d);
        }
        pace(&mut next_tic, Duration::from_nanos(1_000_000_000 / TICRATE));
    }
}
