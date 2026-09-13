//! Escape-time fractals: nineteen families in rotation, each zooming
//! continuously into a point known to sit on its boundary.
//!
//! # Why this is affordable at all
//!
//! Escape-time iteration is per-SAMPLE, and a sample per pixel at 1920x1080 is
//! 2M orbits a frame — an order of magnitude past the whole pod budget. The
//! only lever that matters is sample count, so the sample is the CELL, not the
//! pixel: a 20px cell is 96x54 = 5,184 orbits, 400x fewer. Every cell is
//! `SOLID`, so the fractal reads as chunky pixel art, which is the look the
//! atlas can actually draw.
//!
//! What is left is still expensive, and almost all of it is the iteration.
//! Per-frame cost swings 20x WITHIN a single cycle, because iteration depth
//! tracks zoom depth, and another 10x between families, because a Newton step
//! is not a Mandelbrot step. A spot measurement of this saver therefore means
//! nothing; the only number worth quoting is the mean over a whole rotation,
//! measured interleaved against matrix in one process — build both, alternate
//! 120-frame chunks across the whole rotation at 1920x1080 and 15fps, so the
//! two see the same machine state. See `bench::interleaved_against_matrix`.
//!
//! Nineteen families cost the same per frame as four did. Three runs of each,
//! alternating, on one dev machine: this version 399-450us a frame, the
//! four-family version it replaces 419-448us, with matrix reading 86-120us
//! throughout — the same number inside the noise. Adding fifteen families did
//! not make the frame loop more expensive, because a family is a branch, and
//! because the ones that would have been expensive are held down by the
//! per-family iteration budgets below. Absolute microseconds here drift 2x with
//! machine load, which is why both halves of that comparison were taken minutes
//! apart and why only the pair means anything.
//!
//! Per family the spread is wide — Sierpinski and Burning Ship are ~1.3x
//! matrix, the two magnets and Nova ~12-16x — so `bench::per_family_cost`
//! reports each separately. None of the four pre-existing families got slower.
//!
//! Those numbers are NOT a cost claim. The four-family version was measured on
//! the actual Pi 5 at **125.7 milli-cores (range 111-153m)**, at this grid and
//! this cell size, against a 500m pod limit that the most expensive saver in
//! the set (moire, 140m) does not threaten either. **The 19-family version has
//! not been measured on the Pi.** Scaling 125.7m by a dev-machine ratio is
//! exactly the arithmetic that once put this saver at 610m and was wrong by 5x:
//! measure it, or say it is unmeasured.
//!
//! What IS enforced here is that no family runs away. Newton, Nova and the two
//! magnets converge or blow up in a handful of steps, so running them to the
//! escape-time cap buys boundary noise at several times the price; each gets
//! its own iteration budget in `scan_family`, chosen so no family costs much
//! more than twice Mandelbrot's frame.
//!
//! `FRACTAL_CELL` remains the only global lever on the iteration — orbits scale
//! with its square — but at 125.7m there was nothing to buy with it, so the
//! default stays at 20 and the art keeps its 96x54 resolution. The floor under
//! the whole frame is the full-panel blit, which no cell size changes: under a
//! zoom every cell's colour moves every frame, so there is no sparse version of
//! this saver.
//!
//! Iteration depth rises with zoom depth (detail near the boundary needs it)
//! but is capped, because an uncapped budget is an unbounded frame. Past the
//! cap the zoom outruns the iteration and the panel goes FLAT — one colour, or
//! black — so the total depth of a cycle is clamped to `MAX_OCTAVES`, which is
//! a measured number and the only thing standing between the knob ranges and a
//! blank screen.
//!
//! # Why a cut, not a crossfade
//!
//! Crossfading two families means computing both, i.e. doubling the only
//! expensive thing here. Instead the palette carries three brightness tiers of
//! the same 30 colours, and a cycle dips through them into near-darkness on
//! either side of the cut. The cut happens at the dim tier, so nothing pops.
//!
//! # Why the family is a match per FRAME
//!
//! Nineteen families is nineteen arms in one `match`, evaluated once per frame,
//! each monomorphising `scan` into its own tight loop. A `&dyn Fn` or an `fn`
//! pointer here would turn that into one indirect call per cell — 78k a
//! second — which is the thing `Saver`'s doc bans. Adding a family costs code
//! size and nothing per frame; the same goes for the const-generic powers
//! (`multibrot`, `newton_n`, `cpow`), which exist so the power is a compile-time
//! unrolled ladder rather than a loop with a branch in the innermost loop.
//!
//! # How the targets and rates were chosen
//!
//! Not by eye. `descent::find_boundary_targets` splits a box into quadrants,
//! renders the family over each through the real `scan_family`, keeps the best
//! few and halves — a beam search whose score is what the frame LOOKS like, and
//! whose thresholds are each a bad picture that shipped: colour count alone
//! picks confetti, coherence alone picks an empty frame, brightness alone picks
//! a smooth diagonal wash, and only the band-JUMP count tells a fractal
//! boundary from a pretty gradient. `descent::safe_octaves_per_family` then
//! walks each family's own zoom and reports the last octave that still passes,
//! which is exactly what `rate` is: safe octaves over `MAX_OCTAVES`.
//!
//! Both are `#[ignore]`d tools rather than tests — they are how every
//! coordinate and every `rate` in `FAMILIES` was produced, and re-running them
//! is how to produce the next one.
//!
//! # Why f64
//!
//! f32 runs out of mantissa around a 10^4 zoom and the image dissolves into
//! flat blocks. The default cycle reaches ~640x and the longest
//! `FRACTAL_SECONDS` allows reaches ~7e4, so f32 would dissolve inside the
//! documented range; aarch64 does f64 in hardware, so the only cost is cache.

use std::f64::consts::FRAC_1_SQRT_2;

use crate::env_num;
use crate::font;
use crate::grid::{bake, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};

/// Colours in one loop of the ramp. The ramp is PERIODIC — entry 29 leads back
/// into entry 0 — because iteration count is reduced modulo this, and a seam
/// would draw a hard false contour across every band boundary.
const BANDS: usize = 30;
/// Brightness tiers of that ramp; tier 0 is full, the rest are the fade.
const TIERS: usize = 3;
const PAL_LEN: usize = 1 + BANDS * TIERS;

/// The Ultra Fractal "default" gradient — blue, white, amber, near-black, back
/// to blue — sampled at 30 stops. Its dark stop is lifted off pure black so it
/// cannot be mistaken for the interior.
#[rustfmt::skip]
const BASE_RGB: [[u8; 3]; BANDS] = [
    [0x00, 0x07, 0x64], [0x07, 0x1C, 0x79], [0x0D, 0x31, 0x8F], [0x14, 0x46, 0xA4], [0x1B, 0x5A, 0xBA],
    [0x25, 0x6F, 0xCC], [0x40, 0x82, 0xD3], [0x5A, 0x95, 0xDA], [0x74, 0xA8, 0xE0], [0x8E, 0xBB, 0xE7],
    [0xA9, 0xCE, 0xEE], [0xC3, 0xE1, 0xF4], [0xDD, 0xF4, 0xFB], [0xEE, 0xFA, 0xF0], [0xF1, 0xED, 0xCA],
    [0xF3, 0xE0, 0xA3], [0xF6, 0xD4, 0x7D], [0xF9, 0xC7, 0x57], [0xFC, 0xBA, 0x31], [0xFE, 0xAE, 0x0B],
    [0xE4, 0x97, 0x03], [0xBE, 0x7E, 0x06], [0x98, 0x64, 0x0A], [0x73, 0x4B, 0x0E], [0x4D, 0x31, 0x12],
    [0x27, 0x18, 0x15], [0x0B, 0x05, 0x1D], [0x08, 0x06, 0x2F], [0x06, 0x06, 0x40], [0x03, 0x07, 0x52],
];

/// Per-tier brightness, as a numerator over 255.
const TIER_NUM: [u32; TIERS] = [255, 96, 30];

/// Index 0 is the interior and stays black in every tier, so an interior cell
/// never flickers during a fade.
const fn tiered(base: &[[u8; 3]; BANDS]) -> [[u8; 3]; PAL_LEN] {
    let mut out = [[0u8; 3]; PAL_LEN];
    let mut t = 0;
    while t < TIERS {
        let mut i = 0;
        while i < BANDS {
            let mut k = 0;
            while k < 3 {
                out[1 + t * BANDS + i][k] = ((base[i][k] as u32 * TIER_NUM[t]) / 255) as u8;
                k += 1;
            }
            i += 1;
        }
        t += 1;
    }
    out
}

