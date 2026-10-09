//! Pieces ported from ascii.rest (github.com/bas3line/ascii, MIT, by @bas3line).
//!
//! Upstream a piece is `frame(t) -> string`: a fixed `cols x rows` picture at
//! `t` seconds, optionally with a palette index per cell. Here every piece is
//! composed for the panel it lands on — its grid's shape, landscape through
//! square to portrait — and at upstream's grid with its `UPSTREAM` knobs draws
//! upstream's picture cell for cell; the golden test holds it to that.
//!
//! Two kinds, two savers:
//!
//! * halftone scenes, [`Piece`]s drawn by [`Play`]: every cell a dot whose
//!   size is its brightness — [`halftone::Dots`] holds the dither, the dot
//!   glyphs and the nearest-palette lookup every scene shares. Drawn 1:1, one
//!   picture cell per grid cell, since resampling would smear the ordered
//!   dither: the cell is sized so the panel's shorter side is upstream's 100
//!   cells, and [`tour`] zooms by cell size, never by resampling;
//! * text pieces, [`Canvas`]es drawn by [`Fill`]: characters in a fixed text
//!   cell — [`text::cell`] maps a char to its glyph.
//!
//! [`Play`] and [`Fill`] do everything else — the clock, the upstream frame
//! rate, the ground colour and the flush — so a port is only its own drawing.

// Ports keep upstream's literals (`6.28`, `3.14`) rather than TAU/PI: the
// golden test compares against what upstream computes, not what it meant.
#![allow(clippy::approx_constant)]
// Same reason for the control flow: a port mirrors upstream's `if`/`else`
// order, comparison chains and literals line for line, so it diffs cleanly
// against the JavaScript it has to match bit for bit.
#![allow(
    clippy::if_not_else,
    clippy::comparison_chain,
    clippy::single_match_else,
    clippy::decimal_bitwise_operands,
    clippy::range_plus_one
)]

pub mod halftone;
pub mod math;
pub mod stretch;
pub mod text;
pub mod title;
pub mod tour;

/// The one list of pieces: their modules, their `saver::SAVERS` rows and their
/// tests all come from it. `scenes` are halftone [`Piece`]s drawn by [`Play`],
/// `text` the character [`Canvas`]es drawn by [`Fill`].
macro_rules! each_piece {
    ($cb:ident) => {
        $cb! {
            scenes: [
                alpine_dawn::AlpineDawn,
                aurora_fjord::AuroraFjord,
                deep_reef::DeepReef,
                desert_night::DesertNight,
                earthrise::Earthrise,
                kyoto_dusk::KyotoDusk,
                marine_drive::MarineDrive,
                misty_forest::MistyForest,
                night_coast::NightCoast,
                ocean_sunset::OceanSunset,
                storm_plains::StormPlains,
                taj_dawn::TajDawn,
                varanasi_ghats::VaranasiGhats,
            ],
            text: [
                aurora::Aurora,
                double_pendulum::DoublePendulum,
                fractal_tree::FractalTree,
                lighthouse::Lighthouse,
                reaction_diffusion::ReactionDiffusion,
                synthwave::Synthwave,
                tv_static::TvStatic,
                vinyl::Vinyl,
            ],
        }
    };
}
pub(crate) use each_piece;

macro_rules! declare {
    (
        scenes: [$($sm:ident::$st:ident),* $(,)?],
        text: [$($tm:ident::$tt:ident),* $(,)?] $(,)?
    ) => {
        $(pub mod $sm;)*
        $(pub mod $tm;)*

        #[cfg(test)]
        mod piece_tests {
            $(
                mod $sm {
                    use crate::ascii_rest::tests;
                    type P = crate::ascii_rest::$sm::$st;

                    #[test]
                    fn exercise() {
                        tests::exercise::<P>();
                    }

                    #[test]
                    #[ignore = "needs ASCII_REST_GOLDEN; see tools/ascii-rest-golden.ts"]
                    fn golden() {
                        tests::golden::<P>();
                    }

                    #[test]
                    fn upstream_knobs_change_the_picture() {
                        tests::upstream_knobs_change_the_picture::<P>();
                    }
                }
            )*
            $(
                mod $tm {
                    use crate::ascii_rest::tests;
                    type P = crate::ascii_rest::$tm::$tt;

                    #[test]
                    fn exercise() {
                        tests::exercise_fill::<P>();
                    }

                    #[test]
                    #[ignore = "needs ASCII_REST_GOLDEN; see tools/ascii-rest-golden.ts"]
                    fn golden() {
                        tests::golden_fill::<P>();
                    }

                    #[test]
                    fn upstream_knobs_change_the_picture() {
                        tests::upstream_knobs_change_the_canvas::<P>();
                    }
                }
            )*
        }
    };
}
each_piece!(declare);

