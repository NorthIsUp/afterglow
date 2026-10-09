//! Automatic rotation's order and timing, for `saver::Driver`.

use std::time::{Duration, Instant};

use crate::mirror;
use crate::next_rand;
use crate::saver::NSAVERS;

/// Automatic rotation: move to another saver every N seconds.
///
/// The interval is the MIRROR's, not this struct's: `SAVER_ROTATE_SECS` is only
/// the startup value and `POST /rotate` moves it while the pod runs. What lives
/// here is the deadline that interval implies, re-derived whenever the mirror's
/// control word changes under it.
///
/// Zero — the default — is off, so a deployment that does not ask for this
/// behaves exactly as it did. Out of range falls back to the default rather
/// than clamping, which is `env_num`'s contract everywhere else.
///
/// Order is a SHUFFLED BAG: every row, in random order, none repeated until all
/// of them have been shown. Not a walk down `SAVERS` — the objection to a walk
/// stands and is not being ignored here. A walk is predictable in the wrong way
/// (the same saver always follows the same saver forever, and the three toaster
/// variants are adjacent in the table, so a walk shows them back to back to
/// back); a bag is reshuffled every cycle, so neither is true of it. What the
/// bag adds over the plain roll this used to do is coverage: "rotate through
/// ALL the savers" was the ask, and an independent roll each time takes ~95
/// turns to show you all 25 (coupon collector) where the bag takes exactly 25 —
/// eight hours versus two at a five-minute interval.
///
/// "Never the same saver twice in a row" stays a property of the code rather
/// than a probability, which is the standard the old roll held itself to. Inside
/// a bag it is free (the entries are distinct); across the boundary between two
/// bags it is `refill`'s one swap.
///
/// Uniform turns for every saver, deliberately: the cheap ones do not get
/// longer ones. That trades one number for a table of per-saver seconds to
/// solve a problem nobody has — the expensive savers hold the target fps on
/// this panel, so there is nothing to compensate for.
pub struct Rotate {
    every: Duration,
    /// The control word `every` was decoded from. A `!=` against this is how a
    /// live change is noticed without a lock — see `Mirror::set_rotate_secs`.
    seen: u64,
    next: Instant,
    rng: u32,
    /// The rows still to be shown this cycle, in `bag[..left]`, drawn from the
    /// top. Sized once from the table and shuffled in place, so a refill
    /// allocates nothing — it lands on a rotation boundary, which is already a
    /// saver rebuild, but the frame path is no place to grow a Vec.
    bag: [usize; NSAVERS],
    left: usize,
}

impl Rotate {
    pub fn new(now: Instant) -> Self {
        // Seeded off the clock so a restart does not replay the same order.
        // Same trick sakura grows its tree from.
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0x5EED_1234, |d| d.subsec_nanos() ^ d.as_secs() as u32);
        Self::seeded(now, seed ^ std::process::id().wrapping_mul(0x9E37_79B9))
    }

    /// Starts with no interval at all: the first `due` adopts whatever the
    /// mirror holds, which is the `SAVER_ROTATE_SECS` main put there.
    pub fn seeded(now: Instant, seed: u32) -> Self {
        Self {
            every: Duration::ZERO,
            seen: 0,
            next: now,
            rng: seed,
            // Empty, so the first rotation fills it knowing what is on screen.
            bag: [0; NSAVERS],
            left: 0,
        }
    }

    /// The row to move to, or None when rotation is off or this saver's turn is
    /// not up yet. `now` is the frame's OWN clock read and `ctl` the mirror's
    /// rotation word, both handed down rather than taken here: with rotation
    /// off this is two compares per frame and no clock read at all, and with it
    /// on it is no more than that. See CLAUDE.md on the frame loop.
    pub fn due(
        &mut self,
        now: Instant,
        cur: usize,
        ctl: u64,
        pooled: impl Fn(usize) -> bool,
    ) -> Option<usize> {
        // Someone moved the interval since the last frame. Adopt it and give
        // what is on screen a full turn at the NEW length — the same restart a
        // click gets, and for the same reason: five minutes asked for at 4:59
        // into a turn must mean five minutes, not one second. The counter in
        // the word's high half is what makes re-posting the same number count.
        if ctl != self.seen {
            self.seen = ctl;
            self.every = Duration::from_secs(mirror::ctl_secs(ctl));
            self.restart(now);
        }
        if self.every.is_zero() || now < self.next {
            return None;
        }
        self.restart(now);
        // Rows out of the pool are skipped, not removed: the bag still covers
        // every pooled row once per cycle, and a pool change takes effect on
        // the next draw. Two passes because the first can empty the bag
        // without a hit; a pool with nothing in it but `cur` stays put.
        for _ in 0..2 {
            while self.left > 0 {
                self.left -= 1;
                let i = self.bag[self.left];
                if i != cur && pooled(i) {
                    return Some(i);
                }
            }
            self.refill(cur)?;
        }
        None
    }

    /// Every row, shuffled, none of them repeated until the bag empties.
    /// Fisher-Yates in place: no allocation, and no re-rolling of the whole
    /// shuffle to satisfy the boundary rule below. None for a one-row table,
    /// which has nowhere to go.
    fn refill(&mut self, cur: usize) -> Option<()> {
        let top = NSAVERS.checked_sub(1).filter(|t| *t > 0)?;
        for (i, slot) in self.bag.iter_mut().enumerate() {
            *slot = i;
        }
        for i in (1..NSAVERS).rev() {
            self.bag
                .swap(i, next_rand(&mut self.rng) as usize % (i + 1));
        }
        // The top of the bag is drawn first, so it is the row that would follow
        // `cur` immediately — the one place a bag can show the same saver twice
        // in a row. Swapping it with any other entry fixes that by construction
        // and keeps the bag a permutation; re-shuffling until it comes out
        // right would make it a probability again.
        if self.bag[top] == cur {
            self.bag.swap(top, next_rand(&mut self.rng) as usize % top);
        }
        self.left = NSAVERS;
        Some(())
    }

    /// Give whatever is on screen now a full turn.
    pub fn restart(&mut self, now: Instant) {
        self.next = now + self.every;
    }
}
