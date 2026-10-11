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
pub fn xrgb(c: u16) -> u32 {
    let k = if c & DIM != 0 { 85 } else { 256 };
    let ch = |s: u32| {
        let c5 = u32::from(c) >> s & 31;
        ((c5 << 3 | c5 >> 2) * k) >> 8
    };
    ch(0) << 16 | ch(5) << 8 | ch(10)
}

/// Columns of the screen's edge the glow averages.
const GLOW_COLS: usize = 8;

/// Each side of the `screen_w`-wide screen at `x0` in a `w`-wide view, in
/// the average colour of the screen's edge beside it, blurred over `rows`
/// rows either way and dimmed: one row's edge pixel alone stripes the
/// sides with every brick and letter.
pub fn glow<const H: usize>(out: &mut [u16], w: usize, x0: usize, screen_w: usize, rows: usize) {
    if x0 == 0 {
        return;
    }
    let sum = |row: &[u16]| {
        row.iter().fold([0u32; 3], |a, &c| {
            [
                a[0] + u32::from(c & 31),
                a[1] + u32::from(c >> 5 & 31),
                a[2] + u32::from(c >> 10 & 31),
            ]
        })
    };
    let mut edges = [[[0u32; 3]; 2]; H];
    for (y, e) in edges.iter_mut().enumerate() {
        let row = &out[y * w..][..w];
        *e = [
            sum(&row[x0..x0 + GLOW_COLS]),
            sum(&row[x0 + screen_w - GLOW_COLS..x0 + screen_w]),
        ];
    }
    for y in 0..H {
        let (a, b) = (y.saturating_sub(rows), (y + rows + 1).min(H));
        let mut acc = [[0u32; 3]; 2];
        for e in &edges[a..b] {
            for side in 0..2 {
                for ch in 0..3 {
                    acc[side][ch] += e[side][ch];
                }
            }
        }
        let n = ((b - a) * GLOW_COLS) as u32;
        let px = |c: [u32; 3]| {
            (c[0] / n) as u16 | ((c[1] / n) as u16) << 5 | ((c[2] / n) as u16) << 10 | DIM
        };
        let row = &mut out[y * w..][..w];
        row[..x0].fill(px(acc[0]));
        row[x0 + screen_w..].fill(px(acc[1]));
    }
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

    #[cfg(any(test, feature = "gba"))]
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

    /// The panel pixels the view is drawn over.
    #[cfg(test)]
    pub fn px_rect(&self) -> (usize, usize, usize, usize) {
        self.view.px_rect()
    }

    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    pub fn palette(&self) -> &[u32] {
        &self.grid_palette
    }
}
