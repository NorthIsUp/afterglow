//! Live per-saver knobs: an override layer in front of the environment, and
//! discovery of which knobs a saver has.
//!
//! Every saver reads its knobs through `env_num` / `env_str` in its
//! constructor and NEVER after, so a knob change is a rebuild, not a value the
//! frame loop polls. That keeps this module off the render path entirely: the
//! lock below is taken at construction and by the HTTP thread, nowhere else.
//!
//! There is no central list of knobs to keep in step with the savers.
//! [`discover`] builds the saver once on the calling thread with a recorder
//! switched on, and every `env_num` / `env_str` call it makes reports its key,
//! default and range — the same numbers the clamp-or-default contract uses, so
//! the page validates against exactly what the saver would accept.
//!
//! Overrides live in memory: a restart goes back to the environment.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};

/// Process-wide by design: the render thread's constructor and the HTTP
/// thread's discovery must read the same value.
static OVERRIDES: RwLock<Option<HashMap<String, String>>> = RwLock::new(None);

/// `discover`'s answers by saver name. Which knobs a saver reads can depend
/// on another knob (the tour's switch hides its timings), so any write clears
/// it. Construction is up to ~0.1 s for the heaviest scene here and several
/// times that on the Pi, which is why this exists at all.
static KNOWN: Mutex<Option<HashMap<&'static str, Vec<Knob>>>> = Mutex::new(None);

/// Bumped by every write, so a discovery that raced one is not remembered.
static WRITES: AtomicU64 = AtomicU64::new(0);

thread_local! {
    /// Some only inside [`discover`], and only on that thread, so a saver the
    /// render thread builds records nothing.
    static RECORD: RefCell<Option<Vec<Knob>>> = const { RefCell::new(None) };
}

/// Held by every test that overrides a knob real savers read, or asserts on
/// one: the override map is process-wide and cargo runs tests in parallel.
#[cfg(test)]
pub static SHARED_KNOBS: Mutex<()> = Mutex::new(());

#[cfg(test)]
thread_local! {
    /// Lookups on this thread, for the test that frames make none.
    static LOOKUPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Longest value a string knob accepts. Every string knob is a short name or
/// letter set; anything longer is a mistake.
const MAX_STR: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Num { default: i64, lo: i64, hi: i64 },
    Str { default: String },
}

/// One knob as a constructor asked for it. `key` is the first of the keys it
/// passed — the canonical spelling; later ones are older aliases.
#[derive(Clone, Debug, PartialEq)]
pub struct Knob {
    pub key: &'static str,
    pub kind: Kind,
}

/// The override for any of `keys`, then the environment's, first present
/// wins in each. An override beats every environment alias, so setting the
/// canonical key from the page cannot be shadowed by an old spelling.
pub fn lookup(keys: &[&str]) -> Option<String> {
    #[cfg(test)]
    LOOKUPS.set(LOOKUPS.get() + 1);
    if let Some(map) = OVERRIDES.read().unwrap().as_ref() {
        if let Some(v) = keys.iter().find_map(|k| map.get(*k)) {
            return Some(v.clone());
        }
    }
    keys.iter().find_map(|k| std::env::var(k).ok())
}

