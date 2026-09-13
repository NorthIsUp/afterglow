//! TacTiles — After Dark's tiling module: a grid of square tiles, each carrying
//! one geometric glyph (bar, diagonal, corner, arc), with the glyph, its
//! rotation and its colour each driven by a travelling wave across the tile
//! grid. The pattern is never still and never looks rolled: a wave crossing a
//! quantisation boundary flips a whole diagonal band of tiles at once, which is
//! what reads as a tiling rearranging itself rather than as noise.
//!
//! # Three waves, not one RNG
//!
//! Shape, rotation and colour come from three sine waves with different
//! directions, wavelengths and speeds (one of them running backwards). Each is
//! quantised into its own small set. One wave would move every property in
//! lockstep and the panel would read as a single scrolling stripe; independent
//! directions make the bands cross, so the same tile does not repeat the same
//! (shape, rotation, colour) for long. Random per tile would have no bands at
//! all, which is exactly the look this module is not.
//!
//! # Why the tile glyphs are baked, not rasterised per frame
//!
//! A tile is `TACTILES_TILE` cells on a side and each cell is stamped as a
//! braille pattern — 2x4 dots — so a 6-cell tile is a 12x24 bitmap. Four shapes
//! by four rotations is sixteen bitmaps, rasterised ONCE in `new` into a table
//! of glyph indices. Per frame a cell costs four array reads and a multiply; no
//! trig, no rasterising and no allocation reach the frame loop. The trig is per
//! TILE (a few hundred a frame), not per cell.
//!
//! Cells are square in pixels and the tile is square in cells, so a tile is
//! square at any panel aspect: nothing here assumes 16:9, and 1280x400 is 27x9
//! tiles where 1920x1080 is 40x23.
//!
//! # Damage model: full repaint (Model A)
//!
//! `Grid::flush` diffs cur against prev, so damage is derived rather than
//! tracked and cannot be under-reported. The alternative — a dirty list of the
//! tiles whose band boundary moved — would have to name every CELL of every
//! such tile, and one missed cell freezes that region on the panel forever.
//!
//! Reported ROWS are the whole panel every frame and that is not a bug to fix:
//! the waves cross the entire grid, so each 48px band of scanlines has some
//! tile flipping in it and the runs merge into one. What stays small is the
//! BLIT — about 1% of cells a frame at 1080p30 — because `flush` skips every
//! unchanged cell whatever the runs say.
//!
//! # Knobs
//!
//! * `TACTILES_CELL_W` / `TACTILES_CELL_H` — cell size in px, 4..=32 (default
//!   8 each). A tile is `CELL_W * TILE` by `CELL_H * TILE` px.
//! * `TACTILES_TILE`   — tile side in CELLS, 2..=24 (default 6).
//! * `TACTILES_SPEED`  — wave travel, milli-radians per second, 10..=5000
//!   (default 700).
//! * `TACTILES_SCALE`  — wave spatial frequency, milli-radians per tile,
//!   10..=3000 (default 430).
//! * `TACTILES_STROKE` — glyph stroke width as a percent of the tile,
//!   5..=50 (default 24).

use crate::font;
use crate::grid::{bake, dot_bit, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, next_rand};

const SHAPES: usize = 4;
const ROTS: usize = 4;
const VARIANTS: usize = SHAPES * ROTS;

/// Colour steps around the wheel. 24 is fine enough that the colour wave
/// sweeping a tile reads as a hue shift rather than as a flash, and coarse
/// enough that two tiles one band apart are visibly different colours.
const PAL_N: usize = 24;

/// A pastel wheel: a full-saturation rainbow is garish at tile size, and
/// lifting the floor to 64 keeps every hue's darkest channel visible against
/// the black background instead of letting a tile lose an edge into it.
const fn wheel() -> [[u8; 3]; PAL_N] {
    let mut out = [[0u8; 3]; PAL_N];
    let mut i = 0;
    while i < PAL_N {
        // Six 256-wide ramps around the wheel.
        let t = i * 1536 / PAL_N;
        let f = (t % 256) as u32;
        let (r, g, b) = match t / 256 {
            0 => (255, f, 0),
            1 => (255 - f, 255, 0),
            2 => (0, 255, f),
            3 => (0, 255 - f, 255),
            4 => (f, 0, 255),
            _ => (255, 0, 255 - f),
        };
        out[i] = [
            (64 + r * 191 / 255) as u8,
            (64 + g * 191 / 255) as u8,
            (64 + b * 191 / 255) as u8,
        ];
        i += 1;
    }
    out
}

