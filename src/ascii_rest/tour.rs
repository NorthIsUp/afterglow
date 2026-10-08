//! The scenes' slow camera: hold a view, glide to the next, and every few moves
//! pull back to the whole picture or the plain cover view.
//!
//! Zoom is cell size, never resampling. Every view draws one picture cell per
//! grid cell, so the ordered dither survives and the dots just get bigger; a
//! glide steps through the integer cell widths between two views.
//!
//! Close-ups go where the picture has something in it. Each move re-measures
//! the frame on screen in 4x4-cell blocks — the dither's period, so a flat
//! gradient's dither does not read as detail — and draws a block weighted by
//! its contrast with its neighbours and its brightness, or one of the picture's
//! corners and edges weighted by the blocks it would frame.
//!
//! Time is counted in frames, like `Play`'s clock, so a dump shows the tour the
//! panel does.

use super::halftone::COVER;
use super::title::Title;
use super::{fit_w, origin, Camera, Fit, Piece};
use crate::font;
use crate::grid::{Cell, Grid};
use crate::next_rand;
use crate::surface::Panel;

/// The dither's period: blocks this size average it away.
const BLOCK: usize = 4;

/// The least a close-up magnifies the cover view's cell.
const MIN_ZOOM: f64 = 1.25;

/// A glide's length in seconds, drawn uniformly.
const GLIDE_SECS: (f64, f64) = (3.0, 6.0);

/// What the panel shows: a cell width in square-glass pixels, and the picture
/// cell under the panel's top-left cell. Negative when the picture is padded.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct View {
    pub w: usize,
    pub x0: isize,
    pub y0: isize,
}

/// The tour's knobs, read once per saver build.
#[derive(Clone, Copy, Debug)]
pub struct Knobs {
    pub hold_secs: u32,
    pub max_zoom_pct: u32,
    pub seed: u32,
}

impl Knobs {
    /// None when `ASCII_REST_TOUR=0`.
    pub fn from_env() -> Option<Self> {
        (crate::env_num(&["ASCII_REST_TOUR"], 1, 0, 1) == 1).then(|| Self {
            hold_secs: crate::env_num(&["ASCII_REST_TOUR_HOLD_SECS"], 14, 1, 3600) as u32,
            max_zoom_pct: crate::env_num(&["ASCII_REST_TOUR_MAX_ZOOM_PCT"], 250, 100, 600) as u32,
            seed: crate::saver_seed(&["ASCII_REST_TOUR_SEED"], 0x70C4_5EED),
        })
    }
}

/// A [`Fit::Cover`] piece's camera, and the mirror's view of the same
/// picture.
pub(super) struct Touring {
    tour: Tour,
    /// The plain cover view, whatever the panel shows: a zoom step is a new
    /// geometry, and re-describing the mirror for each would reconnect every
    /// viewer a dozen times a glide.
    mirror: Camera,
}

impl Touring {
    pub(super) fn new<P: Piece>(
        panel: &Panel,
        aspect: usize,
        base: View,
        fps: u32,
        k: Knobs,
    ) -> Self {
        Self {
            tour: Tour::new::<P>(panel, aspect, base, fps, k),
            mirror: Camera::new::<P>(panel, aspect, base, (base.w, base.w)),
        }
    }

    /// The narrowest and widest cell any view uses, for sizing buffers once.
    pub(super) fn widths(&self) -> (usize, usize) {
        (self.tour.min_w, self.tour.max_w)
    }

    /// Advance the tour a frame and point the panel's camera where it says.
    pub(super) fn steer<P: Piece>(&mut self, pic: &[Cell], cam: &mut Camera) {
        let v = self.tour.step(pic);
        if v != cam.view {
            cam.aim::<P>(&self.tour.panel, self.tour.aspect, v);
        }
    }