use crate::grid::{pixel_aspect, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use title::Title;
use tour::{Knobs, Touring, View};

/// A halftone scene, composed for whatever picture [`Play`] gives it: the
/// shorter side upstream's 100 cells, the longer the panel's. Associated
/// consts rather than `&self` methods, so [`Play`] reads them with no dispatch.
pub trait Piece: Sized + 'static {
    /// The saver name, upstream's slug.
    const NAME: &'static str;
    /// Upstream's grid. Built at it with the `UPSTREAM` knobs, `frame` draws
    /// upstream's picture cell for cell.
    #[cfg(test)]
    const COLS: usize = SHORT * 2;
    #[cfg(test)]
    const ROWS: usize = SHORT;
    /// Upstream's frame rate. `frame` sees `t` on this clock's ticks however
    /// fast the panel runs, so a 15 fps scene costs 15 shades a second at
    /// `SAVER_FPS=30`, not 30.
    const FPS: u32;
    const PALETTE: &'static [u32];
    /// What shows between dots.
    const GROUND: u32;
    /// Knob values that draw upstream's picture, for the golden test: a
    /// port's own additions default on.
    #[cfg(test)]
    const UPSTREAM: &'static [(&'static str, &'static str)] = &[];

    /// A `cols x rows` picture: both at least [`SHORT`], one of them exactly
    /// that on any panel bigger than a thumbnail.
    fn new(cols: usize, rows: usize) -> Self;

    /// Write every one of the `cols * rows` cells of the picture at `t`.
    fn frame(&mut self, t: f64, out: &mut [Cell]);
}

/// A scene picture's shorter side, in cells: upstream's rows.
pub const SHORT: usize = 100;

/// A text piece composed for whatever grid the panel has: `cols x rows` is
/// the grid's, every cell is the picture's, and there is nothing to fit.
pub trait Canvas: Sized + 'static {
    const NAME: &'static str;
    /// Upstream's grid. Built at it with the `UPSTREAM` knobs, `frame` draws
    /// upstream's picture cell for cell — the golden test holds it to that.
    #[cfg(test)]
    const COLS: usize;
    #[cfg(test)]
    const ROWS: usize;
    const FPS: u32;
    /// A text cell, two widths tall; here so the mirror groups it as text.
    const CELL: usize = 2;
    /// The knob that turns colour on (the default): cells index `PALETTE`
    /// over `GROUND`. Off is upstream's `INK` on black.
    const COLOR: &'static str;
    const PALETTE: &'static [u32];
    /// Upstream's one ink: the whole palette with colour off.
    const INK: u32;
    const GROUND: u32 = 0;
    /// Knob values beyond `COLOR=0` that draw upstream's picture: a port's
    /// own additions, default on.
    #[cfg(test)]
    const UPSTREAM: &'static [(&'static str, &'static str)] = &[];

    fn new(cols: usize, rows: usize, colour: bool) -> Self;

    /// Write every one of the `cols * rows` cells of the picture at `t`.
    fn frame(&mut self, t: f64, out: &mut [Cell]);
}

/// Upstream's frame rate on the panel's: which tick of a piece's own clock
/// a panel frame shows.
struct Clock {
    frames: u64,
    fps: u64,
    tick: u64,
}

impl Clock {
    fn new(fps: u32) -> Self {
        Self {
            frames: 0,
            fps: u64::from(fps.max(1)),
            tick: u64::MAX,
        }
    }

    /// The piece's `t` when this panel frame lands on a new tick of its
    /// `piece_fps` clock, else None: the last picture still stands.
    #[inline]
    fn next(&mut self, piece_fps: u32) -> Option<f64> {
        let tick = self.frames * u64::from(piece_fps) / self.fps;
        self.frames += 1;
        (tick != self.tick).then(|| {
            self.tick = tick;
            tick as f64 / f64::from(piece_fps)
        })
    }
}

