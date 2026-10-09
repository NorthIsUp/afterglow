use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::*;
use crate::grid::with_test_aspect;
use crate::saver;
use crate::testalloc::allocs_during;

/// The engines are process-wide, so the tests that drive them take turns.
static ENGINES: Mutex<()> = Mutex::new(());

/// A real IWAD for the engine tests. CI downloads Freedoom and sets this;
/// `mise run test-doom` does the same locally.
fn wad() -> String {
    std::env::var("DOOM_TEST_WAD")
        .expect("set DOOM_TEST_WAD to an IWAD (mise run test-doom fetches Freedoom)")
}

fn build(w: usize, h: usize, aspect: usize, wad: &str, views: usize) -> (Panel, Doom) {
    let p = Panel::new(w, h, w);
    let d = with_test_aspect(aspect, || Doom::build(&p, wad, views, 0, 2, 1, 7));
    (p, d)
}

/// Render until `done`, or fail after ten seconds: the engines run on their
/// own clock, so a test can only wait for them.
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

#[test]
fn views_follow_the_glass_shape() {
    for (w, h, aspect, n) in [
        (1920, 1080, 180, 3),
        (1280, 400, 100, 3),
        (1920, 1080, 100, 2),
        (1024, 768, 100, 1),
        (1080, 1080, 100, 1),
        (1080, 1920, 100, 1),
    ] {
        assert_eq!(
            views_for(&Panel::new(w, h, w), aspect),
            n,
            "{w}x{h}@{aspect}"
        );
    }
}

/// Pine draws three whole 320-column views, one cell per Doom pixel across.
#[test]
fn pine_maps_every_column_to_one_of_three_views() {
    let (_, d) = build(1920, 1080, 180, "/nonexistent.wad", 0);
    assert_eq!((d.views, d.grid.cols()), (3, 960));
    for v in 0..3u16 {
        let cols: Vec<_> = (0..960).filter(|&c| d.col_view[c] == v).collect();
        assert_eq!(cols.len(), 320);
        assert!(cols
            .iter()
            .enumerate()
            .all(|(i, &c)| d.col_src[c] as usize == i));
    }
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

/// No WAD is static, not a crash, and never wakes the engines.
#[test]
fn without_a_wad_it_is_static_and_never_allocates() {
    let (p, mut d) = build(640, 400, 100, "/nonexistent.wad", 0);
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

/// An `I_Error` inside an engine unwinds to the C boundary: the process keeps
/// running, the faulted copy is retired, and the view picks up on a spare.
#[test]
fn an_engine_error_retires_that_engine_and_the_view_carries_on() {
    let _g = ENGINES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (p, mut d) = build(640, 400, 100, &wad(), 1);
    let mut buf = vec![0u32; p.buf_len()];
    until(&p, &mut d, &mut buf, "a first frame", |d| {
        d.pal[0].is_some()
    });
    let e = Engines::get();
    let first = e.slot_of(0).expect("view 0 has an engine");

    e.fault(0);
    until(&p, &mut d, &mut buf, "the view to move", |_| {
        e.slot_of(0).is_some_and(|s| s != first)
    });
    let moved = d.seq[0];
    until(&p, &mut d, &mut buf, "frames from the spare", |d| {
        d.pal[0].is_some() && d.seq[0].wrapping_sub(moved) > 2
    });
    saver::frame(&mut d, &mut buf, &p);
    assert!(
        d.grid.cells().iter().all(|c| c.colour() < STATIC),
        "still static"
    );
}

/// Switching away parks the engines; switching back reuses the same copy on a
/// new map, with no restart.
#[test]
fn switching_away_and_back_reuses_the_engine() {
    let _g = ENGINES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let wad = wad();
    let (p, mut a) = build(640, 400, 100, &wad, 1);
    let mut buf = vec![0u32; p.buf_len()];
    until(&p, &mut a, &mut buf, "a first frame", |d| {
        d.pal[0].is_some()
    });
    let slot = Engines::get().slot_of(0);
    drop(a);

    let (_, mut b) = build(640, 400, 100, &wad, 1);
    until(&p, &mut b, &mut buf, "frames after switching back", |d| {
        d.pal[0].is_some()
    });
    assert_eq!(Engines::get().slot_of(0), slot);
}

#[test]
fn render_never_allocates_with_engines_running() {
    let _g = ENGINES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (p, mut d) = build(1920, 1080, 180, &wad(), 0);
    let mut buf = vec![0u32; p.buf_len()];
    until(&p, &mut d, &mut buf, "frames in every view", |d| {
        d.pal[..3].iter().all(Option::is_some)
    });
    let n = allocs_during(|| {
        for _ in 0..50 {
            saver::frame(&mut d, &mut buf, &p);
            std::thread::sleep(Duration::from_millis(2));
        }
    });
    assert_eq!(n, 0, "render allocated");
}
