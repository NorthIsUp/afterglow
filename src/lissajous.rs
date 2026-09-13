//! Lissajous — a point tracing `x = sin(a·t + d)`, `y = sin(b·t)`, leaving a
//! trail that fades behind it, with `b` and `d` drifting so the figure morphs
//! through its family instead of settling on one shape.
//!
//! # Why the phases are integrated, not multiplied
//!
//! The textbook form evaluates `sin(b·t)` with `t` growing without bound. Drift
//! `b` while `t` is large and the whole figure whips around, because the error
//! is `t·Δb`. This keeps a phase accumulator per axis and advances it by
//! `b·dt`, so a change to `b` bends the curve from where the pen is rather than
//! teleporting it. The visible difference is the whole effect.
//!
//! # Damage model: full repaint (Model A)
//!
//! Sparse looks tempting — the pen touches a few hundred cells a frame — but
//! the trail FADES, so every lit cell changes colour on the frame it steps down
//! a brightness level. "What changed" is most of the trail, every frame, and is
//! no cheaper to know than to recompute. `Grid::flush` derives it from a u32
//! compare per cell and cannot under-report.
//!
//! The per-cell work is two array reads and a shift — no trig. The sines are
//! per SAMPLE along the curve (a few hundred a frame), not per cell.
//!
//! # Sub-cell geometry
//!
//! Cells are stamped as braille patterns: 2x4 dots per cell, so the curve is
//! drawn at four times the vertical and twice the horizontal cell resolution
//! while still costing one `Cell` per cell. One colour per cell means a cell's
//! dots all share the newest pen's hue and brightness, which is invisible at an
//! 8x16 cell and is what keeps this a grid saver.

use crate::font;
use crate::grid::{bake, dot_bit, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, next_rand};

/// Brightness steps in one curve's ramp. Eight is where a 2.5s fade stops
/// showing banding at 15fps and still keeps the palette under 32 entries.
const LEVELS: usize = 8;
const MAX_CURVES: usize = 3;

/// Total curve samples the constructor may pre-draw. Each one is two sines, so
/// this is the constructor's whole cost: ~8k samples is ~170us here and under a
/// millisecond on the Pi, against 13ms for an uncapped pre-draw at the top of
/// the env ranges.
const PREWARM_SAMPLES: u32 = 8_000;

/// Full brightness in fixed point: the top of `LEVELS` 256-wide buckets, so a
/// cell's level is `heat >> 8` with no divide.
const HEAT_MAX: u16 = (LEVELS as u16) * 256 - 1;

/// One hue per curve. Cyan / magenta / amber: three families far enough apart
/// in hue that two curves crossing read as two curves, not as one brighter one.
#[rustfmt::skip]
const HUES: [[u8; 3]; MAX_CURVES] = [
    [0x46, 0xE8, 0xFF],
    [0xFF, 0x5C, 0xD0],
    [0xFF, 0xC0, 0x4A],
];

/// Index 0 is black, then `LEVELS` entries per curve. Separate ranges rather
/// than one shared ramp, so a cell cannot borrow another curve's colour by
/// drifting one index.
const PAL_LEN: usize = 1 + MAX_CURVES * LEVELS;

/// `(l+1)(l+2) / LEVELS(LEVELS+1)` — a quadratic ramp, because a linear one
/// spends half its steps in the range where an 8x16 braille dot is already too
/// dim to see, and the tail of the trail then vanishes in one step instead of
/// fading.
const fn ramp() -> [[u8; 3]; PAL_LEN] {
    let mut out = [[0u8; 3]; PAL_LEN];
    let den = (LEVELS * (LEVELS + 1)) as u32;
    let mut k = 0;
    while k < MAX_CURVES {
        let mut l = 0;
        while l < LEVELS {
            let num = ((l + 1) * (l + 2)) as u32;
            let mut c = 0;
            while c < 3 {
                out[1 + k * LEVELS + l][c] = (HUES[k][c] as u32 * num / den) as u8;
                c += 1;
            }
            l += 1;
        }
        k += 1;
    }
    out
}