/// `P`'s colour knob.
fn colour_of<P: Canvas>() -> bool {
    crate::env_num(&[P::COLOR], 1, 0, 1) == 1
}

/// The saver for any [`Canvas`]: one grid the size of the panel, in text
/// cells of `ASCII_REST_TEXT_CELL_W x _H` glass pixels.
pub struct Fill<P: Canvas> {
    piece: P,
    palette: &'static [u32],
    pic: Vec<Cell>,
    grid: Grid,
    title: Option<Title>,
    clock: Clock,
}

impl<P: Canvas> Fill<P> {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        let w = crate::env_num(&["ASCII_REST_TEXT_CELL_W"], 12, 4, 64) as usize;
        let h = crate::env_num(&["ASCII_REST_TEXT_CELL_H"], 24, 8, 128) as usize;
        let grid = Grid::new(panel, w, h);
        let (cols, rows) = (grid.cols(), grid.rows());
        let colour = colour_of::<P>();
        let piece = P::new(cols, rows, colour);
        let (palette, ground) = if colour {
            (P::PALETTE, P::GROUND)
        } else {
            (const { &[P::INK] as &[u32] }, 0)
        };
        let grid = grid.with_ground(ground);
        let title = (crate::env_num(&["ASCII_REST_TITLE"], 0, 0, 1) == 1)
            .then(|| Title::new(P::NAME, P::CELL, palette));
        Self {
            piece,
            palette,
            pic: vec![Cell::CLEAR; cols * rows],
            grid,
            title,
            clock: Clock::new(fps),
        }
    }

    pub fn build(panel: &Panel, fps: u32) -> Box<dyn Saver> {
        Box::new(Self::new(panel, fps))
    }
}

impl<P: Canvas> Saver for Fill<P> {
    fn render(&mut self, s: &mut Surface<'_>) {
        if let Some(t) = self.clock.next(P::FPS) {
            self.piece.frame(t, &mut self.pic);
        }
        let (pic, cols) = (&self.pic, self.grid.cols());
        self.grid.fill(|c, r| pic[r * cols + c]);
        if let Some(title) = &self.title {
            title.stamp(&mut self.grid);
        }
        self.grid.flush(s, self.palette);
    }

    fn name(&self) -> &'static str {
        P::NAME
    }

    fn grid(&self) -> &Grid {
        &self.grid
    }

    fn palette(&self) -> &[u32] {
        self.palette
    }
}

/// The saver for any [`Piece`].
pub struct Play<P: Piece> {
    piece: P,
    pic: Vec<Cell>,
    /// What the panel shows.
    cam: Camera,
    tour: Option<Touring>,
    /// `ASCII_REST_TITLE`: stamped over the camera's grid after the picture
    /// is mapped onto it.
    title: Option<Title>,
    clock: Clock,
}

/// One grid looking at the picture through a [`View`].
struct Camera {
    grid: Grid,
    view: View,
    /// A grid with bleed, for a view that pans by the pixel.
    bleed: bool,
    /// The picture's size, in cells.
    cols: usize,
    rows: usize,
    /// Grid column/row -> picture column/row, `u32::MAX` off the picture.
    xmap: Vec<u32>,
    ymap: Vec<u32>,
}

impl Camera {
    /// Looking at a `pic`-sized picture through `v`, with buffers for every
    /// view from cell width `lo` to `hi` so that aiming between them
    /// allocates nothing.
    fn new<P: Piece>(
        panel: &Panel,
        aspect: usize,
        pic: (usize, usize),
        v: View,
        (lo, hi): (usize, usize),
        bleed: bool,
    ) -> Self {
        // Widest cell first for the longest glyph LUTs, then the narrowest for
        // the most cells.
        let mut grid = Grid::with_aspect(panel, hi, hi, aspect).with_ground(P::GROUND);
        if bleed {
            grid.reshape_bleed(panel, lo, lo, aspect);
        } else {
            grid.reshape(panel, lo, lo, aspect);
        }
        let mut cam = Self {
            xmap: Vec::with_capacity(grid.cols()),
            ymap: Vec::with_capacity(grid.rows()),
            grid,
            view: View { w: lo, ..v },
            bleed,
            cols: pic.0,
            rows: pic.1,
        };
        cam.aim(panel, aspect, v);
        cam
    }

