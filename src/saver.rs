//! The one thing a screensaver is, and the one per-frame call around it.

use std::time::{Duration, Instant};

use crate::ascii_rest::{Piece, Play};
use crate::city::City;
use crate::confetti::Confetti;
use crate::doodles::Doodles;
use crate::dvd::Dvd;
use crate::fire::Fire;
use crate::fractal::Fractal;
use crate::grid::Grid;
use crate::hardrain::HardRain;
use crate::hypercube::Hypercube;
use crate::life::Life;
use crate::lissajous::Lissajous;
use crate::marble::Marble;
use crate::matrix::Matrix;
use crate::mirror::{self, Mirror};
use crate::moire::Moire;
use crate::next_rand;
use crate::plasma::Plasma;
use crate::podracer::Podracer;
use crate::pov::Pov;
use crate::rain::Rain;
use crate::sakura::Sakura;
use crate::satori::Satori;
use crate::speeder::Speeder;
use crate::strings::Strings;
use crate::surface::{Damage, Panel, Surface};
use crate::tactiles::Tactiles;
use crate::toasters::Toasters;
use crate::toasters2::Toasters2;
use crate::toasters3::Toasters3;
use crate::warp::Warp;
use crate::worms::Worms;
use crate::xwing::XWing;
use crate::zot::Zot;

