use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

use super::http::{handle, json_str, param, stat_json, APPLY_WAIT, NEVER};
use super::*;
use crate::config;

/// Describe a scene whose panel is exactly the cell grid — enough for the
/// tests that need SOME geometry and do not care which.
fn scene(m: &Mirror, saver: &str, cols: usize, rows: usize, cw: usize, ch: usize) {
    let panel = Panel::new(cols * cw, rows * ch, cols * cw);
    m.describe(saver, &Grid::new(&panel, cw, ch), &panel, &[0, 0xFF]);
}

/// `/meta` must carry the PANEL, because that is the rectangle the page
/// sizes its canvas to. `rows = panel.h / cell_h` truncates: matrix's 32px
/// cell leaves a 24px strip under the grid, so a page sized to
/// `rows * cell_h` draws a 1.818 image of a 1.778 panel and stretches every
/// block 2.3% vertically. Matrix on a real 1080p panel, because a geometry
/// where the cell divides evenly cannot tell the two apart.
#[test]
fn meta_reports_the_panel_not_the_cell_grid() {
    let m = Mirror::new(15);
    let panel = Panel::new(1920, 1080, 1920);
    let g = Grid::new(&panel, 16, 32);
    m.describe("matrix", &g, &panel, &[0, 0xFF]);
    // The gap this whole test is about: the grid is 24px short of the panel.
    assert_eq!((g.cols(), g.rows()), (120, 33));
    assert_eq!(g.rows() * g.cell_h(), 1056);

    // Geometry only: the glyph table is 260 rows of sixteen bytes, and a
    // failure that prints it is a failure nobody can read.
    let meta = m.meta.lock().unwrap().clone();
    let head = meta.split(",\"palette\"").next().unwrap().to_string();
    assert!(head.contains("\"panel_w\":1920"), "{head}");
    assert!(head.contains("\"panel_h\":1080"), "{head}");
    // And the grid is still reported, since the cells are addressed by it.
    assert!(head.contains("\"cols\":120,\"rows\":33"), "{head}");
    assert!(head.contains("\"cell_w\":16,\"cell_h\":32"), "{head}");
    // 0 when nobody has measured the panel, which is what makes the page
    // hide its actual-size control rather than offer a wrong millimetre.
    assert!(head.contains("\"panel_mm\":0"), "{head}");
    // The page squashes its canvas by this to show what the wall shows.
    // Without it in /meta the browser renders the pre-distorted picture.
    assert!(head.contains("\"pixel_aspect\":100"), "{head}");
}

/// The page divides the canvas by `pixel_aspect` to undo the stretch the
/// renderer applied for the panel's benefit. If `/meta` reports a number
/// the renderer did not use, the mirror silently shows the wrong shape —
/// which is invisible in review and only shows up as "the web version does
/// not match the screen".
#[test]
fn meta_reports_the_aspect_the_renderer_actually_used() {
    for pct in [100usize, 180, 250] {
        let (g, panel) = crate::grid::with_test_aspect(pct, || {
            let panel = Panel::new(1920, 1080, 1920);
            let g = Grid::new(&panel, 16, 32);
            (g, panel)
        });
        // The cell really is that much taller than the env said. Rounded,
        // not truncated -- 32 * 1.8 is 57.6 and the cell is 58.
        assert_eq!(g.cell_h(), (32 * pct + 50) / 100, "aspect {pct}");

        let m = Mirror::new(15);
        crate::grid::with_test_aspect(pct, || {
            m.describe("matrix", &g, &panel, &[0, 0xFF]);
        });
        let meta = m.meta.lock().unwrap().clone();
        let head = meta.split(",\"palette\"").next().unwrap().to_string();
        assert!(
            head.contains(&format!("\"pixel_aspect\":{pct}")),
            "aspect {pct}: {head}"
        );
    }
}

/// The keyframe-on-connect trick is the sentinel: a viewer's `prev` filled
/// with a cell no saver can produce diffs against everything.
#[test]
fn the_sentinel_cell_is_unreachable() {
    for (g, glyph) in font::GLYPHS.iter().enumerate() {
        let _ = glyph;
        assert_ne!(Cell::new(g as u16, 0), NEVER);
    }
    assert_eq!(NEVER.glyph(), u16::MAX as usize);
}