const PAL: [u32; PAL_N] = bake(&wheel());

/// Is the point `(x, y)` of the unit tile inside this variant's stroke? `half`
/// is half the stroke width, in the same units.
///
/// Rotation is applied to the POINT, not to the shape, so each shape is written
/// once in one orientation and the four rotations come for free. Bar and
/// diagonal are two-fold symmetric and so have two distinct orientations;
/// corner and arc have four.
fn hit(shape: usize, rot: usize, x: f32, y: f32, half: f32) -> bool {
    let (x, y) = match rot {
        0 => (x, y),
        1 => (y, 1.0 - x),
        2 => (1.0 - x, 1.0 - y),
        _ => (1.0 - y, x),
    };
    match shape {
        // A bar down the middle.
        0 => (x - 0.5).abs() <= half,
        // A band along the top-left/bottom-right diagonal. The 1/sqrt(2) turns
        // the |x-y| axis distance into a perpendicular one, so the diagonal
        // comes out the same thickness as the bar rather than 1.41x it.
        1 => (x - y).abs() * std::f32::consts::FRAC_1_SQRT_2 <= half,
        // An elbow: in from the left edge, turn at the centre, out the bottom.
        2 => {
            ((y - 0.5).abs() <= half && x <= 0.5 + half)
                || ((x - 0.5).abs() <= half && y >= 0.5 - half)
        }
        // A quarter circle about the bottom-left corner, so it meets the left
        // and bottom edges exactly where the elbow does and adjacent tiles can
        // continue each other's lines.
        _ => ((x * x + (y - 1.0) * (y - 1.0)).sqrt() - 0.5).abs() <= half,
    }
}

pub struct Tactiles {
    grid: Grid,
    /// Tile side in cells.
    tile: usize,
    tiles_x: usize,
    tiles_y: usize,
    /// `VARIANTS * tile * tile` glyph indices, baked in `new`.
    glyphs: Vec<u16>,
    /// Per tile: `variant << 8 | colour`. Rewritten every frame.
    state: Vec<u16>,
    /// cx -> its tile column, and the cell's column within that tile.
    col_tile: Vec<u16>,
    col_off: Vec<u16>,
    /// cy -> its tile row's base offset into `state`, and the cell's row within
    /// the tile already multiplied by `tile`. Two array reads instead of the
    /// two divides per cell a `cy / tile` would cost — the same trade `Grid`
    /// makes for its glyph row map.
    row_base: Vec<u32>,
    row_off: Vec<u16>,
    /// Spatial frequency per wave, radians per tile, in x and y.
    kx: [f32; 3],
    ky: [f32; 3],
    /// Radians per FRAME per wave, so the waves travel at the same speed
    /// whatever `SAVER_FPS` is.
    step: [f32; 3],
    phase: [f32; 3],
}

impl Tactiles {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        let cell_w = env_num(&["TACTILES_CELL_W"], 8, 4, 32) as usize;
        let cell_h = env_num(&["TACTILES_CELL_H"], 8, 4, 32) as usize;
        let tile = env_num(&["TACTILES_TILE"], 6, 2, 24) as usize;
        let speed = env_num(&["TACTILES_SPEED"], 700, 10, 5000) as f32 / 1000.0;
        let scale = env_num(&["TACTILES_SCALE"], 430, 10, 3000) as f32 / 1000.0;
        let half = env_num(&["TACTILES_STROKE"], 24, 5, 50) as f32 / 200.0;

        let grid = Grid::new(panel, cell_w, cell_h);
        let (cols, rows) = (grid.cols(), grid.rows());
        // Ceiling: a panel whose cell count is not a multiple of the tile gets a
        // clipped tile at the right and bottom rather than an unpainted strip.
        let tiles_x = cols.div_ceil(tile);
        let tiles_y = rows.div_ceil(tile);

