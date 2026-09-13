//! Satori — slow colour-field composition.
//!
//! A BSP of the panel into a handful of rectangles, each holding one muted
//! tone, all of it moving far slower than a glance: tones walk a closed ribbon
//! of 24 hues so every intermediate colour on the way to a target is itself a
//! colour the composition could have had, and the split lines glide a cell at a
//! time so the layout recomposes without a cut.
//!
//! Full repaint (`Grid::flush`), not the sparse path, and deliberately: the
//! scene is almost always identical to the last frame, so the diff finds
//! nothing and the frame costs one u32 compare per cell. The sparse path would
//! buy a compare we can already afford and hand us the one bug it can have.

use crate::font;
use crate::grid::{bake, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, next_rand};

/// Hue ribbon: a CYCLE, so a tone can walk to any other hue the short way round
/// and never pass through a colour that is not part of the scheme. Muted and
/// low-contrast on purpose — these are grounds, not accents.
#[rustfmt::skip]
const BASE: [[u8; 3]; 24] = [
    [0x3A,0x4A,0x6B], [0x44,0x55,0x7A], [0x4E,0x5F,0x7E], [0x4A,0x6A,0x79],
    [0x48,0x72,0x68], [0x4E,0x7A,0x60], [0x5A,0x7E,0x58], [0x6B,0x84,0x52],
    [0x7E,0x8A,0x4E], [0x8E,0x8A,0x48], [0x9C,0x85,0x44], [0xA8,0x7A,0x3E],
    [0xB0,0x6C,0x38], [0xB2,0x5C,0x34], [0xAE,0x4C,0x32], [0xA4,0x3E,0x34],
    [0x96,0x34,0x38], [0x86,0x30,0x3E], [0x74,0x30,0x4A], [0x63,0x34,0x50],
    [0x55,0x3A,0x5E], [0x4A,0x3E,0x68], [0x42,0x44,0x6E], [0x3E,0x46,0x68],
];

const HUES: usize = BASE.len();

/// Per-hue luminance steps, in percent of the base. A tone names a hue and a
/// level, so the same colour can sit anywhere from recessive to luminous, and
/// the edge treatment is a step down this ladder rather than a second palette.
const LEVELS: [u32; 5] = [46, 68, 100, 126, 150];

const LVLS: usize = LEVELS.len();

const fn ribbon() -> [[u8; 3]; 1 + HUES * LVLS] {
    let mut out = [[7u8, 7, 10]; 1 + HUES * LVLS];
    let mut h = 0;
    while h < HUES {
        let mut l = 0;
        while l < LVLS {
            let mut c = 0;
            while c < 3 {
                let v = BASE[h][c] as u32 * LEVELS[l] / 100;
                out[1 + h * LVLS + l][c] = if v > 255 { 255 } else { v as u8 };
                c += 1;
            }
            l += 1;
        }
        h += 1;
    }
    out
}

const PAL_RGB: [[u8; 3]; 1 + HUES * LVLS] = ribbon();
const PAL: [u32; 1 + HUES * LVLS] = bake(&PAL_RGB);

/// The four textures a cell can have, as (glyph, level offset), chosen by how
/// many cells from its field's edge a cell sits. A field is FLAT across its
/// middle and only darkens in the last three cells: ramping all the way to the
/// centre draws a bullseye, which is a target, not a Rothko. Index 0 is the
/// outermost ring — a 50% dither of the dimmest tone against black, the closest
/// this atlas gets to a frayed edge.
const TEX_GLYPH: [u16; 4] = [font::SHADE, font::SHADE, font::SOLID, font::SOLID];
const TEX_OFF: [i32; 4] = [-2, -2, -1, 0];

/// Distance from the edge, in cells, at which each texture takes over. Three
/// steps and then flat: every level change lands on a rectangular contour, and
/// a contour in the middle of a field reads as a picture frame however subtle
/// the step is. Only the ones hugging the edge read as an edge.
const TEX_AT: [usize; 4] = [0, 1, 2, 3];

/// Smallest field, in cells — derived, not taste: the edge treatment eats
/// `TEX_AT.len()` cells on each side, so anything narrower than this has no
/// flat interior left and reads as a stray stripe rather than a field. Also the
/// guard that keeps a gliding split line from collapsing a field to nothing.
const MIN: usize = 2 * TEX_AT.len() + 3;