/// Publishing with nobody watching must not even take the lock — that is
/// the whole no-viewer-no-cost claim, and it is one line that could rot.
#[test]
fn no_viewer_means_no_publish() {
    let m = Mirror::new(15);
    m.publish(&[Cell::new(1, 2)]);
    assert_eq!(m.frame.lock().unwrap().gen, 0);

    m.viewers.fetch_add(1, Ordering::Relaxed);
    m.publish(&[Cell::new(1, 2)]);
    let f = m.frame.lock().unwrap();
    assert_eq!((f.gen, f.cells.len()), (1, 1));
}

/// The whole server, over a real socket on an ephemeral port: routing, the
/// gate-defeating header, the chunk framing, and the keyframe-on-connect.
/// `cargo test` runs in CI, which has no card and no monitor, so this is
/// the only place any of that is exercised before it reaches a node.
#[test]
fn a_viewer_gets_a_keyframe_then_deltas() {
    let m = Mirror::new(15);
    scene(&m, "test", 2, 2, 8, 16);
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    {
        let m = Arc::clone(&m);
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let m = Arc::clone(&m);
                std::thread::spawn(move || handle(&m, s));
            }
        });
    }

    let get = |path: &str| {
        let mut s = TcpStream::connect(addr).unwrap();
        s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes())
            .unwrap();
        s
    };
    let mut body = String::new();
    get("/meta").read_to_string(&mut body).unwrap();
    assert!(body.starts_with("HTTP/1.1 200 "), "{body}");
    assert!(body.contains("\"cols\":2"), "{body}");
    let mut body = String::new();
    get("/nope").read_to_string(&mut body).unwrap();
    assert!(body.starts_with("HTTP/1.1 404 "), "{body}");

    let mut s = get("/stream");
    // Wait for the viewer to register before publishing, else the frame is
    // dropped by the no-viewer fast path and this test races.
    while m.viewers.load(Ordering::Relaxed) == 0 {
        std::thread::yield_now();
    }
    let cells = [
        Cell::new(0, 0),
        Cell::new(1, 1),
        Cell::new(0, 0),
        Cell::new(0, 0),
    ];
    m.publish(&cells);
    // Read until the chunk marker is complete, not once: a socket read may
    // return fewer bytes than were written, so a single read can split the
    // response mid-marker and fail a correct stream.
    let mut got = Vec::new();
    let mut buf = [0u8; 512];
    let head = loop {
        let n = s.read(&mut buf).unwrap();
        assert!(n > 0, "stream closed early: {got:?}");
        got.extend_from_slice(&buf[..n]);
        let head = String::from_utf8_lossy(&got).to_string();
        if head.contains("\r\n\r\n24\r\n") || got.len() > 4096 {
            break head;
        }
    };
    // Without this nginx buffers the stream and the mirror runs behind.
    assert!(head.contains("X-Accel-Buffering: no"), "{head}");
    // Chunk of 4 + 4 cells * 8 = 36 bytes: a connect always keyframes.
    assert!(head.contains("\r\n\r\n24\r\n"), "{head}");
}