    /// The cover view of `pic`, under the title if there is one. Only called
    /// while someone is watching.
    pub(super) fn mirror<P: Piece>(&mut self, pic: &[Cell], title: Option<&Title>) -> &Grid {
        self.mirror.draw::<P>(pic);
        if let Some(t) = title {
            t.stamp(&mut self.mirror.grid);
        }
        self.mirror.grid.settle();
        &self.mirror.grid
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Base,
    Full,
    Close,
}

/// Where the camera is headed: a cell width and the picture point it centres.
/// A glide interpolates these rather than views, so the point stays put on the
/// panel while the cell around it grows or shrinks.
#[derive(Clone, Copy, Debug)]
struct Shot {
    w: f64,
    cx: f64,
    cy: f64,
    kind: Kind,
}

pub struct Tour {
    panel: Panel,
    aspect: usize,
    /// The picture, in cells, and its cell height in cell widths.
    cols: usize,
    rows: usize,
    cell: usize,
    base: Shot,
    full: Shot,
    min_w: usize,
    max_w: usize,
    max_zoom: f64,
    from: Shot,
    to: Shot,
    /// The view `Play` draws without a tour, which every `Kind::Base` shot
    /// lands on exactly.
    home: View,
    /// What holding `to` draws, worked out once per move.
    held: View,
    at: u64,
    glide: u64,
    hold: u64,
    fps: f64,
    hold_secs: f64,
    /// Close-ups left before the next pull back.
    wide_in: u32,
    /// The last few close-ups, newest first.
    recent: [Option<View>; 3],
    rng: u32,
    pal: &'static [u32],
    bcols: usize,
    brows: usize,
    /// Per block: mean RGB as drawn, then how much it is worth framing.
    mean: Vec<[f32; 3]>,
    weight: Vec<f32>,
}

impl Tour {
    /// `base` is the view `Play` draws without a tour; the tour starts there.
    pub fn new<P: Piece>(panel: &Panel, aspect: usize, base: View, fps: u32, k: Knobs) -> Self {
        let (cols, rows, cell) = (P::COLS, P::ROWS, P::CELL);
        let full_w = fit_w(panel, aspect, cols, rows, cell, Fit::Contain);
        let max_zoom = f64::from(k.max_zoom_pct) / 100.0;
        let max_w = base.w.max((base.w as f64 * max_zoom).round() as usize);
        let (bcols, brows) = (cols.div_ceil(BLOCK), rows.div_ceil(BLOCK));
        let mut t = Self {
            panel: *panel,
            aspect,
            cols,
            rows,
            cell,
            base: Shot {
                w: base.w as f64,
                cx: 0.0,
                cy: 0.0,
                kind: Kind::Base,
            },
            full: Shot {
                w: full_w as f64,
                cx: cols as f64 / 2.0,
                cy: rows as f64 / 2.0,
                kind: Kind::Full,
            },
            min_w: full_w.min(base.w),
            max_w,
            max_zoom,
            from: Shot {
                w: 1.0,
                cx: 0.0,
                cy: 0.0,
                kind: Kind::Base,
            },
            to: Shot {
                w: 1.0,
                cx: 0.0,
                cy: 0.0,
                kind: Kind::Base,
            },
            home: base,
            held: base,
            at: 0,
            glide: 0,
            hold: 0,
            fps: fps.max(1) as f64,
            hold_secs: f64::from(k.hold_secs),
            wide_in: 0,
            recent: [None; 3],
            rng: k.seed,
            pal: P::PALETTE,
            bcols,
            brows,
            mean: vec![[0.0; 3]; bcols * brows],
            weight: vec![0.0; bcols * brows],
        };
        // The base view's centre, which glides to and from it pass through.
        let (gc, gr) = t.dims(base.w);
        t.base.cx = base.x0 as f64 + gc as f64 / 2.0;
        t.base.cy = base.y0 as f64 + gr as f64 / 2.0;
        (t.from, t.to) = (t.base, t.base);
        t.wide_in = t.close_ups();
        t.hold = t.hold_for(Kind::Base);
        t
    }

    /// The whole picture, padded with ground.
    #[cfg(test)]
    pub fn full(&self) -> View {
        self.view(self.full.w as usize, self.full.cx, self.full.cy)
    }

