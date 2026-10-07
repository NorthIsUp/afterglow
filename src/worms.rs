//! Worms — segmented crawlers wandering a toroidal grid, head bright, body
//! trailing behind it.
//!
//! # Why this is sparse
//!
//! A worm is a ring buffer of cell indices. Extending it touches three cells —
//! the new head, the old head demoted into its band shade, and the tail erased
//! — whatever the body's length. Nothing else in the scene moves, so
//! `flush_sparse` with a caller-maintained dirty list costs a handful of cells
//! a frame instead of a grid scan.
//!
//! Numbers are quoted at `SAVER_FPS=15`, the rate the pod runs at, because fps
//! is not a free parameter here: `subs` is `speed / fps` in half-cell substeps,
//! so 15fps owes two substeps a frame where 30 owes one, and lays twice the
//! cells. `saver::frame` measures about 2us at 1920x1080/15fps (1us at 30, for
//! the same work per second).
//!
//! Damage is wider than that sounds: a dozen worms in a dozen places is a dozen
//! short scanline runs, a measured median of 384 of the panel's 1072 covered
//! rows at 15fps — 224 at 30. So the shadow-to-hardware copy is this saver's
//! real cost, not the blits, and `WORMS_COUNT` is the knob that moves it —
//! each worm is worth about a cell height of damage a frame per substep,
//! wherever it happens to be.
//!
//! # Ownership, and why a cell remembers who lit it
//!
//! Worms cross, so a cell can be a live segment of two of them at once. `refs`
//! counts how many segments stand on a cell and `owner` records which worm
//! painted it last — two different questions, and answering only the second is
//! how this shipped with a hole in it. A tail drop clears its cell only at
//! `refs == 0`; if it still owns a cell someone else is standing on, `rehome`
//! hands the cell to a worm that still holds it, in that worm's colour.
//! Without that, worm B crossing worm A and then dragging its tail over the
//! shared cell punched a dark hole through A's body.
//!
//! # Turning
//!
//! Angular velocity is a damped random walk, not the heading. Perturbing the
//! heading directly gives a jitter that reads as noise; perturbing its
//! derivative gives momentum, so a worm commits to a curve and comes out of it.

use crate::font;
use crate::grid::{bake, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, next_rand};

/// Five families of three: head, then the two body shades the stripes
/// alternate between. Separate ranges per family rather than one ramp, so a
/// worm can never borrow a neighbour's hue by drifting one index. The head is
/// near-white in its own hue — at one cell, brightness is what says which end
/// is the front.
#[rustfmt::skip]
const PAL_RGB: [[u8; 3]; 16] = [
    [0x00, 0x00, 0x00], //  0 background
    [0xE8, 0xFF, 0xD8], //  1 green head
    [0x48, 0xC0, 0x58], //  2 green light band
    [0x1C, 0x60, 0x28], //  3 green dark band
    [0xD8, 0xFC, 0xFF], //  4 cyan head
    [0x38, 0xB0, 0xD0], //  5 cyan light band
    [0x14, 0x54, 0x68], //  6 cyan dark band
    [0xFF, 0xF0, 0xD0], //  7 amber head
    [0xE0, 0xA0, 0x30], //  8 amber light band
    [0x70, 0x48, 0x0C], //  9 amber dark band
    [0xFF, 0xD8, 0xF8], // 10 magenta head
    [0xC8, 0x50, 0xA8], // 11 magenta light band
    [0x5C, 0x1C, 0x4C], // 12 magenta dark band
    [0xE8, 0xE8, 0xFF], // 13 violet head
    [0x70, 0x70, 0xD8], // 14 violet light band
    [0x2C, 0x2C, 0x74], // 15 violet dark band
];
const PAL: [u32; 16] = bake(&PAL_RGB);

const FAMILIES: u16 = 5;

/// Both ends fill their cell. Segmentation comes from the alternating band
/// shade, not from a glyph with a margin: a margin glyph reads as a dotted line
/// running one way and a solid bar running the other, because the atlas cell is
/// twice as tall as it is wide.
const HEAD_GLYPH: u16 = font::SOLID;
const BODY_GLYPH: u16 = font::SOLID;

