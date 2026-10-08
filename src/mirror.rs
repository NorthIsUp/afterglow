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
//! Measured on the Pi at 1920x1080, cell defaults, `SAVER_FPS=15` (what
//! ships), 2026-09-13:
//!
//! | saver  | grid    | cells  | cells changed/frame | damaged scanlines/frame |
//! | ------ | ------- | ------ | ------------------- | ----------------------- |
//! | matrix | 120x33  |   3960 |   810 (20.5%)       | 1056 (every grid row)   |
//! | ascii  | 120x67  |   8040 |  3033 (37.7%)       | 1056                    |
//! | blocks | 480x270 | 129600 | 37064 (28.6%)       | ~620                    |
//!
//! Matrix is the only one whose rate depends on fps — its fall is per-frame —
//! so at the 30fps default it roughly halves.
//!
//! The right-hand column is why a pixel-shaped answer loses: matrix dirties
//! every scanline every frame, so "ship the damaged rows" ships 1920*1056*4 =
//! 8.1 MB per frame. The same frame is 810 changed cells = 6.5 KB. Re-encoding
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

use std::fmt::Write as _;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::config;
use crate::font;
use crate::grid::{Cell, Grid};
use crate::saver;
use crate::surface::Panel;

/// Each viewer holds its own `prev` grid and does its own diff. Two people
/// looking at a screensaver is already generous; the cap is what stops a
/// forgotten browser tab farm from costing the render node real CPU.
const MAX_VIEWERS: usize = 4;

/// No viewer frame for this long and the stream emits an empty record, so a
/// browser that went away is noticed instead of parking a thread forever.
const KEEPALIVE: Duration = Duration::from_secs(10);

const PAGE: &str = include_str!("mirror.html");

/// The rotation interval lives in the low 32 bits of the control word.
const ROTATE_SECS: u64 = u32::MAX as u64;

/// Longest interval `POST /rotate` accepts, in minutes — the same day
/// `SAVER_ROTATE_SECS` tops out at.
const MAX_ROTATE_MINS: u64 = 86_400 / 60;

/// The saver index in the low half of a selection word; the high half is a
/// counter bumped on every selection, so re-selecting the saver already
/// showing still reads as a change. See `Mirror::selection`.
pub fn sel_index(word: u64) -> usize {
    (word & u64::from(u32::MAX)) as usize
}

/// Longest `/select` waits for the render loop to build what it asked for.
/// Past it the pod has no render loop running (no monitor) or is wedged, and
/// the page falls back to reconnecting.
const APPLY_WAIT: Duration = Duration::from_secs(2);

/// Seconds out of a rotation control word. The packing is this module's, so the
/// render loop asks rather than masking a layout it would have to be kept in
/// step with.
pub fn ctl_secs(ctl: u64) -> u64 {
    ctl & ROTATE_SECS
}

/// Cell state that cannot occur: `font::GLYPHS` is nowhere near `u16::MAX`
/// entries, so a `prev` filled with this forces the first
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
    /// Frames that took longer than the frame budget. A count, not a log: at
    /// 30fps a line per late frame is its own outage. Read through `/stat`,
    /// which is why that route exists instead of a field in the cached
    /// `/meta`.
    overruns: AtomicUsize,
    /// The configured frame budget, reported by `/stat` so an overrun count
    /// reads as a rate rather than a bare number.
    fps: u32,
    /// Index into `saver::NAMES` the render loop should be drawing, in the
    /// low half, and a change counter in the high half — see `sel_index`. An
    /// atomic rather than a lock because the render loop reads it every frame
    /// and must never wait on a browser.
    selected: AtomicU64,
    /// Seconds between automatic switches in the low 32 bits, a change counter
    /// in the high 32. One word rather than two so the render loop spots a
    /// change with a single relaxed load — see `set_rotate_secs`.
    rotate: AtomicU64,
    /// Bit per `saver::SAVERS` row, set when rotation may pick it. Read only
    /// when a turn is up, never per frame.
    rotation: [AtomicU64; saver::POOL_WORDS],
    /// Bit per `-wide` row, set while its scene is expanded: the variant a
    /// click last chose, and so the one rotation shows. One pair, one turn.
    expanded: [AtomicU64; saver::POOL_WORDS],
    /// Per row, its pair's other half and whether this row is the `-wide`
    /// one. Built once here, so the rotation boundary on the render thread
    /// looks it up rather than formatting a name.
    twins: Box<[Option<(usize, bool)>]>,
    /// The selection word the render loop last built and announced, so a
    /// `/select` can answer with the saver it asked for rather than the one
    /// it replaced.
    applied: Mutex<u64>,
    built: Condvar,
    frame: Mutex<Frame>,
    ready: Condvar,
    /// `/meta` JSON, rebuilt on modeset, MINUS its closing brace: the rotation
    /// interval is live-settable and so cannot be baked into a string cached at
    /// modeset time. The `/meta` route closes the object with it. Empty until
    /// the first modeset.
    meta: Mutex<String>,
}

