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
//! marks the scanlines it is about to hand out BEFORE handing them out.
//! Under-reporting is unrepresentable. Over-reporting is only a larger
//! shadow-to-hardware copy.
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
//! 4. Pixels outside the cell grid (`w % cell_w`, `h % cell_h`) are never
//!    written and stay at the dumb buffer's zeroed black. At 1920x1080 with
//!    `FIRE_CELL=16` that is an 8 px bottom strip, which is what ships today.
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

/// Scanline runs a frame touched, half-open `y0..y1`, kept sorted and disjoint.
/// Fixed capacity: each run becomes one ClipRect and simpledrm copies each
/// independently, so an unbounded list trades shadow copy for ioctl payload.
pub const MAX_RUNS: usize = 16;

#[derive(Clone, Copy)]
pub struct Damage {
    runs: [(u16, u16); MAX_RUNS],
    n: usize,
}

impl Damage {
    pub fn new() -> Self {
        Self {
            runs: [(0, 0); MAX_RUNS],
            n: 0,
        }
    }

    /// Grids scan row-major, so a new run is almost always identical to or
    /// contiguous with the last. Merging there keeps this O(1) with no scan. A
    /// mark that lands before the last run still records coverage; it just
    /// opens a fresh run rather than being absorbed.
    #[inline]
    fn mark(&mut self, y0: usize, y1: usize) {
        if y1 <= y0 {
            return;
        }
        let (y0, y1) = (y0 as u16, y1 as u16);
        if self.n > 0 {
            let last = &mut self.runs[self.n - 1];
            if y0 <= last.1 && y1 >= last.0 {
                last.0 = last.0.min(y0);
                last.1 = last.1.max(y1);
                return;
            }
        }
        if self.n == MAX_RUNS {
            self.spill();
        }
        self.runs[self.n] = (y0, y1);
        self.n += 1;
    }

    /// Full: merge the pair of runs with the smallest gap. Dropping a run would
    /// leave a region stale forever; merging only copies more.
    fn spill(&mut self) {
        let mut best = 0;
        let mut best_gap = u16::MAX;
        for i in 0..self.n - 1 {
            let gap = self.runs[i + 1].0.saturating_sub(self.runs[i].1);
            if gap < best_gap {
                best_gap = gap;
                best = i;
            }
        }
        self.runs[best].0 = self.runs[best].0.min(self.runs[best + 1].0);
        self.runs[best].1 = self.runs[best].1.max(self.runs[best + 1].1);
        self.runs.copy_within(best + 2..self.n, best + 1);
        self.n -= 1;
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// Fills `out`, returns the count. Caller-owned array: the frame loop never
    /// allocates. ClipRect is (x1, y1, x2, y2) — width before height.
    pub fn rects(&self, width: u16, out: &mut [ClipRect; MAX_RUNS]) -> usize {
        for (rect, &(y0, y1)) in out.iter_mut().zip(self.runs[..self.n].iter()) {
            *rect = ClipRect::new(0, y0, width, y1);
        }
        self.n
    }

    /// Total scanlines reported. Telemetry only — an over-reporting saver has
    /// no visual symptom, only a CPU one.
    pub fn rows(&self) -> usize {
        self.runs[..self.n]
            .iter()
            .map(|&(y0, y1)| (y1 - y0) as usize)
            .sum()
    }

    pub fn runs(&self) -> &[(u16, u16)] {
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
        self.damage.mark(y, y + h);
        let start = y * stride;
        let end = start + h * stride;
        self.buf[start..end]
            .chunks_mut(stride)
            .map(move |r| &mut r[x..x + w])
    }

    pub fn finish(self) -> Damage {
        self.damage
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marked(pairs: &[(usize, usize)]) -> Vec<(u16, u16)> {
        let mut d = Damage::new();
        for &(a, b) in pairs {
            d.mark(a, b);
        }
        d.runs().to_vec()
    }

    #[test]
    fn contiguous_runs_merge() {
        // The exclusive y1 is the whole point: 0..16 and 16..32 are adjacent
        // scanline bands, not overlapping ones, and must collapse to 0..32.
        assert_eq!(marked(&[(0, 16), (16, 32), (32, 48)]), [(0, 48)]);
        assert_eq!(marked(&[(0, 16), (0, 16)]), [(0, 16)]);
        assert_eq!(marked(&[(0, 16), (8, 24)]), [(0, 24)]);
    }

    #[test]
    fn gaps_stay_separate() {
        assert_eq!(marked(&[(0, 16), (17, 32)]), [(0, 16), (17, 32)]);
        assert_eq!(marked(&[(0, 16), (32, 48), (64, 80)]).len(), 3);
    }

    #[test]
    fn empty_marks_are_ignored() {
        assert!(marked(&[(16, 16), (32, 8)]).is_empty());
    }

    #[test]
    fn spill_merges_the_smallest_gap_and_never_drops_coverage() {
        let mut d = Damage::new();
        // MAX_RUNS runs 4 apart, then one more so the array has to spill.
        for i in 0..=MAX_RUNS {
            d.mark(i * 20, i * 20 + 16);
        }
        assert_eq!(d.runs().len(), MAX_RUNS);
        let runs = d.runs();
        assert_eq!(runs[0].0, 0);
        assert_eq!(runs[MAX_RUNS - 1].1 as usize, MAX_RUNS * 20 + 16);
        // Every originally-marked scanline is still inside some run.
        for i in 0..=MAX_RUNS {
            for y in (i * 20)..(i * 20 + 16) {
                let y = y as u16;
                assert!(runs.iter().any(|&(a, b)| y >= a && y < b), "y={y} lost");
            }
        }
    }

    #[test]
    fn rows_and_rects() {
        let mut d = Damage::new();
        d.mark(0, 16);
        d.mark(32, 40);
        assert_eq!(d.rows(), 24);
        let mut out = [ClipRect::new(0, 0, 0, 0); MAX_RUNS];
        assert_eq!(d.rects(1920, &mut out), 2);
        assert_eq!((out[0].y1(), out[0].y2(), out[0].x2()), (0, 16, 1920));
        assert_eq!((out[1].y1(), out[1].y2()), (32, 40));
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
        assert_eq!(d.runs(), [(1, 3)]);
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
        assert_eq!(s.finish().runs(), [(3, 4)]);
    }
}
