//! Strings — "String Theory", the After Dark module. A polygon whose vertices
//! each bounce around the panel on their own heading, redrawn every frame while
//! the polygons behind it fade, so the stack of outlines reads as one ribbon
//! sweeping and folding through space. Several independent ribbons, one hue
//! each.
//!
//! # Why the trail is a fading heat buffer and not a ring of polygons
//!
//! The classic draws N polygons and erases the oldest as it draws the newest.
//! That needs the old vertex positions kept, the old lines re-walked to erase,
//! and — because ribbons overlap — an erase that cannot simply write black.
//! A per-cell brightness that decays does all of it: a cell's age IS its
//! colour, an overlap is just the newer stamp winning, and nothing is ever
//! re-walked. The visible difference from the original is that the tail dims
//! continuously instead of vanishing in one step, which is the better look.
//!
//! # Damage model: full repaint (Model A)
//!
//! Sparse looks tempting — one polygon a frame touches a few thousand cells —
//! but the trail FADES, so every lit cell changes colour on the frame it steps
//! down a brightness level. "What changed" is most of the trail, every frame,
//! and is no cheaper to know than to recompute. `Grid::flush` derives it from a
//! u32 compare per cell and structurally cannot under-report; a hand-kept dirty
//! list can, and an unreported cell keeps its old pixels on the panel forever.
//! Same reasoning, same conclusion as `lissajous`.
//!
//! # Sub-cell geometry, and why nothing here knows the aspect ratio
//!
//! Cells are stamped as braille patterns: 2x4 dots per cell, so lines are drawn
//! at twice the horizontal and four times the vertical cell resolution while
//! still costing one `Cell` per cell. At the default 8x16 cell a sub-cell is a
//! square 4x4 pixels, so a heading in sub-cell space is a heading on the panel
//! and no direction is stretched.
//!
//! Speed is in SUB-CELLS per second, not in a fraction of the panel — and that
//! is the whole reason this looks right on both panels. What makes a ribbon
//! read as strings rather than as a solid sheet is the GAP between successive
//! outlines, which is speed-per-frame in sub-cells and nothing else. Scale the
//! speed to the panel and the 1280x400 one, with 2.7x fewer sub-cells down its
//! short edge, gets 2.7x less separation and fills in. A smaller panel is
//! crossed in less time instead, which is what a fixed pixel speed does and is
//! what it should do. Everything else is derived from `grid.cols()`/`rows()`,
//! so the only geometry assumption in the file is "the panel is a rectangle".
//!
//! # Knobs
//!
//! * `STRINGS_CELL_W` (4..64, default 8), `STRINGS_CELL_H` (8..128, default 16)
//! * `STRINGS_RIBBONS` (1..4, default 3) — independent polygons, one hue each
//! * `STRINGS_VERTICES` (2..8, default 4) — corners per polygon; 2 is the
//!   single bouncing line segment the module is named after
//! * `STRINGS_FADE_MS` (200..20000, default 1000) — how long a line takes to
//!   fade out, i.e. how many outlines deep the ribbon is
//! * `STRINGS_SPEED` (5..1000, default 60) — sub-cells per second per vertex.
//!   Raise it for wider gaps between the outlines, lower it for a denser sheet

use crate::font;
use crate::grid::{bake, dot_bit, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, next_rand};

/// Brightness steps in one ribbon's ramp. Eight is where a multi-second fade
/// stops showing banding at 15fps and still keeps the palette small.
const LEVELS: usize = 8;
const MAX_RIBBONS: usize = 4;
const MAX_VERTS: usize = 8;

/// Full brightness in fixed point: the top of `LEVELS` 256-wide buckets, so a
/// cell's level is `heat >> 8` with no divide.
const HEAT_MAX: u16 = (LEVELS as u16) * 256 - 1;

/// One hue per ribbon. Four families far enough apart that two ribbons
/// crossing read as two ribbons rather than as one brighter one.
#[rustfmt::skip]
const HUES: [[u8; 3]; MAX_RIBBONS] = [
    [0x5A, 0xC8, 0xFF],
    [0xFF, 0x6A, 0x9C],
    [0x9C, 0xFF, 0x6A],
    [0xFF, 0xB0, 0x40],
];