    /// Advance one frame and say what to draw. `pic` is the frame on screen,
    /// read only when a move picks its next target.
    pub fn step(&mut self, pic: &[Cell]) -> View {
        if self.at >= self.glide + self.hold {
            self.advance(pic);
        }
        let at = self.at;
        self.at += 1;
        if at >= self.glide {
            return self.held;
        }
        let s = at as f64 / self.glide as f64;
        let e = s * s * (3.0 - 2.0 * s);
        let (a, b) = (self.from, self.to);
        // Through log width, so each step is the same proportion of zoom.
        let w = (a.w.ln() + (b.w.ln() - a.w.ln()) * e).exp().round() as usize;
        self.view(
            w.clamp(self.min_w, self.max_w),
            a.cx + (b.cx - a.cx) * e,
            a.cy + (b.cy - a.cy) * e,
        )
    }

    fn advance(&mut self, pic: &[Cell]) {
        self.measure(pic);
        self.from = self.to;
        self.to = self.pick();
        self.held = self.view_of(self.to);
        let (lo, hi) = GLIDE_SECS;
        let secs = lo + (hi - lo) * self.unit();
        self.glide = self.frames(secs);
        self.hold = self.hold_for(self.to.kind);
        self.at = 0;
    }

    fn pick(&mut self) -> Shot {
        if self.wide_in == 0 || self.max_w <= self.base.w as usize {
            self.wide_in = self.close_ups();
            return match self.to.kind {
                Kind::Base => self.full,
                Kind::Close if next_rand(&mut self.rng) & 1 == 0 => self.full,
                Kind::Full | Kind::Close => self.base,
            };
        }
        self.wide_in -= 1;
        // A close-up next to the last one is a twitch, not a move.
        let mut shot = self.close_up();
        for _ in 0..4 {
            if self.apart(shot, self.to) {
                break;
            }
            shot = self.close_up();
        }
        self.recent.rotate_right(1);
        self.recent[0] = Some(self.view_of(shot));
        shot
    }

    /// Two to four close-ups between pulls back, so about every fourth move.
    fn close_ups(&mut self) -> u32 {
        2 + next_rand(&mut self.rng) % 3
    }

    fn close_up(&mut self) -> Shot {
        let corner = next_rand(&mut self.rng).is_multiple_of(3);
        // Corners zoom less: a window flush against the edge has nothing to
        // centre on, so it frames more of what is near it instead.
        let lo = MIN_ZOOM.min(self.max_zoom);
        let hi = if corner {
            (lo + self.max_zoom) / 2.0
        } else {
            self.max_zoom
        };
        let z = lo + (hi - lo) * self.unit();
        let w = ((self.base.w * z).round() as usize).clamp(self.base.w as usize + 1, self.max_w);
        let (cx, cy) = if corner {
            const AT: [(f64, f64); 8] = [
                (0.0, 0.0),
                (0.5, 0.0),
                (1.0, 0.0),
                (0.0, 0.5),
                (1.0, 0.5),
                (0.0, 1.0),
                (0.5, 1.0),
                (1.0, 1.0),
            ];
            let (cols, rows) = (self.cols as f64, self.rows as f64);
            let score = AT.map(|(fx, fy)| {
                let v = self.view(w, fx * cols, fy * rows);
                let (gc, gr) = self.dims(w);
                let c = (v.x0 as f64 + gc as f64 / 2.0, v.y0 as f64 + gr as f64 / 2.0);
                self.framed(v) * self.fresh(c)
            });
            let u = self.unit();
            let (fx, fy) = AT[draw(u, AT.len(), |i| score[i])];
            (fx * cols, fy * rows)
        } else {
            let u = self.unit();
            let (b, bcols) = (BLOCK as f64, self.bcols);
            let at = |i: usize| {
                (
                    (i % bcols) as f64 * b + b / 2.0,
                    (i / bcols) as f64 * b + b / 2.0,
                )
            };
            let i = draw(u, self.weight.len(), |i| self.weight[i] * self.fresh(at(i)));
            at(i)
        };
        Shot {
            w: w as f64,
            cx,
            cy,
            kind: Kind::Close,
        }
    }