const PAL: [u32; PAL_LEN] = bake(&tiered(&BASE_RGB));

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// z^2 + c.
    Mandel,
    /// z^P + c. Each power is its own creature: 3 is three-lobed, 5 is a
    /// five-pointed star, and neither reads as "a Mandelbrot".
    Multi3,
    Multi4,
    Multi5,
    /// conj(z)^2 + c — the Tricorn/Mandelbar. The conjugate makes the map
    /// non-analytic, so the boundary grows spikes instead of filaments.
    Tricorn,
    /// The abs-variants. All five take z^2 + c and fold part of it through an
    /// absolute value; which part, and which sign, is the whole difference, and
    /// the five shapes have nothing in common to look at.
    Ship,
    Celtic,
    Perp,
    Buffalo,
    Heart,
    /// Julia, with a c that walks the main cardioid. See `julia_view`.
    Julia,
    /// z^2 + c + p*z_prev — the second-order term drags a curl behind every
    /// filament, which no first-order family does.
    Phoenix,
    /// Rational maps with a convergent attractor as well as an escape, so the
    /// picture has two kinds of exterior at once.
    Magnet1,
    Magnet2,
    /// Relaxed Newton with a c offset: Newton's basins wrapped around a
    /// Mandelbrot-shaped parameter plane.
    Nova,
    /// Newton's method on z^N - 1. N is the number of basins. 5 was here too
    /// and was cut: at a zoom the basin boundaries of 3, 4 and 5 are the same
    /// picture, and three of them is already one more than the difference
    /// carries.
    Newton3,
    Newton4,
    Newton8,
    /// Not escape-time at all: an exact self-similar membership test. The one
    /// family with hard edges and no smooth banding, and the only one whose
    /// zoom is scale-invariant rather than revealing.
    Sierpinski,
}

/// One fractal family: what to draw, where it is worth looking, and how fast
/// that view closes in.
struct Family {
    kind: Kind,
    /// Zoom target. Every one of these sits ON the boundary — a target inside
    /// the set zooms to a black screen, one outside zooms to flat colour, and
    /// both look like the renderer broke.
    ///
    /// Found by BOUNDARY DESCENT, not by eye — `descent::find_boundary_targets`
    /// splits a box into quadrants, renders the family over each, keeps the
    /// best few and halves, scoring on what the frame looks like rather than on
    /// the maths. That makes the point a boundary point at EVERY scale the zoom
    /// passes through, which is the property that matters: a coordinate that
    /// looks boundary-ish at 5x is usually deep inside the set at 500x.
    ///
    /// The descent is seeded from the SET's own centre, never from its previous
    /// answer. Feeding a result back in walks the target outwards a little each
    /// time, and four rounds of that put the Newton targets four units from the
    /// roots, where the opening frame shows no basins at all.
    cx: f64,
    cy: f64,
    /// Complex units across the panel at the start of a cycle.
    span: f64,
    /// Multiplier on the global zoom rate: the fraction of `MAX_OCTAVES` this
    /// family may close in by. Detail lives at a different scale in each of
    /// them, and past that scale the zoom outruns the iteration cap and the
    /// panel turns to confetti.
    ///
    /// Measured, by `descent::safe_octaves_per_family`: walk the family's own
    /// zoom half an octave at a time, stop at the FIRST frame that fails what
    /// `every_family_keeps_structure_on_screen_while_it_zooms` asserts, and
    /// take 0.9 of what is left. Three exceptions are set BELOW their measured
    /// limit by eye, and say so where they sit: Julia, Burning Ship, and the
    /// three abs-variants whose deep frames pass the test and still look like
    /// television static.
    rate: f64,
}

const FAMILIES: [Family; 19] = [
    Family {
        kind: Kind::Mandel,
        // Seahorse valley, the canonical deep-zoom coordinate — the one target
        // here that is famous rather than found.
        cx: -0.743_643_887_037_151,
        cy: 0.131_825_904_205_330,
        span: 3.2,
        rate: 0.9,
    },
    Family {
        kind: Kind::Multi3,
        cx: 0.201_500_684_022_903,
        cy: -0.902_711_123_228_073,
        span: 3.0,
        rate: 0.65,
    },
    Family {
        kind: Kind::Multi4,
        cx: -0.674_803_107_976_914,
        cy: -0.328_324_645_757_675,
        span: 3.0,
        rate: 0.9,
    },
    Family {
        kind: Kind::Multi5,
        cx: -0.780_537_039_041_519,
        cy: 0.540_116_339_921_951,
        span: 3.0,
        rate: 0.9,
    },
    Family {
        kind: Kind::Tricorn,
        cx: 0.676_844_406_127_930,
        cy: -1.027_708_053_588_867,
        span: 3.4,
        rate: 0.79,
    },
    Family {
        kind: Kind::Ship,
        // On the hull's edge above the main ship, and the one coordinate kept
        // from before the descent existed: it frames the recognisable ship, and
        // the descent's own answer does not.
        cx: -1.775,
        cy: -0.015_996_6,
        span: 3.2,
        // Measured at 0.47, which is what it was already hand-tuned to. Past
        // ~7 octaves the hull's fine structure falls below one cell and the
        // panel is confetti; the recognisable ship is the point of this family,
        // so it stops there.
        rate: 0.47,
    },
    Family {
        kind: Kind::Celtic,
        // Celtic, Perpendicular and Buffalo are the three set by eye rather
        // than by measurement: 1.0, 0.9 and 0.9 all hold the structure test,
        // and all three look like static for the last few seconds of a cycle.
        // The test cannot see the difference; a person can.
        cx: -0.762_307_071_685_791,
        cy: 0.251_916_408_538_818,
        span: 3.2,
        rate: 0.5,
    },
    Family {
        kind: Kind::Perp,
        cx: -0.403_985_595_703_125,
        cy: -0.668_341_064_453_125,
        span: 3.2,
        rate: 0.45,
    },
    Family {
        kind: Kind::Buffalo,
        cx: -0.818_426_513_671_875,
        cy: -0.071_282_958_984_375,
        span: 3.2,
        rate: 0.28,
    },
    Family {
        kind: Kind::Heart,
        cx: 0.092_512_989_044_190,
        cy: 0.617_150_974_273_682,
        span: 3.2,
        rate: 0.9,
    },
    Family {
        // Centre and span are placeholders: Julia re-centres itself every frame
        // on the repelling fixed point of its drifting c (see `julia_view`), so
        // the descent has nothing fixed to descend to.
        kind: Kind::Julia,
        cx: 0.0,
        cy: 0.0,
        span: 3.4,
        // Deliberately slow: Julia's detail is in the drifting `c`, and a fast
        // zoom on top of that reads as two effects fighting.
        rate: 0.35,
    },
    Family {
        kind: Kind::Phoenix,
        cx: 0.232_261_687_517_167,
        cy: -0.992_137_938_737_870,
        span: 3.0,
        rate: 0.9,
    },
    Family {
        kind: Kind::Magnet1,
        cx: 0.398_738_980_293_274,
        cy: 1.391_140_103_340_149,
        span: 4.0,
        rate: 0.9,
    },
    Family {
        kind: Kind::Magnet2,
        cx: 1.239_991_784_095_765,
        cy: 1.214_014_410_972_595,
        span: 4.0,
        rate: 0.9,
    },
    Family {
        kind: Kind::Nova,
        cx: -0.093_952_059_745_788,
        cy: -0.437_975_621_223_449,
        span: 2.4,
        rate: 0.9,
    },
    Family {
        kind: Kind::Newton3,
        cx: 0.351_110_833_590_618,
        cy: 0.200_167_765_383_336,
        span: 3.0,
        rate: 0.9,
    },
    Family {
        kind: Kind::Newton4,
        cx: -0.261_640_548_706_055,
        cy: 0.618_238_449_096_680,
        span: 3.0,
        rate: 0.9,
    },
    Family {
        kind: Kind::Newton8,
        cx: 0.870_969_772_338_867,
        cy: -0.233_797_073_364_258,
        span: 3.0,
        rate: 0.9,
    },
    Family {
        kind: Kind::Sierpinski,
        // The gasket is exactly self-similar, so any point of it works and the
        // descent has nothing to find. This is the corner where the three
        // sub-triangles of the unit square meet.
        cx: 0.5,
        cy: 0.5,
        span: 1.4,
        rate: 1.0,
    },
];

