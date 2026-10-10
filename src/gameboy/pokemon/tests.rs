//! The bot against a user's own ROM, read in place from `POKEMON_TEST_ROM`
//! and never copied; every test here skips when it is unset, so CI needs no
//! commercial ROM.

use std::fmt::Write as _;
use std::time::Instant;

use mizu_core::{GameBoy, GameBoyConfig};

use super::super::carts::{load, Pilot, Revision};
use super::super::kanto::Kanto;
use super::super::pilot::{A, B, DOWN, LEFT, RIGHT, START, UP};
use super::intro::{Intro, DOOR, PALLET, STEPS};
use super::nav::Nav;
use super::{screen, story, Bot, Knobs, Ram};

const FPS: f64 = 59.73;

fn rom() -> Option<(Vec<u8>, Revision)> {
    let path = std::env::var("POKEMON_TEST_ROM").ok()?;
    let c = load(&path, "").into_iter().next()?;
    let Pilot::Pokemon(rev) = c.pilot else {
        panic!("{path}: not a known Pokémon revision");
    };
    Some((c.rom, rev))
}

fn env(k: &str, d: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| d.into())
}

/// The story beats, judged from the game's RAM alone, in the order a run
/// reaches them.
const MILESTONES: &[&str] = &[
    "starter",
    "oak's parcel",
    "pokédex",
    "pewter city",
    "boulder badge",
    "mt. moon",
    "cerulean city",
    "cascade badge",
    "thunder badge",
    "rainbow badge",
    "soul badge",
    "marsh badge",
    "volcano badge",
    "earth badge",
    "hall of fame",
];

/// Which milestones the game's RAM says are done.
fn reached(gb: &mut GameBoy, r: Ram, seen_maps: &mut [bool; 256]) -> Vec<bool> {
    let map = gb.peek(r.cur_map);
    seen_maps[map as usize] = true;
    let badges = gb.peek(r.at(0xD356));
    let n = gb.peek(r.at(0xD31D)).min(20);
    let parcel = (0..n).any(|i| gb.peek(r.at(0xD31E) + 2 * u16::from(i)) == 0x46);
    let dex = story::event(gb, r, story::EVENT_GOT_POKEDEX);
    let mut out = vec![
        gb.peek(r.party_count) > 0,
        parcel || dex,
        dex,
        seen_maps[0x02],
        badges & 1 != 0,
        seen_maps[0x3B],
        seen_maps[0x03],
    ];
    out.extend((1..8).map(|b| badges & 1 << b != 0));
    out.push(seen_maps[0x76]);
    out
}

/// Not a check: plays `POKEBOT_HOURS` game hours as fast as the core runs
/// and prints the game time each milestone was first reached. Starts from
/// `POKEBOT_LOAD` (a save state) if set; writes a state per milestone to
/// `POKEBOT_STATE_DIR` if set.
#[test]
#[ignore = "a benchmark, run by hand"]
fn pokebot_bench() {
    let Some((rom, rev)) = rom() else {
        eprintln!("POKEMON_TEST_ROM unset; skipping");
        return;
    };
    let hours: f64 = env("POKEBOT_HOURS", "4").parse().unwrap();
    let seed: u32 = env("POKEBOT_SEED", "7").parse().unwrap();
    let dir = env("POKEBOT_STATE_DIR", "");
    let mut gb = GameBoy::from_rom(rom, None, GameBoyConfig { is_dmg: true }).unwrap();
    let load = env("POKEBOT_LOAD", "");
    if !load.is_empty() {
        gb.load_state(std::fs::File::open(&load).unwrap()).unwrap();
    }
    let starter: u32 = env("POKEMON_STARTER", "0").parse().unwrap();
    let knobs = Knobs {
        text_ms: 200,
        starter,
    };
    let mut bot = Bot::new(rev, seed, knobs);
    eprintln!("starter {:?}", bot.starter());
    let mut seen_maps = [false; 256];
    let mut done = reached(&mut gb, Ram::of(rev), &mut seen_maps);
    let frames = (hours * 3600.0 * FPS) as u64;
    let t0 = Instant::now();
    let trace = env("POKEBOT_TRACE", "") == "1";
    let mut save_at = env("POKEBOT_SAVE_AT", "");
    let keytrace: u64 = env("POKEBOT_KEYTRACE", "0").parse().unwrap();
    let shots: Vec<u64> = env("POKEBOT_SHOTS", "")
        .split(',')
        .filter_map(|s| s.parse().ok())
        .collect();
    let mut log = String::new();
    for f in 0..frames {
        let b = bot.buttons(&mut gb, f);
        if keytrace > f && b != 0 {
            eprintln!(
                "{f} keys {b:02x} font {:02x} battle {} at {},{} map {}",
                gb.peek(0xCFC4),
                gb.peek(0xD057),
                gb.peek(0xD362),
                gb.peek(0xD361),
                gb.peek(0xD35E)
            );
        }
        gb.set_buttons(b);
        gb.clock_for_frame().unwrap();
        if !save_at.is_empty()
            && save_at
                == format!(
                    "{},{},{}",
                    gb.peek(0xD35E),
                    gb.peek(0xD362),
                    gb.peek(0xD361)
                )
        {
            gb.save_state(std::fs::File::create(format!("{dir}/at.state")).unwrap())
                .unwrap();
            eprintln!("saved at {save_at} frame {f}");
            save_at.clear();
        }
        if env("POKEBOT_SAVE_FRAME", "x") == f.to_string() {
            gb.save_state(std::fs::File::create(format!("{dir}/frame.state")).unwrap())
                .unwrap();
        }
        if shots.contains(&f) {
            shot(
                &gb,
                &format!("{}/shot-{f}.ppm", env("POKEBOT_SHOT_DIR", ".")),
            );
        }
        if trace && f % 300 == 0 {
            eprintln!(
                "{f} {} menu {:?}\n{}",
                bot.status(&mut gb),
                screen::menu(&mut gb),
                screen::dump(&mut gb)
            );
        }
        if f % 60 != 0 {
            continue;
        }
        let now = reached(&mut gb, Ram::of(rev), &mut seen_maps);
        let mins = f as f64 / FPS / 60.0;
        for (i, (&n, d)) in now.iter().zip(done.iter_mut()).enumerate() {
            if n && !*d {
                *d = true;
                let line = format!("{:16} {mins:7.1} game-min", MILESTONES[i]);
                eprintln!("MILESTONE {line}");
                let _ = writeln!(log, "{line}");
                if !dir.is_empty() {
                    let name = MILESTONES[i].replace([' ', '\'', '.'], "_");
                    let file = std::fs::File::create(format!("{dir}/{name}.state")).unwrap();
                    gb.save_state(file).unwrap();
                }
            }
        }
        if f % (60 * 60 * 5) == 0 {
            eprintln!("{mins:7.1} min {}", bot.status(&mut gb));
        }
        if done.iter().all(|&d| d) {
            break;
        }
    }
    let intro = bot.intro();
    eprintln!("intro passed {:?} failed {:?}", intro.passed, intro.failed);
    eprintln!("ran {hours} game-h in {:.0?}\n{log}", t0.elapsed());
}

