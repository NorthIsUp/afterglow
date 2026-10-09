//! `doom`: Freedoom played by an autopilot, as many 320x200 views side by side
//! as the panel's shape fits, each on its own random map.
//!
//! Only in a `--features doom` build, which links doomgeneric and is GPL as a
//! whole; see `THIRD_PARTY.md`. The engines run on their own thread
//! (`engine.rs`); this saver only maps their last finished frames to cells.
//!
//! Every Doom pixel is one SOLID cell whose colour is `palette * 256 + index`
//! into all fourteen PLAYPAL palettes at once, so the damage and pickup tints
//! are colour indices too and the mirror's palette stays fixed for the run.

mod engine;

use std::ffi::CString;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::font;
use crate::grid::{pixel_aspect, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, env_str, next_rand, saver_seed};

use engine::{Engines, Want, H, INSTANCES, W};

const PALETTES: usize = 14;
const STATIC: usize = PALETTES * 256;
const STATIC_LEVELS: usize = 16;
const OFF: u16 = u16::MAX;

/// Each saver instance's claim on the engines. Not a pointer: a dropped
/// saver's address can be reused by the next one, and the engine thread must
/// tell a new claim from a stale release.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub struct Doom {
    grid: Grid,
    palette: Vec<u32>,
    id: u64,
    /// Taken by the first `render`, not the constructor: the mirror builds
    /// savers just to read their knobs, and that must not steal the engines.
    want: Option<Want>,
    engaged: bool,
    views: usize,
    /// Per grid column: the view it shows and the source x; `OFF` is margin.
    col_view: Vec<u16>,
    col_src: Vec<u16>,
    row_src: Vec<u16>,
    pix: Vec<u8>,
    seq: [u32; INSTANCES],
    /// The palette each view's frame shows in; `None` draws static.
    pal: [Option<u8>; INSTANCES],
    rng: u32,
}

impl Doom {
    pub fn new(panel: &Panel, _fps: u32) -> Self {
        let wad = env_str(&["DOOM_WAD"], "/freedoom1.wad");
        let auto = env_num(&["DOOM_VIEWS"], 0, 0, INSTANCES as i64) as usize;
        let map_secs = env_num(&["DOOM_MAP_SECS"], 180, 0, 86_400) as u64;
        let gamma = env_num(&["DOOM_GAMMA"], 2, 0, 4) as u32;
        let light = env_num(&["DOOM_LIGHT"], 1, 0, 2) as i32;
        let seed = saver_seed(&["DOOM_SEED"], 1);
        Self::build(panel, &wad, auto, map_secs, gamma, light, seed)
    }

    fn build(
        panel: &Panel,
        wad: &str,
        auto: usize,
        map_secs: u64,
        gamma: u32,
        light: i32,
        seed: u32,
    ) -> Self {
        let aspect = pixel_aspect();
        let views = if auto > 0 {
            auto
        } else {
            views_for(panel, aspect)
        };
        // As many cells as Doom has pixels, no more: a cell per pixel is the
        // whole picture, and a smaller cell would only repeat pixels.
        let cell_w = (panel.w / (views * W)).max(1);
        let cell_h = (panel.h * 100 / aspect / H).max(1);
        let grid = Grid::new(panel, cell_w, cell_h);
        let (cols, rows) = (grid.cols(), grid.rows());
        let per = cols / views;
        let x0 = (cols - per * views) / 2;
        let mut col_view = vec![OFF; cols];
        let mut col_src = vec![0; cols];
        for c in 0..per * views {
            col_view[x0 + c] = (c / per) as u16;
            col_src[x0 + c] = ((c % per) * W / per) as u16;
        }
        let row_src = (0..rows).map(|r| (r * H / rows) as u16).collect();

        let playpal = std::fs::read(wad)
            .ok()
            .and_then(|w| lump(&w, b"PLAYPAL").map(<[u8]>::to_vec));
        if playpal.is_none() {
            eprintln!("[screensaver] doom: no PLAYPAL in {wad:?}; showing static");
        }
        let want = playpal.as_ref().and_then(|_| {
            Some(Want {
                views,
                seed,
                wad: CString::new(wad).ok()?,
                map_every: (map_secs > 0).then(|| Duration::from_secs(map_secs)),
                light,
            })
        });
        Self {
            grid,
            palette: palette(playpal.as_deref(), gamma),
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            want,
            engaged: false,
            views,
            col_view,
            col_src,
            row_src,
            pix: vec![0; INSTANCES * W * H],
            seq: [0; INSTANCES],
            pal: [None; INSTANCES],
            rng: seed,
        }
    }
}

/// Views side by side: one per square-and-a-quarter of glass width, so 4:3
/// and narrower show one, 16:9 two and pine's 3.2:1 three.
fn views_for(panel: &Panel, aspect: usize) -> usize {
    let glass = panel.w as f32 * aspect as f32 / (100.0 * panel.h as f32);
    ((glass + 0.25) as usize).clamp(1, INSTANCES)
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
            Engines::get().claim(self.id, w);
            self.engaged = true;
        }
        let live = self.engaged;
        for v in 0..self.views {
            let dst = &mut self.pix[v * W * H..(v + 1) * W * H];
            if let Some((seq, pal)) = live
                .then(|| Engines::get().latest(v, self.seq[v], dst))
                .flatten()
            {
                self.seq[v] = seq;
                self.pal[v] = pal;
            }
        }
        let (pix, pals, rng) = (&self.pix, &self.pal, &mut self.rng);
        let (col_view, col_src, row_src) = (&self.col_view, &self.col_src, &self.row_src);
        self.grid.fill(|cx, cy| {
            let v = col_view[cx];
            if v == OFF {
                return Cell::CLEAR;
            }
            let v = v as usize;
            let colour = match pals[v] {
                None => STATIC + next_rand(rng) as usize % STATIC_LEVELS,
                Some(pal) => {
                    let at = v * W * H + row_src[cy] as usize * W + col_src[cx] as usize;
                    pal as usize * 256 + pix[at] as usize
                }
            };
            Cell::new(font::SOLID, colour as u16)
        });
        self.grid.flush(s, &self.palette);
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
            Engines::get().release(self.id);
        }
    }
}

#[cfg(test)]
mod tests;