/// Which of the two band shades segment `t` wears, as an offset from the
/// family's head index.
#[inline]
fn band(t: u32, len: u32) -> u16 {
    ((t / len) % 2) as u16 + 1
}

/// Substeps are sized so a head never moves more than this in one, which is
/// what keeps consecutive body cells adjacent — a longer step would leave gaps
/// in the body at high `WORMS_SPEED`.
const MAX_SUBSTEP: f32 = 0.5;

struct Worm {
    x: f32,
    y: f32,
    ang: f32,
    spin: f32,
    /// Cell indices, newest at `h`, oldest at `h + 1`. Fixed length: the body
    /// length is the ring length, so nothing is allocated or freed as it moves.
    ring: Vec<u32>,
    h: usize,
    n: usize,
    /// Segments laid down, ever. Drives the band pattern; wraps harmlessly.
    t: u32,
    fam: u16,
}

impl Worm {
    /// The segment `back` places behind the head; `at(0)` is the head.
    #[inline]
    fn at(&self, back: usize) -> u32 {
        self.ring[(self.h + self.ring.len() - back) % self.ring.len()]
    }
}

pub struct Worms {
    grid: Grid,
    cols: usize,
    rows: usize,
    worms: Vec<Worm>,
    /// Which worm lit each cell, 1-based; 0 is unlit. See the module doc.
    owner: Vec<u16>,
    /// How many live segments — of any worm — sit on each cell. `owner` says
    /// who painted it; this says whether anyone still needs it. See the module
    /// doc.
    refs: Vec<u16>,
    dirty: Vec<u32>,
    /// Cells per substep, and how many substeps a frame owes.
    step: f32,
    subs: usize,
    /// Radians per substep, and the torque the random walk applies.
    max_spin: f32,
    jerk: f32,
    /// Segments per stripe.
    band: u32,
    rng: u32,
}

#[inline]
fn unit(rng: &mut u32) -> f32 {
    next_rand(rng) as f32 / 2_147_483_648.0
}

impl Worms {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        let cell_w = env_num(&["WORMS_CELL_W"], 16, 4, 64) as usize;
        let cell_h = env_num(&["WORMS_CELL_H"], 16, 4, 128) as usize;
        let count = env_num(&["WORMS_COUNT"], 12, 1, 64) as usize;
        let len = env_num(&["WORMS_LEN"], 44, 2, 400) as usize;
        let speed = env_num(&["WORMS_SPEED"], 14, 1, 200) as f32;
        let turn = env_num(&["WORMS_TURN"], 150, 1, 2000) as f32;
        let band = env_num(&["WORMS_BAND"], 3, 1, 40) as u32;

        let grid = Grid::new(panel, cell_w, cell_h);
        let (cols, rows) = (grid.cols(), grid.rows());
        let fps = fps.max(1) as f32;

        let per_frame = speed / fps;
        let subs = (per_frame / MAX_SUBSTEP).ceil().max(1.0) as usize;
        let step = per_frame / subs as f32;
        let max_spin = turn.to_radians() / (fps * subs as f32);

        let mut rng = 0x5EED_BEEF;
        let mut worms = Vec::with_capacity(count);
        for i in 0..count {
            worms.push(Worm {
                x: unit(&mut rng) * cols as f32,
                y: unit(&mut rng) * rows as f32,
                ang: unit(&mut rng) * std::f32::consts::TAU,
                spin: 0.0,
                ring: vec![0; len],
                h: 0,
                n: 0,
                t: 0,
                fam: (i as u16) % FAMILIES,
            });
        }

