use std::path::PathBuf;

use super::carts::{files, pokemon, Pokemon};
use super::engine::{Runner, Session, PANIC_NEXT};
use super::pilot::{Phase, Pilot};
use super::*;
use crate::grid::with_test_aspect;
use crate::saver;
use crate::testalloc::allocs_during;

/// A ROM of our own, so CI plays something: mode 3, diagonal stripes that
/// crawl a step per pass. ARM, assembled with `clang --target=armv4t-none-eabi`.
fn stripes_rom() -> Vec<u8> {
    const CODE: [u32; 19] = [
        0xe3a0_0301, // mov  r0, #0x04000000
        0xe3a0_1b01, // mov  r1, #0x400
        0xe381_1003, // orr  r1, r1, #3
        0xe1c0_10b0, // strh r1, [r0]          DISPCNT: mode 3, BG2
        0xe3a0_3000, // mov  r3, #0
        0xe1d0_20b6, // frame: ldrh r2, [r0, #6]
        0xe352_00a0, // cmp  r2, #160
        0x1aff_fffc, // bne  frame
        0xe283_3001, // add  r3, r3, #1
        0xe3a0_4406, // mov  r4, #0x06000000
        0xe3a0_5c96, // mov  r5, #38400
        0xe085_6003, // fill: add r6, r5, r3
        0xe0c4_60b2, // strh r6, [r4], #2
        0xe255_5001, // subs r5, r5, #1
        0x1aff_fffb, // bne  fill
        0xe1d0_20b6, // vblank: ldrh r2, [r0, #6]
        0xe352_00a0, // cmp  r2, #160
        0x0aff_fffc, // beq  vblank
        0xeaff_fff1, // b    frame
    ];
    let mut rom = vec![0u8; 0xC0];
    rom[..4].copy_from_slice(&0xea00_002e_u32.to_le_bytes()); // b 0xC0
    rom[0xB2] = 0x96;
    for w in CODE {
        rom.extend(w.to_le_bytes());
    }
    rom
}

/// A scratch folder of `(name, bytes)` files, unique per test.
fn folder(tag: &str, roms: &[(&str, &[u8])]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("afterglow-gba-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (name, bytes) in roms {
        std::fs::write(dir.join(name), bytes).unwrap();
    }
    dir
}

fn want(rom: &str, width: usize) -> Want {
    Want {
        rom: rom.into(),
        bios: String::new(),
        width,
        fps: 60,
        rotate: None,
        seed: 7,
        restart: 0,
    }
}

/// The view is as wide as the glass at the screen's full height; glass
/// narrower than 3:2 letterboxes the screen.
#[test]
fn the_view_width_follows_the_glass_shape() {
    for (w, h, aspect, want) in [
        (1920, 1080, 180, (512, 1000, 1000)),
        (1920, 1080, 100, (284, 1000, 1000)),
        (1024, 768, 100, (240, 1000, 888)),
        (1080, 1920, 100, (240, 1000, 375)),
    ] {
        let (vw, wide, tall) = layout(&Panel::new(w, h, w), aspect);
        assert_eq!(vw, want.0, "{w}x{h}@{aspect}");
        assert!(
            wide.abs_diff(want.1) <= 2 && tall.abs_diff(want.2) <= 2,
            "{w}x{h}@{aspect}: {wide} {tall}"
        );
    }
}