/// /select is the one route that changes what the panel draws, so it is the
/// one route where a bad request must not be taken at face value: an
/// unknown name 400s and leaves the selection alone. Over a real socket,
/// because the method check and the query parsing are both on that path.
#[test]
fn select_switches_the_saver_and_refuses_a_name_that_is_not_one() {
    let m = Mirror::new(15);
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    {
        let m = Arc::clone(&m);
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let m = Arc::clone(&m);
                std::thread::spawn(move || handle(&m, s));
            }
        });
    }
    let req = |line: &str| {
        let mut s = TcpStream::connect(addr).unwrap();
        s.write_all(format!("{line} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes())
            .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    };

    assert_eq!(m.selected(), 0);
    let body = req("POST /select?saver=matrix");
    assert!(body.starts_with("HTTP/1.1 200 "), "{body}");
    assert!(body.contains("\"saver\":\"matrix\""), "{body}");
    assert_eq!(crate::saver::name_at(m.selected()), "matrix");

    for bad in [
        "POST /select?saver=nope",   // not a saver
        "POST /select?saver=",       // truncated away
        "POST /select?saver=MATRIX", // names are exact
        "POST /select",              // no parameter at all
        "POST /select?other=ascii",  // wrong parameter
    ] {
        let body = req(bad);
        assert!(body.starts_with("HTTP/1.1 400 "), "{bad}: {body}");
        assert_eq!(crate::saver::name_at(m.selected()), "matrix", "{bad}");
    }

    // A GET must not be able to change the panel. 405, not 404: the route
    // exists, the method is what is wrong.
    let body = req("GET /select?saver=ascii");
    assert!(body.starts_with("HTTP/1.1 405 "), "{body}");
    assert_eq!(crate::saver::name_at(m.selected()), "matrix");

    // Adding a method column to the router deleted the parse-layer check
    // that used to reject every non-GET, which let POST /stream take one of
    // the four viewer slots and hold a thread open on it.
    for bad in ["POST /stream", "PUT /meta", "DELETE /"] {
        let body = req(bad);
        assert!(body.starts_with("HTTP/1.1 405 "), "{bad}: {body}");
    }
    assert_eq!(m.viewers.load(Ordering::Relaxed), 0, "a slot was taken");
}

/// The other route that changes what the panel does, held to the same bar:
/// a POST moves the interval, a GET cannot, and anything that is not a
/// number of minutes in range is a 400 that changes NOTHING — a lenient
/// parse of "5x" or "" would turn rotation off, which is the one outcome
/// nobody asked for. `/meta` is checked here too, because a page that
/// reads the interval from a string cached at modeset shows the value it
/// had before the POST and there is no second place to notice that.
#[test]
fn rotate_sets_the_interval_and_refuses_anything_that_is_not_minutes() {
    let m = Mirror::new(15);
    scene(&m, "matrix", 2, 2, 8, 16);
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    {
        let m = Arc::clone(&m);
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let m = Arc::clone(&m);
                std::thread::spawn(move || handle(&m, s));
            }
        });
    }
    let req = |line: &str| {
        let mut s = TcpStream::connect(addr).unwrap();
        s.write_all(format!("{line} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes())
            .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    };
    // Geometry only — the glyph table is 260 rows nobody can read.
    let meta_tail = || {
        let body = req("GET /meta");
        body.rsplit_once("]],").unwrap().1.to_string()
    };

    assert_eq!(m.rotate_secs(), 0);
    assert!(meta_tail().contains("\"rotate_secs\":0"), "{}", meta_tail());

    let body = req("POST /rotate?mins=7");
    assert!(body.starts_with("HTTP/1.1 200 "), "{body}");
    assert!(body.contains("\"rotate_secs\":420"), "{body}");
    assert_eq!(m.rotate_secs(), 420);
    assert!(
        meta_tail().contains("\"rotate_secs\":420"),
        "{}",
        meta_tail()
    );

    // 0 is off, and is the one value that is not a mistake.
    let body = req("POST /rotate?mins=0");
    assert!(body.starts_with("HTTP/1.1 200 "), "{body}");
    assert_eq!(m.rotate_secs(), 0);
    req("POST /rotate?mins=7");

    for bad in [
        "POST /rotate?mins=1441", // past a day
        "POST /rotate?mins=-1",   // not unsigned
        "POST /rotate?mins=5.5",  // whole minutes only
        "POST /rotate?mins=5x",   // not a number
        "POST /rotate?mins=",     // truncated away
        "POST /rotate",           // no parameter at all
        "POST /rotate?secs=60",   // wrong parameter
    ] {
        let body = req(bad);
        assert!(body.starts_with("HTTP/1.1 400 "), "{bad}: {body}");
        assert_eq!(m.rotate_secs(), 420, "{bad}");
    }

    // A GET must not be able to change the panel's pace. 405, not 404.
    let body = req("GET /rotate?mins=1");
    assert!(body.starts_with("HTTP/1.1 405 "), "{body}");
    assert_eq!(m.rotate_secs(), 420);
}

/// `/meta` must report the interval the TIMER is actually enforcing, not
/// the number the last POST happened to send: a page that shows 5 while the
/// panel moves every 2 minutes is the same class of bug as an aspect ratio
/// the renderer did not use, and just as invisible in review. So drive the
/// real `saver::switch` and check the two agree.
#[test]
fn meta_reports_the_interval_the_timer_actually_enforces() {
    let panel = Panel::new(128, 128, 128);
    let m = Mirror::new(15);
    let t0 = std::time::Instant::now();
    let place = |n: &str| (panel, saver::make(n, &panel, 30));
    let mut d = saver::Driver::new(&m, 30, place);

    m.set_rotate_secs(120);
    let mut step = |secs: u64| d.switch(t0 + Duration::from_secs(secs), &m, place);
    assert!(!step(0));
    assert!(!step(119), "rotated early");
    assert!(step(120), "did not rotate on time");

    // Over the socket, because the interval is spliced in by the route: a
    // /meta served straight out of the cached string would answer with
    // whatever was set at the last modeset.
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    {
        let m = Arc::clone(&m);
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let m = Arc::clone(&m);
                std::thread::spawn(move || handle(&m, s));
            }
        });
    }
    let mut body = String::new();
    let mut s = TcpStream::connect(addr).unwrap();
    s.write_all(b"GET /meta HTTP/1.1\r\nHost: x\r\n\r\n")
        .unwrap();
    s.read_to_string(&mut body).unwrap();
    let tail = body.rsplit_once("]],").unwrap().1.to_string();
    assert!(tail.contains("\"rotate_secs\":120"), "{tail}");
}