/// Roots of unity for the Newton families. One table per N rather than a prefix
/// of the longest: they are different sets of points, not nested.
const ROOTS3: [(f64, f64); 3] = [
    (1.0, 0.0),
    (-0.5, 0.866_025_403_784_439),
    (-0.5, -0.866_025_403_784_439),
];
const ROOTS4: [(f64, f64); 4] = [(1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)];
const ROOTS8: [(f64, f64); 8] = [
    (1.0, 0.0),
    (FRAC_1_SQRT_2, FRAC_1_SQRT_2),
    (0.0, 1.0),
    (-FRAC_1_SQRT_2, FRAC_1_SQRT_2),
    (-1.0, 0.0),
    (-FRAC_1_SQRT_2, -FRAC_1_SQRT_2),
    (0.0, -1.0),
    (FRAC_1_SQRT_2, -FRAC_1_SQRT_2),
];

/// Squared step length at which Newton's iteration counts as converged.
const NEWTON_EPS: f64 = 1e-8;

/// Phoenix's fixed parameters. `c` alone would give an ordinary Julia set; `p`
/// is what puts the curl in it.
const PHOENIX_C: f64 = 0.566_7;
const PHOENIX_P: f64 = -0.5;

/// Bailout for the magnet families. Their orbits linger near the attractor at 1
/// for a long time before leaving, so an ordinary radius-2 bailout cuts the
/// picture off before any of the structure appears.
const MAGNET_BAIL: f64 = 1e4;

/// Squared distance at which a magnet orbit counts as having reached the
/// attractor at 1. Loose on purpose: the approach is geometric, so a tighter
/// epsilon buys a few more identical-looking bands at several iterations of a
/// division each, and this is the second most expensive family in the set.
const MAGNET_EPS: f64 = 1e-8;

/// Longest cycle `FRACTAL_SECONDS` accepts. See where it is read.
const MAX_SECONDS: i64 = 45;

/// How far a cycle may close in, whatever the rate and length asking for it.
/// Measured: 16 octaves (45s at the default 22%/s) keeps structure on every
/// frame of a full rotation; more puts frames at >=90% one colour. The wall is
/// `FRACTAL_ITER_MAX`, not f64 — raising the cap moves it, at a proportional
/// cost per frame.
const MAX_OCTAVES: f64 = 16.0;

/// The per-second zoom, held to `MAX_OCTAVES` over a `cycle`-second run. One
/// definition because the structure test has to reproduce what `new` does with
/// the knobs, and a second copy of this would drift away from it.
fn capped_zoom(zoom_sec: f64, cycle: f64) -> f64 {
    zoom_sec.max((-MAX_OCTAVES / cycle).exp2())
}

/// z^P by square-and-multiply. `P` is const, so the whole ladder unrolls into
/// straight-line code with the leading multiply by 1 folded away: z^8 costs
/// three squarings, not seven multiplies. That difference is the whole reason
/// the high-order families are affordable in this loop.
#[inline(always)]
fn cpow<const P: u32>(zx: f64, zy: f64) -> (f64, f64) {
    let (mut rx, mut ry) = (1.0f64, 0.0f64);
    let (mut bx, mut by) = (zx, zy);
    let mut e = P;
    while e > 0 {
        if e & 1 == 1 {
            let t = rx * bx - ry * by;
            ry = rx * by + ry * bx;
            rx = t;
        }
        e >>= 1;
        if e > 0 {
            let t = bx * bx - by * by;
            by *= 2.0 * bx;
            bx = t;
        }
    }
    (rx, ry)
}

#[inline]
fn band(i: u32) -> u16 {
    1 + (i % BANDS as u32) as u16
}

#[inline]
fn mandel(x: f64, y: f64, max: u32) -> u16 {
    let (mut zx, mut zy) = (0.0f64, 0.0f64);
    for i in 0..max {
        let (x2, y2) = (zx * zx, zy * zy);
        if x2 + y2 > 4.0 {
            return band(i);
        }
        zy = 2.0 * zx * zy + y;
        zx = x2 - y2 + x;
    }
    0
}

/// z^P + c. `P` is a const parameter so `cpow` unrolls into the caller — a
/// runtime `p` here would put a loop with a branch inside the innermost loop.
#[inline]
fn multibrot<const P: u32>(x: f64, y: f64, max: u32) -> u16 {
    let (mut zx, mut zy) = (0.0f64, 0.0f64);
    for i in 0..max {
        if zx * zx + zy * zy > 4.0 {
            return band(i);
        }
        let (px, py) = cpow::<P>(zx, zy);
        zx = px + x;
        zy = py + y;
    }
    0
}

/// conj(z)^2 + c. One sign, and the cardioid becomes a three-cornered star.
#[inline]
fn tricorn(x: f64, y: f64, max: u32) -> u16 {
    let (mut zx, mut zy) = (0.0f64, 0.0f64);
    for i in 0..max {
        let (x2, y2) = (zx * zx, zy * zy);
        if x2 + y2 > 4.0 {
            return band(i);
        }
        zy = -2.0 * zx * zy + y;
        zx = x2 - y2 + x;
    }
    0
}

/// Burning Ship: the absolute value goes on the cross term, and that alone
/// breaks the symmetry into a hull.
#[inline]
fn ship(x: f64, y: f64, max: u32) -> u16 {
    let (mut zx, mut zy) = (0.0f64, 0.0f64);
    for i in 0..max {
        let (x2, y2) = (zx * zx, zy * zy);
        if x2 + y2 > 4.0 {
            return band(i);
        }
        zy = (2.0 * zx * zy).abs() + y;
        zx = x2 - y2 + x;
    }
    0
}

/// Celtic: the abs goes on the real part instead, flattening the cardioid into
/// a shield.
#[inline]
fn celtic(x: f64, y: f64, max: u32) -> u16 {
    let (mut zx, mut zy) = (0.0f64, 0.0f64);
    for i in 0..max {
        let (x2, y2) = (zx * zx, zy * zy);
        if x2 + y2 > 4.0 {
            return band(i);
        }
        let nx = (x2 - y2).abs() + x;
        zy = 2.0 * zx * zy + y;
        zx = nx;
    }
    0
}

/// Perpendicular: abs on z's real part, with the cross term negated.
#[inline]
fn perpendicular(x: f64, y: f64, max: u32) -> u16 {
    let (mut zx, mut zy) = (0.0f64, 0.0f64);
    for i in 0..max {
        let (x2, y2) = (zx * zx, zy * zy);
        if x2 + y2 > 4.0 {
            return band(i);
        }
        zy = -2.0 * zx.abs() * zy + y;
        zx = x2 - y2 + x;
    }
    0
}

/// Buffalo: both parts folded. Named for the silhouette.
#[inline]
fn buffalo(x: f64, y: f64, max: u32) -> u16 {
    let (mut zx, mut zy) = (0.0f64, 0.0f64);
    for i in 0..max {
        let (x2, y2) = (zx * zx, zy * zy);
        if x2 + y2 > 4.0 {
            return band(i);
        }
        let nx = (x2 - y2).abs() + x;
        zy = -(2.0 * zx * zy).abs() + y;
        zx = nx;
    }
    0
}

/// Heart: abs on z's real part, keeping the sign of the cross term.
#[inline]
fn heart(x: f64, y: f64, max: u32) -> u16 {
    let (mut zx, mut zy) = (0.0f64, 0.0f64);
    for i in 0..max {
        let (x2, y2) = (zx * zx, zy * zy);
        if x2 + y2 > 4.0 {
            return band(i);
        }
        zy = 2.0 * zx * zy.abs() + y;
        zx = x2 - y2 + x;
    }
    0
}

#[inline]
fn julia(x: f64, y: f64, cx: f64, cy: f64, max: u32) -> u16 {
    let (mut zx, mut zy) = (x, y);
    for i in 0..max {
        let (x2, y2) = (zx * zx, zy * zy);
        if x2 + y2 > 4.0 {
            return band(i);
        }
        zy = 2.0 * zx * zy + cy;
        zx = x2 - y2 + cx;
    }
    0
}

