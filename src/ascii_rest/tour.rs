//! The scenes' slow camera, Ken Burns style: every shot a slow push in or pull
//! out with a gentle pan, eased at both ends, and every few shots a pull back
//! to the whole picture. `ASCII_REST_TOUR_CUTS`
//! instead cuts between framings and drifts slowly within each.
//!
//! Zoom is cell size, never resampling. Every view draws one picture cell per
//! grid cell, so the ordered dither survives and the dots just get bigger. So
//! zoom steps a pixel of cell width at a time, while the pan is pixel-precise:
//! the grid is drawn shifted by the part of a cell the camera has moved past,
//! and each step re-places the picture so the shot's focus stays put.
//!
//! Close-ups go where the picture has something in it. Each shot re-measures
//! the frame on screen in 4x4-cell blocks — the dither's period, so a flat
//! gradient's dither does not read as detail — and aims at a block weighted by
//! its contrast with its neighbours and its brightness.
//!
//! Time is counted in frames, like `Play`'s clock, so a dump shows the tour the
//! panel does.

use super::halftone::COVER;
use super::title::Title;
use super::{Camera, Piece};
use crate::font;
use crate::grid::{Cell, Grid};
use crate::next_rand;
use crate::surface::Panel;

/// The dither's period: blocks this size average it away.
const BLOCK: usize = 4;

/// The least a close-up magnifies the whole picture's cell.
const MIN_ZOOM: f64 = 1.25;

/// A shot's length as a share of `ASCII_REST_TOUR_SHOT_SECS`, drawn uniformly.
const SHOT_SHARE: (f64, f64) = (0.75, 1.5);

/// Cuts hold each framing for less: a drift is a smaller move than a shot.
const CUT_SHARE: (f64, f64) = (0.5, 1.0);

/// The rest at the end of a shot before the next sets off.
const SETTLE_SECS: f64 = 0.5;

/// The slowest a shot may move before it is cut short: a zoom step every this
/// many seconds, or a pan of this many pixels a second. A pull back of three
/// cell widths stretched over half a minute would sit still for ten seconds
/// at a time.
const STEP_SECS: f64 = 1.5;
const PAN_PX_PER_SEC: f64 = 20.0;

/// The farthest a shot pans, in panel widths and heights at its wider end: a
/// Ken Burns move drifts, it does not sweep.
const MAX_PAN: f64 = 0.5;

/// What the panel shows: a cell width in square-glass pixels, the picture cell
/// under the grid's top-left cell (negative when the picture is padded), and
/// the pixels that cell is drawn past the panel's corner.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct View {
    pub w: usize,
    pub x0: isize,
    pub y0: isize,
    pub dx: usize,
    pub dy: usize,
}

/// The tour's knobs, read once per saver build.
#[derive(Clone, Copy, Debug)]
pub struct Knobs {
    pub shot_secs: u32,
    pub max_zoom_pct: u32,
    pub cuts: bool,
    pub seed: u32,
}

impl Knobs {
    /// None when `ASCII_REST_TOUR=0`.
    pub fn from_env() -> Option<Self> {
        (crate::env_num(&["ASCII_REST_TOUR"], 0, 0, 1) == 1).then(|| Self {
            shot_secs: crate::env_num(&["ASCII_REST_TOUR_SHOT_SECS"], 20, 4, 600) as u32,
            max_zoom_pct: crate::env_num(&["ASCII_REST_TOUR_MAX_ZOOM_PCT"], 250, 100, 600) as u32,
            cuts: crate::env_num(&["ASCII_REST_TOUR_CUTS"], 0, 0, 1) == 1,
            seed: crate::saver_seed(&["ASCII_REST_TOUR_SEED"], 0x70C4_5EED),
        })
    }
}

/// A scene's camera, and the mirror's view of the same picture.
pub(super) struct Touring {
    tour: Tour,
    /// The untoured geometry showing what the panel shows: a zoom step is
    /// a new geometry, and re-describing the mirror for each would reconnect
    /// every viewer a dozen times a shot. So the cells follow the tour and the
    /// grid never does.
    mirror: Camera,
    /// The panel view `mirror`'s maps were last aimed at.
    followed: Option<View>,
}

