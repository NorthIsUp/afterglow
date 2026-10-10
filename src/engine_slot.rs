//! What every emulated saver shares: one engine per process, run by a thread
//! of its own for whichever saver claimed it last and parked when none has,
//! and a finished frame the render thread only ever `try_lock`s, so a slow
//! step there is a stale frame here, never a late one.
//!
//! Each engine keeps its own frame, its own crash handling and its own idea
//! of what a claim means; this is only the handshake.

// Each engine but gameboy is a cargo feature, so a narrower build leaves
// parts of this unused.
#![cfg_attr(
    not(all(feature = "doom", feature = "micropolis", feature = "mac")),
    allow(dead_code)
)]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// A poisoned lock means a panic on the other thread; the data is still a
/// frame or a claim, so carry on with it.
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the render thread last asked for. `X` is an engine's own extra.
pub struct Ctl<W, X> {
    /// The saver the engine runs for; 0 parks it.
    pub owner: u64,
    /// Bumped per claim, so the engine thread re-reads `want`.
    pub rev: u64,
    pub want: Option<W>,
    pub released: Option<Instant>,
    /// Make the engine fail on its next step; only tests set it.
    pub fault: bool,
    pub x: X,
}

impl<W: Clone, X> Ctl<W, X> {
    /// Copy a claim newer than `rev` into `want`; whether there was one.
    pub fn take(&self, rev: &mut u64, want: &mut Option<W>) -> bool {
        let changed = self.rev != *rev;
        if changed {
            *rev = self.rev;
            want.clone_from(&self.want);
        }
        changed
    }
}

pub struct Slot<W, F, X = ()> {
    ctl: Mutex<Ctl<W, X>>,
    wake: Condvar,
    frame: Mutex<F>,
    dead: AtomicBool,
}

impl<W, F, X> Slot<W, F, X> {
    pub fn new(frame: F, x: X) -> Self {
        Self {
            ctl: Mutex::new(Ctl {
                owner: 0,
                rev: 0,
                want: None,
                released: None,
                fault: false,
                x,
            }),
            wake: Condvar::new(),
            frame: Mutex::new(frame),
            dead: AtomicBool::new(false),
        }
    }

    /// Run the engine for saver `id`; `pre` sees the state before the claim
    /// lands, under the same lock.
    pub fn claim_with(&self, id: u64, want: W, pre: impl FnOnce(&mut Ctl<W, X>)) {
        let mut c = lock(&self.ctl);
        pre(&mut c);
        c.owner = id;
        c.rev += 1;
        c.want = Some(want);
        c.released = None;
        self.wake.notify_one();
    }

    pub fn claim(&self, id: u64, want: W) {
        self.claim_with(id, want, |_| {});
    }

    /// Park the engine, unless another saver has claimed it since.
    pub fn release(&self, id: u64) {
        let mut c = lock(&self.ctl);
        if c.owner == id {
            c.owner = 0;
            c.released = Some(Instant::now());
        }
    }

    /// Make the engine fail on its next step.
    #[cfg(test)]
    pub fn fault(&self) {
        lock(&self.ctl).fault = true;
    }

    /// For the engine thread: block while parked, or while `park` says so.
    /// True if it blocked, so the caller's clock restarts.
    pub fn wait(&self, park: impl Fn(&Ctl<W, X>) -> bool) -> (MutexGuard<'_, Ctl<W, X>>, bool) {
        let mut c = lock(&self.ctl);
        let mut parked = false;
        while c.owner == 0 || c.want.is_none() || park(&c) {
            c = self.wake.wait(c).unwrap_or_else(PoisonError::into_inner);
            parked = true;
        }
        (c, parked)
    }

    pub fn frame(&self) -> MutexGuard<'_, F> {
        lock(&self.frame)
    }

    /// The frame, unless the engine thread is mid-write on it.
    pub fn peek(&self) -> Option<MutexGuard<'_, F>> {
        self.frame.try_lock().ok()
    }

    /// The engine's state is unusable for the life of the process.
    pub fn kill(&self) {
        self.dead.store(true, Ordering::Relaxed);
    }

    pub fn dead(&self) -> bool {
        self.dead.load(Ordering::Relaxed)
    }
}

/// The latest finished view of an engine that draws `h` rows of a varying
/// width. `tag` rides along with it (Doom's palette). A blank view has
/// nothing to show and the saver draws static.
pub struct View<P, T> {
    pix: Box<[P]>,
    h: usize,
    w: usize,
    tag: T,
    seq: u32,
    blank: bool,
}

impl<P: Copy + Default, T: Copy + Default> View<P, T> {
    pub fn new(max_w: usize, h: usize) -> Self {
        Self {
            pix: vec![P::default(); max_w * h].into_boxed_slice(),
            h,
            w: 0,
            tag: T::default(),
            seq: 0,
            blank: true,
        }
    }

    pub fn publish(&mut self, src: &[P], w: usize, tag: T) {
        let n = w * self.h;
        self.pix[..n].copy_from_slice(&src[..n]);
        self.w = w;
        self.tag = tag;
        self.blank = false;
        self.seq = self.seq.wrapping_add(1);
    }

    pub fn blank(&mut self) {
        self.blank = true;
        self.seq = self.seq.wrapping_add(1);
    }

    /// Copy the view into `dst` if it moved on since `seen`. Returns the new
    /// sequence number, and the width and tag, `None` for a blank.
    pub fn read(&self, seen: u32, dst: &mut [P]) -> Option<(u32, Option<(usize, T)>)> {
        if self.seq == seen {
            return None;
        }
        if self.blank {
            return Some((self.seq, None));
        }
        let n = self.w * self.h;
        dst[..n].copy_from_slice(&self.pix[..n]);
        Some((self.seq, Some((self.w, self.tag))))
    }
}

/// Sleep until `next` plus a `period`; a step that ran long drops the backlog
/// rather than fast-forwarding through it.
pub fn pace(next: &mut Instant, period: Duration) {
    *next += period;
    let now = Instant::now();
    match next.checked_duration_since(now) {
        Some(d) => std::thread::sleep(d),
        None if now - *next > Duration::from_millis(250) => *next = now,
        None => {}
    }
}

/// An engine a saver can hold.
pub trait Engine: Sync + 'static {
    type Want;
    fn claim(&self, id: u64, want: Self::Want);
    fn release(&self, id: u64);
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A saver's hold on its engine, released on drop. Claimed by the first
/// frame, not the constructor: the mirror builds savers just to read their
/// knobs, and that must not steal the engine.
pub struct Claim<E: Engine> {
    /// Not a pointer: a dropped saver's address can be reused by the next
    /// one, and the engine must tell a new claim from a stale release.
    id: u64,
    /// Not yet handed over; `None` from the start means never.
    pub want: Option<E::Want>,
    pub engine: Option<&'static E>,
}

impl<E: Engine> Claim<E> {
    pub fn new(want: Option<E::Want>) -> Self {
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            want,
            engine: None,
        }
    }

    /// The engine, claimed from `get` on the first call with a want; `None`
    /// while there is none to run.
    pub fn engine(&mut self, get: impl FnOnce() -> &'static E) -> Option<&'static E> {
        if let Some(w) = self.want.take() {
            let e = get();
            e.claim(self.id, w);
            self.engine = Some(e);
        }
        self.engine
    }
}

impl<E: Engine> Drop for Claim<E> {
    fn drop(&mut self) {
        if let Some(e) = self.engine {
            e.release(self.id);
        }
    }
}