/// Phoenix: `z_{n+1} = z_n^2 + c + p*z_{n-1}`, drawn in the z plane. The
/// one-step memory is what separates it from every other family here.
#[inline]
fn phoenix(x: f64, y: f64, max: u32) -> u16 {
    let (mut zx, mut zy) = (x, y);
    let (mut px, mut py) = (0.0f64, 0.0f64);
    for i in 0..max {
        let (x2, y2) = (zx * zx, zy * zy);
        if x2 + y2 > 4.0 {
            return band(i);
        }
        let nx = x2 - y2 + PHOENIX_C + PHOENIX_P * px;
        let ny = 2.0 * zx * zy + PHOENIX_P * py;
        (px, py) = (zx, zy);
        (zx, zy) = (nx, ny);
    }
    0
}

/// Magnet 1: `z -> ((z^2 + c - 1)/(2z + c - 2))^2`, from the Ising-model
/// renormalisation. It has an attracting fixed point at 1 as well as an escape,
/// so both convergence and divergence are colourable events and only a genuine
/// stall is interior.
#[inline]
fn magnet1(x: f64, y: f64, max: u32) -> u16 {
    let (mut zx, mut zy) = (0.0f64, 0.0f64);
    for i in 0..max {
        let (nx, ny) = (zx * zx - zy * zy + x - 1.0, 2.0 * zx * zy + y);
        let (dx, dy) = (2.0 * zx + x - 2.0, 2.0 * zy + y);
        let den = dx * dx + dy * dy;
        let Some(inv) = (den > 1e-30).then(|| 1.0 / den) else {
            return band(i);
        };
        let (qx, qy) = ((nx * dx + ny * dy) * inv, (ny * dx - nx * dy) * inv);
        (zx, zy) = (qx * qx - qy * qy, 2.0 * qx * qy);
        let (ex, ey) = (zx - 1.0, zy);
        if zx * zx + zy * zy > MAGNET_BAIL || ex * ex + ey * ey < MAGNET_EPS {
            return band(i);
        }
    }
    0
}

/// Magnet 2: the same construction one order up. Its boundary carries a
/// cardioid-and-bulb skeleton that Magnet 1's does not.
#[inline]
fn magnet2(x: f64, y: f64, max: u32) -> u16 {
    let (ax, ay) = (x - 1.0, y);
    let (bx, by) = (x - 2.0, y);
    // (c-1)(c-2), constant for the whole orbit.
    let (kx, ky) = (ax * bx - ay * by, ax * by + ay * bx);
    let (mut zx, mut zy) = (0.0f64, 0.0f64);
    for i in 0..max {
        let (z2x, z2y) = (zx * zx - zy * zy, 2.0 * zx * zy);
        let (z3x, z3y) = (z2x * zx - z2y * zy, z2x * zy + z2y * zx);
        let nx = z3x + 3.0 * (ax * zx - ay * zy) + kx;
        let ny = z3y + 3.0 * (ax * zy + ay * zx) + ky;
        let dx = 3.0 * z2x + 3.0 * (bx * zx - by * zy) + kx + 1.0;
        let dy = 3.0 * z2y + 3.0 * (bx * zy + by * zx) + ky;
        let den = dx * dx + dy * dy;
        let Some(inv) = (den > 1e-30).then(|| 1.0 / den) else {
            return band(i);
        };
        let (qx, qy) = ((nx * dx + ny * dy) * inv, (ny * dx - nx * dy) * inv);
        (zx, zy) = (qx * qx - qy * qy, 2.0 * qx * qy);
        let (ex, ey) = (zx - 1.0, zy);
        if zx * zx + zy * zy > MAGNET_BAIL || ex * ex + ey * ey < MAGNET_EPS {
            return band(i);
        }
    }
    0
}

/// Nova: `z -> z - (z^3 - 1)/(3 z^2) + c`, started at the critical point, over
/// c. Newton's basins with a parameter plane wrapped around them, so it reads
/// as a Mandelbrot made out of Newton.
#[inline]
fn nova(x: f64, y: f64, max: u32) -> u16 {
    let (mut zx, mut zy) = (1.0f64, 0.0f64);
    for i in 0..max {
        let (z2x, z2y) = (zx * zx - zy * zy, 2.0 * zx * zy);
        let (z3x, z3y) = (z2x * zx - z2y * zy, z2x * zy + z2y * zx);
        let (dx, dy) = (3.0 * z2x, 3.0 * z2y);
        let den = dx * dx + dy * dy;
        let Some(inv) = (den > 1e-30).then(|| 1.0 / den) else {
            return 0;
        };
        let (nx, ny) = (z3x - 1.0, z3y);
        let (sx, sy) = ((nx * dx + ny * dy) * inv, (ny * dx - nx * dy) * inv);
        // Convergence is tested on the WHOLE update, Newton step plus c. On the
        // step alone every cell returns at i = 0: z0 is the critical point 1,
        // where z^3 - 1 vanishes and the step is exactly zero.
        let (ux, uy) = (x - sx, y - sy);
        zx += ux;
        zy += uy;
        if ux * ux + uy * uy < NEWTON_EPS || zx * zx + zy * zy > 1e6 {
            return band(i);
        }
    }
    0
}

/// Newton's method on `z^N - 1`: which root it falls into picks the hue band,
/// how long it took picks the shade within it.
///
/// Convergence is tested on the STEP, not against the root table, so the inner
/// loop is N complex multiplies and one compare however many roots there are;
/// `roots` is touched once per cell, on the way out.
///
/// `N` and `M` are both const because `N - 1` in a const-generic argument needs
/// an unstable feature; every call site passes the pair. `roots` stays a slice
/// because an array length tied to a const parameter is not expressible here
/// either, and it costs nothing outside the loop.
#[inline]
fn newton_n<const N: u32, const M: u32>(x: f64, y: f64, max: u32, roots: &[(f64, f64)]) -> u16 {
    // Shades per basin, so N basins fill the ramp exactly once whatever N is.
    // Modulo, not a clamp: at a deep zoom every cell is near the boundary and
    // takes many steps, so a clamp collapses each basin to one flat colour.
    // Cycling keeps the contour rings.
    let stride = BANDS as u32 / N;
    let (mut zx, mut zy) = (x, y);
    for i in 0..max {
        // p = z^(N-1), q = z^N.
        let (px, py) = cpow::<M>(zx, zy);
        let (qx, qy) = (px * zx - py * zy, px * zy + py * zx);
        let (dx, dy) = (N as f64 * px, N as f64 * py);
        let den = dx * dx + dy * dy;
        let Some(inv) = (den > 1e-30).then(|| 1.0 / den) else {
            return 0;
        };
        let (nx, ny) = (qx - 1.0, qy);
        let (sx, sy) = ((nx * dx + ny * dy) * inv, (ny * dx - nx * dy) * inv);
        zx -= sx;
        zy -= sy;
        if sx * sx + sy * sy < NEWTON_EPS {
            for (r, &(rx, ry)) in roots.iter().enumerate() {
                let (ex, ey) = (zx - rx, zy - ry);
                if ex * ex + ey * ey < 1e-6 {
                    return 1 + (r as u32 * stride + i % stride) as u16;
                }
            }
            return 0;
        }
    }
    0
}

/// Sierpinski gasket as a membership test on the unit square: fold the point
/// towards a corner and double it. A point of the gasket never leaves the
/// square; everything else does, and WHEN it leaves is the band.
///
/// Exactly self-similar, so unlike every other family here its zoom reveals
/// nothing new — it returns to the same picture every octave, which is the
/// reason to have it in the rotation.
#[inline]
fn sierpinski(x: f64, y: f64, max: u32) -> u16 {
    let (mut u, mut v) = (x, y);
    for i in 0..max {
        if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
            return band(i);
        }
        if v > 0.5 {
            v = 2.0 * v - 1.0;
            u *= 2.0;
        } else if u > 0.5 {
            u = 2.0 * u - 1.0;
            v *= 2.0;
        } else {
            u *= 2.0;
            v *= 2.0;
        }
    }
    0
}

