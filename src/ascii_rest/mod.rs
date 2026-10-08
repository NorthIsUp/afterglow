//! Pieces ported from ascii.rest (github.com/bas3line/ascii, MIT, by @bas3line).
//!
//! Upstream a piece is `frame(t) -> string`: a fixed `cols x rows` picture at
//! `t` seconds, optionally with a palette index per cell. Here a piece writes
//! that picture as [`Cell`]s and [`Play`] does everything else — the clock, the
//! upstream frame rate, centring on the panel, the ground colour and the flush —
//! so a port is only its own drawing code.
//!
//! Two kinds, two helpers:
//!
//! * halftone scenes (`cell: 1`, a palette): every cell a dot whose size is its
//!   brightness — [`halftone::Dots`] holds the dither, the dot glyphs and the
//!   nearest-palette lookup every scene shares;
//! * text pieces (`cell: 2`, one ink): characters — [`text::cell`] maps a
//!   char to its glyph.
//!
//! The picture is drawn 1:1, one piece cell per grid cell, in the largest cell
//! that fits it on the glass: resampling would smear the scenes' ordered
//! dither. Pine's panel is 3.2:1 and the scenes 2:1, so scenes fill it and crop
//! the rows each scene can spare ([`Fit::Cover`]); text pieces stay whole.
//! Cover pieces then tour: [`tour`] zooms by cell size, never by resampling.

// Ports keep upstream's literals (`6.28`, `3.14`) rather than TAU/PI: the
// golden test compares against what upstream computes, not what it meant.
#![allow(clippy::approx_constant)]

pub mod halftone;
pub mod math;
pub mod text;
pub mod tour;

/// The one list of pieces. A port is a row here plus its file: its module, its
/// `saver::SAVERS` row and its tests all come from this list.
macro_rules! each_piece {
    ($cb:ident) => {
        $cb! {
            alpine_dawn::AlpineDawn,
            alpine_dawn_wide::AlpineDawnWide,
            aurora::Aurora,
            aurora_fjord::AuroraFjord,
            aurora_fjord_wide::AuroraFjordWide,
            deep_reef::DeepReef,
            deep_reef_wide::DeepReefWide,
            desert_night::DesertNight,
            desert_night_wide::DesertNightWide,
            double_pendulum::DoublePendulum,
            earthrise::Earthrise,
            earthrise_wide::EarthriseWide,
            fractal_tree::FractalTree,
            kyoto_dusk::KyotoDusk,
            kyoto_dusk_wide::KyotoDuskWide,
            lighthouse::Lighthouse,
            marine_drive::MarineDrive,
            marine_drive_wide::MarineDriveWide,
            misty_forest::MistyForest,
            misty_forest_wide::MistyForestWide,
            night_coast::NightCoast,
            night_coast_wide::NightCoastWide,
            ocean_sunset::OceanSunset,
            ocean_sunset_wide::OceanSunsetWide,
            reaction_diffusion::ReactionDiffusion,
            storm_plains::StormPlains,
            storm_plains_wide::StormPlainsWide,
            synthwave::Synthwave,
            taj_dawn::TajDawn,
            taj_dawn_wide::TajDawnWide,
            tv_static::TvStatic,
            varanasi_ghats::VaranasiGhats,
            varanasi_ghats_wide::VaranasiGhatsWide,
            vinyl::Vinyl,
        }
    };
}
pub(crate) use each_piece;