impl Touring {
    pub(super) fn new<P: Piece>(
        panel: &Panel,
        aspect: usize,
        pic: (usize, usize),
        base: View,
        fps: u32,
        k: Knobs,
    ) -> Self {
        Self {
            tour: Tour::new::<P>(panel, aspect, pic, base, fps, k),
            mirror: Camera::new::<P>(panel, aspect, pic, base, (base.w, base.w), false),
            followed: None,
        }
    }

    /// The narrowest and widest cell any view uses, for sizing buffers once.
    pub(super) fn widths(&self) -> (usize, usize) {
        (self.tour.base.w as usize, self.tour.max_w)
    }

    /// Advance the tour a frame and, on a frame the piece drew, point the
    /// panel's camera where it says. Every move repaints the whole panel, and
    /// between the piece's frames nothing else changes: moving there too
    /// would double the cost of motion the eye reads at the picture's rate.
    pub(super) fn steer(&mut self, pic: &[Cell], cam: &mut Camera, drew: bool) {
        let v = self.tour.step(pic);
        if drew && v != cam.view {
            cam.aim(&self.tour.panel, self.tour.aspect, v);
        }
    }

    /// What `cam` shows of `pic`, on the untoured grid, under the title if
    /// there is one. Only called while someone is watching.
    pub(super) fn mirror(&mut self, pic: &[Cell], cam: &Camera, title: Option<&Title>) -> &Grid {
        if self.followed != Some(cam.view) {
            self.mirror.follow(cam);
            self.followed = Some(cam.view);
        }
        self.mirror.draw(pic);
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
    Close,
}

/// A framing: a cell width, fractional because a shot passes through every
/// width between its ends, and the picture point at the panel's centre.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Shot {
    w: f64,
    cx: f64,
    cy: f64,
    kind: Kind,
}

pub struct Tour {
    panel: Panel,
    aspect: usize,
    /// The picture, in cells.
    cols: usize,
    rows: usize,
    base: Shot,
    max_w: usize,
    max_zoom: f64,
    from: Shot,
    to: Shot,
    /// Frames into the shot, its eased part, and the rest after it.
    at: u64,
    len: u64,
    settle: u64,
    fps: f64,
    shot_secs: f64,
    cuts: bool,
    /// Close-ups left before the next pull back.
    wide_in: u32,
    /// The last few close-ups, newest first.
    recent: [Option<Shot>; 3],
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
    pub fn new<P: Piece>(
        panel: &Panel,
        aspect: usize,
        (cols, rows): (usize, usize),
        base: View,
        fps: u32,
        k: Knobs,
    ) -> Self {
        let max_zoom = f64::from(k.max_zoom_pct) / 100.0;
        let max_w = base.w.max((base.w as f64 * max_zoom).round() as usize);
        let (bcols, brows) = (cols.div_ceil(BLOCK), rows.div_ceil(BLOCK));
        let s = Grid::shape(panel, base.w, base.w, aspect);
        let base = Shot {
            w: base.w as f64,
            cx: base.x0 as f64 + s.cols as f64 / 2.0,
            cy: base.y0 as f64 + s.rows as f64 / 2.0,
            kind: Kind::Base,
        };
        Self {
            panel: *panel,
            aspect,
            cols,
            rows,
            base,
            max_w,
            max_zoom,
            from: base,
            to: base,
            at: 0,
            len: 0,
            settle: 0,
            fps: fps.max(1) as f64,
            shot_secs: f64::from(k.shot_secs),
            cuts: k.cuts,
            wide_in: 0,
            recent: [None; 3],
            rng: k.seed,
            pal: P::PALETTE,
            bcols,
            brows,
            mean: vec![[0.0; 3]; bcols * brows],
            weight: vec![0.0; bcols * brows],
        }
    }

    /// Advance one frame and say what to draw. `pic` is the frame on screen,
    /// read only when a shot picks its next target.
    pub fn step(&mut self, pic: &[Cell]) -> View {
        if self.at >= self.len + self.settle {
            self.advance(pic);
        }
        let s = (self.at as f64 / self.len as f64).min(1.0);
        self.at += 1;
        // Half smoothstep, half linear: eased at both ends, but not so flat
        // there that a shot's first and last zoom steps stall.
        let e = 0.5 * s + 0.5 * s * s * (3.0 - 2.0 * s);
        let (a, b) = (self.from, self.to);
        // Linear in width, so the zoom's one-pixel steps come evenly: through
        // log width the last steps of a pull back, where each is a fifth of
        // the picture's size, would come seconds apart.
        self.view(Shot {
            w: a.w + (b.w - a.w) * e,
            cx: a.cx + (b.cx - a.cx) * e,
            cy: a.cy + (b.cy - a.cy) * e,
            kind: b.kind,
        })
    }