/// The drifting c, and the point to zoom into for it.
///
/// c walks just inside the main cardioid (`c = mu/2 - mu^2/4`, `|mu| < 1`), so
/// the Julia set stays CONNECTED — a c outside the Mandelbrot set is Cantor
/// dust, which at cell resolution is a flat rectangle of one colour.
///
/// The centre is the repelling fixed point `beta = (1 + sqrt(1-4c))/2`, which
/// lies on the Julia set for every c. That is the whole trick: the set's shape
/// changes as c drifts, so no fixed coordinate stays on the boundary, and a
/// zoom target off the boundary lands in flat interior or flat exterior.
#[inline]
fn julia_view(theta: f64) -> (f64, f64, f64, f64) {
    let (s, c) = theta.sin_cos();
    let (mx, my) = (0.985 * c, 0.985 * s);
    let (cx, cy) = (
        mx * 0.5 - (mx * mx - my * my) * 0.25,
        my * 0.5 - (2.0 * mx * my) * 0.25,
    );
    // sqrt(1 - 4c), principal branch.
    let (wx, wy) = (1.0 - 4.0 * cx, -4.0 * cy);
    let m = (wx * wx + wy * wy).sqrt();
    let rx = ((m + wx) * 0.5).max(0.0).sqrt();
    let ry = ((m - wx) * 0.5).max(0.0).sqrt() * if wy < 0.0 { -1.0 } else { 1.0 };
    (cx, cy, (1.0 + rx) * 0.5, ry * 0.5)
}

/// Where the camera is: centre, and complex units per cell.
struct View {
    cx: f64,
    cy: f64,
    step: f64,
}

/// Map the grid onto the complex plane and fill `shade`. Generic so the
/// family's iteration monomorphises into the loop: a `&dyn Fn` here would be
/// one indirect call per cell, which is the thing the trait doc bans.
#[inline]
fn scan<F: Fn(f64, f64) -> u16>(shade: &mut [u16], cols: usize, rows: usize, v: &View, f: F) {
    let (hw, hh) = (cols as f64 * 0.5, rows as f64 * 0.5);
    for cy in 0..rows {
        let y = v.cy + (cy as f64 - hh + 0.5) * v.step;
        let row = &mut shade[cy * cols..][..cols];
        for (cx, out) in row.iter_mut().enumerate() {
            *out = f(v.cx + (cx as f64 - hw + 0.5) * v.step, y);
        }
    }
}

pub struct Fractal {
    cols: usize,
    rows: usize,
    shade: Vec<u16>,
    grid: Grid,
    view: View,
    /// Index into FAMILIES.
    fam: usize,
    /// Seconds into the current family's cycle.
    t: f64,
    dt: f64,
    cycle: f64,
    fade: f64,
    /// Palette tier for this frame: 0 full, TIERS-1 nearly dark.
    tier: usize,
    /// Per-frame zoom multiplier for the current family.
    step_mul: f64,
    /// Zoom rate as a per-second scale multiplier, before the family's factor.
    zoom_sec: f64,
    iter_base: u32,
    iter_cap: u32,
    iter: u32,
    /// Julia's c, and how fast its argument turns.
    jc: (f64, f64),
    jdrift: f64,
    jtheta: f64,
}

impl Fractal {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        let cell = env_num(&["FRACTAL_CELL"], 20, 4, 64) as usize;
        let grid = Grid::new(panel, cell, cell);
        let (cols, rows) = (grid.cols(), grid.rows());
        // 45, not the 600 this used to document. What breaks the saver is
        // DEPTH, and `MAX_OCTAVES` is what bounds that wherever the knobs land;
        // 45 is simply the longest cycle the structure test actually runs, and
        // nothing here is claimed past what is tested. 600 was claimed and
        // never exercised: three frames in four came out >=90% one colour.
        let cycle = env_num(&["FRACTAL_SECONDS"], 26, 5, MAX_SECONDS) as f64;
        let fade = env_num(&["FRACTAL_FADE_MS"], 900, 0, 5000) as f64 / 1000.0;
        // Percent of the view width the zoom eats each second. 22% compounds to
        // roughly 640x across a 26s cycle: a steady crawl rather than a dive.
        let zoom_sec = 1.0 - env_num(&["FRACTAL_ZOOM_PCT"], 22, 1, 90) as f64 / 100.0;
        // Depth is rate x seconds, so bounding each knob separately cannot
        // express the thing that actually breaks: 90%/s for 45s is 150 octaves,
        // hundreds past what the iteration cap resolves, and the panel is one
        // flat colour. Bound the product instead — a cycle may not close in by
        // more than MAX_OCTAVES, whatever rate asked for it.
        let zoom_sec = capped_zoom(zoom_sec, cycle);
        let jdrift = env_num(&["FRACTAL_JULIA_DRIFT"], 90, 0, 2000) as f64 / 1000.0;
        let mut f = Self {
            cols,
            rows,
            shade: vec![0u16; cols * rows],
            grid,
            view: View {
                cx: 0.0,
                cy: 0.0,
                step: 1.0,
            },
            fam: 0,
            // Start past the fade-in: there is no previous image for frame 0
            // to dip out of, and a screensaver whose first second is black
            // reads as one that failed to start.
            t: fade.min(cycle * 0.4),
            dt: 1.0 / fps as f64,
            cycle,
            // A fade longer than half a cycle would never reach full
            // brightness, which reads as a broken palette rather than a fade.
            fade: fade.min(cycle * 0.4),
            tier: TIERS - 1,
            step_mul: 1.0,
            zoom_sec,
            iter_base: env_num(&["FRACTAL_ITER"], 40, 16, 512) as u32,
            iter_cap: env_num(&["FRACTAL_ITER_MAX"], 110, 32, 2000) as u32,
            iter: 0,
            jc: (0.0, 0.0),
            jdrift,
            jtheta: 2.2,
        };
        f.begin();
        f
    }

    /// Reset the camera onto the current family's target.
    fn begin(&mut self) {
        let fam = &FAMILIES[self.fam];
        self.view.cx = fam.cx;
        self.view.cy = fam.cy;
        self.view.step = fam.span / self.cols as f64;
        self.step_mul = self.zoom_sec.powf(fam.rate * self.dt);
        self.iter = self.iter_base;
    }

    fn advance(&mut self) {
        self.t += self.dt;
        if self.t >= self.cycle {
            self.t = 0.0;
            self.fam = (self.fam + 1) % FAMILIES.len();
            self.begin();
        } else {
            self.view.step *= self.step_mul;
        }

        // Detail per cell is constant, so iteration depth has to track zoom
        // depth or a deep view flattens into one band.
        let span0 = FAMILIES[self.fam].span / self.cols as f64;
        let octaves = (span0 / self.view.step).log2().max(0.0);
        self.iter = (self.iter_base + (octaves * 8.0) as u32).min(self.iter_cap);

        self.jtheta += self.jdrift * self.dt;
        let (jx, jy, bx, by) = julia_view(self.jtheta);
        self.jc = (jx, jy);
        if FAMILIES[self.fam].kind == Kind::Julia {
            self.view.cx = bx;
            self.view.cy = by;
        }

        let edge = self.t.min(self.cycle - self.t);
        self.tier = if self.fade <= 0.0 || edge >= self.fade {
            0
        } else {
            TIERS - 1 - ((edge / self.fade) * TIERS as f64) as usize
        };
    }

    fn scan_family(&mut self) {
        let (shade, cols, rows, view, iter) = (
            &mut self.shade[..],
            self.cols,
            self.rows,
            &self.view,
            self.iter,
        );
        // The convergent families settle or blow up in a handful of steps or
        // never; running them to the escape-time cap buys boundary noise at
        // several times the price, and they are the expensive ones. Newton is
        // quadratic and needs the fewest; the magnets linger near their
        // attractor and need the most.
        let n = iter.min(24);
        let nov = iter.min(40);
        let mag = iter.min(40);
        let hi = iter.min(64);
        // One match per FRAME, not per cell: each arm monomorphises `scan`.
        match FAMILIES[self.fam].kind {
            Kind::Mandel => scan(shade, cols, rows, view, |x, y| mandel(x, y, iter)),
            Kind::Multi3 => scan(shade, cols, rows, view, |x, y| multibrot::<3>(x, y, iter)),
            Kind::Multi4 => scan(shade, cols, rows, view, |x, y| multibrot::<4>(x, y, hi)),
            Kind::Multi5 => scan(shade, cols, rows, view, |x, y| multibrot::<5>(x, y, hi)),
            Kind::Tricorn => scan(shade, cols, rows, view, |x, y| tricorn(x, y, iter)),
            Kind::Ship => scan(shade, cols, rows, view, |x, y| ship(x, y, iter)),
            Kind::Celtic => scan(shade, cols, rows, view, |x, y| celtic(x, y, iter)),
            Kind::Perp => scan(shade, cols, rows, view, |x, y| perpendicular(x, y, iter)),
            Kind::Buffalo => scan(shade, cols, rows, view, |x, y| buffalo(x, y, iter)),
            Kind::Heart => scan(shade, cols, rows, view, |x, y| heart(x, y, iter)),
            Kind::Julia => {
                let (jx, jy) = self.jc;
                scan(shade, cols, rows, view, |x, y| julia(x, y, jx, jy, iter))
            }
            Kind::Phoenix => scan(shade, cols, rows, view, |x, y| phoenix(x, y, iter)),
            Kind::Magnet1 => scan(shade, cols, rows, view, |x, y| magnet1(x, y, mag)),
            Kind::Magnet2 => scan(shade, cols, rows, view, |x, y| magnet2(x, y, mag)),
            Kind::Nova => scan(shade, cols, rows, view, |x, y| nova(x, y, nov)),
            Kind::Newton3 => scan(shade, cols, rows, view, |x, y| {
                newton_n::<3, 2>(x, y, n, &ROOTS3)
            }),
            Kind::Newton4 => scan(shade, cols, rows, view, |x, y| {
                newton_n::<4, 3>(x, y, n, &ROOTS4)
            }),
            Kind::Newton8 => scan(shade, cols, rows, view, |x, y| {
                newton_n::<8, 7>(x, y, n, &ROOTS8)
            }),
            Kind::Sierpinski => scan(shade, cols, rows, view, |x, y| sierpinski(x, y, iter)),
        }
    }
}