/// Every viewer reconnects on an epoch bump, so a switch must bump it
/// exactly once. The terminal used to announce the saver `switch` built
/// and then the one it rebuilt for itself, and every viewer reconnected
/// twice.
#[test]
fn a_switch_announces_once() {
    let panel = Panel::new(128, 128, 128);
    let m = Mirror::new(15);
    let epoch = || m.frame.lock().unwrap().epoch;
    let place = |n: &str| (panel, saver::make(n, &panel, 30));
    let mut d = saver::Driver::new(&m, 30, place);
    let before = epoch();
    assert!(m.select("dvd"));
    assert!(d.switch(std::time::Instant::now(), &m, place));
    assert_eq!(epoch(), before + 1);
}

/// A `/select` landing between the page's `/meta` and `/stream` fetches
/// used to hand the viewer one saver's geometry, palette and glyph table
/// with another saver's cells, and nothing recovered it. The epoch in
/// `/meta` is what pairs them, so a stream opened against a stale one must
/// be refused rather than served — driven deterministically here, because
/// the real thing is two clicks a few milliseconds apart.
#[test]
fn a_stream_for_a_scene_that_has_been_replaced_is_refused() {
    let m = Mirror::new(15);
    scene(&m, "matrix", 2, 2, 8, 16);
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    {
        let m = Arc::clone(&m);
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let m = Arc::clone(&m);
                std::thread::spawn(move || handle(&m, s));
            }
        });
    }
    let req = |line: &str| {
        let mut s = TcpStream::connect(addr).unwrap();
        s.write_all(format!("{line} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes())
            .unwrap();
        s
    };
    // Bounded rather than read-to-end: a stale /stream that is wrongly
    // SERVED never closes, so an unbounded read would hang this test
    // forever instead of failing it. Timing out with no 409 in hand is the
    // failure, and it arrives in seconds.
    let text = |line: &str| {
        let mut s = req(line);
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut out = Vec::new();
        let mut buf = [0u8; 512];
        while let Ok(n) = s.read(&mut buf) {
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
            if out.len() > 4096 {
                break;
            }
        }
        String::from_utf8_lossy(&out).to_string()
    };

    // What the page does: read /meta, then open /stream with its epoch.
    let meta = text("GET /meta");
    assert!(meta.contains("\"epoch\":1"), "{meta}");

    // The user clicks another saver and the panel modesets — the exact
    // window the page's two fetches straddle.
    scene(&m, "dvd", 4, 4, 8, 16);
    let after = text("GET /meta");
    assert!(after.contains("\"epoch\":2"), "{after}");

    // The stale epoch is refused outright, and takes no viewer slot.
    let stale = text("GET /stream?epoch=1");
    assert!(stale.starts_with("HTTP/1.1 409 "), "{stale}");
    assert_eq!(m.viewers.load(Ordering::Relaxed), 0, "a slot was taken");

    // The epoch the page would now read is served, and keyframes as usual.
    let mut s = req("GET /stream?epoch=2");
    while m.viewers.load(Ordering::Relaxed) == 0 {
        std::thread::yield_now();
    }
    m.publish(&[Cell::new(1, 1); 16]);
    let mut got = Vec::new();
    let mut buf = [0u8; 512];
    let head = loop {
        let n = s.read(&mut buf).unwrap();
        assert!(n > 0, "stream closed early: {got:?}");
        got.extend_from_slice(&buf[..n]);
        let head = String::from_utf8_lossy(&got).to_string();
        if head.contains("\r\n\r\n") && head.len() > 200 || got.len() > 4096 {
            break head;
        }
    };
    assert!(head.starts_with("HTTP/1.1 200 "), "{head}");
    // 4 + 16 cells * 8 = 132 bytes = 0x84.
    assert!(head.contains("\r\n\r\n84\r\n"), "{head}");
}

/// `/stat` must read LIVE — see the route in `handle` for why it is not in
/// `/meta`. A modeset runs between the two reads, so an implementation that
/// served these out of the cached `/meta`, or reset them on modeset, fails.
#[test]
fn stat_reports_counters_live_and_not_as_of_the_last_modeset() {
    let m = Mirror::new(15);
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    {
        let m = Arc::clone(&m);
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let m = Arc::clone(&m);
                std::thread::spawn(move || handle(&m, s));
            }
        });
    }
    let get = |path: &str| {
        let mut s = TcpStream::connect(addr).unwrap();
        s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes())
            .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    };

    // The modeset that caches /meta happens BEFORE the counters move.
    scene(&m, "matrix", 2, 2, 8, 16);
    let body = get("/stat");
    assert!(body.starts_with("HTTP/1.1 200 "), "{body}");
    assert!(body.contains("application/json"), "{body}");
    assert!(
        body.ends_with(r#"{"overruns":0,"viewers":0,"fps":15}"#),
        "{body}"
    );

    // Three late frames and a viewer, then ANOTHER modeset between the two
    // reads. A /stat served out of the cached /meta would report the state
    // as of this line forever; a /stat that reset on modeset would report
    // zero. Both are real implementations somebody would write.
    m.overran();
    m.overran();
    m.overran();
    m.viewers.fetch_add(1, Ordering::Relaxed);
    scene(&m, "city", 4, 4, 8, 16);
    let body = get("/stat");
    assert!(
        body.ends_with(r#"{"overruns":3,"viewers":1,"fps":15}"#),
        "{body}"
    );

    // And the fps really is the configured one, not a constant that happens
    // to match: a second mirror with a different budget says so.
    let m30 = Mirror::new(30);
    assert!(
        stat_json(&m30).contains("\"fps\":30"),
        "{}",
        stat_json(&m30)
    );
}

/// The display never waits on a viewer: a held lock drops the frame.
#[test]
fn a_busy_lock_drops_the_frame_instead_of_blocking() {
    let m = Mirror::new(15);
    m.viewers.fetch_add(1, Ordering::Relaxed);
    let held = m.frame.lock().unwrap();
    let gen = held.gen;
    std::thread::scope(|sc| {
        sc.spawn(|| m.publish(&[Cell::new(1, 2)]));
    });
    assert_eq!(held.gen, gen);
}
fn serve_test(m: &Arc<Mirror>) -> std::net::SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let m = Arc::clone(m);
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let m = Arc::clone(&m);
            std::thread::spawn(move || handle(&m, s));
        }
    });
    addr
}