/// Not a check: taps `POKEBOT_KEYS` (one letter per tap: a b s u d l r, `.`
/// waits) every 20 frames from power-on and dumps the screen and menu
/// after each.
#[test]
#[ignore = "an explorer, run by hand"]
fn pokebot_explore() {
    let Some((rom, _)) = rom() else { return };
    let mut gb = GameBoy::from_rom(rom, None, GameBoyConfig { is_dmg: true }).unwrap();
    let load = env("POKEBOT_LOAD", "");
    if !load.is_empty() {
        gb.load_state(std::fs::File::open(&load).unwrap()).unwrap();
    }
    let keys = env("POKEBOT_KEYS", "");
    let every: u64 = env("POKEBOT_EVERY", "20").parse().unwrap();
    for (i, k) in keys.bytes().enumerate() {
        let key = match k {
            b'a' => A,
            b'b' => B,
            b's' => START,
            b'u' => UP,
            b'd' => DOWN,
            b'l' => LEFT,
            b'r' => RIGHT,
            _ => 0,
        };
        let hold: u64 = env("POKEBOT_HOLD", "2").parse().unwrap();
        for f in 0..every {
            gb.set_buttons(if f < hold { key } else { 0 });
            gb.clock_for_frame().unwrap();
        }
        if env("POKEBOT_DUMP_ALL", "") == "1" || i + 1 == keys.len() {
            eprintln!(
                "--- {i} {} at {:?} ram {:02x?} menu {:?} opts {:02x}\n{}",
                k as char,
                (gb.peek(0xD35E), gb.peek(0xD362), gb.peek(0xD361)),
                [
                    0xCD6B, 0xD730, 0xD736, 0xCFC4, 0xD057, 0xCFC5, 0xFFD5, 0xFF40, 0xFF41, 0xFF44,
                    0xFFFF, 0xFF0F, 0xFFB0, 0xFF47
                ]
                .map(|a| gb.peek(a)),
                screen::menu(&mut gb),
                gb.peek(0xD355),
                screen::dump(&mut gb)
            );
        }
    }
}