        let mut w = Self {
            grid,
            cols,
            rows,
            worms,
            owner: vec![0; cols * rows],
            refs: vec![0; cols * rows],
            // Reserved below, once the layout loop has stopped pushing into
            // it: reserving here and clearing would leave the frame loop
            // running on whatever capacity the layout happened to grow.
            dirty: Vec::new(),
            step,
            subs,
            max_spin,
            band,
            // Enough torque to change a curve within a second or so, but not so
            // much that a worm can reverse its turn every substep.
            jerk: max_spin * 0.55,
            rng,
        };
        // Lay each worm out at full length behind its head, so frame 0 shows
        // the scene rather than a dozen dots growing into it.
        for _ in 0..len {
            for k in 0..w.worms.len() {
                w.advance(k);
            }
        }
        // A substep lays at most two cells (a diagonal crossing lays the corner
        // cell too) and each lays at most three: new head, demoted head,
        // evicted tail. This is the only allocation the frame loop ever sees.
        w.dirty = Vec::with_capacity(count * subs * 6 + 8);
        w
    }

    /// One substep of worm `k`: turn, move, and restitch the head end if the
    /// head landed in a new cell.
    fn advance(&mut self, k: usize) {
        let (cols, rows) = (self.cols, self.rows);
        let (max_spin, jerk, step) = (self.max_spin, self.jerk, self.step);
        let torque = (unit(&mut self.rng) - 0.5) * 2.0 * jerk;

        let w = &mut self.worms[k];
        // Damped, then clamped: the damping is what stops the walk parking at
        // the clamp and drawing perfect circles forever.
        w.spin = (w.spin * 0.9 + torque).clamp(-max_spin, max_spin);
        w.ang += w.spin;
        w.x += w.ang.cos() * step;
        w.y += w.ang.sin() * step;
        if w.x < 0.0 {
            w.x += cols as f32;
        }
        if w.y < 0.0 {
            w.y += rows as f32;
        }
        if w.x >= cols as f32 {
            w.x -= cols as f32;
        }
        if w.y >= rows as f32 {
            w.y -= rows as f32;
        }
        let (nx, ny) = ((w.x as usize) % cols, (w.y as usize) % rows);
        let head = (w.n > 0).then(|| w.at(0) as usize);
        let Some(h) = head else {
            self.lay(k, ny * cols + nx);
            return;
        };
        let (lx, ly) = (h % cols, h / cols);
        if (nx, ny) == (lx, ly) {
            return;
        }
        // A substep that crosses both axes at once would leave the two cells
        // touching only at a corner, which draws as a dotted diagonal. Lay the
        // corner cell too, so the body is always 4-connected.
        if nx != lx && ny != ly {
            self.lay(k, ly * cols + nx);
        }
        self.lay(k, ny * cols + nx);
    }

    /// Extend worm `k` onto cell `i`: evict the tail, demote the old head into
    /// its band shade, and light the new head.
    fn lay(&mut self, k: usize, i: usize) {
        let band_len = self.band;
        let w = &mut self.worms[k];
        let fam = w.fam * 3 + 1;
        let len = w.ring.len();
        w.h = (w.h + 1) % len;
        let evicted = if w.n == len {
            Some(w.ring[w.h])
        } else {
            w.n += 1;
            None
        };
        w.ring[w.h] = i as u32;
        // The segment being demoted keeps the band it was laid down in, so the
        // stripes stay put in SPACE as the worm crawls over them. Banding by
        // position instead would shimmer whenever a worm turned.
        let demote = (w.n >= 2).then(|| (w.at(1), band(w.t.wrapping_sub(1), band_len)));
        w.t = w.t.wrapping_add(1);

        if let Some(old) = evicted {
            let o = old as usize;
            self.refs[o] -= 1;
            if self.refs[o] == 0 {
                self.owner[o] = 0;
                self.put(old, Cell::CLEAR);
            } else if self.owner[o] == k as u16 + 1 {
                // We painted a cell someone else is still standing on.
                self.rehome(o);
            }
        }
        if let Some((prev, shade)) = demote {
            // Only while this worm still owns it: a worm that crossed us has
            // taken the cell, and recolouring it would speckle that worm.
            if self.owner[prev as usize] == k as u16 + 1 {
                self.put(prev, Cell::new(BODY_GLYPH, fam + shade));
            }
        }
        self.refs[i] += 1;
        self.owner[i] = k as u16 + 1;
        self.put(i as u32, Cell::new(HEAD_GLYPH, fam));
    }

    /// Give a cell back to a worm that still holds it, in that worm's own
    /// colour. Reached only when a tail drop finds the cell refcounted above
    /// zero — a crossing, so a few hundredths of a frame — which is what pays
    /// for the scan.
    fn rehome(&mut self, i: usize) {
        let band_len = self.band;
        let found = self.worms.iter().enumerate().find_map(|(j, w)| {
            let b = (0..w.n).find(|&b| w.at(b) as usize == i)?;
            let fam = w.fam * 3 + 1;
            // `at(b)` was laid at t - 1 - b, and a segment keeps the band it was
            // laid in — same rule as the demote above.
            let c = if b == 0 {
                Cell::new(HEAD_GLYPH, fam)
            } else {
                let shade = band(w.t.wrapping_sub(1 + b as u32), band_len);
                Cell::new(BODY_GLYPH, fam + shade)
            };
            Some((j as u16 + 1, c))
        });
        if let Some((o, c)) = found {
            self.owner[i] = o;
            self.put(i as u32, c);
        }
    }

    /// The only writer. A no-op write is honest damage that still costs a blit,
    /// so it is skipped; everything else goes into `dirty` in the same breath,
    /// which is what makes an unreported write unwritable here.
    #[inline]
    fn put(&mut self, i: u32, c: Cell) {
        if self.grid.cell(i as usize) == c {
            return;
        }
        self.grid.set(i as usize, c);
        self.dirty.push(i);
    }
}

