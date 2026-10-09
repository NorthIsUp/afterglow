//! `doom`: Freedoom played by an autopilot, one Hor+ widescreen game sized to
//! the panel.
//!
//! Only in a `--features doom` build, which links doomgeneric and is GPL as a
//! whole; see `THIRD_PARTY.md`. The engine runs on its own thread
//! (`engine.rs`) at a width chosen here from the panel's glass shape; this
//! saver scales its last finished frame onto the panel.
//!
//! The panel gets the frame scaled straight into its pixels, a source row at a
//! time, and only the rows the engine changed: every Doom pixel moving every
//! frame makes a per-cell diff and glyph blit the whole cost of the saver.
//! The grid is for the mirror and the terminal: every Doom pixel is SOLID
//! cells whose colour is `palette * 256 + index` into all fourteen PLAYPAL
//! palettes at once, so the damage and pickup tints are colour indices too and
//! the mirror's palette stays fixed for the run.

mod engine;

use std::ffi::CString;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::font;
use crate::grid::{pixel_aspect, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, env_str, next_rand, saver_seed};

use engine::{Engine, Want, H, MAX_W};

const PALETTES: usize = 14;
const STATIC: usize = PALETTES * 256;
const STATIC_LEVELS: usize = 16;
const OFF: u16 = u16::MAX;
/// Doom's 320x200 fills 4:3 glass, so a Doom pixel is 1.2 times taller than
/// wide: a screen `r` times as wide as tall is `240 * r` pixels across.
const PX_PER_RATIO: f32 = 240.0;
const MIN_W: usize = 320;

/// Each saver instance's claim on the engine. Not a pointer: a dropped
/// saver's address can be reused by the next one, and the engine thread must
/// tell a new claim from a stale release.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub struct Doom {
    grid: Grid,
    palette: Vec<u32>,
    id: u64,
    /// Taken by the first `render`, not the constructor: the mirror builds
    /// savers just to read their knobs, and that must not steal the engine.
    want: Option<Want>,
    engaged: bool,
    /// The cells the frame covers, `x0..x1` by `y0..y1`; the rest is margin.
    rect: (usize, usize, usize, usize),
    /// The panel pixels the frame covers, the same way, and the panel.
    px_rect: (usize, usize, usize, usize),
    panel: (usize, usize),
    /// Per panel column of the frame, the source column; and per source
    /// column and row, the first panel column and row it covers, with one
    /// past the end appended.
    x_src: Vec<u16>,
    x_first: Vec<u16>,
    y_first: Vec<u16>,
    /// The frame the panel shows, and the width and palette offset it was
    /// drawn at: a source row equal to its old self needs no pixels.
    drawn: Vec<u8>,
    drawn_key: Option<(usize, usize)>,
    /// One panel row of the frame, gathered once per source row and copied
    /// down the rows it covers.
    line: Vec<u32>,
    /// Per grid column and row: the source pixel, `OFF` outside the frame.
    /// Columns are for a frame `col_w` wide, rebuilt when the engine's width
    /// moves (only after a knob change), in place.
    col_src: Vec<u16>,
    col_w: usize,
    row_src: Vec<u16>,
    pix: Vec<u8>,
    seq: u32,
    /// The frame's width and palette; `None` draws static.
    shown: Option<(usize, u8)>,
    /// `pix` holds a frame the panel has not shown yet.
    fresh: bool,
    /// Frame 0: the buffer arrives zeroed, so the margins need painting once.
    first: bool,
    rng: u32,
}

/// The Doom screen width for a panel, and the share of the panel's width and
/// height (per mille) its picture fills without stretching.
fn layout(panel: &Panel, aspect: usize) -> (usize, usize, usize) {
    let glass = panel.w as f32 * aspect as f32 / (100.0 * panel.h as f32);
    let w = ((PX_PER_RATIO * glass).round() as usize).clamp(MIN_W, MAX_W);
    let own = w as f32 / PX_PER_RATIO;
    if own >= glass {
        (w, 1000, (glass / own * 1000.0) as usize)
    } else {
        (w, (own / glass * 1000.0) as usize, 1000)
    }
}

impl Doom {
    pub fn new(panel: &Panel, _fps: u32) -> Self {
        let wad = env_str(&["DOOM_WAD"], "/freedoom1.wad");
        let map_secs = env_num(&["DOOM_MAP_SECS"], 180, 0, 86_400) as u64;
        let gamma = env_num(&["DOOM_GAMMA"], 2, 0, 4) as u32;
        let light = env_num(&["DOOM_LIGHT"], 1, 0, 2) as i32;
        let fov = env_num(&["DOOM_FOV"], 0, 0, 170) as i32;
        let pct = env_num(&["DOOM_WIDTH_PCT"], 100, 0, 100) as i32;
        let hud = env_num(&["DOOM_HUD"], 0, 0, 1) as i32;
        let skill = env_num(&["DOOM_SKILL"], 3, 1, 5) as i32;
        let god = env_num(&["DOOM_GOD"], 1, 0, 1) as i32;
        let seed = saver_seed(&["DOOM_SEED"], 1);
        let knobs = Knobs {
            map_secs,
            gamma,
            light,
            fov: if fov == 0 { 0 } else { fov.max(60) },
            pct: if pct == 0 { 100 } else { pct.max(10) },
            hud,
            skill,
            god,
            seed,
        };
        Self::build(panel, &wad, &knobs)
    }