    /// Look through `v`. A new cell width or pixel shift repaints the whole
    /// panel; a pan by whole cells alone is an ordinary change-detected frame.
    fn aim(&mut self, panel: &Panel, aspect: usize, v: View) {
        if v.w != self.view.w {
            if self.bleed {
                self.grid.reshape_bleed(panel, v.w, v.w, aspect);
            } else {
                self.grid.reshape(panel, v.w, v.w, aspect);
            }
        }
        self.grid.shift(v.dx, v.dy);
        self.view = v;
        map_into(&mut self.xmap, self.grid.cols(), self.cols, v.x0);
        map_into(&mut self.ymap, self.grid.rows(), self.rows, v.y0);
    }

    /// Show what `cam` shows, in this camera's own geometry: each cell takes
    /// the picture cell under its centre on `cam`'s grid, ground where that
    /// is `cam`'s margin or off the picture. Allocates nothing.
    fn follow(&mut self, cam: &Camera) {
        let (me, it) = (self.grid.shape_of(), cam.grid.shape_of());
        let (dx, dy) = cam.grid.shift_of();
        follow_into(
            &mut self.xmap,
            me.cols,
            me.cell_w,
            (it.cell_w, dx),
            &cam.xmap,
        );
        follow_into(
            &mut self.ymap,
            me.rows,
            me.cell_h,
            (it.cell_h, dy),
            &cam.ymap,
        );
    }

    #[inline]
    fn draw(&mut self, pic: &[Cell]) {
        let (xmap, ymap, cols) = (&self.xmap, &self.ymap, self.cols);
        self.grid.fill(|cx, cy| {
            let (x, y) = (xmap[cx], ymap[cy]);
            if x == u32::MAX || y == u32::MAX {
                Cell::CLEAR
            } else {
                pic[y as usize * cols + x as usize]
            }
        });
    }
}

impl<P: Piece> Play<P> {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        let mut play = Self::with_tour(panel, fps, Knobs::from_env());
        if crate::env_num(&["ASCII_REST_TITLE"], 0, 0, 1) == 1 {
            play.title = Some(Title::new(P::NAME, 1, P::PALETTE));
        }
        play
    }

    fn with_tour(panel: &Panel, fps: u32, knobs: Option<Knobs>) -> Self {
        // Cell widths are square-glass units; Grid stretches the height by the
        // panel's pixel aspect itself.
        let aspect = pixel_aspect();
        let (w, cols, rows) = picture(panel, aspect);
        let s = Grid::shape(panel, w, w, aspect);
        let base = View {
            w,
            x0: origin(cols, s.cols),
            y0: origin(rows, s.rows),
            dx: 0,
            dy: 0,
        };
        let pic = (cols, rows);
        let tour = knobs.map(|k| Touring::new::<P>(panel, aspect, pic, base, fps.max(1), k));
        let widths = tour.as_ref().map_or((w, w), Touring::widths);
        Self {
            piece: P::new(cols, rows),
            pic: vec![Cell::CLEAR; cols * rows],
            cam: Camera::new::<P>(panel, aspect, pic, base, widths, tour.is_some()),
            tour,
            title: None,
            clock: Clock::new(fps),
        }
    }

    /// A `saver::SAVERS` row's builder: `(Piece::NAME, Play::<Piece>::build)`.
    pub fn build(panel: &Panel, fps: u32) -> Box<dyn Saver> {
        Box::new(Self::new(panel, fps))
    }
}

/// The cell width a scene draws at and its picture's `cols x rows`: the
/// narrowest cell whose grid's shorter side is at most [`SHORT`], so a
/// picture of that grid's shape with that side [`SHORT`] covers it, cropping
/// the row or two whole pixels could not place. On pine that is upstream's
/// own 100 rows, 320 wide.
fn picture(panel: &Panel, aspect: usize) -> (usize, usize, usize) {
    let short = |w: usize| {
        let s = Grid::shape(panel, w, w, aspect);
        s.cols.min(s.rows)
    };
    let mut w = 1;
    while short(w) > SHORT {
        w += 1;
    }
    let s = Grid::shape(panel, w, w, aspect);
    (w, s.cols.max(SHORT), s.rows.max(SHORT))
}