const PAL: [u32; PAL_LEN] = bake(&ramp());

struct Pen {
    /// Integrated phase of each axis. See the module doc.
    px: f32,
    py: f32,
    /// Constant phase offset between curves, so pens of the same family draw
    /// different members of it.
    offset: f32,
}

pub struct Lissajous {
    grid: Grid,
    cols: usize,
    rows: usize,
    /// Fixed-point brightness per cell, `>> 8` to a palette level.
    heat: Vec<u16>,
    /// Braille pattern accumulated in the cell while it has been lit.
    dots: Vec<u8>,
    /// Which curve last touched the cell.
    hue: Vec<u8>,
    pens: Vec<Pen>,
    decay: u16,
    /// The heat a cell stamped on the PREVIOUS frame has, after this frame's
    /// decay. Below it, the cell's dots belong to an older pass — see `stamp`.
    /// At least 1, so a dead cell still clears when one frame outlives the fade.
    fresh: u16,
    /// Samples along each curve per frame. The trail is only continuous
    /// because consecutive samples land within a sub-cell of each other.
    samples: u32,
    /// Radians per sample: base frequency, ratio morph, phase spin.
    d_phase: f32,
    d_morph: f32,
    d_offset: f32,
    morph: f32,
    ratio_lo: f32,
    ratio_hi: f32,
}

impl Lissajous {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        let cell_w = env_num(&["LISSAJOUS_CELL_W"], 8, 4, 64) as usize;
        let cell_h = env_num(&["LISSAJOUS_CELL_H"], 16, 8, 128) as usize;
        let curves = env_num(&["LISSAJOUS_CURVES"], 3, 1, MAX_CURVES as i64) as usize;
        let fade_ms = env_num(&["LISSAJOUS_FADE_MS"], 9000, 200, 20_000) as u32;
        // Milli-radians per second, because the interesting range for the base
        // frequency is well under 1 rad/s and a knob in whole radians would
        // have two usable values.
        let speed = env_num(&["LISSAJOUS_SPEED"], 900, 10, 5000) as f32 / 1000.0;
        let morph_s = env_num(&["LISSAJOUS_MORPH_S"], 95, 5, 3600) as f32;
        let phase_s = env_num(&["LISSAJOUS_PHASE_S"], 26, 2, 3600) as f32;
        let samples = env_num(&["LISSAJOUS_SAMPLES"], 3200, 100, 40_000) as u32;
        let ratio_lo = env_num(&["LISSAJOUS_RATIO_LO"], 100, 50, 1200) as f32 / 100.0;
        let ratio_hi = env_num(&["LISSAJOUS_RATIO_HI"], 500, 50, 1200) as f32 / 100.0;

        let grid = Grid::new(panel, cell_w, cell_h);
        let (cols, rows) = (grid.cols(), grid.rows());
        let fps = fps.max(1);

        // Rates are per SECOND and divided by fps here, so a slower panel takes
        // more samples per frame and the figure moves at the same speed.
        let per_sample = 1.0 / samples as f32;

        let mut rng = 0x5EED_1A55u32;
        let pens = (0..curves)
            .map(|k| Pen {
                // A small random head start, so pens never sit exactly on top
                // of each other at t=0 and the first seconds are not one curve
                // drawn three times.
                px: (next_rand(&mut rng) % 1000) as f32 / 1000.0,
                py: (next_rand(&mut rng) % 1000) as f32 / 1000.0,
                offset: k as f32 * std::f32::consts::TAU / curves as f32,
            })
            .collect();

        let decay = (HEAT_MAX as u32 * 1000 / (fade_ms * fps).max(1)).max(1) as u16;