    fn advance(&mut self, pic: &[Cell]) {
        self.measure(pic);
        let next = self.pick();
        let (share, settle) = if self.cuts {
            self.from = next;
            self.to = self.drift(next);
            (CUT_SHARE, 0.0)
        } else {
            self.from = self.to;
            self.to = next;
            (SHOT_SHARE, SETTLE_SECS)
        };
        let (lo, hi) = share;
        let u = self.unit();
        let secs = (self.shot_secs * (lo + (hi - lo) * u)).min(self.busy_secs().max(4.0));
        self.len = self.frames(secs);
        self.settle = (settle * self.fps).round() as u64;
        self.at = 0;
    }

    fn pick(&mut self) -> Shot {
        if self.max_w <= self.base.w as usize {
            return self.base;
        }
        // Every few close-ups a pull back to the whole picture, which is
        // where they set off from.
        if self.to.kind == Kind::Base {
            self.wide_in = 2 + next_rand(&mut self.rng) % 3;
        } else if self.wide_in == 0 {
            return self.base;
        }
        self.wide_in -= 1;
        // A close-up next to the last one is a twitch, not a shot.
        let mut shot = self.close_up();
        for _ in 0..4 {
            if self.apart(shot, self.to) {
                break;
            }
            shot = self.close_up();
        }
        if !self.cuts {
            shot = self.near(shot);
        }
        self.recent.rotate_right(1);
        self.recent[0] = Some(shot);
        shot
    }

    /// A push in from a wide framing, a pull out from a close one: the zoom
    /// range splits at its geometric middle and the shot crosses it.
    fn close_up(&mut self) -> Shot {
        let lo = MIN_ZOOM.min(self.max_zoom);
        let mid = (lo * self.max_zoom).sqrt();
        let (a, b) = if self.to.w / self.base.w < mid {
            (mid, self.max_zoom)
        } else {
            (lo, mid)
        };
        let z = a + (b - a) * self.unit();
        let w = (self.base.w * z).clamp(self.base.w + 1.0, self.max_w as f64);
        let u = self.unit();
        let (b, bcols) = (BLOCK as f64, self.bcols);
        let at = |i: usize| {
            (
                (i % bcols) as f64 * b + b / 2.0,
                (i / bcols) as f64 * b + b / 2.0,
            )
        };
        let i = draw(u, self.weight.len(), |i| self.weight[i] * self.fresh(at(i)));
        let (cx, cy) = at(i);
        self.within(Shot {
            w,
            cx,
            cy,
            kind: Kind::Close,
        })
    }

    /// `s` moved a little: a slightly closer framing a short way off, for a
    /// cut to drift through.
    fn drift(&mut self, s: Shot) -> Shot {
        let w = (s.w * (1.0 + 0.08 * self.unit())).min(self.max_w as f64);
        let (gc, gr) = self.extent(w);
        let cx = s.cx + (self.unit() - 0.5) * gc * 0.15;
        let cy = s.cy + (self.unit() - 0.5) * gr * 0.15;
        self.within(Shot { w, cx, cy, ..s })
    }

    /// The longest the shot from `from` to `to` can take and still move at
    /// [`STEP_SECS`] and [`PAN_PX_PER_SEC`]: by its zoom steps, or by its pan
    /// in pixels at its narrower cell.
    fn busy_secs(&self) -> f64 {
        let (a, b) = (self.view(self.from), self.view(self.to));
        let steps = a.w.abs_diff(b.w) as f64;
        let w = a.w.min(b.w) as f64;
        let h = w * self.aspect as f64 / 100.0;
        let pan = ((self.to.cx - self.from.cx) * w)
            .abs()
            .max(((self.to.cy - self.from.cy) * h).abs());
        (steps * STEP_SECS).max(pan / PAN_PX_PER_SEC)
    }