    /// How much a point is worth revisiting: little if a recent close-up
    /// already showed it, so the moon does not get every other move.
    fn fresh(&self, (x, y): (f64, f64)) -> f32 {
        let shown = self.recent.iter().flatten().any(|v| {
            let (gc, gr) = self.dims(v.w);
            (v.x0 as f64..(v.x0 + gc as isize) as f64).contains(&x)
                && (v.y0 as f64..(v.y0 + gr as isize) as f64).contains(&y)
        });
        if shown {
            0.05
        } else {
            1.0
        }
    }

    /// Far enough apart in zoom or in place to be worth gliding between.
    fn apart(&self, a: Shot, b: Shot) -> bool {
        let (va, vb) = (self.view_of(a), self.view_of(b));
        let (gc, gr) = self.dims(va.w.max(vb.w));
        (a.w / b.w).ln().abs() > 0.25
            || va.x0.abs_diff(vb.x0) * 3 > gc
            || va.y0.abs_diff(vb.y0) * 3 > gr
    }

    fn hold_for(&mut self, kind: Kind) -> u64 {
        // The whole picture is the punctuation, not the sentence.
        let (lo, span) = if kind == Kind::Full {
            (0.3, 0.3)
        } else {
            (0.6, 0.8)
        };
        let secs = self.hold_secs * (lo + span * self.unit());
        self.frames(secs)
    }

    fn frames(&self, secs: f64) -> u64 {
        ((secs * self.fps).round() as u64).max(1)
    }

    fn unit(&mut self) -> f64 {
        f64::from(next_rand(&mut self.rng)) / f64::from(1u32 << 31)
    }

    fn dims(&self, w: usize) -> (usize, usize) {
        let s = Grid::shape(&self.panel, w, w * self.cell, self.aspect);
        (s.cols, s.rows)
    }

    fn view_of(&self, s: Shot) -> View {
        match s.kind {
            Kind::Base => self.home,
            _ => self.view(s.w as usize, s.cx, s.cy),
        }
    }

    /// The view at cell width `w` centred as near `(cx, cy)` as the picture
    /// allows, placed by the rule `Play` places its own view by.
    fn view(&self, w: usize, cx: f64, cy: f64) -> View {
        let (gc, gr) = self.dims(w);
        View {
            w,
            x0: origin(self.cols, gc, cx - gc as f64 / 2.0),
            y0: origin(self.rows, gr, cy - gr as f64 / 2.0),
        }
    }

    /// Mean worth of the blocks a view shows.
    fn framed(&self, v: View) -> f32 {
        let (gc, gr) = self.dims(v.w);
        let span = |o: isize, n: usize, len: usize| {
            let a = o.max(0) as usize / BLOCK;
            let b = ((o + n as isize).max(0) as usize).min(len).div_ceil(BLOCK);
            a..b.max(a + 1)
        };
        let (xs, ys) = (span(v.x0, gc, self.cols), span(v.y0, gr, self.rows));
        let n = xs.len() * ys.len();
        let sum: f32 = ys
            .flat_map(|by| xs.clone().map(move |bx| (bx, by)))
            .map(|(bx, by)| {
                self.weight[by.min(self.brows - 1) * self.bcols + bx.min(self.bcols - 1)]
            })
            .sum();
        sum / n as f32
    }