impl Saver for Fractal {
    fn render(&mut self, s: &mut Surface<'_>) {
        self.advance();
        self.scan_family();
        let (grid, shade, cols) = (&mut self.grid, &self.shade[..], self.cols);
        let off = (self.tier * BANDS) as u16;
        grid.fill(|cx, cy| {
            let b = shade[cy * cols + cx];
            Cell::new(font::SOLID, if b == 0 { 0 } else { b + off })
        });
        grid.flush(s, &PAL);
    }

    fn name(&self) -> &'static str {
        "fractal"
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

    /// 1070 is deliberately NOT a multiple of the 20px cell: the 10-line strip
    /// below the last cell row is the "error line" frame 0 has to cover.
    fn panel() -> Panel {
        Panel::new(1920, 1070, 1920)
    }

    #[test]
    fn frame_zero_paints_the_whole_panel() {
        let p = panel();
        let mut f = Fractal::new(&p, 15);
        let mut buf = vec![0u32; p.buf_len()];
        let d = saver::frame(&mut f, &mut buf, &p);
        assert_eq!(d.rows(), p.h, "frame 0 must paint the whole panel");
        // Without this a saver that reported a black rectangle would pass.
        assert!(
            buf.iter().filter(|&&px| px != 0).count() > 10_000,
            "frame 0 painted nothing"
        );
    }

    /// The "screen went blank" bug class: pixels written but never reported.
    #[test]
    fn damage_covers_every_changed_scanline() {
        let p = panel();
        let mut f = Fractal::new(&p, 15);
        let mut buf = vec![0u32; p.buf_len()];
        let mut prev = buf.clone();
        saver::frame(&mut f, &mut buf, &p);

        let mut changed_total = 0;
        for n in 1..30 {
            prev.copy_from_slice(&buf);
            let d = saver::frame(&mut f, &mut buf, &p);
            for y in 0..p.h {
                if prev[y * p.w..][..p.w] == buf[y * p.w..][..p.w] {
                    continue;
                }
                changed_total += 1;
                assert!(
                    d.runs()
                        .iter()
                        .any(|&(a, b)| y as u16 >= a && (y as u16) < b),
                    "frame {n}: scanline {y} changed but was not reported"
                );
            }
        }
        // Otherwise the loop above proves nothing: a saver drawing a still
        // image would satisfy it vacuously.
        assert!(
            changed_total > 10_000,
            "only {changed_total} scanlines ever changed; the zoom is not moving"
        );
    }

