//! A character grid over the panel, and the one glyph blitter in the program.
//!
//! Every saver here draws cells, so the change test, the damage report and the
//! frame-0 full paint live once, in `flush`, instead of once per saver.
//!
//! # Pixel aspect
//!
//! The cell is also where the panel's non-square pixels are corrected. Pine's
//! monitor is 1280x400 and advertises nothing the Pi can read, so the firmware
//! drives it at 1920x1080 and the panel rescales 1.5x across and 2.7x down —
//! everything lands on screen squashed vertically by 1.8. `SAVER_PIXEL_ASPECT`
//! makes the CELL that much taller, and every saver whose coordinates are cells
//! or braille sub-cells — which is all of them but `moire` and `warp` — is
//! corrected for free. A knob 25 savers each have to remember is a knob 25
//! savers get wrong.

use std::sync::OnceLock;

use crate::font;
use crate::surface::{Panel, Surface};

/// How much taller a CELL must be drawn so it lands square on the glass, in
/// per-cent. 100 is no correction and is a byte-for-byte no-op; pine's panel is
/// 180.
///
/// NOT "how much taller a framebuffer pixel is" — it lands SHORTER than it is
/// wide there, and the stretch is the compensation. Getting that backwards is
/// what made `xwing` multiply where it should divide.
///
/// `Grid::new` applies it, so `cell_h()` is ALREADY multiplied. Converting a
/// grid dimension back to square-glass units is a DIVISION by this.
///
/// Read once per process, not per `Grid`: this is a property of the monitor,
/// and a saver switch must not pay an env lookup on the render thread.
pub fn pixel_aspect() -> usize {
    #[cfg(test)]
    if let Some(a) = TEST_ASPECT.with(std::cell::Cell::get) {
        return a;
    }
    static ASPECT: OnceLock<usize> = OnceLock::new();
    *ASPECT.get_or_init(|| crate::env_num(&["SAVER_PIXEL_ASPECT"], 100, 25, 400) as usize)
}

/// The panel's VISIBLE width in millimetres, or 0 when nobody has measured it.
///
/// Not discoverable: this monitor's EDID is 0 bytes, so the physical size exists
/// nowhere the Pi can read. It has to be typed in by someone with a ruler.
///
/// Only the mirror page uses it, to offer a canvas the same physical size as the
/// panel. Nothing in the render path reads it.
pub fn panel_mm() -> usize {
    static MM: OnceLock<usize> = OnceLock::new();
    *MM.get_or_init(|| crate::env_num(&["SAVER_PANEL_MM"], 0, 0, 5000) as usize)
}

