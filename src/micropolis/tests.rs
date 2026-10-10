use std::time::{Duration, Instant};

use super::mayor::{Hands, Mayor, Tool, NEED_BULLDOZE, OK};
use super::*;
use crate::dump::verify;
use crate::saver;
use crate::testalloc::allocs_during;

const CHURCH1: u16 = 960;
const ROAD_WIRE: u16 = 77;
const WIRE: u16 = 208;
const COAL: u16 = 750;
const RESBASE: u16 = 240;

const SHAPES: [(usize, usize, usize); 5] = [
    (1920, 1080, 180),
    (1920, 1080, 100),
    (1080, 1920, 100),
    (1280, 400, 100),
    (1024, 768, 100),
];

fn want() -> Want {
    Want {
        seed: 7,
        year_secs: 1,
        city_mins: 0,
        disaster_mins: 0,
        bundled_pct: 0,
    }
}

/// A saver that never claims the engine, for drawing maps the test sets.
fn offline(w: usize, h: usize, aspect: usize) -> (Panel, Micropolis) {
    let p = Panel::new(w, h, w);
    let mut m = Micropolis::build(&p, 30, aspect, 70, 6.0, true, want());
    m.claim.want = None;
    (p, m)
}

/// The map covers the panel at every shape and zoom, a tile is square on
/// the glass, and the whole map in view leaves at most a tile to pan.
#[test]
fn every_shape_is_filled_with_square_tiles() {
    for (w, h, aspect) in SHAPES {
        for pct in [20, 70, 100] {
            let (tw, th) = layout(&Panel::new(w, h, w), aspect, pct);
            let case = format!("{w}x{h}@{aspect} {pct}%: {tw}x{th}");
            assert!(W as usize * tw >= w && H as usize * th >= h, "{case}: bars");
            let glass_w = (th * 100) as f32 / aspect as f32;
            assert!((glass_w - tw as f32).abs() <= 1.0, "{case}: not square");
            if pct == 100 {
                let spare_x = W as usize * tw - w;
                let spare_y = H as usize * th - h;
                // Whole pixels per tile: up to one spare pixel per tile.
                assert!(
                    spare_x < W as usize || spare_y < H as usize,
                    "{case}: zoomed in at 100%"
                );
            }
        }
    }
}

fn random_map(m: &mut Micropolis, seed: u32) {
    let mut rng = seed;
    for v in &mut m.map {
        *v = (crate::next_rand(&mut rng) % COUNT as u32) as u16;
    }
}

/// Frame 0 paints every pixel; a changed tile redraws only itself; a still
/// map reports nothing; a camera move repaints; the overlay follows its
/// text. Every pixel that changed was reported, checked against a junk
/// "panel" that only ever receives the reported rects.
#[test]
fn frames_report_exactly_what_they_draw() {
    for (w, h, aspect) in SHAPES {
        let (p, mut m) = offline(w, h, aspect);
        let case = format!("{w}x{h}@{aspect}");
        random_map(&mut m, 3);
        m.pan = 0.0;
        let mut buf = vec![0u32; p.buf_len()];
        let mut hw = vec![0xDEAD_BEEF; p.buf_len()];
        for step in 0..6 {
            match step {
                2 => {
                    let (xs, ys) = m.visible(m.cam_drawn.unwrap());
                    let i = (xs.start + 1) as usize * H as usize + ys.start as usize + 1;
                    m.map[i] = (m.map[i] + 1) % COUNT as u16;
                }
                4 => m.cam.0 += 3.0,
                5 => {
                    m.stats.pop = 123_456;
                    m.compose();
                }
                _ => {}
            }
            let before = buf.clone();
            let mut dmg = None;
            let n = allocs_during(|| dmg = Some(saver::frame(&mut m, &mut buf, &p)));
            if step > 0 {
                assert_eq!(n, 0, "{case} step {step}: render allocated");
            }
            let dmg = dmg.expect("rendered");
            verify(&before, &buf, &dmg, &p, step).unwrap();
            for r in dmg.runs() {
                for y in usize::from(r.y0)..usize::from(r.y1) {
                    let row = y * w + usize::from(r.x0)..y * w + usize::from(r.x1);
                    hw[row.clone()].copy_from_slice(&buf[row]);
                }
            }
            assert!(hw == buf, "{case} step {step}: drawn but never reported");
            match step {
                1 | 3 => assert!(dmg.is_empty(), "{case} step {step}: {:?}", dmg.runs()),
                2 => assert!(dmg.px() <= m.tw * m.th * 2, "{case}: {:?}", dmg.runs()),
                4 => assert_eq!(dmg.px(), w * h, "{case}: a move repaints all"),
                _ => {}
            }
        }
        let (hx, hy, hw_, hh) = m.hud_rect;
        assert!(
            hx + hw_ <= w && hy + hh <= h && hw_ > 0,
            "{case}: overlay off the panel"
        );
    }
}

