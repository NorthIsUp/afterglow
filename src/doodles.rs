//! Doodles — After Dark's scribbler. A pen wanders the panel leaving one
//! continuous freehand line that loops back over itself until the sheet is
//! full, then the scribble fades and a fresh sheet starts.
//!
//! # Why this is not lissajous with a different formula
//!
//! Lissajous evaluates a closed-form curve: the pen's position at time t is a
//! function of t alone, the figure is periodic, and it reads as mathematics
//! because it is. Here the pen has STATE — a heading and an angular velocity —
//! and only the angular velocity is driven, by a damped random walk. Position
//! is the double integral of noise, so the line curls, commits to an arc,
//! comes out of it and wanders off: nothing repeats, and no frame of it can be
//! reproduced from a formula.
//!
//! Perturbing the HEADING instead is the failure mode to avoid — that is
//! jitter, and it reads as a noisy straight line. Worms drives its turn rate
//! for the same reason.
//!
//! # Staying on the paper, and not drawing a circle forever
//!
//! Two degenerate ends bracket the one knob that matters, `DOODLES_INERTIA`:
//!
//! * too little (heading uncorrelated between steps) is white noise, a fuzzy
//!   blob rather than a line;
//! * too much (angular velocity effectively constant) is a circle retracing
//!   itself forever, which fills nothing and never ends.
//!
//! The angular velocity is mean-reverting — `w = w * inertia + noise`, clamped
//! to `DOODLES_CURL` — so its sign changes on its own and a fixed cycle is not
//! a state the walk can hold. At the defaults the correlation time is ~25 steps
//! and the typical turn radius ~17 sub-cells, so curvature persists for about
//! one loop's arc: loops and curls, not a spirograph and not static.
//!
//! `DOODLES_CURL` is what actually forecloses the circle, and the inertia only
//! decides how curly the line is: measured over 200k steps, raising the inertia
//! from 0.96 to 0.9999 moves the turn rate's lag-1 correlation from 0.954 to
//! 0.979 and the cells inked per step barely at all (0.074 to 0.069), because
//! the clamp bounds the tightest arc and the edge keeps breaking it. So the
//! high end of the knob's range draws a loopier doodle, not a degenerate one.
//!
//! The panel edge REFLECTS (heading mirrored, position clamped, plus a small
//! random kick so a pen cannot settle into a two-bounce cycle in a corner).
//! Clamping is what makes "the pen never leaves the panel" structural rather
//! than probable.
//!
//! # Damage model: full repaint (Model A)
//!
//! `Grid::flush` diffs cur against prev, so it structurally cannot
//! under-report — and under-reporting on simpledrm is a region of the panel
//! frozen forever. The sparse alternative would need a dirty list covering both
//! the pen's cells and, during the fade, every lit cell on the sheet changing
//! shade at once. That is most of the scribble, every fade frame, and is no
//! cheaper to know than to recompute.
//!
//! # Sub-cell geometry and colour
//!
//! Cells are braille patterns: 2x4 dots per cell, so the line is drawn at twice
//! the horizontal and four times the vertical cell resolution for one `Cell` of
//! storage. At the default 8x16 cell a sub-cell is a square 4x4 pixels, which
//! is why the pen can move in sub-cell units without an aspect correction — and
//! why the knobs are in sub-cells rather than pixels.
//!
//! Each pen owns a colour family that sweeps from one end to the other over the
//! life of a doodle, so the scribble shows its own history: the oldest strokes
//! are one hue, the newest another, and how full the sheet is can be read off
//! the colours. The fade is a fourth dimension of the same palette.
//!
//! Dots accumulate in a cell for the whole doodle, which is correct — a
//! crossing is real ink — and the whole `dots` buffer is zeroed when a sheet
//! ends. Lissajous needs a per-cell freshness threshold because its trail fades
//! continuously underneath a pen that comes back; here nothing is drawn during
//! the fade, so a cell can never hold dots from a sheet that is gone.
//!
//! # Knobs
//!
//! | env | default | range | what |
//! | --- | --- | --- | --- |
//! | `DOODLES_CELL_W` | 8 | 4..64 | cell width in px |
//! | `DOODLES_CELL_H` | 16 | 8..128 | cell height in px |
//! | `DOODLES_PENS` | 3 | 1..4 | pens drawing at once, each its own colour |
//! | `DOODLES_SPEED` | 420 | 30..8000 | pen steps per second (one step is half a sub-cell) |
//! | `DOODLES_INERTIA` | 960 | 500..999 | per-mille of the turn rate carried to the next step |
//! | `DOODLES_WANDER` | 20 | 1..300 | milli-radians of turn-rate noise per step |
//! | `DOODLES_CURL` | 90 | 5..500 | milli-radians per step, the tightest curl allowed |
//! | `DOODLES_FILL_PCT` | 28 | 1..90 | lit cells at which the sheet is full |
//! | `DOODLES_MAX_S` | 90 | 5..900 | a sheet ends here even if it never fills |
//! | `DOODLES_FADE_MS` | 1600 | 200..10000 | how long the finished scribble takes to go |