/// The picture cell under slot 0 of an `n`-slot window centred on a
/// `len`-cell picture, negative when the window is the wider.
fn origin(len: usize, n: usize) -> isize {
    if n >= len {
        -(((n - len) / 2) as isize)
    } else {
        ((len - n) / 2) as isize
    }
}

/// Map `n` grid slots onto a picture `len` long whose cell `x0` is under slot
/// 0, `u32::MAX` off the picture. Into `out`, which allocates nothing once it
/// has held `n` slots.
fn map_into(out: &mut Vec<u32>, n: usize, len: usize, x0: isize) {
    out.clear();
    out.extend((0..n as isize).map(|i| {
        let p = i + x0;
        if p >= 0 && (p as usize) < len {
            p as u32
        } else {
            u32::MAX
        }
    }));
}

/// `n` slots of `size` pixels, each mapped as the slot of `map`'s grid under
/// its centre maps: `of`-pixel slots drawn `shift` pixels before the panel's
/// edge. Equal sizes, unshifted, copy `map`.
fn follow_into(
    out: &mut Vec<u32>,
    n: usize,
    size: usize,
    (of, shift): (usize, usize),
    map: &[u32],
) {
    out.clear();
    out.extend((0..n).map(|i| {
        map.get(((2 * i + 1) * size / 2 + shift) / of)
            .copied()
            .unwrap_or(u32::MAX)
    }));
}

impl<P: Piece> Saver for Play<P> {
    fn render(&mut self, s: &mut Surface<'_>) {
        let next = self.clock.next(P::FPS);
        let ticked = next.is_some();
        if let Some(t) = next {
            self.piece.frame(t, &mut self.pic);
        }
        if let Some(t) = &mut self.tour {
            t.steer(&self.pic, &mut self.cam, ticked);
        }
        self.cam.draw(&self.pic);
        if let Some(title) = &self.title {
            title.stamp(&mut self.cam.grid);
        }
        self.cam.grid.flush(s, P::PALETTE);
    }

    fn name(&self) -> &'static str {
        P::NAME
    }

    fn grid(&self) -> &Grid {
        &self.cam.grid
    }

    fn mirror(&mut self) -> &Grid {
        match &mut self.tour {
            Some(t) => t.mirror(&self.pic, &self.cam, self.title.as_ref()),
            None => &self.cam.grid,
        }
    }

    fn palette(&self) -> &[u32] {
        P::PALETTE
    }
}