#[derive(Clone, Copy)]
struct Tone {
    hue: u8,
    lvl: u8,
    thue: u8,
    tlvl: u8,
}

impl Tone {
    /// One step toward the target, the short way round the ribbon.
    fn step(&mut self) {
        let d = (self.thue as usize + HUES - self.hue as usize) % HUES;
        if d != 0 {
            self.hue = if d * 2 <= HUES {
                ((self.hue as usize + 1) % HUES) as u8
            } else {
                ((self.hue as usize + HUES - 1) % HUES) as u8
            };
        }
        self.lvl = match self.lvl.cmp(&self.tlvl) {
            std::cmp::Ordering::Less => self.lvl + 1,
            std::cmp::Ordering::Greater => self.lvl - 1,
            std::cmp::Ordering::Equal => self.lvl,
        };
    }
}

/// A BSP node. `kids[0] == 0` marks a leaf: node 0 is the root and can never be
/// a child, so the marker costs no extra field.
#[derive(Clone, Copy)]
struct Node {
    kids: [u16; 2],
    /// 0 splits along x, 1 along y.
    axis: u8,
    pos: u16,
    tgt: u16,
    leaf: u8,
}

pub struct Satori {
    grid: Grid,
    cols: usize,
    nodes: Vec<Node>,
    tone: Vec<Tone>,
    /// Per cell: which field it belongs to, and which edge texture.
    /// Rebuilt only when a split line actually moves a whole cell.
    region: Vec<u8>,
    tex: Vec<u8>,
    fade: u32,
    retone: u32,
    glide: u32,
    drift: u32,
    frame: u32,
    rng: u32,
}

impl Satori {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        Self::build(
            panel,
            fps,
            env_num(&["SATORI_CELL_W"], 16, 4, 64) as usize,
            env_num(&["SATORI_CELL_H"], 16, 4, 64) as usize,
            env_num(&["SATORI_FIELDS"], 9, 2, 32) as usize,
            env_num(&["SATORI_SEED"], 0x5A70_1234, 1, u32::MAX as i64) as u32,
        )
    }

    /// The geometry and the seed come in as arguments so the tests can vary
    /// them without `set_var`: cargo runs tests in parallel threads and the
    /// environment is process-wide, so one test's geometry was landing in
    /// another's saver.
    fn build(
        panel: &Panel,
        fps: u32,
        cell_w: usize,
        cell_h: usize,
        fields: usize,
        seed: u32,
    ) -> Self {
        let mut rng = seed;
        // Everything below is expressed in seconds and converted here, so a
        // 15 fps panel and a 30 fps dump move at the same speed.
        let per = |k: &str, def: i64, hi: i64| {
            (env_num(&[k], def, 1, hi) as u32)
                .saturating_mul(fps)
                .max(1)
        };
        let fade = per("SATORI_FADE_SEC", 2, 60);
        let retone = per("SATORI_TONE_SEC", 12, 600);
        let glide = per("SATORI_GLIDE_SEC", 3, 60);
        let drift = per("SATORI_DRIFT_SEC", 30, 3600);

        let grid = Grid::new(panel, cell_w, cell_h);
        let (cols, rows) = (grid.cols(), grid.rows());
        let (nodes, tone) = compose(cols, rows, fields, &mut rng);

        let mut s = Self {
            grid,
            cols,
            nodes,
            tone,
            region: vec![0; cols * rows],
            tex: vec![0; cols * rows],
            fade,
            retone,
            glide,
            drift,
            frame: 0,
            rng,
        };
        s.rebuild();
        s
    }

    /// Re-derive the per-cell field and texture maps from the tree. Called from
    /// `render` on the frames a split line moves, so it allocates nothing: both
    /// maps are sized once in `new` and the walk recurses on the stack.
    fn rebuild(&mut self) {
        let (cols, rows) = (self.grid.cols(), self.grid.rows());
        paint(
            &self.nodes,
            0,
            (0, 0, cols, rows),
            cols,
            &mut self.region,
            &mut self.tex,
        );
    }

    fn step(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        if self.frame.is_multiple_of(self.fade) {
            for t in &mut self.tone {
                t.step();
            }
        }
        if self.frame.is_multiple_of(self.retone) {
            let i = next_rand(&mut self.rng) as usize % self.tone.len();
            // A small hop, never a jump across the ribbon: the walk stays in the
            // neighbourhood the composition already lives in.
            let hop = 1 + next_rand(&mut self.rng) as usize % 3;
            let back = next_rand(&mut self.rng) & 1 == 0;
            let lvl = (next_rand(&mut self.rng) % LVLS as u32) as u8;
            let t = &mut self.tone[i];
            let d = if back { HUES - hop } else { hop };
            t.thue = ((t.hue as usize + d) % HUES) as u8;
            t.tlvl = lvl;
        }
        if self.frame.is_multiple_of(self.drift) {
            self.retarget_split();
        }
        if self.frame.is_multiple_of(self.glide) && self.glide_splits() {
            self.rebuild();
        }
    }

    fn retarget_split(&mut self) {
        let internal = self.nodes.iter().filter(|n| n.kids[0] != 0).count();
        if internal == 0 {
            return;
        }
        let pick = next_rand(&mut self.rng) as usize % internal;
        let span = self.grid.cols().max(self.grid.rows()) as u32;
        let jitter = (next_rand(&mut self.rng) % (span / 6 + 1)) as u16;
        let back = next_rand(&mut self.rng) & 1 == 0;
        let mut seen = 0;
        for n in &mut self.nodes {
            if n.kids[0] == 0 {
                continue;
            }
            if seen == pick {
                // `paint` clamps into the node's own range, so an out-of-range
                // target just glides the line to the edge of what it may have.
                n.tgt = if back {
                    n.pos.saturating_sub(jitter)
                } else {
                    n.pos.saturating_add(jitter)
                };
                return;
            }
            seen += 1;
        }
    }

    /// One cell of movement per split line. True when anything moved.
    fn glide_splits(&mut self) -> bool {
        let mut moved = false;
        for n in &mut self.nodes {
            if n.kids[0] == 0 || n.pos == n.tgt {
                continue;
            }
            n.pos = if n.pos < n.tgt { n.pos + 1 } else { n.pos - 1 };
            moved = true;
        }
        moved
    }
}