use crate::font;
use crate::grid::{bake, dot_bit, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, next_rand};

/// Colour stages across one doodle's life. Eight is enough for the sweep to
/// read as a gradient across the scribble and keeps the palette under 129.
const STAGES: usize = 8;
const MAX_PENS: usize = 4;
/// Fade steps, including 0 = full brightness. Four, because the scribble is
/// gone in under two seconds and nobody counts the steps.
const FADE: usize = 4;

/// How far the pen moves per step, in sub-cells. Half a sub-cell, so a
/// diagonal step still lands in an adjacent sub-cell and the line has no gaps.
const STEP: f32 = 0.5;

/// Cells on a 1080p panel at the default 8x16 cell — the size `DOODLES_SPEED`
/// is quoted at. See `per_frame`.
const REF_CELLS: f32 = 16_000.0;

/// Each pen's family, start of the doodle to end. Two hues apart per pen, so a
/// stroke says both WHO drew it and WHEN.
#[rustfmt::skip]
const FAMILY: [[[u8; 3]; 2]; MAX_PENS] = [
    [[0x46, 0xE8, 0xFF], [0xA8, 0x70, 0xFF]],
    [[0xFF, 0x5C, 0xD0], [0xFF, 0xC0, 0x4A]],
    [[0x9C, 0xF0, 0x50], [0x28, 0xD0, 0xB4]],
    [[0xFF, 0x8C, 0x50], [0xFF, 0x46, 0x8C]],
];

/// Brightness of each fade step, /255.
const DIM: [u32; FADE] = [255, 150, 76, 26];

/// Index 0 is black, then one entry per (pen, stage, fade step). Separate
/// ranges per pen rather than a shared ramp, so a cell cannot borrow another
/// pen's colour by drifting one index.
const PAL_LEN: usize = 1 + MAX_PENS * STAGES * FADE;

const fn ramp() -> [[u8; 3]; PAL_LEN] {
    let mut out = [[0u8; 3]; PAL_LEN];
    let mut k = 0;
    while k < MAX_PENS {
        let mut s = 0;
        while s < STAGES {
            let mut d = 0;
            while d < FADE {
                let mut c = 0;
                while c < 3 {
                    let a = FAMILY[k][0][c] as u32;
                    let b = FAMILY[k][1][c] as u32;
                    let mixed = (a * (STAGES - 1 - s) as u32 + b * s as u32) / (STAGES - 1) as u32;
                    out[1 + (k * STAGES + s) * FADE + d][c] = (mixed * DIM[d] / 255) as u8;
                    c += 1;
                }
                d += 1;
            }
            s += 1;
        }
        k += 1;
    }
    out
}

const PAL: [u32; PAL_LEN] = bake(&ramp());

/// `next_rand` yields 31 bits; this is those as 0.0..1.0.
#[inline]
fn unit(rng: &mut u32) -> f32 {
    next_rand(rng) as f32 / (1u32 << 31) as f32
}

/// Zero-centred noise in `±mag`.
#[inline]
fn jitter(rng: &mut u32, mag: f32) -> f32 {
    (unit(rng) * 2.0 - 1.0) * mag
}

