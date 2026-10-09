//! The Doom engines and the one thread that runs them.
//!
//! build.rs compiles doomgeneric [`INSTANCES`] times with every global renamed
//! `dg<N>_*`, so each copy is a whole separate world. They are C globals all
//! the same, so exactly one thread ever touches them: the engine thread here,
//! started on first use and parked whenever no `doom` saver is showing. The
//! render thread only ever `try_lock`s a finished frame — a level load or a
//! slow tic on this thread is a stale frame there, never a late one.
//!
//! An engine that hits `I_Error` (or any other `exit`) unwinds to the C
//! boundary and reports failure; its copy of the globals is then unusable for
//! the life of the process, so it is retired and its view moves to a spare
//! copy, or shows static once none is left.

use std::ffi::{c_char, c_int, CString};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::next_rand;

/// Must match `INSTANCES` in build.rs.
pub const INSTANCES: usize = 4;
pub const W: usize = 320;
pub const H: usize = 200;
const TICRATE: u64 = 35;

struct Api {
    init: unsafe extern "C" fn(*const c_char, u32) -> c_int,
    warp: unsafe extern "C" fn(u32, c_int) -> c_int,
    tick: unsafe extern "C" fn(u32) -> c_int,
    frame: unsafe extern "C" fn(*mut c_int) -> *const u8,
    fault: unsafe extern "C" fn() -> c_int,
}

macro_rules! apis {
    ($([$init:ident, $warp:ident, $tick:ident, $frame:ident, $fault:ident]),* $(,)?) => {
        extern "C" {
            $(
                fn $init(wad: *const c_char, seed: u32) -> c_int;
                fn $warp(seed: u32, brightness: c_int) -> c_int;
                fn $tick(ms: u32) -> c_int;
                fn $frame(palette: *mut c_int) -> *const u8;
                fn $fault() -> c_int;
            )*
        }
        const APIS: [Api; INSTANCES] = [$(Api {
            init: $init,
            warp: $warp,
            tick: $tick,
            frame: $frame,
            fault: $fault,
        }),*];
    };
}

apis!(
    [
        dg0_dgx_init,
        dg0_dgx_warp,
        dg0_dgx_tick,
        dg0_dgx_frame,
        dg0_dgx_fault
    ],
    [
        dg1_dgx_init,
        dg1_dgx_warp,
        dg1_dgx_tick,
        dg1_dgx_frame,
        dg1_dgx_fault
    ],
    [
        dg2_dgx_init,
        dg2_dgx_warp,
        dg2_dgx_tick,
        dg2_dgx_frame,
        dg2_dgx_fault
    ],
    [
        dg3_dgx_init,
        dg3_dgx_warp,
        dg3_dgx_tick,
        dg3_dgx_frame,
        dg3_dgx_fault
    ],
);

/// One view's latest finished frame. `seq` moves on every change, blanking
/// included; a blank view has nothing to show and the saver draws static.
pub struct Frame {
    pix: Box<[u8]>,
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
#[derive(Clone)]
pub struct Want {
    pub views: usize,
    pub seed: u32,
    pub wad: CString,
    pub map_every: Option<Duration>,
    pub light: c_int,
}

struct Ctl {
    /// The saver the engines run for; 0 parks the thread.
    owner: u64,
    /// Bumped per claim: every view starts a new map.
    gen: u64,
    want: Option<Want>,
    #[cfg(test)]
    fault: Option<usize>,
}

pub struct Engines {
    ctl: Mutex<Ctl>,
    wake: Condvar,
    frames: [Mutex<Frame>; INSTANCES],
    /// Which copy draws each view, `NONE` for static. Read by tests only.
    slot_of: [AtomicUsize; INSTANCES],
}

const NONE: usize = usize::MAX;

#[derive(Clone, Copy, PartialEq)]
enum Slot {
    Idle,
    Live,
    Dead,
}

impl Engines {
    pub fn get() -> &'static Self {
        static E: OnceLock<Engines> = OnceLock::new();
        E.get_or_init(|| {
            std::thread::Builder::new()
                .name("doom".into())
                .spawn(|| run(Engines::get()))
                .expect("spawning the doom thread");
            Engines {
                ctl: Mutex::new(Ctl {
                    owner: 0,
                    gen: 0,
                    want: None,
                    #[cfg(test)]
                    fault: None,
                }),
                wake: Condvar::new(),
                frames: std::array::from_fn(|_| {
                    Mutex::new(Frame {
                        pix: vec![0; W * H].into_boxed_slice(),
                        pal: 0,
                        seq: 0,
                        blank: true,
                    })
                }),
                slot_of: std::array::from_fn(|_| AtomicUsize::new(NONE)),
            }
        })
    }