        let mut me = Self {
            grid,
            cols,
            rows,
            heat: vec![0; cols * rows],
            dots: vec![0; cols * rows],
            hue: vec![0; cols * rows],
            pens,
            decay,
            fresh: HEAT_MAX.saturating_sub(decay).max(1),
            samples: (samples / fps).max(1),
            d_phase: speed * per_sample,
            d_morph: std::f32::consts::TAU / morph_s * per_sample,
            d_offset: std::f32::consts::TAU / phase_s * per_sample,
            morph: 0.0,
            ratio_lo: ratio_lo.min(ratio_hi),
            ratio_hi: ratio_lo.max(ratio_hi),
        };

        // Draw the trail the pen would already have left. Without this the
        // panel opens on a single moving dot and takes a full fade to look
        // like anything.
        //
        // Capped at a sample budget rather than run for the whole fade. This
        // runs on the RENDER thread every time the mirror switches saver, and
        // a full fade at the top of the env ranges is 2.4M sines — 13ms on a
        // laptop, well past a 15fps frame on the Pi. A short pre-draw opens on
        // a short trail that grows to full length within one fade; a stall
        // drops frames on whatever the viewer switched away from.
        let frames = (fade_ms * fps / 1000)
            .min(PREWARM_SAMPLES / me.samples.max(1))
            .max(1);
        for _ in 0..frames {
            me.step();
        }
        me
    }

    /// Light the cell containing sub-cell `(sx, sy)`. A cell the pen was not
    /// already sitting in drops whatever dots it had, because `heat` and `hue`
    /// are about to become this pass's — so any dot kept from the last pass is
    /// drawn in this pass's colour and dies on this pass's schedule, stranded
    /// on a curve it never belonged to. That is the tick marks: a cell shared
    /// with an older, nearly-faded pass keeps that pass's dots, which lie
    /// across the new curve instead of along it. Worst where two passes run
    /// close and near-parallel, and at a cusp, where the curve doubles back
    /// into its own cells and the union fills to a blob.
    #[inline]
    fn stamp(&mut self, sx: usize, sy: usize, k: u8) {
        let i = (sy / 4) * self.cols + (sx / 2);
        if self.heat[i] < self.fresh {
            self.dots[i] = 0;
        }
        self.dots[i] |= dot_bit(sx & 1, sy & 3);
        self.hue[i] = k;
        self.heat[i] = HEAT_MAX;
    }

    fn step(&mut self) {
        for h in self.heat.iter_mut() {
            *h = h.saturating_sub(self.decay);
        }

        let (subw, subh) = (self.cols * 2, self.rows * 4);
        // Half-extents in sub-cells, with the outermost sub-cell reserved so a
        // pen at exactly +/-1 cannot index off the edge.
        let (ax, ay) = ((subw - 1) as f32 / 2.0, (subh - 1) as f32 / 2.0);

        for s in 0..self.samples {
            self.morph += self.d_morph;
            // 0..1 with zero derivative at both ends, so the ratio lingers on
            // the simple integer relationships instead of sweeping past them.
            let u = 0.5 - 0.5 * self.morph.cos();
            let b = self.ratio_lo + (self.ratio_hi - self.ratio_lo) * u;
            let spin = self.d_offset * s as f32;

            for k in 0..self.pens.len() {
                let p = &mut self.pens[k];
                p.px += self.d_phase;
                p.py += self.d_phase * b;
                let (px, py) = (p.px + p.offset + spin, p.py);
                let sx = ((px.sin() + 1.0) * ax) as usize;
                let sy = ((py.sin() + 1.0) * ay) as usize;
                self.stamp(sx, sy, k as u8);
            }
        }
        // Keep the accumulators small: f32 loses phase resolution as the
        // exponent grows, and this runs for months.
        self.morph %= std::f32::consts::TAU;
        let spun = self.d_offset * self.samples as f32;
        for p in self.pens.iter_mut() {
            p.px %= std::f32::consts::TAU;
            p.py %= std::f32::consts::TAU;
            p.offset = (p.offset + spun) % std::f32::consts::TAU;
        }
    }
}

