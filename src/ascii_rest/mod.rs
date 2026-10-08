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
pub mod text;
pub mod title;
pub mod tour;

/// The one list of pieces. A port is a row here plus its file: its module, its
/// `saver::SAVERS` row and its tests all come from this list. `#[no_upstream]`
/// marks this repo's own pieces (the `-wide` recompositions), which get no
/// golden test: upstream has no output for them.
macro_rules! each_piece {
    ($cb:ident) => {
        $cb! {
            alpine_dawn::AlpineDawn,
            #[no_upstream] alpine_dawn_wide::AlpineDawnWide,
            aurora::Aurora,
            aurora_fjord::AuroraFjord,
            #[no_upstream] aurora_fjord_wide::AuroraFjordWide,
            deep_reef::DeepReef,
            #[no_upstream] deep_reef_wide::DeepReefWide,
            desert_night::DesertNight,
            #[no_upstream] desert_night_wide::DesertNightWide,
            double_pendulum::DoublePendulum,
            earthrise::Earthrise,
            #[no_upstream] earthrise_wide::EarthriseWide,
            fractal_tree::FractalTree,
            kyoto_dusk::KyotoDusk,
            #[no_upstream] kyoto_dusk_wide::KyotoDuskWide,
            lighthouse::Lighthouse,
            marine_drive::MarineDrive,
            #[no_upstream] marine_drive_wide::MarineDriveWide,
            misty_forest::MistyForest,
            #[no_upstream] misty_forest_wide::MistyForestWide,
            night_coast::NightCoast,
            #[no_upstream] night_coast_wide::NightCoastWide,
            ocean_sunset::OceanSunset,
            #[no_upstream] ocean_sunset_wide::OceanSunsetWide,
            reaction_diffusion::ReactionDiffusion,
            storm_plains::StormPlains,
            #[no_upstream] storm_plains_wide::StormPlainsWide,
            synthwave::Synthwave,
            taj_dawn::TajDawn,
            #[no_upstream] taj_dawn_wide::TajDawnWide,
            tv_static::TvStatic,
            varanasi_ghats::VaranasiGhats,
            #[no_upstream] varanasi_ghats_wide::VaranasiGhatsWide,
            vinyl::Vinyl,
        }
    };
}
pub(crate) use each_piece;

macro_rules! declare {
    ($($(#[$no:ident])? $m:ident::$t:ident),* $(,)?) => {
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

                    golden!($($no)? $m::$t);
                }
            )*
        }
    };
}

#[cfg(test)]
macro_rules! golden {
    (no_upstream $m:ident::$t:ident) => {};
    ($m:ident::$t:ident) => {
        #[test]
        #[ignore = "needs ASCII_REST_GOLDEN; see tools/ascii-rest-golden.ts"]
        fn golden() {
            tests::golden::<crate::ascii_rest::$m::$t>();
        }
    };
}
each_piece!(declare);

use crate::grid::{pixel_aspect, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use title::Title;
use tour::{Knobs, Touring, View};

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
    /// What the panel shows.
    cam: Camera,
    tour: Option<Touring>,
    /// `ASCII_REST_TITLE`: stamped over the camera's grid after the picture
    /// is mapped onto it.
    title: Option<Title>,
    frames: u64,
    fps: u64,
    tick: u64,
}

/// One grid looking at the picture through a [`View`].
struct Camera {
    grid: Grid,
    view: View,
    /// Grid column/row -> picture column/row, `u32::MAX` off the picture.
    xmap: Vec<u32>,
    ymap: Vec<u32>,
}

impl Camera {
    /// Looking through `v`, with buffers for every view from cell width `lo`
    /// to `hi` so that aiming between them allocates nothing.
    fn new<P: Piece>(panel: &Panel, aspect: usize, v: View, (lo, hi): (usize, usize)) -> Self {
        // Widest cell first for the longest glyph LUTs, then the narrowest for
        // the most cells.
        let mut grid = Grid::with_aspect(panel, hi, hi * P::CELL, aspect).with_ground(P::GROUND);
        grid.reshape(panel, lo, lo * P::CELL, aspect);
        let mut cam = Self {
            xmap: Vec::with_capacity(grid.cols()),
            ymap: Vec::with_capacity(grid.rows()),
            grid,
            view: View { w: lo, ..v },
        };
        cam.aim::<P>(panel, aspect, v);
        cam
    }

