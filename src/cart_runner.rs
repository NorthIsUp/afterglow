//! The engine thread the emulator savers (`gameboy`, `gba`) share: a
//! rotation of cartridges with the one playing booted, one that will not
//! boot skipped, a panic caught at the frame boundary and the next booted,
//! and the loop that paces frames into the slot. What a cartridge is, and
//! how one boots and plays, is the [`Deck`]'s.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::{Duration, Instant};

use crate::engine_slot::{pace, Slot, View};

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
        }
    }

    /// A new want from the saver, `old` the one before.
    pub fn take(&mut self, old: Option<&D::Want>, want: &D::Want) {
        D::take(self, old, want);
    }

    /// A new deck, starting from the cartridge `seed` picks. The playing
    /// one is dropped first.
    pub fn reload(&mut self, deck: impl FnOnce() -> D, seed: u32) {
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
                Ok(s) => {
                    eprintln!("[screensaver] {}: playing {}", D::NAME, D::name(&s));
                    self.session = Some(s);
                    self.started = Instant::now();
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

    /// Drop the playing cartridge before the next is booted, so two are
    /// never running at once.
    fn next(&mut self) {
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
            self.bad[self.idx] = D::RETIRE;
            self.next();
            return self.ensure(want);
        }
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
            let (c, parked) = slot.wait(|_| false);
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
