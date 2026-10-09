//! `gameboy`: a Game Boy (and Game Boy Color) emulator playing free homebrew
//! on autopilot, or a ROM the deployment mounts — with a bot for Pokémon
//! Red, Blue and Yellow.
//!
//! The emulator is mizu-core (MIT), vendored in `vendor/mizu-core`, so this
//! saver is in the default image. It runs on its own thread (`engine.rs`)
//! and composes a view as wide as the panel's glass needs at the screen's
//! full height (`view.rs`); this saver scales the last finished view onto
//! the panel the way `doom` does: straight into the panel's pixels, only
//! the rows that changed, each over the columns that changed. The grid is
//! for the mirror and the terminal, a solid cell per view pixel in RGB444.

mod carts;
mod engine;
mod kanto;
mod pilot;
mod pokemon;
mod view;
mod world;

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::font;
use crate::grid::{pixel_aspect, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, env_str, next_rand, saver_seed};

use engine::{Engine, Want, H, MAX_W, SCREEN_W};
use view::{Mode, DIM};

/// The grid palette: RGB444, then the same dimmed.
const GRID_COLOURS: usize = 2 * 4096;
const STATIC_LEVELS: u32 = 16;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// The saver's hold on the engine. Taken by the first `render`, not the
/// constructor: the mirror builds savers just to read their knobs, and that
/// must not steal the engine.
enum Claim {
    Pending(Want),
    Held,
    /// Never claims: a test drives the view itself.
    #[cfg(test)]
    Off,
}

pub struct GameBoySaver {
    grid: Grid,
    grid_palette: Vec<u32>,
    /// A view pixel (RGB555, bit 15 dim) to the panel's XRGB8888.
    lut: Vec<u32>,
    id: u64,
    claim: Claim,
    w: usize,
    /// The panel pixels the view covers, `x0..x1` by `y0..y1`; the rest is
    /// margin, and so are the cells outside `rect`.
    px_rect: (usize, usize, usize, usize),
    rect: (usize, usize, usize, usize),
    panel: (usize, usize),
    x_src: Vec<u16>,
    x_first: Vec<u16>,
    y_first: Vec<u16>,
    col_src: Vec<u16>,
    row_src: Vec<u16>,
    pix: Vec<u16>,
    drawn: Vec<u16>,
    line: Vec<u32>,
    seq: u32,
    shown: bool,
    fresh: bool,
    first: bool,
    rng: u32,
}

/// The view's width for a panel, in Game Boy pixels at the screen's full
/// height, and the share of the panel's width and height (per mille) it
/// fills. Glass narrower than the screen's 10:9 shows it letterboxed.
fn layout(panel: &Panel, aspect: usize) -> (usize, usize, usize) {
    let glass = panel.w as f32 * aspect as f32 / (100.0 * panel.h as f32);
    let w = ((H as f32 * glass).round() as usize).clamp(SCREEN_W, MAX_W);
    let own = w as f32 / H as f32;
    if own >= glass {
        (w, 1000, (glass / own * 1000.0) as usize)
    } else {
        (w, (own / glass * 1000.0) as usize, 1000)
    }
}

fn expand(c5: u32) -> u32 {
    c5 << 3 | c5 >> 2
}

/// RGB555 to XRGB8888, at full strength or dimmed to the glow's level.
fn xrgb(c: u16) -> u32 {
    let dim = c & DIM != 0;
    let k = if dim { 85 } else { 256 };
    let ch = |s: u32| (expand(u32::from(c) >> s & 31) * k) >> 8;
    ch(0) << 16 | ch(5) << 8 | ch(10)
}

/// RGB555 (bit 15 dim) to a grid colour index.
fn grid_colour(c: u16) -> u16 {
    let (r, g, b) = (c >> 1 & 15, c >> 6 & 15, c >> 11 & 15);
    (r << 8 | g << 4 | b) + if c & DIM != 0 { 4096 } else { 0 }
}

impl GameBoySaver {
    pub fn new(panel: &Panel, _fps: u32) -> Self {
        let rom = env_str(&["GAMEBOY_ROM"], "");
        let sav = env_str(&["GAMEBOY_SAV"], "");
        let wide = env_num(&["GAMEBOY_WIDE"], 1, 0, 1) == 1;
        let palette = env_str(&["GAMEBOY_PALETTE"], "auto");
        let rotate = env_num(&["GAMEBOY_ROTATE_SECS"], 600, 0, 86_400) as u64;
        let seed = saver_seed(&["GAMEBOY_SEED"], 1);
        let aspect = pixel_aspect();
        let (width, ..) = layout(panel, aspect);
        let want = Want {
            rom,
            sav,
            palette,
            width,
            mode: if wide { Mode::Wide } else { Mode::Frame },
            rotate: (rotate > 0).then(|| Duration::from_secs(rotate)),
            seed,
            restart: crate::saver::restarts(),
        };
        Self::build(panel, aspect, want)
    }

