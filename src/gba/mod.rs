//! `gba`: a Game Boy Advance emulator playing the deployment's own ROMs on
//! autopilot, and getting Pokémon FireRed, LeafGreen and Emerald through
//! their intro to the overworld.
//!
//! The emulator is mGBA (MPL-2.0), vendored in `gba/mgba` and built as C by
//! the `gba` feature. It runs on its own thread (`engine.rs`) with mGBA's HLE
//! BIOS unless `GBA_BIOS` names a real one; nothing Nintendo ships with this
//! code. No ROM is bundled either: without one the saver shows a card saying
//! where to put them. The last finished view reaches the panel through
//! `rgb555.rs`.

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
use crate::rgb555::Screen;
use crate::saver::Saver;
use crate::scaled;
use crate::surface::{Panel, Surface};
use crate::{env_num, env_str, saver_seed};

use agb::{H, W};
use engine::{Engine, Want, MAX_W};

pub struct Gba {
    screen: Screen,
    claim: Claim<Engine>,
}

/// The view's width for a panel, in GBA pixels at the screen's full height,
/// and the share of the panel it fills. Glass narrower than 3:2 letterboxes.
fn layout(panel: &Panel, aspect: usize) -> (usize, usize, usize) {
    scaled::layout(panel, aspect, H as f32, W, MAX_W)
}

impl Gba {
    pub fn new(panel: &Panel, _fps: u32) -> Self {
        let rotate = env_num(&["GBA_ROTATE_SECS"], 900, 0, 86_400) as u64;
        let aspect = pixel_aspect();
        let want = Want {
            rom: env_str(&["GBA_ROM"], "/roms/gba"),
            bios: env_str(&["GBA_BIOS"], ""),
            width: layout(panel, aspect).0,
            fps: env_num(&["GBA_FPS"], 60, 1, 60) as u32,
            rotate: (rotate > 0).then(|| Duration::from_secs(rotate)),
            seed: saver_seed(&["GBA_SEED"], 1),
            restart: crate::saver::restarts("gba"),
        };
        Self::build(panel, aspect, want)
    }

    /// The saver for `want`, or the card naming its folder when that holds
    /// no ROM, in which case the engine never starts.
    fn build(panel: &Panel, aspect: usize, want: Want) -> Self {
        let mut screen = Screen::new(panel, aspect, layout(panel, aspect), H, MAX_W);
        let want = if carts::files(&want.rom).is_empty() {
            let lines = ["No GBA ROMs found.", "", "Put .gba files in", &want.rom];
            let w = screen.width();
            card(&lines, &mut screen.pix, w);
            screen.shown = true;
            None
        } else {
            Some(want)
        };
        Self {
            screen,
            claim: Claim::new(want),
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
        let claim = &mut self.claim;
        self.screen
            .render(s, |seen, dst| claim.engine(Engine::get)?.latest(seen, dst));
    }

    fn name(&self) -> &'static str {
        "gba"
    }

    fn grid(&self) -> &Grid {
        self.screen.grid()
    }

    fn palette(&self) -> &[u32] {
        self.screen.palette()
    }
}

#[cfg(test)]
mod tests;
