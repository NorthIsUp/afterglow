//! `micropolis`: the open-source `SimCity`, a city an AI mayor builds on its
//! own, filling the panel with a slow camera over the map.
//!
//! Only in a `--features micropolis` build, which links the Micropolis engine
//! and is GPL-3.0 as a whole; see `THIRD_PARTY.md`. The engine and the mayor
//! run on their own thread (`engine.rs`); this saver draws the last finished
//! map straight into the panel's pixels from a pre-scaled tile atlas, and
//! only the tiles that changed unless the camera moved.
//!
//! The grid is for the mirror and the terminal: one solid cell per tile, in
//! the tile's mean colour.

mod cities;
mod engine;
mod mayor;
mod power;
mod tiles;

use std::io::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::grid::{pixel_aspect, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, font, glyph, saver_seed};

use cities::Name;
use engine::{Engine, Stats, Want, CELLS};
use mayor::{H, W};
use tiles::{is_clear, is_water, COUNT};

/// Each saver instance's claim on the engine; see `doom`'s for why not a
/// pointer.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

const UNDRAWN: u16 = u16::MAX;
const HUD_MAX: usize = 64;
const HUD_INK: u32 = 0xF2F2E8;
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

pub struct Micropolis {
    grid: Grid,
    palette: Vec<u32>,
    id: u64,
    /// Taken by the first `render`: the mirror builds savers just to read
    /// their knobs, and that must not claim the engine.
    want: Option<Want>,
    engaged: bool,
    panel: (usize, usize),
    /// A tile's size on the panel, and every tile at that size.
    tw: usize,
    th: usize,
    atlas: Vec<u32>,
    map: Vec<u16>,
    stats: Stats,
    name: Name,
    seq: u32,
    /// Per map cell, the tile the panel shows there, `UNDRAWN` if unknown.
    drawn: Vec<u16>,
    /// The camera's top-left in map pixels: the float it drifts in, the
    /// whole pixels last drawn at, and the drift's two phases.
    cam: (f32, f32),
    cam_drawn: Option<(i32, i32)>,
    phase: (f32, f32),
    /// The developed part of the map in tiles, `x0, y0, x1, y1`.
    bbox: (i32, i32, i32, i32),
    pan: f32,
    fps: f32,
    frame: u64,
    hud: bool,
    /// The overlay's text, its panel rect and glyph scale.
    text: [u8; HUD_MAX],
    text_len: usize,
    hud_drawn: Option<([u8; HUD_MAX], usize)>,
    hud_rect: (usize, usize, usize, usize),
    glyph_h: usize,
    glyph_fx: f32,
    first: bool,
}

/// A tile's width and height in panel pixels: square on the glass, and big
/// enough that the view spans `pct` per cent of the map along whichever axis
/// the panel's shape makes tightest, so the map always fills the panel.
fn layout(panel: &Panel, aspect: usize, pct: usize) -> (usize, usize) {
    let fit_th = (panel.h as f32 / H as f32).max(panel.w as f32 / W as f32 * aspect as f32 / 100.0);
    let th = ((fit_th * 100.0 / pct as f32).round() as usize).max(panel.h.div_ceil(H as usize));
    let tw = ((th * 100) as f32 / aspect as f32)
        .round()
        .max(panel.w.div_ceil(W as usize) as f32) as usize;
    (tw.max(1), th.max(1))
}

