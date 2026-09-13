//! City — the After Dark night skyline: a band of lit windows along the bottom,
//! scattered lights in the sky above, pure black everywhere else.
//!
//! # What this is a copy of
//!
//! The palette and the layout are sampled off a reference frame, not invented.
//! What the look actually IS — the two colour families, the window styles that
//! keep neighbouring buildings apart, the four roof classes, the band
//! densities and how far this drifts from them on purpose — is written up once,
//! in `k8s/apps/screensaver/README.md`, under "About the city saver". It is not
//! repeated here: the two copies had already drifted from each other and from
//! the measurement inside a single commit.
//!
//! What a reader of THIS file needs, and cannot get from the README:
//!
//! * `STYLES` and `ROOF_CLASS` are the art. Both are weighted draws, both are
//!   validated in a `const` block, and a bad entry is a build failure.
//! * A window is RE-DRAWN at its own slot's odds, never toggled. A toggle has
//!   a fixed point at half lit, so a city generated at 88% fades to 50% over a
//!   few minutes with every test still passing. Re-drawing is memoryless in one
//!   step, so the stationary distribution is exactly the generating one.
//! * `Slot` carries the glyph and the lit percentage per cell because a cell
//!   two buildings overlap belongs to exactly one of them, and the twinkle has
//!   to reproduce that one.
//! * Nothing translates. The scene is still and only shimmers.
//!
//! # Per-frame cost
//!
//! This is the cheapest saver here, by construction rather than by luck. The
//! scene is built once in `new` and lives in the grid between frames; `render`
//! picks a couple of cells, rewrites those, and hands `flush_sparse` the list.
//! There is no per-cell scan, no rebuild, and no allocation — `dirty` is
//! reserved in `new` and only ever `clear`ed, which `render_never_allocates`
//! pins because CLAUDE.md makes the frame loop the top constraint in the repo.
//!
//! Damage is a couple of short runs. The measured figures live in the README
//! beside the other savers' so they can be compared; the invariant that holds
//! them up is here, in `flush_sparse`: `cur` and `prev` are identical after
//! every flush, so a cell written but left out of `dirty` is a test failure
//! rather than a region of the panel frozen forever.
//!
use crate::font;
use crate::grid::{bake, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, next_rand};

/// Sampled from the reference. Index 0 is the background and is never painted
/// over, which is why an unlit region costs nothing.
///
/// 1..=6 is the SKY ramp and 7..=12 the WINDOW ramp, and the split between them
/// is the whole look — see the module doc. They are separate ranges rather than
/// one ordered ramp because a cell must never be able to borrow the other
/// family's colour by drifting one index.
#[rustfmt::skip]
const PAL_RGB: [[u8; 3]; 13] = [
    [0x00, 0x00, 0x00], //  0 background
    [0x68, 0x68, 0x78], //  1 sky, dimmest
    [0x78, 0x78, 0x88], //  2
    [0x88, 0x88, 0x98], //  3
    [0x98, 0x98, 0xA8], //  4
    [0xA8, 0xA8, 0xB8], //  5
    [0xB8, 0xB8, 0xC8], //  6 sky, brightest
    [0xD8, 0xF8, 0xF8], //  7 window, hottest
    [0xC8, 0xE8, 0xF8], //  8
    [0x88, 0xA8, 0xB8], //  9
    [0x68, 0x88, 0x98], // 10
    [0x58, 0x78, 0x88], // 11
    [0x48, 0x68, 0x78], // 12 window, dimmest
];
const PAL: [u32; 13] = bake(&PAL_RGB);

const SKY_LO: u16 = 1;
const SKY_HI: u16 = 6;
const WIN_LO: u16 = 7;
const WIN_HI: u16 = 12;

/// Weighted draws over the two ramps, so brightness varies without every light
/// being equally bright — a uniform draw reads as a flat wash of one grey. Both
/// are skewed dim: a sky of mostly hot dots looks like static, and a skyline of
/// mostly hot windows looks like daylight.
#[rustfmt::skip]
const SKY_DRAW: [u16; 16] = [1, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 4, 4, 5, 6];
#[rustfmt::skip]
const WINDOW_DRAW: [u16; 16] = [7, 8, 8, 9, 9, 9, 10, 10, 10, 10, 11, 11, 11, 12, 12, 12];

