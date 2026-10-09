//! The mirror's HTTP side: the page, `/meta`, the control routes and the
//! cell stream, each on its own connection thread.

use std::fmt::Write as _;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::AsRawFd;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::codec::Encoder;
use super::Mirror;
use crate::config;
use crate::grid::Cell;
use crate::saver;
use crate::surface::Panel;

/// Each viewer holds its own `prev` grid and does its own diff. Two people
/// looking at a screensaver is already generous; the cap is what stops a
/// forgotten browser tab farm from costing the render node real CPU.
const MAX_VIEWERS: usize = 4;

/// No viewer frame for this long and the stream emits an empty record, so a
/// browser that went away is noticed instead of parking a thread forever.
const KEEPALIVE: Duration = Duration::from_secs(10);

/// Bytes a second one viewer may be sent. Doom packs to ~50 KB a frame, so it
/// streams every frame at 30fps with room to spare; a saver that packs badly
/// gets fewer frames, not a backlog.
pub(super) const MAX_RATE: u64 = 2 << 20;

/// `SO_SNDBUF` for a stream; see `small_send_buffer`.
const SEND_BUFFER: libc::c_int = 256 << 10;

const PAGE: &str = include_str!("mirror.html");
const STREAM_JS: &str = include_str!("stream.js");
const KNOBS_JS: &str = include_str!("knobs.js");

/// Longest interval `POST /rotate` accepts, in minutes — the same day
/// `SAVER_ROTATE_SECS` tops out at.
const MAX_ROTATE_MINS: u64 = 86_400 / 60;

/// Longest `/select` waits for the render loop to build what it asked for.
/// Past it the pod has no render loop running (no monitor) or is wedged, and
/// the page falls back to reconnecting.
pub(super) const APPLY_WAIT: Duration = Duration::from_secs(2);

/// Cell state that cannot occur: `font::GLYPHS` is nowhere near `u16::MAX`
/// entries, so a `prev` filled with this forces the first
/// diff against it to emit every cell — that is how a connect gets a keyframe
/// without a second code path.
pub(super) const NEVER: Cell = Cell::new(u16::MAX, u16::MAX);

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
const WRITES: &[&str] = &["/select", "/restart", "/rotate", "/rotation"];

pub(super) fn handle(mirror: &Mirror, mut s: TcpStream) -> std::io::Result<()> {
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
        // As `/select`, but a rebuild from scratch even of the saver showing:
        // a saver whose engine outlives a rebuild (gameboy) starts over too.
        ("POST", "/restart") => {
            match param(&query, "saver").filter(|n| saver::index_of(n).is_some()) {
                Some(name) => {
                    saver::request_restart();
                    let i = saver::index_of(&name).expect("filtered above");
                    if !mirror.reselect(i) {
                        mirror.select(&name);
                    }
                    let body = applied_meta(mirror, mirror.selection())
                        .unwrap_or_else(|| selected_json(mirror));
                    send(&mut s, "200 OK", "application/json", body.as_bytes())
                }
                None => send(
                    &mut s,
                    "400 Bad Request",
                    "application/json",
                    br#"{"error":"unknown saver"}"#,
                ),
            }
        }
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
        // In or out of rotation, by saver or by group.
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
        (_, "/stream.js") => send(
            &mut s,
            "200 OK",
            "text/javascript; charset=utf-8",
            STREAM_JS.as_bytes(),
        ),
        (_, "/knobs.js") => send(
            &mut s,
            "200 OK",
            "text/javascript; charset=utf-8",
            KNOBS_JS.as_bytes(),
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
pub(super) fn param(query: &str, key: &str) -> Option<String> {
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
pub(super) fn json_str(v: &str) -> String {
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

/// The page's list: each group's savers in table order.
pub(super) fn groups_json() -> String {
    let names: Vec<_> = saver::names().collect();
    let groups: Vec<String> = saver::GROUPS
        .iter()
        .enumerate()
        .map(|(g, group)| {
            let members: Vec<String> = names
                .iter()
                .enumerate()
                .filter(|&(i, _)| saver::group_at(i) == g)
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

/// Rows out of rotation. The short list, since everything starts in.
fn excluded_json(mirror: &Mirror) -> String {
    let names: Vec<String> = saver::names()
        .enumerate()
        .filter(|&(i, _)| !mirror.in_rotation(i))
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
        ",\"rotate_secs\":{},\"excluded\":{}}}",
        mirror.rotate_secs(),
        excluded_json(mirror)
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
    let Some(i) = param(query, "saver").and_then(|n| saver::index_of(&n)) else {
        return bad("unknown saver");
    };
    let knobs = knobs_of(i, mirror.fps);
    if method == "GET" {
        return ("200 OK", config_json(&knobs));
    }
    let key = param(query, "key").unwrap_or_default();
    let Some(knob) = knobs.iter().find(|k| k.key == key) else {
        return bad(&format!("{key:?} is not a knob of {}", saver::name_at(i)));
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

pub(super) fn stat_json(mirror: &Mirror) -> String {
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

/// One viewer: wait for a frame, encode what changed since this viewer's last
/// one, write it. The encode and the write are both on THIS thread — the
/// render thread's only involvement is the memcpy in `publish`.
///
/// Nothing queues. The write blocks until the socket takes the record, and the
/// next one is built from whatever frame is newest by then, so a viewer that
/// cannot keep up gets fewer frames rather than older ones. The small send
/// buffer and the byte cap keep "by then" short: a kernel buffer autotuned to
/// megabytes is seconds of picture behind at doom's rate.
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
    small_send_buffer(&s);

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

    let mut enc = Encoder::new();
    let mut cur: Vec<Cell> = Vec::new();
    let mut last_gen = 0u64;
    let mut due = Instant::now();

    loop {
        let fresh = {
            let mut f = mirror.frame.lock().unwrap();
            let mut timed_out = false;
            while f.gen == last_gen && !timed_out {
                let (guard, t) = mirror.ready.wait_timeout(f, KEEPALIVE).unwrap();
                f = guard;
                timed_out = t.timed_out();
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
            // `describe` bumps the generation with no cells behind it; a
            // viewer that connects during a modeset waits for a real frame.
            let fresh = !f.cells.is_empty();
            if fresh {
                cur.clear();
                cur.extend_from_slice(&f.cells);
            }
            fresh || timed_out
        };
        if !fresh {
            continue;
        }

        // An idle stream re-sends `cur` against itself: the empty record,
        // which is the keepalive that notices a dead socket.
        let rec = if cur.is_empty() {
            &0u32.to_le_bytes()[..]
        } else {
            enc.encode(&cur)
        };
        let start = Instant::now();
        write_chunk(&mut s, rec)?;
        due = due.max(start) + Duration::from_secs_f64(rec.len() as f64 / MAX_RATE as f64);
        if let Some(wait) = due.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
    }
}

/// Cap the kernel's queue for this viewer at roughly one packed frame plus
/// headroom for a long-RTT link: ~5 MB/s at 50 ms, more than `MAX_RATE`.
fn small_send_buffer(s: &TcpStream) {
    let size: libc::c_int = SEND_BUFFER;
    // SAFETY: a valid socket fd and an int-sized option value, as the call
    // documents; failure leaves the default buffer, which only costs latency.
    unsafe {
        libc::setsockopt(
            s.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_SNDBUF,
            (&raw const size).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
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
