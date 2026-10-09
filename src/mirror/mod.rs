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
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::font;
use crate::grid::{Cell, Grid};
use crate::saver;
use crate::surface::Panel;

mod http;
pub use http::serve;

/// The rotation interval lives in the low 32 bits of the control word.
const ROTATE_SECS: u64 = u32::MAX as u64;

/// The saver index in the low half of a selection word; the high half is a
/// counter bumped on every selection, so re-selecting the saver already
/// showing still reads as a change. See `Mirror::selection`.
pub fn sel_index(word: u64) -> usize {
    (word & u64::from(u32::MAX)) as usize
}

/// Seconds out of a rotation control word. The packing is this module's, so the
/// render loop asks rather than masking a layout it would have to be kept in
/// step with.
pub fn ctl_secs(ctl: u64) -> u64 {
    ctl & ROTATE_SECS
}

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

    /// Put `name` in or out of rotation. False for a name that is not a
    /// saver.
    pub fn set_in_rotation(&self, name: &str, on: bool) -> bool {
        let Some(i) = saver::index_of(name) else {
            return false;
        };
        let bit = 1 << (i % 64);
        if on {
            self.rotation[i / 64].fetch_or(bit, Ordering::Relaxed);
        } else {
            self.rotation[i / 64].fetch_and(!bit, Ordering::Relaxed);
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
             \"glyph_w\":{gw},\"glyph_h\":{gh},\"groups\":{groups},\"palette\":[",
            groups = http::groups_json(),
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

#[cfg(test)]
mod tests;
