//! Doom fire: a low-res heat grid seeded white-hot along the bottom and
//! propagated upward each frame, palette-mapped onto the character grid.

use crate::env_num;
use crate::font;
use crate::grid::{bake, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};

/// 37-step Doom fire palette (RGB). 0 = cooled black, 36 = white-hot.
const PAL_RGB: [[u8; 3]; 37] = [
    [0x07, 0x07, 0x07],
    [0x1F, 0x07, 0x07],
    [0x2F, 0x0F, 0x07],
    [0x47, 0x0F, 0x07],
    [0x57, 0x17, 0x07],
    [0x67, 0x1F, 0x07],
    [0x77, 0x1F, 0x07],
    [0x8F, 0x27, 0x07],
    [0x9F, 0x2F, 0x07],
    [0xAF, 0x3F, 0x07],
    [0xBF, 0x47, 0x07],
    [0xC7, 0x47, 0x07],
    [0xDF, 0x4F, 0x07],
    [0xDF, 0x57, 0x07],
    [0xDF, 0x57, 0x07],
    [0xD7, 0x5F, 0x07],
    [0xD7, 0x5F, 0x07],
    [0xD7, 0x67, 0x0F],
    [0xCF, 0x6F, 0x0F],
    [0xCF, 0x77, 0x0F],
    [0xCF, 0x7F, 0x0F],
    [0xCF, 0x87, 0x17],
    [0xC7, 0x87, 0x17],
    [0xC7, 0x8F, 0x17],
    [0xC7, 0x97, 0x1F],
    [0xBF, 0x9F, 0x1F],
    [0xBF, 0x9F, 0x1F],
    [0xBF, 0xA7, 0x27],
    [0xBF, 0xA7, 0x27],
    [0xBF, 0xAF, 0x2F],
    [0xB7, 0xAF, 0x2F],
    [0xB7, 0xB7, 0x2F],
    [0xB7, 0xB7, 0x37],
    [0xCF, 0xCF, 0x6F],
    [0xDF, 0xDF, 0x9F],
    [0xEF, 0xEF, 0xC7],
    [0xFF, 0xFF, 0xFF],
];

const PAL: [u32; 37] = bake(&PAL_RGB);

/// Hottest heat value; also the divisor that maps heat onto the ramp.
const HOT: usize = 36;

pub struct Fire {
    cols: usize,
    rows: usize,
    heat: Vec<u8>,
    rng: u32,
    grid: Grid,
    /// true = a ramp glyph per heat; false = a solid block of the heat colour.
    ascii: bool,
    name: &'static str,
}

impl Fire {
    /// The retro glyph look: one heat sample per `FIRE_CELL`-pixel character
    /// cell, blitted as a ramp glyph coloured by the heat palette.
    pub fn ascii(panel: &Panel) -> Self {
        let cell = env_num(&["FIRE_CELL"], 16, 8, 64) as usize;
        Self::new(panel, cell, true, "ascii")
    }

    /// Chunky pixels: the same machinery with a solid glyph, which is why the
    /// old `draw_blocks` is deleted rather than moved. `FIRE_SCALE` is the cell
    /// size, so the grid is still panel width / scale as it always was.
    pub fn blocks(panel: &Panel) -> Self {
        let cell = env_num(&["FIRE_SCALE"], 4, 1, 16) as usize;
        Self::new(panel, cell, false, "blocks")
    }

    fn new(panel: &Panel, cell: usize, ascii: bool, name: &'static str) -> Self {
        let grid = Grid::new(panel, cell, cell);
        let (cols, rows) = (grid.cols(), grid.rows());
        let mut heat = vec![0u8; cols * rows];
        // White-hot source row along the bottom.
        for x in 0..cols {
            heat[(rows - 1) * cols + x] = HOT as u8;
        }
        Self {
            cols,
            rows,
            heat,
            rng: 0x9e37_79b9,
            grid,
            ascii,
            name,
        }
    }

    fn step(&mut self) {
        for x in 0..self.cols {
            for y in 1..self.rows {
                let src = y * self.cols + x;
                let v = self.heat[src];
                if v == 0 {
                    self.heat[src - self.cols] = 0;
                    continue;
                }
                self.rng = self.rng.wrapping_mul(1103515245).wrapping_add(12345);
                let rnd = (self.rng >> 16) & 3;
                let drift = (rnd & 1) as usize;
                let dst = src.saturating_sub(self.cols + drift);
                self.heat[dst] = v.saturating_sub((rnd & 1) as u8);
            }
        }
    }
}

impl Saver for Fire {
    fn render(&mut self, s: &mut Surface<'_>) {
        self.step();
        // Split borrow: `fill` takes `&mut self.grid` while the closure reads
        // the heat grid, so the two cannot both go through `self`.
        let (grid, heat, cols, ascii) = (&mut self.grid, &self.heat[..], self.cols, self.ascii);
        grid.fill(|cx, cy| {
            let h = heat[cy * cols + cx];
            let g = if ascii {
                font::RAMP[(h as usize * (font::RAMP.len() - 1)) / HOT]
            } else {
                font::SOLID
            };
            Cell::new(g, h as u16)
        });
        grid.flush(s, &PAL);
    }

    fn name(&self) -> &'static str {
        self.name
    }
}
