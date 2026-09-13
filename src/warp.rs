//! Warp — flying forward through a starfield. Points stream out of a vanishing
//! point at the centre, accelerating and brightening as they pass the camera.
//!
//! # What makes it read as depth
//!
//! Nothing here moves in screen space. Each star holds a fixed `(x, y)` in the
//! tunnel's cross-section and one `z` that only ever decreases, and the screen
//! position is `centre + x * focal / z` every frame. All of the effect falls out
//! of that divide: the outward acceleration, the streak that lengthens near the
//! edge, and the fact that the centre stays sparse while the rim is busy. A
//! version that interpolated screen positions linearly would look like an
//! explosion in a flat plane and would pass every damage test here — which is
//! what `perspective_accelerates` exists to catch.
//!
//! A star's streak is the segment between where it was LAST frame and where it
//! is now, so it is the motion blur the projection implies rather than a decay
//! trail. That is also why nothing fades: a frame erases exactly what the
//! previous frame stamped, so there is no history on the panel to go stale.
//!
//! # Sub-cell resolution
//!
//! A star drawn as a whole cell is a brick. The field is drawn into braille
//! patterns instead — 2x4 dots per cell, one colour per cell — so at the default
//! 8x8 cell a star is a 4x2 pixel dot on a 480x540 dot field, and the streaks
//! stay thin. `bits`/`col` accumulate the frame into those two per-cell bytes;
//! the grid cell is written once, at the end, from both.
//!
//! # Per-frame cost
//!
//! Sparse (`flush_sparse`). A frame touches only the cells last frame stamped
//! plus the cells this one does — a few thousand of 32400, but scattered, so
//! the DAMAGE is most of the panel even though the blit is not. Full-repaint
//! savers (matrix 113m, fire 144m) blit every cell; this blits roughly a tenth
//! of them and does its arithmetic per STAR, not per cell. `dirty` and the two
//! accumulators are sized in `new` against a bound the draw cannot exceed, so
//! the frame loop never allocates.
use crate::font;
use crate::grid::{bake, dot_bit, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, next_rand};

/// Cold blue at the vanishing point warming to white overhead — the far end of
/// the ramp has to stay dark enough that the centre reads as distance and not
/// as a light source.
#[rustfmt::skip]
const PAL_RGB: [[u8; 3]; 9] = [
    [0x00, 0x00, 0x00], // 0 background
    [0x18, 0x20, 0x3C], // 1 furthest
    [0x28, 0x3C, 0x68],
    [0x40, 0x5C, 0x98],
    [0x60, 0x84, 0xC4],
    [0x8C, 0xAC, 0xE4],
    [0xB4, 0xCC, 0xF4],
    [0xDC, 0xEC, 0xFF],
    [0xFF, 0xFF, 0xFF], // 8 passing the camera
];
const PAL: [u32; 9] = bake(&PAL_RGB);

/// Levels 1..=LEVELS; 0 is the background and no star ever uses it.
const LEVELS: u8 = 8;
/// At and above this, a star is stamped two dots wide. Below it the dots are
/// single, which is what keeps the far field from reading as a grey wash.
const FAT: u8 = 6;

/// Depth range, in the same arbitrary units as `focal`. `Z_NEAR` is small
/// enough that a star near the axis still leaves the frame before it recycles;
/// a larger one pops stars out of existence mid-screen.
const Z_NEAR: f32 = 2.0;
const Z_FAR: f32 = 1000.0;
/// A star spawned on the axis would sit at the vanishing point forever, so the
/// spawn disc has a hole in it.
const R_MIN: f32 = 0.06;

pub struct Warp {
    grid: Grid,
    /// Tunnel cross-section position, |x|,|y| <= 1, and depth.
    xs: Vec<f32>,
    ys: Vec<f32>,
    zs: Vec<f32>,
    /// Braille dot bits and palette index being accumulated for this frame.
    /// Both are zero everywhere except the cells in `stamped`.
    bits: Vec<u8>,
    col: Vec<u8>,
    /// Cells this frame stamped — next frame's erase list, and the reason no
    /// trail can survive: the erase is exact, not a repaint of an approximate
    /// background.
    stamped: Vec<u32>,
    dirty: Vec<u32>,
    rng: u32,
    focal: f32,
    dz: f32,
    max_steps: usize,
    /// Half the panel, in pixels — the projection's origin.
    cx: f32,
    cy: f32,
    /// Pixel size of one braille dot, the step the streak walks in.
    sub_w: f32,
    sub_h: f32,
    w: f32,
    h: f32,
}

