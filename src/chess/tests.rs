use std::sync::atomic::{AtomicBool, AtomicU64};
use std::time::{Duration, Instant};

use super::game::{Phase, MAX_PLIES};
use super::rules::{ended, End, Moves, Pos, BLACK, KING, KNIGHT, PAWN, QUEEN, ROOK};
use super::search::{level_limits, Limits, Searcher};
use super::*;
use crate::grid::{with_test_aspect, Cell};
use crate::saver;
use crate::testalloc::allocs_during;

/// A position from FEN's first four fields.
fn fen(f: &str) -> Pos {
    let mut p = Pos::start();
    let parts: Vec<&str> = f.split(' ').collect();
    p.sq = [0; 64];
    for (i, row) in parts[0].split('/').enumerate() {
        let mut file = 0;
        for c in row.chars() {
            if let Some(d) = c.to_digit(10) {
                file += d as usize;
                continue;
            }
            let k = match c.to_ascii_lowercase() {
                'p' => PAWN,
                'n' => KNIGHT,
                'b' => super::rules::BISHOP,
                'r' => ROOK,
                'q' => QUEEN,
                _ => KING,
            };
            let s = (7 - i) * 8 + file;
            p.sq[s] = k | if c.is_ascii_lowercase() { BLACK } else { 0 };
            if k == KING {
                p.kings[usize::from(c.is_ascii_lowercase())] = s as u8;
            }
            file += 1;
        }
    }
    p.stm = usize::from(parts[1] == "b");
    p.castle = 0;
    for (c, bit) in [('K', 1), ('Q', 2), ('k', 4), ('q', 8)] {
        if parts[2].contains(c) {
            p.castle |= bit;
        }
    }
    p.ep = match parts[3].as_bytes() {
        [f, r] => (r - b'1') * 8 + (f - b'a'),
        _ => 64,
    };
    p.half = 0;
    p.hash = p.compute_hash();
    p
}

fn perft(p: &mut Pos, depth: u32) -> u64 {
    let mut l = Moves::new();
    p.legal(&mut l);
    if depth == 1 {
        return l.n as u64;
    }
    let mut n = 0;
    for &m in l.as_slice() {
        let u = p.make(m);
        assert_eq!(p.hash, p.compute_hash(), "incremental hash drifted");
        n += perft(p, depth - 1);
        p.unmake(m, &u);
    }
    n
}

/// The standard perft suite: start, Kiwipete (castling through and out of
/// check), an en-passant-pin position and a promotion maze.
#[test]
fn move_generation_matches_perft() {
    let cases: [(&str, u32, u64); 4] = [
        (
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq -",
            4,
            197_281,
        ),
        (
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq -",
            3,
            97_862,
        ),
        ("8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - -", 4, 43_238),
        (
            "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq -",
            3,
            9_467,
        ),
    ];
    for (f, d, want) in cases {
        let mut p = fen(f);
        let before = p;
        assert_eq!(perft(&mut p, d), want, "{f}");
        assert_eq!(p, before, "unmake did not restore {f}");
    }
}

fn play(p: &mut Pos, ucis: &str) -> Vec<String> {
    ucis.split(' ')
        .map(|u| {
            let m = p.parse_uci(u).unwrap_or_else(|| panic!("{u} illegal"));
            let san = String::from_utf8(p.san(m).bytes().to_vec()).unwrap();
            p.make(m);
            san
        })
        .collect()
}

#[test]
fn san_names_moves_as_a_scoresheet_does() {
    let mut p = Pos::start();
    assert_eq!(
        play(&mut p, "f2f3 e7e5 g2g4 d8h4"),
        ["f3", "e5", "g4", "Qh4#"]
    );
    let mut p = Pos::start();
    let sans = play(&mut p, "e2e4 d7d5 e4d5 g8f6 g1f3 f6d5 f1c4 c7c6 e1g1 c8g4");
    assert_eq!(
        sans,
        ["e4", "d5", "exd5", "Nf6", "Nf3", "Nxd5", "Bc4", "c6", "O-O", "Bg4"]
    );
    // Both rooks can reach d1: the file tells them apart.
    let p = fen("4k3/8/8/8/8/8/8/R4RK1 w - -");
    let m = p.parse_uci("a1d1").unwrap();
    assert_eq!(p.san(m).bytes(), b"Rad1");
    // A promotion with check.
    let p = fen("4k3/1P6/8/8/8/8/8/4K3 w - -");
    assert_eq!(p.san(p.parse_uci("b7b8q").unwrap()).bytes(), b"b8=Q+");
}