    /// Look through `v`. A new cell width reshapes the grid, which repaints
    /// the whole panel; a pan is an ordinary change-detected frame.
    fn aim<P: Piece>(&mut self, panel: &Panel, aspect: usize, v: View) {
        if v.w != self.view.w {
            self.grid.reshape(panel, v.w, v.w * P::CELL, aspect);
        }
        self.view = v;
        map_into(&mut self.xmap, self.grid.cols(), P::COLS, v.x0);
        map_into(&mut self.ymap, self.grid.rows(), P::ROWS, v.y0);
    }

    #[inline]
    fn draw<P: Piece>(&mut self, pic: &[Cell]) {
        let (xmap, ymap) = (&self.xmap, &self.ymap);
        self.grid.fill(|cx, cy| {
            let (x, y) = (xmap[cx], ymap[cy]);
            if x == u32::MAX || y == u32::MAX {
                Cell::CLEAR
            } else {
                pic[y as usize * P::COLS + x as usize]
            }
        });
    }
}

impl<P: Piece> Play<P> {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        let knobs = match P::FIT {
            Fit::Cover { .. } => Knobs::from_env(),
            Fit::Contain => None,
        };
        let mut play = Self::with_tour(panel, fps, knobs);
        if crate::env_num(&["ASCII_REST_TITLE"], 0, 0, 1) == 1 {
            play.title = Some(Title::new(P::NAME, P::CELL, P::PALETTE));
        }
        play
    }

    fn with_tour(panel: &Panel, fps: u32, knobs: Option<Knobs>) -> Self {
        // Cell widths are square-glass units; Grid stretches the height by the
        // panel's pixel aspect itself.
        let aspect = pixel_aspect();
        let w = fit_w(panel, aspect, P::COLS, P::ROWS, P::CELL, P::FIT);
        let anchor = match P::FIT {
            Fit::Contain => 0.5,
            Fit::Cover { anchor } => anchor,
        };
        let s = Grid::shape(panel, w, w * P::CELL, aspect);
        let spare = |len: usize, n: usize| len as f64 - n as f64;
        let base = View {
            w,
            x0: origin(P::COLS, s.cols, spare(P::COLS, s.cols) * 0.5),
            y0: origin(P::ROWS, s.rows, spare(P::ROWS, s.rows) * anchor),
        };
        let fps = fps.max(1);
        let tour = knobs.map(|k| Touring::new::<P>(panel, aspect, base, fps, k));
        let widths = tour.as_ref().map_or((w, w), Touring::widths);
        Self {
            piece: P::new(),
            pic: vec![Cell::CLEAR; P::COLS * P::ROWS],
            cam: Camera::new::<P>(panel, aspect, base, widths),
            tour,
            title: None,
            frames: 0,
            fps: u64::from(fps),
            tick: u64::MAX,
        }
    }

    /// A `saver::SAVERS` row's builder: `(Piece::NAME, Play::<Piece>::build)`.
    pub fn build(panel: &Panel, fps: u32) -> Box<dyn Saver> {
        Box::new(Self::new(panel, fps))
    }
}

/// The cell width at which a `cols x rows` picture of `cell`-tall cells meets
/// the panel as `fit` asks: the widest whose grid holds all of it, or the
/// narrowest whose grid it covers. Searched through `Grid::shape` rather than
/// solved, because the grid rounds the stretched cell height — the plain
/// quotient leaves contain two rows short on pine.
fn fit_w(panel: &Panel, aspect: usize, cols: usize, rows: usize, cell: usize, fit: Fit) -> usize {
    let fits = |w: usize| {
        let s = Grid::shape(panel, w, w * cell, aspect);
        match fit {
            Fit::Contain => s.cols >= cols && s.rows >= rows,
            Fit::Cover { .. } => s.cols <= cols && s.rows <= rows,
        }
    };
    let mut w = (panel.w / cols)
        .min(panel.h * 100 / (aspect * cell * rows))
        .max(1);
    match fit {
        Fit::Contain => {
            while w > 1 && !fits(w) {
                w -= 1;
            }
            while fits(w + 1) {
                w += 1;
            }
        }
        Fit::Cover { .. } => {
            while !fits(w) {
                w += 1;
            }
            while w > 1 && fits(w - 1) {
                w -= 1;
            }
        }
    }
    w
}

