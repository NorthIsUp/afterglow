//! The web mirror: what the panel is drawing, as the character grid it is
//! drawn from.
//!
//! # Why the grid and not pixels
//!
//! Every saver in this program paints through [`Grid`], so the panel's entire
//! state is `cols * rows` [`Cell`]s — a glyph index and a palette index packed
//! in one u32 — plus a palette and the glyph table, both fixed for the run.
//! Mirroring the cells rather than the pixels is not a compression trick; it is
//! sending the thing the renderer actually has.
//!
//! Measured at 1920x1080, cell defaults, `SAVER_FPS=15` (what ships):
//!
//! | saver  | grid    | cells  | cells changed/frame | damaged scanlines/frame |
//! | ------ | ------- | ------ | ------------------- | ----------------------- |
//! | matrix | 120x33  |   3960 |   792 (20.0%)       | 1056 (every grid row)   |
//! | ascii  | 120x67  |   8040 |  2546 (31.7%)       | 1056                    |
//! | blocks | 480x270 | 129600 | 11022  (8.5%)       | ~620                    |
//!
//! Matrix is the only one whose rate depends on fps — its fall is per-frame —
//! so at the 30fps default it halves to 414.
//!
//! The right-hand column is why a pixel-shaped answer loses: matrix dirties
//! every scanline every frame, so "ship the damaged rows" ships 1920*1056*4 =
//! 8.1 MB per frame. The same frame is 792 changed cells = 6.4 KB. Re-encoding
//! it as JPEG/PNG instead costs the Pi tens of milliseconds a frame, on a pod
//! whose whole render budget is 113m of one core.
//!
//! # What this costs the renderer
//!
//! With no viewer: one relaxed atomic load per frame, and nothing else.
//!
//! With a viewer: one `memcpy` of the cell array (15.8 KB for matrix) per
//! frame, on the render thread, under a `try_lock` that is SKIPPED rather than
//! waited on. Every other cost — the diff, the encode, the socket — is on the
//! viewer's own thread. The display cannot be slowed by a slow client, and a
//! frame the mirror misses is a frame the mirror misses.
//!
//! # Wire format
//!
//! `GET /stream` is an HTTP/1.1 chunked binary stream, not a WebSocket: the
//! traffic is one-way server -> client, which `fetch` + a stream reader already
//! does, and a WebSocket here would buy nothing for a hand-rolled SHA-1 and a
//! frame codec. Each record is `u32 count` then `count * (u32 index, u32 cell)`,
//! little-endian. The first record after connect is every cell (the viewer's
//! `prev` starts impossible), later ones only what changed; `count == 0` is the
//! idle keepalive that notices a dead socket.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::font;
use crate::grid::Cell;
use crate::saver;

/// Each viewer holds its own `prev` grid and does its own diff. Two people
/// looking at a screensaver is already generous; the cap is what stops a
/// forgotten browser tab farm from costing the render node real CPU.
const MAX_VIEWERS: usize = 4;

/// No viewer frame for this long and the stream emits an empty record, so a
/// browser that went away is noticed instead of parking a thread forever.
const KEEPALIVE: Duration = Duration::from_secs(10);

const PAGE: &str = include_str!("mirror.html");

/// Cell state that cannot occur: 66 glyphs ship, so `u16::MAX` forces the first
/// diff against it to emit every cell — that is how a connect gets a keyframe
/// without a second code path.
const NEVER: Cell = Cell::new(u16::MAX, u16::MAX);

#[derive(Default)]
struct Frame {
    /// Bumped per published frame; a viewer waits on a change.
    gen: u64,
    /// Bumped per modeset. Geometry, palette and glyph meaning all belong to an
    /// epoch, so a viewer from the previous one is closed rather than fed cells
    /// it would mis-colour.
    epoch: u64,
    cells: Vec<Cell>,
}