/// The draw tables index the palette by hand, so an entry outside its own ramp
/// has to be a build failure — the same bargain `bake_sprites` makes for the
/// toaster art. A runtime check would fire on the panel, headless, at 3am.
const _: () = {
    let mut i = 0;
    while i < SKY_DRAW.len() {
        assert!(
            SKY_DRAW[i] >= SKY_LO && SKY_DRAW[i] <= SKY_HI,
            "a sky draw is outside the sky ramp"
        );
        i += 1;
    }
    let mut i = 0;
    while i < WINDOW_DRAW.len() {
        assert!(
            WINDOW_DRAW[i] >= WIN_LO && WINDOW_DRAW[i] <= WIN_HI,
            "a window draw is outside the window ramp"
        );
        i += 1;
    }
    assert!(
        WIN_HI as usize == PAL.len() - 1,
        "the window ramp must run to the end of the palette"
    );
    let mut i = 0;
    while i < STYLE_DRAW.len() {
        assert!(
            (STYLE_DRAW[i] as usize) < STYLES.len(),
            "a style draw names a style that does not exist"
        );
        i += 1;
    }
    let mut i = 0;
    while i < STYLES.len() {
        assert!(STYLES[i].col_pitch >= 1, "a style would light no columns");
        assert!(
            STYLES[i].glyph != font::BLANK,
            "a style lights its windows with a blank glyph"
        );
        // A style glyph that collided with the star glyph would break
        // `stars.retain` in `new`: it keeps a sky cell when the glyph it finds
        // is STAR_GLYPH, so a building cell would survive as a "star" and the
        // twinkle would paint sky colours inside the silhouette.
        assert!(
            STYLES[i].glyph != STAR_GLYPH,
            "a style glyph collides with the sky's"
        );
        i += 1;
    }
    let mut i = 0;
    while i < ROOF_DRAW.len() {
        assert!(
            (ROOF_DRAW[i] as usize) < ROOF_CLASS.len(),
            "a roof draw names a class that does not exist"
        );
        i += 1;
    }
    let mut i = 0;
    while i < ROOF_CLASS.len() {
        let (lo, hi) = ROOF_CLASS[i];
        assert!(lo < hi, "a roof class is empty or inverted");
        // Ordered tallest-last, which is what lets the class index double as
        // the width-span narrowing below.
        assert!(
            i == 0 || ROOF_CLASS[i - 1].0 > lo,
            "roof classes are not ordered"
        );
        i += 1;
    }
};

/// A lit window: a small square with dark margin all round, so a run of lit
/// cells reads as separate windows rather than as one filled block. That is the
/// difference between a skyline and a black rectangle.
const WINDOW_GLYPH: u16 = font::BLOCK;
/// A curtain-wall strip: one narrow bar the full height of the cell, so a tower
/// in this style has unbroken vertical lines where its neighbour has a grid of
/// dots. It is the loudest of the style differences and the one that does most
/// of the work at a glance.
const STRIP_GLYPH: u16 = font::ASCII[(b'|' - 0x20) as usize];
/// A sky light: smaller and lower in its cell than a window, which is half of
/// why the two families read apart at a glance. The other half is the colour.
const STAR_GLYPH: u16 = font::ASCII[(b'.' - 0x20) as usize];

/// How one building lights its rectangle, assigned once at generation and never
/// changed — a building that restyled as it twinkled would read as a glitch.
///
/// The point is the SEAM. Two adjacent buildings lit the same way are one wall
/// of lights with no edge in it, and the eye cannot tell where one ends. Give
/// them different pitches, different dark floors and different glyphs and the
/// join draws itself, with nothing painting a border.
struct Style {
    glyph: u16,
    /// Light only every `col_pitch`-th column, counted from the building's own
    /// left edge — so the phase differs between neighbours as well as the pitch.
    col_pitch: usize,
    /// Every `band_every`-th floor is dark plant and service, counted UP from
    /// the baseline so the bands line up with the ground and not with the roof.
    /// 0 for none.
    band_every: usize,
    /// Fill as a percent OF `CITY_WINDOW_PCT`, so one knob still scales the
    /// whole skyline while a style can be the dark one.
    fill_pct: u32,
}

#[rustfmt::skip]
const STYLES: [Style; 5] = [
    // dense grid — the baseline everything else is read against
    Style { glyph: WINDOW_GLYPH, col_pitch: 1, band_every: 0, fill_pct: 100 },
    // wide-spaced columns
    Style { glyph: WINDOW_GLYPH, col_pitch: 2, band_every: 0, fill_pct: 100 },
    // banded: a dark service floor every fourth storey
    Style { glyph: WINDOW_GLYPH, col_pitch: 1, band_every: 4, fill_pct: 100 },
    // curtain wall: vertical strips instead of a grid
    Style { glyph: STRIP_GLYPH,  col_pitch: 2, band_every: 0, fill_pct: 100 },
    // mostly dark — one tower in the row with the lights off
    Style { glyph: WINDOW_GLYPH, col_pitch: 1, band_every: 0, fill_pct: 55 },
];

/// Weighted draw over `STYLES`. Nothing is rare here: the styles exist to make
/// neighbours differ, so each has to turn up often enough to be somebody's
/// neighbour.
#[rustfmt::skip]
const STYLE_DRAW: [u8; 16] = [0, 0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 4, 0, 1, 2];

/// Lit-pixel density, in percent, per tenth of the reference frame, top to
/// bottom. The sky (the upper 60%) is generated straight from this. The lower
/// 40% is generated by BUILDINGS, so 43/63/47 is a measurement this has to
/// reproduce rather than a parameter it can set — see `STREET_ROWS`.
const BAND_DENSITY: [u32; 10] = [0, 29, 40, 31, 22, 20, 43, 63, 47, 0];
const BANDS: usize = BAND_DENSITY.len();