impl Saver for Worms {
    fn render(&mut self, s: &mut Surface<'_>) {
        self.dirty.clear();
        for _ in 0..self.subs {
            for k in 0..self.worms.len() {
                self.advance(k);
            }
        }
        // Two cells in one row collapse into one damage run: `Damage::mark`
        // merges only into the LAST run.
        self.dirty.sort_unstable();
        self.grid.flush_sparse(s, &PAL, &self.dirty);
    }

    fn name(&self) -> &'static str {
        "worms"
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

    fn panel() -> Panel {
        // 1080 / 16 leaves an 8-line strip no cell covers — the whole point of
        // the frame-0 assertion below.
        Panel::new(1920, 1080, 1920)
    }

    /// The rate the pod actually runs at (`SAVER_FPS=15` in
    /// homelab-gitops' `k8s/apps/screensaver/deployment.yaml`). It is not a free parameter:
    /// halving fps doubles `subs`, so every substep-shaped claim — damage,
    /// frame cost — has to be made here or it is a claim about a config
    /// nothing runs.
    const SHIPPED_FPS: u32 = 15;

    fn worms() -> Worms {
        Worms::new(&panel(), SHIPPED_FPS)
    }

    /// T1: frame 0 must cover the panel, strip included, and must cover it with
    /// the scene rather than with a reported black rectangle.
    #[test]
    fn frame_zero_paints_the_whole_panel() {
        let p = panel();
        let mut c = worms();
        let mut buf = vec![0u32; p.buf_len()];
        assert!(
            !p.h.is_multiple_of(c.grid.cell_h()),
            "test panel proves nothing"
        );

        let d = saver::frame(&mut c, &mut buf, &p);
        assert_eq!(d.rows(), p.h, "frame 0 must paint the whole panel");
        let lit = buf.iter().filter(|&&px| px != 0).count();
        assert!(lit > 10_000, "frame 0 painted nothing ({lit} lit pixels)");
    }

