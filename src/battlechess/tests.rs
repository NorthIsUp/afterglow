use std::fs::Permissions;
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

use super::*;
use crate::dump::verify;
use crate::grid::with_test_aspect;
use crate::next_rand;
use crate::saver;
use crate::testalloc::allocs_during;

/// A scratch directory per test, so parallel tests never share files.
fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("afterglow-mac-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn missing() -> Files {
    Files {
        rom: "/nonexistent/Mac-Plus.ROM".into(),
        system: "/nonexistent/System.img".into(),
        disk: "/nonexistent/BattleChess.img".into(),
        engine: "/nonexistent/mac-engine".into(),
    }
}

/// A Plus ROM's checksum and length, and an HFS volume named `name`.
fn fake_files(dir: &Path, name: &str, engine: PathBuf) -> Files {
    let mut rom = vec![0; ROM_LEN as usize];
    rom[..4].copy_from_slice(&0x4D1F_8172u32.to_be_bytes());
    std::fs::write(dir.join("rom"), rom).unwrap();
    let mut disk = vec![0; 4096];
    disk[1024..1026].copy_from_slice(b"BD");
    disk[1024 + 36] = name.len() as u8;
    disk[1024 + 37..1024 + 37 + name.len()].copy_from_slice(name.as_bytes());
    std::fs::write(dir.join("disk"), &disk).unwrap();
    Files {
        rom: dir.join("rom").to_string_lossy().into(),
        system: dir.join("disk").to_string_lossy().into(),
        disk: dir.join("disk").to_string_lossy().into(),
        engine,
    }
}

fn build(
    w: usize,
    h: usize,
    aspect: usize,
    files: &Files,
    engine: &'static Engine,
) -> (Panel, BattleChess) {
    let p = Panel::new(w, h, w);
    let b = with_test_aspect(aspect, || {
        BattleChess::build(&p, aspect, files, "paper", engine)
    });
    (p, b)
}

/// The view is as wide as the glass at the Mac's square pixels; glass
/// narrower than 3:2 letterboxes the screen.
#[test]
fn the_view_width_follows_the_glass_shape() {
    for (w, h, aspect, want) in [
        (1920, 1080, 180, (1094, 1000, 1000)),
        (1920, 1080, 100, (608, 1000, 1000)),
        (1280, 400, 100, (1094, 1000, 1000)),
        (1024, 768, 100, (512, 1000, 889)),
        (1080, 1920, 100, (512, 1000, 375)),
    ] {
        let (sw, wide, tall) = layout(&Panel::new(w, h, w), aspect);
        assert_eq!(sw, want.0, "{w}x{h}@{aspect}");
        assert!(
            wide.abs_diff(want.1) <= 2 && tall.abs_diff(want.2) <= 2,
            "{w}x{h}@{aspect}: {wide} {tall}"
        );
    }
}

/// Pine: the 512-wide screen centred in a 1094-wide view, a bezel against
/// it and the desktop pattern beyond.
#[test]
fn pine_centres_the_screen_between_desk_and_bezel() {
    let (_, b) = build(1920, 1080, 180, &missing(), Engine::spawn());
    let w = b.view.width();
    assert_eq!((w, b.x0), (1094, 291));
    let row = &b.pix[100 * w..][..w];
    assert_eq!(row[b.x0 - 1], BEZEL);
    assert_eq!(row[b.x0 + W], BEZEL);
    assert!(matches!(row[0], DESK_DARK | DESK_LIGHT));
    assert_ne!(row[0], row[1]);
}

/// No files: a card naming every missing one, drawn once, no engine ever
/// claimed, and nothing allocated frame to frame.
#[test]
fn missing_files_show_a_card_and_never_start_a_mac() {
    let problems = missing().problems().join("\n");
    for key in ["MAC_ROM", "MAC_SYSTEM", "MAC_DISK"] {
        assert!(problems.contains(key), "{problems}");
    }
    let (p, mut b) = build(1920, 1080, 180, &missing(), Engine::spawn());
    assert!(b.claim.want.is_none() && b.carded);
    let mut buf = vec![0u32; p.buf_len()];
    saver::frame(&mut b, &mut buf, &p);
    assert!(b.claim.engine.is_none());
    let n = allocs_during(|| {
        for _ in 0..20 {
            assert!(saver::frame(&mut b, &mut buf, &p).is_empty());
        }
    });
    assert_eq!(n, 0, "render allocated");
    let ink = b.palette[INK as usize];
    let lit = b.palette[LIT as usize];
    // The alert's middle: white with text in it, where the desktop is a
    // checkerboard.
    let rows = (450..630).map(|y| &buf[y * 1920 + 700..][..520]);
    let inked: usize = rows.map(|r| r.iter().filter(|&&c| c == ink).count()).sum();
    let all = 180 * 520;
    assert!(inked > 0 && inked < all / 4, "{inked} of {all} ink");
    assert_ne!(lit, ink);
}

#[test]
fn files_are_checked_for_a_plus_rom_and_an_hfs_disk() {
    let dir = scratch("check");
    let mut f = fake_files(&dir, "BattleChess", dir.join("disk"));
    assert!(f.problems().is_empty(), "{:?}", f.problems());
    assert_eq!(
        volume_name(Path::new(&f.disk)).as_deref(),
        Some("BattleChess")
    );
    std::fs::write(dir.join("short"), [0x4D, 0x1F, 0x81, 0x72]).unwrap();
    f.rom = dir.join("short").to_string_lossy().into();
    f.disk = f.rom.clone();
    let p = f.problems().join("\n");
    assert!(
        p.contains("not a Mac Plus ROM") && p.contains("not an HFS"),
        "{p}"
    );
    f.engine = dir.join("no-engine");
    assert!(f.problems().join("\n").contains("mac-engine"));
    std::fs::remove_dir_all(dir).unwrap();
}