/// Percent of the panel height, off the same measurement: the sky fills the top
/// 60%, the skyline the next 30%, and the bottom tenth is empty — every
/// building's baseline is `BASE_PCT` down, not at the panel edge.
const SKY_PCT: usize = 60;
const BASE_PCT: usize = 90;
/// Roof height classes, as a percent of panel height (smaller = taller), and the
/// weighted draw over them. FOUR CLASSES, not one uniform spread, because the
/// spread is what the eye reads: a skyline is low blocks and mid-rise with a
/// handful of towers standing well clear of them, and a uniform draw over one
/// narrow range gives a flat band with no towers at all — which is what this
/// saver shipped with first, and it was the one thing that looked wrong.
///
/// Spires are one draw in sixteen on purpose. Make them common and the skyline
/// becomes a wall; drop them and it becomes a hedge.
const ROOF_CLASS: [(usize, usize); 4] = [
    (70, 78), // low blocks
    (62, 70), // mid-rise
    (52, 62), // towers
    (40, 52), // spires
];
#[rustfmt::skip]
const ROOF_DRAW: [u8; 16] = [0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 2, 2, 2, 3, 1];

/// Floors at street level that light at half rate. This is what puts band 8
/// (47%) under band 7 (63%) in the reference: the bottom of a tower is lobby
/// and plant, not offices.
const STREET_ROWS: usize = 2;

/// Building width in cells, and how much of it the next one overlaps. The span
/// narrows by roof class, so the tall things are the slim things — a 12-cell
/// spire is a slab, and a slab does not read as a tower. The overlap is the
/// depth cue: a nearer building's rectangle is written over the one behind, so
/// the silhouette has corners rather than a picket fence.
const W_MIN: usize = 3;
const W_SPAN: usize = 10;
const OVERLAP_MAX: usize = 3;

/// One window slot's own settings, so the twinkle reproduces the building it
/// belongs to rather than some global average of all of them.
#[derive(Clone, Copy)]
struct Slot {
    glyph: u16,
    lit_pct: u8,
}

pub struct City {
    grid: Grid,
    /// Every window SLOT, lit or not. Twinkle picks from here and toggles;
    /// a slot inside two overlapping buildings appears twice and merely
    /// twinkles twice as often.
    windows: Vec<u32>,
    /// What each entry of `windows` was built with, parallel to it. Carrying
    /// the slot's own glyph is what makes a building's style survive the
    /// twinkle; carrying its own lit percentage is what stops the skyline
    /// DRAINING — a plain on/off toggle has a fixed point at 50%, so a city
    /// generated at 88% would fade to half lit over a few minutes and every
    /// damage test would still pass while it happened.
    slot: Vec<Slot>,
    /// The sky lights, which stay lit for good — a star that blinked out reads
    /// as a dead pixel, not as a twinkle. They only change brightness.
    stars: Vec<u32>,
    /// This frame's changed cells, sorted before the flush so `Damage` merges
    /// neighbours. Capacity is reserved in `new`; `render` never allocates.
    dirty: Vec<u32>,
    /// Toggles per frame in 8.8 fixed point, with their accumulators. Fixed
    /// point rather than an integer count because the sky rate is deliberately
    /// below one cell per frame and rounding it up to 1 is a third of the
    /// "subtle" gone.
    win_rate: u32,
    sky_rate: u32,
    win_acc: u32,
    sky_acc: u32,
    rng: u32,
}

/// Whole units owed this frame, carrying the fraction.
#[inline]
fn due(acc: &mut u32, rate: u32) -> u32 {
    *acc += rate;
    let n = *acc >> 8;
    *acc &= 0xFF;
    n
}

#[inline]
fn pick(rng: &mut u32, draw: &[u16; 16]) -> u16 {
    draw[next_rand(rng) as usize % draw.len()]
}

impl City {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        // 12x16 rather than the 16x32 the other savers use: a window wants to
        // be small, and at 16x32 a 1080p panel holds 33 rows of them, which is
        // three floors per building.
        let cell_w = env_num(&["CITY_CELL_W"], 12, 4, 64) as usize;
        let cell_h = env_num(&["CITY_CELL_H"], 16, 4, 128) as usize;
        let mut grid = Grid::new(panel, cell_w, cell_h);
        let (cols, rows) = (grid.cols(), grid.rows());

        // Percent of a building's slots that are lit. The calibration knob: the
        // right crowd depends on the panel, on the cell size and on taste. It is
        // a percent of the slots a building's STYLE allows, which is why 88 here
        // does not make a band 88% dense — a wide-pitch or banded style has
        // already dropped half its rectangle before this applies. Fitted against
        // the reference's skyline bands.
        let fill = env_num(&["CITY_WINDOW_PCT"], 88, 0, 100) as u32;
        // Window FLIPS PER SECOND — a rate, not a divisor, so the next person
        // does not have to reverse it out of the frame rate. 40/s over the 2488
        // slots a 1920x1080 panel generates turns a given window over about
        // once a minute, which is the calm barely-shimmering scene the
        // reference is. 150/s read as a busy one.
        let win_per_sec = env_num(&["CITY_TWINKLE"], 40, 0, 100_000) as u32;
        // Sky re-shades per second. Slower and subtler by design, and this
        // ratio — a little over 3:1 against the windows — is the only place
        // that is expressed.
        let sky_per_sec = env_num(&["CITY_SKY_TWINKLE"], 12, 0, 100_000) as u32;