    /// T2: the dirty list names every cell that was written. A framebuffer diff
    /// is structurally blind to this — an unreported write is never blitted, so
    /// the pixels never change and the diff finds nothing.
    #[test]
    fn every_written_cell_is_reported() {
        let p = panel();
        // Both rates, the shipped one first: `subs` is `speed / fps / 0.5`
        // rounded up, so 15fps takes two substeps a frame where 30 takes one
        // and damage is nearly double. Testing only at 30 measured a config
        // the panel never runs.
        for fps in [SHIPPED_FPS, 30] {
            let mut c = Worms::new(&p, fps);
            let mut buf = vec![0u32; p.buf_len()];
            let cells = c.grid.cols() * c.grid.rows();
            let mut rows = Vec::new();
            for n in 0..600 {
                let d = saver::frame(&mut c, &mut buf, &p);
                for i in 0..cells {
                    assert_eq!(
                        c.grid.cell(i),
                        c.grid.cells()[i],
                        "fps {fps}, frame {n}: cell {i} was written but not reported"
                    );
                }
                if n > 0 {
                    rows.push(d.rows());
                }
            }
            rows.sort_unstable();
            let median = rows[rows.len() / 2];
            // Non-vacuous both ways: it draws something every frame, and it
            // stays under half a full repaint. Measured medians of the panel's
            // 1072 covered rows: 384 at 15fps, 224 at 30. Each worm is one
            // short run wherever it is, so this is really a ceiling on
            // WORMS_COUNT times the cell height, not a claim about the drawing.
            assert!(median > 0, "fps {fps}: no damage at all after frame 0");
            assert!(
                median * 2 < c.grid.rows() * c.grid.cell_h(),
                "fps {fps}: median {median} rows a frame is not sparse"
            );
        }
    }

    /// T3: the frame loop allocates nothing. `dirty` is the only per-frame Vec;
    /// a Vec that never grows past its reserve never reallocates, and `clear`
    /// keeps capacity, so unchanged capacity IS "did not allocate".
    #[test]
    fn render_never_allocates() {
        let p = panel();
        let mut c = worms();
        let mut buf = vec![0u32; p.buf_len()];
        let reserved = c.dirty.capacity();
        assert!(reserved > 0, "nothing was reserved for the frame loop");

        let mut worst = 0;
        for _ in 0..50_000 {
            saver::frame(&mut c, &mut buf, &p);
            worst = worst.max(c.dirty.len());
            assert_eq!(
                c.dirty.capacity(),
                reserved,
                "`dirty` grew past its reserve: the render path allocated"
            );
        }
        // A reserve nothing ever fills proves nothing: a moving frame costs
        // three or four cells per worm, so the worst frame must reach that.
        assert!(
            worst >= c.worms.len() * 3,
            "`dirty` never held a whole frame's worth ({worst})"
        );
    }

    /// No worm is erased from under itself. `owner` records who PAINTED a
    /// shared cell, not who still needs it: where two worms cross, the second
    /// one's tail drop used to clear a cell that was still a live segment of
    /// the first — a dark hole travelling through its body (~0.05 a frame).
    /// Nothing else here sees it: the hole is a cell the owner map already
    /// agrees is unlit, and it heals as the other worm crawls on.
    #[test]
    fn a_body_is_never_erased_from_under_it() {
        let p = panel();
        for fps in [SHIPPED_FPS, 30] {
            let mut c = Worms::new(&p, fps);
            let mut buf = vec![0u32; p.buf_len()];
            let (mut checked, mut crossings) = (0usize, 0usize);
            for n in 0..600 {
                saver::frame(&mut c, &mut buf, &p);
                for (k, w) in c.worms.iter().enumerate() {
                    for b in 0..w.n {
                        assert!(
                            c.grid.cell(w.at(b) as usize) != Cell::CLEAR,
                            "fps {fps}, frame {n}: worm {k} segment {b} (cell {}) was erased from under it",
                            w.at(b)
                        );
                        checked += 1;
                    }
                }
                crossings += c.refs.iter().filter(|&&r| r > 1).count();
            }
            // Non-vacuous: the crossings this guards against really happen, and
            // the bodies really are full length.
            assert!(crossings > 0, "fps {fps}: no two worms ever crossed");
            assert!(checked > 100_000, "fps {fps}: only {checked} segments");
        }
    }

