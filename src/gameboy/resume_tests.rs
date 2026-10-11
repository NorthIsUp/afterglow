//! Save states: a cartridge picks up where it left off after a rotation, a
//! saver switch or a restart of the process, and a bad state never stops
//! one booting.

use std::path::PathBuf;
use std::time::Duration;

use mizu_core::GameBoy;

use super::carts::{load, test_roms, Cart, Pilot, Revision};
use super::engine::{self, Carts, Session};
use super::pilot::{A, RIGHT, START};
use super::pokemon::Knobs;
use super::view::Mode;
use super::{ram, Want};
use crate::cart_runner::Deck;
use crate::savestate::Store;

fn want() -> Want {
    Want {
        rom: String::new(),
        sav: String::new(),
        palette: "auto".into(),
        width: 256,
        mode: Mode::Frame,
        rotate: None,
        snap_every: None,
        seed: 7,
        restart: 0,
        pokemon: Knobs {
            text_ms: 200,
            starter: 0,
        },
    }
}

fn bundled(name: &str) -> Cart {
    load("", "")
        .into_iter()
        .find(|c| c.name == name)
        .expect(name)
}

/// A runner over `carts` saving under `dir`, as a new process would start.
fn runner_at(dir: &std::path::Path, carts: Vec<Cart>) -> engine::Runner {
    let mut r = engine::runner(carts);
    r.store = Store::at("gameboy", Some(dir.to_path_buf()));
    r
}