impl Saver for Lissajous {
    fn render(&mut self, s: &mut Surface<'_>) {
        self.step();
        let (grid, heat, dots, hue, cols) = (
            &mut self.grid,
            &self.heat[..],
            &self.dots[..],
            &self.hue[..],
            self.cols,
        );
        grid.fill(|cx, cy| {
            let i = cy * cols + cx;
            let h = heat[i];
            if h == 0 {
                return Cell::CLEAR;
            }
            let level = (h >> 8) as usize;
            Cell::new(
                font::BRAILLE[dots[i] as usize],
                (1 + hue[i] as usize * LEVELS + level) as u16,
            )
        });
        grid.flush(s, &PAL);
    }

    fn name(&self) -> &'static str {
        "lissajous"
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

    /// 1080 is not a multiple of 16: 67 rows cover 1072 and the bottom 8
    /// scanlines belong to no cell. That strip is the point of the frame-0
    /// assertion.
    fn panel() -> Panel {
        Panel::new(1920, 1080, 1920)
    }

    /// T1. Frame 0 must cover the panel including the strip below the last cell
    /// row, and must cover it with the SCENE — a reported black rectangle
    /// satisfies the rows assertion on its own.
    #[test]
    fn frame_zero_paints_the_whole_panel() {
        let p = panel();
        let mut c = Lissajous::new(&p, 15);
        assert!(
            !p.h.is_multiple_of(c.grid.cell_h()),
            "test panel divides evenly"
        );
        let mut buf = vec![0u32; p.buf_len()];

        let d = saver::frame(&mut c, &mut buf, &p);
        assert_eq!(d.rows(), p.h, "frame 0 must paint the whole panel");

        let lit = buf.iter().filter(|&&px| px != 0).count();
        assert!(lit > 10_000, "frame 0 painted nothing ({lit} lit)");
    }