    fn build(panel: &Panel, aspect: usize, want: Want) -> Self {
        let (w, wide, tall) = layout(panel, aspect);
        let cell_w = (panel.w / w).max(1);
        let cell_h = (panel.h * 100 / aspect / H).max(1);
        let grid = Grid::new(panel, cell_w, cell_h);
        let (cols, rows) = (grid.cols(), grid.rows());
        let (fw, fh) = (cols * wide / 1000, rows * tall / 1000);
        let (x0, y0) = ((cols - fw) / 2, (rows - fh) / 2);
        let (pw, ph) = (panel.w * wide / 1000, panel.h * tall / 1000);
        let (px0, py0) = ((panel.w - pw) / 2, (panel.h - ph) / 2);
        let span = |n: usize, of: usize, i: usize| (i * of / n.max(1)) as u16;
        let grid_palette = (0..GRID_COLOURS)
            .map(|i| {
                let (dim, c) = (i >= 4096, i % 4096);
                let k = if dim { 85 } else { 255 };
                let ch = |s: usize| ((c >> s & 15) as u32 * 17 * k) / 255;
                ch(8) << 16 | ch(4) << 8 | ch(0)
            })
            .collect();
        Self {
            grid,
            grid_palette,
            lut: (0..=u16::MAX).map(xrgb).collect(),
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            claim: Claim::Pending(want),
            w,
            px_rect: (px0, px0 + pw, py0, py0 + ph),
            rect: (x0, x0 + fw, y0, y0 + fh),
            panel: (panel.w, panel.h),
            x_src: (0..pw).map(|x| span(pw, w, x)).collect(),
            x_first: (0..=w).map(|sx| (sx * pw).div_ceil(w) as u16).collect(),
            y_first: (0..=H).map(|sy| (sy * ph).div_ceil(H) as u16).collect(),
            col_src: (0..cols)
                .map(|c| {
                    if (x0..x0 + fw).contains(&c) {
                        span(fw, w, c - x0)
                    } else {
                        u16::MAX
                    }
                })
                .collect(),
            row_src: (0..rows)
                .map(|r| {
                    if (y0..y0 + fh).contains(&r) {
                        span(fh, H, r - y0)
                    } else {
                        u16::MAX
                    }
                })
                .collect(),
            pix: vec![0; MAX_W * H],
            drawn: vec![0; MAX_W * H],
            line: vec![0; pw],
            seq: 0,
            shown: false,
            fresh: false,
            first: true,
            rng: 0x2545_F491,
        }
    }

    /// Scale the view rows that differ from what the panel shows into it,
    /// each over just the columns that changed.
    fn blit(&mut self, s: &mut Surface<'_>) {
        let w = self.w;
        let (px0, _, py0, _) = self.px_rect;
        for sy in 0..H {
            let (ya, yb) = (self.y_first[sy] as usize, self.y_first[sy + 1] as usize);
            let row = &self.pix[sy * w..][..w];
            let old = &mut self.drawn[sy * w..][..w];
            let (lo, hi) = if self.first {
                (0, w)
            } else if row == old || ya == yb {
                continue;
            } else {
                let diff = |(a, b): (&u16, &u16)| a != b;
                let z = || row.iter().zip(old.iter());
                (
                    z().position(diff).unwrap_or(0),
                    w - z().rev().position(diff).unwrap_or(0),
                )
            };
            old[lo..hi].copy_from_slice(&row[lo..hi]);
            let (xa, xb) = (self.x_first[lo] as usize, self.x_first[hi] as usize);
            for (out, &sx) in self.line[xa..xb].iter_mut().zip(&self.x_src[xa..xb]) {
                *out = self.lut[row[sx as usize] as usize];
            }
            let line = &self.line[xa..xb];
            for out in s.cell_rows(px0 + xa, py0 + ya, xb - xa, yb - ya) {
                out.copy_from_slice(line);
            }
        }
    }

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

impl Saver for GameBoySaver {
    fn render(&mut self, s: &mut Surface<'_>) {
        if matches!(self.claim, Claim::Pending(_)) {
            if let Claim::Pending(w) = std::mem::replace(&mut self.claim, Claim::Held) {
                Engine::get().claim(self.id, w);
            }
        }
        if matches!(self.claim, Claim::Held) {
            if let Some((seq, w)) = Engine::get().latest(self.seq, &mut self.pix) {
                self.seq = seq;
                self.fresh = true;
                // A view the engine composed for an older width (a knob
                // change in flight) would scale wrong; static until it
                // catches up.
                self.shown = w == Some(self.w);
            }
        }
        if !self.shown {
            for p in &mut self.pix[..self.w * H] {
                let v =
                    (next_rand(&mut self.rng) % STATIC_LEVELS * 31 / (STATIC_LEVELS - 1)) as u16;
                *p = v | v << 5 | v << 10;
            }
            self.fresh = true;
        }
        if !(self.fresh || self.first) {
            return;
        }
        self.fresh = false;
        self.blit(s);
        if self.first {
            self.margins(s);
            self.first = false;
        }
        let (x0, x1, ..) = self.rect;
        let w = self.w;
        let (pix, cols, row_src) = (&self.pix, &self.col_src[x0..x1], &self.row_src);
        self.grid.fill_rows(|cy, row| {
            let sy = row_src[cy];
            if sy == u16::MAX {
                return row.fill(Cell::CLEAR);
            }
            let src = &pix[sy as usize * w..][..w];
            row[..x0].fill(Cell::CLEAR);
            row[x1..].fill(Cell::CLEAR);
            for (c, &sx) in row[x0..x1].iter_mut().zip(cols) {
                *c = Cell::new(font::SOLID, grid_colour(src[sx as usize]));
            }
        });
        self.grid.settle();
    }

    fn name(&self) -> &'static str {
        "gameboy"
    }

    fn grid(&self) -> &Grid {
        &self.grid
    }

    fn palette(&self) -> &[u32] {
        &self.grid_palette
    }
}

impl Drop for GameBoySaver {
    fn drop(&mut self) {
        if matches!(self.claim, Claim::Held) {
            Engine::get().release(self.id);
        }
    }
}

#[cfg(test)]
mod tests;