#[cfg(test)]
thread_local! {
    static TEST_ASPECT: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Build a saver as if the panel had this aspect. Per THREAD, because the four
/// savers that stretch something themselves construct from the process-wide
/// value and cargo runs tests in parallel — `set_var` would land one test's
/// geometry in another's saver, which is the trap `satori` documents.
#[cfg(test)]
pub fn with_test_aspect<R>(aspect: usize, f: impl FnOnce() -> R) -> R {
    TEST_ASPECT.with(|c| c.set(Some(aspect)));
    let out = f();
    TEST_ASPECT.with(|c| c.set(None));
    out
}

/// Unlit glyph pixels. A cell always paints its whole rectangle, which is what
/// makes skipping an unchanged cell sound.
const BG: u32 = 0x0000_0000;

/// A cell's entire appearance, packed so the change test is one u32 compare —
/// exactly the compare `heat == prev_heat` was, for a strictly richer cell.
/// Colour is a palette INDEX, not an XRGB value, so the compare is exact.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(transparent)]
pub struct Cell(u32);

impl Cell {
    #[inline]
    pub const fn new(glyph: u16, colour: u16) -> Self {
        Self((glyph as u32) | ((colour as u32) << 16))
    }

    #[inline]
    pub const fn glyph(self) -> usize {
        (self.0 & 0xFFFF) as usize
    }

    #[inline]
    pub const fn colour(self) -> usize {
        (self.0 >> 16) as usize
    }

    /// An unlit, transparent cell: what a cleared cell holds, and what a
    /// sprite's blanks bake down to.
    pub const CLEAR: Self = Self::new(font::BLANK, 0);

    /// The packed word, for the mirror's wire format — which is this exact
    /// u32, so nothing re-encodes what `Cell` already packs.
    #[inline]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// A grid's geometry: `cols x rows` cells of `cell_w x cell_h` pixels.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Shape {
    pub cols: usize,
    pub rows: usize,
    pub cell_w: usize,
    pub cell_h: usize,
}

pub struct Grid {
    cols: usize,
    rows: usize,
    cell_w: usize,
    cell_h: usize,
    cur: Vec<Cell>,
    prev: Vec<Cell>,
    /// dest row -> source glyph row; dest column -> source bit mask. These
    /// replace two integer divides PER PIXEL — roughly four million a frame at
    /// 1080p — with two array reads.
    rowmap: Vec<u8>,
    mask: Vec<u8>,
    /// The buffer arrives zeroed and `set_crtc` has already scanned that black
    /// frame out, so frame 0 must paint every cell. An explicit flag rather
    /// than a sentinel in `prev`, so a saver may use any glyph or colour index.
    first: bool,
    /// What unlit glyph pixels and the margins paint. `BG` unless a saver
    /// draws over a coloured ground.
    ground: u32,
    /// Pixels the cells are drawn left and up of the panel's corner, for a
    /// camera that pans by the pixel rather than the cell. Only a `bleed`
    /// grid shifts: it has a column and a row past the panel's edge to slide
    /// in, so no shift opens a margin.
    shift: (usize, usize),
    bleed: bool,
    /// The panel's size, for which cells `inside` it.
    panel: (usize, usize),
}

impl Grid {
    /// `cols = panel.w / cell_w`, `rows = panel.h / cell_h`, both at least 1.
    /// The right/bottom remainder belongs to no cell, so `flush` paints it
    /// black on frame 0 — see `paint_margins`.
    ///
    /// THE GRID IS NOT THE PANEL, and anything sizing a picture must use the
    /// panel. At 1920x1080 a 32px cell leaves a 24px strip below the last row,
    /// so `rows * cell_h` is a 1.818 rectangle standing in for a 1.778 one. The
    /// web mirror sized its canvas that way and stretched every block 2.3%
    /// vertically to fill; `Mirror::describe` takes both rectangles because of
    /// this.
    ///
    /// `cell_h` is the height a SQUARE pixel would need; `SAVER_PIXEL_ASPECT`
    /// stretches it to what this panel needs. A saver that keeps its own copy
    /// must take it back from `cell_h()`, not from the value it passed in.
    pub fn new(panel: &Panel, cell_w: usize, cell_h: usize) -> Self {
        Self::with_aspect(panel, cell_w, cell_h, pixel_aspect())
    }

    /// `new` with the aspect handed in, so the tests can vary it: cargo runs
    /// them in parallel threads and the environment is process-wide.
    pub fn with_aspect(panel: &Panel, cell_w: usize, cell_h: usize, aspect: usize) -> Self {
        let mut g = Self {
            cols: 0,
            rows: 0,
            cell_w: 0,
            cell_h: 0,
            cur: Vec::new(),
            prev: Vec::new(),
            rowmap: Vec::new(),
            mask: Vec::new(),
            first: true,
            ground: BG,
            shift: (0, 0),
            bleed: false,
            panel: (0, 0),
        };
        g.reshape(panel, cell_w, cell_h, aspect);
        g
    }

    /// The geometry `with_aspect` would build, without building it — for a
    /// saver planning geometries ahead.
    pub fn shape(panel: &Panel, cell_w: usize, cell_h: usize, aspect: usize) -> Shape {
        let cell_w = cell_w.max(1);
        // Rounded, not truncated: a cell is a couple of dozen pixels, so
        // flooring 16 x 1.8 to 28 costs nearly a whole percent of the
        // correction the knob was set to make.
        let cell_h = ((cell_h.max(1) * aspect + 50) / 100).max(1);
        Shape {
            cols: (panel.w / cell_w).max(1),
            rows: (panel.h / cell_h).max(1),
            cell_w,
            cell_h,
        }
    }

    /// Become the grid `with_aspect` would build, keeping the ground, and
    /// repaint in full on the next flush: the panel still shows the old cells.
    /// Allocates nothing once the buffers have held a geometry at least this
    /// large in each of cell count, cell width and cell height.
    pub fn reshape(&mut self, panel: &Panel, cell_w: usize, cell_h: usize, aspect: usize) {
        self.reshape_to(panel, Self::shape(panel, cell_w, cell_h, aspect), false);
    }

    /// `shape`, plus whatever it takes for a [`shift`](Self::shift) of up to a
    /// cell less a pixel to leave no panel pixel uncovered: the cells that
    /// reach past the right and bottom edges, and one more of each.
    pub fn shape_bleed(panel: &Panel, cell_w: usize, cell_h: usize, aspect: usize) -> Shape {
        let s = Self::shape(panel, cell_w, cell_h, aspect);
        Shape {
            cols: panel.w.div_ceil(s.cell_w) + 1,
            rows: panel.h.div_ceil(s.cell_h) + 1,
            ..s
        }
    }

    /// `reshape` to [`shape_bleed`](Self::shape_bleed), unshifted.
    pub fn reshape_bleed(&mut self, panel: &Panel, cell_w: usize, cell_h: usize, aspect: usize) {
        self.reshape_to(
            panel,
            Self::shape_bleed(panel, cell_w, cell_h, aspect),
            true,
        );
    }

    fn reshape_to(&mut self, panel: &Panel, shape: Shape, bleed: bool) {
        let Shape {
            cols,
            rows,
            cell_w,
            cell_h,
        } = shape;
        (self.bleed, self.shift, self.panel) = (bleed, (0, 0), (panel.w, panel.h));
        (self.cols, self.rows, self.cell_w, self.cell_h) = (cols, rows, cell_w, cell_h);
        for v in [&mut self.cur, &mut self.prev] {
            v.clear();
            v.resize(cols * rows, Cell::CLEAR);
        }
        self.rowmap.clear();
        self.rowmap
            .extend((0..cell_h).map(|py| (py * font::GLYPH_H / cell_h) as u8));
        self.mask.clear();
        self.mask
            .extend((0..cell_w).map(|px| 0x80u8 >> (px * font::GLYPH_W / cell_w)));
        self.first = true;
    }

    #[inline]
    pub fn ground(&self) -> u32 {
        self.ground
    }

    /// Paint unlit pixels `ground` (XRGB8888) instead of black.
    pub fn with_ground(mut self, ground: u32) -> Self {
        self.ground = ground;
        self
    }

    /// Draw every cell `dx` pixels left and `dy` up of where it sits, each
    /// under a cell. A new shift moves every cell, so the next flush repaints
    /// them all, as frame 0 does.
    pub fn shift(&mut self, dx: usize, dy: usize) {
        debug_assert!(
            self.bleed || (dx, dy) == (0, 0),
            "shifting a grid with no bleed"
        );
        debug_assert!(dx < self.cell_w && dy < self.cell_h);
        if (dx, dy) != self.shift {
            self.shift = (dx, dy);
            self.first = true;
        }
    }

    #[inline]
    pub fn shift_of(&self) -> (usize, usize) {
        self.shift
    }

    /// The columns and rows wholly on the panel.
    pub fn inside(&self) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
        let span = |n: usize, cell: usize, shift: usize, len: usize| {
            let first = usize::from(shift > 0);
            let last = ((len + shift) / cell).min(n);
            first..last.max(first)
        };
        if !self.bleed {
            return (0..self.cols, 0..self.rows);
        }
        let (dx, dy) = self.shift;
        (
            span(self.cols, self.cell_w, dx, self.panel.0),
            span(self.rows, self.cell_h, dy, self.panel.1),
        )
    }

    #[inline]
    pub fn shape_of(&self) -> Shape {
        Shape {
            cols: self.cols,
            rows: self.rows,
            cell_w: self.cell_w,
            cell_h: self.cell_h,
        }
    }

    #[inline]
    pub fn cols(&self) -> usize {
        self.cols
    }

    #[inline]
    pub fn rows(&self) -> usize {
        self.rows
    }

    #[inline]
    pub fn cell_w(&self) -> usize {
        self.cell_w
    }

    #[inline]
    pub fn cell_h(&self) -> usize {
        self.cell_h
    }

    /// The cells of the frame just flushed. `flush` ends by swapping `cur` into
    /// `prev`, so THIS is the drawn frame and `cur` is next frame's scratch —
    /// read it after `flush`, never before.
    #[inline]
    pub fn cells(&self) -> &[Cell] {
        &self.prev
    }

    /// Take the cells `fill` just wrote as the drawn frame without blitting
    /// them, so `cells()` reads them: for a grid that is read but never drawn.
    #[inline]
    pub fn settle(&mut self) {
        std::mem::swap(&mut self.cur, &mut self.prev);
    }

    /// Write this frame's cells. `f(cx, cy)` is called for EVERY cell, so an
    /// unwritten cell is not expressible — which is what lets `flush` swap
    /// `cur`/`prev` instead of copying.
    #[inline]
    pub fn fill<F: FnMut(usize, usize) -> Cell>(&mut self, mut f: F) {
        for cy in 0..self.rows {
            for cx in 0..self.cols {
                self.cur[cy * self.cols + cx] = f(cx, cy);
            }
        }
    }

    /// `fill` a row at a time: `f(cy, row)` must write all of `row`. For a
    /// saver whose cells are a gather along each row, which a per-cell
    /// closure makes twice as dear.
    #[cfg(feature = "doom")]
    #[inline]
    pub fn fill_rows<F: FnMut(usize, &mut [Cell])>(&mut self, mut f: F) {
        for (cy, row) in self.cur.chunks_exact_mut(self.cols).enumerate() {
            f(cy, row);
        }
    }

    /// One cell of next frame, for a saver that keeps a persistent scene and
    /// touches only what moved. Pairs with `flush_sparse`; mixing it with
    /// `fill` is pointless, not unsound — `fill` overwrites every cell.
    #[inline]
    pub fn set(&mut self, i: usize, c: Cell) {
        self.cur[i] = c;
    }

    /// The cell `set` last wrote. `cells()` is the DRAWN frame and lags this by
    /// a flush, which is the difference that matters to a saver reading its own
    /// scene back on frame 0.
    #[inline]
    pub fn cell(&self, i: usize) -> Cell {
        self.cur[i]
    }

    /// Blit one cell and report its rows. The only code in the program that
    /// writes pixels.
    #[inline]
    fn blit(&self, s: &mut Surface<'_>, pal: &[u32], i: usize) {
        let c = self.cur[i];
        let bits = &font::GLYPHS[c.glyph()];
        let fg = pal[c.colour()];
        let bg = self.ground;
        let (cx, cy) = (i % self.cols, i / self.cols);
        // A shifted grid's first column and row hang off the panel's corner:
        // start them that far into the cell.
        let at = |p: usize, shift: usize| p.checked_sub(shift).map_or((0, shift - p), |q| (q, 0));
        let (x, sx) = at(cx * self.cell_w, self.shift.0);
        let (y, sy) = at(cy * self.cell_h, self.shift.1);
        let mask = &self.mask[sx..];
        for (row, &sr) in s
            .cell_rows(x, y, self.cell_w - sx, self.cell_h - sy)
            .zip(self.rowmap[sy..].iter())
        {
            // & 15 is free and lets the bounds check fold away.
            let line = bits[(sr & (font::GLYPH_H as u8 - 1)) as usize];
            for (out, &m) in row.iter_mut().zip(mask.iter()) {
                *out = if line & m != 0 { fg } else { bg };
            }
        }
    }

    /// Black out the panel the cell grid does not cover: the `w % cell_w` right
    /// strip and the `h % cell_h` bottom strip. Frame 0 only, because nothing
    /// writes there afterwards. It has to go through `Surface` so the strip is
    /// REPORTED as damage — simpledrm scans out of a shadow buffer, so pixels
    /// written but not reported look right in a dump and stay garbage on the
    /// panel. That is the bottom "error line" on matrix/toasters/city.
    #[inline]
    fn paint_margins(&self, s: &mut Surface<'_>) {
        s.fill_outside(
            self.cols * self.cell_w - self.shift.0,
            self.rows * self.cell_h - self.shift.1,
            self.ground,
        );
    }

    /// Blit every changed cell (every cell on frame 0), report exactly the rows
    /// blitted, then swap `cur` into `prev`. Snapshotting is folded in: there is
    /// nothing to misorder.
    pub fn flush(&mut self, s: &mut Surface<'_>, pal: &[u32]) {
        for i in 0..self.cur.len() {
            if self.first || self.cur[i] != self.prev[i] {
                self.blit(s, pal, i);
            }
        }
        if self.first {
            // After the cells, so the strip merges into the frame-0 run
            // instead of opening a second ClipRect.
            self.paint_margins(s);
        }
        self.first = false;
        std::mem::swap(&mut self.cur, &mut self.prev);
    }

    /// Blit exactly the cells named in `dirty` — every cell on frame 0, because
    /// the buffer arrives zeroed. Nothing here is O(cells) after that frame, so
    /// a saver whose scene is static can cost a handful of cells a frame.
    ///
    /// `prev` is kept in step with `cur` rather than swapped: the scene lives in
    /// `cur` between frames, and a swap would hand the saver back the frame
    /// before last. Duplicate indices are allowed and merely blit twice.
    pub fn flush_sparse(&mut self, s: &mut Surface<'_>, pal: &[u32], dirty: &[u32]) {
        debug_assert!(!self.bleed, "flush_sparse cannot follow a shift");
        if self.first {
            for i in 0..self.cur.len() {
                self.blit(s, pal, i);
            }
            self.paint_margins(s);
            self.prev.copy_from_slice(&self.cur);
            self.first = false;
            return;
        }
        for &i in dirty {
            let i = i as usize;
            self.blit(s, pal, i);
            self.prev[i] = self.cur[i];
        }
    }
}