    /// `s`, pulled back along the way from the current framing until the pan
    /// is at most [`MAX_PAN`].
    fn near(&self, s: Shot) -> Shot {
        let from = self.to;
        let (gc, gr) = self.extent(s.w.min(from.w));
        let d = ((s.cx - from.cx) / gc).hypot((s.cy - from.cy) / gr);
        if d <= MAX_PAN {
            return s;
        }
        let k = MAX_PAN / d;
        self.within(Shot {
            cx: from.cx + (s.cx - from.cx) * k,
            cy: from.cy + (s.cy - from.cy) * k,
            ..s
        })
    }

    /// The panel's size in picture cells at cell width `w`.
    fn extent(&self, w: f64) -> (f64, f64) {
        let h = w * self.aspect as f64 / 100.0;
        (self.panel.w as f64 / w, self.panel.h as f64 / h)
    }

    /// `s` with its centre pulled in as far as its framing can reach, so a
    /// shot does not spend its time steering at a wall.
    fn within(&self, s: Shot) -> Shot {
        let (gc, gr) = self.extent(s.w);
        let clamp = |c: f64, span: f64, len: usize| {
            let len = len as f64;
            if span >= len {
                len / 2.0
            } else {
                c.clamp(span / 2.0, len - span / 2.0)
            }
        };
        Shot {
            cx: clamp(s.cx, gc, self.cols),
            cy: clamp(s.cy, gr, self.rows),
            ..s
        }
    }

    /// How much a point is worth revisiting: little if a recent close-up
    /// already showed it, so the moon does not get every other shot.
    fn fresh(&self, (x, y): (f64, f64)) -> f32 {
        let shown = self.recent.iter().flatten().any(|s| {
            let (gc, gr) = self.extent(s.w);
            (x - s.cx).abs() < gc / 2.0 && (y - s.cy).abs() < gr / 2.0
        });
        if shown {
            0.05
        } else {
            1.0
        }
    }

    /// Far enough apart in zoom or in place to be worth a shot between.
    fn apart(&self, a: Shot, b: Shot) -> bool {
        let (gc, gr) = self.extent(a.w.max(b.w));
        (a.w / b.w).ln().abs() > 0.25
            || (a.cx - b.cx).abs() * 3.0 > gc
            || (a.cy - b.cy).abs() * 3.0 > gr
    }

    fn frames(&self, secs: f64) -> u64 {
        ((secs * self.fps).round() as u64).max(1)
    }

    fn unit(&mut self) -> f64 {
        f64::from(next_rand(&mut self.rng)) / f64::from(1u32 << 31)
    }