/// A screensaver. One frame, one call. Dispatch happens here and NOWHERE below
/// it: no `&dyn Palette`, no `fn cell(&self, x, y) -> Cell`, no `&mut dyn FnMut`
/// handed to the blit. Those convert 15 indirect calls a second into ~120k
/// (per cell) or ~31M (per pixel) and would blow the 500m limit on their own.
pub trait Saver {
    /// Advance the simulation and draw the frame. Damage is whatever `s`
    /// accrued while being written — a saver cannot report it and cannot
    /// under-report it.
    fn render(&mut self, s: &mut Surface<'_>);

    /// For the startup log line.
    fn name(&self) -> &'static str;

    /// The grid the panel shows: the one the last `render` flushed. Every
    /// saver here has one — a saver that painted pixels directly could not be
    /// mirrored as cells, and would need its own answer rather than an
    /// `Option` here that every caller has to defend against.
    fn grid(&self) -> &Grid;

    /// The grid the web mirror shows, geometry and this frame's cells. The
    /// panel's, unless the saver's panel geometry moves under a fixed mirror
    /// one (the ascii.rest tour) and pays for the second view here — which is
    /// only after `render` while someone is watching, and in `announce`.
    fn mirror(&mut self) -> &Grid {
        self.grid()
    }

    /// Palette the cells' colour indices address, as XRGB8888.
    fn palette(&self) -> &[u32];
}

/// Every saver, in the order the web mirror offers them. ONE table, because a
/// name list beside a dispatch match is a drift the compiler cannot see: a
/// saver added to the match but not the list is unreachable from the picker and
/// silently means ascii, and no test can enumerate a match's arms to catch it.
///
/// `SAVERS[0]` is the fallback for an unrecognised name. This pod is headless on
/// a remote node — a typo in `SAVER` must never crash-loop it.
///
/// Construction is not a trait method: grid geometry is only known after
/// modeset, and `fn new(&Panel) -> Self` is not object-safe. Each saver reads
/// its own env vars in its own constructor, so no central struct enumerates
/// every saver's knobs. Adding a saver is one row here.
/// Named because clippy is right that the bare tuple is a mouthful, and this is
/// the shape every row shares.
type Build = fn(&Panel, u32) -> Box<dyn Saver>;

// A macro only so the ascii.rest rows come from `ascii_rest::each_piece`, the
// one list of ports, instead of a second copy here.
macro_rules! savers {
    ($($(#[$no:ident])? $m:ident::$t:ident),* $(,)?) => {
        const SAVERS: &[(&str, Build)] = &[
        ("ascii", |p, _| Box::new(Fire::ascii(p))),
        ("blocks", |p, _| Box::new(Fire::blocks(p))),
        ("matrix", |p, fps| Box::new(Matrix::new(p, fps))),
        ("toasters", |p, fps| Box::new(Toasters::new(p, fps))),
        ("toasters2", |p, fps| Box::new(Toasters2::new(p, fps))),
        ("toasters3", |p, fps| Box::new(Toasters3::new(p, fps))),
        ("dvd", |p, fps| Box::new(Dvd::new(p, fps))),
        ("lissajous", |p, fps| Box::new(Lissajous::new(p, fps))),
        ("satori", |p, fps| Box::new(Satori::new(p, fps))),
        ("warp", |p, fps| Box::new(Warp::new(p, fps))),
        ("sakura", |p, fps| Box::new(Sakura::new(p, fps))),
        ("fractal", |p, fps| Box::new(Fractal::new(p, fps))),
        ("hypercube", |p, fps| Box::new(Hypercube::new(p, fps))),
        ("moire", |p, fps| Box::new(Moire::new(p, fps))),
        ("rain", |p, fps| Box::new(Rain::new(p, fps))),
        ("worms", |p, fps| Box::new(Worms::new(p, fps))),
        ("confetti", |p, fps| Box::new(Confetti::new(p, fps))),
        ("city", |p, fps| Box::new(City::new(p, fps))),
        ("life", |p, fps| Box::new(Life::new(p, fps))),
        ("doodles", |p, fps| Box::new(Doodles::new(p, fps))),
        ("strings", |p, fps| Box::new(Strings::new(p, fps))),
        ("tactiles", |p, fps| Box::new(Tactiles::new(p, fps))),
        ("pov", |p, fps| Box::new(Pov::new(p, fps))),
        ("podracer", |p, fps| Box::new(Podracer::new(p, fps))),
        ("speeder", |p, fps| Box::new(Speeder::new(p, fps))),
        ("marble", |p, fps| Box::new(Marble::new(p, fps))),
        ("xwing", |p, fps| Box::new(XWing::new(p, fps))),
        ("hardrain", |p, fps| Box::new(HardRain::new(p, fps))),
        ("zot", |p, fps| Box::new(Zot::new(p, fps))),
        ("plasma", |p, fps| Box::new(Plasma::new(p, fps))),
            $((crate::ascii_rest::$m::$t::NAME, Play::<crate::ascii_rest::$m::$t>::build),)*
        ];
    };
}
crate::ascii_rest::each_piece!(savers);

/// How many savers there are, for `Rotate`'s bag. A const because the bag is a
/// fixed-size array: adding a row to the table above resizes it, and no refill
/// ever allocates.
const NSAVERS: usize = SAVERS.len();

/// The name at an index the render loop is holding. Panics on an index no
/// `index_of` produced, which is unreachable: the only writer is `select`.
pub fn name_at(i: usize) -> &'static str {
    SAVERS[i].0
}

/// Names in picker order, for `/meta`.
pub fn names() -> impl Iterator<Item = &'static str> {
    SAVERS.iter().map(|(n, _)| *n)
}

/// Position in `SAVERS`, or None for a name no saver answers to. The one place
/// a user-supplied name is validated — `make` cannot report a bad name.
pub fn index_of(name: &str) -> Option<usize> {
    SAVERS.iter().position(|(n, _)| *n == name)
}

pub fn make(name: &str, panel: &Panel, fps: u32) -> Box<dyn Saver> {
    let (_, build) = index_of(name).map_or(SAVERS[0], |i| SAVERS[i]);
    build(panel, fps)
}

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
struct Rotate {
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
    fn new(now: Instant) -> Self {
        // Seeded off the clock so a restart does not replay the same order.
        // Same trick sakura grows its tree from.
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0x5EED_1234, |d| d.subsec_nanos() ^ d.as_secs() as u32);
        Self::seeded(now, seed ^ std::process::id().wrapping_mul(0x9E37_79B9))
    }