impl Warp {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        let cell = env_num(&["WARP_CELL"], 8, 4, 32) as usize;
        let stars = env_num(&["WARP_STARS"], 650, 16, 8000) as usize;
        // Depth units per SECOND, so a slow panel flies at the same speed.
        let speed = env_num(&["WARP_SPEED"], 420, 10, 5000) as f32;
        // Half-width in pixels of the |x| = 1 rim at z = 100: the tunnel's
        // apparent width, and the only knob that changes how fast the field
        // opens out.
        let spread = env_num(&["WARP_SPREAD"], 700, 64, 8000) as f32;
        let max_steps = env_num(&["WARP_STREAK"], 256, 1, 2048) as usize;

        let grid = Grid::new(panel, cell, cell);
        let cells = grid.cols() * grid.rows();
        // Each star stamps at most (max_steps + 1) segment dots, each up to two
        // cells wide when fat; and no frame can stamp more cells than exist.
        let bound = stars
            .saturating_mul(max_steps + 1)
            .saturating_mul(2)
            .min(cells);

        let mut w = Self {
            xs: vec![0.0; stars],
            ys: vec![0.0; stars],
            zs: vec![0.0; stars],
            bits: vec![0; cells],
            col: vec![0; cells],
            stamped: Vec::with_capacity(bound),
            // Erase list plus stamp list, both bounded by the same figure.
            dirty: Vec::with_capacity(bound * 2),
            rng: 0x5EED_1234,
            focal: spread * 100.0,
            dz: speed / fps.max(1) as f32,
            max_steps,
            cx: panel.w as f32 / 2.0,
            cy: panel.h as f32 / 2.0,
            sub_w: (grid.cell_w() / 2).max(1) as f32,
            sub_h: (grid.cell_h() / 4).max(1) as f32,
            w: panel.w as f32,
            h: panel.h as f32,
            grid,
        };
        for i in 0..stars {
            w.spawn(i);
            // Frame 0 must already have depth, so the initial field is spread
            // through the tunnel rather than all parked at the far end.
            w.zs[i] = Z_NEAR + w.unit01() * (Z_FAR - Z_NEAR);
        }
        w
    }

    /// `next_rand` returns 31 bits, so this takes the low 16 rather than a
    /// top-down shift — a shift sized for a full u32 silently halves the range
    /// and puts every star in one quadrant.
    #[inline]
    fn unit01(&mut self) -> f32 {
        (next_rand(&mut self.rng) & 0xFFFF) as f32 / 65536.0
    }

    /// A fresh star at the far end. Pairs of coordinates come from consecutive
    /// splitmix32 draws, which is why this is not the LCG the older savers use.
    fn spawn(&mut self, i: usize) {
        let mut x = self.unit01() * 2.0 - 1.0;
        let mut y = self.unit01() * 2.0 - 1.0;
        let r2 = x * x + y * y;
        if r2 < R_MIN * R_MIN {
            if r2 > 1e-9 {
                let s = R_MIN / r2.sqrt();
                x *= s;
                y *= s;
            } else {
                x = R_MIN;
                y = 0.0;
            }
        }
        self.xs[i] = x;
        self.ys[i] = y;
        self.zs[i] = Z_FAR;
    }

    /// Light one braille dot at a pixel, keeping the brighter of the two
    /// colours if something already claimed the cell.
    #[inline]
    fn dot(&mut self, px: f32, py: f32, lev: u8) {
        if px < 0.0 || py < 0.0 || px >= self.w || py >= self.h {
            return;
        }
        let (px, py) = (px as usize, py as usize);
        let (cw, ch) = (self.grid.cell_w(), self.grid.cell_h());
        let (cx, cy) = (px / cw, py / ch);
        if cx >= self.grid.cols() || cy >= self.grid.rows() {
            return;
        }
        let i = cy * self.grid.cols() + cx;
        let sx = (((px % cw) as f32 / self.sub_w) as usize).min(1);
        let sy = (((py % ch) as f32 / self.sub_h) as usize).min(3);
        if self.bits[i] == 0 {
            self.stamped.push(i as u32);
        }
        self.bits[i] |= dot_bit(sx, sy);
        self.col[i] = self.col[i].max(lev);
    }

    fn step(&mut self) {
        for i in 0..self.zs.len() {
            let z1 = self.zs[i] - self.dz;
            if z1 <= Z_NEAR {
                self.spawn(i);
                continue;
            }
            let z0 = self.zs[i];
            self.zs[i] = z1;

            let (x, y) = (self.xs[i], self.ys[i]);
            let (x1, y1) = (self.cx + x * self.focal / z1, self.cy + y * self.focal / z1);
            // Once it is well past the edge it can never come back — recycling
            // it keeps the on-screen density up instead of spending the streak
            // loop on a star nobody can see.
            if (x1 - self.cx).abs() > self.w || (y1 - self.cy).abs() > self.h {
                self.spawn(i);
                continue;
            }
            let (x0, y0) = (self.cx + x * self.focal / z0, self.cy + y * self.focal / z0);

            // Cubed rather than linear in depth: linear brightness puts the
            // whole far field in the middle of the ramp, and the centre then
            // reads as a bright clump instead of as distance.
            let q = 1.0 - z1 / Z_FAR;
            let lev = (1 + (q * q * q * (LEVELS - 1) as f32) as u8).min(LEVELS);
            let (dx, dy) = (x1 - x0, y1 - y0);
            let steps = ((dx.abs() / self.sub_w).max(dy.abs() / self.sub_h).ceil() as usize)
                .clamp(1, self.max_steps);
            let inv = 1.0 / steps as f32;
            for k in 0..=steps {
                let t = k as f32 * inv;
                let (px, py) = (x0 + dx * t, y0 + dy * t);
                self.dot(px, py, lev);
                if lev >= FAT {
                    // Thicken across the streak so a close star is a spark
                    // rather than a hairline.
                    self.dot(px + self.sub_w, py, lev);
                }
            }
        }
    }
}