impl Mirror {
    pub fn new(fps: u32) -> Arc<Self> {
        Arc::new(Self {
            viewers: AtomicUsize::new(0),
            overruns: AtomicUsize::new(0),
            fps,
            selected: AtomicU64::new(0),
            rotate: AtomicU64::new(0),
            rotation: std::array::from_fn(|_| AtomicU64::new(u64::MAX)),
            expanded: std::array::from_fn(|w| {
                AtomicU64::new(
                    (w * 64..(w * 64 + 64).min(saver::NSAVERS))
                        .filter(|&i| {
                            saver::name_at(i).ends_with("-wide")
                                && saver::twin_of(saver::name_at(i)).is_some()
                        })
                        .fold(0, |bits, i| bits | 1 << (i % 64)),
                )
            }),
            twins: saver::names()
                .map(|n| {
                    saver::twin_of(n)
                        .and_then(saver::index_of)
                        .map(|t| (t, n.ends_with("-wide")))
                })
                .collect(),
            applied: Mutex::new(0),
            built: Condvar::new(),
            frame: Mutex::new(Frame::default()),
            ready: Condvar::new(),
            meta: Mutex::new(String::new()),
        })
    }

    /// The render loop missed its frame budget. One relaxed increment, on a
    /// branch only reached when the frame is already over budget.
    pub fn overran(&self) {
        self.overruns.fetch_add(1, Ordering::Relaxed);
    }

    /// Frames that missed the budget since start.
    pub fn overruns(&self) -> usize {
        self.overruns.load(Ordering::Relaxed)
    }

    /// Index into `saver::NAMES` the render loop should be drawing.
    pub fn selected(&self) -> usize {
        sel_index(self.selection())
    }

    /// The selection word, counter included, for the render loop: a change in
    /// it is a rebuild even when the index is the one already showing.
    pub fn selection(&self) -> u64 {
        self.selected.load(Ordering::Relaxed)
    }

    /// Point the render loop at `name`. False for a name no saver answers to,
    /// leaving the selection untouched — the render loop must never be handed
    /// an index that is not a saver.
    pub fn select(&self, name: &str) -> bool {
        let Some(i) = saver::index_of(name) else {
            return false;
        };
        // Picking one half of a scene's pair is the `expanded` choice, and
        // rotation follows it. Here and not in `select_at`: rotation moving
        // on must not change what the viewer chose.
        if let Some((t, wide)) = self.twins[i] {
            let w = if wide { i } else { t };
            let bit = 1u64 << (w % 64);
            if wide {
                self.expanded[w / 64].fetch_or(bit, Ordering::Relaxed);
            } else {
                self.expanded[w / 64].fetch_and(!bit, Ordering::Relaxed);
            }
        }
        self.select_at(i);
        true
    }

