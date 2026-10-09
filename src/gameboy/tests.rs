use super::carts::{load, Cart, Pilot, BUNDLED};
use super::engine::Session;
use super::*;
use crate::grid::with_test_aspect;
use crate::saver;
use crate::testalloc::allocs_during;

fn want(width: usize, mode: Mode) -> Want {
    Want {
        rom: String::new(),
        sav: String::new(),
        palette: "auto".into(),
        width,
        mode,
        rotate: None,
        seed: 7,
    }
}

fn bundled(name: &str) -> Cart {
    load("", "")
        .into_iter()
        .find(|c| c.name == name)
        .expect(name)
}

/// Run a cartridge headless, `frames` frames, and hand back the session.
fn play(cart: &Cart, width: usize, mode: Mode, frames: u64) -> (Session, Vec<u16>) {
    let mut s = Session::boot(cart, &want(width, mode), 7).expect("boots");
    let mut view = vec![0u16; MAX_W * H];
    for _ in 0..frames {
        s.step(&mut view);
    }
    (s, view)
}

/// The view is as wide as the glass at the screen's full height; glass
/// narrower than 10:9 letterboxes the screen.
#[test]
fn the_view_width_follows_the_glass_shape() {
    for (w, h, aspect, want) in [
        (1920, 1080, 180, (461, 1000, 1000)),
        (1280, 400, 100, (461, 1000, 1000)),
        (1920, 1080, 100, (256, 1000, 1000)),
        (1024, 768, 100, (192, 1000, 1000)),
        (1080, 1080, 100, (160, 1000, 900)),
        (1080, 1920, 100, (160, 1000, 506)),
    ] {
        let (vw, wide, tall) = layout(&Panel::new(w, h, w), aspect);
        assert_eq!(vw, want.0, "{w}x{h}@{aspect}");
        assert!(
            wide.abs_diff(want.1) <= 2 && tall.abs_diff(want.2) <= 2,
            "{w}x{h}@{aspect}: {wide} {tall}"
        );
    }
}

/// The panel path: every panel pixel is the view pixel it scales from, a
/// view that changes a few pixels reports only around them, an unchanged one
/// reports nothing, and every change is inside the damage — against a
/// junk-filled "hardware" copy, so an unreported pixel shows. No frame
/// allocates.
#[test]
fn views_scale_onto_the_panel_and_report_exactly_what_moved() {
    use crate::dump::verify;
    for (w, h, aspect) in [
        (1920, 1080, 180),
        (1920, 1080, 100),
        (1024, 768, 100),
        (1080, 1920, 100),
    ] {
        let p = Panel::new(w, h, w);
        let mut g = with_test_aspect(aspect, || {
            let (vw, ..) = layout(&p, aspect);
            GameBoySaver::build(&p, aspect, want(vw, Mode::Frame))
        });
        g.claim = Claim::Off;
        g.shown = true;
        let vw = g.w;
        let mut rng = 3u32;
        for px in &mut g.pix[..vw * H] {
            *px = next_rand(&mut rng) as u16;
        }
        let mut buf = vec![0u32; p.buf_len()];
        let case = format!("{w}x{h}@{aspect}");
        let mut prev_dmg_px = usize::MAX;
        for step in 0..3 {
            if step == 1 {
                g.pix[17 * vw + 5] ^= 1;
                g.pix[140 * vw + vw - 1] ^= 1;
            }
            g.fresh = step != 2;
            let before = buf.clone();
            let mut dmg = None;
            let n = allocs_during(|| dmg = Some(saver::frame(&mut g, &mut buf, &p)));
            assert_eq!(n, 0, "{case} step {step}: render allocated");
            let dmg = dmg.expect("rendered");
            verify(&before, &buf, &dmg, &p, step).unwrap_or_else(|e| panic!("{case}: {e}"));
            match step {
                0 => {
                    let (x0, x1, y0, y1) = g.px_rect;
                    for (y, x) in [(y0, x0), (y1 - 1, x1 - 1), ((y0 + y1) / 2, (x0 + x1) / 2)] {
                        let sy = (y - y0) * H / (y1 - y0);
                        let sx = (x - x0) * vw / (x1 - x0);
                        assert_eq!(
                            buf[y * w + x],
                            g.lut[g.pix[sy * vw + sx] as usize],
                            "{case} ({x},{y})"
                        );
                    }
                }
                1 => assert!(
                    dmg.px() < prev_dmg_px / 50,
                    "{case}: {} px for two pixels",
                    dmg.px()
                ),
                _ => assert!(dmg.is_empty(), "{case}: an unchanged view reported damage"),
            }
            prev_dmg_px = dmg.px();
        }
    }
}