    // Allocations on THIS thread, so tests running in parallel do not see each
    // other's traffic. A `Cell<usize>` has no destructor, so `with` cannot fail
    // during teardown, and `const` init means the key itself never allocates —
    // both of which an allocator hook has to be sure of.
    thread_local! {
        static ALLOCS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    /// Counting global allocator. This is crate-wide — one test binary, one
    /// allocator — and it earns that: the buffer-identity check below is blind
    /// to a scratch `Vec` allocated and dropped inside `render`, which is the
    /// commonest way this frame loop would start allocating.
    struct Counting;

    unsafe impl std::alloc::GlobalAlloc for Counting {
        unsafe fn alloc(&self, l: std::alloc::Layout) -> *mut u8 {
            ALLOCS.with(|n| n.set(n.get() + 1));
            std::alloc::System.alloc(l)
        }
        unsafe fn alloc_zeroed(&self, l: std::alloc::Layout) -> *mut u8 {
            ALLOCS.with(|n| n.set(n.get() + 1));
            std::alloc::System.alloc_zeroed(l)
        }
        unsafe fn realloc(&self, p: *mut u8, l: std::alloc::Layout, new: usize) -> *mut u8 {
            ALLOCS.with(|n| n.set(n.get() + 1));
            std::alloc::System.realloc(p, l, new)
        }
        unsafe fn dealloc(&self, p: *mut u8, l: std::alloc::Layout) {
            std::alloc::System.dealloc(p, l)
        }
    }

    #[global_allocator]
    static COUNTING: Counting = Counting;

    /// `render` owns no per-frame collection — `shade` is sized in `new`, only
    /// ever written through by index, and `Grid::fill` writes in place.
    ///
    /// Two checks, because neither is enough alone. The COUNTER catches any
    /// allocation at all, including a scratch `Vec` that is allocated and
    /// dropped inside the call and leaves no trace behind it. The buffer
    /// ADDRESS catches the one thing a counter would miss if the allocator were
    /// ever swapped out from under it: `shade = vec![...]` in the frame loop
    /// keeps both length and capacity, so those two alone would pass it.
    #[test]
    fn render_never_allocates() {
        let p = Panel::new(200, 150, 200);
        let mut f = Fractal::new(&p, 15);
        let mut buf = vec![0u32; p.buf_len()];
        let (len, cap, at) = (f.shade.len(), f.shade.capacity(), f.shade.as_ptr());
        assert!(len > 0, "nothing was reserved for the frame loop");
        let cells = f.grid.cells().len();
        // Non-vacuous: the counter is wired up and does count.
        let warm = ALLOCS.with(|n| n.get());
        drop(vec![0u8; 8]);
        assert!(
            ALLOCS.with(|n| n.get()) > warm,
            "the counter is not counting"
        );
        // Enough frames to cross a family boundary at this panel's cycle, so
        // `begin` is on the path too — that is where a rebuild would hide.
        for _ in 0..20_000 {
            let before = ALLOCS.with(|n| n.get());
            saver::frame(&mut f, &mut buf, &p);
            assert_eq!(
                ALLOCS.with(|n| n.get()),
                before,
                "the render path allocated"
            );
            assert_eq!(
                (f.shade.len(), f.shade.capacity(), f.shade.as_ptr()),
                (len, cap, at),
                "`shade` moved: the render path allocated"
            );
            assert_eq!(f.grid.cells().len(), cells, "the grid was reallocated");
        }
    }

    /// What this saver IS: a rotation of families, each closing in on a point
    /// that keeps structure on screen.
    ///
    /// The failure this catches is the one that actually happened: a zoom
    /// target that is not on the set's boundary. The iteration is still
    /// correct, the zoom is still smooth, every other test passes — and the
    /// panel shows one flat rectangle of colour. So the invariant is stated on
    /// the drawn cells, not on the maths: no single colour may own the frame,
    /// and the frame may not collapse to a handful of colours.
    ///
    /// Run at the SHIPPED panel and at the DEEPEST cycle the knobs allow, which
    /// is what makes any of it load-bearing. At the default 26s a target five
    /// orders of magnitude off still shows structure — the old 640x360, 26s
    /// version of this test passed with Mandelbrot's cx moved to -0.74. The
    /// deepest zoom is where a wrong target has already left the boundary. The
    /// trailing digits of these coordinates are provenance, not verified
    /// precision.
    ///
    /// Three bounds, because a frame has three ways to stop being a picture
    /// and no one number sees all of them:
    ///
    /// * `top < 0.9`  — not one flat colour. Cannot be tightened: a large
    ///   interior legitimately owns that much at the deepest point of a cycle.
    /// * `lit >= 8`   — not a two-colour field. This one has margin; it
    ///   collapses the moment a target leaves the boundary.
    /// * `coh >= 0.3` — not CONFETTI. Adjacent cells sharing a colour is the
    ///   only one of the three that can see a frame where the zoom has outrun
    ///   the iteration: that frame has every colour on it and no shape, so it
    ///   sails past the other two. It is also the only thing holding the
    ///   per-family `rate` down — without it, every rate here could be 1.0 and
    ///   this test would not notice. A confetti frame scores under 0.2.
    #[test]
    fn every_family_keeps_structure_on_screen_while_it_zooms() {
        // The default rate and the fastest `FRACTAL_ZOOM_PCT` allows. The
        // second is the case `MAX_OCTAVES` exists for: without the clamp it is
        // 150 octaves and a blank panel, and with it the two runs are the same
        // depth, which is the claim being made.
        for pct in [22.0f64, 90.0] {
            structure_holds_for(MAX_SECONDS as f64, pct);
        }
    }

    fn structure_holds_for(cycle: f64, zoom_pct: f64) {
        let p = Panel::new(1920, 1080, 1920);
        let fps = 15u32;
        let mut f = Fractal::new(&p, fps);
        let mut buf = vec![0u32; p.buf_len()];
        // Set here rather than through the env: the whole test binary shares
        // one environment and other tests build this saver concurrently.
        f.cycle = cycle;
        f.fade = f.fade.min(f.cycle * 0.4);
        f.t = f.fade;
        f.zoom_sec = capped_zoom(1.0 - zoom_pct / 100.0, f.cycle);
        f.begin();
        let frames = (f.cycle * FAMILIES.len() as f64 * fps as f64) as usize;

        let mut seen = [false; FAMILIES.len()];
        let mut worst = 0.0f64;
        let mut worst_coh = 1.0f64;
        let mut last_step = f64::MAX;
        let mut fam = usize::MAX;
        let mut hist = [0u32; PAL_LEN];

        for n in 0..frames {
            saver::frame(&mut f, &mut buf, &p);
            seen[f.fam] = true;

            if f.fam == fam {
                assert!(
                    f.view.step < last_step,
                    "frame {n}: the view stopped closing in ({last_step} -> {})",
                    f.view.step
                );
            }
            (fam, last_step) = (f.fam, f.view.step);

            hist.fill(0);
            for c in f.grid.cells() {
                hist[c.colour()] += 1;
            }
            let cells = f.grid.cells().len() as f64;
            let top = *hist.iter().max().unwrap() as f64 / cells;
            let lit = hist.iter().filter(|&&n| n > 0).count();
            let mut same = 0usize;
            let mut pairs = 0usize;
            for row in f.grid.cells().chunks(f.cols) {
                for w in row.windows(2) {
                    pairs += 1;
                    same += usize::from(w[0].colour() == w[1].colour());
                }
            }
            let coh = same as f64 / pairs as f64;
            worst = worst.max(top);
            worst_coh = worst_coh.min(coh);
            assert!(
                top < 0.9 && lit >= 8 && coh >= 0.3,
                "{zoom_pct}%/s, frame {n} ({:?}): {:.0}% of the panel is one colour \
                 across {lit} colours, coherence {coh:.2} — the zoom has left the \
                 boundary or outrun the iteration",
                FAMILIES[f.fam].kind,
                top * 100.0
            );
        }

        assert!(seen.iter().all(|&s| s), "the rotation skipped a family");
        // The bound is not vacuous: frames with a large interior really do
        // approach it.
        assert!(
            worst > 0.5,
            "the busiest frame was only {:.0}% one colour; the 90% bound is \
             nowhere near the data and proves nothing",
            worst * 100.0
        );
        // Same argument for the coherence bound: if the noisiest frame of a
        // whole rotation is nowhere near 0.4, the bound is decorative.
        assert!(
            worst_coh < 0.6,
            "the noisiest frame still had coherence {worst_coh:.2}; the 0.3 \
             bound is nowhere near the data and proves nothing"
        );
    }
}

#[cfg(test)]
mod descent {
    use super::*;

    /// Render the family over one box and report what the frame LOOKS like:
    /// distinct colours, coherence, and the mean and spread of its luminance.
    ///
    /// Three numbers because each catches a different way of being a bad view,
    /// and each was reached by looking at what the previous one shipped:
    ///
    /// * COLOUR COUNT alone picks confetti — at cell resolution a chaotic
    ///   region has every colour and no shape.
    /// * COHERENCE (adjacent cells sharing a colour) fixes that and then picks
    ///   an empty frame, because a flat field is perfectly coherent.
    /// * LUMINANCE mean and spread are what "empty" actually means. The palette
    ///   has three near-black stops, so a frame can be 90% dark and still count
    ///   thirty distinct colours; only the pixels say so.
    fn score(f: &mut Fractal, cx: f64, cy: f64, r: f64, iter: u32) -> Look {
        f.view.cx = cx;
        f.view.cy = cy;
        f.view.step = 2.0 * r / f.cols as f64;
        f.iter = iter;
        f.scan_family();
        let mut hist = [0u32; PAL_LEN];
        for &v in &f.shade {
            hist[v as usize] += 1;
        }
        let mut same = 0usize;
        let mut jumps = 0usize;
        let mut pairs = 0usize;
        for row in f.shade.chunks(f.cols) {
            for w in row.windows(2) {
                pairs += 1;
                same += usize::from(w[0] == w[1]);
                jumps += usize::from(is_jump(w[0], w[1]));
            }
        }
        let n = f.shade.len() as f64;
        let lum = |i: u16| {
            let v = PAL[i as usize];
            0.299 * ((v >> 16) & 255) as f64
                + 0.587 * ((v >> 8) & 255) as f64
                + 0.114 * (v & 255) as f64
        };
        let mean = f.shade.iter().map(|&i| lum(i)).sum::<f64>() / n;
        let var = f
            .shade
            .iter()
            .map(|&i| (lum(i) - mean).powi(2))
            .sum::<f64>()
            / n;
        Look {
            lit: hist.iter().filter(|&&c| c > 0).count(),
            top: *hist.iter().max().unwrap() as f64 / n,
            coh: same as f64 / pairs as f64,
            jump: jumps as f64 / pairs as f64,
            mean,
            sd: var.sqrt(),
        }
    }

    /// Do these two neighbouring cells sit across a BOUNDARY rather than on a
    /// smooth ramp? A colour ramp walks one band at a time; a fractal boundary
    /// jumps, and the interior is a jump from anything.
    ///
    /// This is the measure that finally separates a picture from a pretty
    /// gradient. Colour count, coherence, brightness and spread all rate a
    /// half-black frame with a smooth diagonal wash across it as excellent,
    /// because it is excellent at all four.
    fn is_jump(a: u16, b: u16) -> bool {
        if a == b {
            return false;
        }
        if a == 0 || b == 0 {
            return true;
        }
        let d = (a as i32 - b as i32).abs();
        d.min(BANDS as i32 - d) > 1
    }

    struct Look {
        lit: usize,
        top: f64,
        coh: f64,
        jump: f64,
        mean: f64,
        sd: f64,
    }

    impl Look {
        /// A frame worth zooming into. Every bound here was added because the
        /// previous set of bounds shipped a bad picture:
        ///
        /// * `lit >= 12`     — not a two-colour field.
        /// * `coh >= 0.58`   — not confetti.
        /// * `coh <= 0.88`   — not a smooth gradient either. TOO coherent is
        ///   its own failure: a wide diagonal wash scores perfectly on colour
        ///   count, coherence and brightness, and is not a fractal.
        /// * `0.08 <= top < 0.75` — something owns part of the frame, but not
        ///   most of it. The low end is what keeps a boundary in view.
        /// * `mean`, `sd`    — bright enough to see, varied enough not to be a
        ///   near-black field. The palette has three near-black stops, so a
        ///   dark frame still counts thirty colours; only pixels say otherwise.
        /// * `jump >= 0.06`  — and THIS is the one that separates a fractal
        ///   from a pretty picture. Everything above rates a smooth diagonal
        ///   wash as a fine frame; only the band-jump count says it has one
        ///   long smooth edge where a fractal has a thousand.
        fn good(&self) -> bool {
            self.lit >= 12
                && (0.58..=0.88).contains(&self.coh)
                && (0.08..0.75).contains(&self.top)
                && self.jump >= 0.06
                && self.mean >= 30.0
                && self.sd >= 28.0
        }