    /// Starts with no interval at all: the first `due` adopts whatever the
    /// mirror holds, which is the `SAVER_ROTATE_SECS` main put there.
    fn seeded(now: Instant, seed: u32) -> Self {
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
    fn due(&mut self, now: Instant, cur: usize, ctl: u64) -> Option<usize> {
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
        if self.left == 0 {
            self.refill(cur)?;
        }
        self.left -= 1;
        Some(self.bag[self.left])
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
    fn restart(&mut self, now: Instant) {
        self.next = now + self.every;
    }
}

/// The frame loop every host runs: the saver, which one the mirror has
/// selected, rotation and the frame budget. A host keeps only its own concerns
/// — DRM maps and dirties, the dump verifies and writes PPMs, the terminal
/// composes and prints — so a dump proves what runs on hardware.
///
/// Every saver is built by the host's `place`, which names the panel it is
/// built for along with it: the DRM and dump hosts have one panel and pass
/// `|n| (panel, make(n, &panel, fps))`, and the terminal sizes the panel to
/// the saver.
pub struct Driver {
    saver: Box<dyn Saver>,
    panel: Panel,
    selected: usize,
    rot: Rotate,
    frame_dur: Duration,
}

impl Driver {
    /// Built from the mirror's selection, not `cfg.saver`: one validated path,
    /// so `make`'s fallback arm is not load-bearing for user input.
    ///
    /// Rotation is built here, not once in main, so a modeset retry gives the
    /// current saver a full turn — and picks up whatever `/rotate` has been set
    /// to since.
    pub fn new(mirror: &Mirror, fps: u32, place: impl Fn(&str) -> (Panel, Box<dyn Saver>)) -> Self {
        let selected = mirror.selected();
        let (panel, saver) = place(name_at(selected));
        let mut d = Self {
            saver,
            panel,
            selected,
            rot: Rotate::new(Instant::now()),
            frame_dur: Duration::from_nanos(1_000_000_000 / u64::from(fps)),
        };
        // Geometry, palette and glyph table are fixed until the saver changes,
        // so the mirror is told once and every frame after it is only cells.
        announce(mirror, d.saver.as_mut(), &d.panel);
        d
    }

    pub fn saver(&self) -> &dyn Saver {
        self.saver.as_ref()
    }

    pub fn panel(&self) -> &Panel {
        &self.panel
    }

    /// Build the selected saver again, for a host whose panel changed under it.
    pub fn rebuild(&mut self, mirror: &Mirror, place: impl Fn(&str) -> (Panel, Box<dyn Saver>)) {
        (self.panel, self.saver) = place(name_at(self.selected));
        // The new grid geometry and palette differ, so this bumps the mirror's
        // epoch and every viewer reconnects onto the new /meta.
        announce(mirror, self.saver.as_mut(), &self.panel);
    }

    /// Swap the saver if the mirror's selection moved, building it once
    /// through `place`. True when it switched, so the caller can log it on the
    /// thread that actually draws — the HTTP thread cannot know whether the
    /// render loop is running or parked in its no-monitor retry.
    ///
    /// Rotation lands here, for the same reason as everything else in this
    /// struct: a dump that rotated by its own rules would prove nothing about
    /// what the panel does. It moves the mirror's selection and then falls
    /// through the ordinary switch below, so an automatic move and a click are
    /// the same event from here down — including the `/meta` the page re-reads.
    pub fn switch(
        &mut self,
        now: Instant,
        mirror: &Mirror,
        place: impl Fn(&str) -> (Panel, Box<dyn Saver>),
    ) -> bool {
        // Two relaxed loads per frame now — the rotation word and the
        // selection — off the same cache line, for the reason below.
        if let Some(i) = self.rot.due(now, self.selected, mirror.rotate_ctl()) {
            mirror.select_at(i);
        }
        // One relaxed load per frame, same as the mirror's viewer count, and
        // free for the same reason: adjacent field, already-hot cache line, a
        // plain load with no barrier. Relaxed is right because the atomic
        // publishes no data — it indexes a const table that has existed since
        // program start. Everything a viewer observes travels through the meta
        // and frame mutexes.
        let want = mirror.selected();
        if want == self.selected {
            return false;
        }
        // A manual pick resets the interval, so clicking a saver buys it a
        // WHOLE turn rather than however little was left of the last one's —
        // being overridden a second after choosing is the infuriating version
        // of this feature. It does not PAUSE rotation: a pause needs a resume,
        // which is a second knob and a page that has to show which mode it is
        // in, to save someone setting the interval to 0 in the deployment.
        self.rot.restart(now);
        self.selected = want;
        self.rebuild(mirror, place);
        true
    }

    /// Draw one frame into `buf`, then hand the mirror its cells — after the
    /// flush, so they are the frame that just went to the panel. One atomic
    /// load with nobody watching; see mirror.rs for why this can never make the
    /// display wait.
    #[inline]
    pub fn frame(&mut self, buf: &mut [u32], mirror: &Mirror) -> Damage {
        let d = frame(self.saver.as_mut(), buf, &self.panel);
        if mirror.watched() {
            publish(mirror, self.saver.as_mut());
        }
        d
    }

    /// Sleep out what is left of the frame budget, or count the overrun —
    /// the branch that has to be right before anyone raises `SAVER_FPS`
    /// against the pod's 500m CFS quota.
    pub fn pace(&self, mirror: &Mirror, t0: Instant) {
        match self.frame_dur.checked_sub(t0.elapsed()) {
            Some(rem) => std::thread::sleep(rem),
            // The panel missed its rate. COUNTED, not logged: a log line per
            // frame at 30fps is its own outage. Read it from /stat.
            None => mirror.overran(),
        }
    }
}

/// Tell the mirror what this saver draws through. Every construction of a saver
/// is followed by one of these — a viewer holding the previous saver's geometry
/// and palette would mis-draw every cell.
pub fn announce(mirror: &Mirror, s: &mut dyn Saver, panel: &Panel) {
    // Copied because `mirror()` holds the saver mutably; once per switch.
    let pal = s.palette().to_vec();
    let name = s.name();
    mirror.describe(name, s.mirror(), panel, &pal);
}

/// This frame's cells to the mirror, through the same `mirror()` `announce`
/// described, so the geometry and the cells cannot disagree.
pub fn publish(mirror: &Mirror, s: &mut dyn Saver) {
    mirror.publish(s.mirror().cells());
}

/// The per-frame body, shared verbatim by the DRM path and the dump path. A
/// dump that used its own loop would prove nothing about what runs on hardware.
#[inline]
pub fn frame(saver: &mut dyn Saver, buf: &mut [u32], panel: &Panel) -> Damage {
    let mut s = Surface::new(buf, panel);
    saver.render(&mut s);
    s.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A control word for `secs`, built the way the HTTP thread builds it —
    /// the packing is the mirror's and no test gets to hand-roll it.
    fn ctl(secs: u64) -> u64 {
        let m = Mirror::new(15);
        m.set_rotate_secs(secs);
        m.rotate_ctl()
    }

    /// One table makes NAMES-vs-make drift impossible, so what is left to check
    /// is a copy-paste inside a row: a name paired with the constructor for a
    /// different saver. That still renders the wrong thing for a valid name.
    /// Small panel, because only the dispatch is under test.
    #[test]
    fn every_row_builds_the_saver_it_names() {
        let panel = Panel::new(128, 128, 128);
        for (i, name) in names().enumerate() {
            assert_eq!(index_of(name), Some(i));
            assert_eq!(name_at(i), name);
            assert_eq!(make(name, &panel, 30).name(), name);
        }
        assert_eq!(index_of("nope"), None);
        // An unrecognised name must land on the first row, not panic.
        assert_eq!(make("nope", &panel, 30).name(), name_at(0));
    }

    /// The interval is the whole feature: a rotation that fires early is a
    /// saver nobody gets to look at, and one that never fires is the knob doing
    /// nothing. `Instant` is handed in rather than read, so this is exact
    /// rather than a sleep that CI can lose a race to.
    #[test]
    fn a_turn_lasts_the_whole_interval_and_then_ends() {
        let t0 = Instant::now();
        let c = ctl(30);
        let mut r = Rotate::seeded(t0, 1);
        assert_eq!(r.due(t0, 0, c), None);
        assert_eq!(r.due(t0 + Duration::from_millis(29_999), 0, c), None);
        assert!(r.due(t0 + Duration::from_secs(30), 0, c).is_some());
        // And the next turn is a full interval from THERE, not from t0.
        assert_eq!(r.due(t0 + Duration::from_secs(59), 0, c), None);
        assert!(r.due(t0 + Duration::from_secs(60), 0, c).is_some());
    }

    /// Zero is the default and must be genuinely off — not a very short
    /// interval, not a rotation on the first frame. A day of frames is well
    /// past any interval this knob accepts.
    #[test]
    fn zero_never_rotates() {
        let t0 = Instant::now();
        let c = ctl(0);
        let mut r = Rotate::seeded(t0, 1);
        for s in [0, 1, 30, 3600, 86_400, 172_800] {
            assert_eq!(r.due(t0 + Duration::from_secs(s), 3, c), None, "at {s}s");
        }
    }

    /// The whole point of `POST /rotate`: a new interval takes effect on the
    /// next frame, and what is on screen gets a full turn at the NEW length
    /// rather than being cut off by what was left of the old one. Re-asking for
    /// the same number restarts it too — that is what the counter in the
    /// control word buys, and a plain seconds compare would not.
    #[test]
    fn setting_the_interval_takes_effect_live_and_restarts_the_turn() {
        let t0 = Instant::now();
        let m = Mirror::new(15);
        m.set_rotate_secs(30);
        let mut r = Rotate::seeded(t0, 3);
        let at = |s: u64| t0 + Duration::from_secs(s);
        assert_eq!(r.due(t0, 0, m.rotate_ctl()), None);

        // 29s into a 30s turn, someone asks for 10s. Nothing at 30 — where the
        // old interval would have fired — and the new turn ends at 39.
        m.set_rotate_secs(10);
        assert_eq!(r.due(at(29), 0, m.rotate_ctl()), None);
        assert_eq!(r.due(at(30), 0, m.rotate_ctl()), None);
        assert_eq!(r.due(at(38), 0, m.rotate_ctl()), None);
        assert!(r.due(at(39), 0, m.rotate_ctl()).is_some());

        // Asking for ten again at 48 is still a restart: 58, not the 49 the
        // turn that started at 39 was heading for.
        m.set_rotate_secs(10);
        assert_eq!(r.due(at(48), 0, m.rotate_ctl()), None);
        assert_eq!(r.due(at(49), 0, m.rotate_ctl()), None);
        assert!(r.due(at(58), 0, m.rotate_ctl()).is_some());

        // And off is off from the next frame, not at the end of this turn.
        m.set_rotate_secs(0);
        assert_eq!(r.due(at(3600), 0, m.rotate_ctl()), None);
    }

    /// The bag's coverage rule, which is the whole reason it is a bag: exactly
    /// one visit to every saver before any of them comes round again. Checked
    /// over several cycles, because a bag that refills wrong is right once.
    /// The boundary between two bags is checked at the same time — it is the
    /// one place a shuffle can show the same saver twice in a row, and the
    /// `assert_ne` below straddles it.
    #[test]
    fn a_bag_shows_every_saver_once_before_any_of_them_again() {
        let t0 = Instant::now();
        let c = ctl(1);
        let mut r = Rotate::seeded(t0, 0x0BA6_5EED);
        assert_eq!(r.due(t0, 0, c), None);
        let mut cur = 0;
        let mut t = 0u64;
        for cycle in 0..4 {
            let mut shown = Vec::new();
            for _ in 0..SAVERS.len() {
                t += 1;
                let next = r.due(t0 + Duration::from_secs(t), cur, c).unwrap();
                assert_ne!(next, cur, "{} twice in a row, cycle {cycle}", name_at(cur));
                shown.push(next);
                cur = next;
            }
            let mut once = shown.clone();
            once.sort_unstable();
            once.dedup();
            // Same length after dedup as before, and as the table: every saver,
            // exactly one apiece. A bag that refills early or drops a row fails
            // here however plausible its order looks.
            assert_eq!(
                (once.len(), shown.len()),
                (SAVERS.len(), SAVERS.len()),
                "cycle {cycle} was not a clean sweep: {shown:?}"
            );
        }
    }

    /// A bag is not a walk, and this is where that is enforced: over many
    /// cycles the distance from one saver to the next takes every non-zero
    /// value. A fixed walk (always +1) and a shuffle too narrow to reach the
    /// far end of the table both satisfy "never repeats" and are both wrong.
    #[test]
    fn rotation_never_picks_the_saver_already_showing() {
        let t0 = Instant::now();
        let c = ctl(1);
        let mut r = Rotate::seeded(t0, 0xC0FF_EE01);
        // The first frame is where the interval is adopted and the clock
        // starts; the rolls under test are the ones after it.
        assert_eq!(r.due(t0, 0, c), None);
        let mut steps = vec![false; SAVERS.len()];
        let mut cur = 0;
        for i in 1..=2000u32 {
            let next = r.due(t0 + Duration::from_secs(i.into()), cur, c).unwrap();
            assert_ne!(next, cur, "repeated {} at roll {i}", name_at(cur));
            assert!(next < SAVERS.len());
            steps[(next + SAVERS.len() - cur) % SAVERS.len()] = true;
            cur = next;
        }
        assert!(!steps[0], "a zero step is the repeat this test is about");
        assert!(
            steps[1..].iter().all(|&s| s),
            "not every distance is reachable: {steps:?}"
        );
    }

    /// A driver on a small panel with a seeded rotation, as every host
    /// builds one.
    fn driver(mirror: &Mirror, t0: Instant, seed: u32) -> (Driver, Panel) {
        let panel = Panel::new(128, 128, 128);
        let mut d = Driver::new(mirror, 30, |n| (panel, make(n, &panel, 30)));
        d.rot = Rotate::seeded(t0, seed);
        (d, panel)
    }

    /// A click must buy a full turn. Through `switch`, because the reset lives
    /// on the path a click takes and not in the timer.
    #[test]
    fn a_manual_pick_restarts_the_interval() {
        let mirror = Mirror::new(15);
        let t0 = Instant::now();
        mirror.set_rotate_secs(30);
        let (mut d, panel) = driver(&mirror, t0, 7);
        let mut at = |secs: u64| {
            let place = |n: &str| (panel, make(n, &panel, 30));
            let switched = d.switch(t0 + Duration::from_secs(secs), &mirror, place);
            (switched, d.saver().name())
        };

        // 29 seconds in, someone picks something. One second of the turn left.
        assert!(mirror.select("dvd"));
        assert_eq!(at(29), (true, "dvd"));
        // The second that was left does not end their turn...
        assert_eq!(at(30), (false, "dvd"));
        // ...and neither does anything short of a full interval from the click.
        assert_eq!(at(58), (false, "dvd"));
        // 29 + 30: now it is up.
        let (switched, name) = at(59);
        assert!(switched);
        assert_ne!(name, "dvd");
    }

    /// End to end through the call every host makes: the timer moves the
    /// MIRROR's selection, so the picker and `/meta` follow the panel, and the
    /// saver that is drawing actually changes.
    #[test]
    fn switch_rotates_the_panel_and_the_mirror_together() {
        let mirror = Mirror::new(15);
        let t0 = Instant::now();
        mirror.set_rotate_secs(5);
        let (mut d, panel) = driver(&mirror, t0, 42);
        let place = |n: &str| (panel, make(n, &panel, 30));
        let first = d.saver().name();

        // Frame zero adopts the interval and starts the clock — see `due`.
        assert!(!d.switch(t0, &mirror, place));
        assert!(!d.switch(t0 + Duration::from_secs(4), &mirror, place));
        assert_eq!(d.saver().name(), first);

        assert!(d.switch(t0 + Duration::from_secs(5), &mirror, place));
        assert_ne!(d.saver().name(), first);
        assert_eq!(name_at(mirror.selected()), d.saver().name());
        assert_eq!(d.selected, mirror.selected());
    }

    /// A switch builds the saver once, for the panel `place` names. The
    /// terminal used to build at a base panel and again at its own, and
    /// whatever the first build rolled was thrown away.
    #[test]
    fn a_switch_builds_once_for_the_panel_place_names() {
        let mirror = Mirror::new(15);
        let t0 = Instant::now();
        let builds = std::cell::Cell::new(0);
        let big = Panel::new(256, 192, 256);
        let place = |n: &str| {
            builds.set(builds.get() + 1);
            (big, make(n, &big, 30))
        };
        let mut d = Driver::new(&mirror, 30, place);
        assert_eq!(builds.get(), 1);
        assert!(mirror.select("dvd"));
        assert!(d.switch(t0, &mirror, place));
        assert_eq!(builds.get(), 2);
        assert_eq!((d.panel().w, d.panel().h), (256, 192));
    }

    /// The whole point of the overrun counter: a frame that ran past its
    /// budget is counted, and one that did not is not. CI has no card, so the
    /// render loop itself never runs here — this branch is the testable half.
    #[test]
    fn a_late_frame_is_counted_and_an_early_one_is_not() {
        let m = Mirror::new(15);
        let (mut d, _) = driver(&m, Instant::now(), 1);
        d.frame_dur = Duration::from_millis(20);

        // Frame that took longer than the budget: nothing left to sleep.
        let late = Instant::now()
            .checked_sub(Duration::from_millis(50))
            .unwrap();
        d.pace(&m, late);
        assert_eq!(m.overruns(), 1);
        d.pace(&m, late);
        assert_eq!(m.overruns(), 2);

        // Frame that finished inside the budget: sleeps out the remainder and
        // counts nothing. The elapsed check is what fails if the arms are
        // swapped — a swapped `pace` returns instantly here.
        let t0 = Instant::now();
        d.pace(&m, t0);
        assert_eq!(m.overruns(), 2);
        assert!(
            t0.elapsed() >= d.frame_dur,
            "did not sleep: {:?}",
            t0.elapsed()
        );
    }
}