    /// Re-score every block from the frame on screen. No allocation: the
    /// buffers are sized once in `new`.
    fn measure(&mut self, pic: &[Cell]) {
        for by in 0..self.brows {
            for bx in 0..self.bcols {
                let mut acc = [0.0f32; 3];
                let mut n = 0.0f32;
                for y in by * BLOCK..((by + 1) * BLOCK).min(self.rows) {
                    for x in bx * BLOCK..((bx + 1) * BLOCK).min(self.cols) {
                        let c = pic[y * self.cols + x];
                        let cov = coverage(c.glyph());
                        let rgb = self.pal.get(c.colour()).copied().unwrap_or(0);
                        for (ch, a) in acc.iter_mut().enumerate() {
                            *a += cov * ((rgb >> (16 - 8 * ch)) & 0xFF) as f32 / 255.0;
                        }
                        n += 1.0;
                    }
                }
                self.mean[by * self.bcols + bx] = acc.map(|a| a / n);
            }
        }
        for by in 0..self.brows {
            for bx in 0..self.bcols {
                let m = self.mean[by * self.bcols + bx];
                let mut edge = 0.0;
                for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                    let (x, y) = (bx as isize + dx, by as isize + dy);
                    if x < 0 || y < 0 || x as usize >= self.bcols || y as usize >= self.brows {
                        continue;
                    }
                    let o = self.mean[y as usize * self.bcols + x as usize];
                    edge += (0..3).map(|ch| (m[ch] - o[ch]).abs()).sum::<f32>();
                }
                let luma = 0.3 * m[0] + 0.5 * m[1] + 0.2 * m[2];
                // Squared so the moon outdraws a sky of single stars, and
                // floored so an empty picture still has somewhere to go.
                let v = edge + 0.5 * luma + 0.01;
                self.weight[by * self.bcols + bx] = v * v;
            }
        }
    }
}

/// How much of a cell its glyph inks: the halftone dot's coverage, or all of
/// it for any other glyph.
fn coverage(glyph: usize) -> f32 {
    match font::HALFTONE.iter().position(|&g| usize::from(g) == glyph) {
        Some(i) => COVER[i] as f32,
        None => f32::from(u8::from(glyph != usize::from(font::BLANK))),
    }
}