pub struct Mirror {
    viewers: AtomicUsize,
    /// Index into `saver::NAMES` the render loop should be drawing. An atomic
    /// rather than a lock because the render loop reads it every frame and must
    /// never wait on a browser; `POST /select` is the only writer.
    selected: AtomicUsize,
    frame: Mutex<Frame>,
    ready: Condvar,
    /// `/meta` JSON, rebuilt on modeset. Empty until the first one.
    meta: Mutex<String>,
}

impl Mirror {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            viewers: AtomicUsize::new(0),
            selected: AtomicUsize::new(0),
            frame: Mutex::new(Frame::default()),
            ready: Condvar::new(),
            meta: Mutex::new(String::new()),
        })
    }

    /// Index into `saver::NAMES` the render loop should be drawing.
    pub fn selected(&self) -> usize {
        self.selected.load(Ordering::Relaxed)
    }

    /// Point the render loop at `name`. False for a name no saver answers to,
    /// leaving the selection untouched — the render loop must never be handed
    /// an index that is not a saver.
    pub fn select(&self, name: &str) -> bool {
        let Some(i) = saver::index_of(name) else {
            return false;
        };
        self.selected.store(i, Ordering::Relaxed);
        true
    }

    /// Publish this frame's cells. Called from the render thread, once per
    /// frame, and deliberately gives up rather than waits: see the module doc.
    pub fn publish(&self, cells: &[Cell]) {
        if self.viewers.load(Ordering::Relaxed) == 0 {
            return;
        }
        let Ok(mut f) = self.frame.try_lock() else {
            return;
        };
        f.cells.clear();
        f.cells.extend_from_slice(cells);
        f.gen += 1;
        drop(f);
        self.ready.notify_all();
    }

    /// Record what the viewers need to draw a frame: geometry, the palette and
    /// the glyph table. Called once per successful modeset.
    pub fn describe(
        &self,
        saver: &str,
        cols: usize,
        rows: usize,
        cw: usize,
        ch: usize,
        pal: &[u32],
    ) {
        let mut json = String::with_capacity(8 << 10);
        json.push_str(&format!(
            "{{\"saver\":\"{saver}\",\"savers\":[{savers}],\
             \"cols\":{cols},\"rows\":{rows},\
             \"cell_w\":{cw},\"cell_h\":{ch},\
             \"glyph_w\":{},\"glyph_h\":{},\"palette\":[",
            font::GLYPH_W,
            font::GLYPH_H,
            savers = crate::saver::names()
                .map(|n| format!("\"{n}\""))
                .collect::<Vec<_>>()
                .join(",")
        ));
        for (i, c) in pal.iter().enumerate() {
            json.push_str(&format!(
                "{}{}",
                if i > 0 { "," } else { "" },
                c & 0xFF_FFFF
            ));
        }
        json.push_str("],\"glyphs\":[");
        for (i, g) in font::GLYPHS.iter().enumerate() {
            json.push_str(if i > 0 { ",[" } else { "[" });
            for (j, b) in g.iter().enumerate() {
                json.push_str(&format!("{}{b}", if j > 0 { "," } else { "" }));
            }
            json.push(']');
        }
        json.push_str("]}");
        *self.meta.lock().unwrap() = json;

        let mut f = self.frame.lock().unwrap();
        f.epoch += 1;
        f.gen += 1;
        f.cells.clear();
        drop(f);
        self.ready.notify_all();
    }
}

/// Bind and accept forever. Returns only if the bind fails — a mirror that
/// cannot listen must not take the screensaver down with it.
pub fn serve(mirror: Arc<Mirror>, addr: &str) {
    let listener = match TcpListener::bind(addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[screensaver] mirror: bind {addr}: {e}; web mirror disabled");
            return;
        }
    };
    eprintln!("[screensaver] mirror listening on {addr}");
    for stream in listener.incoming().flatten() {
        let m = Arc::clone(&mirror);
        // Thread per connection: a request is a route match and then either a
        // one-shot body or a long-lived stream, and a thread models the second
        // one for free. Concurrency is bounded by MAX_VIEWERS at the only route
        // that holds a thread open.
        std::thread::spawn(move || {
            let _ = handle(&m, stream);
        });
    }
}