/// Every bundled cartridge boots and animates under its pilot, in both
/// views, and the screen sits in the middle of the view untouched.
#[test]
fn every_bundled_cartridge_plays() {
    for b in BUNDLED {
        let cart = bundled(b.name);
        for mode in [Mode::Frame, Mode::Wide] {
            let (mut s, mut view) = play(&cart, 461, mode, 600);
            let first = view.clone();
            for _ in 0..300 {
                s.step(&mut view);
            }
            assert_ne!(first, view, "{} ({mode:?}) froze", b.name);
            let x0 = (461 - SCREEN_W) / 2;
            assert!(
                view[..461 * H].iter().all(|&p| p != 0xFFFF),
                "{}: unknown pixel left in the view",
                b.name
            );
            assert_eq!(
                view[x0 - 1] & DIM,
                DIM,
                "{}: the side is not the glow",
                b.name
            );
        }
    }
}

/// Rebound scrolls right; the wide view remembers the level behind the
/// ball, so the left side is the game's own background, not the glow.
#[test]
fn the_wide_view_remembers_what_scrolled_past() {
    let (_, view) = play(&bundled("rebound"), 461, Mode::Wide, 60 * 40);
    let x0 = (461 - SCREEN_W) / 2;
    let remembered = (0..H)
        .flat_map(|y| (0..x0).map(move |x| (y, x)))
        .filter(|&(y, x)| view[y * 461 + x] & DIM == 0)
        .count();
    assert!(
        remembered > x0 * H / 3,
        "only {remembered} of {} side pixels remembered",
        x0 * H
    );
}

/// Tobu Tobu Girl's pilot gets past the menus into a level and keeps her
/// alive long enough to climb.
#[test]
fn the_tobu_pilot_plays_a_level() {
    let (mut s, _) = play(&bundled("tobu-tobu-girl-deluxe"), 160, Mode::Frame, 60 * 30);
    assert_eq!(s.gb.peek(0xC0A5), 4, "not in a level");
    assert_eq!(s.pilot(), Pilot::Tobu);
}

/// The engine survives a cartridge that panics: it boots the next one.
#[test]
fn a_crashing_cartridge_moves_on() {
    let carts = load("", "");
    let mut r = engine::tests_runner(carts);
    let w = want(256, Mode::Frame);
    assert!(r.frame_for_test(&w));
    let first = r.playing();
    engine::PANIC_NEXT.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(r.frame_for_test(&w));
    assert_ne!(r.playing(), first);
}

/// A user's Pokémon ROM, read in place and never copied: `POKEMON_TEST_ROM`.
/// Skipped when unset, so CI needs no commercial ROM.
fn pokemon_rom() -> Option<Cart> {
    let path = std::env::var("POKEMON_TEST_ROM").ok()?;
    let carts = load(&path, "");
    let c = carts.into_iter().next()?;
    assert!(
        matches!(c.pilot, Pilot::Pokemon(_)),
        "{path}: not a known Pokémon revision"
    );
    Some(c)
}

/// The bot gets through the intro and out of the bedroom, and the world
/// view lines up with the real screen wherever it is drawn.
#[test]
fn pokemon_bot_leaves_home_and_the_world_lines_up() {
    let Some(cart) = pokemon_rom() else {
        eprintln!("POKEMON_TEST_ROM unset; skipping");
        return;
    };
    let mut s = Session::boot(&cart, &want(461, Mode::Wide), 7).expect("boots");
    let ram = pokemon::Ram::of(match cart.pilot {
        Pilot::Pokemon(r) => r,
        _ => unreachable!(),
    });
    let mut view = vec![0u16; MAX_W * H];
    let mut maps = std::collections::BTreeSet::new();
    let (mut drawn, mut checked) = (0, 0);
    for f in 0..60 * 60 * 10 {
        s.step(&mut view);
        maps.insert(s.gb.peek(ram.cur_map));
        if f % 30 == 0 {
            if let Some(w) = s.composer.world() {
                if w.agree > 0 {
                    checked += 1;
                    if w.agree >= 950 {
                        drawn += 1;
                    }
                }
            }
        }
    }
    eprintln!("maps visited: {maps:?}; world drawn {drawn}/{checked}");
    assert!(maps.len() >= 4, "only saw maps {maps:?}");
    assert!(
        drawn * 2 > checked,
        "world matched the screen in {drawn} of {checked} checks"
    );
}

