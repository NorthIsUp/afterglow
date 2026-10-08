//! City — the After Dark night skyline: a band of lit windows along the bottom,
//! scattered lights in the sky above, pure black everywhere else.
//!
//! # What this is a copy of
//!
//! The palette and the layout are sampled off a reference frame, not invented.
//! What the look actually IS — the colour families, the window styles and
//! silhouettes that keep neighbouring buildings apart, the four roof classes,
//! the band densities and how far this drifts from them on purpose — is written
//! up once, in `docs/savers/city.md`.
//! It is not repeated here: the two copies had already drifted from each other
//! and from the measurement inside a single commit.
//!
//! What a reader of THIS file needs, and cannot get from that page:
//!
//! * `STYLES`, `SHAPE_DRAW`, `STAR_KIND` and `ROOF_CLASS` are the art. All are
//!   weighted draws, all are validated in a `const` block, and a bad entry is a
//!   build failure.
//! * A window is RE-DRAWN at its own slot's odds, never toggled. A toggle has
//!   a fixed point at half lit, so a city generated at 88% fades to 50% over a
//!   few minutes with every test still passing. Re-drawing is memoryless in one
//!   step, so the stationary distribution is exactly the generating one.
//! * `Slot` carries the glyph, the lit percentage AND the brightness profile
//!   per cell because a cell two buildings overlap belongs to exactly one of
//!   them, and the twinkle has to reproduce that one. The same argument puts a
//!   star's shape and brightness tier in `StarKind`: a light that changes shape
//!   or jumps brightness class when it twinkles reads as flicker, not as a star.
//! * The profile is ONE index into `DRAWS`, covering colour family and
//!   brightness together. They were a `bool` and a global table, and separating
//!   them is what let a bright building relight off the district average.
//! * The window GLYPHS are most of what keeps neighbours apart, and the reason
//!   is in `SLOT_GLYPH`'s doc: two buildings whose windows are the same square
//!   read as one texture at two phases however differently they are clocked, so
//!   the shapes have to span proportion and not just pitch.
//! * Nothing translates EXCEPT the shooting star, which is why it is the only
//!   thing here that saves what it covered and puts it back.
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
//! Damage is a couple of short runs. The measured figures live in
//! `docs/savers/city.md`; the invariant that holds
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
/// 1..=6 is the SKY ramp, 7..=12 the WINDOW ramp, 13..=15 the WARM ramp and 16
/// the beacon, and the split between them is the whole look — see the module
/// doc. They are separate ranges rather than one ordered ramp because a cell
/// must never be able to borrow another family's colour by drifting one index.
#[rustfmt::skip]
const PAL_RGB: [[u8; 3]; 17] = [
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
    [0xF8, 0xE0, 0xA8], // 13 warm, hottest — a lamp left on, not a floodlight
    [0xD0, 0xB0, 0x70], // 14
    [0xA0, 0x80, 0x48], // 15 warm, dimmest
    [0xE0, 0x18, 0x18], // 16 the beacon, and the only red in the scene
];
const PAL: [u32; 17] = bake(&PAL_RGB);

const SKY_LO: u16 = 1;
const SKY_HI: u16 = 6;
const WIN_LO: u16 = 7;
const WIN_HI: u16 = 12;
/// The warm ramp is the top of the window ramp turned over: `r > g > b` where a
/// window is `b >= g > r`, at roughly the same luminances. Mirroring the
/// brightness rather than adding a hot orange is what keeps a warm window
/// reading as the same room under a different bulb instead of as a different
/// kind of light — and it is why 1% of them is enough to notice.
const WARM_LO: u16 = 13;
const WARM_HI: u16 = 15;
/// Saturated rather than bright: at one cell the beacon has to be unmistakably
/// NOT a window, and hue carries that where luminance cannot. Deliberately
/// dimmer than the hottest window, so the one red thing on the panel never
/// becomes the brightest thing on it.
const BEACON_COL: u16 = 16;

/// Warm windows per THOUSAND slots, chosen EXACTLY rather than rolled (see
/// `new`). The ask is "up to 1%" and this leaves room under it: a slot is lit
/// about two thirds of the time, so 8 per thousand slots is about 0.7% of the
/// windows alight at any moment — a dozen on a 1080p panel, which reads as
/// somebody working late. Push it to 50 and it stops being an accident and
/// starts being a colour scheme.
const WARM_PER_MILLE: u32 = 8;

/// Whole WARM BUILDINGS, per thousand buildings — a hotel or an apartment block
/// where every room is on the same yellow bulb, against an office district that
/// is entirely cold. The scattered accent above is one lamp left on in someone
/// else's tower; this is a building of a different KIND, and it is the only
/// thing here that changes the colour of a whole silhouette.
///
/// The rate makes it likely, `WARM_BLDG_MAX` makes it bounded. Rate alone does
/// not: a district is ~27 buildings, so a 9% roll lands four of them about one
/// generation in eight, and four warm towers is not a cold skyline with warm
/// exceptions in it — it is a two-colour skyline. Same argument as
/// `WARM_PER_MILLE`: the ceiling has to be a guarantee and not an average.
const WARM_BLDG_PER_MILLE: u32 = 90;
const WARM_BLDG_MAX: u32 = 2;

/// Weighted draws over the ramps, so brightness varies without every light
/// being equally bright — a uniform draw reads as a flat wash of one grey. All
/// are skewed dim: a sky of mostly hot dots looks like static, and a skyline of
/// mostly hot windows looks like daylight.
#[rustfmt::skip]
const WINDOW_DRAW: [u16; 16] = [7, 8, 8, 9, 9, 9, 10, 10, 10, 10, 11, 11, 11, 12, 12, 12];
/// An office floor still at it: every room on the same circuit, so the spread
/// is narrow and it sits at the hot end. A whole building in this reads as one
/// slab of light where its neighbour reads as scattered rooms — which is the
/// point. It is the DISTRIBUTION that differs, not one window.
#[rustfmt::skip]
const BRIGHT_DRAW: [u16; 16] = [7, 7, 7, 7, 8, 8, 8, 8, 8, 8, 9, 9, 9, 9, 7, 8];
/// The other end: a block where almost nothing is on, and what is on is barely
/// on. Pairs with a low `fill_pct` to make a building that is present in the
/// silhouette and nearly absent in the light.
#[rustfmt::skip]
const DIM_DRAW: [u16; 16] = [10, 11, 11, 11, 11, 12, 12, 12, 12, 12, 12, 12, 11, 12, 12, 11];
/// Same shape as `WINDOW_DRAW` over the warm ramp: mostly the two dim entries,
/// the hot one rare.
#[rustfmt::skip]
const WARM_DRAW: [u16; 16] = [13, 13, 14, 14, 14, 14, 14, 15, 15, 15, 15, 15, 15, 14, 15, 14];

/// Every brightness profile a slot can relight from, indexed by `Slot::draw`.
/// One table rather than a `warm` flag and a branch because the thing that has
/// to be stable per slot is "which distribution does this room come back at",
/// and colour family is only one axis of that: a bright building that relit
/// from the mixed ramp would dissolve into its neighbours exactly the way a
/// warm window that relit white flickers.
const DRAWS: [[u16; 16]; 4] = [WINDOW_DRAW, BRIGHT_DRAW, DIM_DRAW, WARM_DRAW];
/// The warm entry, which is the only one outside the window ramp — so it is the
/// only one a STYLE may not name, and the one the accent and the warm buildings
/// both point at.
const WARM_IX: u8 = 3;

/// The window SHAPES, and the reason there is more than one of them.
///
/// Pitch, dark floors and fill rate all vary WHICH cells are lit. They do not
/// vary what a window is, and two buildings whose windows are the same 6x4
/// square read as one texture at two phases however differently they are
/// clocked — which is the note this saver kept coming back on. Real skylines
/// differ far more by window PROPORTION than by grid pitch, so the shapes below
/// span it: a punched square, a floor-to-ceiling slot half as wide as it is
/// tall, a pane twice as wide as it is tall, two small windows per cell, and a
/// band that runs into its neighbour.
///
/// `font::BRAILLE` is this repo's own filled 2x4 quadrants rather than reading
/// pips (see the font module doc), so indexing it by a quadrant mask is a
/// sub-cell rectangle and nothing new has to be added to the atlas. The bits
/// are the braille dot numbers: `0x01/0x02/0x04/0x40` are the left column top
/// to bottom, `0x08/0x10/0x20/0x80` the right.
const fn quad(mask: u8) -> u16 {
    font::BRAILLE[mask as usize]
}

/// A lit window: a small square with dark margin all round, so a run of lit
/// cells reads as separate windows rather than as one filled block. That is the
/// difference between a skyline and a black rectangle.
const WINDOW_GLYPH: u16 = font::BLOCK;
/// Floor-to-ceiling glazing: 6x12 of the cell's 12x16, so the window is twice
/// as tall as it is wide where `WINDOW_GLYPH` is half as tall as it is wide.
/// Same grid, opposite proportion — the loudest shape difference available.
const SLOT_GLYPH: u16 = quad(0x01 | 0x02 | 0x04);
/// The other extreme: 12x8, a big pane wider than it is tall. Drawn at a wide
/// column pitch so it stands alone rather than running into the cell beside it.
const PANE_GLYPH: u16 = quad(0x02 | 0x04 | 0x10 | 0x20);
/// Two small windows stacked in ONE cell, so this building has twice as many
/// storeys as its neighbour on the same grid. Floor HEIGHT is the difference
/// nothing else here could express — an apartment block beside an office slab.
const TWIN_GLYPH: u16 = quad(0x01 | 0x04);
/// A full-width band that meets the band in the next cell, so a lit row draws
/// one unbroken spandrel line. Where `RIBBON_GLYPH` is two thin rules, this is
/// a solid course — the difference between a drawn line and a lit floor.
const BAND_GLYPH: u16 = quad(0x02 | 0x10);
/// A curtain-wall strip: one narrow bar the full height of the cell, so a tower
/// in this style has unbroken vertical lines where its neighbour has a grid of
/// dots. It is the loudest of the style differences and the one that does most
/// of the work at a glance.
const STRIP_GLYPH: u16 = font::ASCII[(b'|' - 0x20) as usize];
/// Ribbon glazing: two horizontal bars, so the building reads as banded floors
/// rather than as a grid of rooms — the horizontal answer to the curtain wall.
const RIBBON_GLYPH: u16 = font::ASCII[(b'=' - 0x20) as usize];

