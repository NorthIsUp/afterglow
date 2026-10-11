//! The engine thread the emulator savers (`gameboy`, `gba`) share: a
//! rotation of cartridges with the one playing booted, one that will not
//! boot skipped, a panic caught at the frame boundary and the next booted,
//! and the loop that paces frames into the slot. What a cartridge is, and
//! how one boots and plays, is the [`Deck`]'s.
//!
//! A cartridge picks up where it left off: a save state is taken when it
//! rotates away, when the saver is switched away, when the deck reloads and
//! every [`Deck::snap_every`], and its next boot (in this process or, under
//! `SAVER_STATE_DIR`, the next) loads it. See `savestate.rs`.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::{Duration, Instant};

use crate::engine_slot::{pace, Slot, View};
use crate::savestate::Store;

pub trait Deck: Sized {
    type Want: Clone;
    type Session;
    /// The saver, for the log.
    const NAME: &'static str;
    /// A cartridge that would not boot or crashed is never tried again.
    const RETIRE: bool;

    fn len(&self) -> usize;
    /// Cartridge `i`, for the log when it will not boot.
    fn label(&self, i: usize) -> String;
    fn boot(&self, i: usize, want: &Self::Want, seed: u32) -> Result<Self::Session, String>;
    fn name(s: &Self::Session) -> &str;
    /// One frame into `view`; false if the core aborted.
    fn step(s: &mut Self::Session, want: &Self::Want, view: &mut [u16]) -> bool;
    /// Running unseen at the core's own speed, unpaced and unpublished.
    fn fast(_: &Self::Session) -> bool {
        false
    }
    fn width(want: &Self::Want) -> usize;
    fn rotate(want: &Self::Want) -> Option<Duration>;
    /// How often a save state is taken while it plays.
    fn snap_every(want: &Self::Want) -> Option<Duration>;
    /// The core's save-state format, so a state from another is not loaded.
    const FORMAT: &'static str;
    /// The playing ROM's SHA-1, hex: whose save state is whose.
    fn rom_id(s: &Self::Session) -> &str;
    /// Append a save state to `out`; false when this is no moment for one.
    fn snap(s: &mut Self::Session, out: &mut Vec<u8>) -> bool;
    /// Load what `snap` wrote into a fresh boot. An error leaves the
    /// session unusable.
    fn restore(s: &mut Self::Session, state: &[u8]) -> Result<(), String>;
    fn period(want: &Self::Want) -> Duration;
    /// A new want from the saver, `old` the one before.
    fn take(r: &mut Runner<Self>, old: Option<&Self::Want>, want: &Self::Want);
}

pub struct Runner<D: Deck> {
    pub deck: D,
    /// Cartridges that would not boot or crashed, under [`Deck::RETIRE`].
    bad: Vec<bool>,
    idx: usize,
    pub session: Option<D::Session>,
    started: Instant,
    seed: u32,
    pub view: Vec<u16>,
    pub store: Store,
    /// When the playing cartridge was last saved, or booted.
    snapped: Instant,
    /// It has played a frame since.
    dirty: bool,
    snap_buf: Vec<u8>,
    /// Boots that loaded a save state.
    #[cfg(test)]
    pub resumes: u32,
}

impl<D: Deck> Runner<D> {
    pub fn new(deck: D, view_len: usize) -> Self {
        Self {
            bad: vec![false; deck.len()],
            deck,
            idx: 0,
            session: None,
            started: Instant::now(),
            seed: 1,
            view: vec![0; view_len],
            store: Store::new(D::NAME),
            snapped: Instant::now(),
            dirty: false,
            snap_buf: Vec::new(),
            #[cfg(test)]
            resumes: 0,
        }
    }

    /// A new want from the saver, `old` the one before.
    pub fn take(&mut self, old: Option<&D::Want>, want: &D::Want) {
        D::take(self, old, want);
    }

    /// A new deck, starting from the cartridge `seed` picks. The playing
    /// one is saved and dropped first.
    pub fn reload(&mut self, deck: impl FnOnce() -> D, seed: u32) {
        self.snap();
        self.session = None;
        self.deck = deck();
        self.bad = vec![false; self.deck.len()];
        self.idx = (seed as usize) % self.deck.len().max(1);
        self.seed = seed;
    }

