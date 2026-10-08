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

// Ports keep upstream's literals (`6.28`, `3.14`) rather than TAU/PI: the
// golden test compares against what upstream computes, not what it meant.
#![allow(clippy::approx_constant)]

pub mod halftone;
pub mod math;
pub mod text;

/// The one list of pieces. A port is a row here plus its file: its module, its
/// `saver::SAVERS` row and its tests all come from this list.
macro_rules! each_piece {
    ($cb:ident) => {
        $cb! {
            alpine_dawn::AlpineDawn,
            aurora::Aurora,
            aurora_fjord::AuroraFjord,
            deep_reef::DeepReef,
            desert_night::DesertNight,
            double_pendulum::DoublePendulum,
            earthrise::Earthrise,
            fractal_tree::FractalTree,
            kyoto_dusk::KyotoDusk,
            lighthouse::Lighthouse,
            marine_drive::MarineDrive,
            misty_forest::MistyForest,
            night_coast::NightCoast,
            ocean_sunset::OceanSunset,
            ocean_sunset_wide::OceanSunsetWide,
            plasma::Plasma,
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
    grid: Grid,
    /// Grid column/row -> picture column/row, `u32::MAX` off the picture.
    xmap: Vec<u32>,
    ymap: Vec<u32>,
    frames: u64,
    fps: u64,
    tick: u64,
}

impl<P: Piece> Play<P> {
    pub fn new(panel: &Panel, fps: u32) -> Self {
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
        Self {
            piece: P::new(),
            pic: vec![Cell::CLEAR; P::COLS * P::ROWS],
            xmap: place(grid.cols(), P::COLS, 0.5),
            ymap: place(grid.rows(), P::ROWS, anchor),
            grid: grid.with_ground(P::GROUND),
            frames: 0,
            fps: u64::from(fps.max(1)),
            tick: u64::MAX,
        }
    }

    /// A `saver::SAVERS` row's builder: `(Piece::NAME, Play::<Piece>::build)`.
    pub fn build(panel: &Panel, fps: u32) -> Box<dyn Saver> {
        Box::new(Self::new(panel, fps))
    }
}

/// Map `n` grid slots onto a picture `len` wide. A narrower picture is padded
/// and a wider one cropped, `anchor` of the difference going before it.
fn place(n: usize, len: usize, anchor: f64) -> Vec<u32> {
    let off = ((n as f64 - len as f64) * anchor) as isize;
    (0..n as isize)
        .map(|i| {
            let p = i - off;
            if p >= 0 && (p as usize) < len {
                p as u32
            } else {
                u32::MAX
            }
        })
        .collect()
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
        let (pic, xmap, ymap) = (&self.pic, &self.xmap, &self.ymap);
        self.grid.fill(|cx, cy| {
            let (x, y) = (xmap[cx], ymap[cy]);
            if x == u32::MAX || y == u32::MAX {
                Cell::CLEAR
            } else {
                pic[y as usize * P::COLS + x as usize]
            }
        });
        self.grid.flush(s, P::PALETTE);
    }

    fn name(&self) -> &'static str {
        P::NAME
    }

    fn grid(&self) -> &Grid {
        &self.grid
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
    use crate::testalloc::allocs_during;

    #[test]
    fn hex_parses_upstream_colours() {
        assert_eq!(hex("#080b12"), 0x08_0b_12);
        assert_eq!(hex("#FFE9AE"), 0xff_e9_ae);
    }

    #[test]
    fn place_pads_and_crops_around_the_anchor() {
        assert_eq!(place(5, 3, 0.5), [u32::MAX, 0, 1, 2, u32::MAX]);
        assert_eq!(place(3, 5, 0.5), [1, 2, 3]);
        assert_eq!(place(3, 5, 0.0), [0, 1, 2]);
        assert_eq!(place(3, 5, 1.0), [2, 3, 4]);
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
