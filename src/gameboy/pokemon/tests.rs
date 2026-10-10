//! The bot against a user's own ROM, read in place from `POKEMON_TEST_ROM`
//! and never copied; every test here skips when it is unset, so CI needs no
//! commercial ROM.

use std::fmt::Write as _;
use std::time::Instant;

use mizu_core::{GameBoy, GameBoyConfig};

use super::super::carts::{load, Pilot, Revision};
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

type Seen = [bool; 256];
type Done = fn(&mut GameBoy, Ram, &Seen) -> bool;

fn badge(gb: &mut GameBoy, r: Ram, b: u8) -> bool {
    gb.peek(r.at(story::BADGES)) & 1 << b != 0
}

/// The story beats in the order a run reaches them, each judged from the
/// game's RAM and the maps seen so far.
const MILESTONES: &[(&str, Done)] = &[
    ("starter", |gb, r, _| gb.peek(r.party_count) > 0),
    ("oak's parcel", |gb, r, _| {
        story::has_item(gb, r, story::OAKS_PARCEL) || story::event(gb, r, story::EVENT_GOT_POKEDEX)
    }),
    ("pokédex", |gb, r, _| {
        story::event(gb, r, story::EVENT_GOT_POKEDEX)
    }),
    ("pewter city", |_, _, seen| seen[0x02]),
    ("boulder badge", |gb, r, _| badge(gb, r, 0)),
    ("mt. moon", |_, _, seen| seen[0x3B]),
    ("cerulean city", |_, _, seen| seen[0x03]),
    ("cascade badge", |gb, r, _| badge(gb, r, 1)),
    ("s.s. ticket", |gb, r, _| {
        story::event(gb, r, story::GOT_SS_TICKET)
    }),
    ("hm01", |gb, r, _| story::event(gb, r, story::GOT_HM01)),
    ("cut learned", |gb, r, _| story::cutter(gb, r).is_some()),
    ("thunder badge", |gb, r, _| badge(gb, r, 2)),
    ("rainbow badge", |gb, r, _| badge(gb, r, 3)),
    ("soul badge", |gb, r, _| badge(gb, r, 4)),
    ("marsh badge", |gb, r, _| badge(gb, r, 5)),
    ("volcano badge", |gb, r, _| badge(gb, r, 6)),
    ("earth badge", |gb, r, _| badge(gb, r, 7)),
    ("hall of fame", |_, _, seen| seen[0x76]),
];

/// Which milestones are done, the current map counted as seen.
fn reached(gb: &mut GameBoy, r: Ram, seen_maps: &mut Seen) -> Vec<bool> {
    seen_maps[usize::from(gb.peek(r.cur_map))] = true;
    MILESTONES
        .iter()
        .map(|(_, done)| done(gb, r, seen_maps))
        .collect()
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
    let r = Ram::of(rev);
    let save_frame: Option<u64> = env("POKEBOT_SAVE_FRAME", "").parse().ok();
    let mut bot = Bot::new(rev, seed, knobs);
    eprintln!("starter {:?}", bot.starter());
    let mut seen_maps = [false; 256];
    let mut done = reached(&mut gb, r, &mut seen_maps);
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
                gb.peek(r.font_loaded),
                gb.peek(r.in_battle),
                gb.peek(r.x),
                gb.peek(r.y),
                gb.peek(r.cur_map)
            );
        }
        gb.set_buttons(b);
        gb.clock_for_frame().unwrap();
        if !save_at.is_empty()
            && save_at == format!("{},{},{}", gb.peek(r.cur_map), gb.peek(r.x), gb.peek(r.y))
        {
            gb.save_state(std::fs::File::create(format!("{dir}/at.state")).unwrap())
                .unwrap();
            eprintln!("saved at {save_at} frame {f}");
            save_at.clear();
        }
        if save_frame == Some(f) {
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
        let now = reached(&mut gb, r, &mut seen_maps);
        let mins = f as f64 / FPS / 60.0;
        for (i, (&n, d)) in now.iter().zip(done.iter_mut()).enumerate() {
            if n && !*d {
                *d = true;
                let line = format!("{:16} {mins:7.1} game-min", MILESTONES[i].0);
                eprintln!("MILESTONE {line}");
                let _ = writeln!(log, "{line}");
                if !dir.is_empty() {
                    let name = MILESTONES[i].0.replace([' ', '\'', '.'], "_");
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
/// waits) every 20 frames from power-on and dumps the screen, the menu and
/// the `POKEBOT_PEEK` addresses (hex, comma-separated) after each.
#[test]
#[ignore = "an explorer, run by hand"]
fn pokebot_explore() {
    let Some((rom, _)) = rom() else { return };
    let mut gb = GameBoy::from_rom(rom, None, GameBoyConfig { is_dmg: true }).unwrap();
    let load = env("POKEBOT_LOAD", "");
    if !load.is_empty() {
        gb.load_state(std::fs::File::open(&load).unwrap()).unwrap();
    }
    let peek: Vec<u16> = env("POKEBOT_PEEK", "")
        .split(',')
        .filter_map(|s| u16::from_str_radix(s, 16).ok())
        .collect();
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
            if !peek.is_empty() {
                eprintln!(
                    "peek {:02x?}",
                    peek.iter().map(|&a| gb.peek(a)).collect::<Vec<_>>()
                );
            }
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
    let mut nav = Nav::new(rev);
    nav.set_cut(env("POKEBOT_CUT", "") == "1");
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

/// The trash cans where pokered's `hidden_events_for VERMILION_GYM` puts
/// them, in its order.
#[test]
fn trash_cans_match_pokered() {
    let pret = [
        (1, 7),
        (1, 9),
        (1, 11),
        (3, 7),
        (3, 9),
        (3, 11),
        (5, 7),
        (5, 9),
        (5, 11),
        (7, 7),
        (7, 9),
        (7, 11),
        (9, 7),
        (9, 9),
        (9, 11),
    ];
    for (i, &at) in pret.iter().enumerate() {
        assert_eq!(story::trash_can(i as u8), at, "can {i}");
    }
}