    fn build(panel: &Panel, wad: &str, k: &Knobs) -> Self {
        let aspect = pixel_aspect();
        let (width, wide, tall) = layout(panel, aspect);
        // A cell per Doom pixel across at most: a smaller cell would only
        // repeat pixels.
        let cell_w = (panel.w / width).max(1);
        let cell_h = (panel.h * 100 / aspect / H).max(1);
        let grid = Grid::new(panel, cell_w, cell_h);
        let (cols, rows) = (grid.cols(), grid.rows());
        let (fw, fh) = (cols * wide / 1000, rows * tall / 1000);
        let (x0, y0) = ((cols - fw) / 2, (rows - fh) / 2);
        let (pw, ph) = (panel.w * wide / 1000, panel.h * tall / 1000);
        let (px0, py0) = ((panel.w - pw) / 2, (panel.h - ph) / 2);
        let y_first = (0..=H).map(|sy| (sy * ph).div_ceil(H) as u16).collect();
        let row_src = (0..rows)
            .map(|r| {
                if (y0..y0 + fh).contains(&r) {
                    ((r - y0) * H / fh.max(1)) as u16
                } else {
                    OFF
                }
            })
            .collect();

        let playpal = std::fs::read(wad)
            .ok()
            .and_then(|w| lump(&w, b"PLAYPAL").map(<[u8]>::to_vec));
        if playpal.is_none() {
            eprintln!("[screensaver] doom: no PLAYPAL in {wad:?}; showing static");
        }
        let want = playpal.as_ref().and_then(|_| {
            Some(Want {
                seed: k.seed,
                wad: CString::new(wad).ok()?,
                width,
                view_pct: k.pct,
                fov: k.fov,
                hud: k.hud,
                skill: k.skill,
                god: k.god,
                map_every: (k.map_secs > 0).then(|| Duration::from_secs(k.map_secs)),
                light: k.light,
            })
        });
        let mut d = Self {
            grid,
            palette: palette(playpal.as_deref(), k.gamma),
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            want,
            engaged: false,
            rect: (x0, x0 + fw, y0, y0 + fh),
            px_rect: (px0, px0 + pw, py0, py0 + ph),
            panel: (panel.w, panel.h),
            x_src: vec![0; pw],
            x_first: Vec::with_capacity(MAX_W + 1),
            y_first,
            drawn: vec![0; MAX_W * H],
            drawn_key: None,
            line: vec![0; pw],
            col_src: vec![OFF; cols],
            col_w: 0,
            row_src,
            pix: vec![0; MAX_W * H],
            seq: 0,
            shown: None,
            fresh: false,
            first: true,
            rng: k.seed,
        };
        d.columns_for(width);
        d
    }

    /// Point the frame's columns, cells and panel pixels, at a source `w`
    /// pixels wide.
    fn columns_for(&mut self, w: usize) {
        let (x0, x1, ..) = self.rect;
        let span = (x1 - x0).max(1);
        for (c, src) in self.col_src.iter_mut().enumerate() {
            *src = if (x0..x1).contains(&c) {
                ((c - x0) * w / span) as u16
            } else {
                OFF
            };
        }
        let pw = self.x_src.len();
        for (x, src) in self.x_src.iter_mut().enumerate() {
            *src = (x * w / pw) as u16;
        }
        // First x with x * w / pw >= sx.
        self.x_first.clear();
        self.x_first
            .extend((0..=w).map(|sx| (sx * pw).div_ceil(w.max(1)) as u16));
        self.col_w = w;
    }

    /// Scale the source rows that differ from what the panel shows into it,
    /// each over just the span of columns that changed. `base` offsets the
    /// indices into the palette.
    fn blit(&mut self, s: &mut Surface<'_>, w: usize, base: usize) {
        let full = self.drawn_key != Some((w, base));
        let (px0, _, py0, _) = self.px_rect;
        for sy in 0..H {
            let (ya, yb) = (self.y_first[sy] as usize, self.y_first[sy + 1] as usize);
            let row = &self.pix[sy * w..][..w];
            let old = &mut self.drawn[sy * w..][..w];
            let (lo, hi) = if full {
                (0, w)
            } else if row == old || ya == yb {
                continue;
            } else {
                let diff = |(a, b): (&u8, &u8)| a != b;
                let z = || row.iter().zip(old.iter());
                let lo = z().position(diff).unwrap_or(0);
                let hi = w - z().rev().position(diff).unwrap_or(0);
                (lo, hi)
            };
            old[lo..hi].copy_from_slice(&row[lo..hi]);
            let (xa, xb) = (self.x_first[lo] as usize, self.x_first[hi] as usize);
            let line = &mut self.line[xa..xb];
            for (out, &sx) in line.iter_mut().zip(&self.x_src[xa..xb]) {
                *out = self.palette[base + row[sx as usize] as usize];
            }
            for out in s.cell_rows(px0 + xa, py0 + ya, xb - xa, yb - ya) {
                out.copy_from_slice(line);
            }
        }
        self.drawn_key = Some((w, base));
    }

