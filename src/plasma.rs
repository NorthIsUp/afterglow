//! plasma: the old demo-scene effect. Four sine fields, one a set of rings
//! round a wandering centre, summed and read as soft blobs of density.
//!
//! Started as a port of ascii.rest's plasma (credit in THIRD_PARTY.md): the
//! ramp, the four terms and the 30 s loop are upstream's. Upstream draws a
//! fixed 64x22 picture in one ink; this evaluates the field at the grid's own
//! resolution, so it fills any panel, and colours it.
//!
//! Colour is its own field — two broad waves far slower than the blobs — so
//! the blobs swim through bands of hue rather than carrying their colour with
//! them, and the whole wheel drifts once round per loop. Slow is also what
//! keeps it cheap: every cell whose colour index changes is a blit, and a hue
//! field as fast as the density repainted half the panel each frame.
//! Brightness rides on the glyph instead — dense cores glow — which costs no
//! extra blits, since a cell whose glyph changed is redrawn anyway.

use std::f32::consts::PI;
use std::f64::consts::PI as PI64;

use crate::env_num;
use crate::glyph;
use crate::grid::{pixel_aspect, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};

const RAMP: [char; 12] = ['.', ',', '-', '~', ':', ';', '=', '+', '*', '#', '%', '@'];
const LEVELS: usize = RAMP.len();
/// Seconds; every term turns a whole number of times in one loop, and so
/// does the hue drift.
const P: f64 = 30.0;
/// Ramp sweeps per unit of the summed field: low, so the blobs are broad.
const GAIN: f32 = 0.32;
/// The hue field: two broad, slow waves, one per axis, so colour sweeps
/// across the panel at a fraction of the blobs' speed. Palette turns per unit
/// of each wave, and the waves' spatial frequencies.
const HUE_GAIN: f32 = 0.4;
const HUE_X: f32 = 0.06;
const HUE_Y: f32 = 0.08;

/// Upstream's picture in cell widths (64 cols, 22 rows two widths tall). The
/// field is scaled so a panel shows as many blobs per unit AREA as upstream's
/// picture did, whatever its shape.
const UP_W: f32 = 64.0;
const UP_H: f32 = 44.0;

/// The cyclic hue wheel: deep blue, violet, magenta, coral, orange, gold, mint,
/// sky, and back. Interpolated to `HUES` steps at construction.
const STOPS: [[u8; 3]; 8] = [
    [0x22, 0x44, 0xe0],
    [0x78, 0x30, 0xf0],
    [0xc8, 0x1c, 0xb4],
    [0xff, 0x3c, 0x6e],
    [0xff, 0x82, 0x1e],
    [0xff, 0xd2, 0x3c],
    [0x3c, 0xe6, 0xa0],
    [0x1e, 0xa0, 0xff],
];
const HUES: usize = 32;
/// Sine table size. The field is quantised to twelve glyphs and 32 hues, so a
/// 4096-step sine is exact to the eye and spares two libm calls a cell.
const SINES: usize = 4096;
const TO_SINE: f32 = SINES as f32 / (2.0 * PI);
const _: () = assert!(SINES.is_power_of_two());

pub struct Plasma {
    /// Cell centres in upstream's units, per column and per row.
    xs: Vec<f32>,
    ys: Vec<f32>,
    /// Per column and per row, refreshed each frame: the axis's own sine term,
    /// its share of the rotating term's phase, its squared distance from the
    /// wandering centre, and its hue wave (the row's carrying the drift).
    col: Vec<[f32; 4]>,
    row: Vec<[f32; 4]>,
    sine: Vec<f32>,
    /// How far the wandering centre roams, in the same units.
    roam: (f32, f32),
    glyphs: [u16; LEVELS],
    /// `HUES * LEVELS`, hue-major: index `level * HUES + hue`.
    pal: Vec<u32>,
    frame: u64,
    /// Frames in one loop, so `frame` wraps and `t` never loses precision.
    period: u64,
    fps: f64,
    grid: Grid,
}

