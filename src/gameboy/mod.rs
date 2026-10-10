//! `gameboy`: a Game Boy (and Game Boy Color) emulator playing free homebrew
//! on autopilot, or a ROM the deployment mounts — with a bot for Pokémon
//! Red, Blue and Yellow.
//!
//! The emulator is mizu-core (MIT), vendored in `vendor/mizu-core`, so this
//! saver is in the default image. It runs on its own thread (`engine.rs`)
//! and composes a view as wide as the panel's glass needs at the screen's
//! full height (`view.rs`); this saver scales the last finished view onto
//! the panel as `doom` does (`scaled.rs`). The grid is for the mirror and the
//! terminal, a solid cell per view pixel in RGB444.

mod carts;
mod engine;
mod kanto;
mod pilot;
mod pokemon;
mod ram;
mod view;
mod world;

use std::time::Duration;

use crate::engine_slot::Claim;
use crate::grid::{pixel_aspect, Grid};
use crate::saver::Saver;
use crate::scaled::{self, Scaled};
use crate::surface::{Panel, Surface};
use crate::{env_num, env_str, next_rand, saver_seed};

use engine::{Engine, Want, H, MAX_W, SCREEN_W};
use pokemon::Knobs;
use view::{Mode, DIM};

/// The grid palette: RGB444, then the same dimmed.
const GRID_COLOURS: usize = 2 * 4096;
const STATIC_LEVELS: u32 = 16;

pub struct GameBoySaver {
    grid: Grid,
    grid_palette: Vec<u32>,
    /// A view pixel (RGB555, bit 15 dim) to the panel's XRGB8888.
    lut: Vec<u32>,
    claim: Claim<Engine>,
    view: Scaled<u16>,
    pix: Vec<u16>,
    seq: u32,
    /// `pix` is the engine's view, not static.
    shown: bool,
    /// `pix` holds a view the panel has not shown yet.
    fresh: bool,
    rng: u32,
}

/// The view's width for a panel, in Game Boy pixels at the screen's full
/// height, and the share of the panel it fills. Glass narrower than the
/// screen's 10:9 shows it letterboxed.
fn layout(panel: &Panel, aspect: usize) -> (usize, usize, usize) {
    scaled::layout(panel, aspect, H as f32, SCREEN_W, MAX_W)
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
        let pokemon = Knobs {
            text_ms: env_num(&["POKEMON_TEXT_MS"], 200, 0, 2000) as u32,
            starter: env_num(&["POKEMON_STARTER"], 0, 0, 3) as u32,
        };
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
            restart: crate::saver::restarts("gameboy"),
            pokemon,
        };
        Self::build(panel, aspect, want)
    }

    fn build(panel: &Panel, aspect: usize, want: Want) -> Self {
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
            claim: Claim::new(Some(want)),
            view,
            pix: vec![0; MAX_W * H],
            seq: 0,
            shown: false,
            fresh: false,
            rng: 0x2545_F491,
        }
    }
}

impl Saver for GameBoySaver {
    fn render(&mut self, s: &mut Surface<'_>) {
        let w = self.view.width();
        if let Some(e) = self.claim.engine(Engine::get) {
            if let Some((seq, shown)) = e.latest(self.seq, &mut self.pix) {
                // A view the engine composed for an older width (a knob
                // change in flight) would scale wrong; static until it
                // catches up.
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
        "gameboy"
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