/// Not a check: writes every `GAMEBOY_PROBE_EVERY`th view of the first
/// `GAMEBOY_PROBE_FRAMES` to `GAMEBOY_PROBE_DIR` as PPM, with the bot's
/// position, for looking at a ROM's play by eye. `GAMEBOY_PROBE_ROM` names
/// the ROM; unset, it is skipped.
#[test]
#[ignore = "a viewer, run by hand"]
fn probe() {
    use std::io::Write;
    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.into());
    let rom = env("GAMEBOY_PROBE_ROM", "");
    let dir = env("GAMEBOY_PROBE_DIR", "/tmp/gbprobe");
    let frames: u64 = env("GAMEBOY_PROBE_FRAMES", "3600").parse().unwrap();
    let every: u64 = env("GAMEBOY_PROBE_EVERY", "300").parse().unwrap();
    let w: usize = env("GAMEBOY_PROBE_W", "461").parse().unwrap();
    let cart = load(&rom, "").remove(0);
    let mut s = Session::boot(&cart, &want(w, Mode::Wide), 7).expect("boots");
    let mut view = vec![0u16; MAX_W * H];
    std::fs::create_dir_all(&dir).unwrap();
    for f in 0..frames {
        s.step(&mut view);
        if f % every == 0 {
            let mut out = format!("P6 {w} {H} 255\n").into_bytes();
            for &p in &view[..w * H] {
                let c = xrgb(p);
                out.extend([(c >> 16) as u8, (c >> 8) as u8, c as u8]);
            }
            std::fs::File::create(format!("{dir}/v-{f:07}.ppm"))
                .unwrap()
                .write_all(&out)
                .unwrap();
            let r = pokemon::Ram::of(match cart.pilot {
                Pilot::Pokemon(r) => r,
                _ => carts::Revision::Red,
            });
            let agree = s.composer.world().map_or(0, |w| w.agree);
            let party = s.gb.peek(r.party_count);
            eprintln!(
                "{f:7} party {party} map {:3} x {:3} y {:3} battle {} font {} agree {agree}",
                s.gb.peek(r.cur_map),
                s.gb.peek(r.x),
                s.gb.peek(r.y),
                s.gb.peek(r.in_battle),
                s.gb.peek(r.font_loaded),
            );
        }
    }
}

/// Not a check: how far `World::camera` is from the offset that fits the
/// screen best, sampled while the bot plays `GAMEBOY_PROBE_ROM`.
#[test]
#[ignore = "a calibration aid, run by hand"]
fn camera_calibration() {
    let rom = std::env::var("GAMEBOY_PROBE_ROM").unwrap_or_default();
    let cart = load(&rom, "").remove(0);
    let Pilot::Pokemon(rev) = cart.pilot else {
        panic!("not Pokémon")
    };
    let r = pokemon::Ram::of(rev);
    let mut s = Session::boot(&cart, &want(461, Mode::Frame), 7).expect("boots");
    let mut world = world::World::new(rev);
    let mut view = vec![0u16; MAX_W * H];
    let (mut off, mut total) = (0, 0);
    for f in 0..60 * 60 * 12 {
        s.step(&mut view);
        if f % 17 != 0 {
            continue;
        }
        if let Some((best, a, here)) = world.best_offset(&mut s.gb, 461, 18) {
            if a < 900 {
                continue;
            }
            total += 1;
            if best != (0, 0) && a > here + 20 {
                off += 1;
                eprintln!(
                    "{f:6} map {:3} walk {} dir {:02x} scx {:3} scy {:3} best {best:?} {a} here {here}",
                    s.gb.peek(r.cur_map),
                    s.gb.peek(r.walk_counter),
                    s.gb.peek(0xD528),
                    s.gb.line_scroll()[0][0],
                    s.gb.line_scroll()[0][1]
                );
            }
        }
    }
    eprintln!("{off} of {total} fitted frames were off");
}

/// Not a check: engine time per Game Boy frame (emulation, pilot, view),
/// for the CPU estimate in the docs. `GAMEBOY_PROBE_ROM` adds a ROM of your
/// own to the bundled ones.
#[test]
#[ignore = "a benchmark, run by hand"]
fn bench_engine() {
    let mut carts = load("", "");
    if let Ok(rom) = std::env::var("GAMEBOY_PROBE_ROM") {
        carts.extend(load(&rom, ""));
    }
    for cart in &carts {
        for (mode, w) in [(Mode::Frame, 461), (Mode::Wide, 461), (Mode::Wide, 256)] {
            let (mut s, mut view) = play(cart, w, mode, 60 * 90);
            let t0 = std::time::Instant::now();
            let n = 60 * 60;
            for _ in 0..n {
                s.step(&mut view);
            }
            let per = t0.elapsed() / n;
            eprintln!(
                "{:24} {mode:?} w{w}: {per:?}/frame, {:.1}% of a core at 59.7 Hz",
                cart.name,
                per.as_secs_f64() * 59.73 * 100.0
            );
        }
    }
}

/// Not a check: render-thread time for a view that changed everywhere (the
/// worst case: every row redrawn), and for one where only the screen's
/// middle moved, at pine's shape.
#[test]
#[ignore = "a benchmark, run by hand"]
fn bench_blit() {
    let p = Panel::new(1920, 1080, 1920);
    let mut g = with_test_aspect(180, || GameBoySaver::build(&p, 180, want(461, Mode::Wide)));
    g.claim = Claim::Off;
    g.shown = true;
    let mut buf = vec![0u32; p.buf_len()];
    let mut rng = 1u32;
    for (name, region) in [("everything", 0..461), ("the screen only", 150..310)] {
        let n = 300;
        let t0 = std::time::Instant::now();
        for _ in 0..n {
            for y in 0..H {
                for x in region.clone() {
                    g.pix[y * 461 + x] = next_rand(&mut rng) as u16;
                }
            }
            g.fresh = true;
            saver::frame(&mut g, &mut buf, &p);
        }
        eprintln!("{name}: {:?}/frame", t0.elapsed() / n);
    }
}