fn handle(mirror: &Mirror, mut s: TcpStream) -> std::io::Result<()> {
    s.set_read_timeout(Some(Duration::from_secs(10)))?;
    // Writes must not park a thread forever on a client that stopped reading.
    s.set_write_timeout(Some(Duration::from_secs(10)))?;
    let _ = s.set_nodelay(true);

    let Some((method, path, query)) = read_request(&mut s)? else {
        return send(&mut s, "400 Bad Request", "text/plain", b"bad request");
    };

    match (method.as_str(), path.as_str()) {
        // Restored after /select added a method column: the old parser rejected
        // every non-GET at the parse layer, and dropping that made POST /stream
        // able to take one of the four viewer slots and hold a thread.
        (m, p) if p == "/select" && m != "POST" => send(
            &mut s,
            "405 Method Not Allowed",
            "text/plain",
            b"method not allowed: POST /select\n",
        ),
        (m, p) if m != "GET" && !(m == "POST" && p == "/select") => send(
            &mut s,
            "405 Method Not Allowed",
            "text/plain",
            b"method not allowed",
        ),
        // A GET must not be able to change the saver.
        ("POST", "/select") => match param(&query, "saver") {
            Some(name) if mirror.select(&name) => send(
                &mut s,
                "200 OK",
                "application/json",
                selected_json(mirror).as_bytes(),
            ),
            // Unknown name: say so and change nothing. The render loop only
            // ever sees an index that is a saver.
            other => {
                eprintln!("[screensaver] rejected saver {other:?}");
                send(
                    &mut s,
                    "400 Bad Request",
                    "application/json",
                    br#"{"error":"unknown saver"}"#,
                )
            }
        },
        (_, "/") => send(
            &mut s,
            "200 OK",
            "text/html; charset=utf-8",
            PAGE.as_bytes(),
        ),
        (_, "/meta") => {
            let meta = mirror.meta.lock().unwrap().clone();
            if meta.is_empty() {
                // No modeset yet: the pod is up but idling on a node with no
                // monitor. A 503 is the honest answer and the page retries.
                send(&mut s, "503 Service Unavailable", "application/json", b"{}")
            } else {
                send(&mut s, "200 OK", "application/json", meta.as_bytes())
            }
        }
        (_, "/stream") => stream(mirror, s),
        _ => send(&mut s, "404 Not Found", "text/plain", b"not found"),
    }
}

/// Method, path and query string, or None if there is no request line to parse.
/// Reads at most 8 KiB: no route here takes a body — `/select` carries its
/// argument in the query string, so nothing waits on a second packet — and a
/// larger request is a mistake or an attack, either way answered the same.
fn read_request(s: &mut TcpStream) -> std::io::Result<Option<(String, String, String)>> {
    let mut buf = [0u8; 8 << 10];
    let mut n = 0;
    while n < buf.len() {
        let got = s.read(&mut buf[n..])?;
        if got == 0 {
            break;
        }
        n += got;
        if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let head = String::from_utf8_lossy(&buf[..n]);
    let Some(line) = head.lines().next() else {
        return Ok(None);
    };
    let mut parts = line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return Ok(None);
    };
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    Ok(Some((method.into(), path.into(), query.into())))
}

fn param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|kv| {
        kv.split_once('=')
            .filter(|(k, _)| *k == key)
            .map(|(_, v)| v.to_string())
    })
}

fn selected_json(mirror: &Mirror) -> String {
    format!(
        "{{\"saver\":\"{}\"}}",
        crate::saver::name_at(mirror.selected())
    )
}

fn send(s: &mut TcpStream, status: &str, ctype: &str, body: &[u8]) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    s.write_all(head.as_bytes())?;
    s.write_all(body)?;
    s.flush()
}