/// The panel path: a view that changes a few pixels reports only around
/// them, an unchanged one nothing, every change inside the damage, and no
/// frame allocates.
#[test]
fn views_scale_onto_the_panel_and_report_exactly_what_moved() {
    use crate::dump::verify;
    for (w, h, aspect) in [(1920, 1080, 180), (1920, 1080, 100), (1080, 1920, 100)] {
        let p = Panel::new(w, h, w);
        let mut g = with_test_aspect(aspect, || Gba::build(&p, aspect, None));
        g.shown = true;
        let vw = g.view.width();
        let mut rng = 3u32;
        for px in &mut g.pix[..vw * H] {
            *px = next_rand(&mut rng) as u16;
        }
        let mut buf = vec![0u32; p.buf_len()];
        let case = format!("{w}x{h}@{aspect}");
        let mut prev = usize::MAX;
        for step in 0..3 {
            if step == 1 {
                g.pix[17 * vw + 5] ^= 1;
                g.pix[150 * vw + vw - 1] ^= 1;
            }
            g.fresh = step != 2;
            let before = buf.clone();
            let mut dmg = None;
            let n = allocs_during(|| dmg = Some(saver::frame(&mut g, &mut buf, &p)));
            assert_eq!(n, 0, "{case} step {step}: render allocated");
            let dmg = dmg.expect("rendered");
            verify(&before, &buf, &dmg, &p, step).unwrap_or_else(|e| panic!("{case}: {e}"));
            match step {
                1 => assert!(dmg.px() < prev / 50, "{case}: {} px for two", dmg.px()),
                2 => assert!(dmg.is_empty(), "{case}: an unchanged view reported damage"),
                _ => {}
            }
            prev = dmg.px();
        }
    }
}

/// No ROMs: a card naming the folder, drawn once, and no engine started.
#[test]
fn no_roms_show_a_card_and_never_start_the_engine() {
    let p = Panel::new(1920, 1080, 1920);
    std::env::set_var("GBA_ROM", "/nonexistent/gba");
    let mut g = with_test_aspect(100, || Gba::new(&p, 30));
    std::env::remove_var("GBA_ROM");
    assert!(g.claim.want.is_none() && g.shown);
    let vw = g.view.width();
    assert!(g.pix[..vw * H].contains(&CARD_INK) && g.pix[..vw * H].contains(&CARD_BG));
    let mut buf = vec![0u32; p.buf_len()];
    let first = saver::frame(&mut g, &mut buf, &p);
    assert!(!first.is_empty());
    assert!(
        saver::frame(&mut g, &mut buf, &p).is_empty(),
        "the card is drawn once"
    );
    assert!(g.claim.engine.is_none());
}

#[test]
fn only_gba_files_are_roms_sorted_by_name() {
    let dir = folder(
        "files",
        &[
            ("b.gba", b""),
            ("a.GBA", b""),
            ("c.gb", b""),
            ("d.sav", b""),
        ],
    );
    let names: Vec<_> = files(dir.to_str().unwrap())
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["a.GBA", "b.gba"]);
    assert_eq!(files(""), Vec::<PathBuf>::new());
    let _ = std::fs::remove_dir_all(dir);
}