/// Pixels are the atlas: a visible tile's top-left pixel is its tile's.
#[test]
fn tiles_land_where_the_camera_says() {
    for (w, h, aspect) in SHAPES {
        let (p, mut m) = offline(w, h, aspect);
        m.hud = false;
        random_map(&mut m, 9);
        let mut buf = vec![0u32; p.buf_len()];
        saver::frame(&mut m, &mut buf, &p);
        let cam = m.cam_drawn.unwrap();
        let (xs, ys) = m.visible(cam);
        for x in xs.clone().skip(1).step_by(7) {
            for y in ys.clone().skip(1).step_by(5) {
                let (px, py) = (x * m.tw as i32 - cam.0, y * m.th as i32 - cam.1);
                if px >= w as i32 || py >= h as i32 {
                    continue;
                }
                let t = tiles::shown(m.map[(x * H + y) as usize], true) as usize;
                let want = m.atlas[t * m.tw * m.th];
                assert_eq!(
                    buf[py as usize * w + px as usize],
                    want,
                    "{w}x{h}: tile {x},{y}"
                );
            }
        }
    }
}

#[test]
fn the_tile_sheet_parses() {
    let t = tiles::tiles();
    assert_eq!(t.pix.len(), COUNT * 256);
    assert_eq!(t.pal.len(), 14);
    assert!(t.pix.iter().all(|&i| (i as usize) < t.pal.len()));
    assert_eq!(
        tiles::shown(CHURCH1 | tiles::ZONEBIT | tiles::PWRBIT, false),
        414 + 4
    );
    assert_eq!(tiles::shown(tiles::FREEZ | tiles::ZONEBIT, true), 827);
    for (name, bytes) in cities::CITIES {
        assert_eq!(bytes.len(), 27120, "{name}");
    }
}

#[test]
fn thousands() {
    let s = |n| Thousands(n).to_string();
    assert_eq!(s(0), "0");
    assert_eq!(s(999), "999");
    assert_eq!(s(1000), "1,000");
    assert_eq!(s(1_234_567), "1,234,567");
    assert_eq!(s(-20_500), "-20,500");
}

/// Builds what the mayor asks for straight onto a map, as the engine
/// would, without the simulation.
struct Fake {
    map: Vec<u16>,
    built: Vec<Tool>,
}

impl Fake {
    fn at(&mut self, x: i32, y: i32) -> &mut u16 {
        &mut self.map[(x * H + y) as usize]
    }

    fn plants(&self) -> i32 {
        self.map
            .iter()
            .filter(|&&v| v & tiles::ZONEBIT != 0 && v & tiles::LOMASK == 750)
            .count() as i32
    }
}