/// A scratch `SAVER_STATE_DIR`, unique per test.
fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("afterglow-gb-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// The buttons for frame `f`: the same whichever machine plays it.
fn script(f: u64) -> u8 {
    match f % 97 {
        0..4 => START,
        10..14 => A,
        _ if f % 300 < 150 => RIGHT,
        _ => 0,
    }
}

/// Each of `n` frames' screen, hashed.
fn frames(gb: &mut GameBoy, from: u64, n: u64) -> Vec<u64> {
    (from..from + n)
        .map(|f| {
            gb.set_buttons(script(f));
            gb.clock_for_frame().unwrap();
            gb.screen_buffer()
                .iter()
                .fold(0xcbf2_9ce4_8422_2325u64, |h, &p| {
                    (h ^ u64::from(p)).wrapping_mul(0x100_0000_01b3)
                })
        })
        .collect()
}

/// A state taken mid-game and loaded into a fresh machine plays on frame
/// for frame as the machine it came from, from the first whole frame.
#[test]
fn a_restored_state_plays_the_same_frames_as_the_original() {
    for name in ["tobu-tobu-girl-deluxe", "rebound"] {
        let cart = bundled(name);
        let mut a = Session::boot(&cart, &want(), 7).unwrap();
        frames(&mut a.gb, 0, 900);
        let mut state = Vec::new();
        assert!(Carts::snap(&mut a, &mut state), "{name}");
        let mut b = Session::boot(&cart, &want(), 9).unwrap();
        Carts::restore(&mut b, &state).unwrap();
        let (on, resumed) = (frames(&mut a.gb, 900, 600), frames(&mut b.gb, 900, 600));
        // The core does not keep its screen, so the first frame's lines
        // drawn before the state was taken are missing.
        let differ = on
            .iter()
            .zip(&resumed)
            .skip(1)
            .filter(|(x, y)| x != y)
            .count();
        assert_eq!(differ, 0, "{name}: {differ} of 599 frames differ");
        assert!(on.windows(2).any(|w| w[0] != w[1]), "{name} never moved");
    }
}

/// Away to the next cartridge and back: the first one resumes, from a
/// save state on disk, and a new process with the same directory does too.
#[test]
fn rotation_away_and_back_resumes() {
    let dir = scratch("rotate");
    let carts = || Carts(vec![bundled("life"), bundled("rebound")]);
    let mut r = runner_at(&dir, Vec::new());
    r.reload(carts, 0);
    let w = want();
    for _ in 0..3 {
        for _ in 0..120 {
            assert!(r.frame(&w));
        }
        r.next();
    }
    assert!(r.frame(&w));
    assert_eq!(r.playing(), "rebound");
    assert_eq!(r.resumes, 2, "life came back, and now rebound");
    assert!(dir
        .join("gameboy")
        .join(format!("{}.state", bundled("life").sha1))
        .exists());

    let mut fresh = runner_at(&dir, Vec::new());
    fresh.reload(carts, 0);
    assert!(fresh.frame(&w));
    assert_eq!((fresh.playing().as_str(), fresh.resumes), ("life", 1));
    let _ = std::fs::remove_dir_all(dir);
}

/// A state that is no longer whole, or not a state at all, is dropped and
/// the cartridge boots from power-on.
#[test]
fn a_corrupt_state_boots_fresh() {
    let dir = scratch("corrupt");
    let cart = bundled("tobu-tobu-girl-deluxe");
    let file = dir.join("gameboy").join(format!("{}.state", cart.sha1));
    let mut r = runner_at(&dir, vec![bundled("tobu-tobu-girl-deluxe")]);
    let w = want();
    for _ in 0..60 {
        assert!(r.frame(&w));
    }
    r.snap();
    let mut b = std::fs::read(&file).unwrap();
    let n = b.len();
    b.truncate(n / 2);
    std::fs::write(&file, b).unwrap();
    let mut r = runner_at(&dir, vec![bundled("tobu-tobu-girl-deluxe")]);
    assert!(r.frame(&w));
    assert_eq!((r.resumes, r.frames()), (0, 1));
    assert!(!file.exists());

    // Whole and checksummed, but the core cannot read it.
    r.store.put(&cart.sha1, Carts::FORMAT, b"not a mizu state");
    let mut r = runner_at(&dir, vec![bundled("tobu-tobu-girl-deluxe")]);
    assert!(r.frame(&w));
    assert_eq!((r.resumes, r.frames()), (0, 1));
    assert!(!file.exists());
    let _ = std::fs::remove_dir_all(dir);
}

/// The page's restart is a fresh start: the playing cartridge's states go.
#[test]
fn a_restart_deletes_the_save_state() {
    let dir = scratch("restart");
    let mut r = runner_at(&dir, Vec::new());
    let w = want();
    r.take(None, &w);
    for _ in 0..120 {
        assert!(r.frame(&w));
    }
    let playing = r.playing();
    let sha1 = load("", "")
        .into_iter()
        .find(|c| c.name == playing)
        .unwrap()
        .sha1;
    r.snap();
    assert!(r.store.get(&sha1, Carts::FORMAT).is_some());
    let restarted = Want {
        restart: 1,
        ..w.clone()
    };
    r.take(Some(&w), &restarted);
    assert!(r.store.get(&sha1, Carts::FORMAT).is_none());
    assert!(r.frame(&restarted));
    assert_eq!((r.resumes, r.frames()), (0, 1));
    let _ = std::fs::remove_dir_all(dir);
}

fn ram_of(cart: &Cart) -> ram::Ram {
    match cart.pilot {
        Pilot::Pokemon(r) => ram::Ram::of(r),
        _ => unreachable!("Pokémon only"),
    }
}

/// Saved with the starter in hand and resumed by a bot that never saw the
/// intro: it walks on to the Pokédex, skipping the intro, with the starter
/// the game holds.
#[test]
fn pokemon_resumed_mid_game_reaches_the_next_milestone() {
    for cart in test_roms() {
        let r = ram_of(&cart);
        let mut a = Session::boot(&cart, &want(), 7).unwrap();
        let mut view = vec![0u16; engine::MAX_W * engine::H];
        let mut f = 0;
        while a.gb.peek(r.party_count) == 0 {
            a.step(&mut view);
            f += 1;
            assert!(f < 60 * 60 * 30, "{}: no starter", cart.name);
        }
        // `wPlayerStarter`, which the resumed bot reads its starter from.
        if matches!(cart.pilot, Pilot::Pokemon(Revision::Red | Revision::Blue)) {
            let starter = a.gb.peek(r.at(0xD717));
            assert!([0x99, 0xB0, 0xB1].contains(&starter), "{starter:02x}");
        }
        let mut state = Vec::new();
        assert!(Carts::snap(&mut a, &mut state));
        drop(a);
        let knobs = Knobs {
            starter: 3,
            ..want().pokemon
        };
        let mut b = Session::boot(
            &cart,
            &Want {
                pokemon: knobs,
                ..want()
            },
            99,
        )
        .unwrap();
        Carts::restore(&mut b, &state).unwrap();
        assert!(!b.fast(), "{}: the intro ran again", cart.name);
        // EVENT_GOT_POKEDEX in wEventFlags.
        let dex = |gb: &mut GameBoy| gb.peek(r.at(0xD747 + 0x25 / 8)) & 1 << (0x25 % 8) != 0;
        let mut g = 0;
        while !dex(&mut b.gb) {
            b.step(&mut view);
            g += 1;
            assert!(g < 60 * 60 * 30, "{}: no Pokédex", cart.name);
        }
        eprintln!(
            "{}: starter at frame {f}; resumed, the Pokédex {:.1} game-min later",
            cart.name,
            g as f64 / 59.73 / 60.0
        );
    }
}

/// A Game Boy state's size and what it costs to take and write, on this
/// machine.
#[test]
#[ignore = "a benchmark, run by hand"]
fn bench_snap() {
    let dir = scratch("bench");
    let cart = test_roms()
        .into_iter()
        .next()
        .unwrap_or_else(|| bundled("rebound"));
    let mut s = Session::boot(&cart, &want(), 7).unwrap();
    let mut view = vec![0u16; engine::MAX_W * engine::H];
    for _ in 0..60 * 60 * 2 {
        s.step(&mut view);
    }
    let mut store = Store::at("gameboy", Some(dir.clone()));
    let mut buf = Vec::new();
    let mut times = Vec::new();
    for _ in 0..50 {
        let t = std::time::Instant::now();
        buf.clear();
        Carts::snap(&mut s, &mut buf);
        let taken = t.elapsed();
        store.put(&cart.sha1, Carts::FORMAT, &buf);
        times.push((taken, t.elapsed()));
        s.step(&mut view);
    }
    times.sort();
    let (snap, all) = times[times.len() / 2];
    let max = times.iter().map(|t| t.1).max().unwrap_or(Duration::ZERO);
    eprintln!(
        "{}: {} KiB; median snap {snap:.2?}, snap+write {all:.2?}, worst {max:.2?}",
        cart.name,
        buf.len() / 1024
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Not a check: every Pokémon ROM `test_roms` names, rotated every
/// `POKEBOT_ROTATE_MINS` game-minutes through the real runner with its
/// states on disk (`POKEBOT_STATE_DIR`, else a scratch folder), and every
/// fourth rotation a new runner over the same folder, as a pod restart
/// would be. Prints each ROM's badges as its game time adds up, until each
/// has played `POKEBOT_HOURS`.
#[test]
#[ignore = "a benchmark, run by hand"]
fn bench_rotation() {
    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.into());
    let roms = test_roms();
    if roms.is_empty() {
        return;
    }
    let mins: f64 = env("POKEBOT_ROTATE_MINS", "5").parse().unwrap();
    let hours: f64 = env("POKEBOT_HOURS", "3").parse().unwrap();
    let dir =
        std::env::var("POKEBOT_STATE_DIR").map_or_else(|_| scratch("rotation"), PathBuf::from);
    let per_turn = (mins * 60.0 * 59.73) as u64;
    let turns = (hours * 60.0 / mins).ceil() as usize * roms.len();
    let names: Vec<String> = roms.iter().map(|c| c.name.clone()).collect();
    let rams: Vec<ram::Ram> = roms.iter().map(ram_of).collect();
    let mut played = vec![0u64; roms.len()];
    let mut r = runner_at(&dir, roms);
    let w = want();
    let t0 = std::time::Instant::now();
    for turn in 0..turns {
        if turn > 0 && turn % 4 == 0 {
            r = runner_at(&dir, test_roms());
        }
        for _ in 0..per_turn {
            assert!(r.frame(&w));
        }
        let name = r.playing();
        let i = names.iter().position(|n| *n == name).unwrap();
        played[i] += per_turn;
        let gb = &mut r.session.as_mut().unwrap().gb;
        let ram = rams[i];
        eprintln!(
            "ROTATION turn {turn:3} {name:50} {:6.1} game-min  badges {}  map {:3}  lv {}",
            played[i] as f64 / 59.73 / 60.0,
            gb.peek(ram.at(0xD356)).count_ones(),
            gb.peek(ram.cur_map),
            gb.peek(ram.party_mons + 0x21),
        );
        r.next();
    }
    eprintln!("resumes {} in {:.0?}", r.resumes, t0.elapsed());
}