        let fps = fps.max(1);
        let base = (rows * BASE_PCT / 100).max(1);
        let sky_end = rows * SKY_PCT / 100;

        let mut rng = 0x0c17_51de;
        let mut stars = Vec::new();
        for cy in 0..sky_end {
            let d = BAND_DENSITY[cy * BANDS / rows];
            for cx in 0..cols {
                if next_rand(&mut rng) % 100 >= d {
                    continue;
                }
                let i = cy * cols + cx;
                grid.set(i, Cell::new(STAR_GLYPH, pick(&mut rng, &SKY_DRAW)));
                stars.push(i as u32);
            }
        }

        // Buildings, left to right on one baseline. Each writes its WHOLE
        // rectangle — blanks included — so a nearer building punches the sky
        // and the building behind it out of its own footprint.
        let mut windows = Vec::new();
        let mut slot: Vec<Slot> = Vec::new();
        // cell -> its entry in `windows`, so a cell an overlapping building
        // redraws keeps ONE slot carrying the FRONT building's style. Without
        // it the building behind keeps a stale entry pointing at the same cell,
        // and the twinkle speckles the overlap with the wrong style at twice
        // the rate. Scratch: dropped at the end of `new`.
        let mut owner = vec![u32::MAX; cols * rows];
        let mut x = 0usize;
        while x < cols {
            let class = ROOF_DRAW[next_rand(&mut rng) as usize % ROOF_DRAW.len()] as usize;
            let (lo, hi) = ROOF_CLASS[class];
            // Taller class, narrower span: see W_SPAN.
            let w = W_MIN + next_rand(&mut rng) as usize % (W_SPAN - class).max(2);
            let (top_lo, top_span) = (rows * lo / 100, (rows * (hi - lo) / 100).max(1));
            let top = (top_lo + next_rand(&mut rng) as usize % top_span).min(base - 1);
            let st = &STYLES[STYLE_DRAW[next_rand(&mut rng) as usize % STYLE_DRAW.len()] as usize];
            let lit_pct = (fill * st.fill_pct / 100).min(100);
            for cy in top..base {
                let dark_floor =
                    st.band_every > 0 && (base - 1 - cy) % st.band_every == st.band_every - 1;
                let p = if cy + STREET_ROWS >= base {
                    lit_pct / 2
                } else {
                    lit_pct
                };
                for cx in x..(x + w).min(cols) {
                    let i = cy * cols + cx;
                    // A cell the style does not light is written CLEAR — the
                    // building still has to punch its whole rectangle through
                    // whatever is behind it — and is NOT a window slot, so the
                    // twinkle can never light it and dissolve the style.
                    if dark_floor || !(cx - x).is_multiple_of(st.col_pitch) {
                        grid.set(i, Cell::CLEAR);
                        // The cell is dark in the style that owns it NOW, so it
                        // must stop being a slot of whatever was behind it.
                        if owner[i] != u32::MAX {
                            slot[owner[i] as usize].lit_pct = 0;
                        }
                        continue;
                    }
                    let c = if next_rand(&mut rng) % 100 < p {
                        Cell::new(st.glyph, pick(&mut rng, &WINDOW_DRAW))
                    } else {
                        Cell::CLEAR
                    };
                    grid.set(i, c);
                    let mine = Slot {
                        glyph: st.glyph,
                        lit_pct: p as u8,
                    };
                    if owner[i] == u32::MAX {
                        owner[i] = windows.len() as u32;
                        windows.push(i as u32);
                        slot.push(mine);
                    } else {
                        slot[owner[i] as usize] = mine;
                    }
                }
            }
            let overlap = next_rand(&mut rng) as usize % OVERLAP_MAX;
            x += w.saturating_sub(overlap).max(1);
        }

        // Sky lights a building was drawn over are gone from the panel; leaving
        // them in `stars` would let the twinkle repaint one inside a silhouette,
        // in the sky family, over a window. The test on the STAR glyph, not on
        // "not blank": a building writes windows as well as blanks.
        stars.retain(|&i| grid.cell(i as usize).glyph() == STAR_GLYPH as usize);

        let (win_rate, sky_rate) = ((win_per_sec << 8) / fps, (sky_per_sec << 8) / fps);
        Self {
            grid,
            windows,
            slot,
            stars,
            // Generous: `due` can hand back one extra when the accumulator
            // rolls over, and a push past capacity would allocate in `render`.
            dirty: Vec::with_capacity(((win_rate + sky_rate) >> 8) as usize + 4),
            win_rate,
            sky_rate,
            win_acc: 0,
            sky_acc: 0,
            rng,
        }
    }
}