/// The panel path: a new Mac screen scales onto the panel, a frame that
/// flips a few pixels reports only around them, an unchanged one nothing,
/// and every change is inside the damage.
#[test]
fn screens_scale_onto_the_panel_and_report_exactly_what_moved() {
    for (w, h, aspect) in [(1920, 1080, 180), (1920, 1080, 100), (1024, 768, 100)] {
        let (p, mut b) = build(w, h, aspect, &missing(), Engine::spawn());
        let case = format!("{w}x{h}@{aspect}");
        let mut rng = 5u32;
        for byte in &mut *b.bits {
            *byte = next_rand(&mut rng) as u8;
        }
        let mut buf = vec![0u32; p.buf_len()];
        let mut hw = vec![0xDEAD_BEEF; p.buf_len()];
        for step in 0..3 {
            match step {
                0 => b.unpack(),
                1 => {
                    b.bits[100 * W / 8 + 3] ^= 0x10;
                    b.unpack();
                }
                _ => {}
            }
            let before = buf.clone();
            let mut dmg = None;
            let n = allocs_during(|| dmg = Some(saver::frame(&mut b, &mut buf, &p)));
            assert_eq!(n, 0, "{case} step {step}: render allocated");
            let dmg = dmg.unwrap();
            verify(&before, &buf, &dmg, &p, step).unwrap();
            for r in dmg.runs() {
                for y in usize::from(r.y0)..usize::from(r.y1) {
                    let row = y * w + usize::from(r.x0)..y * w + usize::from(r.x1);
                    hw[row.clone()].copy_from_slice(&buf[row]);
                }
            }
            assert!(hw == buf, "{case} step {step}: drawn but never reported");
            match step {
                1 => assert!(dmg.px() < w * 10, "{case}: {:?}", dmg.runs()),
                2 => assert!(dmg.is_empty(), "{case}: an old screen dirtied rows"),
                _ => {}
            }
        }
        // The flipped Mac pixel, on the panel: x 3*8+3, row 100.
        let (x0, x1, y0, y1) = b.view.px_rect();
        let sw = b.view.width();
        let (px, py) = (b.x0 + 27, 100);
        let x = x0 + (px * (x1 - x0)).div_ceil(sw);
        let y = y0 + (py * (y1 - y0)).div_ceil(H);
        let want = b.palette[usize::from(b.bits[100 * W / 8 + 3] >> 4 & 1)];
        assert_eq!(buf[y * w + x], want, "{case}");
    }
}

/// An engine that dies at once, three times running, is given up on, and the
/// saver says so; it never takes the frame loop with it.
#[test]
fn an_engine_that_keeps_dying_shows_the_failure_card() {
    let dir = scratch("dies");
    let script = dir.join("mac-engine");
    std::fs::write(&script, "#!/bin/sh\nexit 3\n").unwrap();
    std::fs::set_permissions(&script, Permissions::from_mode(0o755)).unwrap();
    let files = fake_files(&dir, "Bat", script);
    let e = Engine::spawn();
    let (p, mut b) = build(1920, 1080, 180, &files, e);
    let mut buf = vec![0u32; p.buf_len()];
    let t0 = Instant::now();
    while e.state() != State::Failed {
        assert!(t0.elapsed() < Duration::from_secs(10), "never gave up");
        saver::frame(&mut b, &mut buf, &p);
        std::thread::sleep(Duration::from_millis(5));
    }
    saver::frame(&mut b, &mut buf, &p);
    assert!(b.carded);
    std::fs::remove_dir_all(dir).unwrap();
}

/// The real thing, with the user's own files (never in the repo): boots,
/// opens Battle Chess, hands it to the Mac and the board moves. Unpaced, so
/// minutes of Mac time pass in seconds. Needs `cargo build --features mac`
/// first, for `mac-engine`.
#[test]
fn the_mac_boots_and_plays_battle_chess() {
    let (Ok(rom), Ok(system), Ok(disk)) = (
        std::env::var("MAC_TEST_ROM"),
        std::env::var("MAC_TEST_SYSTEM"),
        std::env::var("MAC_TEST_DISK"),
    ) else {
        eprintln!("skipped: set MAC_TEST_ROM, MAC_TEST_SYSTEM and MAC_TEST_DISK");
        return;
    };
    let exe = std::env::current_exe().unwrap();
    let engine = exe
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .join("mac-engine");
    let files = Files {
        rom,
        system,
        disk,
        engine,
    };
    assert!(files.problems().is_empty(), "{:?}", files.problems());
    let e = Engine::spawn();
    let (p, mut b) = build(1920, 1080, 180, &files, e);
    let mut buf = vec![0u32; p.buf_len()];
    let t0 = Instant::now();
    while !e.playing() {
        assert!(
            t0.elapsed() < Duration::from_secs(120),
            "never reached the game"
        );
        assert_ne!(e.state(), State::Failed);
        saver::frame(&mut b, &mut buf, &p);
        std::thread::sleep(Duration::from_millis(20));
    }
    // The board itself moving, not just the menu bar's clock.
    let mut board = b.bits[20 * W / 8..].to_vec();
    let mut moves = 0;
    let t1 = Instant::now();
    while moves < 20 {
        assert!(t1.elapsed() < Duration::from_secs(30), "the board stopped");
        saver::frame(&mut b, &mut buf, &p);
        if b.bits[20 * W / 8..] != board[..] {
            board.copy_from_slice(&b.bits[20 * W / 8..]);
            moves += 1;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let n = allocs_during(|| {
        for _ in 0..50 {
            saver::frame(&mut b, &mut buf, &p);
        }
    });
    assert_eq!(n, 0, "render allocated");
}