    /// The panel outside the frame, black, once.
    fn margins(&self, s: &mut Surface<'_>) {
        let (x0, x1, y0, y1) = self.px_rect;
        let (pw, ph) = self.panel;
        for (x, y, w, h) in [
            (0, 0, pw, y0),
            (0, y1, pw, ph - y1),
            (0, y0, x0, y1 - y0),
            (x1, y0, pw - x1, y1 - y0),
        ] {
            for row in s.cell_rows(x, y, w, h) {
                row.fill(0);
            }
        }
    }
}

struct Knobs {
    map_secs: u64,
    gamma: u32,
    light: i32,
    fov: i32,
    pct: i32,
    hud: i32,
    skill: i32,
    god: i32,
    seed: u32,
}

/// A WAD lump by name, or None for a file that is not a WAD or lacks it.
fn lump<'a>(wad: &'a [u8], name: &[u8]) -> Option<&'a [u8]> {
    let word = |at: usize| Some(u32::from_le_bytes(wad.get(at..at + 4)?.try_into().ok()?) as usize);
    if !matches!(wad.get(..4), Some(b"IWAD" | b"PWAD")) {
        return None;
    }
    let (count, dir) = (word(4)?, word(8)?);
    (0..count).find_map(|i| {
        let e = dir + i * 16;
        let raw = wad.get(e + 8..e + 16)?;
        let n = raw.iter().position(|&b| b == 0).unwrap_or(8);
        (&raw[..n] == name).then(|| wad.get(word(e)?..word(e)? + word(e + 4)?))?
    })
}

/// All fourteen PLAYPAL palettes through a gamma lift, then the grey ramp
/// static is drawn in. Without a WAD, the ramp alone.
fn palette(playpal: Option<&[u8]>, gamma: u32) -> Vec<u32> {
    let exp = 1.0 - gamma as f32 * 0.125;
    let lift = |c: u8| ((c as f32 / 255.0).powf(exp) * 255.0 + 0.5) as u32;
    let mut pal = vec![0; STATIC + STATIC_LEVELS];
    if let Some(p) = playpal {
        for (i, rgb) in p.as_chunks::<3>().0.iter().take(STATIC).enumerate() {
            pal[i] = lift(rgb[0]) << 16 | lift(rgb[1]) << 8 | lift(rgb[2]);
        }
    }
    for (i, c) in pal[STATIC..].iter_mut().enumerate() {
        let v = (i * 255 / (STATIC_LEVELS - 1)) as u32;
        *c = v << 16 | v << 8 | v;
    }
    pal
}

impl Saver for Doom {
    fn render(&mut self, s: &mut Surface<'_>) {
        if let Some(w) = self.want.take() {
            Engine::get().claim(self.id, w);
            self.engaged = true;
        }
        if self.engaged {
            if let Some((seq, shown)) = Engine::get().latest(self.seq, &mut self.pix) {
                (self.seq, self.shown, self.fresh) = (seq, shown, true);
            }
        }
        let (w, base) = if let Some((w, pal)) = self.shown {
            (w, pal as usize * 256)
        } else {
            let w = self.col_w;
            for p in &mut self.pix[..w * H] {
                *p = (next_rand(&mut self.rng) as usize % STATIC_LEVELS) as u8;
            }
            (w, STATIC)
        };
        // The engine runs at 35 Hz and the panel at its own rate: a frame
        // already drawn is no work at all.
        if !(self.fresh || self.first) && self.shown.is_some() {
            return;
        }
        self.fresh = false;
        if w != self.col_w {
            self.columns_for(w);
        }
        self.blit(s, w, base);
        if self.first {
            self.margins(s);
            self.first = false;
        }
        let (x0, x1, ..) = self.rect;
        let (pix, cols, row_src) = (&self.pix, &self.col_src[x0..x1], &self.row_src);
        self.grid.fill_rows(|cy, row| {
            let sy = row_src[cy];
            if sy == OFF {
                return row.fill(Cell::CLEAR);
            }
            let src = &pix[sy as usize * w..][..w];
            row[..x0].fill(Cell::CLEAR);
            row[x1..].fill(Cell::CLEAR);
            for (c, &sx) in row[x0..x1].iter_mut().zip(cols) {
                *c = Cell::new(font::SOLID, (base + src[sx as usize] as usize) as u16);
            }
        });
        self.grid.settle();
    }

    fn name(&self) -> &'static str {
        "doom"
    }

    fn grid(&self) -> &Grid {
        &self.grid
    }

    fn palette(&self) -> &[u32] {
        &self.palette
    }
}

impl Drop for Doom {
    fn drop(&mut self) {
        if self.engaged {
            Engine::get().release(self.id);
        }
    }
}

#[cfg(test)]
mod tests;