/// Sky lights are smaller and lower in their cell than a window, which is half
/// of why the two families read apart at a glance; the other half is colour.
/// Four shapes, not one: a field of one repeated glyph reads as a texture, and
/// the shapes give it depth.
const DOT_GLYPH: u16 = font::ASCII[(b'.' - 0x20) as usize];
/// The fire ramp's dot — taller than ASCII '.', so it reads as a nearer star.
const BIGDOT_GLYPH: u16 = font::RAMP[1];
const SPARK_GLYPH: u16 = font::ASCII[(b'*' - 0x20) as usize];
const CROSS_GLYPH: u16 = font::ASCII[(b'+' - 0x20) as usize];

/// A star's kind: its shape and the slice of the sky ramp it is allowed to be.
/// The pairing is the point — the dim end of the ramp gets the small dots and
/// the bright end gets the sparkles, so brightness and size agree the way they
/// do in a photograph of a sky.
///
/// Both halves are FIXED per star. A twinkle re-shades within the kind's own
/// range; it never changes the shape and never crosses into another kind's
/// brightness. A star that changed shape reads as noise, and one that jumped
/// from `#686878` to `#B8B8C8` reads as a strobe.
struct StarKind {
    glyph: u16,
    lo: u16,
    hi: u16,
}

#[rustfmt::skip]
const STAR_KIND: [StarKind; 4] = [
    StarKind { glyph: DOT_GLYPH,    lo: 1, hi: 2 }, // far and faint: most of the field
    StarKind { glyph: BIGDOT_GLYPH, lo: 3, hi: 4 },
    StarKind { glyph: SPARK_GLYPH,  lo: 5, hi: 5 },
    StarKind { glyph: CROSS_GLYPH,  lo: 6, hi: 6 }, // the handful of bright ones
];
/// Weighted the way the old flat `SKY_DRAW` was weighted: mostly dim.
#[rustfmt::skip]
const STAR_DRAW: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 2, 3];

/// The shooting star's tail is drawn with the SLASH that matches its direction,
/// so the cells join up into one line. Dots do not: a tail of dots in a sky
/// made of dots is five more stars, which is exactly how the first cut of this
/// read — the streak was invisible in a still frame and only the motion gave it
/// away.
const BACKSLASH_GLYPH: u16 = font::ASCII[(b'\\' - 0x20) as usize];
const SLASH_GLYPH: u16 = font::ASCII[(b'/' - 0x20) as usize];
/// Head first, fading back. Five cells is enough to read as a streak at 30fps
/// and short enough that the frame it fires on stays a rounding error against
/// the frame budget.
#[rustfmt::skip]
const STREAK_RAMP: [u16; 5] = [7, 7, 8, 9, 10];
const STREAK_LEN: usize = STREAK_RAMP.len();
/// How many cells the streak travels before it starts draining. About 0.8s at
/// 30fps, which is what a real one looks like.
const STREAK_STEPS: u8 = 24;

/// How one building lights its rectangle, assigned once at generation and never
/// changed — a building that restyled as it twinkled would read as a glitch.
///
/// The point is the SEAM. Two adjacent buildings lit the same way are one wall
/// of lights with no edge in it, and the eye cannot tell where one ends. Give
/// them different pitches, different dark floors and different glyphs and the
/// join draws itself, with nothing painting a border.
struct Style {
    glyph: u16,
    /// Light only every `col_pitch`-th column, counted from the BUILDING's own
    /// left edge — so the phase differs between neighbours as well as the
    /// pitch, and a setback or a podium does not shift the columns under it.
    col_pitch: usize,
    /// Light only every `row_pitch`-th floor, counted UP from the baseline. 1
    /// for every floor. Where `band_every` punches a single dark service floor
    /// out of a full building, this is a building whose floors are simply
    /// further apart — a different reading of the same silhouette.
    row_pitch: usize,
    /// Every `band_every`-th floor is dark plant and service, counted UP from
    /// the baseline so the bands line up with the ground and not with the roof.
    /// 0 for none.
    band_every: usize,
    /// Light every other cell of the grid instead of all of them, which at this
    /// size reads as a diagonal weave rather than as a grid with gaps.
    checker: bool,
    /// Percent of the building's height, measured up from the baseline, whose
    /// windows light at a FIFTH of the style's rate: the tower whose offices
    /// have gone home downstairs and not upstairs. A fifth rather than zero
    /// because a building with a black bottom half does not read as dark, it
    /// reads as floating.
    dim_below: u32,
    /// Fill as a percent OF `CITY_WINDOW_PCT`, so one knob still scales the
    /// whole skyline while a style can be the dark one.
    fill_pct: u32,
    /// Which `DRAWS` profile this building's rooms relight from. Two buildings
    /// at the same fill rate off the same ramp shimmer at the same rate in the
    /// same colour whatever their pitch; one bright and one dim tell apart in
    /// peripheral vision, which is where a skyline is actually read. Never
    /// `WARM_IX` — warm is a property of a BUILDING or of one lamp, not of a
    /// style, and the const block enforces it.
    draw: u8,
    /// A dark service core running the full height up the middle of the tier:
    /// lifts, risers and stairs, which have no windows. It splits the facade
    /// into two lit halves, so the building has an interior edge of its own
    /// rather than only the two its neighbours give it.
    core: bool,
}

/// A tier narrower than this has no middle to lose — a 4-cell building with a
/// dark core is two 1-cell slivers, which reads as damage rather than as a
/// building.
const CORE_MIN_W: usize = 5;

/// The ways a building lights up. Nothing here is rare: the styles exist to
/// make neighbours differ, so each has to turn up often enough to be somebody's
/// neighbour.
///
/// The table is read down the GLYPH and `draw` columns, not across the rows. An
/// earlier cut varied only pitch, dark floors and fill, and rendered a skyline
/// every building of which was the same 6x4 square at the same brightness on a
/// different clock — different on paper, one texture on the panel. Two entries
/// that share a glyph therefore differ on brightness, and two that share a
/// brightness differ on shape.
#[rustfmt::skip]
const STYLES: [Style; 17] = [
    // dense grid — the baseline everything else is read against
    Style { glyph: WINDOW_GLYPH, col_pitch: 1, row_pitch: 1, band_every: 0, checker: false, dim_below: 0,  fill_pct: 100, draw: 0, core: false },
    // wide-spaced columns, and barely on
    Style { glyph: WINDOW_GLYPH, col_pitch: 2, row_pitch: 1, band_every: 0, checker: false, dim_below: 0,  fill_pct: 85,  draw: 2, core: false },
    // banded: a dark service floor every fourth storey
    Style { glyph: WINDOW_GLYPH, col_pitch: 1, row_pitch: 1, band_every: 4, checker: false, dim_below: 0,  fill_pct: 100, draw: 0, core: false },
    // curtain wall: vertical strips instead of a grid, and lit like an office
    Style { glyph: STRIP_GLYPH,  col_pitch: 2, row_pitch: 1, band_every: 0, checker: false, dim_below: 0,  fill_pct: 95,  draw: 1, core: false },
    // mostly dark — one tower in the row with the lights off
    Style { glyph: WINDOW_GLYPH, col_pitch: 1, row_pitch: 1, band_every: 0, checker: false, dim_below: 0,  fill_pct: 45,  draw: 2, core: false },
    // checkerboard
    Style { glyph: WINDOW_GLYPH, col_pitch: 1, row_pitch: 1, band_every: 0, checker: true,  dim_below: 0,  fill_pct: 100, draw: 0, core: false },
    // tall floors: every other storey, so the bands are twice as far apart
    Style { glyph: WINDOW_GLYPH, col_pitch: 1, row_pitch: 2, band_every: 0, checker: false, dim_below: 0,  fill_pct: 100, draw: 1, core: false },
    // ribbon glazing on alternating floors
    Style { glyph: RIBBON_GLYPH, col_pitch: 1, row_pitch: 2, band_every: 0, checker: false, dim_below: 0,  fill_pct: 100, draw: 0, core: false },
    // working late: the top third lit hard, the rest nearly dark
    Style { glyph: WINDOW_GLYPH, col_pitch: 1, row_pitch: 1, band_every: 0, checker: false, dim_below: 66, fill_pct: 100, draw: 1, core: false },
    // wide pitch AND a service floor: the two loudest of the old knobs together
    Style { glyph: WINDOW_GLYPH, col_pitch: 3, row_pitch: 1, band_every: 5, checker: false, dim_below: 0,  fill_pct: 90,  draw: 0, core: false },
    // sparse curtain wall, mostly dark
    Style { glyph: STRIP_GLYPH,  col_pitch: 3, row_pitch: 1, band_every: 0, checker: false, dim_below: 0,  fill_pct: 60,  draw: 2, core: false },
    // floor-to-ceiling glazing, every floor, all of it on: the tall-window slab
    Style { glyph: SLOT_GLYPH,   col_pitch: 1, row_pitch: 1, band_every: 0, checker: false, dim_below: 0,  fill_pct: 100, draw: 1, core: false },
    // the same tall windows around a dark lift core, at a wider pitch
    Style { glyph: SLOT_GLYPH,   col_pitch: 2, row_pitch: 1, band_every: 0, checker: false, dim_below: 0,  fill_pct: 95,  draw: 0, core: true  },
    // big panes, well apart: a gallery or a showroom, not an office grid
    Style { glyph: PANE_GLYPH,   col_pitch: 2, row_pitch: 1, band_every: 0, checker: false, dim_below: 0,  fill_pct: 100, draw: 0, core: false },
    // apartments: two short storeys per cell, most of them out
    Style { glyph: TWIN_GLYPH,   col_pitch: 1, row_pitch: 1, band_every: 0, checker: false, dim_below: 0,  fill_pct: 75,  draw: 2, core: false },
    // unbroken spandrel courses every other floor
    Style { glyph: BAND_GLYPH,   col_pitch: 1, row_pitch: 2, band_every: 0, checker: false, dim_below: 0,  fill_pct: 100, draw: 1, core: false },
    // the 9pm office slab: every room on one circuit, split by its service core
    Style { glyph: WINDOW_GLYPH, col_pitch: 1, row_pitch: 1, band_every: 0, checker: false, dim_below: 0,  fill_pct: 100, draw: 1, core: true  },
];

