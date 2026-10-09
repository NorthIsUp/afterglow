use std::time::{Duration, Instant};

use super::*;
use crate::grid::with_test_aspect;
use crate::saver;
use crate::testalloc::allocs_during;

/// A real IWAD for the engine test. CI downloads Freedoom and sets this;
/// `mise run test-doom` does the same locally.
fn wad() -> String {
    std::env::var("DOOM_TEST_WAD")
        .expect("set DOOM_TEST_WAD to an IWAD (mise run test-doom fetches Freedoom)")
}

fn knobs(fov: i32, pct: i32) -> Knobs {
    Knobs {
        map_secs: 0,
        gamma: 2,
        light: 1,
        fov,
        pct,
        seed: 7,
    }
}

fn build(w: usize, h: usize, aspect: usize, wad: &str, k: &Knobs) -> (Panel, Doom) {
    let p = Panel::new(w, h, w);
    let d = with_test_aspect(aspect, || Doom::build(&p, wad, k));
    (p, d)
}

/// Render until `done`, or fail after ten seconds: the engine runs on its own
/// clock, so a test can only wait for it.
fn until(p: &Panel, d: &mut Doom, buf: &mut [u32], what: &str, done: impl Fn(&Doom) -> bool) {
    let t0 = Instant::now();
    while !done(d) {
        assert!(
            t0.elapsed() < Duration::from_secs(10),
            "timed out waiting for {what}"
        );
        saver::frame(d, buf, p);
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The screen is as wide as the glass shape needs at Doom's pixel aspect, and
/// a panel narrower than 4:3 letterboxes the 320-wide minimum.
#[test]
fn the_screen_width_follows_the_glass_shape() {
    for (w, h, aspect, want) in [
        (1920, 1080, 180, (768, 1000, 1000)),
        (1280, 400, 100, (768, 1000, 1000)),
        (1920, 1080, 100, (427, 1000, 1000)),
        (1024, 768, 100, (320, 1000, 1000)),
        (1080, 1080, 100, (320, 1000, 750)),
        (1080, 1920, 100, (320, 1000, 421)),
    ] {
        let (sw, wide, tall) = layout(&Panel::new(w, h, w), aspect);
        assert_eq!(sw, want.0, "{w}x{h}@{aspect}");
        assert!(
            wide.abs_diff(want.1) <= 2 && tall.abs_diff(want.2) <= 2,
            "{w}x{h}@{aspect}: {wide} {tall}"
        );
    }
}

/// Pine's 768 Doom columns spread over 960 cells, every one of them used,
/// in order, none skipped.
#[test]
fn pine_maps_the_whole_width() {
    let (_, d) = build(1920, 1080, 180, "/nonexistent.wad", &knobs(0, 100));
    assert_eq!((d.grid.cols(), d.col_w), (960, 768));
    let used: std::collections::BTreeSet<_> = d.col_src.iter().copied().collect();
    assert_eq!(used.len(), 768);
    assert!(d.col_src.windows(2).all(|p| p[0] <= p[1]));
    assert!(d.row_src.iter().all(|&r| (r as usize) < H));
}

#[test]
fn lump_reads_a_wad_directory() {
    let mut wad = b"IWAD".to_vec();
    wad.extend(1u32.to_le_bytes());
    wad.extend(15u32.to_le_bytes());
    wad.extend([1, 2, 3]);
    wad.extend(12u32.to_le_bytes());
    wad.extend(3u32.to_le_bytes());
    wad.extend(b"PLAYPAL\0");
    assert_eq!(lump(&wad, b"PLAYPAL"), Some(&[1, 2, 3][..]));
    assert_eq!(lump(&wad, b"COLORMAP"), None);
    assert_eq!(lump(b"not a wad", b"PLAYPAL"), None);
}

/// No WAD is static, not a crash, and never wakes the engine.
#[test]
fn without_a_wad_it_is_static_and_never_allocates() {
    let (p, mut d) = build(640, 400, 100, "/nonexistent.wad", &knobs(0, 100));
    let mut buf = vec![0u32; p.buf_len()];
    saver::frame(&mut d, &mut buf, &p);
    assert!(!d.engaged);
    let n = allocs_during(|| {
        for _ in 0..20 {
            saver::frame(&mut d, &mut buf, &p);
        }
    });
    assert_eq!(n, 0, "render allocated");
    assert!(d.grid.cells().iter().all(|c| c.colour() >= STATIC));
}

/// The engine is one per process and an engine error ends it for good, so
/// everything that needs it runs here, in order, with the error last.
#[test]
fn the_engine_plays_resizes_survives_a_switch_and_then_an_error() {
    let wad = wad();
    let e = Engine::get();

    // Pine: a 768-wide game, drawn without allocating.
    let (p, mut d) = build(1920, 1080, 180, &wad, &knobs(0, 100));
    let mut buf = vec![0u32; p.buf_len()];
    until(&p, &mut d, &mut buf, "a first frame", |d| d.shown.is_some());
    assert_eq!(d.shown.map(|s| s.0), Some(768));
    let n = allocs_during(|| {
        for _ in 0..50 {
            saver::frame(&mut d, &mut buf, &p);
            std::thread::sleep(Duration::from_millis(2));
        }
    });
    assert_eq!(n, 0, "render allocated");

    // A knob change rebuilds the saver; the engine keeps playing, never
    // blanking, at the new box and field of view.
    drop(d);
    let (_, mut d) = build(1920, 1080, 180, &wad, &knobs(90, 60));
    for _ in 0..40 {
        saver::frame(&mut d, &mut buf, &p);
        assert!(
            d.seq == 0 || d.shown.is_some(),
            "a rebuild blanked the game"
        );
        std::thread::sleep(Duration::from_millis(5));
    }

    // 4:3 after more than a second away is a switch: a new 320-wide game.
    drop(d);
    std::thread::sleep(REBUILD_GAP);
    let (p, mut d) = build(1024, 768, 100, &wad, &knobs(0, 100));
    let mut buf = vec![0u32; p.buf_len()];
    until(&p, &mut d, &mut buf, "the 320-wide game", |d| {
        d.shown.is_some_and(|s| s.0 == 320)
    });

    // An I_Error inside the engine unwinds to the C boundary: the process
    // carries on, the saver shows static, and still never allocates.
    e.fault();
    until(&p, &mut d, &mut buf, "static after the error", |d| {
        d.shown.is_none()
    });
    assert!(e.dead());
    let n = allocs_during(|| {
        for _ in 0..20 {
            saver::frame(&mut d, &mut buf, &p);
        }
    });
    assert_eq!(n, 0, "render allocated");
    assert!(d
        .grid
        .cells()
        .iter()
        .all(|c| c.colour() >= STATIC || *c == Cell::CLEAR));
}

const REBUILD_GAP: Duration = Duration::from_millis(1100);