#[test]
fn every_way_a_game_ends_is_recognised() {
    let mut p = Pos::start();
    play(&mut p, "f2f3 e7e5 g2g4 d8h4");
    assert_eq!(ended(&p, &[p.hash], MAX_PLIES), Some(End::Mate(1)));

    let p = fen("7k/5Q2/6K1/8/8/8/8/8 b - -");
    assert_eq!(ended(&p, &[p.hash], MAX_PLIES), Some(End::Stalemate));

    let p = fen("7k/8/6K1/8/8/8/8/2B5 b - -");
    assert_eq!(ended(&p, &[p.hash], MAX_PLIES), Some(End::Material));

    let mut p = Pos::start();
    let mut hist = vec![p.hash];
    for u in "g1f3 g8f6 f3g1 f6g8 g1f3 g8f6 f3g1 f6g8".split(' ') {
        let m = p.parse_uci(u).unwrap();
        p.make(m);
        hist.push(p.hash);
    }
    assert_eq!(ended(&p, &hist, MAX_PLIES), Some(End::Repetition));

    let mut p = fen("4k3/8/8/8/8/8/8/R3K3 w - -");
    p.half = 100;
    assert_eq!(ended(&p, &[p.hash], MAX_PLIES), Some(End::Fifty));
}

fn search(p: &Pos, depth: u8) -> (super::rules::Move, i32, u8) {
    let lim = Limits {
        depth,
        budget: Duration::from_secs(30),
        noise: 0,
        seed: 0,
    };
    Searcher::new().think(
        p,
        &[p.hash],
        lim,
        &AtomicBool::new(false),
        &AtomicU64::new(0),
    )
}

#[test]
fn the_engine_mates_and_takes_what_hangs() {
    let p = fen("6k1/5ppp/8/8/8/8/8/R5K1 w - -");
    let (m, score, _) = search(&p, 3);
    assert_eq!((m.from, m.to), (0, 56), "back-rank mate");
    assert!(score > search::MATE - 10);

    let p = fen("4k3/8/8/3q4/8/8/3R4/4K3 w - -");
    let (m, _, _) = search(&p, 3);
    assert_eq!((m.from, m.to), (11, 35), "Rxd5");
}

/// Whole games, engine against engine, to a result. Fast levels so the test
/// stays a test; `long_games_report` plays real ones.
fn play_games(n: u32, think: Duration, quiet: bool) -> Vec<(End, usize)> {
    let mut out = Vec::new();
    let mut searchers = [Searcher::new(), Searcher::new()];
    let (abort, progress) = (AtomicBool::new(false), AtomicU64::new(0));
    for g in 0..n {
        let mut p = Pos::start();
        let mut hist = vec![p.hash];
        let levels = [1 + (g % 5) as u8, 1 + ((g * 3 + 2) % 5) as u8];
        let end = loop {
            if let Some(e) = ended(&p, &hist, MAX_PLIES) {
                break e;
            }
            let side = p.stm;
            let lim = level_limits(levels[side], think, u64::from(g) * 2 + side as u64);
            let (m, _, _) = searchers[side].think(&p, &hist, lim, &abort, &progress);
            assert!(
                p.find(m.from, m.to, m.promo).is_some(),
                "illegal engine move"
            );
            p.make(m);
            hist.push(p.hash);
        };
        if !quiet {
            println!(
                "game {g}: levels {levels:?} -> {} by {} in {} plies",
                end.score(),
                end.reason(),
                hist.len() - 1
            );
        }
        out.push((end, hist.len() - 1));
    }
    out
}