    /// Run the engines for saver `id`, each view on a fresh random map. The
    /// frames are blanked first so the new saver shows static, not the last
    /// saver's map, until its own first frame lands.
    pub fn claim(&self, id: u64, want: Want) {
        for f in &self.frames {
            lock(f).blank();
        }
        let mut c = lock(&self.ctl);
        c.owner = id;
        c.gen += 1;
        c.want = Some(want);
        self.wake.notify_one();
    }

    /// Park the engines, unless another saver has claimed them since.
    pub fn release(&self, id: u64) {
        let mut c = lock(&self.ctl);
        if c.owner == id {
            c.owner = 0;
        }
    }

    /// Copy view `v`'s frame into `dst` if it moved on since `seen`. Never
    /// waits: a frame the engine thread is mid-write on is skipped. Returns
    /// the new sequence number and palette, or `None` for the palette when the
    /// view has nothing to show.
    pub fn latest(&self, v: usize, seen: u32, dst: &mut [u8]) -> Option<(u32, Option<u8>)> {
        let f = self.frames[v].try_lock().ok()?;
        if f.seq == seen {
            return None;
        }
        if f.blank {
            return Some((f.seq, None));
        }
        dst.copy_from_slice(&f.pix);
        Some((f.seq, Some(f.pal)))
    }

    #[cfg(test)]
    pub fn slot_of(&self, v: usize) -> Option<usize> {
        Some(self.slot_of[v].load(Ordering::Relaxed)).filter(|&s| s != NONE)
    }

    /// Make view `v`'s engine hit `I_Error` on its next tic.
    #[cfg(test)]
    pub fn fault(&self, v: usize) {
        lock(&self.ctl).fault = Some(v);
    }
}

/// A poisoned lock here means a panic on the engine thread, which aborts the
/// process anyway (`panic = "abort"`); in tests, keep going with the data.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

struct Runner {
    slots: [Slot; INSTANCES],
    view: [Option<usize>; INSTANCES],
    rng: u32,
}

impl Runner {
    fn publish(&self, e: &Engines, v: usize) {
        e.slot_of[v].store(self.view[v].unwrap_or(NONE), Ordering::Relaxed);
    }

    /// Give view `v` a working copy: its own, or the first idle or live one no
    /// other view holds. A copy that fails to start is retired.
    fn assign(&mut self, e: &Engines, v: usize, want: &Want) -> bool {
        if self.view[v].is_some_and(|s| self.slots[s] == Slot::Live) {
            return false;
        }
        self.view[v] = None;
        for (s, api) in APIS.iter().enumerate() {
            if self.slots[s] == Slot::Dead || self.view.contains(&Some(s)) {
                continue;
            }
            if self.slots[s] == Slot::Idle {
                let seed = next_rand(&mut self.rng);
                // SAFETY: only this thread calls into the engines, and `wad`
                // outlives the call.
                let ok = unsafe { (api.init)(want.wad.as_ptr(), seed) } == 0;
                self.slots[s] = if ok { Slot::Live } else { Slot::Dead };
                if !ok {
                    eprintln!("[screensaver] doom: engine {s} failed to start");
                    continue;
                }
            }
            self.view[v] = Some(s);
            break;
        }
        self.publish(e, v);
        true
    }

