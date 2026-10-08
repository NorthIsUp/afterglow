//! The mapped frame and the damage accrued against it.
//!
//! # Contract
//!
//! simpledrm — the driver U-Boot hands over on a Pi 5 — scans out of a SHADOW
//! buffer. A pixel written into the mapping reaches the panel only if the
//! driver is told its scanline changed. A region written but never reported
//! shows the previous frame forever; that is the "screen went blank and never
//! animated" symptom, and it reproduces on hardware and nowhere else.
//!
//! So writable memory is obtainable only through [`Surface::cell_rows`], which
//! marks the RECTANGLE it is about to hand out — both axes — BEFORE handing it
//! out. Under-reporting is unrepresentable. Over-reporting is only a larger
//! shadow-to-hardware copy. A rect too narrow is the same defect as a scanline
//! never reported: vertical bands of stale pixels on the panel.
//!
//! Four consequences a saver still owns:
//!
//! 1. Empty damage is a promise that zero pixels changed. Honour it by not
//!    writing, never by not reporting.
//! 2. The FIRST frame must cover every pixel the saver owns. `create_dumb_buffer`
//!    hands back zeroed memory and `set_crtc` has already scanned that black
//!    frame out. `Grid` handles this with its `first` flag; a saver that does
//!    not use `Grid` must handle it itself.
//! 3. Rows never written keep the previous frame. The host clears nothing and
//!    the mapping is the same buffer object every frame.
//! 4. Pixels outside the cell grid (`w % cell_w`, `h % cell_h`) belong to no
//!    cell. They are painted and reported ONCE, on frame 0, by
//!    [`Surface::fill_outside`] — see `Grid::flush`. Writing them without
//!    reporting them looks right in a dump and stays garbage on the panel.
//!
//! Row slices are cut to `w`, never `stride32`. `pitch` may exceed `w * 4`;
//! on this panel it does not, so getting that wrong would survive hardware
//! verification and break on the next board. `stride32` is therefore private.

use drm::control::ClipRect;

/// Panel geometry. The buffer is addressed as `u32` rather than bytes: writing
/// a pixel as a 4-byte slice copy costs a bounds check plus a memcpy call per
/// pixel, and at 1920x1080 that is two million of them per frame, where a `u32`
/// store is one bounds-checked write. `stride32` is the row stride in u32 units
/// — a 32bpp pitch is always a multiple of 4, so that division is exact.
#[derive(Clone, Copy)]
pub struct Panel {
    pub w: usize,
    pub h: usize,
    stride32: usize,
}

impl Panel {
    pub fn new(w: usize, h: usize, stride32: usize) -> Self {
        Self { w, h, stride32 }
    }

    /// Length in `u32` of a buffer this panel can be drawn into.
    pub fn buf_len(&self) -> usize {
        self.stride32 * self.h
    }
}

/// Rectangles a frame touched, half-open in BOTH axes. Fixed capacity: each run
/// becomes one `ClipRect` and simpledrm copies each independently, so an unbounded
/// list trades shadow copy for ioctl payload.
pub const MAX_RUNS: usize = 16;

/// One damaged rectangle. The x extent is the point: a 224px toaster reported as
/// a full-width scanline band makes simpledrm copy 8.5x the pixels that changed,
/// and `cell_rows` already knows the exact `x..x+w` it is handing out.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Run {
    pub x0: u16,
    pub y0: u16,
    pub x1: u16,
    pub y1: u16,
}

impl Run {
    pub const fn new(x0: u16, y0: u16, x1: u16, y1: u16) -> Self {
        Self { x0, y0, x1, y1 }
    }

