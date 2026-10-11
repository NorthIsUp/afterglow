//! `gameboy`: a Game Boy (and Game Boy Color) emulator playing free homebrew
//! on autopilot, or a ROM the deployment mounts — with a bot for Pokémon
//! Red, Blue and Yellow.
//!
//! The emulator is mizu-core (MIT), vendored in `vendor/mizu-core`, so this
//! saver is in the default image. It runs on its own thread (`engine.rs`)
//! and composes a view as wide as the panel's glass needs at the screen's
//! full height (`view.rs`); this saver scales the last finished view onto
//! the panel through `rgb555.rs`, as `gba` does.

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
use crate::rgb555::Screen;
use crate::saver::Saver;
use crate::scaled;
use crate::surface::{Panel, Surface};
use crate::{env_num, env_str, saver_seed};

use engine::{Engine, Want, H, MAX_W, SCREEN_W};
use pokemon::Knobs;
use view::Mode;

pub struct GameBoySaver {
    screen: Screen,
    claim: Claim<Engine>,
}

/// The view's width for a panel, in Game Boy pixels at the screen's full
/// height, and the share of the panel it fills. Glass narrower than the
/// screen's 10:9 shows it letterboxed.
fn layout(panel: &Panel, aspect: usize) -> (usize, usize, usize) {
    scaled::layout(panel, aspect, H as f32, SCREEN_W, MAX_W)
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
        Self {
            screen: Screen::new(panel, aspect, layout(panel, aspect), H, MAX_W),
            claim: Claim::new(Some(want)),
        }
    }
}

impl Saver for GameBoySaver {
    fn render(&mut self, s: &mut Surface<'_>) {
        let claim = &mut self.claim;
        self.screen
            .render(s, |seen, dst| claim.engine(Engine::get)?.latest(seen, dst));
    }

    fn name(&self) -> &'static str {
        "gameboy"
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
