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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

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

/// The latest finished frame. `seq` moves on every change, blanking included;
/// a blank frame has nothing to show and the saver draws static.
pub struct Frame {
    pix: Box<[u8]>,
    w: usize,
    pal: u8,
    seq: u32,
    blank: bool,
}

impl Frame {
    fn blank(&mut self) {
        self.blank = true;
        self.seq = self.seq.wrapping_add(1);
    }
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

struct Ctl {
    /// The saver the engine runs for; 0 parks the thread.
    owner: u64,
    /// Bumped per claim that should start a new map.
    gen: u64,
    /// Bumped per claim at all, so the engine thread re-reads `want`.
    rev: u64,
    want: Option<Want>,
    released: Option<Instant>,
    #[cfg(test)]
    fault: bool,
}

pub struct Engine {
    ctl: Mutex<Ctl>,
    wake: Condvar,
    frame: Mutex<Frame>,
    dead: AtomicBool,
}

/// A poisoned lock here means a panic on the engine thread, which aborts the
/// process anyway (`panic = "abort"`); in tests, keep going with the data.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Engine {
    pub fn get() -> &'static Self {
        static E: OnceLock<Engine> = OnceLock::new();
        E.get_or_init(|| {
            std::thread::Builder::new()
                .name("doom".into())
                .spawn(|| run(Engine::get()))
                .expect("spawning the doom thread");
            Engine {
                ctl: Mutex::new(Ctl {
                    owner: 0,
                    gen: 0,
                    rev: 0,
                    want: None,
                    released: None,
                    #[cfg(test)]
                    fault: false,
                }),
                wake: Condvar::new(),
                frame: Mutex::new(Frame {
                    pix: vec![0; MAX_W * H].into_boxed_slice(),
                    w: 0,
                    pal: 0,
                    seq: 0,
                    blank: true,
                }),
                dead: AtomicBool::new(false),
            }
        })
    }

    /// Run the engine for saver `id`. A switch to `doom` starts a new map and
    /// blanks the frame so the last showing's map does not flash up; the same
    /// saver rebuilt for a knob change keeps playing.
    pub fn claim(&self, id: u64, want: Want) {
        let mut c = lock(&self.ctl);
        let rebuilt = c.released.is_some_and(|t| t.elapsed() < REBUILD);
        if !rebuilt {
            c.gen += 1;
            lock(&self.frame).blank();
        }
        c.owner = id;
        c.rev += 1;
        c.want = Some(want);
        c.released = None;
        self.wake.notify_one();
    }

    /// Park the engine, unless another saver has claimed it since.
    pub fn release(&self, id: u64) {
        let mut c = lock(&self.ctl);
        if c.owner == id {
            c.owner = 0;
            c.released = Some(Instant::now());
        }
    }

    /// Copy the frame into `dst` if it moved on since `seen`. Never waits: a
    /// frame the engine thread is mid-write on is skipped. Returns the new
    /// sequence number, and the frame's width and palette, or `None` when
    /// there is nothing to show.
    pub fn latest(&self, seen: u32, dst: &mut [u8]) -> Option<(u32, Option<(usize, u8)>)> {
        let f = self.frame.try_lock().ok()?;
        if f.seq == seen {
            return None;
        }
        if f.blank {
            return Some((f.seq, None));
        }
        let n = f.w * H;
        dst[..n].copy_from_slice(&f.pix[..n]);
        Some((f.seq, Some((f.w, f.pal))))
    }

    pub fn dead(&self) -> bool {
        self.dead.load(Ordering::Relaxed)
    }

    /// Make the engine hit `I_Error` on its next tic.
    #[cfg(test)]
    pub fn fault(&self) {
        lock(&self.ctl).fault = true;
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
            self.e.dead.store(true, Ordering::Relaxed);
            lock(&self.e.frame).blank();
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
        let mut f = lock(&self.e.frame);
        f.pix[..w * H].copy_from_slice(src);
        f.w = w;
        f.pal = pal.clamp(0, 13) as u8;
        f.blank = false;
        f.seq = f.seq.wrapping_add(1);
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
    let (mut gen, mut rev) = (0, 0);
    let mut want: Option<Want> = None;
    let mut tics: u64 = 0;
    let mut next_tic = Instant::now();
    let mut next_map = None;
    let mut level = -1;
    while !e.dead() {
        // Engine calls (an init is a WAD parse, a warp a level load) happen
        // with the lock released: `claim` on the render thread takes it.
        let (fresh, changed, fault) = {
            let mut c = lock(&e.ctl);
            while c.owner == 0 || c.want.is_none() {
                c = e
                    .wake
                    .wait(c)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                next_tic = Instant::now();
            }
            let fresh = c.gen != gen;
            let changed = c.rev != rev;
            (gen, rev) = (c.gen, c.rev);
            if changed {
                want.clone_from(&c.want);
            }
            #[cfg(test)]
            let fault = std::mem::take(&mut c.fault);
            #[cfg(not(test))]
            let fault = false;
            (fresh, changed, fault)
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
        next_tic += Duration::from_nanos(1_000_000_000 / TICRATE);
        let now = Instant::now();
        match next_tic.checked_duration_since(now) {
            Some(d) => std::thread::sleep(d),
            // A level load ran long: drop the backlog rather than fast-forward.
            None if now - next_tic > Duration::from_millis(250) => next_tic = now,
            None => {}
        }
    }
}