    /// Point the render loop at a row of `saver::SAVERS`. The by-index twin of
    /// `select`, for the rotation timer — it picks a row rather than a name, so
    /// there is nothing to validate. Every other caller must come through
    /// `select`, which is where a user-supplied name is checked.
    pub fn select_at(&self, i: usize) {
        let _ = self
            .selected
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                Some(((v >> 32).wrapping_add(1) << 32) | i as u64)
            });
    }

    /// Rebuild row `i` if it is still the one selected, so it re-reads its
    /// knobs. Conditional, because a `/select` racing a `/config` must not be
    /// undone by it.
    pub fn reselect(&self, i: usize) -> bool {
        self.selected
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                (sel_index(v) == i).then(|| ((v >> 32).wrapping_add(1) << 32) | i as u64)
            })
            .is_ok()
    }

    /// May rotation pick row `i`.
    pub fn in_rotation(&self, i: usize) -> bool {
        self.rotation[i / 64].load(Ordering::Relaxed) & (1 << (i % 64)) != 0
    }

    /// May rotation pick row `i` now: in rotation, and for a scene's pair,
    /// the half its `expanded` choice names — the `-wide` until a viewer
    /// picks. Allocation-free; called at the rotation boundary.
    /// Is `-wide` row `w` the half its pair shows.
    fn pickable_half(&self, w: usize) -> bool {
        self.expanded[w / 64].load(Ordering::Relaxed) & (1 << (w % 64)) != 0
    }

    pub fn pickable(&self, i: usize) -> bool {
        self.in_rotation(i)
            && match self.twins[i] {
                None => true,
                Some((_, true)) => self.pickable_half(i),
                Some((t, false)) => !self.pickable_half(t),
            }
    }

    /// Put `name` in or out of rotation, and its `-wide` twin with it: the
    /// page shows the pair as one row. False for a name that is not a saver.
    pub fn set_in_rotation(&self, name: &str, on: bool) -> bool {
        let Some(i) = saver::index_of(name) else {
            return false;
        };
        for i in std::iter::once(i).chain(saver::twin_of(name).and_then(saver::index_of)) {
            let bit = 1 << (i % 64);
            if on {
                self.rotation[i / 64].fetch_or(bit, Ordering::Relaxed);
            } else {
                self.rotation[i / 64].fetch_and(!bit, Ordering::Relaxed);
            }
        }
        true
    }

    /// `SAVER_ROTATE_EXCLUDE`: comma-separated names taken out of rotation at
    /// start. Returns the ones that are not savers, for the log.
    pub fn exclude(&self, list: &str) -> Vec<String> {
        list.split(',')
            .map(str::trim)
            .filter(|n| !n.is_empty() && !self.set_in_rotation(n, false))
            .map(String::from)
            .collect()
    }

    /// The render loop built and announced the saver for selection `word`.
    pub fn applied(&self, word: u64) {
        *self.applied.lock().unwrap() = word;
        self.built.notify_all();
    }

    /// Wait until the render loop has built selection `word`, or a later
    /// one, for at most `limit`. Later counts: a second click supersedes the
    /// first, and the first's answer is then the second's saver.
    fn wait_applied(&self, word: u64, limit: Duration) -> bool {
        let count = |w: u64| (w >> 32) as u32;
        let reached = |a: u64| count(a).wrapping_sub(count(word)).cast_signed() >= 0;
        let g = self.applied.lock().unwrap();
        let (g, _) = self
            .built
            .wait_timeout_while(g, limit, |a| !reached(*a))
            .unwrap();
        reached(*g)
    }

    /// The rotation control word, for the render loop: one relaxed load, same
    /// as `selected` and adjacent to it. Relaxed is right for the same reason —
    /// the word publishes no data, it is a number of seconds and a counter.
    pub fn rotate_ctl(&self) -> u64 {
        self.rotate.load(Ordering::Relaxed)
    }

    /// Seconds between automatic switches; 0 is off.
    pub fn rotate_secs(&self) -> u64 {
        ctl_secs(self.rotate_ctl())
    }

    /// Set the interval. Out-of-range seconds are the caller's problem — the
    /// route validates, exactly as `select` is the one place a saver name is
    /// checked.
    ///
    /// The counter in the high half is bumped on EVERY call, so the render loop
    /// restarts the turn even when the number did not change. Without it,
    /// asking for five minutes 4:59 into a five-minute turn would buy one
    /// second, which is the infuriating version of this control.
    pub fn set_rotate_secs(&self, secs: u64) {
        let _ = self
            .rotate
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                Some((v >> 32).wrapping_add(1) << 32 | (secs & ROTATE_SECS))
            });
    }

    /// Anyone watching. The render loop asks before building the frame it
    /// would publish, so a saver's mirror-only work costs nothing unwatched.
    pub fn watched(&self) -> bool {
        self.viewers.load(Ordering::Relaxed) != 0
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
    ///
    /// Both rectangles, because they are not the same one: `Grid::new` discards
    /// the remainder rows, and the page must size its canvas to the PANEL or it
    /// draws at the wrong aspect. `Grid::new` is where that story lives.
    pub fn describe(&self, saver: &str, g: &Grid, panel: &Panel, pal: &[u32]) {
        // Bump the epoch FIRST and bake that number into the JSON, so the page
        // can hand it back on `/stream` and be refused if the scene has moved
        // on. The pairing cannot tear because this is the only writer of either
        // and the compare happens under the same `frame` lock: a viewer either
        // gets meta and cells from one epoch, or gets nothing. Before this, two
        // quick clicks gave the page saver A's geometry, palette and glyph
        // table with saver B's cells, and it drew garbage until clicked again.
        let epoch = {
            let mut f = self.frame.lock().unwrap();
            f.epoch += 1;
            f.gen += 1;
            f.cells.clear();
            f.epoch
        };
        self.ready.notify_all();

        let mut json = String::with_capacity(8 << 10);
        let _ = write!(
            json,
            "{{\"saver\":\"{saver}\",\"savers\":[{savers}],\"epoch\":{epoch},\
             \"panel_w\":{pw},\"panel_h\":{ph},\"pixel_aspect\":{pa},\"panel_mm\":{pmm},\
             \"cols\":{cols},\"rows\":{rows},\
             \"cell_w\":{cw},\"cell_h\":{ch},\"ground\":{ground},\
             \"glyph_w\":{gw},\"glyph_h\":{gh},\"groups\":{groups},\"wide\":{wide},\"palette\":[",
            groups = groups_json(),
            wide = wide_json(),
            gw = font::GLYPH_W,
            gh = font::GLYPH_H,
            pw = panel.w,
            ph = panel.h,
            pa = crate::grid::pixel_aspect(),
            pmm = crate::grid::panel_mm(),
            cols = g.cols(),
            rows = g.rows(),
            cw = g.cell_w(),
            ch = g.cell_h(),
            ground = g.ground() & 0xFF_FFFF,
            savers = crate::saver::names()
                .map(|n| format!("\"{n}\""))
                .collect::<Vec<_>>()
                .join(",")
        );
        for (i, c) in pal.iter().enumerate() {
            let _ = write!(json, "{}{}", if i > 0 { "," } else { "" }, c & 0xFF_FFFF);
        }
        json.push_str("],\"glyphs\":[");
        for (i, g) in font::GLYPHS.iter().enumerate() {
            json.push_str(if i > 0 { ",[" } else { "[" });
            for (j, b) in g.iter().enumerate() {
                let _ = write!(json, "{}{b}", if j > 0 { "," } else { "" });
            }
            json.push(']');
        }
        // No closing brace: the `/meta` route appends the live rotation
        // interval and closes the object there.
        json.push(']');
        *self.meta.lock().unwrap() = json;
    }
}