impl Saver for Warp {
    fn render(&mut self, s: &mut Surface<'_>) {
        self.dirty.clear();
        for k in 0..self.stamped.len() {
            let i = self.stamped[k];
            self.grid.set(i as usize, Cell::CLEAR);
            self.bits[i as usize] = 0;
            self.col[i as usize] = 0;
            self.dirty.push(i);
        }
        self.stamped.clear();

        self.step();

        for k in 0..self.stamped.len() {
            let i = self.stamped[k] as usize;
            self.grid.set(
                i,
                Cell::new(font::BRAILLE[self.bits[i] as usize], self.col[i] as u16),
            );
            self.dirty.push(i as u32);
        }
        // Sorted so two cells on one row collapse into one damage run; deduped
        // because a cell erased and restamped would otherwise blit twice.
        self.dirty.sort_unstable();
        self.dirty.dedup();
        self.grid.flush_sparse(s, &PAL, &self.dirty);
    }

    fn name(&self) -> &'static str {
        "warp"
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

    fn panel() -> Panel {
        // 1070 is not a multiple of the 8px cell: 133 rows and a 6-line strip
        // below them that belongs to no cell. A height that divided exactly
        // would prove nothing about the bottom of the panel.
        Panel::new(1920, 1070, 1920)
    }

    fn warp() -> Warp {
        Warp::new(&panel(), 30)
    }

    /// Frame 0 has to cover every pixel, including the strip no cell owns, and
    /// it has to cover it with the STARFIELD — a reported black rectangle
    /// satisfies the damage assertion on its own.
    #[test]
    fn frame_zero_paints_the_whole_panel() {
        let p = panel();
        let mut c = warp();
        let mut buf = vec![0u32; p.buf_len()];
        let d = saver::frame(&mut c, &mut buf, &p);
        assert_eq!(d.rows(), p.h, "frame 0 must paint the whole panel");
        let lit = buf.iter().filter(|&&px| px != 0).count();
        assert!(lit > 10_000, "frame 0 painted nothing ({lit} lit pixels)");
    }