fn req(addr: std::net::SocketAddr, line: &str) -> String {
    let mut s = TcpStream::connect(addr).unwrap();
    s.write_all(format!("{line} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes())
        .unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    out
}

/// `/config` over the socket: the knob list carries the constructor's own
/// range, a write in range takes and rebuilds the saver showing, anything
/// else is a 400 that changes nothing, and DELETE puts the default back.
/// `TOASTER3_DENSITY` because no other test reads it — the override map is
/// process-wide.
#[test]
fn config_lists_validates_sets_and_resets_a_knob() {
    let m = Mirror::new(15);
    let addr = serve_test(&m);
    assert!(m.select("toasters3"));
    let word = m.selection();

    let body = req(addr, "GET /config?saver=toasters3");
    assert!(body.starts_with("HTTP/1.1 200 "), "{body}");
    assert!(
        body.contains(
            r#"{"key":"TOASTER3_DENSITY","label":"density","kind":"num","default":2,"lo":1,"hi":60,"value":2,"overridden":false"#
        ),
        "{body}"
    );
    // A port's old `-wide` name reads the port's knobs.
    let body = req(addr, "GET /config?saver=tv-static-wide");
    assert!(body.contains(r#""key":"TV_STATIC_COLOR""#), "{body}");

    let body = req(
        addr,
        "POST /config?saver=toasters3&key=TOASTER3_DENSITY&value=9",
    );
    assert!(body.starts_with("HTTP/1.1 200 "), "{body}");
    assert!(body.contains(r#""value":9,"overridden":true"#), "{body}");
    assert!(body.contains(r#"{"rebuilt":true,"knobs":["#), "{body}");
    assert_eq!(crate::env_num(&["TOASTER3_DENSITY"], 2, 1, 60), 9);
    assert_ne!(m.selection(), word, "the saver showing was not rebuilt");
    assert_eq!(m.selected(), saver::index_of("toasters3").unwrap());

    let word = m.selection();
    for bad in [
        "POST /config?saver=toasters3&key=TOASTER3_DENSITY&value=61",
        "POST /config?saver=toasters3&key=TOASTER3_DENSITY&value=0",
        "POST /config?saver=toasters3&key=TOASTER3_DENSITY&value=5x",
        "POST /config?saver=toasters3&key=TOASTER_DENSITY&value=5",
        "POST /config?saver=toasters3&key=SAVER_FPS&value=5",
        "POST /config?saver=nope&key=TOASTER3_DENSITY&value=5",
        "GET /config?saver=nope",
        "GET /config",
    ] {
        let body = req(addr, bad);
        assert!(body.starts_with("HTTP/1.1 400 "), "{bad}: {body}");
        assert!(body.contains("\"error\":"), "{bad}: {body}");
        assert_eq!(crate::env_num(&["TOASTER3_DENSITY"], 2, 1, 60), 9, "{bad}");
    }
    assert_eq!(m.selection(), word, "a refused write rebuilt the saver");

    // A write to a saver that is not showing does not touch the panel.
    assert!(m.select("dvd"));
    let word = m.selection();
    let body = req(
        addr,
        "POST /config?saver=toasters3&key=TOASTER3_SPEED&value=300",
    );
    assert!(body.starts_with("HTTP/1.1 200 "), "{body}");
    assert!(body.contains(r#"{"rebuilt":false,"#), "{body}");
    assert_eq!(m.selection(), word);

    for reset in [
        "DELETE /config?saver=toasters3&key=TOASTER3_DENSITY",
        "POST /config?saver=toasters3&key=TOASTER3_SPEED&value=",
    ] {
        let body = req(addr, reset);
        assert!(body.starts_with("HTTP/1.1 200 "), "{reset}: {body}");
    }
    assert_eq!(crate::env_num(&["TOASTER3_DENSITY"], 2, 1, 60), 2);
    assert!(!config::is_overridden("TOASTER3_SPEED"));

    let body = req(addr, "PUT /config?saver=toasters3");
    assert!(body.starts_with("HTTP/1.1 405 "), "{body}");
}

/// The tour's switch decides which of its other knobs a scene reads, so
/// the list after turning it off has no timings — and a scene not showing
/// is rebuilt when it shares the key with the one that is.
#[test]
fn config_follows_a_switch_that_hides_other_knobs() {
    let _knobs = config::SHARED_KNOBS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let m = Mirror::new(15);
    let addr = serve_test(&m);
    assert!(m.select("ocean-sunset"));
    let word = m.selection();
    let body = req(addr, "GET /config?saver=storm-plains");
    assert!(!body.contains("ASCII_REST_TOUR_SHOT_SECS"), "{body}");
    assert!(
        body.contains(r#""key":"ASCII_REST_TITLE","label":"title","kind":"bool""#),
        "{body}"
    );

    let body = req(
        addr,
        "POST /config?saver=storm-plains&key=ASCII_REST_TOUR&value=1",
    );
    assert!(body.starts_with("HTTP/1.1 200 "), "{body}");
    assert!(body.contains("ASCII_REST_TOUR_SHOT_SECS"), "{body}");
    assert_ne!(m.selection(), word, "ocean-sunset reads the tour too");

    req(
        addr,
        "DELETE /config?saver=storm-plains&key=ASCII_REST_TOUR",
    );
    let body = req(addr, "GET /config?saver=storm-plains");
    assert!(!body.contains("ASCII_REST_TOUR_SHOT_SECS"), "{body}");
}

/// Tour knobs set from the page reach the panel: each write rebuilds the
/// scene showing, and the rebuilt scene tours by the new values. With
/// short shots the camera pans by the pixel and zooms to at most 250% of
/// the untoured cell by default, past it at 600%; then the switch off, and
/// nothing moves.
#[test]
fn tour_knobs_from_the_page_rebuild_the_scene_with_them() {
    let _knobs = config::SHARED_KNOBS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let m = Mirror::new(15);
    let addr = serve_test(&m);
    assert!(m.select("night-coast"));
    let panel = Panel::new(960, 540, 960);
    let place = |n: &str| (panel, saver::make(n, &panel, 30));
    let mut d = saver::Driver::new(&m, 30, place);
    let mut buf = vec![0u32; panel.buf_len()];
    // The widest cell and how many pixel shifts over `frames`.
    let mut run = |d: &mut saver::Driver, frames: usize| {
        let (mut widest, mut shifts) = (0, 0);
        let mut last = d.saver().grid().shift_of();
        for _ in 0..frames {
            d.frame(&mut buf, &m);
            let g = d.saver().grid();
            widest = widest.max(g.cell_w());
            shifts += usize::from(g.shift_of() != last);
            last = g.shift_of();
        }
        (widest, shifts)
    };
    let set = |kv: &str| {
        let body = req(addr, &format!("POST /config?saver=night-coast&{kv}"));
        assert!(body.contains(r#""rebuilt":true"#), "{body}");
    };
    let t0 = std::time::Instant::now();
    let cover = d.saver().grid().cell_w();

    set("key=ASCII_REST_TOUR&value=1");
    assert!(d.switch(t0, &m, place), "the write did not rebuild");
    set("key=ASCII_REST_TOUR_SHOT_SECS&value=4");
    assert!(d.switch(t0, &m, place), "the write did not rebuild");
    let (widest, shifts) = run(&mut d, 30 * 90);
    assert!(widest * 2 <= cover * 5, "{widest} past 250% of {cover}");
    assert!(shifts > 30 * 10, "only {shifts} pixel shifts");

    set("key=ASCII_REST_TOUR_MAX_ZOOM_PCT&value=600");
    assert!(d.switch(t0, &m, place), "the write did not rebuild");
    let (widest, _) = run(&mut d, 30 * 90);
    assert!(
        widest * 2 > cover * 5,
        "{widest} never past 250% of {cover}"
    );

    set("key=ASCII_REST_TOUR&value=0");
    assert!(d.switch(t0, &m, place), "the write did not rebuild");
    let w = d.saver().grid().cell_w();
    assert_eq!(run(&mut d, 30 * 10), (w, 0), "the tour is off");

    for k in [
        "ASCII_REST_TOUR",
        "ASCII_REST_TOUR_SHOT_SECS",
        "ASCII_REST_TOUR_MAX_ZOOM_PCT",
    ] {
        req(addr, &format!("DELETE /config?saver=night-coast&key={k}"));
    }
}

#[test]
fn meta_lists_groups_and_the_rotation() {
    let m = Mirror::new(15);
    scene(&m, "matrix", 2, 2, 8, 16);
    let addr = serve_test(&m);
    let body = req(addr, "GET /meta");
    let head = body.split(",\"palette\"").next().unwrap();
    assert!(
        head.contains(r#"{"name":"scenes","savers":["alpine-dawn","#),
        "{head}"
    );
    assert!(
        head.contains(r#""savers":["alpine-dawn","aurora-fjord""#),
        "{head}"
    );
    assert!(!head.contains("-wide"), "{head}");
    assert!(
        head.contains(r#"{"name":"classics","savers":["ascii","blocks","matrix""#),
        "{head}"
    );
    let tail = |b: &str| b.rsplit_once("]],").unwrap().1.to_string();
    assert!(tail(&body).contains(r#""excluded":[]"#), "{}", tail(&body));

    // A group takes every member.
    let body = req(addr, "POST /rotation?saver=night-coast&on=0");
    assert!(body.starts_with("HTTP/1.1 200 "), "{body}");
    assert!(body.ends_with(r#"{"excluded":["night-coast"]}"#), "{body}");
    let body = req(addr, "POST /rotation?group=flights&on=0");
    assert!(body.contains(r#""warp","hypercube","pov""#), "{body}");
    let body = req(addr, "POST /rotation?group=flights&on=1");
    assert!(body.ends_with(r#"{"excluded":["night-coast"]}"#), "{body}");
    assert!(tail(&req(addr, "GET /meta")).contains(r#""excluded":["night-coast"]"#));
    for bad in [
        "POST /rotation?saver=nope&on=0",
        "POST /rotation?saver=dvd&on=2",
        "POST /rotation?saver=dvd",
        "POST /rotation?group=nope&on=0",
        "POST /rotation?saver=dvd&group=flights&on=0",
        "POST /rotation",
    ] {
        let body = req(addr, bad);
        assert!(body.starts_with("HTTP/1.1 400 "), "{bad}: {body}");
    }
    let body = req(addr, "GET /rotation?saver=dvd&on=0");
    assert!(body.starts_with("HTTP/1.1 405 "), "{body}");
    // A port's old `-wide` name is the port.
    req(addr, "POST /rotation?saver=night-coast-wide&on=1");
    assert!(tail(&req(addr, "GET /meta")).contains(r#""excluded":[]"#));
    let body = req(addr, "POST /rotation?saver=vinyl-wide&on=0");
    assert!(body.ends_with(r#"{"excluded":["vinyl"]}"#), "{body}");
    req(addr, "POST /rotation?saver=vinyl&on=1");

    assert_eq!(m.exclude("dvd, nope,,matrix, aurora-wide"), ["nope"]);
    assert!(!m.in_rotation(saver::index_of("aurora").unwrap()));
    assert!(!m.in_rotation(saver::index_of("dvd").unwrap()));
    assert!(!m.in_rotation(saver::index_of("matrix").unwrap()));
}

/// `/select` answers once the render loop has built the saver, with its
/// `/meta` — one round trip, and the epoch in it is the one `/stream`
/// will accept. Here a thread stands in for the render loop.
#[test]
fn select_answers_with_the_new_savers_meta() {
    let m = Mirror::new(15);
    scene(&m, "matrix", 2, 2, 8, 16);
    let addr = serve_test(&m);
    {
        let m = Arc::clone(&m);
        // Read before spawning: a /select that beats the thread to its
        // first read would otherwise look like the starting state, and
        // nothing would ever apply it.
        let mut seen = m.selection();
        std::thread::spawn(move || loop {
            let want = m.selection();
            if want != seen {
                seen = want;
                std::thread::sleep(Duration::from_millis(30));
                scene(&m, saver::name_at(sel_index(want)), 3, 3, 8, 16);
                m.applied(want);
            }
            std::thread::sleep(Duration::from_millis(2));
        });
    }
    let body = req(addr, "POST /select?saver=dvd");
    assert!(body.starts_with("HTTP/1.1 200 "), "{body}");
    assert!(body.contains(r#"{"saver":"dvd","#), "{body}");
    assert!(body.contains(r#""epoch":2,"#), "{body}");
    assert!(
        body.ends_with(r#""rotate_secs":0,"excluded":[]}"#),
        "{body}"
    );
}

/// No render loop (a node with no monitor): `/select` still answers, after
/// the bounded wait, with only the name.
#[test]
fn select_without_a_render_loop_answers_after_the_wait() {
    let m = Mirror::new(15);
    scene(&m, "matrix", 2, 2, 8, 16);
    let addr = serve_test(&m);
    let t0 = std::time::Instant::now();
    let body = req(addr, "POST /select?saver=dvd");
    assert!(body.ends_with(r#"{"saver":"dvd"}"#), "{body}");
    assert!(t0.elapsed() >= APPLY_WAIT && t0.elapsed() < APPLY_WAIT * 2);
}

#[test]
fn params_are_percent_decoded_and_strings_escaped() {
    assert_eq!(param("v=a%20b+c%2C", "v").as_deref(), Some("a b c,"));
    assert_eq!(param("v=%zz", "v"), None);
    assert_eq!(param("v=%2", "v"), None);
    assert_eq!(json_str("a\"b\\c\n"), r#""a\"b\\c\u000a""#);
}