/// Bind and accept forever. Returns only if the bind fails — a mirror that
/// cannot listen must not take the screensaver down with it.
pub fn serve(mirror: &Arc<Mirror>, addr: &str) {
    let listener = match TcpListener::bind(addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[screensaver] mirror: bind {addr}: {e}; web mirror disabled");
            return;
        }
    };
    eprintln!("[screensaver] mirror listening on {addr}");
    for stream in listener.incoming().flatten() {
        let m = Arc::clone(mirror);
        // Thread per connection: a request is a route match and then either a
        // one-shot body or a long-lived stream, and a thread models the second
        // one for free. Concurrency is bounded by MAX_VIEWERS at the only route
        // that holds a thread open.
        std::thread::spawn(move || {
            let _ = handle(&m, stream);
        });
    }
}

/// Routes that change what the panel does. POST only — a GET must not be able
/// to, and everything else on any other path is a 405 so `POST /stream` cannot
/// take a viewer slot and hold a thread.
const WRITES: &[&str] = &["/select", "/rotate", "/rotation"];

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
        (m, p) if WRITES.contains(&p) && m != "POST" => send(
            &mut s,
            "405 Method Not Allowed",
            "text/plain",
            format!("method not allowed: POST {p}\n").as_bytes(),
        ),
        // Read, set and reset, by method — the one route with all three.
        (m @ ("GET" | "POST" | "DELETE"), "/config") => {
            let (status, body) = config_route(mirror, m, &query);
            send(&mut s, status, "application/json", body.as_bytes())
        }
        (m, p) if m != "GET" && !WRITES.contains(&p) => send(
            &mut s,
            "405 Method Not Allowed",
            "text/plain",
            b"method not allowed",
        ),
        // A GET must not be able to change the saver.
        // Answers once the render loop has built the saver, with its `/meta`,
        // so the page opens `/stream` on the new epoch in one round trip —
        // no reconnect after a dropped stream, no 409 for racing the switch.
        ("POST", "/select") => match param(&query, "saver") {
            Some(name) if mirror.select(&name) => {
                let body = applied_meta(mirror, mirror.selection())
                    .unwrap_or_else(|| selected_json(mirror));
                send(&mut s, "200 OK", "application/json", body.as_bytes())
            }
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
        // Minutes, because that is the unit anyone setting this thinks in; the
        // renderer's own unit is seconds and `/meta` reports those. 0 is off.
        ("POST", "/rotate") => {
            if let Some(mins) = param(&query, "mins")
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|m| *m <= MAX_ROTATE_MINS)
            {
                mirror.set_rotate_secs(mins * 60);
                send(
                    &mut s,
                    "200 OK",
                    "application/json",
                    rotate_json(mirror).as_bytes(),
                )
            } else {
                // Anything else changes nothing: a garbled number must not be able
                // to turn rotation off, which is what a lenient parse would do.
                eprintln!(
                    "[screensaver] rejected rotate mins {:?}",
                    param(&query, "mins")
                );
                send(
                    &mut s,
                    "400 Bad Request",
                    "application/json",
                    br#"{"error":"rotate mins must be 0..=1440"}"#,
                )
            }
        }
        // In or out of rotation, by saver (its twin follows) or by group.
        ("POST", "/rotation") => {
            let on = match param(&query, "on").as_deref() {
                Some("1") => Some(true),
                Some("0") => Some(false),
                _ => None,
            };
            let names: Option<Vec<&str>> = match (param(&query, "saver"), param(&query, "group")) {
                (Some(n), None) => saver::index_of(&n).map(|i| vec![saver::name_at(i)]),
                (None, Some(g)) => saver::GROUPS.iter().position(|x| *x == g).map(|g| {
                    saver::names()
                        .enumerate()
                        .filter(|&(i, _)| saver::group_at(i) == g)
                        .map(|(_, n)| n)
                        .collect()
                }),
                _ => None,
            };
            match (on, names) {
                (Some(on), Some(names)) => {
                    for n in names {
                        mirror.set_in_rotation(n, on);
                    }
                    send(
                        &mut s,
                        "200 OK",
                        "application/json",
                        format!("{{\"excluded\":{}}}", excluded_json(mirror)).as_bytes(),
                    )
                }
                _ => send(
                    &mut s,
                    "400 Bad Request",
                    "application/json",
                    br#"{"error":"rotation takes saver=<name> or group=<name>, and on=0|1"}"#,
                ),
            }
        }
        (_, "/") => send(
            &mut s,
            "200 OK",
            "text/html; charset=utf-8",
            PAGE.as_bytes(),
        ),
        (_, "/meta") => match meta_json(mirror) {
            Some(meta) => send(&mut s, "200 OK", "application/json", meta.as_bytes()),
            // No modeset yet: the pod is up but idling on a node with no
            // monitor. A 503 is the honest answer and the page retries.
            None => send(&mut s, "503 Service Unavailable", "application/json", b"{}"),
        },
        // Live counters, deliberately NOT folded into /meta: that is a String
        // cached at modeset time, so a counter baked into it would report its
        // value as of the last modeset forever.
        (_, "/stat") => send(
            &mut s,
            "200 OK",
            "application/json",
            stat_json(mirror).as_bytes(),
        ),
        (_, "/stream") => stream(mirror, s, &query),
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