        let mut glyphs = vec![0u16; VARIANTS * tile * tile];
        for v in 0..VARIANTS {
            for cy in 0..tile {
                for cx in 0..tile {
                    let mut bits = 0u8;
                    for dy in 0..4 {
                        for dx in 0..2 {
                            // Sub-cell CENTRES in unit-tile coordinates. The
                            // tile is square in pixels, so this is the shape's
                            // own geometry however non-square one dot is.
                            let x = ((cx * 2 + dx) as f32 + 0.5) / (tile * 2) as f32;
                            let y = ((cy * 4 + dy) as f32 + 0.5) / (tile * 4) as f32;
                            if hit(v / ROTS, v % ROTS, x, y, half) {
                                bits |= dot_bit(dx, dy);
                            }
                        }
                    }
                    glyphs[v * tile * tile + cy * tile + cx] = font::BRAILLE[bits as usize];
                }
            }
        }

        // Three directions that are neither axis-aligned nor multiples of each
        // other: axis-aligned waves make the bands the tile grid's own rows and
        // columns, which reads as a scrolling stripe rather than as a pattern
        // moving THROUGH the tiling.
        let dirs = [0.0f32, 2.1, 4.0];
        let freq = [1.0f32, 0.78, 1.31];
        // The middle wave runs backwards, so the shape and rotation bands sweep
        // across each other instead of travelling together.
        let rate = [1.0f32, -0.71, 0.46];
        let fps = fps.max(1) as f32;
        let mut kx = [0.0; 3];
        let mut ky = [0.0; 3];
        let mut step = [0.0; 3];
        for i in 0..3 {
            kx[i] = scale * freq[i] * dirs[i].cos();
            ky[i] = scale * freq[i] * dirs[i].sin();
            step[i] = speed * rate[i] / fps;
        }

        // A restart must not always open on the same frame of the same pattern.
        let mut rng = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() ^ d.as_secs() as u32)
            .unwrap_or(0x7AC7_11E5);
        let mut phase = [0.0f32; 3];
        for p in phase.iter_mut() {
            *p = (next_rand(&mut rng) % 6283) as f32 / 1000.0;
        }

        let mut me = Self {
            grid,
            tile,
            tiles_x,
            tiles_y,
            glyphs,
            state: vec![0; tiles_x * tiles_y],
            col_tile: (0..cols).map(|cx| (cx / tile) as u16).collect(),
            col_off: (0..cols).map(|cx| (cx % tile) as u16).collect(),
            row_base: (0..rows).map(|cy| (cy / tile * tiles_x) as u32).collect(),
            row_off: (0..rows).map(|cy| (cy % tile * tile) as u16).collect(),
            kx,
            ky,
            step,
            phase,
        };
        me.advance();
        me
    }

    /// Move the waves one frame and re-derive every tile's (shape, rotation,
    /// colour). Three sines per TILE — a few hundred a frame at 1080p, against
    /// the tens of thousands of cells they drive.
    fn advance(&mut self) {
        for (p, s) in self.phase.iter_mut().zip(self.step.iter()) {
            // Kept inside one turn: f32 loses phase resolution as the exponent
            // grows, and this runs for months.
            *p = (*p + s) % std::f32::consts::TAU;
        }
        let steps = [SHAPES, ROTS, PAL_N];
        for ty in 0..self.tiles_y {
            let fy = ty as f32;
            for tx in 0..self.tiles_x {
                let fx = tx as f32;
                let mut q = [0usize; 3];
                for i in 0..3 {
                    // sin is -1..1; fold to 0..1 before quantising, or half the
                    // buckets are unreachable.
                    let u = (self.kx[i] * fx + self.ky[i] * fy - self.phase[i]).sin() * 0.5 + 0.5;
                    q[i] = ((u * steps[i] as f32) as usize).min(steps[i] - 1);
                }
                self.state[ty * self.tiles_x + tx] = ((q[0] * ROTS + q[1]) << 8 | q[2]) as u16;
            }
        }
    }
}

impl Saver for Tactiles {
    fn render(&mut self, s: &mut Surface<'_>) {
        self.advance();
        let (grid, state, glyphs, tt) = (
            &mut self.grid,
            &self.state[..],
            &self.glyphs[..],
            self.tile * self.tile,
        );
        let (col_tile, col_off, row_base, row_off) = (
            &self.col_tile[..],
            &self.col_off[..],
            &self.row_base[..],
            &self.row_off[..],
        );
        grid.fill(|cx, cy| {
            let t = state[row_base[cy] as usize + col_tile[cx] as usize];
            let g = glyphs[(t >> 8) as usize * tt + row_off[cy] as usize + col_off[cx] as usize];
            Cell::new(g, t & 0xFF)
        });
        grid.flush(s, &PAL);
    }