struct Pen {
    /// Position in sub-cells. Float, because a pen that moved in whole
    /// sub-cells could only draw at 45-degree increments.
    x: f32,
    y: f32,
    /// Heading, radians.
    th: f32,
    /// Turn rate, radians per step. The only driven quantity — see the module
    /// doc.
    w: f32,
    rng: u32,
}

pub struct Doodles {
    grid: Grid,
    cols: usize,
    /// Braille dots laid in each cell so far this doodle.
    dots: Vec<u8>,
    /// `pen * STAGES + stage` of the last stroke through the cell.
    tint: Vec<u8>,
    pens: Vec<Pen>,
    /// Cells with at least one dot. Counted as they are lit rather than
    /// scanned for, so "is the sheet full" costs nothing per frame.
    lit: usize,
    /// `lit` at which the sheet is full.
    target: usize,
    /// Frames drawn on this sheet, and the cap that ends it regardless.
    age: u32,
    max_frames: u32,
    /// Fade progress, in frames, once the sheet is done. `None` while drawing.
    fade: Option<u32>,
    fade_frames: u32,
    /// Current fade step, 0 while drawing.
    dim: usize,
    /// Pen steps per frame.
    per_frame: u32,
    inertia: f32,
    wander: f32,
    curl: f32,
    /// Half-open sub-cell extents the pen is clamped into.
    subw: f32,
    subh: f32,
    rng: u32,
}

impl Doodles {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        let cell_w = env_num(&["DOODLES_CELL_W"], 8, 4, 64) as usize;
        let cell_h = env_num(&["DOODLES_CELL_H"], 16, 8, 128) as usize;
        let pens = env_num(&["DOODLES_PENS"], 3, 1, MAX_PENS as i64) as usize;
        let speed = env_num(&["DOODLES_SPEED"], 420, 30, 8000) as u32;
        let inertia = env_num(&["DOODLES_INERTIA"], 960, 500, 999) as f32 / 1000.0;
        let wander = env_num(&["DOODLES_WANDER"], 20, 1, 300) as f32 / 1000.0;
        let curl = env_num(&["DOODLES_CURL"], 90, 5, 500) as f32 / 1000.0;
        let fill_pct = env_num(&["DOODLES_FILL_PCT"], 28, 1, 90) as usize;
        let max_s = env_num(&["DOODLES_MAX_S"], 90, 5, 900) as u32;
        let fade_ms = env_num(&["DOODLES_FADE_MS"], 1600, 200, 10_000) as u32;

        let grid = Grid::new(panel, cell_w, cell_h);
        let (cols, rows) = (grid.cols(), grid.rows());
        let fps = fps.max(1);
        let scale = ((cols * rows) as f32 / REF_CELLS).clamp(1.0 / 3.0, 1.0);