impl Saver for City {
    fn render(&mut self, s: &mut Surface<'_>) {
        self.dirty.clear();

        if !self.windows.is_empty() {
            for _ in 0..due(&mut self.win_acc, self.win_rate) {
                let k = next_rand(&mut self.rng) as usize % self.windows.len();
                let i = self.windows[k] as usize;
                let spec = self.slot[k];
                // Re-DRAW at the slot's own odds rather than toggling: a toggle
                // walks every building toward 50% lit whatever it was generated
                // at. Its own glyph, too — relighting a curtain-wall cell as a
                // grid window dissolves the building into its neighbour one
                // twinkle at a time.
                let c = if next_rand(&mut self.rng) % 100 < spec.lit_pct as u32 {
                    Cell::new(spec.glyph, pick(&mut self.rng, &WINDOW_DRAW))
                } else {
                    Cell::CLEAR
                };
                // About one draw in six lands on what is already there — an
                // unlit cell redrawn unlit, or a lit one redrawn in the same
                // colour. Reporting those would be honest but pointless damage.
                if c == self.grid.cell(i) {
                    continue;
                }
                self.grid.set(i, c);
                self.dirty.push(i as u32);
            }
        }

        if !self.stars.is_empty() {
            for _ in 0..due(&mut self.sky_acc, self.sky_rate) {
                let i = self.stars[next_rand(&mut self.rng) as usize % self.stars.len()];
                self.grid.set(
                    i as usize,
                    Cell::new(STAR_GLYPH, pick(&mut self.rng, &SKY_DRAW)),
                );
                self.dirty.push(i);
            }
        }

        // Row-major order, so two cells in the same row collapse into one damage
        // run rather than two — `Damage::mark` merges only into the LAST run.
        // At the shipped 40 flips/s this is a measured no-op: a frame draws 1.7
        // cells and the runs and rows are identical with the sort removed. It
        // starts paying at roughly 400/s, and it is free insurance for whoever
        // raises `CITY_TWINKLE` rather than a claim about today.
        self.dirty.sort_unstable();
        self.grid.flush_sparse(s, &PAL, &self.dirty);
    }