/// Index 0 is black, then `LEVELS` entries per ribbon. Separate ranges rather
/// than one shared ramp, so a cell cannot borrow another ribbon's colour by
/// drifting one index.
const PAL_LEN: usize = 1 + MAX_RIBBONS * LEVELS;

/// `(l+1)(l+2) / LEVELS(LEVELS+1)` — quadratic, because a linear ramp spends
/// half its steps in the range where a 4x4 braille dot is already too dim to
/// see and the tail of the ribbon then vanishes in one step instead of fading.
const fn ramp() -> [[u8; 3]; PAL_LEN] {
    let mut out = [[0u8; 3]; PAL_LEN];
    let den = (LEVELS * (LEVELS + 1)) as u32;
    let mut k = 0;
    while k < MAX_RIBBONS {
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

/// A polygon corner, in sub-cell coordinates. Each one bounces on its own
/// heading — that independence is the whole effect. Vertices moving together
/// draw a rigid shape sliding around, which is a different and much duller
/// screensaver.
struct Vertex {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
}

pub struct Strings {
    grid: Grid,
    cols: usize,
    /// Fixed-point brightness per cell, `>> 8` to a palette level.
    heat: Vec<u16>,
    /// Braille pattern accumulated in the cell while it has been lit.
    dots: Vec<u8>,
    /// Which ribbon last touched the cell.
    hue: Vec<u8>,
    /// All ribbons' vertices, flat, `per` of them each.
    verts: Vec<Vertex>,
    per: usize,
    /// Edges to walk per ribbon. Two vertices make ONE segment, not two
    /// coincident ones drawn back to back.
    edges: usize,
    decay: u16,
    /// The heat a cell stamped on the PREVIOUS frame has, after this frame's
    /// decay. Below it, the cell's dots belong to an older pass — see `stamp`.
    /// At least 1, so a dead cell still clears when one frame outlives the fade.
    fresh: u16,
    /// Sub-cell bounds, inclusive: the outermost sub-cell is reachable and
    /// nothing may go past it.
    max_x: f32,
    max_y: f32,
}

/// Palette entry for a lit cell. The `1 +` is load-bearing: index 0 is black
/// and no lit cell may land on it. Dropping it renders the dimmest step of
/// every ribbon invisible and hands each ribbon's brightest step to the next
/// ribbon's colour.
#[inline]
const fn colour(hue: u8, heat: u16) -> u16 {
    1 + hue as u16 * LEVELS as u16 + (heat >> 8)
}

/// Edges to walk for a polygon of `per` corners. Two vertices make ONE
/// segment: walking `per` edges there draws the same line twice, wasting half
/// the frame's stamps, and is the easy mistake in the `(e + 1) % per` below.
const fn edges_for(per: usize) -> usize {
    if per == 2 {
        1
    } else {
        per
    }
}

impl Strings {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        let cell_w = env_num(&["STRINGS_CELL_W"], 8, 4, 64) as usize;
        let cell_h = env_num(&["STRINGS_CELL_H"], 16, 8, 128) as usize;
        let ribbons = env_num(&["STRINGS_RIBBONS"], 3, 1, MAX_RIBBONS as i64) as usize;
        let per = env_num(&["STRINGS_VERTICES"], 4, 2, MAX_VERTS as i64) as usize;
        let fade_ms = env_num(&["STRINGS_FADE_MS"], 1000, 200, 20_000) as u32;
        let speed = env_num(&["STRINGS_SPEED"], 60, 5, 1000) as f32;

        let grid = Grid::new(panel, cell_w, cell_h);
        let (cols, rows) = (grid.cols(), grid.rows());
        let fps = fps.max(1);

        let (max_x, max_y) = ((cols * 2 - 1) as f32, (rows * 4 - 1) as f32);
        // Sub-cells per frame. Panel-independent on purpose: this number IS
        // the gap between consecutive outlines, and the gap is what makes a
        // ribbon read as strings instead of as a filled sheet.
        let step = speed / fps as f32;

        let mut rng = 0x571E_5000u32;
        let mut roll =
            |lo: f32, hi: f32| lo + (next_rand(&mut rng) % 1024) as f32 / 1024.0 * (hi - lo);
        let verts = (0..ribbons * per)
            .map(|_| {
                let a = roll(0.0, std::f32::consts::TAU);
                // 0.6..1.4 of the base speed, so a polygon's corners drift out
                // of phase with each other instead of orbiting in lockstep.
                let m = step * roll(0.6, 1.4);
                Vertex {
                    // Kept off the very edge, so the opening frames are a
                    // polygon rather than a shape flattened against a wall.
                    x: roll(0.1, 0.9) * max_x,
                    y: roll(0.1, 0.9) * max_y,
                    vx: a.cos() * m,
                    vy: a.sin() * m,
                }
            })
            .collect();

        let decay = (HEAT_MAX as u32 * 1000 / (fade_ms * fps).max(1)).max(1) as u16;

        let mut me = Self {
            grid,
            cols,
            heat: vec![0; cols * rows],
            dots: vec![0; cols * rows],
            hue: vec![0; cols * rows],
            verts,
            per,
            edges: edges_for(per),
            decay,
            fresh: HEAT_MAX.saturating_sub(decay).max(1),
            max_x,
            max_y,
        };

        // Draw the ribbon the polygons would already have left. Without this
        // the panel opens on a single outline and takes a full fade to look
        // like anything; the steady state is one fade's worth of frames, so
        // running exactly that costs a few milliseconds once.
        for _ in 0..(fade_ms * fps / 1000).max(1) {
            me.step();
        }
        me
    }

    /// Light the cell containing sub-cell `(sx, sy)`. A cell not lit on the
    /// previous frame drops whatever dots it had, because `heat` and `hue` are
    /// about to become this pass's — so any dot kept from an older pass is
    /// drawn in this pass's colour and dies on this pass's schedule, stranded
    /// on a line it never belonged to. Without the guard, a nearly-faded
    /// crossing flashes back at full brightness as a phantom tick across the
    /// new line. (`lissajous` shipped that bug; this is the same fix.)
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

    /// Bresenham between two sub-cells, inclusive of both ends. Integer, so a
    /// long edge cannot accumulate float error into a gap, and every sub-cell
    /// on the line is touched exactly once.
    fn line(&mut self, (x0, y0): (i32, i32), (x1, y1): (i32, i32), k: u8) {
        let (mut x, mut y) = (x0, y0);
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            self.stamp(x as usize, y as usize, k);
            if x == x1 && y == y1 {
                return;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// Advance and reflect one coordinate. Reflecting the OVERSHOOT rather than
    /// clamping keeps the speed exactly constant across a bounce; a clamp makes
    /// every wall a small pause, and at three vertices a second those add up to
    /// a visibly sticky edge.
    #[inline]
    fn bounce(p: &mut f32, v: &mut f32, hi: f32) {
        *p += *v;
        if *p < 0.0 {
            *p = -*p;
            *v = -*v;
        }
        if *p > hi {
            *p = 2.0 * hi - *p;
            *v = -*v;
        }
        // A vertex started outside, or a speed wider than the panel, would
        // still be out after one reflection. Costs nothing and is the only
        // thing between that and an out-of-bounds stamp.
        *p = p.clamp(0.0, hi);
    }

    fn step(&mut self) {
        for h in self.heat.iter_mut() {
            *h = h.saturating_sub(self.decay);
        }
        let (max_x, max_y) = (self.max_x, self.max_y);
        for v in self.verts.iter_mut() {
            Self::bounce(&mut v.x, &mut v.vx, max_x);
            Self::bounce(&mut v.y, &mut v.vy, max_y);
        }
        for k in 0..self.verts.len() / self.per {
            let base = k * self.per;
            for e in 0..self.edges {
                let a = &self.verts[base + e];
                let a = (a.x as i32, a.y as i32);
                let b = &self.verts[base + (e + 1) % self.per];
                let b = (b.x as i32, b.y as i32);
                self.line(a, b, k as u8);
            }
        }
    }
}

impl Saver for Strings {
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
            Cell::new(font::BRAILLE[dots[i] as usize], colour(hue[i], h))
        });
        grid.flush(s, &PAL);
    }

    fn name(&self) -> &'static str {
        "strings"
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
    use crate::testalloc::count as allocs;

    /// 1080 is not a multiple of 16: 67 rows cover 1072 and the bottom 8
    /// scanlines belong to no cell. That strip is the point of the frame-0
    /// assertion.
    fn panel() -> Panel {
        Panel::new(1920, 1080, 1920)
    }

    /// The panel change in flight. Nothing in this saver may assume 16:9.
    fn short_panel() -> Panel {
        Panel::new(1280, 400, 1280)
    }

    /// T1. Frame 0 must cover the panel including the strip below the last cell
    /// row, and must cover it with the SCENE — a reported black rectangle
    /// satisfies the rows assertion on its own.
    #[test]
    fn frame_zero_paints_the_whole_panel() {
        for p in [panel(), short_panel()] {
            let mut c = Strings::new(&p, 15);
            let mut buf = vec![0u32; p.buf_len()];
            let d = saver::frame(&mut c, &mut buf, &p);
            assert_eq!(d.rows(), p.h, "frame 0 must paint the whole panel");
            let lit = buf.iter().filter(|&&px| px != 0).count();
            assert!(lit > 5_000, "frame 0 painted nothing ({lit} lit)");
        }
    }

    /// 1080 / 16 leaves a remainder; the assertion above is only worth
    /// something on a panel that has one.
    #[test]
    fn the_test_panel_has_a_margin_strip() {
        let p = panel();
        let c = Strings::new(&p, 15);
        assert!(!p.h.is_multiple_of(c.grid.cell_h()));
    }

    /// T2. Every scanline whose pixels changed is inside a damage run —
    /// otherwise simpledrm scans out the previous frame there forever. The
    /// companions keep this from being a bound nothing approaches: damage must
    /// be non-empty and must stay under a full repaint.
    #[test]
    fn damage_covers_every_changed_scanline() {
        let p = panel();
        let mut c = Strings::new(&p, 15);
        let mut buf = vec![0u32; p.buf_len()];
        let mut prev = vec![0u32; p.buf_len()];
        let stride = p.buf_len() / p.h;
        saver::frame(&mut c, &mut buf, &p);

        let mut worst = 0usize;
        let mut total = 0usize;
        const FRAMES: usize = 200;
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
        assert!(
            total / (FRAMES - 1) > 0,
            "nothing moved: the coverage check proves nothing"
        );
        assert!(worst <= c.grid.rows() * c.grid.cell_h());
    }

    /// T3. The frame loop is sacred: `render` must not allocate. Counted for
    /// real rather than inferred from Vec capacities — a capacity check misses
    /// a temporary that is allocated and freed inside the frame, which is
    /// exactly the shape a `collect()` or a `format!` sneaking into the render
    /// path would have.
    #[test]
    fn render_never_allocates() {
        let p = Panel::new(320, 200, 320);
        let mut c = Strings::new(&p, 15);
        let mut buf = vec![0u32; p.buf_len()];
        // Warm the first frame: `Grid`'s frame-0 arm is not the steady state.
        saver::frame(&mut c, &mut buf, &p);

        let before = allocs();
        for _ in 0..2_000 {
            saver::frame(&mut c, &mut buf, &p);
        }
        assert_eq!(allocs(), before, "the render path allocated");

        // Non-vacuous twice over: the counter does count, and the frames drew.
        let probe = allocs();
        drop(vec![0u8; 8]);
        assert!(allocs() > probe, "the counting allocator counts nothing");
        assert!(
            c.heat.iter().any(|&h| h > 0),
            "2k frames drew nothing into the buffers"
        );
    }

    /// T4. A vertex must never leave the sub-cell grid: `stamp` indexes with it
    /// and an escape is a panic on the panel, weeks in. Run at an absurd speed,
    /// where a single-reflection bounce would overshoot.
    #[test]
    fn vertices_never_leave_the_panel() {
        for p in [panel(), short_panel()] {
            let mut c = Strings::new(&p, 15);
            // Faster than the panel is wide, so a single reflection overshoots
            // the far wall and only the clamp saves the index.
            for v in c.verts.iter_mut() {
                v.vx = c.max_x * 3.0;
                v.vy = -c.max_y * 7.0;
            }
            for _ in 0..5_000 {
                c.step();
                for v in &c.verts {
                    assert!((0.0..=c.max_x).contains(&v.x), "x {} escaped", v.x);
                    assert!((0.0..=c.max_y).contains(&v.y), "y {} escaped", v.y);
                }
            }
        }
    }

    /// T5. The polygon FLEXES: two corners' separation must actually change,
    /// or what is drawn is a rigid shape sliding around — a different and much
    /// duller screensaver — or, in the degenerate case, corners stacked on each
    /// other drawing nothing.
    ///
    /// What this deliberately does NOT claim to catch is a shared initial
    /// HEADING. Mutating the per-corner angle roll to a constant survives every
    /// test here, and that is a property of the simulation rather than a gap:
    /// `bounce` flips a sign per corner per axis, so corners that set off
    /// together are on different headings within a second of frames and the
    /// panel is indistinguishable. It is an equivalent mutant, not an untested
    /// branch.
    #[test]
    fn the_polygon_flexes_rather_than_sliding() {
        let mut c = Strings::new(&panel(), 15);
        let span = |c: &Strings| {
            let (a, b) = (&c.verts[0], &c.verts[1]);
            ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
        };
        let start = span(&c);
        let mut worst: f32 = 0.0;
        for _ in 0..300 {
            c.step();
            worst = worst.max((span(&c) - start).abs());
        }
        assert!(
            worst > c.max_x.min(c.max_y) * 0.1,
            "two corners stayed {worst} apart: the polygon is rigid"
        );
        // And the shape is closed: as many edges as corners.
        assert_eq!(c.edges, c.per);
    }

    /// T6. Two vertices is ONE segment; every other corner count closes the
    /// polygon.
    #[test]
    fn a_two_vertex_ribbon_is_one_segment() {
        assert_eq!(edges_for(2), 1);
        for per in 3..=MAX_VERTS {
            assert_eq!(edges_for(per), per);
        }
    }

    /// T7. An edge is CONTINUOUS: every sub-cell between the endpoints is
    /// stamped, so a long diagonal is a line and not a dotted spray.
    #[test]
    fn an_edge_leaves_no_gaps() {
        let p = panel();
        let mut c = Strings::new(&p, 15);
        c.heat.fill(0);
        c.dots.fill(0);
        c.line((3, 5), (400, 190), 0);
        // x is the dominant axis, so every sub-column the edge spans must hold
        // a lit cell. A gap is a column with nothing in it, which is what a
        // per-frame step instead of a per-sub-cell one would leave.
        for cx in (3 / 2)..=(400 / 2) {
            assert!(
                (0..c.grid.rows()).any(|cy| c.heat[cy * c.cols + cx] == HEAT_MAX),
                "gap: nothing lit in sub-column {cx}"
            );
        }
        // ...and it is a LINE, not a fill: one cell high per column, give or
        // take the cell the slope crosses in.
        let lit = c.heat.iter().filter(|&&h| h == HEAT_MAX).count();
        assert!((199..=400).contains(&lit), "{lit} cells lit for one edge");
    }

    /// T8. The artifact `lissajous` shipped: a pass that has faded must leave
    /// NOTHING in a cell a later pass relights, or an inherited dot is drawn in
    /// the wrong hue, lies across the new line instead of along it, and
    /// outlives the ribbon it belonged to. One frame of decay is one frame, so
    /// subtracting `decay` ages the cell exactly as `step` would.
    #[test]
    fn a_faded_pass_leaves_no_dots_in_a_cell_a_later_pass_relights() {
        let mut c = Strings::new(&Panel::new(320, 200, 320), 15);
        c.heat[0] = 0;
        c.dots[0] = 0;

        c.stamp(0, 0, 0);
        c.stamp(0, 1, 0);
        assert_eq!(c.dots[0], dot_bit(0, 0) | dot_bit(0, 1));

        // Two frames old: no longer a cell this pass is drawing through.
        c.heat[0] -= c.decay * 2;
        c.stamp(1, 2, 1);
        assert_eq!(
            c.dots[0],
            dot_bit(1, 2),
            "the relit cell kept a faded pass's dots: a tick across the new line"
        );
        assert_eq!((c.heat[0], c.hue[0]), (HEAT_MAX, 1));

        // The other side of the boundary, and the reason this is not simply
        // "always clear": a cell the line is still crossing must accumulate, or
        // every edge breaks into dashes at the frame boundary.
        c.heat[0] -= c.decay;
        c.stamp(1, 3, 1);
        assert_eq!(
            c.dots[0],
            dot_bit(1, 2) | dot_bit(1, 3),
            "a cell relit on the very next frame lost the dots it just drew"
        );
    }

    /// T9. The ribbon is `STRINGS_FADE_MS` long. `decay` is what `fresh` is
    /// derived from, so a change that widened the freshness window by slowing
    /// the fade would pass T8 and quietly leave ten-second ribbons.
    #[test]
    fn a_cell_fades_out_in_the_configured_time() {
        let fps = 15u32;
        let mut c = Strings::new(&Panel::new(320, 200, 320), fps);
        c.heat[0] = HEAT_MAX;
        let mut frames = 0u32;
        while c.heat[0] > 0 {
            c.heat[0] = c.heat[0].saturating_sub(c.decay);
            frames += 1;
            assert!(frames < 10_000, "the ribbon never fades");
        }
        let want = 1000 * fps / 1000; // default STRINGS_FADE_MS
        assert!(
            frames.abs_diff(want) * 100 <= want * 15,
            "a full-heat cell took {frames} frames to fade, wanted ~{want}"
        );
    }

    /// T10. The gap between consecutive outlines is the look, and it must be
    /// the same on a 1280x400 panel as on a 1920x1080 one. Scaling the speed to
    /// the panel — the obvious thing, and what the first cut of this did —
    /// gives the short panel 2.7x less separation and renders a solid sheet.
    #[test]
    fn outline_spacing_does_not_depend_on_the_panel() {
        let speed = |c: &Strings| (c.verts[0].vx.powi(2) + c.verts[0].vy.powi(2)).sqrt();
        let a = speed(&Strings::new(&panel(), 15));
        let b = speed(&Strings::new(&short_panel(), 15));
        assert_eq!(a, b, "{a} sub-cells/frame at 1080, {b} at 400");
        assert!(
            a > 1.0,
            "outlines {a} sub-cells apart would merge into a sheet"
        );
    }

    /// Every colour index a cell can address must exist: one off the end is an
    /// index panic inside `Grid::blit`, on the panel, weeks in.
    #[test]
    fn every_reachable_colour_index_is_in_the_palette() {
        // The dimmest a LIT cell can be is heat 1, and it must not be index 0:
        // that is the black entry, and a ribbon whose tail renders as black is
        // a ribbon with no fade at all.
        assert_eq!(colour(0, 1), 1);
        assert_ne!(PAL[colour(0, 1) as usize], 0, "the dimmest level is black");
        assert_eq!(
            colour(MAX_RIBBONS as u8 - 1, HEAT_MAX) as usize,
            PAL_LEN - 1
        );
        assert_ne!(PAL[PAL_LEN - 1], 0, "the brightest level is black");
        // No ribbon's range may overlap its neighbour's.
        assert_eq!(colour(1, 0), colour(0, HEAT_MAX) + 1);
    }
}