    /// The view drawing `s`: its width rounded to a whole pixel, and the
    /// picture placed to the pixel so `s`'s centre is the panel's — or as
    /// near as the picture's edges allow, or centred when it is the smaller.
    fn view(&self, s: Shot) -> View {
        let w = (s.w.round() as usize).clamp(self.base.w as usize, self.max_w);
        let g = Grid::shape(&self.panel, w, w, self.aspect);
        // Where picture cell 0 lands, then the grid cell under the panel's
        // corner and how far into it the corner is.
        let place = |len: usize, cell: usize, c: f64, span: usize| {
            let (pic, span, cell) = ((len * cell) as isize, span as isize, cell as isize);
            let o = if pic >= span {
                ((span as f64 / 2.0 - c * cell as f64).round() as isize).clamp(span - pic, 0)
            } else {
                (span - pic) / 2
            };
            ((-o).div_euclid(cell), (-o).rem_euclid(cell) as usize)
        };
        let (x0, dx) = place(self.cols, g.cell_w, s.cx, self.panel.w);
        let (y0, dy) = place(self.rows, g.cell_h, s.cy, self.panel.h);
        View { w, x0, y0, dx, dy }
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
    use crate::ascii_rest::tests::SHAPES;
    use crate::ascii_rest::Play;
    use crate::grid::with_test_aspect;
    use crate::saver::Saver;
    use crate::surface::Surface;
    use crate::testalloc::allocs_during;

    /// A scene of any size whose every cell moves.
    struct Probe(usize);

    impl Piece for Probe {
        const NAME: &'static str = "probe";
        const FPS: u32 = 15;
        const PALETTE: &'static [u32] = &[0x00_0000, 0xFF_FFFF, 0xFF_8000];
        const GROUND: u32 = 0x10_2030;

        fn new(cols: usize, _: usize) -> Self {
            Self(cols)
        }

        fn frame(&mut self, t: f64, out: &mut [Cell]) {
            let k = (t * 15.0) as usize;
            for (i, c) in out.iter_mut().enumerate() {
                let (x, y) = (i % self.0, i / self.0);
                let dot = crate::font::HALFTONE[(x / 3 + y / 2 + k) % 4];
                *c = Cell::new(dot, ((x * y + k) % 3) as u16);
            }
        }
    }

    /// Short shots, so a test sees many.
    fn knobs(seed: u32) -> Knobs {
        Knobs {
            shot_secs: 4,
            max_zoom_pct: 250,
            cuts: false,
            seed,
        }
    }

    /// The picture `Play` gives a scene on `panel`, and the view it starts on.
    fn base_of(panel: &Panel, aspect: usize) -> ((usize, usize), View) {
        with_test_aspect(aspect, || {
            let cam = Play::<Probe>::with_tour(panel, 30, None).cam;
            ((cam.cols, cam.rows), cam.view)
        })
    }

    /// Where picture cell 0 lands on the panel, in pixels, along each axis.
    fn origin_px(v: View, g: crate::grid::Shape) -> (isize, isize) {
        (
            -(v.x0 * g.cell_w as isize + v.dx as isize),
            -(v.y0 * g.cell_h as isize + v.dy as isize),
        )
    }

    /// Every view on any panel and any picture: inside its width limits, the
    /// picture covering the panel where it is the larger and centred where it
    /// is the smaller. And over a long run pull backs to the whole picture
    /// and close-ups both come up, in either style.
    fn every_view_frames(cuts: bool) {
        for (pw, ph, aspect) in SHAPES {
            let panel = Panel::new(pw, ph, pw);
            let ((cols, rows), base) = base_of(&panel, aspect);
            let pic: Vec<Cell> = (0..cols * rows)
                .map(|i| Cell::new(font::HALFTONE[i * 7 % 4], (i % 3) as u16))
                .collect();
            let k = Knobs { cuts, ..knobs(7) };
            let mut t = Tour::new::<Probe>(&panel, aspect, (cols, rows), base, 30, k);
            let (lo, hi) = (base.w, t.max_w);
            let (mut saw_back, mut saw_close) = (false, false);
            for _ in 0..30 * 300 {
                let v = t.step(&pic);
                let case = format!("{pw}x{ph}@{aspect} {cols}x{rows} cuts {cuts} {v:?}");
                assert!((lo..=hi).contains(&v.w), "{case}");
                let g = Grid::shape(&panel, v.w, v.w, aspect);
                assert!(v.dx < g.cell_w && v.dy < g.cell_h, "{case}");
                let (ox, oy) = origin_px(v, g);
                for (o, pic, span) in [(ox, cols * g.cell_w, pw), (oy, rows * g.cell_h, ph)] {
                    let (pic, span) = (pic as isize, span as isize);
                    if pic >= span {
                        assert!((span - pic..=0).contains(&o), "{case}: shows ground");
                    } else {
                        assert_eq!(o, (span - pic) / 2, "{case}: not centred");
                    }
                }
                saw_back |= saw_close && v.w == base.w;
                saw_close |= v.w > base.w;
            }
            assert!(saw_back && saw_close, "{pw}x{ph} {cols}x{rows} cuts {cuts}");
        }
    }

    #[test]
    fn every_view_frames_the_picture() {
        for cuts in [false, true] {
            every_view_frames(cuts);
        }
    }

    /// Ken Burns at the default shot length on pine: never still for longer
    /// than 4 s — a settle and the ease's slow end over the last half pixel
    /// of a pull back, at the whole picture where nothing else can move —
    /// zoom one pixel of cell width
    /// at a time, and between zoom steps the picture glides a few pixels a
    /// frame at most, rather than jumping a cell.
    #[test]
    fn the_camera_glides_and_never_rests_for_long() {
        let panel = Panel::new(1920, 1080, 1920);
        let (pic_size, base) = base_of(&panel, 180);
        let pic: Vec<Cell> = (0..pic_size.0 * pic_size.1)
            .map(|i| Cell::new(font::HALFTONE[(i / 7) % 4], (i % 3) as u16))
            .collect();
        let k = Knobs {
            shot_secs: 20,
            ..knobs(5)
        };
        let mut t = Tour::new::<Probe>(&panel, 180, pic_size, base, 30, k);
        let mut last = t.step(&pic);
        let (mut still, mut longest, mut glides, mut zooms) = (0, 0, 0, 0);
        for n in 0..30 * 600 {
            let v = t.step(&pic);
            let case = format!("frame {n}: {last:?} -> {v:?}");
            if v == last {
                still += 1;
                longest = longest.max(still);
            } else {
                still = 0;
            }
            if v.w == last.w {
                let g = Grid::shape(&panel, v.w, v.w, 180);
                let ((ax, ay), (bx, by)) = (origin_px(last, g), origin_px(v, g));
                assert!((ax - bx).abs() <= 4 && (ay - by).abs() <= 4, "{case}");
                glides += usize::from(v != last);
            } else {
                assert_eq!(v.w.abs_diff(last.w), 1, "{case}");
                zooms += 1;
            }
            last = v;
        }
        assert!(longest <= 30 * 4, "still for {longest} frames");
        assert!(
            glides > 30 * 60 && zooms > 20,
            "{glides} glides, {zooms} zooms"
        );
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
            dx: 0,
            dy: 0,
        };
        let mut t = Tour::new::<Probe>(&panel, 180, (cols, rows), base, 30, knobs(3));
        t.measure(&pic);
        let (mut hits, mut n) = (0, 0);
        for _ in 0..400 {
            let s = t.close_up();
            let (gc, gr) = t.extent(s.w);
            let x = (s.cx - 160.0).abs() < gc / 2.0;
            let y = (s.cy - 70.0).abs() < gr / 2.0;
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
    /// render of the view the tour chose, through every zoom step, pixel pan
    /// and pull back. A geometry change or shift that left stale cells or
    /// margins, or under-reported them, fails here.
    fn panel_is_the_tours_view(pw: usize, ph: usize, aspect: usize) {
        type P = Probe;
        with_test_aspect(aspect, || {
            let panel = Panel::new(pw, ph, pw);
            let mut play = Play::<P>::with_tour(&panel, 30, Some(knobs(11)));
            let pic = (play.cam.cols, play.cam.rows);
            let mut buf = vec![0xDEAD_BEEFu32; panel.buf_len()];
            let mut hw = buf.clone();
            let mut fresh = vec![0u32; panel.buf_len()];
            let home = play.cam.view;
            let (mut moves, mut last) = (0, play.cam.view);
            let (mut close, mut shifted) = (false, false);
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
                    let mut cam = Camera::new::<P>(&panel, aspect, pic, v, (v.w, v.w), true);
                    cam.aim(&panel, aspect, v);
                    cam.draw(&play.pic);
                    cam.grid
                        .flush(&mut Surface::new(&mut fresh, &panel), P::PALETTE);
                    assert!(fresh == buf, "{case}: panel is not {v:?}");
                    let g = play.grid();
                    assert_eq!(g.shape_of(), cam.grid.shape_of());
                    assert!(
                        g.cells() == cam.grid.cells(),
                        "{case}: grid() is not what was drawn"
                    );
                }
                close |= v.w > home.w;
                shifted |= v.dx > 0 && v.dy > 0;
            }
            assert!(
                moves > 20 && close && shifted,
                "{} {pw}x{ph}: {moves} moves, close {close} shifted {shifted}",
                P::NAME
            );
        });
    }