#[test]
fn engine_games_reach_a_result() {
    for (end, plies) in play_games(2, Duration::from_millis(20), true) {
        assert!(plies <= MAX_PLIES + 1, "{end:?} after {plies}");
    }
}

/// `cargo test --release long_games_report -- --ignored --nocapture`
#[test]
#[ignore = "plays full-strength games; minutes"]
fn long_games_report() {
    let t0 = Instant::now();
    let games = play_games(8, Duration::from_millis(300), false);
    let decisive = games
        .iter()
        .filter(|(e, _)| matches!(e, End::Mate(_)))
        .count();
    println!("{decisive}/{} decisive, {:?}", games.len(), t0.elapsed());
}

fn knobs(think: u64, games: usize, level: u8) -> Knobs {
    Knobs {
        think,
        games,
        level,
        result: 1,
        fight_secs: 1,
        seed: 11,
    }
}

fn build(w: usize, h: usize, aspect: usize, k: &Knobs) -> (Panel, Chess) {
    let p = Panel::new(w, h, w);
    let c = with_test_aspect(aspect, || Chess::build(&p, 30, k));
    (p, c)
}

const SHAPES: [(&str, usize, usize, usize); 6] = [
    ("pine 3.2:1", 1920, 1080, 180),
    ("16:9", 1920, 1080, 100),
    ("4:3", 1024, 768, 100),
    ("square", 1080, 1080, 100),
    ("portrait", 1080, 1920, 100),
    ("tiny", 128, 128, 100),
];

fn inside(r: (i32, i32, i32, i32), o: (i32, i32, i32, i32)) -> bool {
    r.0 >= o.0 && r.1 >= o.1 && r.0 + r.2 <= o.0 + o.2 && r.1 + r.3 <= o.1 + o.3
}

fn overlap(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)) -> bool {
    a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
}

/// Every shape gets boards that fit, a side panel that does not cover them,
/// and a picture that spans the panel rather than a box marooned in black.
#[test]
fn every_shape_is_filled_edge_to_edge() {
    for (name, w, h, aspect) in SHAPES {
        let (p, mut c) = build(w, h, aspect, &knobs(1500, 0, 0));
        let mut buf = vec![0u32; p.buf_len()];
        saver::frame(&mut c, &mut buf, &p);
        let (cols, rows) = (c.canvas.grid.cols() as i32, c.canvas.grid.rows() as i32);
        let whole = (0, 0, cols, rows);
        for l in &c.layouts {
            let board = (l.bx, l.by, l.sq * 8, l.sq * 8);
            assert!(
                inside(board, l.slot) && inside(l.slot, whole),
                "{name}: board outside"
            );
            assert!(
                inside(l.ev, l.slot) && !overlap(l.ev, board),
                "{name}: eval bar"
            );
            assert!(!overlap(l.info, board), "{name}: panel over the board");
            if name != "tiny" {
                assert!(inside(l.info, l.slot), "{name}: panel outside");
                assert!(l.sq >= 24, "{name}: squares of {}", l.sq);
                assert!(
                    l.info.2 >= 100 && l.info.3 >= 100,
                    "{name}: panel {:?}",
                    l.info
                );
            }
        }
        if name == "tiny" {
            continue;
        }
        let cells = c.canvas.grid.cells();
        let bg = Cell::new(crate::font::SOLID, BG);
        let (mut x0, mut x1, mut y0, mut y1) = (cols, 0, rows, 0);
        for (i, &cell) in cells.iter().enumerate() {
            if cell != bg {
                let (x, y) = ((i as i32) % cols, (i as i32) / cols);
                (x0, x1, y0, y1) = (x0.min(x), x1.max(x), y0.min(y), y1.max(y));
            }
        }
        let (span_x, span_y) = ((x1 - x0 + 1) * 100 / cols, (y1 - y0 + 1) * 100 / rows);
        assert!(
            span_x >= 94 && span_y >= 94,
            "{name}: drawn {span_x}% x {span_y}%"
        );
    }
    let pine = build(1920, 1080, 180, &knobs(1500, 0, 0)).1;
    assert_eq!(pine.layouts.len(), 2, "pine shows two games side by side");
    assert_eq!(
        build(1920, 1080, 100, &knobs(1500, 0, 0)).1.layouts.len(),
        1
    );
    assert_eq!(
        build(1920, 1080, 100, &knobs(1500, 3, 0)).1.layouts.len(),
        3
    );
}