        let mut me = Self {
            grid,
            cols,
            dots: vec![0; cols * rows],
            tint: vec![0; cols * rows],
            pens: Vec::with_capacity(pens),
            lit: 0,
            target: (cols * rows * fill_pct / 100).max(1),
            age: 0,
            max_frames: max_s * fps,
            fade: None,
            fade_frames: (fade_ms * fps / 1000).max(1),
            dim: 0,
            // Speed is per SECOND and divided by fps here, so a slower panel
            // takes more steps per frame and the pen draws at the same rate.
            // It is also scaled by panel AREA: a doodle lasts as long as the
            // pen takes to fill the sheet, so the same hand speed finishes a
            // 1280x400 panel four times sooner than a 1080p one — measured 5s
            // against 19s. The floor stops a small panel being drawn in slow
            // motion; between them a doodle is a quarter-minute either way.
            // Rounded, not truncated: steps per frame is a small integer, and
            // truncating 3.5 costs a seventh of the hand's speed.
            per_frame: ((speed as f32 * scale / fps as f32).round() as u32).max(1),
            inertia,
            wander,
            curl,
            subw: (cols * 2) as f32,
            subh: (rows * 4) as f32,
            // Seeded off the clock so a restart does not replay one doodle
            // forever. Same trick sakura grows its tree from.
            rng: crate::saver_seed(&["DOODLES_SEED"], 0xD00D_1E55),
        };
        for k in 0..pens {
            let seed = me.rng ^ (k as u32).wrapping_mul(0x9E37_79B9);
            me.pens.push(Self::fresh_pen(me.subw, me.subh, seed));
        }
        // Open on a sheet that has been drawn on for a second, so the panel
        // does not come up blank and frame 0 has something on it.
        for _ in 0..fps {
            me.tick();
        }
        me
    }

    fn fresh_pen(subw: f32, subh: f32, seed: u32) -> Pen {
        let mut rng = seed | 1;
        // Start in the middle half of the panel: a pen begun in a corner spends
        // its first seconds bouncing out of it.
        Pen {
            x: subw * (0.25 + unit(&mut rng) * 0.5),
            y: subh * (0.25 + unit(&mut rng) * 0.5),
            th: unit(&mut rng) * std::f32::consts::TAU,
            w: 0.0,
            rng,
        }
    }

    /// Light the cell holding sub-cell `(sx, sy)`, in pen `k`'s colour at the
    /// doodle's current stage.
    #[inline]
    fn stamp(&mut self, sx: usize, sy: usize, tint: u8) {
        let i = (sy / 4) * self.cols + (sx / 2);
        if self.dots[i] == 0 {
            self.lit += 1;
        }
        self.dots[i] |= dot_bit(sx & 1, sy & 3);
        self.tint[i] = tint;
    }

    /// One pen, one step: turn rate first, then heading, then position, then
    /// the edge. Returns the sub-cell it landed in.
    #[inline]
    fn advance(&mut self, k: usize) -> (usize, usize) {
        let (subw, subh) = (self.subw, self.subh);
        let (inertia, wander, curl) = (self.inertia, self.wander, self.curl);
        let p = &mut self.pens[k];
        p.w = (p.w * inertia + jitter(&mut p.rng, wander)).clamp(-curl, curl);
        p.th += p.w;
        p.x += STEP * p.th.cos();
        p.y += STEP * p.th.sin();
        // Reflect, and kick: without the kick a pen can fall into a two-bounce
        // cycle in a corner and draw the same V forever.
        let hi_x = subw - 1.0;
        let hi_y = subh - 1.0;
        if p.x < 0.0 || p.x > hi_x {
            p.x = p.x.clamp(0.0, hi_x);
            p.th = std::f32::consts::PI - p.th + jitter(&mut p.rng, 0.3);
            p.w = -p.w * 0.5;
        }
        if p.y < 0.0 || p.y > hi_y {
            p.y = p.y.clamp(0.0, hi_y);
            p.th = -p.th + jitter(&mut p.rng, 0.3);
            p.w = -p.w * 0.5;
        }
        // Keep the accumulator small: f32 loses angular resolution as the
        // exponent grows, and this runs for months.
        p.th %= std::f32::consts::TAU;
        // NaN would index the whole scene out of bounds; it cannot arise from
        // the arithmetic above, and `as usize` saturates rather than wrapping,
        // so the clamp is the guard and costs two compares a step.
        (
            (p.x as usize).min(subw as usize - 1),
            (p.y as usize).min(subh as usize - 1),
        )
    }

    /// Advance one frame of simulation: draw, or fade, or start a new sheet.
    fn tick(&mut self) {
        if let Some(t) = self.fade {
            let t = t + 1;
            self.fade = Some(t);
            self.dim = (t as usize * FADE / self.fade_frames as usize).min(FADE);
            if self.dim >= FADE {
                self.restart();
            }
            return;
        }
        // The stage is read once a frame, not once a step: within a frame the
        // sheet cannot fill enough to matter, and this keeps the colour of a
        // stroke a property of the frame it was drawn on.
        let stage = (self.lit * STAGES / self.target).min(STAGES - 1) as u8;
        for _ in 0..self.per_frame {
            for k in 0..self.pens.len() {
                let (sx, sy) = self.advance(k);
                self.stamp(sx, sy, k as u8 * STAGES as u8 + stage);
            }
        }
        self.age += 1;
        if self.lit >= self.target || self.age >= self.max_frames {
            self.fade = Some(0);
        }
    }

    /// A blank sheet. The dots go with it, so no stroke of the old doodle can
    /// survive into the new one's colours.
    fn restart(&mut self) {
        self.dots.fill(0);
        self.tint.fill(0);
        self.lit = 0;
        self.age = 0;
        self.dim = 0;
        self.fade = None;
        for k in 0..self.pens.len() {
            let seed = next_rand(&mut self.rng);
            self.pens[k] = Self::fresh_pen(self.subw, self.subh, seed);
        }
    }
}