/// The picture cell under slot 0 of an `n`-slot window on a `len`-cell
/// picture that would like to start at `start`. A window wider than the
/// picture is centred over the ground; a narrower one is kept inside it.
fn origin(len: usize, n: usize, start: f64) -> isize {
    if n >= len {
        -(((n - len) / 2) as isize)
    } else {
        (start as isize).clamp(0, (len - n) as isize)
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
            t.steer::<P>(&self.pic, &mut self.cam);
        }
        self.cam.draw::<P>(&self.pic);
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
            Some(t) => t.mirror::<P>(&self.pic, self.title.as_ref()),
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
    use crate::glyph;
    use crate::testalloc::allocs_during;

    #[test]
    fn hex_parses_upstream_colours() {
        assert_eq!(hex("#080b12"), 0x08_0b_12);
        assert_eq!(hex("#FFE9AE"), 0xff_e9_ae);
    }

    fn place(n: usize, len: usize, anchor: f64) -> Vec<u32> {
        let mut v = Vec::new();
        map_into(
            &mut v,
            n,
            len,
            origin(len, n, (len as f64 - n as f64) * anchor),
        );
        v
    }

    #[test]
    fn place_pads_and_crops_around_the_anchor() {
        assert_eq!(place(5, 3, 0.5), [u32::MAX, 0, 1, 2, u32::MAX]);
        assert_eq!(place(3, 5, 0.5), [1, 2, 3]);
        assert_eq!(place(3, 5, 0.0), [0, 1, 2]);
        assert_eq!(place(3, 5, 1.0), [2, 3, 4]);
    }

    #[test]
    fn a_window_stays_on_the_picture_and_a_wide_one_centres() {
        assert_eq!(origin(200, 50, 75.0), 75);
        assert_eq!(origin(200, 50, -25.0), 0);
        assert_eq!(origin(200, 50, 174.0), 150);
        assert_eq!(origin(200, 300, 0.0), -50);
        assert_eq!(origin(200, 200, 7.0), 0);
    }

    /// Pine: 1920x1080 at 180, a 200x100 scene. The grid rounds the stretched
    /// cell, so the plain quotient is two rows short of whole.
    #[test]
    fn contain_holds_the_whole_picture_on_pine() {
        let panel = Panel::new(1920, 1080, 1920);
        let w = fit_w(&panel, 180, 200, 100, 1, Fit::Contain);
        let s = Grid::shape(&panel, w, w, 180);
        assert!(s.cols >= 200 && s.rows >= 100, "w={w}: {s:?}");
        let s = Grid::shape(&panel, w + 1, w + 1, 180);
        assert!(
            s.cols < 200 || s.rows < 100,
            "w={w} is not the largest that fits"
        );
    }

    /// Cover is the narrowest cell the picture still covers: one narrower
    /// and the grid would show ground.
    #[test]
    fn cover_is_the_narrowest_cell_the_picture_fills() {
        for (pw, ph, aspect) in [(1920, 1080, 180), (1920, 1080, 100), (1280, 400, 100)] {
            let panel = Panel::new(pw, ph, pw);
            for (cols, rows) in [(200, 100), (320, 100), (37, 23)] {
                let anchor = Fit::Cover { anchor: 0.5 };
                let w = fit_w(&panel, aspect, cols, rows, 1, anchor);
                let s = Grid::shape(&panel, w, w, aspect);
                assert!(s.cols <= cols && s.rows <= rows, "{pw}x{ph} {cols}x{rows}");
                let s = Grid::shape(&panel, w - 1, w - 1, aspect);
                assert!(s.cols > cols || s.rows > rows, "{pw}x{ph} {cols}x{rows}");
            }
        }
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
                glyph::of(c)
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