/// Note a knob read, if this thread is discovering. One thread-local check
/// per read, and reads happen at construction only.
pub fn record(keys: &[&'static str], kind: Kind) {
    let Some(&key) = keys.first() else {
        return;
    };
    // Process-wide settings (`SAVER_PIXEL_ASPECT`, …) are read by whichever
    // saver happens to be built first and belong to the monitor, not to it.
    if key.starts_with("SAVER_") {
        return;
    }
    RECORD.with_borrow_mut(|r| {
        if let Some(r) = r {
            // A constructor that reads a knob twice lists it once.
            if !r.iter().any(|k| k.key == key) {
                r.push(Knob { key, kind });
            }
        }
    });
}

/// Every knob `build` reads, in the order it reads them.
pub fn discover(build: impl FnOnce()) -> Vec<Knob> {
    RECORD.set(Some(Vec::new()));
    build();
    RECORD.take().unwrap_or_default()
}

/// `discover(build)` for saver `name`, remembered until the next write.
pub fn knobs_of(name: &'static str, build: impl FnOnce()) -> Vec<Knob> {
    if let Some(k) = KNOWN.lock().unwrap().as_ref().and_then(|m| m.get(name)) {
        return k.clone();
    }
    let before = WRITES.load(Ordering::Acquire);
    let k = discover(build);
    let mut known = KNOWN.lock().unwrap();
    if WRITES.load(Ordering::Acquire) == before {
        known
            .get_or_insert_with(HashMap::new)
            .insert(name, k.clone());
    }
    k
}

fn forget() {
    let mut known = KNOWN.lock().unwrap();
    WRITES.fetch_add(1, Ordering::Release);
    *known = None;
}

/// Why a value was refused. Shown to the page as the 400's message.
#[derive(Debug, PartialEq)]
pub enum Refused {
    NotANumber,
    OutOfRange { lo: i64, hi: i64 },
    TooLong,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotANumber => write!(f, "not a whole number"),
            Self::OutOfRange { lo, hi } => write!(f, "must be {lo}..={hi}"),
            Self::TooLong => write!(f, "longer than {MAX_STR} characters"),
        }
    }
}

/// Store `value` for `knob`, or refuse it and change nothing. The same test
/// `env_num` applies — the difference is that a bad environment value falls
/// back quietly, and a bad value from the page is a 400 the person sees.
pub fn set(knob: &Knob, value: &str) -> Result<(), Refused> {
    match &knob.kind {
        Kind::Num { lo, hi, .. } => {
            let v: i64 = value.parse().map_err(|_| Refused::NotANumber)?;
            if v < *lo || v > *hi {
                return Err(Refused::OutOfRange { lo: *lo, hi: *hi });
            }
        }
        Kind::Str { .. } if value.chars().count() > MAX_STR => return Err(Refused::TooLong),
        Kind::Str { .. } => {}
    }
    OVERRIDES
        .write()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(knob.key.to_string(), value.to_string());
    forget();
    Ok(())
}

/// Drop the override, back to the environment or the default.
pub fn reset(key: &str) {
    if let Some(map) = OVERRIDES.write().unwrap().as_mut() {
        map.remove(key);
    }
    forget();
}

pub fn is_overridden(key: &str) -> bool {
    OVERRIDES
        .read()
        .unwrap()
        .as_ref()
        .is_some_and(|m| m.contains_key(key))
}

/// What the saver would get for `knob` if it were built now — `env_num`'s
/// fallback included, so a garbled environment value shows as the default the
/// saver really uses.
pub fn effective(knob: &Knob) -> String {
    match &knob.kind {
        Kind::Num { default, lo, hi } => {
            crate::env_num(&[knob.key], *default, *lo, *hi).to_string()
        }
        Kind::Str { default } => crate::env_str(&[knob.key], default),
    }
}

/// `TOASTER_TOAST_PCT` -> `toast pct`, given the saver's other keys: the
/// longest underscore-ended prefix they all share is the saver's own and says
/// nothing — unless that would leave a label of a letter or two. A saver with
/// one knob keeps all but its first word.
pub fn label(key: &str, all: &[Knob]) -> String {
    let mut cut = key.find('_').map_or(0, |i| i + 1);
    if all.len() > 1 {
        let first = all[0].key;
        let shared = all.iter().fold(first.len(), |n, k| {
            first
                .bytes()
                .zip(k.key.bytes())
                .take(n)
                .take_while(|(a, b)| a == b)
                .count()
        });
        cut = first[..shared].rfind('_').map_or(0, |i| i + 1);
        // `MATRIX_CELL_W` and `_H` share `MATRIX_CELL_`, which would leave
        // `w` and `h`: a label that short has lost a word it needed.
        if cut > 0 && all.iter().any(|k| k.key.len() - cut <= 2) {
            cut = first[..cut - 1].rfind('_').map_or(0, |i| i + 1);
        }
    }
    key.get(cut..)
        .filter(|s| !s.is_empty())
        .unwrap_or(key)
        .to_lowercase()
        .replace('_', " ")
}