impl Micropolis {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        let pct = env_num(&["MICROPOLIS_VIEW_PCT"], 70, 20, 100) as usize;
        let pan = env_num(&["MICROPOLIS_PAN"], 6, 0, 120) as f32;
        let hud = env_num(&["MICROPOLIS_HUD"], 1, 0, 1) == 1;
        let want = Want {
            seed: saver_seed(&["MICROPOLIS_SEED"], 1),
            year_secs: env_num(&["MICROPOLIS_YEAR_SECS"], 40, 1, 3600) as u32,
            city_mins: env_num(&["MICROPOLIS_CITY_MINS"], 120, 0, 10_080) as u32,
            disaster_mins: env_num(&["MICROPOLIS_DISASTER_MINS"], 25, 0, 10_080) as u32,
            bundled_pct: env_num(&["MICROPOLIS_BUNDLED_PCT"], 25, 0, 100) as u32,
        };
        Self::build(panel, fps, pixel_aspect(), pct, pan, hud, want)
    }

    fn build(
        panel: &Panel,
        fps: u32,
        aspect: usize,
        pct: usize,
        pan: f32,
        hud: bool,
        want: Want,
    ) -> Self {
        let (tw, th) = layout(panel, aspect, pct);
        let glyph_h = (panel.h.min(panel.w * 100 / aspect) / 400).max(1);
        let glyph_fx = glyph_h as f32 * 100.0 / aspect as f32;
        let mut m = Self {
            grid: Grid::with_aspect(panel, tw, (th * 100 / aspect).max(1), aspect),
            palette: tiles::means(),
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            want: Some(want),
            engaged: false,
            panel: (panel.w, panel.h),
            tw,
            th,
            atlas: tiles::atlas(tw, th),
            map: vec![0; CELLS],
            stats: Stats::default(),
            name: Name::default(),
            seq: 0,
            drawn: vec![UNDRAWN; CELLS],
            cam: (0.0, 0.0),
            cam_drawn: None,
            phase: (0.0, 0.0),
            bbox: (W / 2, H / 2, W / 2, H / 2),
            pan,
            fps: fps.max(1) as f32,
            frame: 0,
            hud,
            text: [0; HUD_MAX],
            text_len: 0,
            hud_drawn: None,
            hud_rect: (0, 0, 0, 0),
            glyph_h,
            glyph_fx,
            first: true,
        };
        m.cam = m.aim();
        m
    }

    fn cam_max(&self) -> (f32, f32) {
        let (pw, ph) = self.panel;
        (
            (W as usize * self.tw).saturating_sub(pw) as f32,
            (H as usize * self.th).saturating_sub(ph) as f32,
        )
    }

    /// Where the camera's drift puts it now: a slow sweep across what is
    /// built, centred on it while it fits in view, clamped to the map.
    fn aim(&self) -> (f32, f32) {
        let (pw, ph) = (self.panel.0 as f32, self.panel.1 as f32);
        let (mx, my) = self.cam_max();
        let (x0, y0, x1, y1) = self.bbox;
        let axis = |a: i32, b: i32, t: usize, view: f32, phase: f32, max: f32| {
            let (lo, hi) = (a as f32 * t as f32, (b + 1) as f32 * t as f32);
            let span = (hi - lo - view).max(0.0);
            let c = lo + (hi - lo - view).min(0.0) / 2.0 + span * (0.5 - 0.5 * phase.cos());
            c.clamp(0.0, max)
        };
        (
            axis(x0, x1, self.tw, pw, self.phase.0, mx),
            axis(y0, y1, self.th, ph, self.phase.1, my),
        )
    }

    /// Move the camera one frame along its drift, no faster than `pan`.
    fn drift(&mut self) {
        let step = self.pan / self.fps;
        let (x0, y0, x1, y1) = self.bbox;
        let sx = ((x1 - x0 + 1) as f32 * self.tw as f32).max(1.0);
        let sy = ((y1 - y0 + 1) as f32 * self.th as f32).max(1.0);
        // The two axes at slightly different rates so the sweep never
        // retraces itself.
        self.phase.0 += 2.0 * step / sx;
        self.phase.1 += 1.6 * step / sy;
        let (ax, ay) = self.aim();
        let toward = |c: f32, a: f32| c + (a - c).clamp(-step, step);
        self.cam = (toward(self.cam.0, ax), toward(self.cam.1, ay));
    }

    fn take_map(&mut self) -> bool {
        let Some((seq, stats, name)) = Engine::get().latest(self.seq, &mut self.map) else {
            return false;
        };
        let new_city = self.seq == 0 || name != self.name || stats.city_time < self.stats.city_time;
        (self.seq, self.stats, self.name) = (seq, stats, name);
        let (mut x0, mut y0, mut x1, mut y1) = (W, H, -1, -1);
        for x in 0..W {
            for y in 0..H {
                let t = self.map[(x * H + y) as usize] & tiles::LOMASK;
                if !is_clear(t) && !is_water(t) {
                    (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                }
            }
        }
        if x1 >= 0 {
            self.bbox = (x0 - 3, y0 - 3, x1 + 3, y1 + 3);
        }
        if new_city {
            // A new city cuts to it rather than drifting over from the last.
            self.phase = (0.0, 0.0);
            self.cam = self.aim();
        }
        true
    }

    /// The tile under panel pixel row `py` of tile row `ty`, as the atlas
    /// slice of that row from map pixel column `mx`, `n` pixels long.
    #[inline]
    fn atlas_row(&self, tile: u16, row: usize, from: usize, n: usize) -> &[u32] {
        let t = (tile as usize).min(COUNT - 1);
        &self.atlas[(t * self.th + row) * self.tw + from..][..n]
    }

    /// Visible tiles as (map x range, map y range).
    fn visible(&self, cam: (i32, i32)) -> (std::ops::Range<i32>, std::ops::Range<i32>) {
        let (pw, ph) = (self.panel.0 as i32, self.panel.1 as i32);
        let (tw, th) = (self.tw as i32, self.th as i32);
        (
            cam.0 / tw..((cam.0 + pw - 1) / tw + 1).min(W),
            cam.1 / th..((cam.1 + ph - 1) / th + 1).min(H),
        )
    }

    fn redraw_all(&mut self, s: &mut Surface<'_>, cam: (i32, i32), blink: bool) {
        let (pw, ph) = (self.panel.0 as i32, self.panel.1 as i32);
        let (tw, th) = (self.tw as i32, self.th as i32);
        let (xs, ys) = self.visible(cam);
        for ty in ys {
            let y0 = ty * th - cam.1;
            let (ya, yb) = (y0.max(0), (y0 + th).min(ph));
            for x in xs.clone() {
                let i = (x * H + ty) as usize;
                self.drawn[i] = tiles::shown(self.map[i], blink);
            }
            let rows = s.cell_rows(0, ya as usize, pw as usize, (yb - ya) as usize);
            for (py, out) in (ya..yb).zip(rows) {
                let row = (py - y0) as usize;
                for x in xs.clone() {
                    let x0 = x * tw - cam.0;
                    let (xa, xb) = (x0.max(0), (x0 + tw).min(pw));
                    let tile = self.drawn[(x * H + ty) as usize];
                    out[xa as usize..xb as usize].copy_from_slice(self.atlas_row(
                        tile,
                        row,
                        (xa - x0) as usize,
                        (xb - xa) as usize,
                    ));
                }
            }
        }
    }

    /// Redraw the tiles whose picture changed. Returns whether any of them
    /// lies under the overlay.
    fn redraw_changed(&mut self, s: &mut Surface<'_>, cam: (i32, i32), blink: bool) -> bool {
        let (pw, ph) = (self.panel.0 as i32, self.panel.1 as i32);
        let (tw, th) = (self.tw as i32, self.th as i32);
        let (hx, hy, hw, hh) = self.hud_rect;
        let (hx, hy, hw, hh) = (hx as i32, hy as i32, hw as i32, hh as i32);
        let mut under = false;
        let (xs, ys) = self.visible(cam);
        for x in xs {
            for ty in ys.clone() {
                let i = (x * H + ty) as usize;
                let tile = tiles::shown(self.map[i], blink);
                if tile == self.drawn[i] {
                    continue;
                }
                self.drawn[i] = tile;
                let (x0, y0) = (x * tw - cam.0, ty * th - cam.1);
                let (xa, xb) = (x0.max(0), (x0 + tw).min(pw));
                let (ya, yb) = (y0.max(0), (y0 + th).min(ph));
                under |= xa < hx + hw && xb > hx && ya < hy + hh && yb > hy;
                let rows = s.cell_rows(
                    xa as usize,
                    ya as usize,
                    (xb - xa) as usize,
                    (yb - ya) as usize,
                );
                for (py, out) in (ya..yb).zip(rows) {
                    let src =
                        self.atlas_row(tile, (py - y0) as usize, (xa - x0) as usize, out.len());
                    out.copy_from_slice(src);
                }
            }
        }
        under
    }

    /// The overlay's line for the current stats, into `text`.
    fn compose(&mut self) {
        let s = &self.stats;
        let mut buf = [0u8; HUD_MAX];
        let mut w = std::io::Cursor::new(&mut buf[..]);
        let dead = if Engine::get().dead() {
            "  ·  stopped"
        } else {
            ""
        };
        let _ = write!(
            w,
            "{}{}  ·  pop {}  ·  {} {}  ·  ${}{dead}",
            self.name.0,
            self.name.1,
            Thousands(s.pop),
            MONTHS[s.month.clamp(0, 11) as usize],
            s.year,
            Thousands(s.funds)
        );
        let n = w.position() as usize;
        (self.text, self.text_len) = (buf, n);
    }

    fn text_chars(&self) -> impl Iterator<Item = char> + '_ {
        std::str::from_utf8(&self.text[..self.text_len])
            .unwrap_or("")
            .chars()
    }

    /// Lay out the overlay box for the current text: the bottom-left corner,
    /// a glyph's width in from the edges.
    fn place_hud(&mut self) -> (usize, usize, usize, usize) {
        let n = self.text_chars().count();
        let gw = (8.0 * self.glyph_fx).round() as usize;
        let (pad_x, pad_y) = ((6.0 * self.glyph_fx) as usize, 4 * self.glyph_h);
        let w = (n * gw + 2 * pad_x).min(self.panel.0);
        let h = (16 * self.glyph_h + 2 * pad_y).min(self.panel.1);
        let margin_x = gw.min(self.panel.0 - w);
        let margin_y = (2 * self.glyph_h).min(self.panel.1 - h);
        (margin_x, self.panel.1 - h - margin_y, w, h)
    }

    /// The overlay: the map under it at half brightness, then the text.
    fn draw_hud(&mut self, s: &mut Surface<'_>, cam: (i32, i32)) {
        let (hx, hy, hw, hh) = self.hud_rect;
        let (tw, th) = (self.tw, self.th);
        let (pad_x, pad_y) = ((6.0 * self.glyph_fx) as usize, 4 * self.glyph_h);
        let gw = (8.0 * self.glyph_fx).round() as usize;
        let rows = s.cell_rows(hx, hy, hw, hh);
        for (dy, out) in rows.enumerate() {
            let my = (cam.1 as usize + hy + dy).min(H as usize * th - 1);
            let (ty, row) = (my / th, my % th);
            for (dx, px) in out.iter_mut().enumerate() {
                let mx = (cam.0 as usize + hx + dx).min(W as usize * tw - 1);
                let t = self.drawn[mx / tw * H as usize + ty];
                let t = if t == UNDRAWN { 0 } else { t as usize };
                let c = self.atlas[(t * th + row) * tw + mx % tw];
                *px = (c >> 1 & 0x7f7f7f) + (0x0a0c10 >> 1);
            }
            let Some(gy) = (dy.checked_sub(pad_y)).filter(|&g| g < 16 * self.glyph_h) else {
                continue;
            };
            let line = gy / self.glyph_h;
            let text = std::str::from_utf8(&self.text[..self.text_len]).unwrap_or("");
            for (k, c) in text.chars().enumerate() {
                let bits = font::GLYPHS[glyph::of(c) as usize][line];
                if bits == 0 {
                    continue;
                }
                let x0 = pad_x + k * gw;
                for u in 0..8 {
                    if bits & (0x80 >> u) == 0 {
                        continue;
                    }
                    let a = x0 + (u as f32 * self.glyph_fx) as usize;
                    let b = (x0 + ((u + 1) as f32 * self.glyph_fx) as usize).max(a + 1);
                    for px in out.iter_mut().take(b.min(hw)).skip(a) {
                        *px = HUD_INK;
                    }
                }
            }
        }
    }
}