/// Our own ROM boots on the HLE BIOS and its stripes crawl, centred in the
/// view, without the core or the pilot allocating per frame.
#[test]
fn the_stripes_rom_plays_on_the_hle_bios() {
    let dir = folder("play", &[("stripes.gba", &stripes_rom())]);
    let mut s = Session::boot(&dir.join("stripes.gba"), &[], 1).expect("boots");
    assert_eq!(pokemon(&stripes_rom()), None);
    let w = 512;
    let mut view = vec![0u16; MAX_W * H];
    for _ in 0..30 {
        assert!(s.step(w, &mut view));
    }
    let a = view.clone();
    let n = allocs_during(|| {
        for _ in 0..10 {
            s.step(w, &mut view);
        }
    });
    assert_eq!(n, 0, "a frame allocated");
    assert_ne!(a[..w * H], view[..w * H], "the stripes stopped");
    let x0 = (w - W) / 2;
    assert!(
        view[x0..x0 + W].iter().any(|&p| p != view[x0]),
        "no stripes"
    );
    assert!(
        view[..x0].iter().all(|&p| p & DIM != 0),
        "the sides are glow"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A file mGBA rejects is skipped for good; one that panics mid-frame
/// moves on to the next; with nothing left, the runner shows nothing.
#[test]
fn bad_and_crashing_roms_move_on() {
    let dir = folder(
        "bad",
        &[("a-empty.gba", b""), ("b-stripes.gba", &stripes_rom())],
    );
    let w = want(dir.to_str().unwrap(), 284);
    let mut r = Runner::new();
    r.take(None, &w);
    assert!(r.frame(&w), "the good ROM plays");
    assert_eq!(r.session.as_ref().unwrap().name, "b-stripes");
    PANIC_NEXT.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(
        !r.frame(&w),
        "a crash with nothing left to play shows nothing"
    );
    assert!(r.session.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_generic_game_boots_on_a_and_start_then_walks_without_start() {
    let screen = vec![0u32; W * H];
    let mut p = Pilot::new(None, 3);
    let mut pressed = 0;
    for _ in 0..600 {
        pressed |= p.keys(&screen, false);
    }
    assert_eq!(pressed, agb::keys::A | agb::keys::START);
    let moving: Vec<u32> = (0..(W * H) as u32).collect();
    let mut keys = 0;
    for f in 0..6000u32 {
        let mut s = moving.clone();
        s[0] = f;
        let k = p.keys(&s, false);
        if p.phase() == Phase::Explore {
            keys |= k;
        }
    }
    assert_eq!(p.phase(), Phase::Explore);
    assert_eq!(keys & agb::keys::START, 0, "START on a moving screen");
    assert_ne!(keys & agb::keys::A, 0);
    assert_ne!(keys & (agb::keys::UP | agb::keys::DOWN), 0);
}

/// A still screen in a generic game earns a START, in Pokémon never: its
/// START is a menu.
#[test]
fn only_a_generic_game_presses_start_on_a_still_screen() {
    let still = vec![0u32; W * H];
    for (game, want_start) in [(None, true), (Some(Pokemon::Emerald), false)] {
        let mut p = Pilot::new(game, 5);
        let mut start = false;
        for _ in 0..20_000 {
            let k = p.keys(&still, true);
            start |= p.phase() == Phase::Explore && k & agb::keys::START != 0;
        }
        assert_eq!(start, want_start, "{game:?}");
    }
}

/// `GBA_TEST_ROM` set to a Pokémon FireRed, LeafGreen or Emerald: the pilot
/// gets through the intro to the overworld, and walks.
#[test]
fn pokemon_reaches_the_overworld() {
    let Ok(path) = std::env::var("GBA_TEST_ROM") else {
        return;
    };
    let path = PathBuf::from(path);
    let mut s = Session::boot(&path, &[], 7).expect("boots");
    let Some(game) = pokemon(&std::fs::read(&path).unwrap()) else {
        return;
    };
    let mut view = vec![0u16; MAX_W * H];
    let mut f = 0;
    while s.pilot().phase() == Phase::Boot {
        assert!(s.step(W, &mut view));
        f += 1;
        assert!(f < 20_000, "{game:?}: still in the intro after {f} frames");
    }
    eprintln!("{game:?}: overworld after {f} frames ({} s)", f / 60);
}

/// `GBA_TEST_ROM`'s cost per frame: `cargo test --release --features gba
/// bench_core -- --ignored --nocapture`.
#[test]
#[ignore = "a benchmark, not a check: run it explicitly"]
fn bench_core() {
    let path = std::env::var("GBA_TEST_ROM").map_or_else(
        |_| {
            let dir = folder("bench", &[("stripes.gba", &stripes_rom())]);
            dir.join("stripes.gba")
        },
        PathBuf::from,
    );
    let mut s = Session::boot(&path, &[], 7).expect("boots");
    let mut view = vec![0u16; MAX_W * H];
    let n = crate::env_num(&["BENCH_FRAMES"], 7200, 1, 1_000_000) as u32;
    let t = std::time::Instant::now();
    for _ in 0..n {
        s.step(512, &mut view);
    }
    let us = t.elapsed().as_secs_f64() * 1e6 / f64::from(n);
    println!(
        "{}: {n} frames, {us:.0} us/frame, {:.0}m of a core at 59.73 Hz",
        path.display(),
        us * 59.73 / 1000.0
    );
}