    fn retire(&mut self, e: &Engines, v: usize) {
        if let Some(s) = self.view[v].take() {
            eprintln!("[screensaver] doom: engine {s} hit an engine error; retired");
            self.slots[s] = Slot::Dead;
        }
        lock(&e.frames[v]).blank();
        self.publish(e, v);
    }

    fn warp(&mut self, e: &Engines, v: usize, light: c_int) {
        let Some(s) = self.view[v] else { return };
        let seed = next_rand(&mut self.rng);
        // SAFETY: as in `assign`.
        if unsafe { (APIS[s].warp)(seed, light) } < 0 {
            self.retire(e, v);
        }
    }

    fn tick(&mut self, e: &Engines, v: usize, ms: u32) {
        let Some(s) = self.view[v] else { return };
        // SAFETY: as in `assign`.
        if unsafe { (APIS[s].tick)(ms) } < 0 {
            self.retire(e, v);
            return;
        }
        let mut pal: c_int = 0;
        // SAFETY: a live engine's video buffer is W x H bytes, allocated at
        // init and freed only on the error path that retired it above.
        let src = unsafe { std::slice::from_raw_parts((APIS[s].frame)(&raw mut pal), W * H) };
        let mut f = lock(&e.frames[v]);
        f.pix.copy_from_slice(src);
        f.pal = pal.clamp(0, 13) as u8;
        f.blank = false;
        f.seq = f.seq.wrapping_add(1);
    }
}

fn run(e: &Engines) {
    let mut r = Runner {
        slots: [Slot::Idle; INSTANCES],
        view: [None; INSTANCES],
        rng: 1,
    };
    let mut gen = 0;
    let mut want: Option<Want> = None;
    let mut tics: u64 = 0;
    let mut next_tic = Instant::now();
    let mut next_map = None;
    loop {
        // Engine calls (an init is a WAD parse, a warp a level load) happen
        // with the lock released: `claim` on the render thread takes it.
        let (fresh, fault) = {
            let mut c = lock(&e.ctl);
            while c.owner == 0 || c.want.is_none() {
                c = e
                    .wake
                    .wait(c)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                next_tic = Instant::now();
            }
            let fresh = c.gen != gen;
            if fresh {
                gen = c.gen;
                want.clone_from(&c.want);
            }
            #[cfg(test)]
            let fault = c.fault.take();
            #[cfg(not(test))]
            let fault: Option<usize> = None;
            (fresh, fault)
        };
        let w = want.as_ref().expect("set on the first claim");
        let views = w.views.min(INSTANCES);
        if fresh {
            r.rng = w.seed ^ gen as u32;
        }
        let mut moved = false;
        for v in 0..views {
            moved |= r.assign(e, v, w);
        }
        let now = Instant::now();
        if fresh || next_map.is_some_and(|t| now >= t) {
            next_map = w.map_every.map(|d| now + d);
            for v in 0..views {
                r.warp(e, v, w.light);
            }
        } else if moved {
            // A view that just moved to a spare copy starts on a map of its own.
            for v in 0..views {
                if e.frames[v].try_lock().is_ok_and(|f| f.blank) {
                    r.warp(e, v, w.light);
                }
            }
        }
        if let Some(v) = fault.filter(|&v| v < views) {
            if let Some(s) = r.view[v] {
                // SAFETY: as in `Runner::assign`.
                if unsafe { (APIS[s].fault)() } < 0 {
                    r.retire(e, v);
                }
            }
        }
        // Doom's clock is whole milliseconds; stepping by the exact share of
        // each tic keeps 35 tics a second without drift.
        let ms = ((tics + 1) * 1000 / TICRATE - tics * 1000 / TICRATE) as u32;
        tics += 1;
        for v in 0..views {
            r.tick(e, v, ms);
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