/// An index drawn with probability proportional to its weight, `u` uniform
/// in 0..1.
fn draw(u: f64, n: usize, w: impl Fn(usize) -> f32) -> usize {
    let total: f32 = (0..n).map(&w).sum();
    let mut r = u as f32 * total;
    for i in 0..n {
        r -= w(i);
        if r < 0.0 {
            return i;
        }
    }
    n - 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ascii_rest::Play;
    use crate::grid::with_test_aspect;
    use crate::saver::Saver;
    use crate::surface::Surface;
    use crate::testalloc::allocs_during;

    /// A cover scene of any size whose every cell moves.
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

    fn knobs(seed: u32) -> Knobs {
        Knobs {
            hold_secs: 1,
            max_zoom_pct: 250,
            seed,
        }
    }

    /// The view `Play` starts a `P` on.
    fn base_of<P: Piece>(panel: &Panel, aspect: usize) -> View {
        with_test_aspect(aspect, || {
            Play::<P>::with_tour(panel, 30, Some(knobs(11)))
                .tour
                .unwrap()
                .tour
                .home
        })
    }

    /// Every view on any panel and any picture: inside its width limits, inside
    /// the picture when it is narrower, centred when wider, and a close-up
    /// never shows ground. And over a long run every kind of shot comes up.
    fn every_view_frames<P: Piece>() {
        for (pw, ph, aspect) in [(1920, 1080, 180), (1920, 1080, 100), (1280, 400, 100)] {
            let (cols, rows) = (P::COLS, P::ROWS);
            let panel = Panel::new(pw, ph, pw);
            let base = base_of::<P>(&panel, aspect);
            let pic: Vec<Cell> = (0..cols * rows)
                .map(|i| Cell::new(font::HALFTONE[i * 7 % 4], (i % 3) as u16))
                .collect();
            let mut t = Tour::new::<P>(&panel, aspect, base, 30, knobs(7));
            let (lo, hi) = (t.min_w, t.max_w);
            let full = t.full();
            let s = Grid::shape(&panel, full.w, full.w, aspect);
            assert!(s.cols >= cols && s.rows >= rows, "full view crops");
            let (mut saw_full, mut saw_base, mut saw_close) = (false, false, false);
            for _ in 0..30 * 300 {
                let v = t.step(&pic);
                let case = format!("{pw}x{ph}@{aspect} {cols}x{rows} {v:?}");
                assert!((lo..=hi).contains(&v.w), "{case}");
                let s = Grid::shape(&panel, v.w, v.w, aspect);
                for (o, n, len) in [(v.x0, s.cols, cols), (v.y0, s.rows, rows)] {
                    if n < len {
                        assert!(o >= 0 && o as usize + n <= len, "{case}");
                    } else {
                        assert_eq!(o, -(((n - len) / 2) as isize), "{case}");
                    }
                }
                if v.w > base.w {
                    assert!(
                        s.cols <= cols && s.rows <= rows,
                        "{case}: close-up shows ground"
                    );
                }
                saw_full |= v == full;
                saw_base |= v == base;
                saw_close |= v.w > base.w;
            }
            assert!(saw_full && saw_base && saw_close, "{pw}x{ph} {cols}x{rows}");
        }
    }

    #[test]
    fn every_view_frames_the_picture() {
        every_view_frames::<Probe<200, 100>>();
        every_view_frames::<Probe<320, 100>>();
        every_view_frames::<Probe<37, 23>>();
    }

    /// Close-ups go where the picture is: a single bright patch in a dark
    /// picture draws most of them.
    #[test]
    fn close_ups_find_the_detail() {
        let panel = Panel::new(1920, 1080, 1920);
        let (cols, rows) = (200, 100);
        let pic: Vec<Cell> = (0..cols * rows)
            .map(|i| {
                let (x, y) = (i % cols, i / cols);
                let lit = (150..170).contains(&x) && (60..80).contains(&y);
                Cell::new(font::HALFTONE[if lit { 3 } else { 0 }], u16::from(lit))
            })
            .collect();
        let base = View {
            w: 10,
            x0: 4,
            y0: 20,
        };
        let mut t = Tour::new::<Probe<200, 100>>(&panel, 180, base, 30, knobs(3));
        t.measure(&pic);
        let (mut hits, mut n) = (0, 0);
        for _ in 0..400 {
            let s = t.close_up();
            let v = t.view_of(s);
            let (gc, gr) = t.dims(v.w);
            let x = (v.x0..v.x0 + gc as isize).contains(&160);
            let y = (v.y0..v.y0 + gr as isize).contains(&70);
            hits += usize::from(x && y);
            n += 1;
        }
        assert!(
            hits * 10 > n * 6,
            "{hits} of {n} close-ups framed the patch"
        );
    }

    /// What reaches the panel — only the reported rects, as simpledrm copies
    /// them out of a shadow buffer full of junk — is exactly a from-scratch
    /// render of the view the tour chose, through every zoom step, pan and
    /// pull back. A geometry change that left stale cells or margins, or
    /// under-reported them, fails here.
    fn panel_is_the_tours_view<P: Piece>(pw: usize, ph: usize, aspect: usize) {
        with_test_aspect(aspect, || {
            let panel = Panel::new(pw, ph, pw);
            let mut play = Play::<P>::with_tour(&panel, 30, Some(knobs(11)));
            let mut buf = vec![0xDEAD_BEEFu32; panel.buf_len()];
            let mut hw = buf.clone();
            let mut fresh = vec![0u32; panel.buf_len()];
            let home = play.tour.as_ref().unwrap().tour.home;
            let (mut moves, mut last) = (0, play.cam.view);
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

                let v = play.cam.view;
                if v != last || n % 25 == 0 {
                    moves += usize::from(v != last);
                    last = v;
                    let mut cam = Camera::new::<P>(&panel, aspect, v, (v.w, v.w));
                    cam.draw::<P>(&play.pic);
                    cam.grid
                        .flush(&mut Surface::new(&mut fresh, &panel), P::PALETTE);
                    assert!(fresh == buf, "{case}: panel is not {v:?}");
                    let g = play.grid();
                    assert_eq!(
                        (g.cols(), g.rows(), g.cell_w()),
                        (cam.grid.cols(), cam.grid.rows(), cam.grid.cell_w())
                    );
                    assert!(
                        g.cells() == cam.grid.cells(),
                        "{case}: grid() is not what was drawn"
                    );
                }
                wide |= v.w < home.w;
                close |= v.w > home.w;
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

    /// Moves, measuring, reshapes, pans and the mirror all draw on buffers
    /// sized at build.
    #[test]
    fn touring_never_allocates() {
        with_test_aspect(180, || {
            let panel = Panel::new(1920, 1080, 1920);
            let mut play = Play::<Probe<200, 100>>::with_tour(&panel, 30, Some(knobs(11)));
            let mut buf = vec![0u32; panel.buf_len()];
            let mut frame = |play: &mut Play<Probe<200, 100>>| {
                let mut s = Surface::new(&mut buf, &panel);
                play.render(&mut s);
                play.mirror();
            };
            frame(&mut play);
            let mut widths = [false; 64];
            let n = allocs_during(|| {
                for _ in 0..1500 {
                    frame(&mut play);
                    widths[play.cam.view.w] = true;
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
            let mut on = Play::<Probe<200, 100>>::with_tour(&panel, 30, Some(knobs(11)));
            let mut off = Play::<Probe<200, 100>>::with_tour(&panel, 30, None);
            let (mut a, mut b) = (vec![0u32; panel.buf_len()], vec![0u32; panel.buf_len()]);
            let home = on.tour.as_ref().unwrap().tour.home;
            let mut held = true;
            for n in 0..900 {
                let mut s = Surface::new(&mut a, &panel);
                on.render(&mut s);
                let da = s.finish();
                let mut s = Surface::new(&mut b, &panel);
                off.render(&mut s);
                let db = s.finish();
                held &= on.cam.view == home;
                if held {
                    assert_eq!(da.runs(), db.runs(), "frame {n}: damage");
                    assert!(a == b, "frame {n}: pixels");
                }
                let (m, o) = (on.mirror(), off.mirror());
                assert_eq!(
                    (m.cols(), m.rows(), m.cell_w(), m.cell_h()),
                    (o.cols(), o.rows(), o.cell_w(), o.cell_h())
                );
                assert!(m.cells() == o.cells(), "frame {n}: mirror");
            }
            assert!(!held, "the tour never left the cover view");
        });
    }
    /// The caption changes the panel only inside its own corner block, never
    /// the picture the piece drew, and costs no allocation per frame — on the
    /// cover view and through every tour step.
    #[test]
    fn a_title_touches_only_its_corner() {
        with_test_aspect(180, || {
            type P = Play<Probe<200, 100>>;
            let panel = Panel::new(1920, 1080, 1920);
            let mut on = P::with_tour(&panel, 30, Some(knobs(11)));
            let mut off = P::with_tour(&panel, 30, Some(knobs(11)));
            on.title = Some(Title::new("night-coast", 1, Probe::<200, 100>::PALETTE));
            let (mut a, mut b) = (vec![0u32; panel.buf_len()], vec![0u32; panel.buf_len()]);
            let frame = |on: &mut P, off: &mut P, a: &mut [u32], b: &mut [u32]| {
                on.render(&mut Surface::new(a, &panel));
                off.render(&mut Surface::new(b, &panel));
            };
            frame(&mut on, &mut off, &mut a, &mut b);
            let n = allocs_during(|| {
                for _ in 0..600 {
                    frame(&mut on, &mut off, &mut a, &mut b);
                }
            });
            assert_eq!(n, 0, "the title allocated");
            for _ in 0..30 {
                frame(&mut on, &mut off, &mut a, &mut b);
                assert!(on.pic == off.pic, "the title reached the picture");
                let g = &on.cam.grid;
                let (rows, cw, ch) = (g.rows(), g.cell_w(), g.cell_h());
                let t = on.title.as_ref().unwrap();
                let corner = |x: usize, y: usize| x < t.w() * cw && y >= (rows - t.h()) * ch;
                let mut inside = 0;
                for y in 0..panel.h {
                    for x in 0..panel.w {
                        let i = y * panel.w + x;
                        if corner(x, y) {
                            inside += usize::from(a[i] != b[i]);
                        } else {
                            assert_eq!(a[i], b[i], "pixel {x},{y} outside the title");
                        }
                    }
                }
                assert!(inside > 0, "no title at cell width {cw}");
            }
        });
    }
}