impl Saver for Doodles {
    fn render(&mut self, s: &mut Surface<'_>) {
        self.tick();
        let (grid, dots, tint, cols, dim) = (
            &mut self.grid,
            &self.dots[..],
            &self.tint[..],
            self.cols,
            self.dim,
        );
        grid.fill(|cx, cy| {
            let i = cy * cols + cx;
            let d = dots[i];
            if d == 0 {
                return Cell::CLEAR;
            }
            Cell::new(
                font::BRAILLE[d as usize],
                (1 + tint[i] as usize * FADE + dim) as u16,
            )
        });
        grid.flush(s, &PAL);
    }

    fn name(&self) -> &'static str {
        "doodles"
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
    use crate::dump;
    use crate::saver;
    use crate::testalloc::count as allocs;

    /// 1080 is not a multiple of 16: 67 rows cover 1072 and the bottom 8
    /// scanlines belong to no cell. That strip is the point of the frame-0
    /// assertion.
    fn panel() -> Panel {
        Panel::new(1920, 1080, 1920)
    }

    /// The panel change in flight. A saver that only looks right at 1080 is a
    /// saver that breaks the day the mode does.
    fn small() -> Panel {
        Panel::new(1280, 400, 1280)
    }

    /// T1. Frame 0 must cover the panel, including the strip below the last
    /// cell row, and must cover it with the SCENE — a reported black rectangle
    /// satisfies the rows assertion on its own.
    #[test]
    fn frame_zero_paints_the_whole_panel() {
        for p in [panel(), small()] {
            let mut c = Doodles::new(&p, 15);
            let mut buf = vec![0u32; p.buf_len()];
            let d = saver::frame(&mut c, &mut buf, &p);
            assert_eq!(d.rows(), p.h, "frame 0 must paint the whole panel");
            let lit = buf.iter().filter(|&&px| px != 0).count();
            assert!(lit > 1000, "frame 0 painted nothing ({lit} lit)");
        }
        assert!(
            !panel().h.is_multiple_of(16),
            "the 1080 panel divides evenly: the margin strip is untested"
        );
    }

    /// T2. Every scanline whose pixels changed is inside a damage run —
    /// otherwise simpledrm scans out the previous frame there forever. The
    /// companion assertions keep this from being a bound nothing approaches.
    #[test]
    fn damage_covers_every_changed_scanline() {
        let p = panel();
        let mut c = Doodles::new(&p, 15);
        let mut buf = vec![0u32; p.buf_len()];
        let mut prev = vec![0u32; p.buf_len()];
        let stride = p.buf_len() / p.h;
        saver::frame(&mut c, &mut buf, &p);

        let mut total = 0usize;
        const FRAMES: usize = 300;
        for n in 1..FRAMES {
            prev.copy_from_slice(&buf);
            let d = saver::frame(&mut c, &mut buf, &p);
            for y in 0..p.h {
                let row = y * stride..y * stride + p.w;
                if buf[row.clone()] != prev[row] {
                    assert!(
                        dump::row_reported(
                            &prev[y * stride..][..p.w],
                            &buf[y * stride..][..p.w],
                            y,
                            &d
                        ),
                        "frame {n}: scanline {y} changed outside every reported rect"
                    );
                }
            }
            total += d.rows();
        }
        assert!(
            total > 0,
            "nothing moved: the coverage check proves nothing"
        );
    }