        /// Ordering has to stay meaningful for BAD boxes too. A hard zero for
        /// everything that fails `good` leaves the beam with nothing to sort
        /// by at the first bad level, and it then wanders into whichever
        /// quadrant came first — which is how Heart and Magnet1 landed on a
        /// one-colour field.
        /// What `every_family_keeps_structure_on_screen_while_it_zooms`
        /// actually asserts, plus margin on the coherence bound. This — not
        /// `good` — is what a family's `rate` has to hold at every octave:
        /// `good` is a stricter taste test for CHOOSING a target, and the wide
        /// establishing frame at octave 0 fails it on every family.
        fn holds(&self) -> bool {
            self.top < 0.9 && self.lit >= 8 && self.coh >= 0.45
        }

        fn rank(&self) -> f64 {
            let base = self.coh * self.jump * (self.lit as f64).min(24.0);
            if self.good() {
                base
            } else {
                base * 1e-3
            }
        }
    }

    /// Boundary descent: split each live box into quadrants, render the family
    /// over each, keep the best few, halve, repeat. Uses the REAL render path,
    /// so the point it lands on is a boundary point for exactly what gets
    /// drawn. A BEAM rather than a greedy walk — greedy walks into a cul-de-sac
    /// on families whose good region is not the showiest one early on.
    #[test]
    #[ignore]
    fn find_boundary_targets() {
        // The SHIPPED geometry. At a coarser grid a smooth wash looks like
        // structure, and the descent hands back a diagonal gradient.
        let p = Panel::new(1920, 1080, 1920);
        for (fi, fam) in FAMILIES.iter().enumerate() {
            if matches!(fam.kind, Kind::Julia | Kind::Sierpinski) {
                continue;
            }
            let mut f = Fractal::new(&p, 15);
            f.fam = fi;
            let mut beam = vec![(fam.cx, fam.cy, fam.span * 0.5)];
            // The answer is the deepest point the descent could still see a
            // picture at, not wherever the beam happened to stop.
            let mut best = (0u32, fam.cx, fam.cy, fam.span * 0.5);
            for level in 0..18 {
                let iter = (40 + level * 8).min(110);
                let mut next: Vec<(f64, f64, f64, f64)> = Vec::new();
                for &(cx, cy, r) in &beam {
                    let h = r * 0.5;
                    for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                        let (qx, qy) = (cx + sx * h, cy + sy * h);
                        let look = score(&mut f, qx, qy, h, iter);
                        next.push((look.rank(), qx, qy, h));
                    }
                }
                next.sort_by(|a, b| b.0.total_cmp(&a.0));
                next.truncate(8);
                beam = next.iter().map(|&(_, x, y, r)| (x, y, r)).collect();
                let (cx, cy, r) = beam[0];
                let iter = (40 + level * 8).min(110);
                if score(&mut f, cx, cy, r, iter).good() {
                    best = (level + 1, cx, cy, r);
                }
            }
            let (level, cx, cy, r) = best;
            let l = score(&mut f, cx, cy, r, 110);
            println!(
                "{:?}: cx {cx:.15} cy {cy:.15} lvl {level} lit {} coh {:.2} jump {:.2}",
                fam.kind, l.lit, l.coh, l.jump
            );
        }
    }

    /// How far each family can be zoomed before the picture stops being a
    /// picture. Walks the family's own zoom path an octave at a time with the
    /// iteration schedule `advance` would have given it, and reports the last
    /// octave whose frame is still coherent.
    #[test]
    #[ignore]
    fn safe_octaves_per_family() {
        let p = Panel::new(1920, 1080, 1920);
        for (fi, fam) in FAMILIES.iter().enumerate() {
            let mut f = Fractal::new(&p, 15);
            f.fam = fi;
            let mut last = 0.0f64;
            let mut worst = 1.0f64;
            for k in 0..=32 {
                let oct = k as f64 * 0.5;
                let (cx, cy) = if fam.kind == Kind::Julia {
                    let (_, _, bx, by) = julia_view(f.jtheta);
                    (bx, by)
                } else {
                    (fam.cx, fam.cy)
                };
                let iter = (f.iter_base + (oct * 8.0) as u32).min(f.iter_cap);
                let r = fam.span * 0.5 / oct.exp2();
                let l = score(&mut f, cx, cy, r, iter);
                // STOP at the first bad octave rather than remembering the
                // deepest good one. A family can be fine at 14 octaves and
                // wrong at 9, and a zoom passes through 9 on its way.
                if !l.holds() {
                    break;
                }
                last = oct;
                worst = worst.min(l.coh);
            }
            println!(
                "{:<11} safe {last:4.1} oct  rate {:.2}  (min coh {worst:.2})",
                format!("{:?}", fam.kind),
                (last / MAX_OCTAVES).min(1.0)
            );
        }
    }
}

#[cfg(test)]
mod bench {
    use super::*;
    use crate::matrix::Matrix;
    use crate::saver;
    use std::time::Instant;

    fn run(s: &mut dyn saver::Saver, buf: &mut [u32], p: &Panel, n: usize) -> f64 {
        let t0 = Instant::now();
        for _ in 0..n {
            saver::frame(s, buf, p);
        }
        t0.elapsed().as_secs_f64() * 1e6 / n as f64
    }

    /// Interleaved: alternate chunks so both savers see the same machine state,
    /// and cover a WHOLE rotation, because the per-frame cost swings 5x between
    /// the start and the end of one zoom.
    #[test]
    #[ignore]
    fn interleaved_against_matrix() {
        let p = Panel::new(1920, 1080, 1920);
        let mut buf = vec![0u32; p.buf_len()];
        let mut f = Fractal::new(&p, 15);
        let mut m = Matrix::new(&p, 15);
        run(&mut f, &mut buf, &p, 60);
        run(&mut m, &mut buf, &p, 60);

        let chunk = 120usize;
        let chunks = (f.cycle * FAMILIES.len() as f64 * 15.0 / chunk as f64).ceil() as usize;
        let (mut fs, mut ms) = (Vec::new(), Vec::new());
        for _ in 0..chunks {
            fs.push(run(&mut f, &mut buf, &p, chunk));
            ms.push(run(&mut m, &mut buf, &p, chunk));
        }
        let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
        let (fm, mm) = (mean(&fs), mean(&ms));
        let lo = fs.iter().cloned().fold(f64::MAX, f64::min);
        let hi = fs.iter().cloned().fold(0.0, f64::max);
        println!(
            "ROTATION families={} frames={}",
            FAMILIES.len(),
            chunks * chunk
        );
        println!(
            "  fractal mean {fm:.1}us (chunk range {lo:.0}..{hi:.0})  matrix {mm:.1}us  ratio {:.2}x",
            fm / mm
        );
    }

    /// Per family, one whole cycle each, interleaved against matrix so the
    /// numbers are comparable to each other and to the rotation mean.
    #[test]
    #[ignore]
    fn per_family_cost() {
        let p = Panel::new(1920, 1080, 1920);
        let mut buf = vec![0u32; p.buf_len()];
        let mut m = Matrix::new(&p, 15);
        run(&mut m, &mut buf, &p, 60);
        let mut rows = Vec::new();
        for (fi, fam) in FAMILIES.iter().enumerate() {
            let mut f = Fractal::new(&p, 15);
            let n = (f.cycle * 15.0) as usize;
            let cycle = |f: &mut Fractal, buf: &mut [u32], frames| {
                f.fam = fi;
                f.t = 0.0;
                f.begin();
                run(f, buf, &p, frames)
            };
            cycle(&mut f, &mut buf, 20);
            let us = cycle(&mut f, &mut buf, n);
            let mus = run(&mut m, &mut buf, &p, 120);
            rows.push((us, format!("{:?}", fam.kind), us / mus));
        }
        rows.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (us, name, ratio) in &rows {
            println!("  {name:<11} {us:7.1}us  {ratio:5.2}x matrix");
        }
    }
}
