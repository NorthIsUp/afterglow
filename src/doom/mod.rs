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
use std::time::Duration;

use crate::engine_slot::Claim;
use crate::grid::{pixel_aspect, Grid};
use crate::saver::Saver;
use crate::scaled::{self, Scaled};
use crate::surface::{Panel, Surface};
use crate::{env_num, env_str, next_rand, saver_seed};

use engine::{Engine, Want, H, MAX_W};

const PALETTES: usize = 14;
const STATIC: usize = PALETTES * 256;
const STATIC_LEVELS: usize = 16;
/// Doom's 320x200 fills 4:3 glass, so a Doom pixel is 1.2 times taller than
/// wide: a screen `r` times as wide as tall is `240 * r` pixels across.
const PX_PER_RATIO: f32 = 240.0;
const MIN_W: usize = 320;

pub struct Doom {
    grid: Grid,
    palette: Vec<u32>,
    claim: Claim<Engine>,
    view: Scaled<u8>,
    /// The width and palette offset the panel was last drawn at: a change of
    /// either redraws every row.
    drawn_key: Option<(usize, usize)>,
    pix: Vec<u8>,
    seq: u32,
    /// The frame's width and palette; `None` draws static.
    shown: Option<(usize, u8)>,
    /// `pix` holds a frame the panel has not shown yet.
    fresh: bool,
    rng: u32,
}

/// The Doom screen width for a panel, and the share of the panel it fills.
fn layout(panel: &Panel, aspect: usize) -> (usize, usize, usize) {
    scaled::layout(panel, aspect, PX_PER_RATIO, MIN_W, MAX_W)
}

impl Doom {
    pub fn new(panel: &Panel, _fps: u32) -> Self {
        let wad = env_str(&["DOOM_WAD"], "/freedoom1.wad");
        let map_secs = env_num(&["DOOM_MAP_SECS"], 300, 0, 86_400) as u64;
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
        let (grid, view) = Scaled::new(panel, aspect, layout(panel, aspect), H, MAX_W);
        let width = view.width();

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
        Self {
            grid,
            palette: palette(playpal.as_deref(), k.gamma),
            claim: Claim::new(want),
            view,
            drawn_key: None,
            pix: vec![0; MAX_W * H],
            seq: 0,
            shown: None,
            fresh: false,
            rng: k.seed,
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
        if let Some(e) = self.claim.engine(Engine::get) {
            if let Some((seq, shown)) = e.latest(self.seq, &mut self.pix) {
                (self.seq, self.shown, self.fresh) = (seq, shown, true);
            }
        }
        let (w, base) = if let Some((w, pal)) = self.shown {
            (w, pal as usize * 256)
        } else {
            let w = self.view.width();
            for p in &mut self.pix[..w * H] {
                *p = (next_rand(&mut self.rng) as usize % STATIC_LEVELS) as u8;
            }
            (w, STATIC)
        };
        // The engine runs at 35 Hz and the panel at its own rate: a frame
        // already drawn is no work at all.
        if !self.fresh && self.drawn_key.is_some() && self.shown.is_some() {
            return;
        }
        self.fresh = false;
        if w != self.view.width() {
            self.view.set_width(w);
        }
        let full = self.drawn_key != Some((w, base));
        self.drawn_key = Some((w, base));
        let palette = &self.palette;
        self.view.draw(
            s,
            &mut self.grid,
            &self.pix,
            full,
            |p| palette[base + p as usize],
            |p| (base + p as usize) as u16,
        );
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

#[cfg(test)]
mod tests;