    /// The sparse model's one real failure: a cell written into the scene but
    /// left out of `dirty` is never blitted, so on the panel that region is
    /// frozen for the life of the pod — and a framebuffer diff cannot see it,
    /// because the framebuffer never changed either.
    ///
    /// The second assertion is the trail check, and it is exact rather than a
    /// ratio: the lit cells ARE the cells this frame stamped, no more and no
    /// fewer. One un-erased cell fails it.
    #[test]
    fn every_written_cell_is_reported_and_nothing_is_left_behind() {
        let p = panel();
        let mut c = warp();
        let mut buf = vec![0u32; p.buf_len()];
        let cells = c.grid.cols() * c.grid.rows();
        for n in 0..200 {
            saver::frame(&mut c, &mut buf, &p);
            for i in 0..cells {
                assert_eq!(
                    c.grid.cell(i),
                    c.grid.cells()[i],
                    "frame {n}: cell {i} was written but not reported"
                );
            }
            let lit: Vec<u32> = (0..cells)
                .filter(|&i| c.grid.cells()[i].glyph() != font::BLANK as usize)
                .map(|i| i as u32)
                .collect();
            let mut stamped = c.stamped.clone();
            stamped.sort_unstable();
            assert_eq!(lit, stamped, "frame {n}: the scene is not what was stamped");
            assert!(!lit.is_empty(), "frame {n}: the field drained");
        }
    }

    /// `dirty` never growing past its reserve IS "the render path did not
    /// allocate": `clear` keeps capacity, and every other buffer here is a
    /// fixed-length `vec!` written in place.
    #[test]
    fn render_never_allocates() {
        let p = panel();
        let mut c = warp();
        let mut buf = vec![0u32; p.buf_len()];
        let reserved = c.dirty.capacity();
        let stamp_reserved = c.stamped.capacity();
        assert!(reserved > 0, "nothing was reserved for the frame loop");
        let mut worst = 0;
        for _ in 0..20_000 {
            saver::frame(&mut c, &mut buf, &p);
            assert_eq!(
                c.dirty.capacity(),
                reserved,
                "`dirty` grew past its reserve: the render path allocated"
            );
            assert_eq!(c.stamped.capacity(), stamp_reserved, "`stamped` grew");
            worst = worst.max(c.dirty.len());
        }
        // Non-vacuous: a reserve nothing ever fills proves nothing. A frame
        // stamps at least one cell per star, so passing the star count means
        // the list really does hold a whole field.
        assert!(
            worst > c.zs.len(),
            "`dirty` never held a whole field ({worst} cells)"
        );
    }

    /// What warp IS: 1/z perspective. A star's distance from the vanishing
    /// point must grow, and grow FASTER each frame. Interpolating screen
    /// positions linearly in z gives a constant step — a flat explosion that
    /// looks plausible in a still and passes every other test here.
    #[test]
    fn perspective_accelerates() {
        let p = panel();
        let mut c = warp();
        let mut buf = vec![0u32; p.buf_len()];
        // One star out on the rim, the rest parked at the far end near the
        // axis, so the outermost lit cell is that star for the whole run.
        for i in 0..c.zs.len() {
            c.xs[i] = R_MIN;
            c.ys[i] = 0.0;
            c.zs[i] = Z_FAR;
        }
        c.xs[0] = 0.9;
        c.ys[0] = 0.0;
        c.zs[0] = 500.0;

        let radius = |c: &Warp| {
            let (cols, cw, ch) = (c.grid.cols(), c.grid.cell_w(), c.grid.cell_h());
            let mut r: f32 = 0.0;
            for (i, cell) in c.grid.cells().iter().enumerate() {
                if cell.glyph() == font::BLANK as usize {
                    continue;
                }
                let x = (i % cols * cw) as f32 - c.cx;
                let y = (i / cols * ch) as f32 - c.cy;
                r = r.max((x * x + y * y).sqrt());
            }
            r
        };

        // Sampled every few frames: the radius is quantised to the 8px cell,
        // and a single frame's growth out here is smaller than that.
        const EVERY: usize = 5;
        saver::frame(&mut c, &mut buf, &p);
        let mut prev = radius(&c);
        let mut prev_step = 0.0f32;
        for n in 0..6 {
            for _ in 0..EVERY {
                saver::frame(&mut c, &mut buf, &p);
            }
            let r = radius(&c);
            let step = r - prev;
            assert!(step > 0.0, "frame {n}: the field is not expanding ({r})");
            assert!(
                step > prev_step,
                "frame {n}: step {step} did not exceed {prev_step} — motion is linear, not 1/z"
            );
            prev = r;
            prev_step = step;
        }
    }
}