/// Through whole games and into the next, the frame loop allocates nothing
/// and every changed pixel is reported, with junk in the panel's copy so an
/// unreported write shows.
#[test]
fn whole_games_never_allocate_and_report_every_pixel() {
    let (p, mut c) = build(1920, 1080, 180, &knobs(100, 0, 1));
    let mut buf = vec![0x00AB_CDEFu32; p.buf_len()];
    saver::frame(&mut c, &mut buf, &p);
    let mut prev = buf.clone();
    let t0 = Instant::now();
    let mut finished = [false; 2];
    let mut frames = 0;
    while !finished.iter().all(|&f| f) {
        assert!(
            t0.elapsed() < Duration::from_secs(120),
            "no game finished: {:?}",
            c.games[0].phase
        );
        let mut d = None;
        let n = allocs_during(|| d = Some(saver::frame(&mut c, &mut buf, &p)));
        assert_eq!(n, 0, "frame {frames} allocated in {:?}", c.games[0].phase);
        crate::dump::verify(&prev, &buf, &d.unwrap(), &p, frames).unwrap();
        prev.copy_from_slice(&buf);
        for (f, g) in finished.iter_mut().zip(&c.games) {
            *f |= g.plies == 0 && matches!(g.phase, Phase::Wait { .. }) && frames > 100;
        }
        frames += 1;
        if frames % 64 == 0 {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

/// Unchanged frames cost nothing: while both engines think, only the
/// thinking indicators move.
#[test]
fn a_thinking_frame_touches_a_sliver_of_the_panel() {
    let (p, mut c) = build(1920, 1080, 100, &knobs(20_000, 0, 5));
    let mut buf = vec![0u32; p.buf_len()];
    saver::frame(&mut c, &mut buf, &p);
    let mut worst = 0;
    for _ in 0..400 {
        let d = saver::frame(&mut c, &mut buf, &p);
        let fighting = c.games[0].fight.is_some();
        if !fighting && matches!(c.games[0].phase, Phase::Think { asked: Some(_), .. }) {
            worst = worst.max(d.px());
        }
    }
    assert!(worst > 0, "never reached a think");
    assert!(worst < p.w * p.h / 20, "{worst} px a thinking frame");
}

/// One of each fight, staged from a position: `(name, FEN, capture)`.
const FIGHTS: [(&str, &str, &str); 13] = [
    ("jab", "4k3/8/8/3n4/4P3/8/8/4K3 w - -", "e4d5"),
    ("charge", "4k3/8/8/3b4/8/4N3/8/4K3 w - -", "e3d5"),
    ("cast", "4k3/8/8/3p4/8/1B6/8/4K3 w - -", "b3d5"),
    ("slam", "4k3/8/8/3n4/8/8/3R4/4K3 w - -", "d2d5"),
    ("zap", "4k3/8/8/8/3r4/8/8/Q3K3 w - -", "a1d4"),
    ("swing", "4k3/8/8/8/8/8/3p4/4K3 w - -", "e1d2"),
    ("duel", "4k3/8/8/3q4/8/8/8/3QK3 w - -", "d1d5"),
    ("crumble", "4k3/8/8/3r4/8/4N3/8/4K3 w - -", "e3d5"),
    ("comedy", "4k3/8/8/3q4/4P3/8/8/4K3 w - -", "e4d5"),
    ("en-passant", "4k3/8/8/3pP3/8/8/8/4K3 w - d6", "e5d6"),
    ("promotion", "3r1k2/4P3/8/8/8/8/8/4K3 w - -", "e7d8q"),
    ("black", "4k3/8/8/3p4/4N3/8/8/4K3 b - -", "d5e4"),
    ("mate", "3r2k1/5ppp/8/8/8/8/5PPP/3R2K1 w - -", "d1d8"),
];

/// The attacker stands most of a square to the victim's side: on an edge
/// file that has to be the board side, whichever way it came from.
#[test]
fn a_fight_on_an_edge_file_stays_on_the_board() {
    use super::fight::{Fight, Flourish, Style};
    for (f, uci, dir, style) in [
        ("4k3/8/8/n7/8/8/8/R3K3 w - -", "a1a5", -1, Style::Slam),
        ("4k2r/8/8/7N/8/8/8/4K3 b - -", "h8h5", 1, Style::Slam),
        ("4k3/8/8/3r4/8/4N3/8/4K3 w - -", "e3d5", -1, Style::Crumble),
        ("4k3/8/8/3r4/8/2N5/8/4K3 w - -", "c3d5", 1, Style::Crumble),
    ] {
        let pos = fen(f);
        let m = pos.parse_uci(uci).unwrap();
        let fight = Fight::new(&pos.sq, m, Flourish::None, 90);
        assert_eq!((fight.dir, fight.style), (dir, style), "{uci}");
    }
}

/// A capture is fought out, the fight ends, and the game goes on: the board
/// shows the position after the move and the next move comes.
#[test]
fn every_fight_ends_and_the_game_goes_on() {
    let dir = std::env::var("FIGHT_SHEET_DIR").ok();
    for (name, f, uci) in FIGHTS {
        let mut k = knobs(100, 1, 1);
        if dir.is_some() {
            k.fight_secs = 3;
        }
        let (p, mut c) = build(1920, 1080, 100, &k);
        let mut buf = vec![0x00AB_CDEFu32; p.buf_len()];
        saver::frame(&mut c, &mut buf, &p);
        let pos = fen(f);
        let m = pos.parse_uci(uci).unwrap();
        let g = &mut c.games[0];
        g.pos = pos;
        g.hist[0] = pos.hash;
        g.phase = Phase::Wait { until: u64::MAX };
        g.glide(m, c.now);
        assert!(c.games[0].fight.is_some(), "{name}: no fight");
        let mut prev = buf.clone();
        let mut frames = 0;
        while c.games[0].fight.is_some() {
            let mut d = None;
            let n = allocs_during(|| d = Some(saver::frame(&mut c, &mut buf, &p)));
            assert_eq!(n, 0, "{name}: frame {frames} allocated");
            crate::dump::verify(&prev, &buf, &d.unwrap(), &p, frames).unwrap();
            prev.copy_from_slice(&buf);
            if let Some(dir) = &dir {
                if frames % 5 == 0 {
                    write_board(&c, &buf, &p, &format!("{dir}/{name}-{frames:03}.ppm"));
                }
            }
            frames += 1;
            assert!(frames < 200, "{name}: fight never ended");
        }
        let after = c.games[0].pos.sq;
        assert_ne!(after, pos.sq, "{name}: move not played");
        c.games[0].phase = Phase::Wait { until: 0 };
        let plies = c.games[0].plies;
        let t0 = Instant::now();
        while c.games[0].plies == plies && c.games[0].end.is_none() {
            saver::frame(&mut c, &mut buf, &p);
            std::thread::sleep(Duration::from_millis(1));
            assert!(
                t0.elapsed() < Duration::from_secs(20),
                "{name}: game stalled"
            );
        }
    }
}

/// The board's pixels, for `FIGHT_SHEET_DIR` contact sheets.
fn write_board(c: &Chess, buf: &[u32], p: &Panel, path: &str) {
    let l = &c.layouts[0];
    let cell = p.w / c.canvas.grid.cols();
    let (x0, y0, side) = (
        l.bx as usize * cell,
        l.by as usize * cell,
        l.sq as usize * 8 * cell,
    );
    let stride = buf.len() / p.h;
    let mut out = format!("P6 {side} {side} 255\n").into_bytes();
    for y in y0..y0 + side {
        for &px in &buf[y * stride + x0..][..side] {
            out.extend([(px >> 16) as u8, (px >> 8) as u8, px as u8]);
        }
    }
    std::fs::write(path, out).unwrap();
}