    fn name(&self) -> &'static str {
        "city"
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
        Panel::new(1920, 1080, 1920)
    }

    fn city() -> City {
        City::new(&panel(), 30)
    }

    /// Lit cells per tenth of the grid, in percent. Shared because the same
    /// measurement has to hold at generation AND after a long run — the second
    /// is the only check that the twinkle reproduces the city it was given.
    fn bands(c: &City) -> Vec<usize> {
        let (cols, rows) = (c.grid.cols(), c.grid.rows());
        let mut lit = [0usize; BANDS];
        let mut total = [0usize; BANDS];
        for cy in 0..rows {
            for cx in 0..cols {
                let b = cy * BANDS / rows;
                total[b] += 1;
                lit[b] += usize::from(c.grid.cell(cy * cols + cx).glyph() != font::BLANK as usize);
            }
        }
        (0..BANDS).map(|b| lit[b] * 100 / total[b]).collect()
    }

    /// Both glyphs have to exist and have to be non-blank, or a light is drawn
    /// as nothing at all and every other test here still passes.
    #[test]
    fn every_glyph_the_scene_uses_is_drawn() {
        for g in [WINDOW_GLYPH, STAR_GLYPH] {
            assert_ne!(g, font::BLANK, "a light must not be a blank glyph");
            assert!(
                font::GLYPHS[g as usize].iter().any(|&r| r != 0),
                "glyph {g} has no lit pixels"
            );
        }
        // And they must differ, or the sky and the skyline are one texture.
        assert_ne!(WINDOW_GLYPH, STAR_GLYPH);
        // The window must not fill its cell, or adjacent windows merge into a
        // filled block and the silhouette stops being made of windows.
        assert!(font::GLYPHS[WINDOW_GLYPH as usize]
            .iter()
            .any(|&r| r != 0xFF));
    }

    /// The colour split is the look: sky neutral (`g == r`) with a blue shift,
    /// windows cyan (`g > r`) and hotter than any sky light.
    #[test]
    fn the_two_ramps_are_different_colour_families() {
        let lum = |c: [u8; 3]| c[0] as u32 + c[1] as u32 + c[2] as u32;
        for i in SKY_LO..=SKY_HI {
            let c = PAL_RGB[i as usize];
            assert_eq!(c[0], c[1], "sky {i} is not neutral");
            assert!(c[2] > c[0], "sky {i} has no blue shift");
        }
        for i in WIN_LO..=WIN_HI {
            let c = PAL_RGB[i as usize];
            assert!(c[1] > c[0], "window {i} is not cyan");
            assert!(c[2] >= c[1], "window {i} is not cyan");
        }
        let hottest_sky = (SKY_LO..=SKY_HI)
            .map(|i| lum(PAL_RGB[i as usize]))
            .max()
            .unwrap();
        let hottest_win = (WIN_LO..=WIN_HI)
            .map(|i| lum(PAL_RGB[i as usize]))
            .max()
            .unwrap();
        assert!(
            hottest_win > hottest_sky,
            "windows must outshine the sky ({hottest_win} vs {hottest_sky})"
        );

        // And what is ON SCREEN has to come from the right one. A sky cell
        // holding a window index is the bug this catches, and no palette
        // assertion above would see it.
        let c = city();
        let cols = c.grid.cols();
        for &i in &c.stars {
            let col = c.grid.cell(i as usize).colour() as u16;
            assert!((SKY_LO..=SKY_HI).contains(&col), "sky cell {i} is {col}");
        }
        for &i in &c.windows {
            let cell = c.grid.cell(i as usize);
            if cell.glyph() == font::BLANK as usize {
                continue;
            }
            let col = cell.colour() as u16;
            assert!((WIN_LO..=WIN_HI).contains(&col), "window cell {i} is {col}");
            assert!(
                i as usize / cols >= c.grid.rows() * ROOF_CLASS[ROOF_CLASS.len() - 1].0 / 100,
                "a lit window above the tallest roof a spire can reach"
            );
        }
    }

    /// The layout measurements: an empty top edge, an empty bottom edge, sky
    /// thinning toward the top, and a skyline denser than the sky above it.
    ///
    /// These are ORDERING assertions, not a fit. `BAND_DENSITY` was read off one
    /// compressed screenshot whose anti-aliasing halos count as lit pixels, so
    /// it is a guide to the shape of the frame and not a target to three
    /// significant figures. Where the art and the table disagree the art wins —
    /// the roof classes deliberately push band 5 to about 28% against the
    /// table's 20% to buy a skyline with real towers in it. Pinning the numbers
    /// here would have made that change a test failure instead of an
    /// improvement, which is the wrong way round.
    #[test]
    fn the_scene_matches_the_bands_it_was_measured_from() {
        let c = city();
        let (cols, rows) = (c.grid.cols(), c.grid.rows());
        let pct = bands(&c);

        assert_eq!(pct[0], 0, "the top edge is empty: {pct:?}");
        assert_eq!(pct[BANDS - 1], 0, "the bottom edge is empty: {pct:?}");
        assert!(pct[1] < pct[2], "the sky must thin toward the top: {pct:?}");
        assert!(
            pct[7] > pct[4] && pct[7] > pct[5],
            "the skyline must be denser than the sky: {pct:?}"
        );
        // Street level, measured on the rows `STREET_ROWS` actually covers.
        // Band 8 will not do: it takes in an empty row below the baseline that
        // dilutes it by about 14% whatever the generator does, so
        // `pct[8] < pct[7]` passed with the half-rate deleted entirely.
        let base = rows * BASE_PCT / 100;
        let density = |r0: usize, r1: usize| {
            let n: usize = (r0..r1)
                .map(|cy| {
                    (0..cols)
                        .filter(|&cx| c.grid.cell(cy * cols + cx).glyph() != font::BLANK as usize)
                        .count()
                })
                .sum();
            n * 100 / (cols * (r1 - r0))
        };
        let street = density(base - STREET_ROWS, base);
        let above = density(base - 2 * STREET_ROWS, base - STREET_ROWS);
        assert!(
            street * 4 < above * 3,
            "street level {street}% is not darker than the floors above it ({above}%)"
        );
        // The sky bands stay inside a loose envelope of the measurement, so a
        // generator that stopped drawing sky entirely, or filled it, still
        // fails — without pinning any band to its exact reference value.
        for b in 1..=4 {
            let r = BAND_DENSITY[b] as usize;
            assert!(
                pct[b] * 2 > r && pct[b] < r * 2,
                "sky band {b} is {} against a reference {r}: {pct:?}",
                pct[b]
            );
        }
    }

    /// The roofline. A skyline is low blocks, mid-rise and a handful of towers
    /// standing clear of them; a uniform band of near-equal roofs is the single
    /// most visible way to get this wrong, and it passes every density check
    /// above. So this pins the SPREAD, which is the thing the eye actually
    /// reads.
    #[test]
    fn the_roofline_has_towers_and_not_one_flat_band() {
        let c = city();
        let (cols, rows) = (c.grid.cols(), c.grid.rows());
        let base = rows * BASE_PCT / 100;

        // The highest window SLOT in each column is that column's roof.
        let mut roof = vec![usize::MAX; cols];
        for &i in &c.windows {
            let (cx, cy) = (i as usize % cols, i as usize / cols);
            roof[cx] = roof[cx].min(cy);
        }
        // Columns a wide-pitch style left with no slots have no roof to read.
        let roof: Vec<usize> = roof.into_iter().filter(|&r| r != usize::MAX).collect();
        let mut sorted = roof.clone();
        sorted.sort_unstable();
        let (highest, lowest) = (sorted[0], sorted[sorted.len() - 1]);
        let median = sorted[sorted.len() / 2];

        // A fifth of the panel between the tallest roof and the shortest. The
        // first cut of this saver managed 9 rows of 67 and read as a flat band.
        assert!(
            lowest - highest >= rows / 5,
            "roofline spans only {} rows of {rows}: {highest}..{lowest}",
            lowest - highest
        );
        // And the tall ones must stand CLEAR of the crowd, not just be the top
        // of one smooth spread: the tallest is a third of the building height
        // above the median roof.
        assert!(
            median - highest >= (base - median) / 3,
            "the tallest roof {highest} does not stand clear of the median {median}"
        );
        // Towers are a minority. If most columns were at spire height the
        // skyline would be a wall, and the spread assertion alone would pass.
        let tall = roof
            .iter()
            .filter(|&&r| r < median - (base - median) / 4)
            .count();
        assert!(
            (1..cols / 3).contains(&tall),
            "{tall} of {cols} columns are towers"
        );
    }

    /// Buildings sit on ONE baseline. Independent bottoms would look like a
    /// bar chart and would still pass every density check above.
    #[test]
    fn every_building_shares_the_baseline() {
        let c = city();
        let (cols, rows) = (c.grid.cols(), c.grid.rows());
        let base = rows * BASE_PCT / 100;
        // Every column is built on, and its lowest slot is the baseline row.
        // A wide-pitch style leaves whole columns with no slots at all, so the
        // claim is about columns that ARE built on, not about every column.
        let mut lowest = vec![0usize; cols];
        for &i in &c.windows {
            let (cx, cy) = (i as usize % cols, i as usize / cols);
            lowest[cx] = lowest[cx].max(cy + 1);
        }
        let built: Vec<usize> = lowest.into_iter().filter(|&l| l > 0).collect();
        assert!(
            built.len() > cols / 2,
            "only {} columns built on",
            built.len()
        );
        assert!(
            built.iter().all(|&l| l == base),
            "columns do not share a baseline: {:?}",
            &built[..8.min(built.len())]
        );
    }

    /// Styles have to actually reach the panel. All the const assertions in the
    /// world do not prove the generator ever picks a second style, and a
    /// skyline in one style is the "buildings blend into one wall" bug — which
    /// every density, roofline and damage test here passes happily.
    #[test]
    fn adjacent_buildings_are_lit_differently() {
        let c = city();
        let cols = c.grid.cols();

        let mut glyphs: Vec<u16> = c.slot.iter().map(|s| s.glyph).collect();
        glyphs.sort_unstable();
        glyphs.dedup();
        assert!(
            glyphs.len() > 1,
            "the whole skyline uses one window glyph: {glyphs:?}"
        );

        // Wide-pitch styles leave dark gutter columns inside a building, which
        // is the cheapest evidence that a pitch other than 1 was drawn.
        let mut has_slot = vec![false; cols];
        for &i in &c.windows {
            has_slot[i as usize % cols] = true;
        }
        let gutters = has_slot.iter().filter(|&&b| !b).count();
        assert!(gutters > 0, "no building uses a wide column pitch");

        // And a dark service floor: a row with no slots, between two rows that
        // have them, in the same column.
        let mut floors = 0;
        for cx in 0..cols {
            let rows_here: Vec<usize> = c
                .windows
                .iter()
                .filter(|&&i| i as usize % cols == cx)
                .map(|&i| i as usize / cols)
                .collect();
            if let (Some(&lo), Some(&hi)) = (rows_here.iter().min(), rows_here.iter().max()) {
                floors += usize::from(hi - lo + 1 > rows_here.len());
            }
        }
        assert!(floors > 0, "no building has a dark service floor");

        // And the style must SURVIVE the twinkle. Relighting a slot from a
        // global glyph rather than its own dissolves a curtain wall into its
        // neighbour's grid one cell at a time, over minutes, and every other
        // test here passes throughout.
        let p = panel();
        let mut c = city();
        let mut buf = vec![0u32; p.buf_len()];
        let before: Vec<Cell> = c.windows.iter().map(|&i| c.grid.cell(i as usize)).collect();
        for _ in 0..6000 {
            saver::frame(&mut c, &mut buf, &p);
        }
        let mut redrawn = 0;
        for (k, &i) in c.windows.iter().enumerate() {
            let cell = c.grid.cell(i as usize);
            redrawn += usize::from(cell != before[k]);
            assert!(
                cell.glyph() == font::BLANK as usize || cell.glyph() == c.slot[k].glyph as usize,
                "slot {k} holds glyph {} but its building lights {}",
                cell.glyph(),
                c.slot[k].glyph
            );
        }
        // Non-vacuous: the check is worthless if nothing was ever redrawn.
        assert!(
            redrawn > c.windows.len() / 20,
            "only {redrawn} of {} slots were redrawn in 6000 frames",
            c.windows.len()
        );
    }

    /// The "screen went blank" bug class: cells written but never reported.
    ///
    /// This CANNOT be checked against the framebuffer, and the first version of
    /// this test tried. A write that is never reported is never blitted, so the
    /// framebuffer never changes, so `prev == buf` skips the scanline and there
    /// is nothing to catch — deleting `self.dirty.push` from the sky loop
    /// survived the whole suite. A framebuffer diff can only find OVER-writing.
    ///
    /// So the check is against the grid's own two buffers. `flush_sparse` copies
    /// `cur[i]` into `prev[i]` for exactly the indices in `dirty`, so after any
    /// frame `cell(i) != cells()[i]` means cell `i` was written and left out —
    /// which on simpledrm is a region of the panel frozen forever.
    #[test]
    fn damage_covers_every_changed_scanline() {
        let p = panel();
        let mut c = city();
        let mut buf = vec![0u32; p.buf_len()];
        let mut prev = buf.clone();

        let d = saver::frame(&mut c, &mut buf, &p);
        // The whole panel, strip included — see grid.rs `paint_margins`.
        assert_eq!(d.rows(), p.h, "frame 0 must paint the whole panel");
        // And it must actually be the SCENE, not a black panel that happens to
        // be reported: a vacuous frame-0 assertion is worth nothing.
        assert!(
            buf.iter().filter(|&&px| px != 0).count() > 10_000,
            "frame 0 painted nothing"
        );

        let cells = c.grid.cols() * c.grid.rows();
        for i in 0..cells {
            assert_eq!(
                c.grid.cell(i),
                c.grid.cells()[i],
                "frame 0 left cell {i} stale"
            );
        }

        let mut rows = Vec::new();
        let mut moved = 0;
        for n in 1..200 {
            prev.copy_from_slice(&buf);
            let d = saver::frame(&mut c, &mut buf, &p);
            let mut changed = 0;
            for y in 0..p.h {
                if prev[y * p.w..][..p.w] == buf[y * p.w..][..p.w] {
                    continue;
                }
                changed += 1;
                assert!(
                    d.runs()
                        .iter()
                        .any(|&(a, b)| y as u16 >= a && (y as u16) < b),
                    "frame {n}: scanline {y} changed but was not reported"
                );
            }
            moved += usize::from(changed > 0);
            rows.push(d.rows());

            // The real one. Every cell the saver wrote has to be in `dirty`.
            for i in 0..cells {
                assert_eq!(
                    c.grid.cell(i),
                    c.grid.cells()[i],
                    "frame {n}: cell {i} was written but not reported"
                );
            }
        }
        // Over the window, not per frame: at the default rate a frame draws
        // about 1.3 windows, and a re-draw that lands on what is already there
        // is skipped, so an idle frame is legitimate. It is a STALLED twinkle
        // that is the bug, and only the window tells the two apart.
        assert!(moved > 60, "the twinkle stalled ({moved}/199 frames moved)");

        // The whole point of the saver. A regression to a full-screen repaint
        // is the thing this catches, and it is why the bound is a fraction of
        // the grid rather than "less than the panel".
        rows.sort_unstable();
        let median = rows[rows.len() / 2];
        let all = c.grid.rows() * c.grid.cell_h();
        assert!(
            median * 5 < all,
            "median damage {median} of {all} scanlines: the scene is not static"
        );
    }

    /// CLAUDE.md makes the frame loop the top constraint in this repo, and it
    /// had no test at all: changing `dirty`'s reserve to `with_capacity(0)`
    /// made `render` allocate every single frame and nothing failed.
    ///
    /// Capacity, not length: a `Vec` that never grows past the capacity `new`
    /// reserved never reallocates, and `clear` keeps capacity. So an unchanged
    /// capacity after a long run IS "render did not allocate".
    #[test]
    fn render_never_allocates() {
        let p = panel();
        let mut c = city();
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
        // Non-vacuous: a reserve nothing ever fills proves nothing either.
        assert!(worst > 0, "`dirty` was never used");
        assert!(
            worst <= reserved,
            "{worst} entries in a reserve of {reserved}"
        );
    }

    /// The scene does not decay. Over a long run every band has to come back to
    /// the density it was generated at — a twinkle with any fixed point of its
    /// own rewrites the city into a different one, slowly, while every damage
    /// and layout test here keeps passing.
    #[test]
    fn the_scene_is_still_and_does_not_drain() {
        let p = panel();
        let mut c = city();
        let mut buf = vec![0u32; p.buf_len()];

        // PER BAND, not one global lit count. A global count with a +/-33%
        // envelope let the toggle bug through at 0.694 against a 0.75
        // threshold, and it was diluted further because the sky — which cannot
        // drain, its cells are only ever re-shaded — is in the same total.
        // Per band, a drained skyline is unmissable: band 7 goes 62 -> ~50.
        let early = bands(&c);
        for _ in 0..300_000 {
            saver::frame(&mut c, &mut buf, &p);
        }
        let late = bands(&c);
        for b in 0..BANDS {
            assert!(
                late[b].abs_diff(early[b]) <= 2,
                "band {b} drifted {} -> {} over 300k frames: {early:?} -> {late:?}",
                early[b],
                late[b]
            );
        }
        // Non-vacuous: 300k frames must actually have redrawn most of the city,
        // or "no drift" is just "nothing happened".
        assert!(
            c.windows.len() < 300_000 / 30,
            "{} slots against 300k frames is too few draws to prove anything",
            c.windows.len()
        );
    }
}