    /// T3. `render` cannot allocate: every buffer is sized in `new` and the
    /// frame path only indexes them. Counted for real, so a `Vec` built inside
    /// `render` shows up whatever its capacity does.
    #[test]
    fn render_never_allocates() {
        let p = Panel::new(320, 200, 320);
        let mut c = Doodles::new(&p, 15);
        let mut buf = vec![0u32; p.buf_len()];
        saver::frame(&mut c, &mut buf, &p);

        // Non-vacuous: the counter must be able to see an allocation at all.
        let before = allocs();
        std::hint::black_box(vec![0u8; 64]);
        assert!(allocs() > before, "the counting allocator counted nothing");

        // Long enough to cross several sheets, so the fade and the restart —
        // which zero the buffers — are inside the window too.
        let before = allocs();
        for _ in 0..40_000 {
            saver::frame(&mut c, &mut buf, &p);
        }
        assert_eq!(allocs(), before, "the render path allocated");
        assert!(c.lit > 0, "40k frames drew nothing");
    }

    /// T4. The pen covers ground, stays on the paper, and never settles.
    /// A heading random walk with too little inertia is noise and with too much
    /// is a circle retracing itself; both are visible here as coverage that
    /// stalls.
    #[test]
    fn the_pen_wanders_the_whole_panel_without_leaving_it() {
        for p in [panel(), small()] {
            let mut c = Doodles::new(&p, 15);
            let cells = c.grid.cols() * c.grid.rows();
            // No fade, no restart: this is about ONE pen over a long run.
            c.pens.truncate(1);
            c.target = cells;
            c.max_frames = u32::MAX;

            let (mut lo_x, mut hi_x) = (f32::MAX, f32::MIN);
            let (mut lo_y, mut hi_y) = (f32::MAX, f32::MIN);
            let mut sign_flips = 0usize;
            let mut last_w = 0.0f32;
            for _ in 0..4000 {
                c.tick();
                let pen = &c.pens[0];
                assert!(
                    pen.x >= 0.0 && pen.x <= c.subw - 1.0,
                    "pen left the panel at x={}",
                    pen.x
                );
                assert!(
                    pen.y >= 0.0 && pen.y <= c.subh - 1.0,
                    "pen left the panel at y={}",
                    pen.y
                );
                lo_x = lo_x.min(pen.x);
                hi_x = hi_x.max(pen.x);
                lo_y = lo_y.min(pen.y);
                hi_y = hi_y.max(pen.y);
                if pen.w * last_w < 0.0 {
                    sign_flips += 1;
                }
                last_w = pen.w;
            }
            let case = format!("{}x{}", p.w, p.h);
            // A circle sits in a disc: it covers neither the width nor the
            // height, and its turn rate never changes sign.
            assert!(
                hi_x - lo_x > c.subw * 0.8 && hi_y - lo_y > c.subh * 0.8,
                "{case}: the pen stayed in a {}x{} box of {}x{}",
                hi_x - lo_x,
                hi_y - lo_y,
                c.subw,
                c.subh
            );
            assert!(
                sign_flips > 100,
                "{case}: the turn rate changed sign {sign_flips} times: a fixed cycle"
            );
            // And it actually inked a meaningful share of the sheet.
            assert!(
                c.lit * 5 > cells,
                "{case}: one pen covered {}/{cells} cells",
                c.lit
            );
        }
    }

    /// T5. The line is CONTINUOUS: consecutive steps land in the same or an
    /// adjacent sub-cell. Raise `STEP` past a sub-cell and the doodle becomes a
    /// dotted spray that still looks vaguely hand-drawn in a still frame.
    #[test]
    fn the_line_never_breaks() {
        let p = panel();
        let mut c = Doodles::new(&p, 15);
        let mut last = {
            let pen = &c.pens[0];
            (pen.x as i64, pen.y as i64)
        };
        let mut moved = 0usize;
        for _ in 0..20_000 {
            let (sx, sy) = c.advance(0);
            let jump = (sx as i64 - last.0).abs().max(sy as i64 - last.1);
            assert!(jump <= 1, "the pen jumped {jump} sub-cells: a broken line");
            if jump > 0 {
                moved += 1;
            }
            last = (sx as i64, sy as i64);
        }
        assert!(
            moved > 5000,
            "the pen barely moved: the bound proves nothing"
        );
    }

