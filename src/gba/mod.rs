//! `gba`: a Game Boy Advance emulator playing the deployment's own ROMs on
//! autopilot, and getting Pokémon FireRed, LeafGreen and Emerald through
//! their intro to the overworld.
//!
//! The emulator is mGBA (MPL-2.0), vendored in `gba/mgba` and built as C by
//! the `gba` feature. It runs on its own thread (`engine.rs`) with mGBA's HLE
//! BIOS unless `GBA_BIOS` names a real one; nothing Nintendo ships with this
//! code. No ROM is bundled either: without one the saver shows a card saying
//! where to put them. The last finished view is scaled onto the panel as
//! `gameboy` does (`scaled.rs`), and the grid is for the mirror and the
//! terminal, a solid cell per view pixel in RGB444.

mod agb;
mod carts;
mod engine;
mod pilot;
mod view;

use std::time::Duration;

use crate::engine_slot::Claim;
use crate::font;
use crate::glyph;
use crate::grid::{pixel_aspect, Grid};
use crate::saver::Saver;
use crate::scaled::{self, Scaled};
use crate::surface::{Panel, Surface};
use crate::{env_num, env_str, next_rand, saver_seed};

use agb::{H, W};
use engine::{Engine, Want, MAX_W};
use view::DIM;

/// The grid palette: RGB444, then the same dimmed.
const GRID_COLOURS: usize = 2 * 4096;
const STATIC_LEVELS: u32 = 16;

pub struct Gba {
    grid: Grid,
    grid_palette: Vec<u32>,
    /// A view pixel (RGB555, bit 15 dim) to the panel's XRGB8888.
    lut: Vec<u32>,
    claim: Claim<Engine>,
    view: Scaled<u16>,
    pix: Vec<u16>,
    seq: u32,
    /// `pix` is the engine's view (or the card), not static.
    shown: bool,
    /// `pix` holds a view the panel has not shown yet.
    fresh: bool,
    rng: u32,
}

/// The view's width for a panel, in GBA pixels at the screen's full height,
/// and the share of the panel it fills. Glass narrower than 3:2 letterboxes.
fn layout(panel: &Panel, aspect: usize) -> (usize, usize, usize) {
    scaled::layout(panel, aspect, H as f32, W, MAX_W)
}

/// RGB555 to XRGB8888, at full strength or dimmed to the glow's level.
fn xrgb(c: u16) -> u32 {
    let k = if c & DIM != 0 { 85 } else { 256 };
    let ch = |s: u32| {
        let c5 = u32::from(c) >> s & 31;
        ((c5 << 3 | c5 >> 2) * k) >> 8
    };
    ch(0) << 16 | ch(5) << 8 | ch(10)
}

/// RGB555 (bit 15 dim) to a grid colour index.
fn grid_colour(c: u16) -> u16 {
    let (r, g, b) = (c >> 1 & 15, c >> 6 & 15, c >> 11 & 15);
    (r << 8 | g << 4 | b) + if c & DIM != 0 { 4096 } else { 0 }
}

impl Gba {
    pub fn new(panel: &Panel, _fps: u32) -> Self {
        let rom = env_str(&["GBA_ROM"], "/roms/gba");
        let bios = env_str(&["GBA_BIOS"], "");
        let fps = env_num(&["GBA_FPS"], 60, 1, 60) as u32;
        let rotate = env_num(&["GBA_ROTATE_SECS"], 900, 0, 86_400) as u64;
        let seed = saver_seed(&["GBA_SEED"], 1);
        let aspect = pixel_aspect();
        let (width, ..) = layout(panel, aspect);
        let want = (!carts::files(&rom).is_empty()).then(|| Want {
            rom: rom.clone(),
            bios,
            width,
            fps,
            rotate: (rotate > 0).then(|| Duration::from_secs(rotate)),
            seed,
            restart: crate::saver::restarts("gba"),
        });
        let mut s = Self::build(panel, aspect, want);
        if s.claim.want.is_none() {
            let lines = ["No GBA ROMs found.", "", "Put .gba files in", &rom];
            card(&lines, &mut s.pix, s.view.width());
            s.shown = true;
        }
        s
    }

    fn build(panel: &Panel, aspect: usize, want: Option<Want>) -> Self {
        let (grid, view) = Scaled::new(panel, aspect, layout(panel, aspect), H, MAX_W);
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
            claim: Claim::new(want),
            view,
            pix: vec![0; MAX_W * H],
            seq: 0,
            shown: false,
            fresh: true,
            rng: 0x2545_F491,
        }
    }
}

/// RGB555: a night blue, and a pale blue-white on it.
const CARD_BG: u16 = 0x1842;
const CARD_INK: u16 = 0x7F9C;

/// `lines` centred on a dark screen in the middle of a `w`-wide view, a
/// line that does not fit losing its start.
fn card(lines: &[&str], pix: &mut [u16], w: usize) {
    pix[..w * H].fill(CARD_BG);
    let x0 = (w - W) / 2;
    let cols = W / font::GLYPH_W - 2;
    let top = (H - lines.len() * font::GLYPH_H) / 2;
    for (row, line) in lines.iter().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let shown = &chars[chars.len().saturating_sub(cols)..];
        let left = x0 + (W - shown.len() * font::GLYPH_W) / 2;
        for (col, &c) in shown.iter().enumerate() {
            let c = if c == ' ' || c.is_ascii_graphic() {
                c
            } else {
                '?'
            };
            let glyph = font::GLYPHS[glyph::of(c) as usize];
            for (gy, bits) in glyph.iter().enumerate() {
                for gx in 0..font::GLYPH_W {
                    if bits << gx & 0x80 != 0 {
                        let y = top + row * font::GLYPH_H + gy;
                        pix[y * w + left + col * font::GLYPH_W + gx] = CARD_INK;
                    }
                }
            }
        }
    }
}

impl Saver for Gba {
    fn render(&mut self, s: &mut Surface<'_>) {
        let w = self.view.width();
        if let Some(e) = self.claim.engine(Engine::get) {
            if let Some((seq, shown)) = e.latest(self.seq, &mut self.pix) {
                // A view composed for an older width (a knob change in
                // flight) would scale wrong; static until it catches up.
                (self.seq, self.shown, self.fresh) = (seq, shown == Some(w), true);
            }
        }
        if !self.shown {
            for p in &mut self.pix[..w * H] {
                let v =
                    (next_rand(&mut self.rng) % STATIC_LEVELS * 31 / (STATIC_LEVELS - 1)) as u16;
                *p = v | v << 5 | v << 10;
            }
            self.fresh = true;
        }
        if !self.fresh {
            return;
        }
        self.fresh = false;
        let lut = &self.lut;
        self.view.draw(
            s,
            &mut self.grid,
            &self.pix,
            false,
            |p| lut[p as usize],
            grid_colour,
        );
    }

    fn name(&self) -> &'static str {
        "gba"
    }

    fn grid(&self) -> &Grid {
        &self.grid
    }

    fn palette(&self) -> &[u32] {
        &self.grid_palette
    }
}

#[cfg(test)]
mod tests;