    /// T4: the scene is exactly the live segments and nothing else. A worm that
    /// fails to drop its tail, or erases a cell another worm has taken, breaks
    /// this on the frame it happens — a lit-cell count or a ratio would not,
    /// because a trail on a torus saturates instead of growing.
    #[test]
    fn the_scene_is_exactly_the_live_segments() {
        let p = panel();
        let mut c = worms();
        let mut buf = vec![0u32; p.buf_len()];
        let cells = c.grid.cols() * c.grid.rows();
        for n in 0..600 {
            saver::frame(&mut c, &mut buf, &p);
            for i in 0..cells {
                assert_eq!(
                    c.grid.cell(i) != Cell::CLEAR,
                    c.owner[i] != 0,
                    "frame {n}: cell {i} is lit by nobody, or owned by nobody"
                );
                let o = c.owner[i];
                if o == 0 {
                    continue;
                }
                // And every owned cell is a segment its owner still holds,
                // wearing one of that worm's own three colours. A worm whose
                // head cell is taken by one crossing it is fine and expected —
                // that is what `owner` is for — so what has to hold is that the
                // cell belongs to whoever painted it, not to who came first.
                let w = &c.worms[o as usize - 1];
                assert!(
                    (0..w.n).any(|b| w.at(b) as usize == i),
                    "frame {n}: cell {i} is owned by worm {} but is not in its body",
                    o - 1
                );
                let fam = (w.fam * 3 + 1) as usize;
                assert!(
                    (fam..fam + 3).contains(&c.grid.cell(i).colour()),
                    "frame {n}: cell {i} wears a colour worm {}'s family does not own",
                    o - 1
                );
            }
        }
    }

    /// A body is unbroken: consecutive segments are edge-neighbours on the
    /// torus, never corner-neighbours. A diagonal substep that laid only its
    /// destination would draw a dotted worm — which looks like an art choice
    /// rather than a bug, and no other test here would notice.
    #[test]
    fn a_body_is_never_broken() {
        let p = panel();
        let mut c = worms();
        let mut buf = vec![0u32; p.buf_len()];
        let (cols, rows) = (c.grid.cols(), c.grid.rows());
        let mut pairs = 0usize;
        for n in 0..600 {
            saver::frame(&mut c, &mut buf, &p);
            for (k, w) in c.worms.iter().enumerate() {
                for b in 1..w.n {
                    let (a, z) = (w.at(b - 1) as usize, w.at(b) as usize);
                    let dx = ((a % cols) as isize - (z % cols) as isize).unsigned_abs();
                    let dy = ((a / cols) as isize - (z / cols) as isize).unsigned_abs();
                    // A wrap shows up as a step of cols-1 / rows-1.
                    let dx = dx.min(cols - dx);
                    let dy = dy.min(rows - dy);
                    assert!(
                        (dx, dy) == (1, 0) || (dx, dy) == (0, 1),
                        "frame {n}: worm {k} has a {dx},{dy} gap between segments"
                    );
                    pairs += 1;
                }
            }
        }
        // Non-vacuous: a loop over empty worms would assert nothing at all.
        // Dropping the corner cell from `advance` fails this within a frame or
        // two, which is the evidence that these pairs are really being checked.
        assert!(
            c.worms.iter().all(|w| w.n == w.ring.len()),
            "the worms never grew a full body to check"
        );
        assert!(pairs > 100_000, "only {pairs} segment pairs were checked");
    }

    /// A worm moves. Everything above holds perfectly for a dozen worms sitting
    /// still, which is the one way this could be entirely wrong and entirely
    /// green.
    #[test]
    fn worms_actually_crawl() {
        let p = panel();
        let mut c = worms();
        let mut buf = vec![0u32; p.buf_len()];
        let start: Vec<u32> = c.worms.iter().map(|w| w.at(0)).collect();
        for _ in 0..60 {
            saver::frame(&mut c, &mut buf, &p);
        }
        let moved = c
            .worms
            .iter()
            .zip(&start)
            .filter(|(w, &s)| w.at(0) != s)
            .count();
        assert_eq!(moved, c.worms.len(), "a worm stayed put for two seconds");
    }
}