    /// Test-only: what the panel does with a rect is copy it, not query it.
    #[cfg(test)]
    #[inline]
    pub fn covers(self, x: u16, y: u16) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }

    /// Overlapping or merely touching, in both axes — half-open, so `0..16` and
    /// `16..32` touch and must collapse rather than open a second rect.
    #[inline]
    fn touches(self, o: Run) -> bool {
        o.x0 <= self.x1 && o.x1 >= self.x0 && o.y0 <= self.y1 && o.y1 >= self.y0
    }

    #[inline]
    fn absorb(&mut self, o: Run) {
        self.x0 = self.x0.min(o.x0);
        self.y0 = self.y0.min(o.y0);
        self.x1 = self.x1.max(o.x1);
        self.y1 = self.y1.max(o.y1);
    }

    #[inline]
    fn area(self) -> u32 {
        u32::from(self.x1 - self.x0) * u32::from(self.y1 - self.y0)
    }

    /// Pixels merging the two would copy that neither copies now. Saturating
    /// because overlapping runs make the union smaller than the parts, and a
    /// pair that overlaps is the pair you most want merged.
    #[inline]
    fn union_cost(mut self, o: Run) -> u32 {
        let (a, b) = (self.area(), o.area());
        self.absorb(o);
        self.area().saturating_sub(a + b)
    }
}

/// Spills a frame may pay for precision before its damage is called dense. A
/// frame of separated objects needs a few dozen. A frame whose dirty cells are
/// scattered over the whole panel needs thousands, and buys nothing with them:
/// its rects converge on the panel either way. Past this every rect widens to
/// the span they collectively cover, which turns the rules below back into the
/// scanline-run algorithm this had before x extents — at the cost it had.
///
/// Measured, Mpx copied per frame: at 64 and 128 toasters2 sits at 1.19 and
/// 1.18 where 256 gets it to 0.35. Past 256 the scan starts to show — at 512
/// matrix spends ~280us a frame separating rects that are already the panel,
/// to save 0.2 Mpx of copy.
///
/// Those are 1080p numbers, where a dense saver pays ~50us a frame to reach
/// saturation and gains nothing. On the panel this actually drives, 1280x400,
/// there are a third as many cell rows and the same budget takes matrix from
/// 0.49 to 0.28 Mpx, rain 0.50 to 0.07 and lissajous 0.33 to 0.02 — the "dense"
/// savers are only dense at 1080p. `BENCH_W`/`BENCH_H` bench either.
const SPILL_BUDGET: u16 = 256;

#[derive(Clone, Copy)]
pub struct Damage {
    runs: [Run; MAX_RUNS],
    n: usize,
    spills: u16,
    /// The x span every rect widens to once saturated. Meaningless before then.
    wide: (u16, u16),
}

impl Damage {
    pub fn new() -> Self {
        Self {
            runs: [Run::new(0, 0, 0, 0); MAX_RUNS],
            n: 0,
            spills: 0,
            wide: (u16::MAX, 0),
        }
    }

    /// Grids scan row-major, so a new rect is almost always identical to or
    /// contiguous with the LAST: cells along a row extend it sideways, and the
    /// next row extends it downwards. That probe comes first, and a
    /// full-repaint frame never looks past it or leaves one rect.
    ///
    /// A mark that touches nothing opens a fresh rect rather than widening one
    /// across a gap — that is the whole point of carrying x.
    #[inline]
    fn mark(&mut self, x0: usize, y0: usize, x1: usize, y1: usize) {
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let mut r = Run::new(x0 as u16, y0 as u16, x1 as u16, y1 as u16);
        // Saturated: x has stopped separating anything, so every rect — this one
        // included — widens to the span they collectively cover. `touches` then
        // reduces to a y-only test and `union_cost` to the vertical gap times a
        // constant, which IS the algorithm this had before x extents, at the
        // cost it had. Folding into a single bounding box instead loses the
        // vertical sparsity those scanline runs kept, and measured WORSE than
        // the old code on toasters2: 1.69 Mpx a frame against 1.32.
        if self.spills >= SPILL_BUDGET {
            self.wide = (self.wide.0.min(r.x0), self.wide.1.max(r.x1));
            (r.x0, r.x1) = self.wide;
            // Every band spans the same x, so this is a y-only search, and it
            // is worth all sixteen compares: a band that absorbs the mark is a
            // `spill` not paid, and on a dense frame nearly every mark lands in
            // one. See `spill` for what it would otherwise cost.
            for band in &mut self.runs[..self.n] {
                if band.touches(r) {
                    band.absorb(r);
                    return;
                }
            }
        }
        if self.n > 0 {
            let last = &mut self.runs[self.n - 1];
            if last.touches(r) {
                last.absorb(r);
                return;
            }
        }
        if self.n == MAX_RUNS {
            self.spill();
        }
        self.runs[self.n] = r;
        self.n += 1;
    }