    #[test]
    fn the_panel_is_always_the_tours_view() {
        panel_is_the_tours_view(1920, 1080, 180);
        panel_is_the_tours_view(1080, 1920, 100);
        panel_is_the_tours_view(1024, 768, 100);
    }

    /// Shots, measuring, reshapes, shifts and the mirror all draw on buffers
    /// sized at build.
    #[test]
    fn touring_never_allocates() {
        with_test_aspect(180, || {
            let panel = Panel::new(1920, 1080, 1920);
            let mut play = Play::<Probe>::with_tour(&panel, 30, Some(knobs(11)));
            let mut buf = vec![0u32; panel.buf_len()];
            let mut frame = |play: &mut Play<Probe>| {
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

    /// The mirror's geometry is the untoured view's on every frame, so the
    /// tour never bumps its epoch.
    #[test]
    fn the_mirrors_geometry_is_the_untoured_savers() {
        with_test_aspect(180, || {
            let panel = Panel::new(1920, 1080, 1920);
            let mut on = Play::<Probe>::with_tour(&panel, 30, Some(knobs(11)));
            let mut off = Play::<Probe>::with_tour(&panel, 30, None);
            let (mut a, mut b) = (vec![0u32; panel.buf_len()], vec![0u32; panel.buf_len()]);
            let home = off.cam.view;
            let mut moved = false;
            for n in 0..900 {
                on.render(&mut Surface::new(&mut a, &panel));
                off.render(&mut Surface::new(&mut b, &panel));
                moved |= on.cam.view.w != home.w;
                let (m, o) = (on.mirror(), off.mirror());
                assert_eq!(m.shape_of(), o.shape_of(), "frame {n}: mirror geometry");
                assert_eq!(m.shift_of(), (0, 0));
            }
            assert!(moved, "the tour never left the whole picture");
        });
    }

    /// Each mirror cell is the panel's cell under its centre, through every
    /// close-up, pixel pan and pull back.
    #[test]
    fn the_mirror_shows_what_the_panel_shows() {
        with_test_aspect(180, || {
            let panel = Panel::new(1920, 1080, 1920);
            let mut play = Play::<Probe>::with_tour(&panel, 30, Some(knobs(11)));
            let mut buf = vec![0u32; panel.buf_len()];
            let home = play.cam.view;
            let mut close = 0;
            for n in 0..3000 {
                play.render(&mut Surface::new(&mut buf, &panel));
                let v = play.cam.view;
                let g = play.grid().shape_of();
                let (dx, dy) = play.grid().shift_of();
                let shown = play.grid().cells().to_vec();
                let m = play.mirror();
                let ms = m.shape_of();
                for my in 0..ms.rows {
                    for mx in 0..ms.cols {
                        let (px, py) = (
                            mx * ms.cell_w + ms.cell_w / 2,
                            my * ms.cell_h + ms.cell_h / 2,
                        );
                        let (gx, gy) = ((px + dx) / g.cell_w, (py + dy) / g.cell_h);
                        let want = if gx < g.cols && gy < g.rows {
                            shown[gy * g.cols + gx]
                        } else {
                            Cell::CLEAR
                        };
                        let got = m.cells()[my * ms.cols + mx];
                        assert_eq!(got, want, "frame {n} {v:?}: mirror cell {mx},{my}");
                    }
                }
                close += usize::from(v.w > home.w);
            }
            assert!(close > 0, "no close-up");
        });
    }

    /// The caption changes the panel only inside its own corner block, never
    /// the picture the piece drew, and costs no allocation per frame — on the
    /// untoured view and through every tour step and shift.
    #[test]
    fn a_title_touches_only_its_corner() {
        with_test_aspect(180, || {
            type P = Play<Probe>;
            let panel = Panel::new(1920, 1080, 1920);
            let mut on = P::with_tour(&panel, 30, Some(knobs(11)));
            let mut off = P::with_tour(&panel, 30, Some(knobs(11)));
            on.title = Some(Title::new("night-coast", 1, Probe::PALETTE));
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
                let (cw, ch) = (g.cell_w(), g.cell_h());
                let (dx, dy) = g.shift_of();
                let (xs, ys) = g.inside();
                let (tw, th) = on.title.as_ref().unwrap().size(xs.len(), ys.len()).unwrap();
                let (x0, x1) = (xs.start * cw - dx, (xs.start + tw) * cw - dx);
                let (y0, y1) = ((ys.end - th) * ch - dy, ys.end * ch - dy);
                assert!(x0 < x1 && y1 <= panel.h, "the title left the panel");
                let mut inside = 0;
                for y in 0..panel.h {
                    for x in 0..panel.w {
                        let i = y * panel.w + x;
                        if (x0..x1).contains(&x) && (y0..y1).contains(&y) {
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