impl Plasma {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        let cell_w = env_num(&["PLASMA_CELL_W"], 12, 4, 64) as usize;
        let cell_h = env_num(&["PLASMA_CELL_H"], 24, 4, 128) as usize;
        let grid = Grid::new(panel, cell_w, cell_h);
        let (cols, rows) = (grid.cols(), grid.rows());
        // A row's height in cell widths on the glass: the grid's stretch undone.
        let row_w = (grid.cell_h() * 100) as f32 / (pixel_aspect() * grid.cell_w()) as f32;
        let (w, h) = (cols as f32, rows as f32 * row_w);
        let s = (UP_W * UP_H / (w * h)).sqrt();
        let xs = (0..cols).map(|c| (c as f32 + 0.5 - w / 2.0) * s).collect();
        let ys = (0..rows)
            .map(|r| ((r as f32 + 0.5) * row_w - h / 2.0) * s)
            .collect();
        // Upstream's 14 and 9 of a 32 x 22 half-picture.
        let roam = (w / 2.0 * s * 14.0 / 32.0, h / 2.0 * s * 9.0 / 22.0);
        let fps = f64::from(fps.max(1));
        Self {
            xs,
            ys,
            col: vec![[0.0; 4]; cols],
            row: vec![[0.0; 4]; rows],
            sine: (0..SINES).map(|i| (i as f32 / TO_SINE).sin()).collect(),
            roam,
            glyphs: RAMP.map(glyph::of),
            pal: palette(),
            frame: 0,
            period: (P * fps).round() as u64,
            fps,
            grid,
        }
    }
}

/// The hue wheel at every brightness. The bottom of the ramp is dim so the
/// thin glyphs read as haze; the top three lift towards white so the cores
/// glow rather than merely saturate.
fn palette() -> Vec<u32> {
    let mut pal = Vec::with_capacity(HUES * LEVELS);
    for l in 0..LEVELS {
        let k = l as f32 / (LEVELS - 1) as f32;
        let gain = 0.45 + 0.55 * k;
        let white = ((k - 0.72) / 0.28).max(0.0) * 0.45;
        for h in 0..HUES {
            let u = h as f32 * STOPS.len() as f32 / HUES as f32;
            let (i, f) = (u as usize, u.fract());
            let (a, b) = (STOPS[i], STOPS[(i + 1) % STOPS.len()]);
            let ch = |n: usize| {
                let c = (a[n] as f32 * (1.0 - f) + b[n] as f32 * f) * gain;
                (c + (255.0 - c) * white).round().min(255.0) as u32
            };
            pal.push((ch(0) << 16) | (ch(1) << 8) | ch(2));
        }
    }
    pal
}

impl Saver for Plasma {
    fn render(&mut self, s: &mut Surface<'_>) {
        let t = self.frame as f64 / self.fps;
        self.frame = (self.frame + 1) % self.period;
        let a = (2.0 * PI64 / P * t) as f32;
        let (ca, sa) = (a.cos(), a.sin());
        let (cx, cy) = (self.roam.0 * a.sin(), self.roam.1 * (2.0 * a).cos());
        let drift = a / (2.0 * PI);
        let sine = &self.sine[..];
        // `as i32` keeps a negative phase negative; masking its two's complement wraps it
        // into the table, which is only a modulo because SINES is a power of two.
        let sin = |p: f32| sine[(p * TO_SINE) as i32 as usize & (SINES - 1)];
        for (o, &x) in self.col.iter_mut().zip(&self.xs) {
            *o = [
                sin(x * 0.11 + 3.0 * a),
                x * ca * 0.09 + 4.0 * a,
                (x - cx) * (x - cx),
                sin(x * HUE_X + a) * HUE_GAIN,
            ];
        }
        for (o, &y) in self.row.iter_mut().zip(&self.ys) {
            *o = [
                sin(y * 0.13 - 2.0 * a),
                y * sa * 0.09,
                (y - cy) * (y - cy),
                sin(y * HUE_Y - a) * HUE_GAIN + drift,
            ];
        }
        let (ramp, five_a) = (a / PI, 5.0 * a);
        let (grid, col, row, glyphs) = (&mut self.grid, &self.col[..], &self.row[..], &self.glyphs);
        grid.fill(|c, r| {
            let ([t1, p3x, dx2, hx], [t2, p3y, dy2, hy]) = (col[c], row[r]);
            let t3 = sin(p3x + p3y);
            let t4 = sin((dx2 + dy2).sqrt() * 0.17 - five_a);
            let v = t1 + t2 + t3 + t4;
            // Up the ramp and back down, evenly, drifting through it once a loop.
            let u = v * GAIN + ramp;
            let k = (u - 2.0 * (u / 2.0 + 0.5).floor()).abs();
            let l = ((k * LEVELS as f32) as usize).min(LEVELS - 1);
            let hu = hx + hy;
            let h = ((hu - hu.floor()) * HUES as f32) as usize % HUES;
            Cell::new(glyphs[l], (l * HUES + h) as u16)
        });
        grid.flush(s, &self.pal);
    }

