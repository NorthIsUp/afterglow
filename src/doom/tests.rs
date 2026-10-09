use std::time::{Duration, Instant};

use super::*;
use crate::grid::with_test_aspect;
use crate::mirror::codec::Encoder;
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
        hud: 1,
        skill: 3,
        god: 1,
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

/// The panel path: every panel pixel is the source pixel its column and row
/// scale from, a frame that changes a few pixels reports only around them,
/// an unchanged one reports nothing, and every change is inside the damage —
/// against a junk-filled "hardware" copy, so an unreported pixel shows.
#[test]
fn frames_scale_onto_the_panel_and_report_exactly_what_moved() {
    use crate::dump::verify;
    for (w, h, aspect) in [(1920, 1080, 180), (1080, 1080, 100), (1024, 768, 100)] {
        let (p, mut d) = build(w, h, aspect, "/nonexistent.wad", &knobs(0, 100));
        for (i, c) in d.palette.iter_mut().enumerate() {
            *c = (i as u32).wrapping_mul(0x0001_0307) & 0xFF_FFFF;
        }
        let sw = d.col_w;
        let mut rng = 3u32;
        for px in &mut d.pix[..sw * H] {
            *px = next_rand(&mut rng) as u8;
        }
        let mut buf = vec![0u32; p.buf_len()];
        let mut hw = vec![0xDEAD_BEEF; p.buf_len()];
        let case = format!("{w}x{h}@{aspect}");
        for step in 0..4 {
            if step == 1 {
                d.pix[17 * sw + 5] ^= 1;
                d.pix[150 * sw + sw - 1] ^= 1;
            }
            if step == 3 {
                d.shown = Some((sw, 3));
            }
            if step != 2 {
                d.fresh = true;
            }
            if step == 0 {
                d.shown = Some((sw, 0));
            }
            let before = buf.clone();
            let mut dmg = None;
            let n = allocs_during(|| dmg = Some(saver::frame(&mut d, &mut buf, &p)));
            assert_eq!(n, 0, "{case} step {step}: render allocated");
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
                1 => assert!(dmg.px() < w * 20, "{case}: {:?}", dmg.runs()),
                2 => assert!(dmg.is_empty(), "{case}: an old frame dirtied rows"),
                _ => {}
            }
            let (x0, x1, y0, y1) = d.px_rect;
            let base = d.shown.map_or(0, |s| s.1 as usize * 256);
            for y in 0..h {
                for x in 0..w {
                    let want = if (x0..x1).contains(&x) && (y0..y1).contains(&y) {
                        let sx = (x - x0) * sw / (x1 - x0);
                        let sy = (y - y0) * H / (y1 - y0);
                        d.palette[base + d.pix[sy * sw + sx] as usize]
                    } else {
                        0
                    };
                    assert_eq!(buf[y * w + x], want, "{case} step {step}: pixel {x},{y}");
                }
            }
        }
    }
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

/// Per-frame cost at pine's shape, split into the engine (its own thread),
/// the saver's frame plus the shadow-to-hardware copy, and the mirror's copy
/// and diff encode with one viewer. Every timed frame is a NEW engine frame,
/// which is what the panel sees at any `SAVER_FPS` under 35.
///
/// ```text
/// DOOM_TEST_WAD=... cargo test --release --features doom bench_doom -- --ignored --nocapture
/// ```
#[test]
#[ignore = "a benchmark, not a check: run it explicitly"]
fn bench_doom() {
    use crate::surface::MAX_RUNS;
    use drm::control::ClipRect;
    let wad = wad();
    let (p, mut d) = build(1920, 1080, 180, &wad, &knobs(0, 100));
    let mut buf = vec![0u32; p.buf_len()];
    until(&p, &mut d, &mut buf, "a first frame", |d| d.shown.is_some());
    let (n0, t0) = (
        engine::TICKS.load(Ordering::Relaxed),
        engine::TICK_NS.load(Ordering::Relaxed),
    );
    let mut frames = Vec::new();
    let mut last = d.seq;
    let start = Instant::now();
    while frames.len() < 120 && start.elapsed() < Duration::from_secs(20) {
        saver::frame(&mut d, &mut buf, &p);
        if d.seq != last {
            last = d.seq;
            if let Some((w, pal)) = d.shown {
                frames.push((d.pix[..w * H].to_vec(), w, pal));
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let ticks = engine::TICKS.load(Ordering::Relaxed) - n0;
    let tick_ns = engine::TICK_NS.load(Ordering::Relaxed) - t0;
    let pal = d.palette.clone();
    drop(d);

    let (_, mut r) = build(1920, 1080, 180, "/nonexistent.wad", &knobs(0, 100));
    r.palette = pal;
    let mut hw = vec![0u32; p.buf_len()];
    let mut rects = [ClipRect::new(0, 0, 0, 0); MAX_RUNS];
    let (mut render, mut mirror, mut px, mut bytes) = (Duration::ZERO, Duration::ZERO, 0, 0);
    let mut saver_only = Duration::ZERO;
    let mut cells: Vec<Cell> = Vec::new();
    let mut enc = Encoder::new();
    let rounds = 600;
    for i in 0..=rounds {
        let (f, w, pl) = &frames[i % frames.len()];
        r.pix[..f.len()].copy_from_slice(f);
        (r.shown, r.fresh) = (Some((*w, *pl)), true);
        let t = Instant::now();
        let dmg = saver::frame(&mut r, &mut buf, &p);
        let rl = t.elapsed();
        let n = dmg.rects(&mut rects);
        for rc in &rects[..n] {
            let (x0, x1) = (rc.x1() as usize, rc.x2() as usize);
            for y in rc.y1() as usize..rc.y2() as usize {
                let o = y * p.w;
                hw[o + x0..o + x1].copy_from_slice(&buf[o + x0..o + x1]);
            }
        }
        std::hint::black_box(&mut hw);
        let el = t.elapsed();
        // The mirror: publish's copy, then one viewer's diff encode.
        let t = Instant::now();
        cells.clear();
        cells.extend_from_slice(saver::Saver::mirror(&mut r).cells());
        let out = enc.encode(&cells).len();
        let ml = t.elapsed();
        if i > 0 {
            render += el;
            saver_only += rl;
            mirror += ml;
            px += dmg.px();
            bytes += out;
        }
    }
    let per = |d: Duration| d.as_secs_f64() * 1e6 / rounds as f64;
    println!(
        "\ndoom @1920x1080/180: grid {}x{} cells {}x{}px, {} captured frames",
        r.grid.cols(),
        r.grid.rows(),
        r.grid.cell_w(),
        r.grid.cell_h(),
        frames.len()
    );
    println!(
        "  engine  {:8.1} us/tic  ({ticks} tics) -> x35/s",
        tick_ns as f64 / 1e3 / ticks.max(1) as f64
    );
    println!(
        "  frame   {:8.1} us/frame (render {:.1} + hw copy)  {:.3} Mpx/frame",
        per(render),
        per(saver_only),
        px as f64 / rounds as f64 / 1e6
    );
    println!(
        "  mirror  {:8.1} us/frame (copy + encode)    {:.0} KB/frame",
        per(mirror),
        bytes as f64 / rounds as f64 / 1e3
    );
}