/// One line for the knobs whose name does not say enough.
pub fn help(key: &str) -> Option<&'static str> {
    Some(match key {
        "ASCII_REST_TOUR" => "the slow camera: drifts into detail, pulls back now and then",
        "ASCII_REST_TOUR_SHOT_SECS" => "seconds a shot takes, give or take; slow moves end sooner",
        "ASCII_REST_TOUR_CUTS" => "cut between framings and drift, instead of one long move",
        "ASCII_REST_TOUR_MAX_ZOOM_PCT" => "closest zoom, per cent of the cover view's cell",
        "ASCII_REST_TITLE" => "the scene's name in a corner",
        "TV_STATIC_COLOR" => "the set in colour: bars, cabinet and dial; off is upstream's one ink",
        k if k.ends_with("_SEED") => "0 rolls a new one every build; anything else pins it",
        k if k.ends_with("_CELL_W") || k.ends_with("_CELL_H") || k.ends_with("_CELL") => {
            "cell size in framebuffer pixels"
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{env_num, env_str};

    // Each test uses keys no other test or saver reads, because the override
    // map is process-wide and cargo runs tests in parallel.

    fn num(key: &'static str, default: i64, lo: i64, hi: i64) -> Knob {
        Knob {
            key,
            kind: Kind::Num { default, lo, hi },
        }
    }

    /// Against `PATH` rather than a variable set here: `set_var` races every
    /// other test thread's `getenv`, and `PATH` is set wherever cargo runs.
    #[test]
    fn an_override_beats_the_environment_and_every_alias() {
        let path = std::env::var("PATH").unwrap();
        let keys = ["CFGT_A_NEW", "PATH"];
        let k = Knob {
            key: "CFGT_A_NEW",
            kind: Kind::Str {
                default: "d".into(),
            },
        };
        assert_eq!(env_str(&keys, "d"), path);
        set(&k, "mine").unwrap();
        assert_eq!(env_str(&keys, "d"), "mine");
        reset("CFGT_A_NEW");
        assert_eq!(env_str(&keys, "d"), path);
    }

    #[test]
    fn out_of_range_and_garbage_are_refused_and_change_nothing() {
        let k = num("CFGT_B", 5, 1, 60);
        set(&k, "9").unwrap();
        for bad in ["0", "61", "-1", "5.5", "5x", "", " 9"] {
            assert!(set(&k, bad).is_err(), "{bad:?} accepted");
            assert_eq!(env_num(&["CFGT_B"], 5, 1, 60), 9, "{bad:?} changed it");
        }
        assert_eq!(set(&k, "61"), Err(Refused::OutOfRange { lo: 1, hi: 60 }));
        assert_eq!(effective(&k), "9");
        assert!(is_overridden("CFGT_B"));
        reset("CFGT_B");
        assert!(!is_overridden("CFGT_B"));
        assert_eq!(effective(&k), "5");
    }

    #[test]
    fn string_knobs_take_any_short_value() {
        let k = Knob {
            key: "CFGT_C",
            kind: Kind::Str {
                default: "sc".into(),
            },
        };
        set(&k, "cr").unwrap();
        assert_eq!(env_str(&["CFGT_C"], "sc"), "cr");
        assert_eq!(set(&k, &"x".repeat(65)), Err(Refused::TooLong));
        assert_eq!(env_str(&["CFGT_C"], "sc"), "cr");
        reset("CFGT_C");
    }

    #[test]
    fn discovery_records_what_a_build_reads_and_nothing_else() {
        let got = discover(|| {
            let _ = env_num(&["CFGT_D_ONE", "CFGT_D_ALIAS"], 3, 1, 9);
            let _ = env_str(&["CFGT_D_TWO"], "x");
            let _ = env_num(&["CFGT_D_ONE"], 3, 1, 9);
            let _ = env_num(&["SAVER_CFGT_GLOBAL"], 1, 0, 1);
        });
        assert_eq!(
            got,
            [
                num("CFGT_D_ONE", 3, 1, 9),
                Knob {
                    key: "CFGT_D_TWO",
                    kind: Kind::Str {
                        default: "x".into()
                    }
                }
            ]
        );
        // Off again afterwards: a build outside `discover` records nothing.
        assert_eq!(discover(|| {}), []);
        let _ = env_num(&["CFGT_D_ONE"], 3, 1, 9);
        assert!(RECORD.with_borrow(Option::is_none));
    }

    /// Discovery on the real table: a saver's own knobs, with the ranges its
    /// constructor clamps to.
    #[test]
    fn discovery_finds_a_real_savers_knobs() {
        let _knobs = SHARED_KNOBS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let panel = crate::surface::Panel::new(128, 128, 128);
        let toaster = discover(|| {
            crate::saver::make("toasters", &panel, 30);
        });
        assert!(
            toaster.contains(&num("TOASTER_DENSITY", 4, 1, 60)),
            "{toaster:?}"
        );
        assert!(toaster.iter().all(|k| k.key.starts_with("TOASTER_")));

        // The tour's timings only exist with it on.
        set(&num("ASCII_REST_TOUR", 0, 0, 1), "1").unwrap();
        let scene = discover(|| {
            crate::saver::make("night-coast", &panel, 30);
        });
        reset("ASCII_REST_TOUR");
        let keys: Vec<_> = scene.iter().map(|k| k.key).collect();
        for k in [
            "ASCII_REST_TITLE",
            "ASCII_REST_TOUR",
            "ASCII_REST_TOUR_SHOT_SECS",
            "ASCII_REST_TOUR_CUTS",
            "ASCII_REST_TOUR_MAX_ZOOM_PCT",
        ] {
            assert!(keys.contains(&k), "{k} not in {keys:?}");
        }
    }

    /// The contract that keeps the override lock off the render path: every
    /// saver reads its knobs while it is built and never again. A saver that
    /// read one per frame would take a lock and walk the environment 30 times
    /// a second, and would pick up a change without the rebuild the page
    /// relies on to reset it.
    #[test]
    fn no_saver_reads_a_knob_after_it_is_built() {
        let panel = crate::surface::Panel::new(128, 128, 128);
        let mut buf = vec![0u32; panel.buf_len()];
        for name in crate::saver::names() {
            let mut s = crate::saver::make(name, &panel, 30);
            let before = LOOKUPS.get();
            for _ in 0..45 {
                crate::saver::frame(s.as_mut(), &mut buf, &panel);
                let _ = s.mirror();
            }
            assert_eq!(LOOKUPS.get(), before, "{name} read a knob while drawing");
        }
    }

    #[test]
    fn labels_drop_the_savers_own_prefix() {
        let toaster = [
            num("TOASTER_CELL_W", 0, 0, 0),
            num("TOASTER_TOAST_PCT", 0, 0, 0),
        ];
        assert_eq!(label("TOASTER_TOAST_PCT", &toaster), "toast pct");
        let scene = [
            num("ASCII_REST_TITLE", 0, 0, 1),
            num("ASCII_REST_TOUR", 0, 0, 1),
            num("ASCII_REST_TOUR_MAX_ZOOM_PCT", 0, 0, 1),
        ];
        assert_eq!(
            label("ASCII_REST_TOUR_MAX_ZOOM_PCT", &scene),
            "tour max zoom pct"
        );
        assert_eq!(label("ASCII_REST_TITLE", &scene), "title");
        assert_eq!(label("FIRE_SCALE", &[num("FIRE_SCALE", 0, 0, 0)]), "scale");
        let matrix = [num("MATRIX_CELL_W", 0, 0, 0), num("MATRIX_CELL_H", 0, 0, 0)];
        assert_eq!(label("MATRIX_CELL_H", &matrix), "cell h");
        // The tour off: only its switch and the title left, both still words.
        let scene = [
            num("ASCII_REST_TOUR", 0, 0, 1),
            num("ASCII_REST_TITLE", 0, 0, 1),
        ];
        assert_eq!(label("ASCII_REST_TOUR", &scene), "tour");
    }
}