    /// Full: merge the pair whose union copies the fewest extra pixels, making
    /// room for the new rect. Dropping a rect would leave a region stale
    /// forever; merging only copies more — but with an x extent "more" is no
    /// longer proportional to the vertical gap. Two 64px-wide rects a thousand
    /// scanlines apart cost less merged than a 64px one and a full-width one
    /// that are adjacent, and the smallest-gap rule picked the latter.
    ///
    /// Any pair, not just neighbours: the row-slices of one object are
    /// separated in the array by every other object's slices, and they are
    /// exactly the pair that unions for free. That costs 120 unions, which is
    /// affordable only because [`SPILL_BUDGET`] bounds how often a frame pays.
    fn spill(&mut self) {
        let (mut bi, mut bj, mut best) = (0, 1, u32::MAX);
        for i in 0..self.n - 1 {
            for j in i + 1..self.n {
                let cost = self.runs[i].union_cost(self.runs[j]);
                if cost < best {
                    (best, bi, bj) = (cost, i, j);
                }
            }
        }
        let other = self.runs[bj];
        self.runs[bi].absorb(other);
        self.runs.copy_within(bj + 1..self.n, bj);
        self.n -= 1;
        self.spills += 1;
        if self.spills == SPILL_BUDGET {
            // The frame is dense. Widen what is already here to one span, so
            // every later mark merges by y alone — see `mark`. Rects that x was
            // keeping apart now overlap, so coalesce them back into disjoint
            // bands: sixteen full-width rects over the same scanlines would
            // copy the panel several times over.
            self.wide = self.runs[..self.n]
                .iter()
                .fold((u16::MAX, 0), |w, r| (w.0.min(r.x0), w.1.max(r.x1)));
            for r in &mut self.runs[..self.n] {
                (r.x0, r.x1) = self.wide;
            }
            self.runs[..self.n].sort_unstable_by_key(|r| r.y0);
            let mut w = 0;
            for i in 1..self.n {
                let o = self.runs[i];
                if o.y0 <= self.runs[w].y1 {
                    self.runs[w].absorb(o);
                } else {
                    w += 1;
                    self.runs[w] = o;
                }
            }
            self.n = w + 1;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// Fills `out`, returns the count. Caller-owned array: the frame loop never
    /// allocates. `ClipRect` is (x1, y1, x2, y2) — width before height.
    pub fn rects(&self, out: &mut [ClipRect; MAX_RUNS]) -> usize {
        for (rect, r) in out.iter_mut().zip(self.runs[..self.n].iter()) {
            *rect = ClipRect::new(r.x0, r.y0, r.x1, r.y1);
        }
        self.n
    }

    /// Is this pixel inside some reported rect? The whole contract as a
    /// predicate. Test-only: `dump::row_reported` is the same question asked
    /// per scanline, which is the shape every caller outside a unit test wants.
    #[cfg(test)]
    pub fn covers(self, x: usize, y: usize) -> bool {
        let (x, y) = (x as u16, y as u16);
        self.runs[..self.n].iter().any(|r| r.covers(x, y))
    }

    /// Distinct scanlines reported — a UNION, not a sum: two objects side by
    /// side report their own rect over the same scanlines, and adding those
    /// would say a frame touched more rows than the panel has. Telemetry and
    /// tests only; the frame loop never calls it.
    pub fn rows(&self) -> usize {
        let mut ys = [(0u16, 0u16); MAX_RUNS];
        for (o, r) in ys.iter_mut().zip(self.runs()) {
            *o = (r.y0, r.y1);
        }
        let ys = &mut ys[..self.n];
        ys.sort_unstable();
        let (mut total, mut end) = (0usize, 0u16);
        for &(y0, y1) in ys.iter() {
            let start = y0.max(end);
            if y1 > start {
                total += usize::from(y1 - start);
                end = y1;
            }
        }
        total
    }

    /// Pixels the shadow-to-hardware copy will move. Overlap IS counted: the
    /// copy really does move those pixels once per rect.
    pub fn px(&self) -> usize {
        self.runs[..self.n].iter().map(|r| r.area() as usize).sum()
    }

    pub fn runs(&self) -> &[Run] {
        &self.runs[..self.n]
    }
}

impl Default for Damage {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Surface<'a> {
    buf: &'a mut [u32],
    w: usize,
    h: usize,
    stride32: usize,
    damage: Damage,
}

impl<'a> Surface<'a> {
    pub fn new(buf: &'a mut [u32], panel: &Panel) -> Self {
        Self {
            buf,
            w: panel.w,
            h: panel.h,
            stride32: panel.stride32,
            damage: Damage::new(),
        }
    }

    /// The rows of one cell, each cut to `w` visible pixels, clamped to the
    /// panel, and MARKED DIRTY BEFORE RETURN. This is the only route to a
    /// writable pixel in the program.
    ///
    /// One damage mark per CELL, not per row: `y..y+h` is marked once, then the
    /// rows come out of a single `chunks_mut` with no per-row offset
    /// re-derivation.
    #[inline]
    pub fn cell_rows(
        &mut self,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
    ) -> impl Iterator<Item = &mut [u32]> + '_ {
        let (sw, sh, stride) = (self.w, self.h, self.stride32);
        let x = x.min(sw);
        let y = y.min(sh);
        let w = w.min(sw - x);
        let h = h.min(sh - y);
        self.damage.mark(x, y, x + w, y + h);
        let start = y * stride;
        let end = start + h * stride;
        self.buf[start..end]
            .chunks_mut(stride)
            .map(move |r| &mut r[x..x + w])
    }

    /// Paint everything outside a `gw` x `gh` area and report it. The grid
    /// covers whole cells only, so a panel whose height is not a multiple of
    /// `cell_h` has a bottom strip (and one on the right for the width) that no
    /// saver ever writes; on simpledrm it shows whatever the shadow buffer held,
    /// which is the garbage line along the bottom of the real panel. Frame 0
    /// only — after that the strip is already this colour and nothing moves it.
    pub fn fill_outside(&mut self, gw: usize, gh: usize, v: u32) {
        if gw < self.w {
            for row in self.cell_rows(gw, 0, self.w - gw, gh.min(self.h)) {
                row.fill(v);
            }
        }
        if gh < self.h {
            for row in self.cell_rows(0, gh, self.w, self.h - gh) {
                row.fill(v);
            }
        }
    }

    pub fn finish(self) -> Damage {
        self.damage
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marked(rects: &[(usize, usize, usize, usize)]) -> Vec<Run> {
        let mut d = Damage::new();
        for &(x0, y0, x1, y1) in rects {
            d.mark(x0, y0, x1, y1);
        }
        d.runs().to_vec()
    }

    /// Full-width marks, which is what a repainting grid produces.
    fn band(y0: usize, y1: usize) -> (usize, usize, usize, usize) {
        (0, y0, 1920, y1)
    }

    #[test]
    fn contiguous_runs_merge() {
        // The exclusive y1 is the whole point: 0..16 and 16..32 are adjacent
        // scanline bands, not overlapping ones, and must collapse to 0..32.
        assert_eq!(
            marked(&[band(0, 16), band(16, 32), band(32, 48)]),
            [Run::new(0, 0, 1920, 48)]
        );
        assert_eq!(
            marked(&[band(0, 16), band(0, 16)]),
            [Run::new(0, 0, 1920, 16)]
        );
        assert_eq!(
            marked(&[band(0, 16), band(8, 24)]),
            [Run::new(0, 0, 1920, 24)]
        );
    }

    #[test]
    fn gaps_stay_separate() {
        assert_eq!(
            marked(&[band(0, 16), band(17, 32)]),
            [Run::new(0, 0, 1920, 16), Run::new(0, 17, 1920, 32)]
        );
        assert_eq!(marked(&[band(0, 16), band(32, 48), band(64, 80)]).len(), 3);
    }

    #[test]
    fn empty_marks_are_ignored() {
        assert_eq!(marked(&[band(16, 16), band(32, 8)]), [] as [Run; 0]);
        // And in the other axis, which a zero-width clamp produces.
        assert_eq!(marked(&[(8, 0, 8, 16), (8, 0, 4, 16)]), [] as [Run; 0]);
    }

    /// The finding this type exists for: a narrow object must not report a
    /// full-width band. Cells down one column, one scanline band each.
    #[test]
    fn a_narrow_object_reports_a_narrow_rect() {
        let cells: Vec<_> = (0..10).map(|i| (224, i * 32, 448, i * 32 + 32)).collect();
        assert_eq!(marked(&cells), [Run::new(224, 0, 448, 320)]);
    }

    /// Two objects in the same scanline band are two rects, not one spanning
    /// the gap between them. This is what `touches` checking x buys.
    #[test]
    fn a_horizontal_gap_opens_a_second_rect() {
        let runs = marked(&[(0, 0, 64, 32), (1600, 0, 1664, 32)]);
        assert_eq!(runs, [Run::new(0, 0, 64, 32), Run::new(1600, 0, 1664, 32)]);
        assert_eq!(runs.iter().map(|r| r.area()).sum::<u32>(), 2 * 64 * 32);
    }

    #[test]
    fn spill_merges_the_cheapest_union_and_never_drops_coverage() {
        let mut d = Damage::new();
        // MAX_RUNS full-width bands 4 apart, then one more so it has to spill.
        for i in 0..=MAX_RUNS {
            d.mark(0, i * 20, 1920, i * 20 + 16);
        }
        assert_eq!(d.runs().len(), MAX_RUNS);
        let runs = d.runs();
        assert_eq!(runs[0].y0, 0);
        assert_eq!(runs[MAX_RUNS - 1].y1 as usize, MAX_RUNS * 20 + 16);
        // Every originally-marked pixel is still inside some run.
        for i in 0..=MAX_RUNS {
            for y in (i * 20)..(i * 20 + 16) {
                assert!(d.covers(0, y) && d.covers(1919, y), "y={y} lost");
            }
        }
    }

    /// The rule change. Interleaved row-slices of a 64px-wide object and a
    /// 920px one: the smallest VERTICAL gap is zero — between the two objects'
    /// slices on the same band — and merging there makes a full-width rect.
    /// The cheapest UNION is two slices of the narrow object, at 1024px.
    #[test]
    fn spill_prefers_the_cheap_distant_union_over_the_zero_gap_one() {
        let mut d = Damage::new();
        for i in 0..=MAX_RUNS / 2 {
            d.mark(0, i * 32, 64, i * 32 + 16);
            d.mark(1000, i * 32, 1920, i * 32 + 16);
        }
        assert!(
            d.runs().iter().any(|r| r.x1 == 64 && r.y1 - r.y0 > 16),
            "the narrow object's slices should have merged with each other: {:?}",
            d.runs()
        );
        assert!(
            d.runs().iter().all(|r| r.x1 - r.x0 <= 920),
            "a spill widened a rect across the empty middle: {:?}",
            d.runs()
        );
    }

    /// Past `SPILL_BUDGET` the rects stop being separated horizontally: they
    /// widen to one span and merge by y alone, which is the scanline-run
    /// algorithm this had before x extents. Keeping the y structure is the
    /// point — one bounding box instead would fill in every gap between the
    /// bands and copy MORE than the old code did (measured: toasters2, 1.32
    /// Mpx a frame before, 1.69 with a single box).
    #[test]
    fn a_saturated_frame_falls_back_to_scanline_bands() {
        let mut d = Damage::new();
        // Isolated cells with a gap around every one, so each opens a rect and
        // the table spills on each: the worst case this budget exists for.
        let cells: Vec<_> = (0..40)
            .flat_map(|r| (0..40).map(move |c| (c * 48, r * 27, c * 48 + 16, r * 27 + 16)))
            .collect();
        for &(x0, y0, x1, y1) in &cells {
            d.mark(x0, y0, x1, y1);
        }
        let span = (d.runs()[0].x0, d.runs()[0].x1);
        assert!(d.runs().iter().all(|r| (r.x0, r.x1) == span));
        assert!(d.runs().len() > 1, "collapsed to one box: {:?}", d.runs());
        // The gaps between the bands are not being copied.
        assert!(
            d.px() < usize::from(span.1 - span.0) * 40 * 27,
            "a saturated frame copies the gaps too: {:?}",
            d.runs()
        );
        for &(x0, y0, x1, y1) in &cells {
            assert!(
                d.covers(x0, y0) && d.covers(x1 - 1, y1 - 1),
                "lost {x0},{y0}"
            );
        }
    }

    /// And the budget must not fire on the frames it exists to protect: a
    /// handful of separated objects, each a column of cells, stays several
    /// narrow rects rather than collapsing to the panel.
    #[test]
    fn a_few_separated_objects_do_not_saturate() {
        let mut d = Damage::new();
        for row in 0..20 {
            for obj in 0..4 {
                let x = 400 * obj;
                d.mark(x, row * 32, x + 224, row * 32 + 32);
            }
        }
        assert!(d.runs().len() > 1, "collapsed to one box: {:?}", d.runs());
        assert!(d.runs().iter().all(|r| r.x1 - r.x0 == 224));
        // No overlap and nothing wasted: exactly the four columns, once each.
        assert_eq!(d.px(), 4 * 224 * 640);
    }

    #[test]
    fn rows_rects_and_px() {
        let mut d = Damage::new();
        d.mark(0, 0, 1920, 16);
        d.mark(64, 32, 128, 40);
        assert_eq!(d.rows(), 24);
        assert_eq!(d.px(), 1920 * 16 + 64 * 8);
        let mut out = [ClipRect::new(0, 0, 0, 0); MAX_RUNS];
        assert_eq!(d.rects(&mut out), 2);
        assert_eq!(
            (out[0].x1(), out[0].y1(), out[0].x2(), out[0].y2()),
            (0, 0, 1920, 16)
        );
        assert_eq!(
            (out[1].x1(), out[1].y1(), out[1].x2(), out[1].y2()),
            (64, 32, 128, 40)
        );
    }

    #[test]
    fn cell_rows_marks_before_it_hands_out_memory() {
        let panel = Panel::new(4, 4, 4);
        let mut buf = vec![0u32; panel.buf_len()];
        let mut s = Surface::new(&mut buf, &panel);
        for row in s.cell_rows(0, 1, 2, 2) {
            row.fill(0xFF);
        }
        let d = s.finish();
        assert_eq!(d.runs(), [Run::new(0, 1, 2, 3)]);
        assert_eq!(
            buf,
            [0, 0, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn cell_rows_clamps_to_the_panel() {
        let panel = Panel::new(4, 4, 4);
        let mut buf = vec![0u32; panel.buf_len()];
        let mut s = Surface::new(&mut buf, &panel);
        let widths: Vec<usize> = s.cell_rows(2, 3, 4, 4).map(|r| r.len()).collect();
        assert_eq!(widths, [2]);
        // The CLAMPED rect, not the asked-for one: reporting x 2..6 on a 4px
        // panel is an out-of-range ClipRect.
        assert_eq!(s.finish().runs(), [Run::new(2, 3, 4, 4)]);
    }

    /// The contract, exhaustively, on a panel small enough to check every
    /// pixel: whatever pattern of cells is written, no written pixel lies
    /// outside a reported rect. And the inverse — shrinking any reported rect
    /// by one column must leave a written pixel uncovered, so a rect that is
    /// merely too wide cannot pass for a correct one.
    #[test]
    fn every_written_pixel_is_inside_some_reported_rect() {
        let panel = Panel::new(64, 48, 64);
        let mut rng = 0x1234_5678u32;
        for _ in 0..64 {
            let mut buf = vec![0u32; panel.buf_len()];
            let mut s = Surface::new(&mut buf, &panel);
            for _ in 0..12 {
                rng = rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let x = (rng >> 8) as usize % 60;
                let y = (rng >> 16) as usize % 44;
                for row in s.cell_rows(x, y, 4, 4) {
                    row.fill(0xFF);
                }
            }
            let d = s.finish();
            let mut written = 0;
            for y in 0..panel.h {
                for x in 0..panel.w {
                    if buf[y * 64 + x] == 0 {
                        continue;
                    }
                    written += 1;
                    assert!(d.covers(x, y), "({x},{y}) written but not reported");
                }
            }
            assert!(written > 0);
            let mut shrunk = d;
            for r in &mut shrunk.runs[..shrunk.n] {
                r.x1 -= 1;
            }
            let leak = (0..panel.h)
                .any(|y| (0..panel.w).any(|x| buf[y * 64 + x] != 0 && !shrunk.covers(x, y)));
            assert!(leak, "every rect is wider than what was written into it");
        }
    }
}