/// An integer with thousands separators, written without allocating.
struct Thousands(i32);

impl std::fmt::Display for Thousands {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.0;
        if n < 0 {
            f.write_str("-")?;
        }
        let n = n.unsigned_abs();
        let mut div = 1;
        while n / div >= 1000 {
            div *= 1000;
        }
        write!(f, "{}", n / div)?;
        while div > 1 {
            div /= 1000;
            write!(f, ",{:03}", n / div % 1000)?;
        }
        Ok(())
    }
}

impl Saver for Micropolis {
    fn render(&mut self, s: &mut Surface<'_>) {
        if let Some(w) = self.want.take() {
            Engine::get().claim(self.id, w);
            self.engaged = true;
        }
        let fresh = self.engaged && self.take_map();
        self.drift();
        let cam = (self.cam.0.round() as i32, self.cam.1.round() as i32);
        // The lightning bolt over an unpowered zone blinks once a second.
        let blink = (self.frame as f32 / self.fps).fract() < 0.5;
        self.frame += 1;
        let moved = self.cam_drawn != Some(cam);
        let mut hud_dirty = false;
        if moved || self.first {
            self.redraw_all(s, cam, blink);
            self.cam_drawn = Some(cam);
            hud_dirty = true;
        } else {
            hud_dirty |= self.redraw_changed(s, cam, blink);
        }
        if self.hud {
            if fresh || self.first {
                self.compose();
            }
            let text = (self.text, self.text_len);
            if self.hud_drawn != Some(text) {
                let old = self.hud_rect;
                self.hud_rect = self.place_hud();
                if old.2 > self.hud_rect.2 {
                    // A shorter line uncovers map the old box hid.
                    self.forget(old, cam);
                    self.redraw_changed(s, cam, blink);
                }
                hud_dirty = true;
            }
            if hud_dirty {
                self.draw_hud(s, cam);
                self.hud_drawn = Some(text);
            }
        }
        self.first = false;
        if fresh || moved {
            let (tx, ty) = (cam.0 as usize / self.tw, cam.1 as usize / self.th);
            let map = &self.map;
            self.grid.fill(|cx, cy| {
                let (x, y) = ((tx + cx).min(W as usize - 1), (ty + cy).min(H as usize - 1));
                let t = tiles::shown(map[x * H as usize + y], false);
                Cell::new(font::SOLID, t.min(COUNT as u16 - 1))
            });
        }
        self.grid.settle();
    }

    fn name(&self) -> &'static str {
        "micropolis"
    }

    fn grid(&self) -> &Grid {
        &self.grid
    }

    fn palette(&self) -> &[u32] {
        &self.palette
    }
}

impl Micropolis {
    /// Mark the tiles under panel rect `r` as unknown, so the next diff
    /// redraws them.
    fn forget(&mut self, r: (usize, usize, usize, usize), cam: (i32, i32)) {
        let (x, y, w, h) = r;
        let (tw, th) = (self.tw, self.th);
        let (cx, cy) = (cam.0 as usize, cam.1 as usize);
        for ty in (cy + y) / th..=((cy + y + h).saturating_sub(1) / th).min(H as usize - 1) {
            for tx in (cx + x) / tw..=((cx + x + w).saturating_sub(1) / tw).min(W as usize - 1) {
                self.drawn[tx * H as usize + ty] = UNDRAWN;
            }
        }
    }
}

impl Drop for Micropolis {
    fn drop(&mut self) {
        if self.engaged {
            Engine::get().release(self.id);
        }
    }
}

#[cfg(test)]
mod tests;