impl Hands for Fake {
    fn tool(&mut self, tool: Tool, x: i32, y: i32) -> i32 {
        let clear = |v: u16| tiles::is_clear(v & tiles::LOMASK);
        match tool {
            Tool::Road => {
                let v = *self.at(x, y);
                *self.at(x, y) = if (208..=209).contains(&(v & tiles::LOMASK)) {
                    ROAD_WIRE | tiles::CONDBIT
                } else {
                    66
                };
            }
            Tool::Wire => {
                let v = *self.at(x, y) & tiles::LOMASK;
                *self.at(x, y) = if (66..=67).contains(&v) {
                    ROAD_WIRE
                } else {
                    WIRE
                } | tiles::CONDBIT;
            }
            Tool::Park => *self.at(x, y) = 40,
            Tool::Bulldozer => *self.at(x, y) = 0,
            _ => {
                let n = tool.size();
                let foot: Vec<(i32, i32)> = (x - 1..x - 1 + n)
                    .flat_map(|px| (y - 1..y - 1 + n).map(move |py| (px, py)))
                    .collect();
                if foot
                    .iter()
                    .any(|&(px, py)| !clear(self.map[(px * H + py) as usize]))
                {
                    return NEED_BULLDOZE;
                }
                let (edge, centre) = match tool {
                    Tool::Res => (240, 244),
                    Tool::Com => (423, 427),
                    Tool::Ind => (612, 616),
                    Tool::Coal => (745, 750),
                    _ => (770, 774),
                };
                for (px, py) in foot {
                    *self.at(px, py) = edge | tiles::CONDBIT;
                }
                *self.at(x, y) = centre | tiles::CONDBIT | tiles::ZONEBIT;
                self.built.push(tool);
            }
        }
        OK
    }
}

/// On empty land the mayor's first building is a plant, then it zones all
/// three kinds, every zone with a road beside it and a wire to the plant.
#[test]
fn the_mayor_powers_first_and_connects_everything() {
    let mut fake = Fake {
        map: vec![0; CELLS],
        built: Vec::new(),
    };
    let mut mayor = Mayor::new(&fake.map, 5);
    let mut s = Stats {
        funds: 20_000,
        res_valve: 1500,
        com_valve: 900,
        ind_valve: 1200,
        ..Stats::default()
    };
    for _ in 0..4000 {
        s.coal = fake.plants();
        let map = fake.map.clone();
        mayor.act(&map, &s, &mut fake);
    }
    assert_eq!(fake.built.first(), Some(&Tool::Coal), "{:?}", fake.built);
    for t in [Tool::Res, Tool::Com, Tool::Ind] {
        assert!(fake.built.contains(&t), "no {t:?} in {:?}", fake.built);
    }
    let r = power::route(&fake.map, |_| Some(0));
    assert!(r.is_none(), "a zone has no power: wire wanted at {r:?}");
    let tile = |x: i32, y: i32| fake.map[(x * H + y) as usize] & tiles::LOMASK;
    for x in 0..W {
        for y in 0..H {
            if fake.map[(x * H + y) as usize] & tiles::ZONEBIT == 0 || tile(x, y) == 750 {
                continue;
            }
            let ring = (x - 2..=x + 2)
                .flat_map(|px| [(px, y - 2), (px, y + 2)])
                .chain((y - 1..=y + 1).flat_map(|py| [(x - 2, py), (x + 2, py)]));
            let roads = ring
                .filter(|&(px, py)| tiles::is_road(tile(px, py)))
                .count();
            assert!(roads > 0, "zone at {x},{y} has no road");
        }
    }
}

/// A route from a plant to a zone a road away crosses the road.
#[test]
fn power_routes_over_a_road() {
    let mut map = vec![0u16; CELLS];
    let set = |map: &mut Vec<u16>, x: i32, y: i32, v: u16| map[(x * H + y) as usize] = v;
    for x in 10..14 {
        for y in 10..14 {
            set(&mut map, x, y, COAL | tiles::CONDBIT);
        }
    }
    for y in 0..H {
        set(&mut map, 20, y, 67);
    }
    for x in 25..28 {
        for y in 10..13 {
            set(&mut map, x, y, RESBASE | tiles::CONDBIT);
        }
    }
    set(
        &mut map,
        26,
        11,
        tiles::FREEZ | tiles::CONDBIT | tiles::ZONEBIT,
    );
    let path = power::route(&map, |_| Some(0)).expect("a route");
    assert!(path.contains(&(20 * H as usize + 11)) || path.iter().any(|&i| i / H as usize == 20));
    assert_eq!(path.len(), 25 - 14, "{path:?}");
}