    /// T2. Every scanline whose pixels changed is inside a damage run —
    /// otherwise simpledrm scans out the previous frame there forever.
    ///
    /// The frame count is the load-bearing part. This ran 200 frames and
    /// asserted damage stayed UNDER a full repaint, which was true only inside
    /// that window: the trail keeps growing, the first full repaint lands
    /// around frame 663, and after that a frame damages 430-1072 rows with a
    /// mean near 900. A bound that holds because the loop stops early is worse
    /// than no bound. So the run reaches steady state, and the companions state
    /// what steady state is — Model A, most of the panel, most frames.
    #[test]
    fn damage_covers_every_changed_scanline() {
        let p = panel();
        let mut c = Lissajous::new(&p, 15);
        let mut buf = vec![0u32; p.buf_len()];
        let mut prev = vec![0u32; p.buf_len()];
        let stride = p.buf_len() / p.h;
        saver::frame(&mut c, &mut buf, &p);

        let mut worst = 0usize;
        let mut total = 0usize;
        const FRAMES: usize = 1200;
        for n in 1..FRAMES {
            prev.copy_from_slice(&buf);
            let d = saver::frame(&mut c, &mut buf, &p);
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
        let mean = total / (FRAMES - 1);
        let full = c.grid.rows() * c.grid.cell_h();
        // Model A, stated as an assertion rather than only in the module doc:
        // the fade repaints the whole trail, so a full repaint is REACHED, and
        // the typical frame is most of the panel. Measured here: full is first
        // reached around frame 663, mean ~900 of 1072 rows. A change that made this
        // genuinely sparse would fail both, and would be right to — but then
        // the doc has to move too.
        assert_eq!(worst, full, "a full repaint is never reached: {worst} rows");
        assert!(
            mean * 2 > full,
            "the typical frame damages {mean} of {full} rows — this is sparse \
             now, and the module doc says it is not"
        );
    }

    /// T6. The fade rate IS the effect: it sets how long the trail is, and
    /// `LISSAJOUS_FADE_MS` is also the knob for cutting the saver's cost. A cell
    /// lit to full must go dark in about the configured time and no other.
    ///
    /// Stated on `step`, not on `decay`, because the bug this catches is in the
    /// per-frame subtraction — a `saturating_sub(1)` there is a 136-second
    /// trail that saturates the panel, and every other test in this file passes
    /// with it.
    #[test]
    fn a_cell_fades_out_in_the_configured_time() {
        // The defaults, restated: `new` reads them from env and the test cannot
        // set env without racing every other test in the binary.
        const FADE_MS: usize = 9_000;
        const FPS: u32 = 15;
        let p = panel();
        let mut c = Lissajous::new(&p, FPS);
        // Every cell at full: the pen re-lights a few hundred a frame, so the
        // MINIMUM is a cell it has not touched since, decaying untouched.
        for h in c.heat.iter_mut() {
            *h = HEAT_MAX;
        }
        let mut frames = 0usize;
        while c.heat.iter().copied().min().unwrap() > 0 {
            c.step();
            frames += 1;
            assert!(frames < 10_000, "nothing ever faded out");
        }
        let want = FADE_MS * FPS as usize / 1000;
        // +/-15%: the decay is an integer per frame (15 of 2047 at the
        // defaults), so the achievable time is quantised, and rounding costs
        // ~1.5% here. Anything looser stops pinning the trail length.
        assert!(
            frames * 100 >= want * 85 && frames * 100 <= want * 115,
            "a full cell took {frames} frames to fade, not ~{want}: the trail \
             is {:.1}x the configured {FADE_MS}ms",
            frames as f64 / want as f64
        );
    }

    /// T3. `render` cannot allocate: every buffer is sized in `new` and the
    /// frame path only indexes them. A Vec that never grows past its reserve
    /// never reallocates, so unchanged capacity IS "did not allocate".
    #[test]
    fn render_never_allocates() {
        let p = Panel::new(320, 200, 320);
        let mut c = Lissajous::new(&p, 15);
        let mut buf = vec![0u32; p.buf_len()];
        let shape = |c: &Lissajous| {
            (
                c.heat.len(),
                c.heat.capacity(),
                c.dots.len(),
                c.dots.capacity(),
                c.hue.len(),
                c.hue.capacity(),
                c.pens.len(),
                c.pens.capacity(),
            )
        };
        let reserved = shape(&c);
        assert!(reserved.1 > 0, "nothing was reserved for the frame loop");
        // Sized exactly in `new`, with no slack. Without this the constructor
        // could absorb a one-off growth from the frame path — it pre-draws a
        // trail through the same `step` the loop calls — and the capacities
        // below would then never move again.
        assert_eq!(
            (reserved.0, reserved.2, reserved.4),
            (reserved.1, reserved.3, reserved.5),
            "a frame buffer is over-allocated: growth in `step` would hide here"
        );
        for _ in 0..50_000 {
            saver::frame(&mut c, &mut buf, &p);
            assert_eq!(shape(&c), reserved, "the render path allocated");
        }
        // Non-vacuous: the buffers are in use, not empty vecs whose capacity
        // trivially never moves.
        assert!(
            c.heat.iter().any(|&h| h > 0),
            "50k frames drew nothing into the reserved buffers"
        );
    }

    /// T4. The trail is a CURVE: one frame of samples moves the pen at most one
    /// sub-cell per sample. Raise the speed or drop the per-sample stepping for
    /// a per-frame one and the figure becomes a dotted spray that still looks
    /// vaguely Lissajous in a still frame.
    #[test]
    fn the_pen_draws_a_continuous_curve() {
        let p = panel();
        let mut c = Lissajous::new(&p, 15);
        let (subw, subh) = (c.cols * 2, c.rows * 4);
        let (ax, ay) = ((subw - 1) as f32 / 2.0, (subh - 1) as f32 / 2.0);
        let mut last: Vec<Option<(i64, i64)>> = vec![None; c.pens.len()];
        let mut moved = 0usize;

        for _ in 0..600 {
            c.step();
            for (k, pen) in c.pens.iter().enumerate() {
                let sx = (((pen.px + pen.offset).sin() + 1.0) * ax) as i64;
                let sy = ((pen.py.sin() + 1.0) * ay) as i64;
                if let Some((lx, ly)) = last[k] {
                    // A frame is `samples` steps apart, so bound the frame's
                    // travel, not one sample's.
                    let jump = (sx - lx).abs().max((sy - ly).abs());
                    assert!(
                        jump <= c.samples as i64,
                        "pen {k} jumped {jump} sub-cells in one frame of {} \
                         samples: the trail is not continuous",
                        c.samples
                    );
                    if jump > 0 {
                        moved += 1;
                    }
                }
                last[k] = Some((sx, sy));
            }
        }
        assert!(
            moved > 500,
            "the pen barely moved: the bound proves nothing"
        );
    }

    /// T5. A cell's braille dots belong to the pass that lit it. Let a cell fade
    /// to black and light one different dot, and the cell must show that dot
    /// ALONE — otherwise an arm of an old figure flashes back at full
    /// brightness when the pen wanders past, weeks in.
    #[test]
    fn a_relit_cell_forgets_the_pass_that_faded() {
        let p = Panel::new(320, 200, 320);
        let mut c = Lissajous::new(&p, 15);
        // The constructor pre-draws a trail; cell 0 may be part of it.
        c.heat[0] = 0;
        c.dots[0] = 0;
        c.stamp(0, 0, 0);
        c.stamp(1, 1, 0);
        assert_eq!(c.dots[0], dot_bit(0, 0) | dot_bit(1, 1));

        c.heat[0] = 0;
        c.stamp(0, 2, 1);
        assert_eq!(
            c.dots[0],
            dot_bit(0, 2),
            "a relit cell kept dots from a faded pass"
        );
        assert_eq!(c.hue[0], 1);
    }

    /// T6. The artifact: a pass that has faded must leave NOTHING in a cell a
    /// later pass relights. It cannot — the cell is about to carry the new
    /// pass's colour and the new pass's heat, so an inherited dot is drawn in
    /// the wrong hue, lies across the new curve instead of along it, and
    /// outlives the trail it belonged to. One frame of decay is one frame, so
    /// subtracting `decay` ages the cell exactly as `step` would.
    #[test]
    fn a_faded_pass_leaves_no_dots_in_a_cell_a_later_pass_relights() {
        let p = Panel::new(320, 200, 320);
        let mut c = Lissajous::new(&p, 15);
        c.heat[0] = 0;
        c.dots[0] = 0;

        // An older pass, going down the left column.
        c.stamp(0, 0, 0);
        c.stamp(0, 1, 0);
        // Two frames old: no longer the cell the pen is sitting in.
        c.heat[0] -= c.decay * 2;
        // A later pass crosses it.
        c.stamp(1, 2, 1);
        assert_eq!(
            c.dots[0],
            dot_bit(1, 2),
            "the relit cell kept a faded pass's dots: a tick across the new curve"
        );
        assert_eq!(c.heat[0], HEAT_MAX);
        assert_eq!(c.hue[0], 1);

        // The other side of the boundary, and the reason this is not simply
        // "always clear": a cell the pen is still crossing must accumulate, or
        // every curve breaks into dashes at the frame boundary.
        c.heat[0] -= c.decay;
        c.stamp(1, 3, 1);
        assert_eq!(
            c.dots[0],
            dot_bit(1, 2) | dot_bit(1, 3),
            "a cell relit on the very next frame lost the dots it just drew"
        );
    }

    /// Every colour index a cell can address must exist: one off the end is an
    /// index panic inside `Grid::blit`, on the panel, weeks in.
    #[test]
    fn every_reachable_colour_index_is_in_the_palette() {
        let brightest = 1 + (MAX_CURVES - 1) * LEVELS + (HEAT_MAX >> 8) as usize;
        assert_eq!(brightest, PAL_LEN - 1);
        assert_ne!(PAL[PAL_LEN - 1], 0, "the brightest level is black");
    }
}