impl Saver for Satori {
    fn render(&mut self, s: &mut Surface<'_>) {
        self.step();
        let (grid, region, tex, tone, cols) = (
            &mut self.grid,
            &self.region[..],
            &self.tex[..],
            &self.tone[..],
            self.cols,
        );
        grid.fill(|cx, cy| {
            let i = cy * cols + cx;
            let t = tex[i] as usize;
            let tn = tone[region[i] as usize];
            let lvl = (tn.lvl as i32 + TEX_OFF[t]).clamp(0, LVLS as i32 - 1) as u16;
            Cell::new(TEX_GLYPH[t], 1 + tn.hue as u16 * LVLS as u16 + lvl)
        });
        grid.flush(s, &PAL);
    }

    fn name(&self) -> &'static str {
        "satori"
    }

    fn grid(&self) -> &Grid {
        &self.grid
    }

    fn palette(&self) -> &[u32] {
        &PAL
    }
}

/// Where a node's line actually falls: its wish, clamped so neither child drops
/// below `MIN` — or `None` when the range has no room for two of them. The
/// clamp lives here rather than at the writer because a parent that glided may
/// have squeezed this range since the wish was set.
///
/// `None` is the whole point: the old form returned the midpoint of a squeezed
/// range, which is a `MIN` that anything narrower than `2 * MIN` ignores, and
/// it measured 2-cell fields against a stated floor of 11.
fn split_at(a: usize, b: usize, want: usize) -> Option<usize> {
    (b - a >= 2 * MIN).then(|| want.clamp(a + MIN, b - MIN))
}

type Rect = (usize, usize, usize, usize);