    /// Boot the current cartridge, skipping any that will not. False when
    /// none will.
    fn ensure(&mut self, want: &D::Want) -> bool {
        if self.session.is_some() {
            return true;
        }
        let n = self.deck.len();
        for _ in 0..n {
            let i = self.idx;
            self.idx = (i + 1) % n;
            if self.bad[i] {
                continue;
            }
            self.seed = self.seed.wrapping_mul(0x9E37_79B9).wrapping_add(1);
            match self.deck.boot(i, want, self.seed) {
                Ok(mut s) => {
                    if !self.resume(&mut s) {
                        match self.deck.boot(i, want, self.seed) {
                            Ok(fresh) => s = fresh,
                            Err(_) => continue,
                        }
                    }
                    eprintln!("[screensaver] {}: playing {}", D::NAME, D::name(&s));
                    self.session = Some(s);
                    self.started = Instant::now();
                    self.snapped = self.started;
                    self.dirty = false;
                    // Rotation and a crash move on from the one playing.
                    self.idx = i;
                    return true;
                }
                Err(e) => {
                    let label = self.deck.label(i);
                    eprintln!("[screensaver] {}: {label} will not boot: {e}", D::NAME);
                    self.bad[i] = D::RETIRE;
                }
            }
        }
        false
    }

    /// Load a fresh boot's saved state, if it has one. False if loading
    /// failed and the session must be booted again.
    fn resume(&mut self, s: &mut D::Session) -> bool {
        let id = D::rom_id(s).to_owned();
        let Some(state) = self.store.get(&id, D::FORMAT) else {
            return true;
        };
        let loaded = catch_unwind(AssertUnwindSafe(|| D::restore(s, &state)))
            .unwrap_or_else(|_| Err("the core panicked".into()));
        match loaded {
            Ok(()) => {
                eprintln!("[screensaver] {}: resuming {}", D::NAME, D::name(s));
                #[cfg(test)]
                {
                    self.resumes += 1;
                }
                true
            }
            Err(e) => {
                eprintln!(
                    "[screensaver] {}: {}'s save state will not load ({e}); dropped",
                    D::NAME,
                    D::name(s)
                );
                self.store.discard(&id);
                false
            }
        }
    }

    /// Save the playing cartridge, if it has played since it was last saved.
    pub fn snap(&mut self) {
        let Some(s) = self.session.as_mut().filter(|_| self.dirty) else {
            return;
        };
        let t = Instant::now();
        self.snap_buf.clear();
        if D::snap(s, &mut self.snap_buf) {
            self.store.put(D::rom_id(s), D::FORMAT, &self.snap_buf);
            eprintln!(
                "[screensaver] {}: saved {} ({} KiB in {:.1?})",
                D::NAME,
                D::name(s),
                self.snap_buf.len() / 1024,
                t.elapsed()
            );
        }
        self.snapped = Instant::now();
        self.dirty = false;
    }

    /// Drop the playing cartridge and its save states: a deliberate fresh
    /// start.
    pub fn forget(&mut self) {
        if let Some(s) = self.session.take() {
            self.store.forget(D::rom_id(&s));
        }
    }

    /// Save and drop the playing cartridge before the next is booted, so
    /// two are never running at once.
    pub fn next(&mut self) {
        self.snap();
        self.session = None;
        self.idx = (self.idx + 1) % self.deck.len().max(1);
    }

    /// One frame. False when there is nothing to show.
    pub fn frame(&mut self, want: &D::Want) -> bool {
        if !self.ensure(want) {
            return false;
        }
        let rotate = D::rotate(want).filter(|_| self.deck.len() > 1);
        if rotate.is_some_and(|d| self.started.elapsed() >= d) {
            self.next();
            if !self.ensure(want) {
                return false;
            }
        } else if D::snap_every(want).is_some_and(|d| self.snapped.elapsed() >= d) {
            self.snap();
        }
        let s = self.session.as_mut().expect("ensured above");
        let view = &mut self.view;
        let ok = catch_unwind(AssertUnwindSafe(|| D::step(s, want, view))).unwrap_or(false);
        if !ok {
            eprintln!(
                "[screensaver] {}: {} crashed; moving to the next cartridge",
                D::NAME,
                D::name(s)
            );
            // Its newest state may be what crashes it: the next turn loads
            // the one before.
            self.store.discard(D::rom_id(s));
            self.bad[self.idx] = D::RETIRE;
            self.session = None;
            self.idx = (self.idx + 1) % self.deck.len().max(1);
            return self.ensure(want);
        }
        self.dirty = true;
        true
    }
}

/// The engine thread: take each new want, play a frame, publish it (or a
/// blank: nothing to show) and pace to the want's period.
pub fn run<D: Deck>(slot: &Slot<D::Want, View<u16, ()>>, mut r: Runner<D>) -> ! {
    let mut rev = 0;
    let mut want: Option<D::Want> = None;
    let mut next = Instant::now();
    loop {
        let mut claimed = None;
        {
            // Saved before it parks: the pod may restart before it is shown
            // again.
            let (c, parked) = slot.wait_or_idle(|_| false, || r.snap());
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
        if r.session.as_ref().is_some_and(D::fast) {
            r.frame(w);
            next = Instant::now();
            continue;
        }
        if r.frame(w) {
            slot.frame().publish(&r.view, D::width(w), ());
        } else {
            slot.frame().blank();
        }
        pace(&mut next, D::period(w));
    }
}