/// One viewer: wait for a frame, diff it against what this viewer last saw,
/// write the changed cells. The diff and the write are both on THIS thread —
/// the render thread's only involvement is the memcpy in `publish`.
fn stream(mirror: &Mirror, mut s: TcpStream) -> std::io::Result<()> {
    if mirror.viewers.fetch_add(1, Ordering::Relaxed) >= MAX_VIEWERS {
        mirror.viewers.fetch_sub(1, Ordering::Relaxed);
        return send(
            &mut s,
            "503 Service Unavailable",
            "text/plain",
            b"too many viewers",
        );
    }
    let _guard = ViewerGuard(mirror);

    // X-Accel-Buffering: the tailscale-auth gate is nginx, and nginx buffers a
    // proxied response by default — which for a trickle of small chunks means
    // the mirror runs frames behind, or stalls until a buffer fills. This
    // header turns that off for THIS response, so the shared component does not
    // need a streaming exception for one app.
    s.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n\
          Transfer-Encoding: chunked\r\nCache-Control: no-store\r\n\
          X-Accel-Buffering: no\r\n\r\n",
    )?;
    s.flush()?;

    let mut prev: Vec<Cell> = Vec::new();
    let mut cur: Vec<Cell> = Vec::new();
    let mut out: Vec<u8> = Vec::new();
    let mut last_gen = 0u64;
    let epoch = mirror.frame.lock().unwrap().epoch;

    loop {
        {
            let mut f = mirror.frame.lock().unwrap();
            while f.gen == last_gen {
                let (guard, timeout) = mirror.ready.wait_timeout(f, KEEPALIVE).unwrap();
                f = guard;
                if timeout.timed_out() {
                    break;
                }
            }
            // A modeset invalidates geometry, palette and glyph meaning; the
            // page reconnects and re-reads /meta rather than being patched.
            if f.epoch != epoch {
                return Ok(());
            }
            last_gen = f.gen;
            cur.clear();
            cur.extend_from_slice(&f.cells);
        }

        // `describe` bumps the generation with no cells behind it, so a viewer
        // that connects during a modeset would otherwise open with an empty
        // record. Wait for a real frame instead.
        if cur.is_empty() {
            continue;
        }

        if prev.len() != cur.len() {
            prev.clear();
            prev.resize(cur.len(), NEVER);
        }
        out.clear();
        out.extend_from_slice(&0u32.to_le_bytes());
        let mut n = 0u32;
        for (i, (a, b)) in prev.iter().zip(cur.iter()).enumerate() {
            if a != b {
                out.extend_from_slice(&(i as u32).to_le_bytes());
                out.extend_from_slice(&b.raw().to_le_bytes());
                n += 1;
            }
        }
        out[..4].copy_from_slice(&n.to_le_bytes());
        prev.copy_from_slice(&cur);
        write_chunk(&mut s, &out)?;
    }
}

struct ViewerGuard<'a>(&'a Mirror);

impl Drop for ViewerGuard<'_> {
    fn drop(&mut self) {
        self.0.viewers.fetch_sub(1, Ordering::Relaxed);
    }
}

fn write_chunk(s: &mut TcpStream, body: &[u8]) -> std::io::Result<()> {
    write!(s, "{:x}\r\n", body.len())?;
    s.write_all(body)?;
    s.write_all(b"\r\n")?;
    s.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let m = Mirror::new();
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
        let m = Mirror::new();
        m.describe("test", 2, 2, 8, 16, &[0, 0xFF]);
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
        let m = Mirror::new();
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

    /// The display never waits on a viewer: a held lock drops the frame.
    #[test]
    fn a_busy_lock_drops_the_frame_instead_of_blocking() {
        let m = Mirror::new();
        m.viewers.fetch_add(1, Ordering::Relaxed);
        let held = m.frame.lock().unwrap();
        let gen = held.gen;
        std::thread::scope(|sc| {
            sc.spawn(|| m.publish(&[Cell::new(1, 2)]));
        });
        assert_eq!(held.gen, gen);
    }
}