/// Not a check: `POKEBOT_MAP`'s walkable squares (`.`, `#` wall, `W`
/// warp), tileset and warps, and a route from `POKEBOT_FROM` (map,x,y) to
/// `POKEBOT_TO` (map,x,y) if both are set.
#[test]
#[ignore = "a viewer, run by hand"]
fn pokebot_map() {
    let Some((rom, rev)) = rom() else { return };
    let mut nav = Nav::new(Kanto::new(rev), Ram::of(rev));
    let map: u8 = env("POKEBOT_MAP", "0").parse().unwrap();
    let g = nav.grid(&rom, map).unwrap();
    eprintln!(
        "map {map}: {}x{} tileset {} grass {:02x} warps {:?}",
        g.w, g.h, g.tileset, g.grass, g.warps
    );
    for y in 0..g.h {
        let line: String = (0..g.w)
            .map(|x| {
                if g.warps
                    .iter()
                    .any(|w| (usize::from(w.0), usize::from(w.1)) == (x, y))
                {
                    'W'
                } else if g.walkable(x, y) && g.tile[y * g.w + x] == g.grass {
                    ','
                } else if g.walkable(x, y) {
                    '.'
                } else {
                    '#'
                }
            })
            .collect();
        eprintln!("{y:3} {line}");
        if env("POKEBOT_TILES", "") == y.to_string() {
            eprintln!(
                "    {:02x?}",
                (0..g.w).map(|x| g.tile[y * g.w + x]).collect::<Vec<_>>()
            );
        }
    }
    let parse = |k: &str| -> Option<(u8, u8, u8)> {
        let v: Vec<u8> = env(k, "")
            .split(',')
            .filter_map(|s| s.parse().ok())
            .collect();
        (v.len() == 3).then(|| (v[0], v[1], v[2]))
    };
    if let (Some(from), Some(to)) = (parse("POKEBOT_FROM"), parse("POKEBOT_TO")) {
        let t0 = Instant::now();
        let r = nav.route(&rom, from, &move |_, s| s == to);
        eprintln!("route {from:?} -> {to:?}: {r:?} in {:?}", t0.elapsed());
    }
}

/// The last finished frame as a PPM.
fn shot(gb: &GameBoy, path: &str) {
    let mut out = b"P6 160 144 255\n".to_vec();
    for &c in gb.screen_buffer() {
        let ch = |s: u16| (((c >> s) & 31) * 255 / 31) as u8;
        out.extend([ch(0), ch(5), ch(10)]);
    }
    std::fs::write(path, out).unwrap();
}

/// From power-on with blank battery RAM, every step of the intro passes in
/// order, checked in RAM (`intro.rs`), and the run ends in Pallet Town at
/// the house door with both names set, inside a frame bound; ten times,
/// each from a different number of idle frames after power-on, so it does
/// not depend on timing luck. Every Pokémon ROM `POKEMON_TEST_ROM` names
/// (a file or a folder) is run.
#[test]
fn pokemon_intro_passes_every_step() {
    const BOUND: u64 = 60 * 60 * 4;
    let Ok(path) = std::env::var("POKEMON_TEST_ROM") else {
        eprintln!("POKEMON_TEST_ROM unset; skipping");
        return;
    };
    let shots = std::env::var("POKEBOT_SHOT_DIR").ok();
    for cart in load(&path, "") {
        let Pilot::Pokemon(rev) = cart.pilot else {
            continue;
        };
        let r = Ram::of(rev);
        let mut frames = Vec::new();
        for offset in 0..10u64 {
            Intro::forget();
            let mut gb =
                GameBoy::from_rom(cart.rom.clone(), None, GameBoyConfig { is_dmg: true }).unwrap();
            for _ in 0..offset * 37 {
                gb.clock_for_frame().unwrap();
            }
            let mut bot = Bot::new(
                rev,
                7 + offset as u32,
                Knobs {
                    text_ms: 200,
                    starter: 0,
                },
            );
            let mut f = 0;
            while !bot.intro().done && f < BOUND {
                let b = bot.buttons(&mut gb, f);
                gb.set_buttons(b);
                gb.clock_for_frame().unwrap();
                f += 1;
            }
            let intro = bot.intro();
            let case = format!("{} offset {offset}", cart.name);
            for (step, at) in STEPS.iter().zip(intro.passed) {
                assert!(
                    at.is_some(),
                    "{case}: {step:?} never passed; {:?} {:?} at {:?}",
                    intro.passed,
                    intro.failed,
                    (gb.peek(r.cur_map), gb.peek(r.x), gb.peek(r.y))
                );
            }
            assert!(
                intro.passed.windows(2).all(|w| w[0] <= w[1]),
                "{case}: steps out of order {:?}",
                intro.passed
            );
            assert_eq!(intro.failed, None, "{case}");
            assert_eq!(gb.peek(r.cur_map), PALLET, "{case}");
            assert_eq!((gb.peek(r.x), gb.peek(r.y)), DOOR, "{case}");
            for at in [r.player_name, r.rival_name] {
                let name = screen::name(&mut gb, at);
                assert!(name[0].is_ascii_uppercase(), "{case}: name {name:?}");
            }
            assert!(f < BOUND, "{case}: {f} frames");
            frames.push(f);
            if offset == 0 {
                if let Some(dir) = &shots {
                    shot(&gb, &format!("{dir}/door-{rev:?}.ppm"));
                }
            }
        }
        eprintln!(
            "{}: 10/10 passed every step; frames to the door {frames:?}",
            cart.name
        );
    }
}