/// Braille bit for a sub-cell of a 2x4 cell. Dot 1 is bit 0 and the numbering
/// runs down the left column (1,2,3), down the right (4,5,6), then the two
/// dot-7/8 feet — which is why row 3 is not `col * 3 + 3`.
#[inline]
pub const fn dot_bit(col: usize, row: usize) -> u8 {
    if row < 3 {
        1u8 << (col * 3 + row)
    } else {
        1u8 << (6 + col)
    }
}

/// RGB -> XRGB8888 (`0x00RRGGBB`) at compile time, so no saver bakes a palette
/// at runtime.
pub const fn bake<const N: usize>(rgb: &[[u8; 3]; N]) -> [u32; N] {
    let mut out = [0u32; N];
    let mut i = 0;
    while i < N {
        out[i] = ((rgb[i][0] as u32) << 16) | ((rgb[i][1] as u32) << 8) | rgb[i][2] as u32;
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::Run;

    /// The knob unset must be a no-op down to the byte: every saver's frames
    /// were dumped before and after this went in and diffed, and that proof is
    /// only worth something if 100 cannot quietly round a cell somewhere.
    #[test]
    fn the_default_aspect_changes_no_geometry_at_all() {
        for (w, h) in [(1920, 1080), (1280, 400), (1918, 1051)] {
            let panel = Panel::new(w, h, w);
            for (cw, ch) in [(8, 16), (16, 32), (12, 16), (4, 4), (24, 24), (8, 128)] {
                let g = Grid::with_aspect(&panel, cw, ch, 100);
                assert_eq!((g.cell_w(), g.cell_h()), (cw, ch), "{w}x{h} cell {cw}x{ch}");
                assert_eq!((g.cols(), g.rows()), ((w / cw).max(1), (h / ch).max(1)));
            }
        }
    }

    /// And set, it has to actually move: the cell gets TALLER, by the rounded
    /// per-cent, and nothing gets wider. A correction applied to the wrong axis
    /// renders exactly as wrong as no correction at all.
    #[test]
    fn a_non_default_aspect_stretches_the_cell_down_and_only_down() {
        let panel = Panel::new(1920, 1080, 1920);
        // 16 x 1.80 = 28.8: rounds UP. Truncating costs a percent of the
        // correction the knob was set to make.
        let g = Grid::with_aspect(&panel, 8, 16, 180);
        assert_eq!((g.cell_w(), g.cell_h()), (8, 29));
        assert_eq!((g.cols(), g.rows()), (240, 1080 / 29));

        // Monotonic, never wider, and never zero however small the knob goes.
        let mut last = 0;
        for aspect in [25, 50, 99, 100, 101, 150, 180, 400] {
            let g = Grid::with_aspect(&panel, 8, 16, aspect);
            assert_eq!(g.cell_w(), 8, "aspect {aspect} moved the width");
            assert!(g.cell_h() >= last, "aspect {aspect} shrank the cell");
            assert!(g.cell_h() >= 1);
            last = g.cell_h();
        }
        assert_eq!(Grid::with_aspect(&panel, 8, 1, 25).cell_h(), 1);
    }

    #[test]
    fn cell_packs_both_fields() {
        let c = Cell::new(1234, 4321);
        assert_eq!((c.glyph(), c.colour()), (1234, 4321));
        assert_ne!(c, Cell::new(1234, 4322));
    }

    /// The LUTs must reproduce the arithmetic the pre-refactor blit did inline,
    /// scaled from an 8x8 source to an 8x16 one: flooring an already-floored
    /// index by two is the same as flooring the original by twice the cell.
    #[test]
    fn lookup_tables_match_the_divides_they_replace() {
        let panel = Panel::new(1920, 1080, 1920);
        for cell in [8usize, 16, 24, 32, 64] {
            let g = Grid::new(&panel, cell, cell);
            for py in 0..cell {
                assert_eq!(g.rowmap[py] as usize, py * font::GLYPH_H / cell);
                assert_eq!(g.rowmap[py] as usize / 2, py * 8 / cell, "cell={cell}");
            }
            for px in 0..cell {
                assert_eq!(g.mask[px], 0x80u8 >> (px * 8 / cell));
            }
        }
    }

    /// The bottom "error line" on the real panel: `h % cell_h` scanlines no
    /// cell covers, left holding whatever was in simpledrm's shadow buffer.
    /// The garbage prefill is the point — a zeroed buffer hides this bug,
    /// which is why it survived every dump.
    #[test]
    fn frame_zero_covers_the_whole_panel_not_just_whole_cells() {
        const JUNK: u32 = 0xDEAD_BEEF;
        // (w, h, cell_w, cell_h): the first two are the shipped savers at
        // 1080, then heights with odd remainders, then one that divides
        // exactly (must not regress), an odd WIDTH, and a panel narrower
        // than one cell.
        for (w, h, cw, ch) in [
            (1920, 1080, 16, 32),
            (1920, 1080, 12, 16),
            (1920, 1050, 16, 32),
            (1920, 800, 12, 16),
            (1024, 768, 16, 16),
            (1918, 1080, 16, 32),
            (90, 100, 64, 64),
        ] {
            let panel = Panel::new(w, h, w);
            let mut g = Grid::new(&panel, cw, ch);
            let mut buf = vec![JUNK; panel.buf_len()];

            g.fill(|_, _| Cell::new(font::SOLID, 0));
            let mut s = Surface::new(&mut buf, &panel);
            g.flush(&mut s, &[0x11]);
            let d = s.finish();

            let case = format!("{w}x{h} cell {cw}x{ch}");
            assert_eq!(
                d.runs(),
                [Run::new(0, 0, w as u16, h as u16)],
                "{case}: frame 0 damage"
            );
            assert_eq!(d.rows(), h, "{case}: frame 0 rows");
            assert!(
                !buf.contains(&JUNK),
                "{case}: {} pixels never painted on frame 0",
                buf.iter().filter(|&&v| v == JUNK).count()
            );

            // And it stays a frame-0 cost.
            let mut s = Surface::new(&mut buf, &panel);
            g.fill(|_, _| Cell::new(font::SOLID, 0));
            g.flush(&mut s, &[0x11]);
            assert!(s.finish().is_empty(), "{case}: idle frame dirtied rows");
        }
    }

    /// Same panel coverage for the sparse path, which has its own frame-0 arm.
    #[test]
    fn sparse_frame_zero_covers_the_whole_panel() {
        const JUNK: u32 = 0xDEAD_BEEF;
        let panel = Panel::new(1920, 1080, 1920);
        let mut g = Grid::new(&panel, 12, 16);
        let mut buf = vec![JUNK; panel.buf_len()];
        g.fill(|_, _| Cell::new(font::SOLID, 0));
        let mut s = Surface::new(&mut buf, &panel);
        g.flush_sparse(&mut s, &[0x11], &[]);
        assert_eq!(s.finish().runs(), [Run::new(0, 0, 1920, 1080)]);
        assert!(!buf.contains(&JUNK));
    }

    /// A shifted grid: every pixel is the cell under it, `shift` pixels in;
    /// a new shift repaints and reports the whole panel, and an unshifted,
    /// unchanged frame still reports nothing. Junk in the "hardware" copy, so
    /// a pixel written but not reported shows.
    #[test]
    fn a_shift_moves_every_cell_and_reports_it() {
        const JUNK: u32 = 0xDEAD_BEEF;
        for (w, h, cw, ch) in [(1920, 1080, 10, 18), (37, 23, 5, 7), (40, 40, 8, 8)] {
            let panel = Panel::new(w, h, w);
            let mut g = Grid::with_aspect(&panel, cw, ch, 100);
            g.reshape_bleed(&panel, cw, ch, 100);
            let (cols, rows) = (g.cols(), g.rows());
            assert!(cols * cw >= w + cw - 1 && rows * ch >= h + ch - 1);
            let mut buf = vec![JUNK; panel.buf_len()];
            let mut hw = buf.clone();
            let pal = [0x11, 0x22, 0x33];
            let cell = |cx: usize, cy: usize| Cell::new(font::SOLID, ((cx + 2 * cy) % 3) as u16);
            for (n, (dx, dy)) in [(0, 0), (3, 1), (3, 1), (cw - 1, ch - 1), (1, 0), (0, 0)]
                .into_iter()
                .enumerate()
            {
                g.shift(dx, dy);
                g.fill(cell);
                let mut s = Surface::new(&mut buf, &panel);
                g.flush(&mut s, &pal);
                let d = s.finish();
                for r in d.runs() {
                    for y in usize::from(r.y0)..usize::from(r.y1) {
                        let row = y * w + usize::from(r.x0)..y * w + usize::from(r.x1);
                        hw[row.clone()].copy_from_slice(&buf[row]);
                    }
                }
                let case = format!("{w}x{h} cell {cw}x{ch} step {n} shift {dx},{dy}");
                assert!(hw == buf, "{case}: drawn but never reported");
                for y in 0..h {
                    for x in 0..w {
                        let c = cell((x + dx) / cw, (y + dy) / ch);
                        assert_eq!(buf[y * w + x], pal[c.colour()], "{case}: pixel {x},{y}");
                    }
                }
                if n == 2 {
                    assert!(d.is_empty(), "{case}: an unmoved frame dirtied rows");
                }
            }
        }
    }

    #[test]
    fn frame_zero_paints_every_cell_then_only_changes() {
        let panel = Panel::new(4, 4, 4);
        let mut g = Grid::new(&panel, 2, 2);
        let mut buf = vec![0u32; panel.buf_len()];

        g.fill(|_, _| Cell::new(font::BLANK, 0));
        let mut s = Surface::new(&mut buf, &panel);
        g.flush(&mut s, &[0x11]);
        // Every cell is blank, but frame 0 still has to cover the panel.
        assert_eq!(s.finish().rows(), 4);

        let mut s = Surface::new(&mut buf, &panel);
        g.fill(|_, _| Cell::new(font::BLANK, 0));
        g.flush(&mut s, &[0x11]);
        assert!(
            s.finish().is_empty(),
            "an unchanged frame must dirty nothing"
        );

        let mut s = Surface::new(&mut buf, &panel);
        g.fill(|cx, cy| Cell::new(font::SOLID, (cx == 1 && cy == 1) as u16));
        g.flush(&mut s, &[0x11, 0x22]);
        assert_eq!(s.finish().rows(), 4);
        assert_eq!(buf[3 * 4 + 3], 0x22);
        assert_eq!(buf[0], 0x11);
    }
}
