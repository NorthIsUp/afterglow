//! The Micropolis engine, its mayor, and the one thread that runs them.
//!
//! The simulator is one C++ object behind `micropolis/afterglow_micropolis.cpp`,
//! touched by exactly one thread: this one, started on first use and parked
//! whenever no `micropolis` saver is showing. The city keeps going across
//! showings. The render thread only ever `try_lock`s the last finished map.
//!
//! An engine fatal error unwinds to the glue's boundary and the call reports
//! failure. The simulator is then abandoned for the life of the process, and
//! the saver keeps showing the last map it had, frozen.

use std::ffi::c_int;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::cities::{self, Name, CITIES};
use super::mayor::{Hands, Mayor, Tool, H, W};
use crate::next_rand;

pub const CELLS: usize = (W * H) as usize;
/// Simulator passes per city year: 16 per city week, 48 weeks.
pub const TICKS_PER_YEAR: u32 = 768;
const FRAME: Duration = Duration::from_millis(33);
/// Tile animation (traffic, smoke, the stadium's game) at this many steps a
/// second, whatever the simulation speed.
const ANIM_HZ: u32 = 8;
/// The mayor acts once per this many passes.
const MAYOR_EVERY: u32 = 2;

/// `MpxStats` in the glue, field for field.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Stats {
    pub pop: c_int,
    pub year: c_int,
    pub month: c_int,
    pub funds: c_int,
    pub res_valve: c_int,
    pub com_valve: c_int,
    pub ind_valve: c_int,
    pub res_cap: c_int,
    pub com_cap: c_int,
    pub ind_cap: c_int,
    pub powered: c_int,
    pub unpowered: c_int,
    pub coal: c_int,
    pub nuclear: c_int,
    pub police: c_int,
    pub fire: c_int,
    pub stadium: c_int,
    pub seaport: c_int,
    pub airport: c_int,
    pub tax: c_int,
    pub crime: c_int,
    pub pollution: c_int,
    pub res_pop: c_int,
    pub com_pop: c_int,
    pub ind_pop: c_int,
    pub city_time: c_int,
    pub score: c_int,
    pub land_value: c_int,
    pub cash_flow: c_int,
    pub roads: c_int,
}

extern "C" {
    fn mpx_init() -> c_int;
    fn mpx_new_city(seed: c_int, bytes: *const u8, len: c_int, funds: c_int) -> c_int;
    fn mpx_step(ticks: c_int) -> c_int;
    fn mpx_animate() -> c_int;
    fn mpx_tool(tool: c_int, x: c_int, y: c_int) -> c_int;
    fn mpx_disaster(kind: c_int) -> c_int;
    fn mpx_set_tax(tax: c_int) -> c_int;
    fn mpx_map() -> *const u16;
    fn mpx_stats(out: *mut Stats) -> c_int;
    fn mpx_fault() -> c_int;
}

/// The last finished map and what the overlay says about it.
pub struct Frame {
    pub map: Box<[u16]>,
    pub stats: Stats,
    pub name: Name,
    pub seq: u32,
}

/// What the showing saver wants. Written by the render thread on a switch,
/// read by the engine thread once a frame.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Want {
    pub seed: u32,
    /// Wall seconds per city year.
    pub year_secs: u32,
    /// Wall minutes before the next city; 0 keeps one forever.
    pub city_mins: u32,
    /// Mean wall minutes between disasters; 0 for none.
    pub disaster_mins: u32,
    /// Chance in a hundred that a new city is a bundled one.
    pub bundled_pct: u32,
}

struct Ctl {
    owner: u64,
    want: Option<Want>,
    fault: bool,
}

pub struct Engine {
    ctl: Mutex<Ctl>,
    wake: Condvar,
    frame: Mutex<Frame>,
    dead: AtomicBool,
}