/// Weighted draw over `STYLES`. Half the draws are the punched square, which is
/// the shape the skyline is read against; the other half is split across the
/// four other proportions, so a run of four or five neighbours is unlikely to
/// be two of anything.
#[rustfmt::skip]
const STYLE_DRAW: [u8; 32] = [
     0,  0,  1,  1,  2,  2,  3,  3,  4,  5,  5,  6,  6,  7,  8,  8,
     9, 10, 11, 11, 11, 12, 12, 13, 13, 13, 14, 14, 15, 15, 16, 16,
];

/// The silhouette, independent of how the building is lit. Two towers in the
/// same style still read apart when one steps back and the other carries a
/// mast, and the stepped tops are most of what stops the skyline looking like
/// one rectangle repeated at different heights.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Shape {
    /// One box, floor to roof.
    Plain,
    /// The upper two thirds step in by a cell each side.
    Setback,
    /// A wider block at street level with the tower inset above it.
    Podium,
    /// A narrow mast standing clear above the roof. Towers and spires only —
    /// a radio mast on a four-storey block reads as a mistake.
    Mast,
}

#[rustfmt::skip]
const SHAPE_DRAW: [Shape; 16] = [
    Shape::Plain, Shape::Plain, Shape::Plain, Shape::Plain, Shape::Plain, Shape::Plain,
    Shape::Setback, Shape::Setback, Shape::Setback,
    Shape::Podium, Shape::Podium, Shape::Podium,
    Shape::Mast, Shape::Mast, Shape::Mast,
    Shape::Plain,
];

/// A setback or a podium needs a building wide enough to inset and tall enough
/// for the step to be visible; below this the shape falls back to `Plain`.
const STEP_MIN_W: usize = 5;
const STEP_MIN_H: usize = 6;
/// Storeys in a podium.
const PODIUM_ROWS: usize = 3;
/// Nothing — not a spire, not a mast, not the beacon — goes above this percent
/// of the panel height. It is the ceiling the sky is generated under, so a mast
/// can never punch out of the top of the frame.
const TOP_CEIL_PCT: usize = 32;

/// Lit-pixel density, in percent, per tenth of the frame, top to bottom. The
/// sky (the upper 60%) is generated straight from this. The lower 40% is
/// generated by BUILDINGS, so 43/63/47 is a measurement this has to reproduce
/// rather than a parameter it can set — see `STREET_ROWS`.
///
/// The SKY bands are half what was first read off the reference, which came off
/// a compressed screenshot whose anti-aliasing halos counted as lit pixels. The
/// reference sky is a scattered field with plenty of black in it, and the first
/// cut was a speckle. Old sky row: 29/40/31/22/20.
const BAND_DENSITY: [u32; 10] = [0, 14, 20, 15, 11, 10, 43, 63, 47, 0];
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

/// The draw tables index the palette and the art tables by hand, so an entry
/// outside its own ramp has to be a build failure — the same bargain
/// `bake_sprites` makes for the toaster art. A runtime check would fire on the
/// panel, headless, at 3am.
const _: () = {
    // Every cold profile inside the window ramp, the warm one inside the warm
    // ramp, and nothing straddling: a slot's profile IS its colour family, so a
    // profile with one foot in each ramp would make a building flicker between
    // white and yellow and no runtime test could name the cause.
    let mut d = 0;
    while d < DRAWS.len() {
        let mut i = 0;
        while i < DRAWS[d].len() {
            let c = DRAWS[d][i];
            if d == WARM_IX as usize {
                assert!(
                    c >= WARM_LO && c <= WARM_HI,
                    "the warm profile draws outside the warm ramp"
                );
            } else {
                assert!(
                    c >= WIN_LO && c <= WIN_HI,
                    "a cold profile draws outside the window ramp"
                );
            }
            i += 1;
        }
        d += 1;
    }
    assert!(
        WARM_IX as usize == DRAWS.len() - 1,
        "the warm profile must be last, so `draw < WARM_IX` means cold"
    );
    // The profiles have to actually be different distributions. Means, not
    // sets: two tables over the same ramp that merely reordered each other
    // would pass a set comparison and render identically.
    let mut d = 1;
    while d < WARM_IX as usize {
        let mut a = 0;
        let mut b = 0;
        let mut i = 0;
        while i < DRAWS[0].len() {
            a += DRAWS[0][i] as u32;
            b += DRAWS[d][i] as u32;
            i += 1;
        }
        assert!(
            a.abs_diff(b) >= DRAWS[0].len() as u32,
            "a brightness profile is not a stop away from the mixed one"
        );
        d += 1;
    }
    let mut i = 0;
    while i < STREAK_RAMP.len() {
        assert!(
            STREAK_RAMP[i] >= WIN_LO && STREAK_RAMP[i] <= WIN_HI,
            "the streak fades outside the window ramp"
        );
        assert!(
            i == 0 || STREAK_RAMP[i - 1] <= STREAK_RAMP[i],
            "the streak tail does not fade"
        );
        i += 1;
    }
    assert!(
        BACKSLASH_GLYPH != font::BLANK && SLASH_GLYPH != font::BLANK,
        "the streak tail is drawn with a blank glyph"
    );
    assert!(
        BEACON_COL as usize == PAL.len() - 1 && WARM_HI + 1 == BEACON_COL,
        "the ramps must tile the palette with the beacon last"
    );
    // The beacon is the only red, and the test that says so on screen is worth
    // nothing if the palette itself has a second one. `g * 2 < r` is the line:
    // the warm ramp is 0xF8/0xE0 at its most orange and nowhere near it.
    let mut i = 0;
    while i < PAL_RGB.len() {
        let red = PAL_RGB[i][1] as u32 * 2 < PAL_RGB[i][0] as u32;
        assert!(
            red == (i == BEACON_COL as usize),
            "the beacon is not the only red in the palette"
        );
        i += 1;
    }
    let mut i = 0;
    while i < STAR_DRAW.len() {
        assert!(
            (STAR_DRAW[i] as usize) < STAR_KIND.len(),
            "a star draw names a kind that does not exist"
        );
        i += 1;
    }
    let mut i = 0;
    while i < STAR_KIND.len() {
        assert!(
            STAR_KIND[i].lo >= SKY_LO && STAR_KIND[i].hi <= SKY_HI,
            "a star kind shades outside the sky ramp"
        );
        assert!(
            STAR_KIND[i].lo <= STAR_KIND[i].hi,
            "a star kind is inverted"
        );
        assert!(
            STAR_KIND[i].glyph != font::BLANK,
            "a star kind is drawn with a blank glyph"
        );
        // Brightness and size have to agree, or the pairing that makes the
        // field read as depth is just four random shapes.
        assert!(
            i == 0 || STAR_KIND[i - 1].hi < STAR_KIND[i].lo,
            "star kinds do not climb the sky ramp"
        );
        i += 1;
    }
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
        assert!(STYLES[i].row_pitch >= 1, "a style would light no floors");
        assert!(STYLES[i].dim_below <= 100, "a style dims more than it has");
        assert!(
            STYLES[i].glyph != font::BLANK,
            "a style lights its windows with a blank glyph"
        );
        // A quadrant mask that is all eight bits is `font::SOLID`, and a
        // building of solid cells is a filled rectangle with no windows in it.
        assert!(
            STYLES[i].glyph != font::SOLID,
            "a style fills its cells solid"
        );
        // Warm is a property of a BUILDING or of one lamp left on, never of a
        // style: a warm style would put an unbounded number of yellow windows
        // on the panel and `WARM_PER_MILLE` would stop being the cap it claims.
        assert!(
            STYLES[i].draw < WARM_IX,
            "a style relights from the warm ramp"
        );
        // A style glyph that collided with a star glyph would break the star
        // filter in `new`: it keeps a sky cell when the glyph it finds is one
        // of the star shapes, so a building cell would survive as a "star" and
        // the twinkle would paint sky colours inside the silhouette.
        let mut k = 0;
        while k < STAR_KIND.len() {
            assert!(
                STYLES[i].glyph != STAR_KIND[k].glyph,
                "a style glyph collides with the sky's"
            );
            k += 1;
        }
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
        assert!(
            lo > TOP_CEIL_PCT,
            "a roof class reaches over the mast ceiling"
        );
        // Ordered tallest-last, which is what lets the class index double as
        // the width-span narrowing below.
        assert!(
            i == 0 || ROOF_CLASS[i - 1].0 > lo,
            "roof classes are not ordered"
        );
        i += 1;
    }
};

/// One window slot's own settings, so the twinkle reproduces the building it
/// belongs to rather than some global average of all of them.
#[derive(Clone, Copy)]
struct Slot {
    glyph: u16,
    lit_pct: u8,
    /// Which `DRAWS` profile this window relights from — its colour family AND
    /// its brightness distribution in one index. Per SLOT rather than per draw,
    /// so a warm window is warm and a bright building stays bright every time
    /// they come back on; a window that picked either fresh each twinkle would
    /// flicker, which reads as a broken pixel rather than as a room.
    draw: u8,
}

#[cfg(test)]
impl Slot {
    /// Only the tests ask this — the render path wants the whole profile, not
    /// one bit of it, which is the point of folding family and brightness into
    /// one index.
    fn warm(self) -> bool {
        self.draw == WARM_IX
    }
}