macro_rules! declare {
    ($($m:ident::$t:ident),* $(,)?) => {
        $(pub mod $m;)*

        #[cfg(test)]
        mod piece_tests {
            $(
                mod $m {
                    use crate::ascii_rest::tests;

                    #[test]
                    fn exercise() {
                        tests::exercise::<crate::ascii_rest::$m::$t>();
                    }

                    #[test]
                    #[ignore = "needs ASCII_REST_GOLDEN; see tools/ascii-rest-golden.ts"]
                    fn golden() {
                        tests::golden::<crate::ascii_rest::$m::$t>();
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
use tour::{Knobs, Tour, View};

/// One ascii.rest piece. Associated consts rather than `&self` methods, so
/// [`Play`] reads them with no dispatch.
pub trait Piece: Sized + 'static {
    /// The saver name, upstream's slug.
    const NAME: &'static str;
    const COLS: usize;
    const ROWS: usize;
    /// Upstream's frame rate. `frame` sees `t` on this clock's ticks however
    /// fast the panel runs, so a 15 fps scene costs 15 shades a second at
    /// `SAVER_FPS=30`, not 30.
    const FPS: u32;
    /// Cell height in cell widths on the glass: 1 for scenes, 2 for text.
    const CELL: usize;
    const PALETTE: &'static [u32];
    /// What shows between dots and around the picture.
    const GROUND: u32;
    /// How the picture meets a panel of another shape.
    const FIT: Fit = Fit::Contain;
    /// False for this repo's own pieces (the `-wide` recompositions), which
    /// have no upstream output for the golden test to compare against. Only
    /// the test build reads it.
    #[cfg_attr(not(test), allow(dead_code))]
    const UPSTREAM: bool = true;

    fn new() -> Self;

    /// Write every one of the `COLS * ROWS` cells of the picture at `t`.
    fn frame(&mut self, t: f64, out: &mut [Cell]);
}

/// How a piece's fixed `COLS x ROWS` picture meets the panel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fit {
    /// All of it, centred over the ground. Text pieces: crop one and words go
    /// missing.
    Contain,
    /// Fill the panel and crop the overflow. The width always crops evenly;
    /// `anchor` places the kept band of rows, 0.0 the top of the picture, 1.0
    /// the bottom, so a scene keeps its horizon rather than empty sky.
    Cover { anchor: f64 },
}

/// The saver for any [`Piece`].
pub struct Play<P: Piece> {
    piece: P,
    pic: Vec<Cell>,
    /// The view without a tour, and the mirror's geometry with one.
    grid: Grid,
    /// Grid column/row -> picture column/row, `u32::MAX` off the picture.
    xmap: Vec<u32>,
    ymap: Vec<u32>,
    tour: Option<Touring>,
    frames: u64,
    fps: u64,
    tick: u64,
}

/// A [`Fit::Cover`] piece's camera, and the grid it draws every view but the
/// plain cover one through.
struct Touring {
    tour: Tour,
    /// The view `Play::grid` draws.
    base: View,
    /// The view on the panel, through `Play::grid` when it is `base` and
    /// through `grid` here otherwise.
    shown: View,
    grid: Grid,
    xmap: Vec<u32>,
    ymap: Vec<u32>,
    panel: Panel,
    aspect: usize,
}

impl<P: Piece> Play<P> {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        let knobs = match P::FIT {
            Fit::Cover { .. } => Knobs::from_env(),
            Fit::Contain => None,
        };
        Self::with_tour(panel, fps, knobs)
    }

    fn with_tour(panel: &Panel, fps: u32, knobs: Option<Knobs>) -> Self {
        // Cell widths are square-glass units; Grid stretches the height by the
        // panel's pixel aspect itself.
        let aspect = pixel_aspect();
        let rows_h = aspect * P::CELL * P::ROWS;
        let (mut w, anchor) = match P::FIT {
            Fit::Contain => ((panel.w / P::COLS).min(panel.h * 100 / rows_h), 0.5),
            Fit::Cover { anchor } => (
                panel
                    .w
                    .div_ceil(P::COLS)
                    .max((panel.h * 100).div_ceil(rows_h)),
                anchor,
            ),
        };
        w = w.max(1);
        let mut grid = Grid::with_aspect(panel, w, w * P::CELL, aspect);
        // Grid rounds the stretched height, which can leave cover one row
        // short of the panel's last row.
        while matches!(P::FIT, Fit::Cover { .. }) && grid.rows() > P::ROWS {
            w += 1;
            grid = Grid::with_aspect(panel, w, w * P::CELL, aspect);
        }
        let base = View {
            w,
            x0: -offset(grid.cols(), P::COLS, 0.5),
            y0: -offset(grid.rows(), P::ROWS, anchor),
        };
        let (mut xmap, mut ymap) = (Vec::new(), Vec::new());
        map_into(&mut xmap, grid.cols(), P::COLS, base.x0);
        map_into(&mut ymap, grid.rows(), P::ROWS, base.y0);
        let fps = fps.max(1);
        Self {
            piece: P::new(),
            pic: vec![Cell::CLEAR; P::COLS * P::ROWS],
            xmap,
            ymap,
            grid: grid.with_ground(P::GROUND),
            tour: knobs.map(|k| Touring::new::<P>(panel, aspect, base, fps, k)),
            frames: 0,
            fps: u64::from(fps),
            tick: u64::MAX,
        }
    }

    /// A `saver::SAVERS` row's builder: `(Piece::NAME, Play::<Piece>::build)`.
    pub fn build(panel: &Panel, fps: u32) -> Box<dyn Saver> {
        Box::new(Self::new(panel, fps))
    }

    #[inline]
    fn draw(grid: &mut Grid, pic: &[Cell], xmap: &[u32], ymap: &[u32]) {
        grid.fill(|cx, cy| {
            let (x, y) = (xmap[cx], ymap[cy]);
            if x == u32::MAX || y == u32::MAX {
                Cell::CLEAR
            } else {
                pic[y as usize * P::COLS + x as usize]
            }
        });
    }
}

impl Touring {
    fn new<P: Piece>(panel: &Panel, aspect: usize, base: View, fps: u32, k: Knobs) -> Self {
        let tour = Tour::new(
            panel,
            aspect,
            (P::COLS, P::ROWS, P::CELL),
            base,
            P::PALETTE,
            fps,
            k,
        );
        let (lo, hi) = tour.widths();
        // Widest cell first for the longest glyph LUTs, then the narrowest for
        // the most cells: every view between reshapes into these buffers.
        let mut grid = Grid::with_aspect(panel, hi, hi * P::CELL, aspect).with_ground(P::GROUND);
        grid.reshape(panel, lo, lo * P::CELL, aspect);
        Self {
            tour,
            base,
            shown: base,
            xmap: Vec::with_capacity(grid.cols()),
            ymap: Vec::with_capacity(grid.rows()),
            grid,
            panel: *panel,
            aspect,
        }
    }

    /// Point the tour's grid at `v`. A new cell width, or coming off the base
    /// grid, repaints the whole panel; a pan at the same width is an ordinary
    /// change-detected frame.
    fn show<P: Piece>(&mut self, v: View) {
        if self.shown == self.base || v.w != self.grid.cell_w() {
            self.grid
                .reshape(&self.panel, v.w, v.w * P::CELL, self.aspect);
        }
        map_into(&mut self.xmap, self.grid.cols(), P::COLS, v.x0);
        map_into(&mut self.ymap, self.grid.rows(), P::ROWS, v.y0);
        self.shown = v;
    }
}

/// Where a picture `len` long starts in `n` grid slots, `anchor` of the
/// difference going before it: positive pads, negative crops.
fn offset(n: usize, len: usize, anchor: f64) -> isize {
    ((n as f64 - len as f64) * anchor) as isize
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

impl<P: Piece> Saver for Play<P> {
    fn render(&mut self, s: &mut Surface<'_>) {
        let tick = self.frames * u64::from(P::FPS) / self.fps;
        self.frames += 1;
        if tick != self.tick {
            self.tick = tick;
            self.piece
                .frame(tick as f64 / f64::from(P::FPS), &mut self.pic);
        }
        if let Some(t) = &mut self.tour {
            let v = t.tour.step(&self.pic);
            if v != t.base {
                if v != t.shown {
                    t.show::<P>(v);
                }
                Self::draw(&mut t.grid, &self.pic, &t.xmap, &t.ymap);
                t.grid.flush(s, P::PALETTE);
                return;
            }
            if t.shown != t.base {
                self.grid.repaint();
                t.shown = t.base;
            }
        }
        Self::draw(&mut self.grid, &self.pic, &self.xmap, &self.ymap);
        self.grid.flush(s, P::PALETTE);
    }

    fn name(&self) -> &'static str {
        P::NAME
    }

    fn grid(&self) -> &Grid {
        &self.grid
    }

    /// The mirror keeps the plain cover view while the panel tours: a zoom
    /// step is a new geometry, and re-describing the mirror for each would
    /// reconnect every viewer a dozen times a glide.
    fn shown(&self) -> &Grid {
        match &self.tour {
            Some(t) if t.shown != t.base => &t.grid,
            _ => &self.grid,
        }
    }

    fn mirror_cells(&mut self) -> &[Cell] {
        match &self.tour {
            Some(t) if t.shown != t.base => {
                Self::draw(&mut self.grid, &self.pic, &self.xmap, &self.ymap);
                self.grid.pending()
            }
            _ => self.grid.cells(),
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
    use crate::grid::with_test_aspect;
    use crate::testalloc::allocs_during;

    #[test]
    fn hex_parses_upstream_colours() {
        assert_eq!(hex("#080b12"), 0x08_0b_12);
        assert_eq!(hex("#FFE9AE"), 0xff_e9_ae);
    }

    fn place(n: usize, len: usize, anchor: f64) -> Vec<u32> {
        let mut v = Vec::new();
        map_into(&mut v, n, len, -offset(n, len, anchor));
        v
    }

    #[test]
    fn place_pads_and_crops_around_the_anchor() {
        assert_eq!(place(5, 3, 0.5), [u32::MAX, 0, 1, 2, u32::MAX]);
        assert_eq!(place(3, 5, 0.5), [1, 2, 3]);
        assert_eq!(place(3, 5, 0.0), [0, 1, 2]);
        assert_eq!(place(3, 5, 1.0), [2, 3, 4]);
    }

    /// A cover scene of any size whose every cell moves, for the tour.
    struct Probe<const C: usize, const R: usize>;

    impl<const C: usize, const R: usize> Piece for Probe<C, R> {
        const NAME: &'static str = "probe";
        const COLS: usize = C;
        const ROWS: usize = R;
        const FPS: u32 = 15;
        const CELL: usize = 1;
        const PALETTE: &'static [u32] = &[0x00_0000, 0xFF_FFFF, 0xFF_8000];
        const GROUND: u32 = 0x10_2030;
        const FIT: Fit = Fit::Cover { anchor: 0.3 };

        fn new() -> Self {
            Self
        }

        fn frame(&mut self, t: f64, out: &mut [Cell]) {
            let k = (t * 15.0) as usize;
            for (i, c) in out.iter_mut().enumerate() {
                let (x, y) = (i % C, i / C);
                let dot = crate::font::HALFTONE[(x / 3 + y / 2 + k) % 4];
                *c = Cell::new(dot, ((x * y + k) % 3) as u16);
            }
        }
    }

    fn short_tour() -> Option<Knobs> {
        Some(Knobs {
            hold_secs: 1,
            max_zoom_pct: 250,
            seed: 11,
        })
    }

    /// What reaches the panel — only the reported rects, as simpledrm copies
    /// them out of a shadow buffer full of junk — is exactly a from-scratch
    /// render of the view the tour chose, through every zoom step, pan and
    /// pull back. A geometry change that left stale cells or margins, or
    /// under-reported them, fails here.
    fn panel_is_the_tours_view<P: Piece>(pw: usize, ph: usize, aspect: usize) {
        with_test_aspect(aspect, || {
            let panel = Panel::new(pw, ph, pw);
            let mut play = Play::<P>::with_tour(&panel, 30, short_tour());
            let mut buf = vec![0xDEAD_BEEFu32; panel.buf_len()];
            let mut hw = buf.clone();
            let mut fresh = vec![0u32; panel.buf_len()];
            let (mut xm, mut ym) = (Vec::new(), Vec::new());
            let (mut moves, mut last) = (0, play.tour.as_ref().unwrap().shown);
            let (mut wide, mut close) = (false, false);
            for n in 0..3000 {
                let mut s = Surface::new(&mut buf, &panel);
                play.render(&mut s);
                for r in s.finish().runs() {
                    for y in usize::from(r.y0)..usize::from(r.y1) {
                        let row = y * pw + usize::from(r.x0)..y * pw + usize::from(r.x1);
                        hw[row.clone()].copy_from_slice(&buf[row]);
                    }
                }
                let case = format!("{} {pw}x{ph}@{aspect} frame {n}", P::NAME);
                assert!(hw == buf, "{case}: drawn but never reported");

                let t = play.tour.as_ref().unwrap();
                let v = t.shown;
                if v != last || n % 25 == 0 {
                    moves += usize::from(v != last);
                    last = v;
                    let mut g = Grid::with_aspect(&panel, v.w, v.w * P::CELL, aspect)
                        .with_ground(P::GROUND);
                    map_into(&mut xm, g.cols(), P::COLS, v.x0);
                    map_into(&mut ym, g.rows(), P::ROWS, v.y0);
                    Play::<P>::draw(&mut g, &play.pic, &xm, &ym);
                    g.flush(&mut Surface::new(&mut fresh, &panel), P::PALETTE);
                    assert!(fresh == buf, "{case}: panel is not {v:?}");
                    let sh = play.shown();
                    assert_eq!(
                        (sh.cols(), sh.rows(), sh.cell_w()),
                        (g.cols(), g.rows(), g.cell_w())
                    );
                    assert!(
                        sh.cells() == g.cells(),
                        "{case}: shown() is not what was drawn"
                    );
                }
                wide |= v.w < t.base.w;
                close |= v.w > t.base.w;
            }
            assert!(
                moves > 20 && wide && close,
                "{} {pw}x{ph}: {moves} moves, wide {wide} close {close}",
                P::NAME
            );
        });
    }

    #[test]
    fn the_panel_is_always_the_tours_view() {
        panel_is_the_tours_view::<Probe<200, 100>>(1920, 1080, 180);
        panel_is_the_tours_view::<Probe<320, 100>>(1920, 1080, 180);
        panel_is_the_tours_view::<Probe<37, 23>>(1280, 400, 100);
    }

    /// Moves, measuring, reshapes and pans all draw on buffers sized at build.
    #[test]
    fn touring_never_allocates() {
        with_test_aspect(180, || {
            let panel = Panel::new(1920, 1080, 1920);
            let mut play = Play::<Probe<200, 100>>::with_tour(&panel, 30, short_tour());
            let mut buf = vec![0u32; panel.buf_len()];
            let mut frame = |play: &mut Play<Probe<200, 100>>| {
                let mut s = Surface::new(&mut buf, &panel);
                play.render(&mut s);
                play.mirror_cells();
            };
            frame(&mut play);
            let mut widths = [false; 64];
            let n = allocs_during(|| {
                for _ in 0..1500 {
                    frame(&mut play);
                    widths[play.tour.as_ref().unwrap().shown.w] = true;
                }
            });
            assert_eq!(n, 0, "touring allocated");
            let seen = widths.iter().filter(|&&w| w).count();
            assert!(seen > 8, "only {seen} cell widths");
        });
    }

    /// The mirror's geometry and cells are the untoured view's on every frame,
    /// and while the tour holds that view the panel is byte for byte what
    /// `Play` drew before the tour existed, damage included.
    #[test]
    fn the_mirror_and_the_first_hold_are_the_untoured_saver() {
        with_test_aspect(180, || {
            let panel = Panel::new(1920, 1080, 1920);
            let mut on = Play::<Probe<200, 100>>::with_tour(&panel, 30, short_tour());
            let mut off = Play::<Probe<200, 100>>::with_tour(&panel, 30, None);
            let (mut a, mut b) = (vec![0u32; panel.buf_len()], vec![0u32; panel.buf_len()]);
            let mut held = true;
            for n in 0..900 {
                let mut s = Surface::new(&mut a, &panel);
                on.render(&mut s);
                let da = s.finish();
                let mut s = Surface::new(&mut b, &panel);
                off.render(&mut s);
                let db = s.finish();
                held &= on.tour.as_ref().unwrap().shown == on.tour.as_ref().unwrap().base;
                if held {
                    assert_eq!(da.runs(), db.runs(), "frame {n}: damage");
                    assert!(a == b, "frame {n}: pixels");
                }
                assert_eq!(
                    (on.grid().cols(), on.grid().rows(), on.grid().cell_w()),
                    (off.grid().cols(), off.grid().rows(), off.grid().cell_w())
                );
                assert!(on.mirror_cells() == off.mirror_cells(), "frame {n}: mirror");
            }
            assert!(!held, "the tour never left the cover view");
        });
    }

    /// The checks every piece gets: it fills its picture with in-range glyphs
    /// and colours, the picture moves, and steady-state frames never allocate.
    pub fn exercise<P: Piece>() {
        let panel = Panel::new(1920, 1080, 1920);
        let mut play = Play::<P>::new(&panel, P::FPS);
        let mut buf = vec![0u32; panel.buf_len()];
        let mut frame = |play: &mut Play<P>| {
            let mut s = Surface::new(&mut buf, &panel);
            play.render(&mut s);
            s.finish();
        };
        frame(&mut play);
        let first = play.pic.clone();
        for c in &first {
            assert!(
                c.glyph() < crate::font::GLYPHS.len(),
                "{}: glyph {}",
                P::NAME,
                c.glyph()
            );
            assert!(
                c.colour() < P::PALETTE.len(),
                "{}: colour {}",
                P::NAME,
                c.colour()
            );
        }
        assert!(
            first.iter().any(|c| *c != Cell::CLEAR),
            "{}: blank picture",
            P::NAME
        );
        for _ in 0..P::FPS * 2 {
            frame(&mut play);
        }
        assert_ne!(first, play.pic, "{}: nothing moved in two seconds", P::NAME);
        let n = allocs_during(|| {
            for _ in 0..P::FPS {
                frame(&mut play);
            }
        });
        assert_eq!(n, 0, "{}: render allocated", P::NAME);
    }

    /// Compare the port with upstream's own output, written by
    /// `tools/ascii-rest-golden.ts` into `$ASCII_REST_GOLDEN/<name>.golden`.
    /// Ignored by default: it needs bun and an upstream checkout.
    ///
    /// Exact, cell for cell: `math` reproduces JavaScriptCore wherever libm
    /// differs, so one stray cell is a port bug, not rounding noise.
    pub fn golden<P: Piece>() {
        if !P::UPSTREAM {
            return;
        }
        let dir = std::env::var("ASCII_REST_GOLDEN").expect("set ASCII_REST_GOLDEN");
        let path = format!("{dir}/{}.golden", P::NAME);
        let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let mut lines = src.lines();
        let head: Vec<usize> = lines
            .next()
            .unwrap()
            .split(' ')
            .map(|v| v.parse().unwrap())
            .collect();
        assert_eq!(
            head,
            [P::COLS, P::ROWS, P::FPS as usize],
            "{path}: geometry"
        );
        let halftone = P::CELL == 1;
        let glyph = |c: char| -> u16 {
            if halftone {
                let i = " ·•●".chars().position(|d| d == c).expect("halftone char");
                crate::font::HALFTONE[i]
            } else {
                text::glyph(c)
            }
        };
        let mut piece = P::new();
        let mut pic = vec![Cell::CLEAR; P::COLS * P::ROWS];
        let mut tick = 0u64;
        let mut differ = 0usize;
        while let Some(at) = lines.next() {
            let want: u64 = at.strip_prefix('@').unwrap().parse().unwrap();
            while tick <= want {
                piece.frame(tick as f64 / f64::from(P::FPS), &mut pic);
                tick += 1;
            }
            let mut expect = vec![Cell::CLEAR; P::COLS * P::ROWS];
            for r in 0..P::ROWS {
                let row: Vec<char> = lines.next().unwrap().chars().collect();
                assert_eq!(row.len(), P::COLS, "{path} @{want} row {r} width");
                for (x, &c) in row.iter().enumerate() {
                    expect[r * P::COLS + x] = Cell::new(glyph(c), 0);
                }
            }
            if halftone {
                for r in 0..P::ROWS {
                    let row = lines.next().unwrap().as_bytes();
                    for x in 0..P::COLS {
                        let h = std::str::from_utf8(&row[x * 2..x * 2 + 2]).unwrap();
                        let e = &mut expect[r * P::COLS + x];
                        *e = Cell::new(e.glyph() as u16, u16::from_str_radix(h, 16).unwrap());
                    }
                }
            }
            let bad: Vec<usize> = (0..expect.len()).filter(|&i| expect[i] != pic[i]).collect();
            differ += bad.len();
            println!(
                "{} @{want}: {} of {} cells differ{}",
                P::NAME,
                bad.len(),
                expect.len(),
                bad.first()
                    .map(|&i| format!(
                        ", first at col {} row {}: want {:?} got {:?}",
                        i % P::COLS,
                        i / P::COLS,
                        (expect[i].glyph(), expect[i].colour()),
                        (pic[i].glyph(), pic[i].colour())
                    ))
                    .unwrap_or_default()
            );
        }
        assert_eq!(differ, 0, "{}: cells differ from upstream", P::NAME);
    }
}