/// Render until `done`, or fail after `secs`: the engine runs on its own
/// clock, so a test can only wait for it.
fn until(
    p: &Panel,
    m: &mut Micropolis,
    buf: &mut [u32],
    secs: u64,
    what: &str,
    done: impl Fn(&Micropolis) -> bool,
) {
    let t0 = Instant::now();
    while !done(m) {
        assert!(
            t0.elapsed() < Duration::from_secs(secs),
            "timed out waiting for {what}"
        );
        saver::frame(m, buf, p);
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The simulator is one per process and a fatal error ends it for good, so
/// everything that needs it runs here, in order, with the error last.
#[test]
fn the_engine_grows_a_city_and_survives_an_error() {
    let p = Panel::new(640, 400, 640);
    let mut m = Micropolis::build(&p, 30, 100, 70, 6.0, true, want());
    let mut buf = vec![0u32; p.buf_len()];
    until(&p, &mut m, &mut buf, 10, "a first map", |m| m.seq > 0);
    until(&p, &mut m, &mut buf, 60, "people", |m| m.stats.pop > 0);
    let built = |m: &Micropolis| {
        m.map
            .iter()
            .filter(|&&v| tiles::is_road(v & tiles::LOMASK))
            .count()
    };
    assert!(built(&m) > 20, "the mayor laid {} road tiles", built(&m));
    let n = allocs_during(|| {
        for _ in 0..30 {
            saver::frame(&mut m, &mut buf, &p);
            std::thread::sleep(Duration::from_millis(5));
        }
    });
    assert_eq!(n, 0, "render allocated");

    Engine::get().fault();
    let t0 = Instant::now();
    while !Engine::get().dead() {
        assert!(
            t0.elapsed() < Duration::from_secs(5),
            "the fault never landed"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    // The frame loop carries on over the frozen city and says so.
    m.compose();
    saver::frame(&mut m, &mut buf, &p);
    let text = std::str::from_utf8(&m.text[..m.text_len]).unwrap();
    assert!(text.ends_with("stopped"), "{text}");
}

/// Not a test: `cargo test --release --features micropolis
/// micropolis::tests::bench -- --ignored --nocapture` grows a city at full
/// speed on this thread and prints a line a year. Alone only: it shares the
/// one simulator with the engine thread. `MP_SEED`, `MP_YEARS`, `MP_CITY`
/// (a bundled city's index), `MP_DISASTER` (years between disasters),
/// `MP_OUT` (a directory for whole-map PPMs) and `MP_DEBUG` (what the mayor
/// is waiting to build).
#[test]
#[ignore = "a bench, run alone: it shares the one simulator"]
fn bench() {
    let env = |k: &str| std::env::var(k).ok().and_then(|s| s.parse().ok());
    let (seed, years) = (env("MP_SEED").unwrap_or(5), env("MP_YEARS").unwrap_or(50));
    let out = std::env::var("MP_OUT").ok();
    let t0 = Instant::now();
    let mut n = 0;
    let city = std::env::var("MP_CITY").ok().and_then(|s| s.parse().ok());
    engine::bench(
        seed,
        years,
        city,
        env("MP_DISASTER").unwrap_or(0),
        |s, map, m| {
            n += 1;
            if let (Some(dir), true) = (&out, n % 4 == 0) {
                write_map(&format!("{dir}/map-{}.ppm", s.year), map);
            }
            if s.month < 3 && std::env::var("MP_DEBUG").is_ok() {
                println!("  {}", m.top(map));
            }
            if s.month < 3 {
                println!(
                "{} pop {:>7} funds {:>7} tax {} R{:>5} C{:>5} I{:>5} zones {}/{} coal {} nuc {} police {} fire {} actions {}",
                s.year, s.pop, s.funds, s.tax, s.res_valve, s.com_valve, s.ind_valve,
                s.powered, s.unpowered, s.coal, s.nuclear, s.police, s.fire, m.actions
            );
            }
        },
    );
    println!("{years} years in {:?}", t0.elapsed());
}

/// The whole map at 1:1 as a PPM, for looking at what the bench built.
fn write_map(path: &str, map: &[u16]) {
    let atlas = tiles::atlas(16, 16);
    let (w, h) = (W as usize * 16, H as usize * 16);
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    for py in 0..h {
        for px in 0..w {
            let t = tiles::shown(map[px / 16 * H as usize + py / 16], false) as usize;
            let c = atlas[(t * 16 + py % 16) * 16 + px % 16];
            out.extend([(c >> 16) as u8, (c >> 8) as u8, c as u8]);
        }
    }
    std::fs::write(path, out).unwrap();
}
