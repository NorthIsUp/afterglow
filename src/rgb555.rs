//! The render side of an emulator saver whose engine thread publishes an
//! RGB555 view (bit 15 marking a pixel dimmed for the glow): the view copied
//! out when it moves on, scaled onto the panel (`scaled.rs`), TV static while
//! there is nothing to show, and the grid for the mirror and the terminal, a
//! solid cell per view pixel in RGB444.

use crate::grid::Grid;
use crate::next_rand;
use crate::scaled::Scaled;
use crate::surface::{Panel, Surface};

pub const DIM: u16 = 0x8000;

/// The grid palette: RGB444, then the same dimmed.
const GRID_COLOURS: usize = 2 * 4096;
const STATIC_LEVELS: u32 = 16;

pub struct Screen {
    grid: Grid,
    grid_palette: Vec<u32>,
    /// A view pixel to the panel's XRGB8888.
    lut: Vec<u32>,
    view: Scaled<u16>,
    h: usize,
    pub pix: Vec<u16>,
    seq: u32,
    /// `pix` is the engine's view (or the saver's own picture), not static.
    pub shown: bool,
    /// `pix` holds a view the panel has not shown yet.
    pub fresh: bool,
    rng: u32,
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

fn grid_colour(c: u16) -> u16 {
    let (r, g, b) = (c >> 1 & 15, c >> 6 & 15, c >> 11 & 15);
    (r << 8 | g << 4 | b) + if c & DIM != 0 { 4096 } else { 0 }
}

impl Screen {
    /// A view `h` high and at most `max_w` wide, at `layout`'s width.
    pub fn new(
        panel: &Panel,
        aspect: usize,
        layout: (usize, usize, usize),
        h: usize,
        max_w: usize,
    ) -> Self {
        let (grid, view) = Scaled::new(panel, aspect, layout, h, max_w);
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
            view,
            h,
            pix: vec![0; max_w * h],
            seq: 0,
            shown: false,
            fresh: true,
            rng: 0x2545_F491,
        }
    }

    pub fn width(&self) -> usize {
        self.view.width()
    }

    /// `latest` copies the engine's view into its buffer if it moved on
    /// since the sequence number given, returning the new one and the width
    /// the view was composed for (`None`: nothing to show).
    pub fn render(
        &mut self,
        s: &mut Surface<'_>,
        latest: impl FnOnce(u32, &mut [u16]) -> Option<(u32, Option<usize>)>,
    ) {
        let w = self.view.width();
        if let Some((seq, shown)) = latest(self.seq, &mut self.pix) {
            // A view composed for an older width (a knob change in flight)
            // would scale wrong; static until it catches up.
            (self.seq, self.shown, self.fresh) = (seq, shown == Some(w), true);
        }
        if !self.shown {
            for p in &mut self.pix[..w * self.h] {
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

    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    pub fn palette(&self) -> &[u32] {
        &self.grid_palette
    }
}