fn paint(nodes: &[Node], n: usize, r: Rect, cols: usize, region: &mut [u8], tex: &mut [u8]) {
    let nd = nodes[n];
    let (x0, y0, x1, y1) = r;
    if nd.kids[0] == 0 {
        stamp(nd.leaf, r, cols, region, tex);
        return;
    }
    let (lo, hi) = if nd.axis == 0 { (x0, x1) } else { (y0, y1) };
    let Some(p) = split_at(lo, hi, nd.pos as usize) else {
        // Squeezed below two fields' worth: the whole range goes to one child
        // rather than being cut into stripes. The other field is gone until the
        // glide gives the range back, which is the honest reading of MIN — a
        // field is either at least MIN across or it is not on the panel.
        paint(nodes, nd.kids[0] as usize, r, cols, region, tex);
        return;
    };
    let (a, b) = if nd.axis == 0 {
        ((x0, y0, p, y1), (p, y0, x1, y1))
    } else {
        ((x0, y0, x1, p), (x0, p, x1, y1))
    };
    paint(nodes, nd.kids[0] as usize, a, cols, region, tex);
    paint(nodes, nd.kids[1] as usize, b, cols, region, tex);
}

fn stamp(leaf: u8, r: Rect, cols: usize, region: &mut [u8], tex: &mut [u8]) {
    let (x0, y0, x1, y1) = r;
    for cy in y0..y1 {
        for cx in x0..x1 {
            let d = (cx - x0).min(x1 - 1 - cx).min(cy - y0).min(y1 - 1 - cy);
            let t = TEX_AT.iter().rposition(|&at| d >= at).unwrap_or(0);
            let i = cy * cols + cx;
            region[i] = leaf;
            tex[i] = t as u8;
        }
    }
}