    fn name(&self) -> &'static str {
        "tactiles"
    }

    fn grid(&self) -> &Grid {
        &self.grid
    }

    fn palette(&self) -> &[u32] {
        &PAL
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::saver;

    use crate::testalloc::count;

    fn panel() -> Panel {
        Panel::new(1920, 1080, 1920)
    }

    /// T1. Frame 0 must cover the panel — including the strip below the last
    /// cell row — and must cover it with the SCENE: a reported black rectangle
    /// satisfies the rows assertion on its own. Both shipped panel sizes plus
    /// two whose height leaves a strip no cell covers.
    #[test]
    fn frame_zero_paints_the_whole_panel() {
        for (w, h) in [(1920, 1080), (1280, 400), (1920, 1050), (1918, 401)] {
            let p = Panel::new(w, h, w);
            let mut c = Tactiles::new(&p, 30);
            let mut buf = vec![0u32; p.buf_len()];
            let d = saver::frame(&mut c, &mut buf, &p);
            assert_eq!(d.rows(), h, "{w}x{h}: frame 0 must paint the whole panel");
            let lit = buf.iter().filter(|&&px| px != 0).count();
            assert!(lit > w * h / 100, "{w}x{h}: frame 0 drew almost nothing");
        }
        // The odd sizes above are only worth testing if one of them really does
        // leave a remainder strip.
        let p = Panel::new(1918, 401, 1918);
        let c = Tactiles::new(&p, 30);
        assert!(!p.h.is_multiple_of(c.grid.cell_h()));
        assert!(!p.w.is_multiple_of(c.grid.cell_w()));
    }

    /// T2. Every scanline whose pixels changed is inside a damage run —
    /// otherwise simpledrm scans out the previous frame there forever. The
    /// companions keep this from being a bound nothing approaches: something
    /// must move, and it must stay under a full repaint.
    #[test]
    fn damage_covers_every_changed_scanline() {
        let p = panel();
        let mut c = Tactiles::new(&p, 30);
        let mut buf = vec![0u32; p.buf_len()];
        let mut prev = vec![0u32; p.buf_len()];
        let stride = p.buf_len() / p.h;
        saver::frame(&mut c, &mut buf, &p);

        let mut worst = 0usize;
        let mut total = 0usize;
        let mut cells_changed = 0usize;
        let mut before: Vec<Cell> = c.grid.cells().to_vec();
        const FRAMES: usize = 200;
        for n in 1..FRAMES {
            prev.copy_from_slice(&buf);
            before.copy_from_slice(c.grid.cells());
            let d = saver::frame(&mut c, &mut buf, &p);
            cells_changed += c
                .grid
                .cells()
                .iter()
                .zip(before.iter())
                .filter(|(a, b)| a != b)
                .count();
            for y in 0..p.h {
                let row = y * stride..y * stride + p.w;
                if buf[row.clone()] != prev[row] {
                    assert!(
                        d.runs()
                            .iter()
                            .any(|&(a, b)| (a as usize..b as usize).contains(&y)),
                        "frame {n}: scanline {y} changed but was not reported"
                    );
                }
            }
            worst = worst.max(d.rows());
            total += d.rows();
        }
        assert!(
            total / (FRAMES - 1) > 0,
            "nothing moved: the coverage check proves nothing"
        );
        assert!(worst <= p.h, "more rows reported than the panel has");
        // Rows are the wrong unit for this saver and the right bound is cells:
        // the waves cross the WHOLE grid, so a handful of tiles flipping in
        // every 48px band still reports most of the panel (measured: 1080 of
        // 1080 rows, ~1% of cells). What must stay small is the blit, which is
        // per changed CELL — `flush` skips the rest whatever the runs say.
        let per_frame = cells_changed / (FRAMES - 1);
        let cells = c.grid.rows() * c.grid.cols();
        assert!(per_frame > 0, "no cell changed: nothing is moving");
        assert!(
            per_frame * 10 < cells,
            "{per_frame} of {cells} cells repainted a frame: a full repaint"
        );
    }

    /// T3. `render` must not allocate. Every buffer is sized in `new` and the
    /// frame path only indexes them; a counting allocator is the check a
    /// capacity assertion cannot make, because it also sees a temporary that is
    /// allocated and freed inside the same frame.
    #[test]
    fn render_never_allocates() {
        let p = Panel::new(1280, 400, 1280);
        let mut c = Tactiles::new(&p, 30);
        let mut buf = vec![0u32; p.buf_len()];
        // Frame 0 runs the same code, but outside the window, so a failure
        // points at the frame loop rather than at first-frame setup.
        saver::frame(&mut c, &mut buf, &p);

        let before = count();
        for _ in 0..2_000 {
            saver::frame(&mut c, &mut buf, &p);
        }
        let after = count();
        assert_eq!(
            after,
            before,
            "the render path allocated {} times in 2000 frames",
            after - before
        );
        // Non-vacuous: the counter does move for something that DOES allocate,
        // so the equality above is a fact about `render`, not about the harness.
        drop(vec![0u8; 64]);
        assert!(after < count(), "the allocation counter is dead");
        // ...and those frames drew something.
        assert!(c
            .grid
            .cells()
            .iter()
            .any(|c| c.glyph() != font::BLANK as usize));
    }

    /// T4. Every colour index a tile can address must exist. One off the end is
    /// an index panic inside `Grid::blit`, on the panel, weeks in.
    #[test]
    fn every_reachable_colour_index_is_in_the_palette() {
        let p = panel();
        let mut c = Tactiles::new(&p, 30);
        let mut seen = [false; PAL_N];
        for _ in 0..2000 {
            c.advance();
            for &t in c.state.iter() {
                assert!((t & 0xFF) < PAL_N as u16, "colour {} off the end", t & 0xFF);
                assert!(((t >> 8) as usize) < VARIANTS, "variant off the end");
                seen[(t & 0xFF) as usize] = true;
            }
        }
        assert_eq!(PAL.len(), PAL_N);
        // And the bound is tight: every entry is reachable, so the palette is
        // not carrying colours nothing can address.
        assert!(seen.iter().all(|&s| s), "unreachable palette entries");
    }

    /// T5. The baked glyphs have to be SHAPES: a variant that lit no dots is an
    /// invisible tile, one that lit every dot is a solid block, and a rotation
    /// that changed nothing means the rotation wave does nothing for that shape.
    /// Corner and arc are the two with four genuinely distinct orientations.
    #[test]
    fn every_variant_is_a_distinct_visible_stroke() {
        let p = panel();
        let c = Tactiles::new(&p, 30);
        let tt = c.tile * c.tile;
        for v in 0..VARIANTS {
            let g = &c.glyphs[v * tt..][..tt];
            assert!(
                g.iter().any(|&x| x != font::BRAILLE[0]),
                "variant {v} is blank"
            );
            assert!(
                g.iter().any(|&x| x == font::BRAILLE[0]),
                "variant {v} fills the whole tile"
            );
        }
        for shape in [2usize, 3] {
            for rot in 0..ROTS {
                for other in rot + 1..ROTS {
                    let a = &c.glyphs[(shape * ROTS + rot) * tt..][..tt];
                    let b = &c.glyphs[(shape * ROTS + other) * tt..][..tt];
                    assert_ne!(a, b, "shape {shape} rotations {rot}/{other} are the same");
                }
            }
        }
    }

    /// T6. The bake, at sub-cell resolution. Everything above compares glyph
    /// indices to each other, which is blind to the two ways the bake itself
    /// goes wrong: a `dot_bit` that maps two sub-cells onto one braille bit
    /// (dots 7 and 8 are NOT `col * 3 + 3`), and a stroke width that is off by
    /// a factor. Both leave sixteen distinct non-blank variants.
    ///
    /// The unrotated bar is the shape with an answer that can be written down:
    /// one contiguous run of sub-columns, centred, identical on every sub-row.
    #[test]
    fn the_bar_bakes_to_a_centred_full_height_stripe() {
        let p = panel();
        let c = Tactiles::new(&p, 30);
        // `BRAILLE` maps a dot byte to a glyph index; the check needs it the
        // other way round.
        let mut from_glyph = [0u8; font::GLYPHS.len()];
        for (bits, &g) in font::BRAILLE.iter().enumerate() {
            from_glyph[g as usize] = bits as u8;
        }
        let sub_w = c.tile * 2;
        let lit = |sx: usize, sy: usize| {
            let g = c.glyphs[(sy / 4) * c.tile + sx / 2];
            from_glyph[g as usize] & dot_bit(sx & 1, sy & 3) != 0
        };

        let row0: Vec<usize> = (0..sub_w).filter(|&sx| lit(sx, 0)).collect();
        for sy in 1..c.tile * 4 {
            let row: Vec<usize> = (0..sub_w).filter(|&sx| lit(sx, sy)).collect();
            assert_eq!(row, row0, "sub-row {sy} of the bar differs from sub-row 0");
        }
        assert_eq!(
            row0.last().unwrap() - row0[0] + 1,
            row0.len(),
            "the bar is not one contiguous run: {row0:?}"
        );
        // Centred: the gaps either side match.
        assert_eq!(row0[0], sub_w - 1 - row0.last().unwrap(), "{row0:?}");
        // Default TACTILES_STROKE is 24% of the tile, which at 12 sub-columns
        // rounds to two or three. Double the stroke and it is five.
        assert!((2..=4).contains(&row0.len()), "bar is {} wide", row0.len());
    }

    /// T7. What the panel actually shows: every CELL must carry its tile's
    /// baked glyph at its own offset within the tile, in its tile's colour.
    /// Nothing else here looks at the cells the frame loop produced, so a
    /// render that drew one variant everywhere, or dropped the within-tile row
    /// offset, or coloured tiles by their shape, passes every other test and is
    /// a visibly wrong panel.
    #[test]
    fn every_cell_carries_its_tile_s_glyph_and_colour() {
        let p = Panel::new(1280, 400, 1280);
        let mut c = Tactiles::new(&p, 30);
        let mut buf = vec![0u32; p.buf_len()];
        for _ in 0..3 {
            saver::frame(&mut c, &mut buf, &p);
        }
        let (cols, rows, tile, tt) = (c.grid.cols(), c.grid.rows(), c.tile, c.tile * c.tile);
        let mut variants = [0usize; VARIANTS];
        for cy in 0..rows {
            for cx in 0..cols {
                let t = c.state[(cy / tile) * c.tiles_x + cx / tile];
                let want = Cell::new(
                    c.glyphs[(t >> 8) as usize * tt + (cy % tile) * tile + cx % tile],
                    t & 0xFF,
                );
                assert_eq!(c.grid.cells()[cy * cols + cx], want, "cell ({cx}, {cy})");
                variants[(t >> 8) as usize] += 1;
            }
        }
        // Non-vacuous: the frame is not one variant repeated, so "matches the
        // table" is a claim about sixteen lookups and not about one.
        assert!(
            variants.iter().filter(|&&n| n > 0).count() > VARIANTS / 2,
            "only {:?} variants on screen",
            variants
        );
    }

    /// T8. The pattern travels in BANDS. A tile's variant must change over time
    /// (or the panel is a still image), and neighbouring tiles must agree far
    /// more often than chance (or it is noise, not a tiling). Chance for
    /// sixteen variants is 6% and the measured figure is 46%; the bar at 25%
    /// only has to separate "banded" from "rolled", so it sits far from both.
    #[test]
    fn the_tiling_rearranges_in_bands_not_at_random() {
        let p = panel();
        let mut c = Tactiles::new(&p, 30);
        let (tx, ty) = (c.tiles_x, c.tiles_y);

        let first: Vec<u16> = c.state.clone();
        for _ in 0..60 {
            c.advance();
        }
        let moved = c
            .state
            .iter()
            .zip(first.iter())
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            moved > c.state.len() / 10,
            "only {moved} of {} tiles changed in two seconds: a still image",
            c.state.len()
        );

        let mut same = 0usize;
        let mut pairs = 0usize;
        for y in 0..ty {
            for x in 0..tx - 1 {
                let (a, b) = (c.state[y * tx + x], c.state[y * tx + x + 1]);
                same += (a >> 8 == b >> 8) as usize;
                pairs += 1;
            }
        }
        assert!(
            same * 4 > pairs,
            "only {same} of {pairs} neighbours share a variant: that is noise"
        );
    }
}