/// A poisoned lock means a panic on the engine thread, which aborts the
/// process anyway (`panic = "abort"`); in tests, keep going with the data.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Engine {
    pub fn get() -> &'static Self {
        static STARTED: OnceLock<()> = OnceLock::new();
        let e = Self::get_unstarted();
        STARTED.get_or_init(|| {
            std::thread::Builder::new()
                .name("micropolis".into())
                .spawn(|| run(Engine::get()))
                .expect("spawning the micropolis thread");
        });
        e
    }

    fn get_unstarted() -> &'static Self {
        static E: OnceLock<Engine> = OnceLock::new();
        E.get_or_init(|| Engine {
            ctl: Mutex::new(Ctl {
                owner: 0,
                want: None,
                fault: false,
            }),
            wake: Condvar::new(),
            frame: Mutex::new(Frame {
                map: vec![0; CELLS].into_boxed_slice(),
                stats: Stats::default(),
                name: Name::default(),
                seq: 0,
            }),
            dead: AtomicBool::new(false),
        })
    }

    /// Run the engine for saver `id` with `want`. The city carries on.
    pub fn claim(&self, id: u64, want: Want) {
        let mut c = lock(&self.ctl);
        c.owner = id;
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

    /// Copy the map into `dst` if it moved on since `seen`. Never waits: a
    /// map the engine thread is writing is skipped this frame.
    pub fn latest(&self, seen: u32, dst: &mut [u16]) -> Option<(u32, Stats, Name)> {
        let f = self.frame.try_lock().ok()?;
        if f.seq == seen {
            return None;
        }
        dst.copy_from_slice(&f.map);
        Some((f.seq, f.stats, f.name))
    }

    pub fn dead(&self) -> bool {
        self.dead.load(Ordering::Relaxed)
    }

    /// Make the engine hit a fatal error on its next frame.
    #[cfg(test)]
    pub fn fault(&self) {
        lock(&self.ctl).fault = true;
    }
}

pub struct Sim;

impl Hands for Sim {
    fn tool(&mut self, tool: Tool, x: i32, y: i32) -> i32 {
        // SAFETY (every engine call in this file): only this thread enters
        // the engine, and pointers handed in outlive the call.
        unsafe { mpx_tool(tool as c_int, x, y) }
    }
}

/// One city, from its first tick to the next.
struct City {
    mayor: Mayor,
    name: Name,
    born: Instant,
    /// The most people it has had, and the city time it had them.
    peak: (i32, i32),
}

impl City {
    /// Fallen to an eighth of its peak and no better for eight years: a
    /// ghost town is no screensaver, so the next city starts.
    fn ruined(&mut self, s: &Stats) -> bool {
        if s.pop >= self.peak.0 {
            self.peak = (s.pop, s.city_time);
        }
        self.peak.0 > 2000 && s.pop * 8 < self.peak.0 && s.city_time - self.peak.1 > 8 * 48
    }
}

struct Runner<'a> {
    e: &'a Engine,
    rng: u32,
    stats: Stats,
}

impl Runner<'_> {
    fn ok(&self, rc: c_int) -> bool {
        if rc < 0 {
            eprintln!("[screensaver] micropolis: the engine hit an error; the city stops");
            self.e.dead.store(true, Ordering::Relaxed);
        }
        rc >= 0
    }

    fn map() -> &'static [u16] {
        // SAFETY: the map is allocated once, at init, and lives as long as
        // the simulator, which is never freed.
        unsafe { std::slice::from_raw_parts(mpx_map(), CELLS) }
    }

    fn refresh(&mut self) -> bool {
        let rc = unsafe { mpx_stats(&raw mut self.stats) };
        self.ok(rc)
    }

    fn new_city(&mut self, w: &Want) -> Option<City> {
        let seed = next_rand(&mut self.rng);
        let bundled = (next_rand(&mut self.rng) % 100) < w.bundled_pct;
        let (name, rc) = if bundled {
            let (name, bytes) = CITIES[next_rand(&mut self.rng) as usize % CITIES.len()];
            (Name(name, ""), unsafe {
                mpx_new_city(0, bytes.as_ptr(), bytes.len() as c_int, 0)
            })
        } else {
            let name = cities::name(&mut self.rng);
            (name, unsafe {
                mpx_new_city(seed as c_int & 0x7fff_ffff, std::ptr::null(), 0, 20_000)
            })
        };
        if !self.ok(rc) || !self.refresh() {
            return None;
        }
        Some(City {
            mayor: Mayor::new(Self::map(), seed),
            name,
            born: Instant::now(),
            peak: (0, 0),
        })
    }

    fn publish(&mut self, city: &City) {
        if !self.refresh() {
            return;
        }
        let mut f = lock(&self.e.frame);
        f.map.copy_from_slice(Self::map());
        f.stats = self.stats;
        f.name = city.name;
        f.seq = f.seq.wrapping_add(1);
    }
}