    /// T6. A sheet ends and the next one starts from blank paper. The bug this
    /// forecloses is lissajous's: strokes of a dead pass surviving in a cell,
    /// drawn in the new pass's colour, lying across the new line instead of
    /// along it.
    #[test]
    fn a_finished_doodle_is_gone_before_the_next_one_starts() {
        let p = Panel::new(320, 200, 320);
        let mut c = Doodles::new(&p, 15);
        c.target = c.lit.max(1); // full as of now
        c.tick(); // notices, and starts the fade
        assert!(c.fade.is_some(), "a full sheet did not start fading");

        // Bounded: a fade that never reaches the last step would otherwise hang
        // the suite rather than fail it.
        let mut dims = vec![c.dim];
        for _ in 0..c.fade_frames * 4 {
            if c.fade.is_none() {
                break;
            }
            c.tick();
            if c.fade.is_some() {
                dims.push(c.dim);
            }
        }
        assert!(
            dims.windows(2).all(|w| w[1] >= w[0]) && dims.contains(&(FADE - 1)),
            "the scribble did not fade through every step: {dims:?}"
        );
        assert!(c.fade.is_none(), "the fade never ended");
        assert_eq!(c.lit, 0, "the new sheet started with ink on it");
        assert!(
            c.dots.iter().all(|&d| d == 0),
            "a stroke outlived its sheet"
        );
        assert_eq!(c.dim, 0, "the new sheet started dim");
    }

    /// How long a sheet takes to fill, at both panel sizes. A measurement, not
    /// a check: the pen speed and `DOODLES_FILL_PCT` are chosen against it.
    #[test]
    #[ignore = "a measurement, not a check"]
    fn fill_time() {
        for p in [panel(), small()] {
            let mut c = Doodles::new(&p, 15);
            let cells = c.grid.cols() * c.grid.rows();
            c.restart();
            let mut f = 0;
            while c.fade.is_none() {
                c.tick();
                f += 1;
            }
            println!(
                "{}x{}: {cells} cells, {} steps/frame, full at {} lit after {f} frames = {:.1}s",
                p.w,
                p.h,
                c.per_frame,
                c.target,
                f as f32 / 15.0
            );
        }
    }

    /// T7. Colour tracks how full the sheet is, which is the whole "the
    /// scribble shows its own history" claim. A stage that never advances is a
    /// monochrome doodle that still passes every other test here.
    #[test]
    fn the_hue_walks_across_the_doodle() {
        let p = small();
        let mut c = Doodles::new(&p, 15);
        c.max_frames = u32::MAX;
        c.restart();
        // Read the stage off the INK, not off a formula copied from `tick` — a
        // test that recomputes what the code should have done passes happily
        // while the code stamps stage 0 on everything.
        let mut seen = [false; STAGES];
        for _ in 0..100_000 {
            if c.fade.is_some() {
                break;
            }
            c.tick();
            for (t, &d) in c.tint.iter().zip(c.dots.iter()) {
                if d != 0 {
                    seen[*t as usize % STAGES] = true;
                }
            }
        }
        assert!(
            seen.iter().all(|&s| s),
            "the doodle never passed through every colour stage: {seen:?}"
        );
    }

    /// T8. The pen has MOMENTUM, and not too much of it. Coverage tests cannot
    /// see either failure: a pen whose turn rate is pinned at the clamp draws
    /// circles that still drift across the panel, and one with no momentum at
    /// all draws near-straight lines that cover it just as well. Both are
    /// wrong, and this is the property that separates them — the turn rate is
    /// correlated with its own past, and rarely saturated.
    #[test]
    fn the_turn_rate_has_momentum_without_saturating() {
        let p = panel();
        let mut c = Doodles::new(&p, 15);
        let (mut num, mut den, mut pinned) = (0.0f64, 0.0f64, 0usize);
        let mut last = c.pens[0].w as f64;
        const N: usize = 40_000;
        for _ in 0..N {
            c.advance(0);
            let w = c.pens[0].w as f64;
            num += w * last;
            den += last * last;
            if w.abs() >= c.curl as f64 * 0.98 {
                pinned += 1;
            }
            last = w;
        }
        let corr = num / den;
        assert!(
            (0.5..0.995).contains(&corr),
            "turn-rate correlation {corr:.4}: below 0.5 is jitter with no \
             momentum, above 0.995 is a curve that never comes out of its arc"
        );
        assert!(
            pinned * 5 < N,
            "the turn rate sat at the clamp for {pinned}/{N} steps: a circle, \
             not a doodle"
        );
    }