/// `"#rrggbb"` -> XRGB8888 at compile time, so palettes stay upstream's
/// strings and a typo is a build error.
pub const fn hex(s: &str) -> u32 {
    let b = s.as_bytes();
    assert!(b.len() == 7 && b[0] == b'#', "colour must be #rrggbb");
    let mut v = 0u32;
    let mut i = 1;
    while i < 7 {
        let d = match b[i] {
            c @ b'0'..=b'9' => c - b'0',
            c @ b'a'..=b'f' => c - b'a' + 10,
            c @ b'A'..=b'F' => c - b'A' + 10,
            _ => panic!("colour must be #rrggbb"),
        };
        v = v << 4 | d as u32;
        i += 1;
    }
    v
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::config;
    use crate::glyph;
    use crate::testalloc::allocs_during;

    #[test]
    fn hex_parses_upstream_colours() {
        assert_eq!(hex("#080b12"), 0x08_0b_12);
        assert_eq!(hex("#FFE9AE"), 0xff_e9_ae);
    }

    fn place(n: usize, len: usize) -> Vec<u32> {
        let mut v = Vec::new();
        map_into(&mut v, n, len, origin(len, n));
        v
    }

    #[test]
    fn place_pads_and_crops_around_the_centre() {
        assert_eq!(place(5, 3), [u32::MAX, 0, 1, 2, u32::MAX]);
        assert_eq!(place(3, 5), [1, 2, 3]);
        assert_eq!(place(3, 3), [0, 1, 2]);
    }

    /// Every panel shape the scenes are tested on, as `(w, h, pixel aspect)`:
    /// pine's 3.2:1 glass, 16:9, 1280x400's 3.2:1, 4:3, square, portrait, and
    /// the 128px square the saver tests build at.
    pub const SHAPES: [(usize, usize, usize); 8] = [
        (1920, 1080, 180),
        (1920, 1080, 100),
        (1280, 400, 100),
        (1024, 768, 100),
        (1080, 1080, 100),
        (1080, 1920, 100),
        (800, 1280, 100),
        (128, 128, 100),
    ];

    /// The picture covers the grid with a row or two to spare at most, one
    /// side is upstream's 100 cells, and the other follows the panel: pine
    /// gets the 320x100 upstream's rows make at 3.2:1.
    #[test]
    fn a_scene_picture_is_the_panels_shape_at_upstreams_scale() {
        for (pw, ph, aspect) in SHAPES {
            let panel = Panel::new(pw, ph, pw);
            let (w, cols, rows) = picture(&panel, aspect);
            let s = Grid::shape(&panel, w, w, aspect);
            let at = format!("{pw}x{ph}@{aspect}: {cols}x{rows} on {s:?}");
            assert!(cols >= s.cols && rows >= s.rows, "{at}: short");
            assert!(cols.min(rows) == SHORT || w == 1, "{at}: scale");
            assert!(
                cols - s.cols <= 4 && rows - s.rows <= 4 || pw < 200,
                "{at}: crop"
            );
        }
        let pine = Panel::new(1920, 1080, 1920);
        assert_eq!(picture(&pine, 180), (6, 320, 100));
        assert_eq!(picture(&Panel::new(2000, 1000, 2000), 100), (10, 200, 100));
    }

    /// The checks every scene gets, on every panel shape: the picture fills
    /// the panel's grid with in-range glyphs and colours, reaches both sides,
    /// moves, and steady-state frames never allocate.
    pub fn exercise<P: Piece>() {
        use crate::grid::with_test_aspect;
        for (w, h, aspect) in SHAPES {
            let panel = Panel::new(w, h, w);
            let mut play = with_test_aspect(aspect, || Play::<P>::with_tour(&panel, P::FPS, None));
            let mut buf = vec![0u32; panel.buf_len()];
            let mut frame = |play: &mut Play<P>| {
                let mut s = Surface::new(&mut buf, &panel);
                play.render(&mut s);
                s.finish();
            };
            let at = format!("{} {w}x{h}@{aspect}", P::NAME);
            let cam = &play.cam;
            assert!(
                cam.xmap.iter().chain(&cam.ymap).all(|&i| i != u32::MAX),
                "{at}: the picture does not cover the grid"
            );
            frame(&mut play);
            let first = play.pic.clone();
            for c in &first {
                assert!(c.glyph() < crate::font::GLYPHS.len(), "{at}: glyph");
                assert!(c.colour() < P::PALETTE.len(), "{at}: colour");
            }
            let cols = play.cam.cols;
            let lit = |c: &Cell| c.glyph() != usize::from(crate::font::HALFTONE[0]);
            let edge = cols / 8 + 1;
            let reach = [
                first.chunks(cols).any(|r| r[..edge].iter().any(lit)),
                first.chunks(cols).any(|r| r[cols - edge..].iter().any(lit)),
            ];
            assert_eq!(reach, [true; 2], "{at}: a side stays empty");
            for _ in 0..P::FPS * 2 {
                frame(&mut play);
            }
            assert_ne!(first, play.pic, "{at}: nothing moved in two seconds");
            let n = allocs_during(|| {
                for _ in 0..P::FPS {
                    frame(&mut play);
                }
            });
            assert_eq!(n, 0, "{at}: render allocated");
        }
    }

    /// A [`Canvas`]'s checks, on every panel shape down to the 128px square
    /// the saver tests build at — pine's 3.2:1, 16:9, 4:3, square, portrait:
    /// its grid covers the panel, the picture is in range and not blank, it
    /// moves, and steady-state frames never allocate. On the landscape
    /// panels it must also reach both sides: a responsive piece that left
    /// the edges empty would be upstream's fixed picture again.
    pub fn exercise_fill<P: Canvas>() {
        use crate::grid::with_test_aspect;
        for (w, h, aspect) in SHAPES {
            let panel = Panel::new(w, h, w);
            let mut fill = with_test_aspect(aspect, || Fill::<P>::new(&panel, P::FPS));
            let mut buf = vec![0u32; panel.buf_len()];
            let mut frame = |fill: &mut Fill<P>| {
                let mut s = Surface::new(&mut buf, &panel);
                fill.render(&mut s);
                s.finish();
            };
            let at = format!("{} {w}x{h}@{aspect}", P::NAME);
            let (cols, cw) = (fill.grid.cols(), fill.grid.cell_w());
            let (rows, ch) = (fill.grid.rows(), fill.grid.cell_h());
            assert!(cols * cw + cw > w && rows * ch + ch > h, "{at}: grid short");
            frame(&mut fill);
            let first = fill.pic.clone();
            for c in &first {
                assert!(c.glyph() < crate::font::GLYPHS.len(), "{at}: glyph");
                assert!(c.colour() < fill.palette.len(), "{at}: colour");
            }
            assert!(first.iter().any(|c| *c != Cell::CLEAR), "{at}: blank");
            let blank = text::cell(' ');
            let mut reach = [false; 2];
            for _ in 0..P::FPS * 4 {
                frame(&mut fill);
                for r in 0..rows {
                    let row = &fill.pic[r * cols..(r + 1) * cols];
                    let edge = cols / 8 + 1;
                    reach[0] |= row[..edge].iter().any(|c| *c != blank);
                    reach[1] |= row[cols - edge..].iter().any(|c| *c != blank);
                }
            }
            assert_ne!(first, fill.pic, "{at}: nothing moved in four seconds");
            if w > h && w >= 1024 {
                assert_eq!(reach, [true; 2], "{at}: a side stays empty");
            }
            let n = allocs_during(|| {
                for _ in 0..P::FPS {
                    frame(&mut fill);
                }
            });
            assert_eq!(n, 0, "{at}: render allocated");
        }
    }

    /// `build` run with `knobs` set, each checked against what it really
    /// reads.
    fn with_knobs<T>(name: &str, knobs: &[(&str, &str)], build: impl Fn() -> T) -> T {
        let _lock = config::SHARED_KNOBS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let read = config::discover(|| drop(build()));
        for (key, value) in knobs {
            let knob = read.iter().find(|k| k.key == *key);
            let knob = knob.unwrap_or_else(|| panic!("{name}: reads no {key}"));
            config::set(knob, value).unwrap();
        }
        let built = build();
        for (key, _) in knobs {
            config::reset(key);
        }
        built
    }

    /// `P` built with its `UPSTREAM` knobs.
    fn upstream<P: Piece>() -> P {
        with_knobs(P::NAME, P::UPSTREAM, || P::new(P::COLS, P::ROWS))
    }

    /// The upstream knobs are the port's own picture switched off, so they
    /// must change it: a no-op entry would leave the golden test vacuous.
    pub fn upstream_knobs_change_the_picture<P: Piece>() {
        if P::UPSTREAM.is_empty() {
            return;
        }
        fn pic<P: Piece>(mut p: P) -> Vec<Cell> {
            let mut out = vec![Cell::CLEAR; P::COLS * P::ROWS];
            p.frame(4.0, &mut out);
            out
        }
        assert_ne!(
            pic(with_knobs(P::NAME, &[], || P::new(P::COLS, P::ROWS))),
            pic(upstream::<P>()),
            "{}",
            P::NAME
        );
    }

    /// `P` at upstream's grid, with `knobs` set.
    fn canvas<P: Canvas>(knobs: &[(&str, &str)]) -> P {
        with_knobs(P::NAME, knobs, || {
            P::new(P::COLS, P::ROWS, super::colour_of::<P>())
        })
    }

    /// `P` at upstream's grid with colour off and its `UPSTREAM` knobs.
    fn upstream_canvas<P: Canvas>() -> P {
        let knobs: Vec<_> = std::iter::once((P::COLOR, "0"))
            .chain(P::UPSTREAM.iter().copied())
            .collect();
        canvas(&knobs)
    }

    /// A canvas's colour, at least, is its own: switching to upstream's
    /// knobs must change its picture at upstream's grid.
    pub fn upstream_knobs_change_the_canvas<P: Canvas>() {
        let pic = |mut p: P| {
            let mut out = vec![Cell::CLEAR; P::COLS * P::ROWS];
            p.frame(4.0, &mut out);
            out
        };
        assert_ne!(
            pic(canvas::<P>(&[])),
            pic(upstream_canvas::<P>()),
            "{}",
            P::NAME
        );
    }

    /// [`golden_cells`] for a [`Piece`].
    pub fn golden<P: Piece>() {
        let mut piece = upstream::<P>();
        golden_cells(P::NAME, [P::COLS, P::ROWS, P::FPS as usize], 1, |t, pic| {
            piece.frame(t, pic);
        });
    }

    /// [`golden_cells`] for a [`Canvas`], built at upstream's grid.
    pub fn golden_fill<P: Canvas>() {
        let mut piece = upstream_canvas::<P>();
        golden_cells(
            P::NAME,
            [P::COLS, P::ROWS, P::FPS as usize],
            P::CELL,
            |t, pic| {
                piece.frame(t, pic);
            },
        );
    }

    /// Compare the port with upstream's own output, written by
    /// `tools/ascii-rest-golden.ts` into `$ASCII_REST_GOLDEN/<name>.golden`.
    /// Ignored by default: it needs bun and an upstream checkout.
    ///
    /// Exact, cell for cell: `math` reproduces JavaScriptCore wherever libm
    /// differs, so one stray cell is a port bug, not rounding noise.
    fn golden_cells(
        name: &str,
        geometry: [usize; 3],
        cell: usize,
        mut frame: impl FnMut(f64, &mut [Cell]),
    ) {
        let [cols, rows, fps] = geometry;
        let dir = std::env::var("ASCII_REST_GOLDEN").expect("set ASCII_REST_GOLDEN");
        let path = format!("{dir}/{name}.golden");
        let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let mut lines = src.lines();
        let head: Vec<usize> = lines
            .next()
            .unwrap()
            .split(' ')
            .map(|v| v.parse().unwrap())
            .collect();
        assert_eq!(head, geometry, "{path}: geometry");
        let halftone = cell == 1;
        let glyph = |c: char| -> u16 {
            if halftone {
                let i = " ·•●".chars().position(|d| d == c).expect("halftone char");
                crate::font::HALFTONE[i]
            } else {
                glyph::of(c)
            }
        };
        let mut pic = vec![Cell::CLEAR; cols * rows];
        let mut tick = 0u64;
        let mut differ = 0usize;
        while let Some(at) = lines.next() {
            let want: u64 = at.strip_prefix('@').unwrap().parse().unwrap();
            while tick <= want {
                frame(tick as f64 / fps as f64, &mut pic);
                tick += 1;
            }
            let mut expect = vec![Cell::CLEAR; cols * rows];
            for r in 0..rows {
                let row: Vec<char> = lines.next().unwrap().chars().collect();
                assert_eq!(row.len(), cols, "{path} @{want} row {r} width");
                for (x, &c) in row.iter().enumerate() {
                    expect[r * cols + x] = Cell::new(glyph(c), 0);
                }
            }
            if halftone {
                for r in 0..rows {
                    let row = lines.next().unwrap().as_bytes();
                    for x in 0..cols {
                        let h = std::str::from_utf8(&row[x * 2..x * 2 + 2]).unwrap();
                        let e = &mut expect[r * cols + x];
                        *e = Cell::new(e.glyph() as u16, u16::from_str_radix(h, 16).unwrap());
                    }
                }
            }
            let bad: Vec<usize> = (0..expect.len()).filter(|&i| expect[i] != pic[i]).collect();
            differ += bad.len();
            println!(
                "{} @{want}: {} of {} cells differ{}",
                name,
                bad.len(),
                expect.len(),
                bad.first()
                    .map(|&i| format!(
                        ", first at col {} row {}: want {:?} got {:?}",
                        i % cols,
                        i / cols,
                        (expect[i].glyph(), expect[i].colour()),
                        (pic[i].glyph(), pic[i].colour())
                    ))
                    .unwrap_or_default()
            );
        }
        assert_eq!(differ, 0, "{name}: cells differ from upstream");
    }
}