/// The value of `key`, percent-decoded. A malformed escape is None, the same
/// as a missing parameter: a route never acts on a value it half-read.
fn param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|kv| {
        kv.split_once('=')
            .filter(|(k, _)| *k == key)
            .and_then(|(_, v)| decode(v))
    })
}

fn decode(v: &str) -> Option<String> {
    let b = v.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 2;
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8(out).ok()
}

/// `"a\"b"` as a JSON string literal. String knobs are typed by a person, so
/// their values are the one thing in these bodies that is not ours.
fn json_str(v: &str) -> String {
    let mut out = String::with_capacity(v.len() + 2);
    out.push('"');
    for c in v.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The page's list: each group's savers in table order, a `-wide` twin left
/// out because its scene is listed once — see `wide_json`.
fn groups_json() -> String {
    let names: Vec<_> = saver::names().collect();
    let groups: Vec<String> = saver::GROUPS
        .iter()
        .enumerate()
        .map(|(g, group)| {
            let members: Vec<String> = names
                .iter()
                .enumerate()
                .filter(|&(i, n)| saver::group_at(i) == g && !is_twin(n))
                .map(|(_, n)| json_str(n))
                .collect();
            format!(
                "{{\"name\":{},\"savers\":[{}]}}",
                json_str(group),
                members.join(",")
            )
        })
        .collect();
    format!("[{}]", groups.join(","))
}

fn is_twin(name: &str) -> bool {
    name.strip_suffix("-wide")
        .is_some_and(|base| saver::index_of(base).is_some())
}

/// Scene -> its full-width twin, for the page's `expanded` toggle.
fn wide_json() -> String {
    let pairs: Vec<String> = saver::names()
        .filter_map(|n| saver::wide_of(n).map(|w| format!("{}:{}", json_str(n), json_str(w))))
        .collect();
    format!("{{{}}}", pairs.join(","))
}

/// Rows out of rotation. The short list, since everything starts in.
fn excluded_json(mirror: &Mirror) -> String {
    let names: Vec<String> = saver::names()
        .enumerate()
        .filter(|&(i, _)| !mirror.in_rotation(i))
        .map(|(_, n)| json_str(n))
        .collect();
    format!("[{}]", names.join(","))
}

/// The `-wide` halves whose pair is expanded: what a click on the pair's row
/// shows and what rotation picks. Live, since a click moves it.
fn expanded_json(mirror: &Mirror) -> String {
    let names: Vec<String> = saver::names()
        .enumerate()
        .filter(|&(i, n)| {
            n.ends_with("-wide") && mirror.twins[i].is_some() && mirror.pickable_half(i)
        })
        .map(|(_, n)| json_str(n))
        .collect();
    format!("[{}]", names.join(","))
}

/// `/meta`: the JSON cached at modeset, closed with the live values — the
/// interval and the rotation set move between modesets, and a stale one is
/// a page showing what the panel is not doing. None before the first
/// modeset.
fn meta_json(mirror: &Mirror) -> Option<String> {
    let mut meta = mirror.meta.lock().unwrap().clone();
    if meta.is_empty() {
        return None;
    }
    let _ = write!(
        meta,
        ",\"rotate_secs\":{},\"excluded\":{},\"expanded\":{}}}",
        mirror.rotate_secs(),
        excluded_json(mirror),
        expanded_json(mirror)
    );
    Some(meta)
}

/// `/meta` once the render loop has built selection `word`, or None if it
/// has not within `APPLY_WAIT`.
fn applied_meta(mirror: &Mirror, word: u64) -> Option<String> {
    mirror
        .wait_applied(word, APPLY_WAIT)
        .then(|| meta_json(mirror))
        .flatten()
}

/// The knobs `name`'s constructor reads, found by building it here with the
/// recorder on — see `config`. On the HTTP thread, never the render thread,
/// and at a small panel: which knobs a saver reads does not depend on the
/// panel, and building at 1080p would only cost time.
fn knobs_of(i: usize, fps: u32) -> Vec<config::Knob> {
    let name = saver::name_at(i);
    config::knobs_of(name, || {
        saver::make(name, &Panel::new(320, 180, 320), fps);
    })
}

fn config_json(knobs: &[config::Knob]) -> String {
    let rows: Vec<String> = knobs
        .iter()
        .map(|k| {
            let (kind, range, default) = match &k.kind {
                config::Kind::Num { default, lo, hi } => (
                    if (*lo, *hi) == (0, 1) { "bool" } else { "num" },
                    format!(",\"lo\":{lo},\"hi\":{hi}"),
                    default.to_string(),
                ),
                config::Kind::Str { default } => ("str", String::new(), json_str(default)),
            };
            let value = config::effective(k);
            format!(
                "{{\"key\":{key},\"label\":{label},\"kind\":\"{kind}\",\"default\":{default}{range},\
                 \"value\":{value},\"overridden\":{over},\"help\":{help}}}",
                key = json_str(k.key),
                label = json_str(&config::label(k.key, knobs)),
                value = match k.kind {
                    config::Kind::Num { .. } => value,
                    config::Kind::Str { .. } => json_str(&value),
                },
                over = config::is_overridden(k.key),
                help = config::help(k.key).map_or("null".into(), json_str),
            )
        })
        .collect();
    format!("[{}]", rows.join(","))
}

/// `GET` lists `saver`'s knobs; `POST key=K&value=V` overrides one, and
/// `DELETE key=K` (or an empty value) drops the override; both answer
/// `{"rebuilt":bool,"knobs":[...]}`. A write rebuilds the
/// saver on the panel if it reads that key — the saver named, or another that
/// shares it, as every scene shares the tour's — through the same selection
/// word a click moves, so the epoch and reconnect story is a switch's.
fn config_route(mirror: &Mirror, method: &str, query: &str) -> (&'static str, String) {
    let bad = |msg: &str| {
        (
            "400 Bad Request",
            format!("{{\"error\":{}}}", json_str(msg)),
        )
    };
    let Some((i, name)) = param(query, "saver").and_then(|n| saver::index_of(&n).map(|i| (i, n)))
    else {
        return bad("unknown saver");
    };
    let knobs = knobs_of(i, mirror.fps);
    if method == "GET" {
        return ("200 OK", config_json(&knobs));
    }
    let key = param(query, "key").unwrap_or_default();
    let Some(knob) = knobs.iter().find(|k| k.key == key) else {
        return bad(&format!("{key:?} is not a knob of {name}"));
    };
    match param(query, "value").filter(|v| method == "POST" && !v.is_empty()) {
        Some(v) => {
            if let Err(e) = config::set(knob, &v) {
                return bad(&format!("{key}: {e}"));
            }
        }
        None => config::reset(&key),
    }
    let cur = mirror.selected();
    let rebuilt = knobs_of(cur, mirror.fps).iter().any(|k| k.key == key) && mirror.reselect(cur);
    // As `/select`: the rebuilt saver's `/meta`, once it is on the panel.
    let meta = if rebuilt {
        applied_meta(mirror, mirror.selection())
    } else {
        None
    };
    // Re-read: a switch like ASCII_REST_TOUR decides which other knobs exist.
    // `rebuilt` so the page can drop its stream itself, as it does for a
    // `/select`, rather than be cut off mid-chunk.
    (
        "200 OK",
        format!(
            "{{\"rebuilt\":{rebuilt},\"knobs\":{},\"meta\":{}}}",
            config_json(&knobs_of(i, mirror.fps)),
            meta.as_deref().unwrap_or("null")
        ),
    )
}

fn stat_json(mirror: &Mirror) -> String {
    format!(
        "{{\"overruns\":{},\"viewers\":{},\"fps\":{}}}",
        mirror.overruns(),
        mirror.viewers.load(Ordering::Relaxed),
        mirror.fps,
    )
}

/// Seconds, not the minutes that were posted: the page shows what the renderer
/// will actually use, and `/meta` says the same thing in the same unit.
fn rotate_json(mirror: &Mirror) -> String {
    format!("{{\"rotate_secs\":{}}}", mirror.rotate_secs())
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
fn stream(mirror: &Mirror, mut s: TcpStream, query: &str) -> std::io::Result<()> {
    // The epoch the page read from `/meta`, checked under the same lock
    // `describe` bumps it under, before a viewer slot or a single frame is
    // committed. That is the whole fix for the half-switched panel: cells are
    // only meaningful against the geometry, palette and glyph table of their
    // own epoch, and `/meta` + `/stream` are two round trips with a `/select`
    // able to land between them. Absent (a curl, an old page) means unchecked.
    let want = param(query, "epoch").and_then(|v| v.parse::<u64>().ok());
    let epoch = mirror.frame.lock().unwrap().epoch;
    if want.is_some_and(|w| w != epoch) {
        return send(&mut s, "409 Conflict", "text/plain", b"stale epoch");
    }

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
            // Ended with the zero-length chunk, so the browser reads a clean
            // end of stream: a socket simply closed mid-body is a network
            // error (ERR_INCOMPLETE_CHUNKED_ENCODING) on every switch.
            if f.epoch != epoch {
                drop(f);
                s.write_all(b"0\r\n\r\n")?;
                return s.flush();
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
    /// the cover cell by default, past it at 600%; then the switch off, and
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

    /// `/meta`'s `expanded` is the Rust default until a pick moves it: every
    /// scene starts on its `-wide`, the text pieces on their originals, and a
    /// pick of either half sticks. Picking by index — rotation, and the
    /// startup `SAVER` — moves nothing.
    #[test]
    fn pairs_start_expanded_and_a_pick_sticks() {
        let m = Mirror::new(15);
        scene(&m, "matrix", 2, 2, 8, 16);
        let expanded = || {
            let meta = meta_json(&m).unwrap();
            let tail = meta.rsplit_once(",\"expanded\":").unwrap().1.to_string();
            tail
        };
        let e = expanded();
        assert!(e.starts_with(r#"["alpine-dawn-wide","#), "{e}");
        assert!(
            e.contains(r#""night-coast-wide""#) && e.contains(r#""vinyl-wide""#),
            "{e}"
        );
        assert_eq!(e.matches("-wide").count(), 21, "{e}");

        m.select_at(saver::index_of("night-coast").unwrap());
        assert!(expanded().contains(r#""night-coast-wide""#));
        assert!(m.select("night-coast"));
        assert!(!expanded().contains("night-coast"));
        assert!(m.select("vinyl"));
        assert!(!expanded().contains("vinyl"));
        assert!(m.select("vinyl-wide"));
        assert!(expanded().contains(r#""vinyl-wide""#));
        assert!(m.select("night-coast-wide"));
        assert!(expanded().contains(r#""night-coast-wide""#));
    }

    #[test]
    fn meta_lists_groups_twins_and_the_rotation() {
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
            !head.contains(r#""savers":["alpine-dawn","alpine-dawn-wide""#),
            "{head}"
        );
        assert!(
            head.contains(r#""night-coast":"night-coast-wide""#),
            "{head}"
        );
        assert!(
            head.contains(r#"{"name":"classics","savers":["ascii","blocks","matrix""#),
            "{head}"
        );
        let tail = |b: &str| b.rsplit_once("]],").unwrap().1.to_string();
        assert!(tail(&body).contains(r#""excluded":[]"#), "{}", tail(&body));

        // A scene takes its twin with it; a group takes every member.
        let body = req(addr, "POST /rotation?saver=night-coast&on=0");
        assert!(body.starts_with("HTTP/1.1 200 "), "{body}");
        assert!(
            body.ends_with(r#"{"excluded":["night-coast","night-coast-wide"]}"#),
            "{body}"
        );
        let body = req(addr, "POST /rotation?group=flights&on=0");
        assert!(body.contains(r#""warp","hypercube","pov""#), "{body}");
        let body = req(addr, "POST /rotation?group=flights&on=1");
        assert!(
            body.ends_with(r#"{"excluded":["night-coast","night-coast-wide"]}"#),
            "{body}"
        );
        assert!(tail(&req(addr, "GET /meta"))
            .contains(r#""excluded":["night-coast","night-coast-wide"]"#));
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
        req(addr, "POST /rotation?saver=night-coast-wide&on=1");
        assert!(tail(&req(addr, "GET /meta")).contains(r#""excluded":[]"#));

        assert_eq!(m.exclude("dvd, nope,,matrix"), ["nope"]);
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
        assert!(body.contains(r#""rotate_secs":0,"excluded":[],"expanded":["#));
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
}