    /// T9. A lit cell is drawn in ITS pen's colour at ITS stage. Checked
    /// through the palette entry rather than the index, because the index
    /// arithmetic is the thing under test: shift it by one and every stroke
    /// comes out a shade off, which no other test here can see.
    #[test]
    fn a_cell_is_drawn_in_its_pens_colour() {
        let p = small();
        let mut c = Doodles::new(&p, 15);
        let mut buf = vec![0u32; p.buf_len()];
        saver::frame(&mut c, &mut buf, &p);

        let mut checked = 0;
        let mut pens = [false; MAX_PENS];
        for i in 0..c.dots.len() {
            if c.dots[i] == 0 {
                continue;
            }
            let (pen, stage) = (c.tint[i] as usize / STAGES, c.tint[i] as usize % STAGES);
            // Derived from FAMILY here, independently of `ramp`'s indexing.
            let want: Vec<u32> = (0..3)
                .map(|ch| {
                    let (a, b) = (FAMILY[pen][0][ch] as u32, FAMILY[pen][1][ch] as u32);
                    (a * (STAGES - 1 - stage) as u32 + b * stage as u32) / (STAGES - 1) as u32
                })
                .collect();
            let got = PAL[c.grid.cells()[i].colour()];
            let want = (want[0] << 16) | (want[1] << 8) | want[2];
            assert_eq!(
                got, want,
                "cell {i} (pen {pen}, stage {stage}) drawn as {got:06x}, wanted {want:06x}"
            );
            checked += 1;
            pens[pen] = true;
        }
        assert!(checked > 50, "only {checked} cells were lit to check");
        // The braille dots ACCUMULATE within a cell: a pen crossing a cell
        // passes through two to four of its sub-cells, and keeping only the
        // last one thins the line to a dotted trail at eight times the cell
        // resolution — which still looks like a line in a thumbnail.
        assert!(
            c.dots.iter().any(|d| d.count_ones() > 1),
            "no cell holds more than one dot: the line is a dotted trail"
        );
        // And the pens are told apart. Stamping every stroke with pen 0's
        // colour passes every assertion above — and paints a three-pen doodle
        // in one hue.
        assert!(
            pens.iter().filter(|&&p| p).count() >= 3,
            "three pens drew but only {} colour families appear: {pens:?}",
            pens.iter().filter(|&&p| p).count()
        );
    }

    /// T10. The hand moves at the same speed whatever the frame rate — the pod
    /// runs at 15fps and the dump defaults to 30, and a doodle that drew twice
    /// as fast in one of them would be tuned against the wrong one.
    #[test]
    fn the_pen_draws_at_the_same_rate_at_any_fps() {
        let p = panel();
        let per_second = |fps| Doodles::new(&p, fps).per_frame * fps;
        let base = per_second(15);
        for fps in [1, 10, 30, 60, 120] {
            let got = per_second(fps);
            // Steps per frame is a small integer, so the achievable rate is
            // quantised: allow the half-step either way that rounding leaves,
            // and nothing more.
            assert!(
                got.abs_diff(base) <= fps,
                "at {fps}fps the pen takes {got} steps a second, {base} at 15"
            );
        }
    }

    /// Every colour index a cell can address must exist: one off the end is an
    /// index panic inside `Grid::blit`, on the panel, weeks in.
    #[test]
    fn every_reachable_colour_index_is_in_the_palette() {
        let brightest = 1 + ((MAX_PENS - 1) * STAGES + STAGES - 1) * FADE + FADE - 1;
        assert_eq!(brightest, PAL_LEN - 1);
        for k in 0..MAX_PENS {
            // Full brightness, both ends of every family, is not black.
            assert_ne!(PAL[1 + k * STAGES * FADE], 0, "pen {k} starts black");
            assert_ne!(PAL[1 + (k * STAGES + STAGES - 1) * FADE], 0, "pen {k} ends");
        }
    }
}