/// The shooting star. A head cell and a short tail, saved-and-restored: this is
/// the only thing in the scene that moves, so it is the only thing that can
/// leave a trail behind it if it gets the bookkeeping wrong.
struct Streak {
    /// Cell indices, oldest first. `saved` holds what was underneath each.
    cells: [u32; STREAK_LEN],
    saved: [Cell; STREAK_LEN],
    n: usize,
    x: isize,
    y: isize,
    dx: isize,
    /// Head steps left. At zero the tail drains and the streak ends.
    steps: u8,
    live: bool,
}

pub struct City {
    grid: Grid,
    /// Every window SLOT, lit or not. Twinkle picks from here and re-draws;
    /// a slot inside two overlapping buildings appears once, carrying the
    /// FRONT building's style.
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
    /// Which `STAR_KIND` each star is, parallel to `stars`. Same argument as
    /// `slot`: shape and brightness class are fixed for the life of the scene.
    star_kind: Vec<u8>,
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
    /// The aircraft warning light on the tallest thing in the scene: a cell
    /// index, a period in frames and where in that period we are. The only
    /// thing here on a fixed clock rather than on the RNG.
    beacon: u32,
    beacon_period: u32,
    beacon_on: u32,
    beacon_ctr: u32,
    streak: Streak,
    /// Mean frames between shooting stars. Drawn per frame rather than counted
    /// down, so the interval is exponential and the sky is never metronomic.
    /// 0 disables them.
    shoot_rate: u32,
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

/// A star's cell, re-shaded inside its own kind.
#[inline]
fn star_cell(rng: &mut u32, kind: u8) -> Cell {
    let k = &STAR_KIND[kind as usize];
    let n = (k.hi - k.lo + 1) as u32;
    Cell::new(k.glyph, k.lo + (next_rand(rng) % n) as u16)
}

fn is_star_glyph(g: usize) -> bool {
    STAR_KIND.iter().any(|k| k.glyph as usize == g)
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
        // does not have to reverse it out of the frame rate. 40/s over the 1800
        // slots a 1920x1080 panel generates turns a given window over about
        // once a minute, which is the calm barely-shimmering scene the
        // reference is. 150/s read as a busy one.
        let win_per_sec = env_num(&["CITY_TWINKLE"], 40, 0, 100_000) as u32;
        // Sky re-shades per second. Slower and subtler by design, and this
        // ratio — a little over 3:1 against the windows — is the only place
        // that is expressed.
        let sky_per_sec = env_num(&["CITY_SKY_TWINKLE"], 12, 0, 100_000) as u32;
        // Aircraft warning light: period in milliseconds, lit for a quarter of
        // it. 1500ms is 0.67 flashes/sec, inside the 1-2s a real one runs at.
        let beacon_ms = env_num(&["CITY_BEACON_MS"], 1500, 100, 60_000) as u32;
        // MEAN seconds between shooting stars, 0 for none. One a minute.
        let shoot_secs = env_num(&["CITY_SHOOT_SECS"], 60, 0, 3600) as u32;

        let fps = fps.max(1);
        let base = (rows * BASE_PCT / 100).max(1);
        let sky_end = rows * SKY_PCT / 100;
        let ceil = rows * TOP_CEIL_PCT / 100;

        let mut rng = 0x0c17_51de;
        let mut stars = Vec::new();
        let mut star_kind = Vec::new();
        for cy in 0..sky_end {
            let d = BAND_DENSITY[cy * BANDS / rows];
            for cx in 0..cols {
                if next_rand(&mut rng) % 100 >= d {
                    continue;
                }
                let i = cy * cols + cx;
                let kind = STAR_DRAW[next_rand(&mut rng) as usize % STAR_DRAW.len()];
                grid.set(i, star_cell(&mut rng, kind));
                stars.push(i as u32);
                star_kind.push(kind);
            }
        }

        // Buildings, left to right on one baseline. Each writes its WHOLE
        // silhouette — blanks included — so a nearer building punches the sky
        // and the building behind it out of its own footprint.
        let mut windows = Vec::new();
        let mut slot: Vec<Slot> = Vec::new();
        // cell -> its entry in `windows`, so a cell an overlapping building
        // redraws keeps ONE slot carrying the FRONT building's style. Without
        // it the building behind keeps a stale entry pointing at the same cell,
        // and the twinkle speckles the overlap with the wrong style at twice
        // the rate. Scratch: dropped at the end of `new`.
        let mut owner = vec![u32::MAX; cols * rows];
        // The tallest tier seen so far, for the beacon. Strictly-less keeps the
        // LEFTMOST of a tie, so the beacon does not hop between two equal
        // towers from one generation to the next.
        let mut tallest = (usize::MAX, 0usize, 1usize);
        let mut warm_bldgs = 0u32;
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
            // A whole warm building. Rolled HERE, per building, so it is the
            // silhouette that is warm and not a patch of it — and so a warm
            // building keeps its style's pitch, shape and fill and differs from
            // its neighbour on colour as well as on all of those.
            let warm_bldg =
                warm_bldgs < WARM_BLDG_MAX && next_rand(&mut rng) % 1000 < WARM_BLDG_PER_MILLE;
            warm_bldgs += u32::from(warm_bldg);
            let draw = if warm_bldg { WARM_IX } else { st.draw };

            // The silhouette as up to three stacked boxes, bottom first: each
            // is (left, width, top, solid), and box k runs from its own top
            // down to box k-1's. A shape whose building is too small for it
            // falls back to one box rather than drawing a degenerate step.
            let mut tier = [(x, w, top, false); 3];
            let mut tiers = 1;
            let tall_enough = w >= STEP_MIN_W && base - top >= STEP_MIN_H;
            match SHAPE_DRAW[next_rand(&mut rng) as usize % SHAPE_DRAW.len()] {
                Shape::Setback if tall_enough => {
                    tier[0] = (x, w, top + (base - top) / 3, false);
                    tier[1] = (x + 1, w - 2, top, false);
                    tiers = 2;
                }
                Shape::Podium if tall_enough => {
                    tier[0] = (x, w, base - PODIUM_ROWS, false);
                    tier[1] = (x + 1, w - 2, top, false);
                    tiers = 2;
                }
                // The mast is SOLID: it lights every cell whatever the style
                // says, because a mast is structure and not a wall of rooms —
                // and because a pitch or a service floor could otherwise leave
                // the tallest thing in the scene entirely dark.
                Shape::Mast if class >= 1 => {
                    let mw = if w >= 7 { 2 } else { 1 };
                    let h = 2 + next_rand(&mut rng) as usize % 4;
                    let mtop = top.saturating_sub(h).max(ceil);
                    if mtop + 1 < top {
                        tier[1] = (x + (w - mw) / 2, mw, mtop, true);
                        tiers = 2;
                    }
                }
                _ => {}
            }
            if tier[tiers - 1].2 < tallest.0 {
                let (tx, tw, ttop, _) = tier[tiers - 1];
                tallest = (ttop, tx, tw);
            }

            let mut bot = base;
            for (t, &(tx, tw, ttop, solid)) in tier[..tiers].iter().enumerate() {
                for cy in ttop..bot {
                    // The top row of the topmost tier is always lit-eligible:
                    // it draws the roof edge, and it is what the beacon stands
                    // on. A service floor or a row pitch that landed there
                    // would leave the roofline with no windows in it at all.
                    let roof = t == tiers - 1 && cy == ttop;
                    // And the ground floor is always lit-eligible, so every
                    // building MEETS the ground rather than hovering a row
                    // above it on the wrong parity.
                    let plain = roof || cy + 1 == base;
                    let up = base - 1 - cy;
                    let dark_floor =
                        !plain && st.band_every > 0 && up % st.band_every == st.band_every - 1;
                    let dark_row = !plain && !up.is_multiple_of(st.row_pitch);
                    let mut p = if cy + STREET_ROWS >= base {
                        lit_pct / 2
                    } else {
                        lit_pct
                    };
                    if st.dim_below > 0 && (base - cy) * 100 <= (base - top) * st.dim_below as usize
                    {
                        p /= 5;
                    }
                    if solid {
                        p = lit_pct;
                    }
                    for cx in tx..(tx + tw).min(cols) {
                        let i = cy * cols + cx;
                        // A cell the style does not light is written CLEAR —
                        // the building still has to punch its whole silhouette
                        // through whatever is behind it — and is NOT a window
                        // slot, so the twinkle can never light it and dissolve
                        // the style.
                        let dark = !solid
                            && (dark_floor
                                || dark_row
                                || !(cx - x).is_multiple_of(st.col_pitch)
                                // Counted on the TIER, so the core stays up the
                                // middle of a setback instead of drifting to
                                // one side of it when the facade steps in.
                                || (st.core && !plain && tw >= CORE_MIN_W && cx == tx + tw / 2)
                                || (st.checker && !plain && !(cx + cy).is_multiple_of(2)));
                        if dark {
                            grid.set(i, Cell::CLEAR);
                            // The cell is dark in the style that owns it NOW,
                            // so it must stop being a slot of whatever was
                            // behind it.
                            if owner[i] != u32::MAX {
                                slot[owner[i] as usize].lit_pct = 0;
                            }
                            continue;
                        }
                        let c = if next_rand(&mut rng) % 100 < p {
                            Cell::new(st.glyph, pick(&mut rng, &DRAWS[draw as usize]))
                        } else {
                            Cell::CLEAR
                        };
                        grid.set(i, c);
                        let mine = Slot {
                            glyph: st.glyph,
                            lit_pct: p as u8,
                            draw,
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
                bot = ttop;
            }
            let overlap = next_rand(&mut rng) as usize % OVERLAP_MAX;
            x += w.saturating_sub(overlap).max(1);
        }

        // The warm windows, chosen AFTER the fact rather than rolled per cell.
        // A per-cell roll at the same nominal rate came out anywhere between
        // 0.35% and 1.3% of the panel depending on how the RNG stream lined up
        // with the cell loop, and "at or under 1%" has to be a guarantee rather
        // than an average. Duplicates are allowed and merely mean one fewer.
        let warm_n = slot.len() * WARM_PER_MILLE as usize / 1000;
        for _ in 0..warm_n {
            let k = next_rand(&mut rng) as usize % slot.len().max(1);
            if k >= slot.len() {
                break;
            }
            slot[k].draw = WARM_IX;
            let i = windows[k] as usize;
            if grid.cell(i).glyph() != font::BLANK as usize {
                grid.set(i, Cell::new(slot[k].glyph, pick(&mut rng, &WARM_DRAW)));
            }
        }

        // The beacon sits one row above the tallest tier, centred on it. That
        // row is above every window slot in the scene, so nothing else can own
        // the cell.
        let (btop, bx, bw) = tallest;
        let beacon = ((btop.max(1) - 1) * cols + (bx + bw / 2).min(cols - 1)) as u32;
        grid.set(beacon as usize, Cell::new(WINDOW_GLYPH, BEACON_COL));

        // Sky lights a building — or the beacon — was drawn over are gone from
        // the panel; leaving them in `stars` would let the twinkle repaint one
        // inside a silhouette, in the sky family, over a window. The test is on
        // the star GLYPHS, not on "not blank": a building writes windows as
        // well as blanks.
        // Compacted in step rather than `retain`ed, because the kind has to
        // stay parallel to the index it belongs to.
        let mut keep = 0;
        for k in 0..stars.len() {
            if is_star_glyph(grid.cell(stars[k] as usize).glyph()) {
                stars[keep] = stars[k];
                star_kind[keep] = star_kind[k];
                keep += 1;
            }
        }
        stars.truncate(keep);
        star_kind.truncate(keep);

        let (win_rate, sky_rate) = ((win_per_sec << 8) / fps, (sky_per_sec << 8) / fps);
        let beacon_period = (beacon_ms * fps / 1000).max(2);
        Self {
            grid,
            windows,
            slot,
            stars,
            star_kind,
            // Generous: `due` can hand back one extra when the accumulator
            // rolls over, and a push past capacity would allocate in `render`.
            // The streak can repaint its whole tail plus the cell it restores,
            // and the beacon can toggle, in the same frame as a twinkle.
            dirty: Vec::with_capacity(((win_rate + sky_rate) >> 8) as usize + STREAK_LEN + 6),
            win_rate,
            sky_rate,
            win_acc: 0,
            sky_acc: 0,
            beacon,
            beacon_period,
            beacon_on: (beacon_period / 4).max(1),
            beacon_ctr: 0,
            streak: Streak {
                cells: [0; STREAK_LEN],
                saved: [Cell::CLEAR; STREAK_LEN],
                n: 0,
                x: 0,
                y: 0,
                dx: 1,
                steps: 0,
                live: false,
            },
            shoot_rate: if shoot_secs == 0 { 0 } else { shoot_secs * fps },
            rng,
        }
    }

    /// Advance the shooting star one cell, or start one. Everything it writes
    /// goes through `dirty` like every other cell — there is no second path to
    /// the panel, which is what makes `damage_covers_every_changed_scanline`
    /// able to catch a streak that under-reports.
    fn shoot(&mut self) {
        let (cols, rows) = (self.grid.cols(), self.grid.rows());
        let sky_end = rows * SKY_PCT / 100;
        if !self.streak.live {
            if self.shoot_rate == 0 || !next_rand(&mut self.rng).is_multiple_of(self.shoot_rate) {
                return;
            }
            // Start high and to one side, travelling down across the sky. The
            // skyline is off limits: a streak that crossed the buildings would
            // have to save and restore window slots the twinkle is also
            // writing, for no visual gain.
            self.streak.dx = if next_rand(&mut self.rng).is_multiple_of(2) {
                1
            } else {
                -1
            };
            self.streak.y = (rows * TOP_CEIL_PCT / 100) as isize;
            self.streak.x = (next_rand(&mut self.rng) as usize % cols) as isize;
            self.streak.steps = STREAK_STEPS;
            self.streak.n = 0;
            self.streak.live = true;
        }

        // Step the head, or start draining once it has run its length or left
        // the sky.
        if self.streak.steps > 0 {
            self.streak.steps -= 1;
            self.streak.x += self.streak.dx;
            self.streak.y += 1;
            let (x, y) = (self.streak.x, self.streak.y);
            if x < 0 || x >= cols as isize || y >= sky_end as isize {
                self.streak.steps = 0;
            } else {
                if self.streak.n == STREAK_LEN {
                    self.drop_tail();
                }
                let i = (y as usize * cols + x as usize) as u32;
                self.streak.cells[self.streak.n] = i;
                self.streak.saved[self.streak.n] = self.grid.cell(i as usize);
                self.streak.n += 1;
            }
        } else if self.streak.n > 0 {
            self.drop_tail();
        } else {
            self.streak.live = false;
            return;
        }

        // Repaint the live trail, newest hottest. Costs at most `STREAK_LEN`
        // cells on a frame that fires, once a minute.
        for k in 0..self.streak.n {
            let age = self.streak.n - 1 - k;
            let glyph = if age == 0 {
                SPARK_GLYPH
            } else if self.streak.dx > 0 {
                BACKSLASH_GLYPH
            } else {
                SLASH_GLYPH
            };
            let i = self.streak.cells[k];
            let c = Cell::new(glyph, STREAK_RAMP[age]);
            if c != self.grid.cell(i as usize) {
                self.grid.set(i as usize, c);
                self.dirty.push(i);
            }
        }
    }

    /// Put the oldest trail cell back exactly as it was found. Exact, not
    /// approximate: a streak that repainted sky over what it covered is the
    /// toaster-trail bug in a different costume.
    fn drop_tail(&mut self) {
        let i = self.streak.cells[0];
        self.grid.set(i as usize, self.streak.saved[0]);
        self.dirty.push(i);
        self.streak.cells.copy_within(1..self.streak.n, 0);
        self.streak.saved.copy_within(1..self.streak.n, 0);
        self.streak.n -= 1;
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
                // at. Its own glyph and its own brightness profile, too —
                // relighting a curtain-wall cell as a grid window dissolves the
                // building into its neighbour one twinkle at a time, relighting
                // a warm window white makes the accent flicker, and relighting a
                // dim block off the mixed ramp walks it to the district average.
                let c = if next_rand(&mut self.rng) % 100 < spec.lit_pct as u32 {
                    Cell::new(spec.glyph, pick(&mut self.rng, &DRAWS[spec.draw as usize]))
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
                let k = next_rand(&mut self.rng) as usize % self.stars.len();
                let i = self.stars[k];
                let c = star_cell(&mut self.rng, self.star_kind[k]);
                self.grid.set(i as usize, c);
                self.dirty.push(i);
            }
        }

        self.shoot();

        // Last, so that a streak which covered the beacon and restored a stale
        // phase is corrected inside the same frame rather than a flash later.
        let want = if self.beacon_ctr < self.beacon_on {
            Cell::new(WINDOW_GLYPH, BEACON_COL)
        } else {
            Cell::CLEAR
        };
        if want != self.grid.cell(self.beacon as usize) {
            self.grid.set(self.beacon as usize, want);
            self.dirty.push(self.beacon);
        }
        self.beacon_ctr = (self.beacon_ctr + 1) % self.beacon_period;

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
    use crate::dump;
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

    /// Every glyph the scene can draw has to exist and have to be non-blank, or
    /// a light is drawn as nothing at all and every other test here still
    /// passes.
    #[test]
    fn every_glyph_the_scene_uses_is_drawn() {
        let mut all: Vec<u16> = STYLES.iter().map(|s| s.glyph).collect();
        all.extend(STAR_KIND.iter().map(|k| k.glyph));
        all.push(WINDOW_GLYPH);
        for g in &all {
            assert_ne!(*g, font::BLANK, "a light must not be a blank glyph");
            assert!(
                font::GLYPHS[*g as usize].iter().any(|&r| r != 0),
                "glyph {g} has no lit pixels"
            );
            // Nothing may fill its cell, or adjacent lights merge into a filled
            // block and the silhouette stops being made of windows.
            assert!(
                font::GLYPHS[*g as usize].iter().any(|&r| r != 0xFF),
                "glyph {g} fills its cell"
            );
        }
        // The sky and the skyline must not share a shape, or they are one
        // texture — and `is_star_glyph` would misfile a building cell.
        for s in &STYLES {
            assert!(
                !is_star_glyph(s.glyph as usize),
                "a style uses a star shape"
            );
        }
        // The shapes must actually differ from each other.
        let mut shapes: Vec<u16> = STAR_KIND.iter().map(|k| k.glyph).collect();
        shapes.sort_unstable();
        shapes.dedup();
        assert_eq!(
            shapes.len(),
            STAR_KIND.len(),
            "two star kinds share a shape"
        );
    }

    /// The colour split is the look: sky neutral (`g == r`) with a blue shift,
    /// windows cyan (`g > r`) and hotter than any sky light, the warm accent the
    /// window ramp turned over (`r > g > b`), and the beacon red and alone.
    #[test]
    fn the_ramps_are_different_colour_families() {
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
        for i in WARM_LO..=WARM_HI {
            let c = PAL_RGB[i as usize];
            assert!(c[0] > c[1] && c[1] > c[2], "warm {i} is not warm");
            // And it has to sit beside the window ramp, not above it: a warm
            // window brighter than the hottest cyan one reads as a fire.
            assert!(
                lum(c) <= lum(PAL_RGB[WIN_LO as usize]),
                "warm {i} outshines the hottest window"
            );
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
        for (k, &i) in c.windows.iter().enumerate() {
            let cell = c.grid.cell(i as usize);
            if cell.glyph() == font::BLANK as usize {
                continue;
            }
            let col = cell.colour() as u16;
            let want = if c.slot[k].warm() {
                WARM_LO..=WARM_HI
            } else {
                WIN_LO..=WIN_HI
            };
            assert!(want.contains(&col), "window cell {i} is {col}");
            assert!(
                i as usize / cols >= c.grid.rows() * TOP_CEIL_PCT / 100,
                "a lit window above the ceiling a mast can reach"
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
    /// the roof classes deliberately push band 5 above the table's number to
    /// buy a skyline with real towers in it, and the sky was halved against the
    /// first reading of it because the reference sky has black in it. Pinning
    /// the numbers here would have made those changes test failures instead of
    /// improvements, which is the wrong way round.
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
        // The sky is SCATTERED, not speckled: plenty of black between the
        // lights. An absolute ceiling, because the envelope below is measured
        // against the table and would follow the table straight back up to the
        // dense field this replaced (26/42/29/28/33 measured, band for band).
        for b in 1..=5 {
            assert!(
                pct[b] < 25,
                "sky band {b} is a speckle at {}%: {pct:?}",
                pct[b]
            );
        }
        // The sky bands stay inside a loose envelope of the table, so a
        // generator that stopped drawing sky entirely, or filled it, still
        // fails — without pinning any band to its exact value.
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
        let built: Vec<(usize, usize)> = (0..cols)
            .filter(|&cx| lowest[cx] > 0)
            .map(|cx| (cx, lowest[cx]))
            .collect();
        assert!(
            built.len() > cols / 2,
            "only {} columns built on",
            built.len()
        );
        // The one legal exception is a MAST: a two-or-three cell tier standing
        // above a neighbour whose gutter column is dark all the way down, so
        // the only slots in that column are up in the sky. It is a fragment,
        // not a building — which is exactly what the count check below says,
        // and what a building floating at mid-height would fail.
        let floating: Vec<(usize, usize)> =
            built.iter().copied().filter(|&(_, l)| l != base).collect();
        assert!(
            floating.len() <= cols / 20,
            "{} columns do not reach the baseline: {floating:?}",
            floating.len()
        );
        for (cx, l) in floating {
            let n = c
                .windows
                .iter()
                .filter(|&&i| i as usize % cols == cx)
                .count();
            assert!(
                n <= 6,
                "column {cx} holds a {n}-slot building that stops at {l} instead of {base}"
            );
        }
    }

    /// Every style and every shape has to be REACHABLE. A table entry no draw
    /// names is art nobody will ever see, and it is invisible to every other
    /// test here.
    #[test]
    fn every_style_and_shape_is_drawable() {
        for s in 0..STYLES.len() {
            assert!(
                STYLE_DRAW.contains(&(s as u8)),
                "style {s} is unreachable from STYLE_DRAW"
            );
        }
        for sh in [Shape::Plain, Shape::Setback, Shape::Podium, Shape::Mast] {
            assert!(
                SHAPE_DRAW.contains(&sh),
                "shape {sh:?} is unreachable from SHAPE_DRAW"
            );
        }
        for k in 0..STAR_KIND.len() {
            assert!(
                STAR_DRAW.contains(&(k as u8)),
                "star kind {k} is unreachable from STAR_DRAW"
            );
        }
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
            glyphs.len() > 2,
            "the whole skyline uses {} window glyphs: {glyphs:?}",
            glyphs.len()
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

        // A mast: a column standing at least two rows clear of everything
        // within two columns of it. Only the `Mast` shape can do that — a
        // setback steps IN, and a neighbouring tower is a whole building wide.
        let mut roof = vec![usize::MAX; cols];
        for &i in &c.windows {
            let (cx, cy) = (i as usize % cols, i as usize / cols);
            roof[cx] = roof[cx].min(cy);
        }
        let masts = (2..cols - 2)
            .filter(|&cx| {
                let r = roof[cx];
                r != usize::MAX
                    && [cx - 2, cx + 2]
                        .iter()
                        .all(|&o| roof[o] == usize::MAX || roof[o] >= r + 2)
            })
            .count();
        assert!(masts > 0, "no building carries a mast");

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

    /// The warm light is TWO things at two scales, and the scales are the point.
    /// A lamp left on in a cold tower is one window; a hotel is a whole
    /// silhouette. A city with only the first is a cold skyline with pixels in
    /// it, and one with only the second has lost the accent — so both are
    /// asserted, and the second is bounded, because four warm towers of
    /// twenty-odd is not an exception any more, it is a second colour scheme.
    ///
    /// Both are STABLE. A window that picked its family fresh on every twinkle
    /// would hold every percentage here on average and still read as a broken
    /// pixel rather than as a lamp.
    #[test]
    fn warm_light_is_a_few_whole_buildings_and_a_scatter_of_lamps() {
        let p = panel();
        let mut c = city();
        let mut buf = vec![0u32; p.buf_len()];
        let cols = c.grid.cols();

        let warm_col = |col: u16| (WARM_LO..=WARM_HI).contains(&col);
        // Per COLUMN, because "a warm building" is a claim about the panel and
        // not about the order `slot` happens to be filled in. A warm column is
        // one whose lit windows are mostly warm; a warm building is a run of
        // them, and three columns is the narrowest building the generator makes.
        let warm_runs = |c: &City| -> (Vec<usize>, usize, usize) {
            let mut warm = vec![0usize; cols];
            let mut lit = vec![0usize; cols];
            for &i in &c.windows {
                let cell = c.grid.cell(i as usize);
                if cell.glyph() == font::BLANK as usize {
                    continue;
                }
                lit[i as usize % cols] += 1;
                warm[i as usize % cols] += usize::from(warm_col(cell.colour() as u16));
            }
            let mut runs = Vec::new();
            let mut run = 0;
            for cx in 0..=cols {
                let hot = cx < cols && lit[cx] > 0 && warm[cx] * 2 > lit[cx];
                if hot {
                    run += 1;
                } else {
                    if run >= 3 {
                        runs.push(run);
                    }
                    run = 0;
                }
            }
            (runs, warm.iter().sum(), lit.iter().sum())
        };

        let (runs, warm, lit) = warm_runs(&c);
        assert!(warm > 0, "no warm window in the whole city");
        assert!(
            !runs.is_empty(),
            "no whole warm building: {warm} warm windows of {lit}, all scattered"
        );
        assert!(
            runs.len() <= WARM_BLDG_MAX as usize,
            "{} warm buildings against a ceiling of {WARM_BLDG_MAX}: {runs:?}",
            runs.len()
        );
        // The ceiling on the whole thing. Two buildings of the widest class is
        // about an eighth of the lit windows; past that the skyline has stopped
        // being a cold one.
        assert!(
            warm * 6 <= lit,
            "{warm} warm windows of {lit} lit is more than a sixth of the city"
        );

        // And the scattered lamps survive alongside them: warm windows OUTSIDE
        // any warm building. Without this the whole test passes on two hotels
        // and no lamps at all, which is the accent the reference has, gone.
        // A lamp is one or two warm windows in a column with no warm building
        // NEXT TO IT either. Both halves are needed: a warm building's
        // overlapped edge column holds one or two warm windows too, so counting
        // sparse columns alone scores a hotel's own corner as somebody's desk
        // light and the assertion passes with every lamp deleted.
        let mut warm_in_col = vec![0usize; cols];
        for (k, &i) in c.windows.iter().enumerate() {
            warm_in_col[i as usize % cols] += usize::from(c.slot[k].warm());
        }
        // A warm building's own columns, and TWO either side. Two, not one: a
        // warm building at a column pitch of 2 ends in a column of its own with
        // a single warm window in it, and at a reach of one that column scored
        // as somebody's desk light — so the lamp check passed with every lamp
        // deleted from the generator.
        let near_hotel =
            |cx: usize| (cx.saturating_sub(2)..=(cx + 2).min(cols - 1)).any(|o| warm_in_col[o] > 2);
        let strays: usize = (0..cols)
            .filter(|&cx| warm_in_col[cx] > 0 && !near_hotel(cx))
            .map(|cx| warm_in_col[cx])
            .sum();
        assert!(
            strays > 0,
            "every warm window is in one of the {} warm buildings",
            runs.len()
        );
        // Rare, at the rate `WARM_PER_MILLE` promises, with room for a few of
        // the draws to have landed inside a warm building already.
        assert!(
            strays * 1000 <= c.slot.len() * WARM_PER_MILLE as usize,
            "{strays} scattered warm lamps is over the {WARM_PER_MILLE}-per-mille cap"
        );

        // Stability: a slot's family never changes, however often it is redrawn.
        for _ in 0..60_000 {
            saver::frame(&mut c, &mut buf, &p);
        }
        for (k, &i) in c.windows.iter().enumerate() {
            let cell = c.grid.cell(i as usize);
            if cell.glyph() == font::BLANK as usize {
                continue;
            }
            assert_eq!(
                warm_col(cell.colour() as u16),
                c.slot[k].warm(),
                "slot {k} changed colour family under the twinkle"
            );
        }
        let (runs2, warm2, lit2) = warm_runs(&c);
        assert!(warm2 > 0, "the warm windows drained away");
        assert_eq!(
            runs2.len(),
            runs.len(),
            "the warm buildings dissolved over 60k frames: {runs:?} -> {runs2:?}"
        );
        assert!(
            warm2 * 6 <= lit2,
            "{warm2} warm of {lit2} lit after 60k frames"
        );
    }

    /// The complaint this saver kept coming back on: every building read the
    /// same, because every WINDOW was the same 6x4 square and only the clock
    /// changed. Pitch, dark floors and fill rate all vary which cells are lit —
    /// none of them varies what a window IS, so a skyline built only out of
    /// them is one texture at a dozen phases, and every other test in this file
    /// passes on it.
    ///
    /// So this pins PROPORTION, which is what the eye reads a facade by, and it
    /// pins the share of draws the commonest shape gets, because four distinct
    /// shapes nobody ever draws is the same skyline.
    #[test]
    fn the_window_shapes_span_several_proportions() {
        // Bounding box AND lit area. The box alone is not the shape: two small
        // windows stacked in one cell and one tall window filling it have the
        // SAME box and read nothing alike, so a box-only comparison would have
        // scored this table one proportion richer than it is.
        let extent = |g: u16| -> (usize, usize, u32) {
            let rows = &font::GLYPHS[g as usize];
            let lo = rows.iter().position(|&r| r != 0).expect("a blank window");
            let hi = rows.iter().rposition(|&r| r != 0).expect("a blank window");
            let bits = rows.iter().fold(0u8, |a, &r| a | r);
            let w = (0..8).filter(|b| bits & (0x80 >> b) != 0).count();
            let fill = rows.iter().map(|r| r.count_ones()).sum();
            (w, hi + 1 - lo, fill)
        };

        let mut shapes: Vec<(usize, usize, u32)> = STYLES.iter().map(|s| extent(s.glyph)).collect();
        shapes.sort_unstable();
        shapes.dedup();
        assert!(
            shapes.len() >= 5,
            "the whole skyline is built from {} window proportions: {shapes:?}",
            shapes.len()
        );
        // Both orientations and a square between them. A set of four shapes
        // that were all wider than tall would satisfy the count and still read
        // as one facade.
        assert!(
            shapes.iter().any(|&(w, h, _)| h >= 2 * w),
            "no window is taller than it is wide: {shapes:?}"
        );
        assert!(
            shapes.iter().any(|&(w, h, _)| w >= 2 * h),
            "no window is wider than it is tall: {shapes:?}"
        );
        assert!(
            shapes.iter().any(|&(w, h, _)| w * 2 > h && h * 2 > w),
            "no window is roughly square: {shapes:?}"
        );

        // And the shapes have to be DRAWN. A commonest shape over three fifths
        // of the draws is the skyline this replaced: everything else is
        // seasoning and the eye reads the template.
        let mut per_glyph = [0usize; STYLES.len()];
        for &d in &STYLE_DRAW {
            per_glyph[STYLES
                .iter()
                .position(|s| s.glyph == STYLES[d as usize].glyph)
                .expect("a style with no glyph")] += 1;
        }
        let commonest = *per_glyph.iter().max().expect("no styles");
        assert!(
            commonest * 5 <= STYLE_DRAW.len() * 3,
            "one window shape takes {commonest} of {} draws",
            STYLE_DRAW.len()
        );

        // On the panel, not just in the table: at least four shapes actually
        // reach a slot, and none of them owns most of the skyline.
        let c = city();
        let mut count = std::collections::BTreeMap::new();
        for s in &c.slot {
            *count.entry(s.glyph).or_insert(0usize) += 1;
        }
        assert!(
            count.len() >= 4,
            "{} window shapes on the panel: {count:?}",
            count.len()
        );
        let top = *count.values().max().expect("an empty city");
        assert!(
            top * 5 <= c.slot.len() * 3,
            "one shape holds {top} of {} slots",
            c.slot.len()
        );
    }

    /// Buildings differ in BRIGHTNESS, not only in pitch. Every window drawn
    /// from one ramp gives a skyline that shimmers at the same rate in the same
    /// colour whatever its geometry — which is most of why the old one read as
    /// one wall — and it passes every density, roofline and glyph test here.
    ///
    /// And the profile is per SLOT, so it survives the twinkle: a dim block
    /// that relit off the mixed ramp would walk to the district average over a
    /// few minutes with nothing failing while it happened.
    #[test]
    fn buildings_differ_in_brightness_and_keep_their_profile() {
        let p = panel();
        let mut c = city();
        let mut buf = vec![0u32; p.buf_len()];
        let cols = c.grid.cols();

        // Percent of a column's lit cold windows that are in the HOT half of
        // the ramp. Deliberately not the mean colour: with a dozen windows to a
        // column the mean of one global draw wanders by a whole ramp step on
        // sampling noise alone, so a "one step between the deciles" assertion
        // passes with every building on the same table — measured, not
        // reasoned. The hot SHARE separates the profiles by construction:
        // `BRIGHT_DRAW` is 14/16 hot, `WINDOW_DRAW` 3/16 and `DIM_DRAW` none.
        //
        // Warm windows are excluded: they are a different family and would
        // answer this question by accident.
        let hot = (WIN_LO + WIN_HI) / 2;
        let hot_share = |c: &City| -> Vec<u32> {
            let mut h = vec![0u32; cols];
            let mut n = vec![0u32; cols];
            for &i in &c.windows {
                let cell = c.grid.cell(i as usize);
                let col = cell.colour() as u16;
                if cell.glyph() == font::BLANK as usize || !(WIN_LO..=WIN_HI).contains(&col) {
                    continue;
                }
                h[i as usize % cols] += u32::from(col <= hot);
                n[i as usize % cols] += 1;
            }
            (0..cols)
                .filter(|&cx| n[cx] >= 8)
                .map(|cx| h[cx] * 100 / n[cx])
                .collect()
        };

        let mut m = hot_share(&c);
        m.sort_unstable();
        assert!(m.len() > 20, "only {} columns to compare", m.len());
        // Decile to decile, so a single odd column cannot carry it. Fifty
        // points between the dim tenth of the skyline and the bright tenth; one
        // global draw measures about twenty-five, all of it noise.
        let (lo, hi) = (m[m.len() / 10], m[m.len() - 1 - m.len() / 10]);
        assert!(
            hi - lo >= 50,
            "only {} points of hot-window share between the skyline's dim and bright ends",
            hi - lo
        );

        for _ in 0..60_000 {
            saver::frame(&mut c, &mut buf, &p);
        }
        // Every lit window still holds a colour its OWN profile can produce.
        // The profiles overlap in the middle, so this is not a strong claim
        // cell by cell — it is a strong claim over 1,800 of them, and a slot
        // relit from the wrong table trips it within a few hundred frames.
        for (k, &i) in c.windows.iter().enumerate() {
            let cell = c.grid.cell(i as usize);
            if cell.glyph() == font::BLANK as usize {
                continue;
            }
            let draw = &DRAWS[c.slot[k].draw as usize];
            assert!(
                draw.contains(&(cell.colour() as u16)),
                "slot {k} holds colour {} which its profile {} never draws",
                cell.colour(),
                c.slot[k].draw
            );
        }
        // And the spread is still there: a twinkle that homogenised the city
        // would leave every window inside its own profile and still flatten it.
        let mut m2 = hot_share(&c);
        m2.sort_unstable();
        let (lo2, hi2) = (m2[m2.len() / 10], m2[m2.len() - 1 - m2.len() / 10]);
        assert!(
            hi2 - lo2 >= 50,
            "the brightness spread collapsed to {} points over 60k frames",
            hi2 - lo2
        );
    }

    /// A dark service core: lifts and risers have no windows, so the facade is
    /// split into two lit halves and the building carries an edge of its own
    /// instead of only the two its neighbours give it.
    ///
    /// Two things pretend to be a core and both had to be ruled out, because
    /// with either of them in the detector this test scored three panels out of
    /// four with the core deleted from the generator:
    ///
    /// * The WIDE COLUMN PITCH leaves dark columns too. Looking two columns out
    ///   separates them — at a pitch of 2 the column beyond the lit neighbour is
    ///   dark again, at a pitch of 3 the neighbour itself is — so only a core
    ///   has four built columns around one dark one.
    /// * A SEAM between two buildings can also show four built columns around a
    ///   dark one, by borrowing two of them from each neighbour. The five
    ///   columns of a core are one facade, so they share a roof row; a seam's do
    ///   not, and requiring that is what made the detector fire on 3 of 4 panels
    ///   with the core and 0 of 4 without it.
    ///
    /// A slot is "built" only where its building still lights it: a cell an
    /// overlapping building darkened keeps its entry at `lit_pct` 0, and
    /// counting those as windows is the third way this measured the wrong thing.
    #[test]
    fn some_buildings_have_a_dark_service_core() {
        // Over several geometries, not one seed. A core needs a cored style to
        // land on a building wide enough to have a middle, which is a handful
        // of the two dozen a panel generates — so one generation carries one or
        // two and any unrelated change to the RNG stream can take those away.
        let mut with_cores = 0;
        for (w, h) in [(1920, 1080), (1600, 900), (1280, 720), (2560, 1440)] {
            let c = City::new(&Panel::new(w, h, w), 30);
            let (cols, rows) = (c.grid.cols(), c.grid.rows());

            let mut built = vec![vec![false; rows]; cols];
            let mut roof = vec![usize::MAX; cols];
            for (k, &i) in c.windows.iter().enumerate() {
                let (cx, cy) = (i as usize % cols, i as usize / cols);
                if c.slot[k].lit_pct > 0 {
                    built[cx][cy] = true;
                    roof[cx] = roof[cx].min(cy);
                }
            }
            // The core has to run a real distance, or one missing cell counts.
            let core_rows = STEP_MIN_H;
            let cores = (2..cols - 2)
                .filter(|&cx| {
                    [cx - 2, cx - 1, cx + 1, cx + 2]
                        .iter()
                        .all(|&o| roof[o] != usize::MAX && roof[o] == roof[cx - 1])
                        && (0..rows - core_rows).any(|r0| {
                            (r0..r0 + core_rows).all(|cy| {
                                !built[cx][cy]
                                    && [cx - 2, cx - 1, cx + 1, cx + 2]
                                        .iter()
                                        .all(|&o| built[o][cy])
                            })
                        })
                })
                .count();
            // A minority wherever they appear, or it is not a core, it is the
            // column pitch leaving every other column dark.
            assert!(
                cores < cols / 8,
                "{cores} of {cols} columns are service cores ({w}x{h})"
            );
            with_cores += usize::from(cores > 0);
        }
        // Two of four, not three: a core is rare enough per panel that pinning
        // the observed count would make any future change to the draw order a
        // failure. Nought of four is the regression, and that is what this
        // catches.
        assert!(
            with_cores >= 2,
            "only {with_cores} of 4 panels has a dark service core: a column dark for \
             {STEP_MIN_H} rows inside a facade of five columns on one roofline"
        );
    }

    /// The beacon: one red light, on the tallest thing, centred on it, and
    /// actually flashing. Stuck on and stuck off are the failure modes, and a
    /// beacon painted anywhere at all passes every other test in this file.
    #[test]
    fn the_beacon_flashes_on_the_tallest_building() {
        // Over several geometries, because the tallest thing on any one panel
        // may be a single-cell mast, and a beacon nailed to the LEFT edge of a
        // one-cell roof is indistinguishable from a centred one.
        let mut widest = 0;
        for (w, h) in [(1920, 1080), (1600, 900), (1280, 720), (2560, 1440)] {
            let c = City::new(&Panel::new(w, h, w), 30);
            let cols = c.grid.cols();
            let (bx, by) = (c.beacon as usize % cols, c.beacon as usize / cols);

            // It is above everything: the highest window slot is the roof it
            // stands on, exactly one row down.
            let top = c
                .windows
                .iter()
                .map(|&i| i as usize / cols)
                .min()
                .expect("a city with no windows");
            assert_eq!(by + 1, top, "the beacon is not one row above the roofline");

            // Centred on THAT roof: the run of slots in the top row it sits on.
            let mut on_top: Vec<usize> = c
                .windows
                .iter()
                .filter(|&&i| i as usize / cols == top)
                .map(|&i| i as usize % cols)
                .collect();
            on_top.sort_unstable();
            // The contiguous group containing the beacon. STRICTLY adjacent:
            // an earlier cut allowed a gap of up to the widest column pitch, and
            // swallowed a neighbouring tower's roof two columns away into a
            // one-cell mast's "roof", which put the beacon off centre on a group
            // it was never standing on. A pitched roof row then reads as a
            // one-cell group and says nothing — which is what `widest` below is
            // for: some roof in the set has to be wide enough to mean something.
            let mut lo = bx;
            let mut hi = bx;
            loop {
                let l = on_top.iter().rev().find(|&&x| x + 1 == lo);
                let h = on_top.iter().find(|&&x| x == hi + 1);
                match (l, h) {
                    (None, None) => break,
                    (l, h) => {
                        lo = *l.unwrap_or(&lo);
                        hi = *h.unwrap_or(&hi);
                    }
                }
            }
            widest = widest.max(hi + 1 - lo);
            // Within half a cell of the middle: `|2x - (lo+hi)| <= 1` is exact
            // for an odd-width roof and allows either middle column of an even
            // one, which is all "centred" can mean on a grid.
            assert!(
                (2 * bx).abs_diff(lo + hi) <= 1,
                "the beacon at {bx} is not centred on its roof {lo}..={hi} ({w}x{h})"
            );
        }
        // Non-vacuous: at least one of those roofs has to be wide enough that
        // being centred on it says anything at all.
        assert!(
            widest >= 3,
            "the widest tallest-roof seen was {widest} cells"
        );

        let p = panel();
        let mut c = city();
        let cols = c.grid.cols();

        // It is red, and it is the only thing on the panel that is.
        let red = |cell: Cell| cell.colour() as u16 == BEACON_COL;
        let mut buf = vec![0u32; p.buf_len()];
        let mut lit = 0;
        let mut dark = 0;
        for _ in 0..(c.beacon_period * 4) {
            saver::frame(&mut c, &mut buf, &p);
            let cell = c.grid.cell(c.beacon as usize);
            if cell.glyph() == font::BLANK as usize {
                dark += 1;
            } else {
                assert!(red(cell), "the beacon is lit in {}", cell.colour());
                lit += 1;
            }
            let reds = (0..cols * c.grid.rows())
                .filter(|&i| c.grid.cell(i).glyph() != font::BLANK as usize && red(c.grid.cell(i)))
                .count();
            assert!(reds <= 1, "{reds} red cells on the panel");
        }
        assert!(
            lit > 0 && dark > 0,
            "the beacon does not flash: {lit}/{dark}"
        );
        // A quarter duty cycle, give or take the frame the sampling lands on.
        assert!(
            lit * 3 < dark,
            "the beacon is lit {lit} frames against {dark} dark: that is not a flash"
        );
    }

    /// Stars have shapes, they keep them, and the shapes track brightness. A
    /// field of one glyph is a texture, and a star that changes shape when it
    /// re-shades is noise — both pass every density test in this file.
    #[test]
    fn the_stars_have_several_shapes_and_keep_them() {
        let p = panel();
        let mut c = city();
        let mut buf = vec![0u32; p.buf_len()];

        let mut seen = [0usize; STAR_KIND.len()];
        for (k, &i) in c.stars.iter().enumerate() {
            seen[c.star_kind[k] as usize] += 1;
            let cell = c.grid.cell(i as usize);
            let kind = &STAR_KIND[c.star_kind[k] as usize];
            assert_eq!(
                cell.glyph(),
                kind.glyph as usize,
                "star {i} has a foreign shape"
            );
            assert!(
                (kind.lo..=kind.hi).contains(&(cell.colour() as u16)),
                "star {i} is shaded outside its kind"
            );
        }
        for (k, &n) in seen.iter().enumerate() {
            assert!(n > 0, "star kind {k} never appears in the sky");
        }

        for _ in 0..60_000 {
            saver::frame(&mut c, &mut buf, &p);
        }
        for (k, &i) in c.stars.iter().enumerate() {
            let cell = c.grid.cell(i as usize);
            let kind = &STAR_KIND[c.star_kind[k] as usize];
            // A streak may be sitting on a star as the run ends; it is the only
            // thing allowed to, and it restores what it covered.
            if c.streak.live && c.streak.cells[..c.streak.n].contains(&i) {
                continue;
            }
            assert_eq!(
                cell.glyph(),
                kind.glyph as usize,
                "star {i} changed shape under the twinkle"
            );
            assert!(
                (kind.lo..=kind.hi).contains(&(cell.colour() as u16)),
                "star {i} left its brightness tier"
            );
        }
    }

    /// The shooting star crosses and leaves NOTHING behind. Exact, not a
    /// ratio: the cells it covered have to hold what they held before, which is
    /// the invariant the toaster trails broke.
    #[test]
    fn a_shooting_star_crosses_and_leaves_nothing_behind() {
        let p = panel();
        let mut c = city();
        let mut buf = vec![0u32; p.buf_len()];
        // Nothing else may move, or "unchanged" is unmeasurable. The beacon
        // owns one cell and is excluded by index below.
        c.win_rate = 0;
        c.sky_rate = 0;
        c.shoot_rate = 1;

        let cells = c.grid.cols() * c.grid.rows();
        let before: Vec<Cell> = (0..cells).map(|i| c.grid.cell(i)).collect();

        saver::frame(&mut c, &mut buf, &p);
        assert!(c.streak.live, "the streak never fired");
        c.shoot_rate = 0; // exactly one, so the end state is comparable

        let mut moved = 0;
        let mut frames = 0;
        while c.streak.live {
            saver::frame(&mut c, &mut buf, &p);
            frames += 1;
            moved += (0..cells)
                .filter(|&i| i as u32 != c.beacon && c.grid.cell(i) != before[i])
                .count();
            assert!(frames < 200, "the streak never finished");
        }
        // It has to have actually crossed something: a streak that fired and
        // died in one frame would pass the restore check vacuously.
        assert!(
            moved > 20,
            "the streak only ever covered {moved} cell-frames"
        );
        assert!(frames > 10, "the streak lasted {frames} frames");
        for (i, &was) in before.iter().enumerate() {
            if i as u32 == c.beacon {
                continue;
            }
            assert_eq!(c.grid.cell(i), was, "the streak left cell {i} changed");
        }
    }

    /// And it fires about as often as the knob says. A streak that never fires
    /// passes the restore test above by doing nothing.
    #[test]
    fn shooting_stars_fire_at_roughly_the_configured_rate() {
        let p = panel();
        let mut c = city();
        let mut buf = vec![0u32; p.buf_len()];
        // 60s at 30fps, over 120k frames: 66 expected.
        let mut fired = 0;
        let mut was = false;
        for _ in 0..120_000 {
            saver::frame(&mut c, &mut buf, &p);
            fired += usize::from(c.streak.live && !was);
            was = c.streak.live;
        }
        assert!(
            (30..130).contains(&fired),
            "{fired} shooting stars in 120k frames against ~66 expected"
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
        // Shooting stars are rare by design, so at the shipped rate this test
        // would almost never cover one. They are the only thing here that
        // MOVES, so they are exactly what the check is for.
        c.shoot_rate = 40;

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
        let mut streak_frames = 0;
        for n in 1..2000 {
            prev.copy_from_slice(&buf);
            let d = saver::frame(&mut c, &mut buf, &p);
            let mut changed = 0;
            for y in 0..p.h {
                if prev[y * p.w..][..p.w] == buf[y * p.w..][..p.w] {
                    continue;
                }
                changed += 1;
                assert!(
                    dump::row_reported(&prev[y * p.w..][..p.w], &buf[y * p.w..][..p.w], y, &d),
                    "frame {n}: scanline {y} changed outside every reported rect"
                );
            }
            moved += usize::from(changed > 0);
            streak_frames += usize::from(c.streak.live);
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
        assert!(
            moved > 600,
            "the twinkle stalled ({moved}/1999 frames moved)"
        );
        assert!(streak_frames > 50, "no shooting star ran in 2000 frames");

        // The whole point of the saver. A regression to a full-screen repaint
        // is the thing this catches, and it is why the bound is a fraction of
        // the grid rather than "less than the panel". The MEDIAN, so the
        // handful of streak frames cannot hide a static-scene regression and
        // cannot fail it either.
        rows.sort_unstable();
        let median = rows[rows.len() / 2];
        let all = c.grid.rows() * c.grid.cell_h();
        assert!(
            median * 5 < all,
            "median damage {median} of {all} scanlines: the scene is not static"
        );
        // And the worst frame — a streak at full stretch — still has to be a
        // small fraction of the panel, or one shooting star costs a repaint.
        assert!(
            rows[rows.len() - 1] * 3 < all,
            "worst-frame damage {} of {all} scanlines",
            rows[rows.len() - 1]
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
        // Including the streak, which is the biggest thing `dirty` ever holds.
        c.shoot_rate = 30;
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
        assert!(worst > STREAK_LEN, "`dirty` never held a whole streak");
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