    fn name(&self) -> &'static str {
        "plasma"
    }

    fn grid(&self) -> &Grid {
        &self.grid
    }

    fn palette(&self) -> &[u32] {
        &self.pal
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font;
    use crate::grid::with_test_aspect;
    use crate::saver;
    use crate::testalloc::allocs_during;

    fn build(w: usize, h: usize, aspect: usize) -> (Panel, Plasma) {
        let p = Panel::new(w, h, w);
        let s = with_test_aspect(aspect, || Plasma::new(&p, 30));
        (p, s)
    }

    /// Every cell is drawn on every panel shape: no bars, no crop.
    #[test]
    fn fills_the_panel_at_any_shape() {
        for (w, h, aspect) in [
            (1920, 1080, 100),
            (1920, 1080, 180),
            (1280, 400, 100),
            (800, 1280, 100),
        ] {
            let (p, mut s) = build(w, h, aspect);
            let mut buf = vec![0u32; p.buf_len()];
            saver::frame(&mut s, &mut buf, &p);
            let g = &s.grid;
            assert!(
                g.cols() * g.cell_w() + g.cell_w() > w,
                "{w}x{h}: columns short"
            );
            assert!(
                g.rows() * g.cell_h() + g.cell_h() > h,
                "{w}x{h}: rows short"
            );
            assert!(
                g.cells().iter().all(|c| c.glyph() != font::BLANK as usize),
                "{w}x{h}@{aspect}: a blank cell"
            );
        }
    }

    /// Non-vacuous colour: a frame uses many hues and every brightness.
    #[test]
    fn a_frame_is_many_coloured_and_spans_the_ramp() {
        let (p, mut s) = build(1920, 1080, 180);
        let mut buf = vec![0u32; p.buf_len()];
        saver::frame(&mut s, &mut buf, &p);
        let mut hues = [false; HUES];
        let mut levels = [false; LEVELS];
        for c in s.grid.cells() {
            hues[c.colour() % HUES] = true;
            levels[c.colour() / HUES] = true;
        }
        assert!(
            hues.iter().filter(|&&b| b).count() > HUES / 2,
            "too few hues"
        );
        assert!(levels.iter().all(|&b| b), "a ramp level never appears");
    }

    /// It moves, and it loops: frame `period` is frame 0 again.
    #[test]
    fn moves_and_loops_every_thirty_seconds() {
        let (p, mut s) = build(1280, 400, 100);
        let mut buf = vec![0u32; p.buf_len()];
        saver::frame(&mut s, &mut buf, &p);
        let first = s.grid.cells().to_vec();
        saver::frame(&mut s, &mut buf, &p);
        assert_ne!(s.grid.cells(), &first[..], "frame 1 is frame 0");
        for _ in 2..s.period {
            saver::frame(&mut s, &mut buf, &p);
        }
        saver::frame(&mut s, &mut buf, &p);
        assert_eq!(s.grid.cells(), &first[..], "the loop does not close");
    }

    /// The cost guard. Every cell whose glyph OR colour changed is a blit,
    /// and a hue field moving as fast as the density repainted half the panel
    /// a frame (1.6x matrix at 1080p). Measured: ~17% at 30 fps, ~11% of that
    /// the glyphs.
    #[test]
    fn under_a_quarter_of_the_cells_change_a_frame() {
        let (p, mut s) = build(1920, 1080, 100);
        let mut buf = vec![0u32; p.buf_len()];
        saver::frame(&mut s, &mut buf, &p);
        let mut prev = s.grid.cells().to_vec();
        let (mut changed, mut total) = (0, 0);
        for _ in 0..300 {
            saver::frame(&mut s, &mut buf, &p);
            let cells = s.grid.cells();
            changed += prev.iter().zip(cells).filter(|(a, b)| a != b).count();
            total += cells.len();
            prev.copy_from_slice(cells);
        }
        let pct = changed * 100 / total;
        assert!((5..25).contains(&pct), "{pct}% of cells changed a frame");
    }

    #[test]
    fn render_never_allocates() {
        let (p, mut s) = build(1920, 1080, 180);
        let mut buf = vec![0u32; p.buf_len()];
        saver::frame(&mut s, &mut buf, &p);
        let n = allocs_during(|| {
            for _ in 0..200 {
                saver::frame(&mut s, &mut buf, &p);
            }
        });
        assert_eq!(n, 0, "render allocated");
    }
}