/// Build the composition once: split the panel until there are `fields` of them,
/// then hand each field a hue a short walk from its neighbour's, so the whole
/// thing reads as one palette rather than a set of swatches.
fn compose(cols: usize, rows: usize, fields: usize, rng: &mut u32) -> (Vec<Node>, Vec<Tone>) {
    let leaf = |id: u8| Node {
        kids: [0, 0],
        axis: 0,
        pos: 0,
        tgt: 0,
        leaf: id,
    };
    let mut nodes = vec![leaf(0)];
    let mut bounds: Vec<Rect> = vec![(0, 0, cols, rows)];
    let mut leaves: Vec<usize> = vec![0];

    let mut tries = 0;
    while leaves.len() < fields && tries < fields * 32 {
        tries += 1;
        // Best of three by area: splitting a uniformly random leaf leaves one
        // huge field and a corner of crumbs.
        let k = (0..3)
            .map(|_| next_rand(rng) as usize % leaves.len())
            .max_by_key(|&i| {
                let (x0, y0, x1, y1) = bounds[i];
                (x1 - x0) * (y1 - y0)
            })
            .unwrap_or(0);
        let (x0, y0, x1, y1) = bounds[k];
        let (w, h) = (x1 - x0, y1 - y0);
        let axis = if w * 3 >= h * 4 {
            0
        } else if h * 3 >= w * 4 {
            1
        } else {
            (next_rand(rng) & 1) as u8
        };
        let (lo, hi) = if axis == 0 { (x0, x1) } else { (y0, y1) };
        if hi - lo < 2 * MIN {
            continue;
        }
        // Off-centre but not near an edge: the interesting asymmetries are in
        // the middle 60% of the span, and a sliver is just a stripe.
        let span = hi - lo;
        let a = (lo + span / 5).max(lo + MIN);
        let b = (hi - span / 5).min(hi - MIN);
        let p = a + next_rand(rng) as usize % (b - a + 1);
        let (ra, rb) = if axis == 0 {
            ((x0, y0, p, y1), (p, y0, x1, y1))
        } else {
            ((x0, y0, x1, p), (x0, p, x1, y1))
        };

        let parent = leaves[k];
        let id = nodes[parent].leaf;
        let next_id = leaves.len() as u8;
        let n = nodes.len() as u16;
        nodes.push(leaf(id));
        nodes.push(leaf(next_id));
        nodes[parent] = Node {
            kids: [n, n + 1],
            axis,
            pos: p as u16,
            tgt: p as u16,
            leaf: 0,
        };
        leaves[k] = n as usize;
        bounds[k] = ra;
        leaves.push(n as usize + 1);
        bounds.push(rb);
    }

    let mut hue = (next_rand(rng) % HUES as u32) as u8;
    let tone = leaves
        .iter()
        .map(|_| {
            let hop = 1 + next_rand(rng) as usize % 4;
            let back = next_rand(rng) & 1 == 0;
            hue = ((hue as usize + if back { HUES - hop } else { hop }) % HUES) as u8;
            let lvl = (next_rand(rng) % LVLS as u32) as u8;
            Tone {
                hue,
                lvl,
                thue: hue,
                tlvl: lvl,
            }
        })
        .collect();
    (nodes, tone)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::saver;

    /// 1080 is not a multiple of 32, so the bottom 24 scanlines belong to no
    /// cell — the strip frame 0 has to paint anyway.
    fn panel() -> Panel {
        Panel::new(1920, 1080, 1920)
    }

    /// Explicit geometry, never `set_var`: the tests run in parallel threads
    /// of one process, so an env var set by one is read by every other.
    fn saver_at(cell: usize) -> Satori {
        Satori::build(&panel(), 15, cell, cell, 9, 0x5A70_1234)
    }

    #[test]
    fn frame_zero_paints_the_whole_panel() {
        let p = panel();
        let mut c = saver_at(32);
        assert_ne!(p.h % c.grid.cell_h(), 0, "the test panel divides evenly");
        let mut buf = vec![0u32; p.buf_len()];
        let d = saver::frame(&mut c, &mut buf, &p);
        assert_eq!(d.rows(), p.h, "frame 0 must paint the whole panel");
        assert!(
            buf.iter().filter(|&&px| px != 0).count() > 10_000,
            "frame 0 painted nothing"
        );
    }

    /// Model A cannot under-report, so what this checks is that the claim holds
    /// against the real framebuffer over a run long enough to include tone steps
    /// and a gliding split line.
    #[test]
    fn damage_covers_every_changed_scanline() {
        let p = panel();
        let mut c = saver_at(16);
        let mut buf = vec![0u32; p.buf_len()];
        let mut prev = vec![0u32; p.buf_len()];
        saver::frame(&mut c, &mut buf, &p);
        prev.copy_from_slice(&buf);

        let mut changed_frames = 0;
        for n in 1..900 {
            let d = saver::frame(&mut c, &mut buf, &p);
            let mut dirty_rows = 0;
            let stride = buf.len() / p.h;
            for y in 0..p.h {
                let (a, b) = (y * stride, y * stride + p.w);
                if buf[a..b] != prev[a..b] {
                    dirty_rows += 1;
                    assert!(
                        d.runs()
                            .iter()
                            .any(|&(y0, y1)| y >= y0 as usize && y < y1 as usize),
                        "frame {n}: scanline {y} changed but was not reported"
                    );
                }
            }
            if dirty_rows > 0 {
                changed_frames += 1;
            }
            prev.copy_from_slice(&buf);
        }
        assert!(
            changed_frames > 5,
            "nothing changed in 900 frames: the check never ran"
        );
    }

    /// `render` holds no per-frame Vec — the scene is four buffers sized in
    /// `new`, and the only frame-time write is into `region`/`tex` in place — so
    /// what proves it never allocates is that none of them ever resizes.
    #[test]
    fn render_never_allocates() {
        // Small, but still wide enough in cells for `compose` to split at all.
        let p = Panel::new(640, 400, 640);
        let mut c = Satori::build(&p, 15, 16, 16, 9, 0x5A70_1234);
        let mut buf = vec![0u32; p.buf_len()];
        let sizes = |c: &Satori| {
            (
                (c.region.len(), c.region.capacity()),
                (c.tex.len(), c.tex.capacity()),
                (c.tone.len(), c.tone.capacity()),
                (c.nodes.len(), c.nodes.capacity()),
            )
        };
        let before = sizes(&c);
        assert!(before.0 .1 > 0, "nothing was reserved for the frame loop");
        let mut moves = 0;
        let mut pos: Vec<u16> = c.nodes.iter().map(|n| n.pos).collect();
        for _ in 0..50_000 {
            saver::frame(&mut c, &mut buf, &p);
            if c.nodes.iter().map(|n| n.pos).ne(pos.iter().copied()) {
                moves += 1;
                pos.clear();
                pos.extend(c.nodes.iter().map(|n| n.pos));
            }
            assert_eq!(
                sizes(&c),
                before,
                "a scene buffer resized: render allocated"
            );
        }
        assert!(moves > 0, "no split line ever moved: the run was idle");
    }

    /// The bug this saver can actually have: a split line glides and the
    /// per-cell field map is not rebuilt, so fields keep being drawn to stale
    /// bounds. The invariant is exact — every cell's field and texture is the
    /// one the tree puts there, every frame, for every cell.
    #[test]
    fn the_field_map_always_matches_the_tree() {
        let p = panel();
        let mut c = saver_at(16);
        let mut buf = vec![0u32; p.buf_len()];
        let (cols, rows) = (c.grid.cols(), c.grid.rows());
        let mut want_region = vec![0u8; cols * rows];
        let mut want_tex = vec![0u8; cols * rows];
        let mut glides = 0;
        for n in 0..900 {
            saver::frame(&mut c, &mut buf, &p);
            paint(
                &c.nodes,
                0,
                (0, 0, cols, rows),
                cols,
                &mut want_region,
                &mut want_tex,
            );
            assert!(
                c.region == want_region && c.tex == want_tex,
                "frame {n}: the field map is stale"
            );
            if c.nodes.iter().any(|nd| nd.kids[0] != 0 && nd.pos != nd.tgt) {
                glides += 1;
            }
        }
        assert!(glides > 0, "no split line was ever in motion");
    }

    /// Every leaf rectangle the tree currently describes.
    fn leaves(nodes: &[Node], n: usize, r: Rect, out: &mut Vec<Rect>) {
        let nd = nodes[n];
        let (x0, y0, x1, y1) = r;
        if nd.kids[0] == 0 {
            out.push(r);
            return;
        }
        let (lo, hi) = if nd.axis == 0 { (x0, x1) } else { (y0, y1) };
        let Some(p) = split_at(lo, hi, nd.pos as usize) else {
            leaves(nodes, nd.kids[0] as usize, r, out);
            return;
        };
        let (a, b) = if nd.axis == 0 {
            ((x0, y0, p, y1), (p, y0, x1, y1))
        } else {
            ((x0, y0, x1, p), (x0, p, x1, y1))
        };
        leaves(nodes, nd.kids[0] as usize, a, out);
        leaves(nodes, nd.kids[1] as usize, b, out);
    }

    /// `MIN` is an invariant, not a comment. It used to be a comment: when a
    /// gliding parent squeezed a node's range below `2 * MIN` the clamp
    /// inverted and `split_at` fell back to the midpoint, so fields collapsed —
    /// 2 cells against a stated floor of 11, over these same seeds. A range
    /// with no room for two fields now holds one.
    #[test]
    fn no_field_ever_falls_below_min() {
        let p = panel();
        let mut buf = vec![0u32; p.buf_len()];
        let mut worst = usize::MAX;
        let mut rects = Vec::new();
        for seed in [
            1u32,
            7,
            99,
            0x5A70_1234,
            0xDEAD_BEEF,
            12_345,
            777,
            31_337,
            424_242,
            0x0BAD_F00D,
        ] {
            let mut c = Satori::build(&p, 15, 16, 16, 9, seed);
            let (cols, rows) = (c.grid.cols(), c.grid.rows());
            for n in 0..20_000 {
                saver::frame(&mut c, &mut buf, &p);
                if n % 15 != 0 {
                    continue;
                }
                rects.clear();
                leaves(&c.nodes, 0, (0, 0, cols, rows), &mut rects);
                for (x0, y0, x1, y1) in rects.iter().copied() {
                    worst = worst.min((x1 - x0).min(y1 - y0));
                }
            }
        }
        assert!(
            worst >= MIN,
            "smallest field was {worst} cells against a floor of {MIN}"
        );
    }

    /// The layout has to keep recomposing. Every moving part of it — the drift
    /// that retargets a split, the glide that walks it a cell at a time, and
    /// `split_at` honouring the wish at all — can be deleted by pinning every
    /// split to the midpoint, and the rest of this file stays green: the field
    /// map is still consistent with the tree, it just never changes again.
    #[test]
    fn the_split_lines_actually_move_the_fields() {
        let p = panel();
        let mut c = saver_at(16);
        let mut buf = vec![0u32; p.buf_len()];
        saver::frame(&mut c, &mut buf, &p);
        let first = c.region.clone();
        let mut moved = 0;
        for _ in 0..3_000 {
            saver::frame(&mut c, &mut buf, &p);
            moved += usize::from(c.region != first);
        }
        assert!(
            moved > 0,
            "3000 frames and no cell ever changed field: the layout is frozen"
        );
    }
}