fn run(e: &Engine) {
    let mut r = Runner {
        e,
        rng: 1,
        stats: Stats::default(),
    };
    let mut started = false;
    let mut city: Option<City> = None;
    let mut due = 0.0f64;
    let mut anim = 0.0f64;
    let mut ticks: u32 = 0;
    let mut next_disaster: Option<Instant> = None;
    let mut last = Instant::now();
    while !e.dead() {
        let (w, fault) = {
            let mut c = lock(&e.ctl);
            while c.owner == 0 || c.want.is_none() {
                c = e
                    .wake
                    .wait(c)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                last = Instant::now();
            }
            (c.want.expect("checked above"), std::mem::take(&mut c.fault))
        };
        if !started {
            started = true;
            r.rng = w.seed | 1;
            if !r.ok(unsafe { mpx_init() }) {
                break;
            }
        }
        let now = Instant::now();
        let old = city.as_mut().is_some_and(|c| {
            w.city_mins > 0 && now - c.born >= Duration::from_secs(u64::from(w.city_mins) * 60)
                || c.ruined(&r.stats)
        });
        if city.is_none() || old {
            city = r.new_city(&w);
            next_disaster = None;
        }
        let Some(c) = city.as_mut() else { break };
        if fault && !r.ok(unsafe { mpx_fault() }) {
            break;
        }
        let dt = (now - last).as_secs_f64().min(0.25);
        last = now;
        due += dt * f64::from(TICKS_PER_YEAR) / f64::from(w.year_secs.max(1));
        let mut alive = true;
        while due >= 1.0 && alive {
            due -= 1.0;
            alive = r.ok(unsafe { mpx_step(1) });
            ticks = ticks.wrapping_add(1);
            if alive && ticks.is_multiple_of(MAYOR_EVERY) {
                alive = r.refresh();
                c.mayor.act(Runner::map(), &r.stats, &mut Sim);
                if ticks.is_multiple_of(TICKS_PER_YEAR / 4) {
                    alive = r.ok(unsafe { mpx_set_tax(Mayor::tax(&r.stats)) });
                }
            }
        }
        anim += dt * f64::from(ANIM_HZ);
        while anim >= 1.0 && alive {
            anim -= 1.0;
            alive = r.ok(unsafe { mpx_animate() });
        }
        if !alive {
            break;
        }
        if w.disaster_mins > 0 {
            let mean = f64::from(w.disaster_mins) * 60.0;
            let t = next_disaster.get_or_insert_with(|| {
                let u = f64::from(next_rand(&mut r.rng) % 1000 + 1) / 1001.0;
                now + Duration::from_secs_f64(-u.ln() * mean)
            });
            if now >= *t {
                next_disaster = None;
                let kinds = if r.stats.nuclear > 0 { 4 } else { 3 };
                let k = next_rand(&mut r.rng) % kinds;
                if !r.ok(unsafe { mpx_disaster(k as c_int) }) {
                    break;
                }
            }
        }
        let c = city.as_ref().expect("set above");
        r.publish(c);
        let spent = now.elapsed();
        if let Some(rest) = FRAME.checked_sub(spent) {
            std::thread::sleep(rest);
        }
    }
}

/// Runs a city on the calling thread as fast as it goes, logging what the
/// mayor built. Never alongside [`Engine`]: there is one simulator.
#[cfg(test)]
pub fn bench(
    seed: u32,
    years: u32,
    bundled: Option<usize>,
    disaster_years: u32,
    mut log: impl FnMut(&Stats, &[u16], &Mayor),
) {
    let e = Engine::get_unstarted();
    let mut r = Runner {
        e,
        rng: seed | 1,
        stats: Stats::default(),
    };
    assert!(r.ok(unsafe { mpx_init() }));
    let rc = match bundled {
        Some(i) => unsafe { mpx_new_city(0, CITIES[i].1.as_ptr(), CITIES[i].1.len() as c_int, 0) },
        None => unsafe { mpx_new_city(seed as c_int & 0x7fff_ffff, std::ptr::null(), 0, 20_000) },
    };
    assert!(r.ok(rc) && r.refresh());
    let mut mayor = Mayor::new(Runner::map(), seed);
    for t in 1..=years * TICKS_PER_YEAR {
        if disaster_years > 0 && t % (disaster_years * TICKS_PER_YEAR) == 0 {
            let kind = t / (disaster_years * TICKS_PER_YEAR) % 3;
            println!("disaster {kind}");
            assert!(r.ok(unsafe { mpx_disaster(kind as c_int) }));
        }
        assert!(r.ok(unsafe { mpx_step(1) }));
        if t.is_multiple_of(MAYOR_EVERY) {
            assert!(r.refresh());
            mayor.act(Runner::map(), &r.stats, &mut Sim);
        }
        if t.is_multiple_of(TICKS_PER_YEAR / 4) {
            assert!(r.ok(unsafe { mpx_set_tax(Mayor::tax(&r.stats)) }));
            log(&r.stats, Runner::map(), &mayor);
        }
    }
}
