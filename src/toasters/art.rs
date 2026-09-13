//! The toasters' art: the palette, the ink keys, and every sprite baked into
//! cells at compile time. No logic, and nothing here runs — `toasters.rs` reads
//! `MODELS` and `TOAST_CELLS` and nothing else.

use crate::font;
use crate::grid::{bake, Cell};

/// Sampled from the original's sprite sheet and quantised to colour families,
/// which is how the one real surprise showed up: the toaster body is NOT
/// chrome. It is an olive chassis with a chrome front panel and white wings,
/// and painting the whole thing one silver is the colour equivalent of
/// scrolling straight left.
///
/// The two olives are the depth cue. #707030 is the lit top face, #303010 the
/// sides turned away from the light, and neither is ever painted against the
/// background: the chassis FILL is where the art has blanks and blanks are
/// transparent, so what a stroke has to be distinguishable from is the stroke
/// beside it. Against the lit olive and the chrome panel, #303010 reads as a
/// turned-away face. Flatten them into one olive and the three-quarter view
/// goes with it, the same way a single wing colour would.
///
/// The sheet's 8.6% of near-black #101010 is the one family with no entry, and
/// that is structural rather than perceptual: it is the outline that separated
/// the sprite from the sheet's background, and here a cell lights only its
/// glyph pixels, so the gaps between glyphs already draw it.
///
/// Index 0 is the background. Nothing paints over it, which is why an idle
/// region costs zero blits.
#[rustfmt::skip]
pub(crate) const PAL_RGB: [[u8; 3]; 16] = [
    [0x00, 0x00, 0x00], //  0        background
    [0x90, 0x90, 0x90], //  1 'C' chrome front panel, the sheet's largest family
    [0xF0, 0xF0, 0xF0], //  2 'W' white — the near wing
    [0xD0, 0xD0, 0xD0], //  3 'c' light chrome — far wing, lever, panel highlight
    [0x30, 0x30, 0x10], //  4 'O' dark olive — the chassis sides, turned away
    [0x70, 0x70, 0x30], //  5 'o' lit olive — the top face
    [0x70, 0x70, 0x70], //  6 's' shadow chrome — the panel's lower lip
    [0xB0, 0xB0, 0xB0], //  7 'm' chrome midtone — the slot rims
    [0xF0, 0xD0, 0x70], //  8 'G' golden crumb, the slice's largest family
    [0xD0, 0x90, 0x30], //  9 'g' mid gold
    [0x90, 0x70, 0x10], // 10 'b' brown crust
    [0x70, 0x30, 0x10], // 11 'd' dark crust edge
    [0xB0, 0x70, 0x10], // 12 'B' mid brown
    [0xF0, 0xF0, 0x90], // 13 'P' pale highlight
    [0xD0, 0xB0, 0x30], // 14 'y' amber
    [0xF0, 0xD0, 0x50], // 15 'Y' deep gold
];
pub(crate) const PAL: [u32; 16] = bake(&PAL_RGB);

/// Ink key -> palette index. Every sprite carries a grid of these parallel to
/// its art, because the art reuses characters across regions: the `/` in column
/// 0 is a white wing and the `/` in column 9 is the olive body's receding edge,
/// and a character-to-colour map could not tell them apart.
///
/// There is no fallback arm. This runs inside `bake_sprites`, in a const, so a
/// key nobody defined is a build failure and not a stroke silently painted the
/// background colour.
pub(crate) const fn ink(k: u8) -> u16 {
    match k {
        b'C' => 1,
        b'W' => 2,
        b'c' => 3,
        b'O' => 4,
        b'o' => 5,
        b's' => 6,
        b'm' => 7,
        b'G' => 8,
        b'g' => 9,
        b'b' => 10,
        b'd' => 11,
        b'B' => 12,
        b'P' => 13,
        b'y' => 14,
        b'Y' => 15,
        _ => panic!("sprite ink uses a key the palette does not define"),
    }
}

/// Resolve a sprite set's art and ink into cells, at COMPILE time — the same
/// bargain `grid::bake` makes for a palette, for the same reason. The frame
/// loop reads a `&'static [Cell]` out of the binary, so there is no allocation,
/// no `Vec` of `Vec`s to chase, and no construction-time work at all.
///
/// Row width comes from `CELLS / H`, and every row of both grids has to match
/// it: a ragged row, an ink grid that is not blank exactly where its art is, and
/// an undefined ink key are all `error[E0080]` rather than a sprite that renders
/// wrong. The one drift this cannot see is an ink row shifted within its own
/// width — same length, no blanks moved — which is what the colour assertions
/// in the tests are for.
const fn bake_sprites<const H: usize, const N: usize, const CELLS: usize>(
    art: &[[&str; H]; N],
    ink_rows: &[[&str; H]; N],
) -> [[Cell; CELLS]; N] {
    let w = CELLS / H;
    assert!(w * H == CELLS, "sprite cell count is not width x height");
    let mut out = [[Cell::CLEAR; CELLS]; N];
    let mut s = 0;
    while s < N {
        let mut r = 0;
        while r < H {
            let (a, k) = (art[s][r].as_bytes(), ink_rows[s][r].as_bytes());
            assert!(a.len() == w, "art row is not the sprite's width");
            assert!(k.len() == w, "ink row is not the sprite's width");
            let mut c = 0;
            while c < w {
                if a[c] == b' ' {
                    assert!(k[c] == b' ', "ink where the art is blank");
                } else {
                    assert!(k[c] != b' ', "art stroke with no ink");
                    out[s][r * w + c] = Cell::new(font::ASCII[(a[c] - 0x20) as usize], ink(k[c]));
                }
                c += 1;
            }
            r += 1;
        }
        s += 1;
    }
    out
}

/// One toaster MODEL. The original flew one toaster in four wing positions;
/// this flies four toasters, each with its own four. A model is rolled when an
/// object spawns and never looked at again, so the variety costs the frame loop
/// one const index and no branch per cell.
///
/// The models are different SIZES, which is the only thing about them that
/// `toasters.rs` has to care about: every clear, stamp and despawn check goes
/// through `Obj::size`, and a shared constant there would leave the bigger
/// models trailing their own right-hand columns across the panel.
pub(super) struct Model {
    pub(super) w: usize,
    pub(super) h: usize,
    pub(super) frames: [&'static [Cell]; 4],
}

/// The only way to build one, so the three numbers a `Model` carries cannot
/// disagree: width is DERIVED from the baked cell count, and a height that does
/// not divide it is `error[E0080]`. Declaring the five-row model as four rows
/// was otherwise a silent half-drawn toaster that every test passed.
const fn model<const N: usize>(h: usize, c: &'static [[Cell; N]; 4]) -> Model {
    assert!(
        h > 0 && (N / h) * h == N,
        "model cell count is not width x height"
    );
    Model {
        w: N / h,
        h,
        frames: [&c[0], &c[1], &c[2], &c[3]],
    }
}

const CLASSIC_H: usize = 4;

/// Four wing positions of one two-slot toaster in three-quarter view: the top
/// face with its two slots recedes to the right, the near wing is the larger
/// one on the left. Every frame is the same size, so the stamp is a fixed
/// rectangle and the flap never moves the body.
#[rustfmt::skip]
const CLASSIC: [[&str; CLASSIC_H]; 4] = [
    // 0 — wings fully up
    [
        r"\    ____    /",
        r"\\  /[][]/| //",
        r" \\|=====|/// ",
        r"   |__o__|    ",
    ],
    // 1 — mid upstroke
    [
        r"     ____     ",
        r"\   /[][]/|  /",
        r"\\_|=====|/_//",
        r"   |__o__|    ",
    ],
    // 2 — level, fully extended
    [
        r"     ____     ",
        r"    /[][]/|   ",
        r"\__|=====|/__/",
        r"   |__o__|    ",
    ],
    // 3 — wings fully down
    [
        r"     ____     ",
        r"    /[][]/|   ",
        r" //|=====|/\\ ",
        r"// |__o__|  \\",
    ],
];

/// The colour of every stroke above, cell for cell. Four regions, and they are
/// what the palette buys: the wings are white (the near one) and light chrome
/// (the far one, one stop down so the flap reads as depth), the body columns of
/// each row are olive edges around a chrome front panel that runs light-to-
/// shadow left to right, and the top face is the lit olive with chrome slot
/// rims. A space here must line up with a space in the art; the glyph test
/// checks it, and every model below paints the same regions the same way.
#[rustfmt::skip]
const CLASSIC_INK: [[&str; CLASSIC_H]; 4] = [
    [
        "W    oooo    c",
        "WW  ommmmOO cc",
        " WWOcCCCsOccc ",
        "   OsscssO    ",
    ],
    [
        "     oooo     ",
        "W   ommmmOO  c",
        "WWWOcCCCsOcccc",
        "   OsscssO    ",
    ],
    [
        "     oooo     ",
        "    ommmmOO   ",
        "WWWOcCCCsOcccc",
        "   OsscssO    ",
    ],
    [
        "     oooo     ",
        "    ommmmOO   ",
        " WWOcCCCsOccc ",
        "WW OsscssO  cc",
    ],
];

const WIDE_H: usize = 4;

/// The four-slotter: the same machine four columns longer, which is the
/// silhouette difference that survives being 16 px a cell across a room — it is
/// a third wider than the classic and its slot row is twice as busy.
#[rustfmt::skip]
const WIDE: [[&str; WIDE_H]; 4] = [
    [
        r"\    ________    /",
        r"\\  /[][][][]/| //",
        r" \\|=========|/// ",
        r"   |__o___o__|    ",
    ],
    [
        r"     ________     ",
        r"\   /[][][][]/|  /",
        r"\\_|=========|/_//",
        r"   |__o___o__|    ",
    ],
    [
        r"     ________     ",
        r"    /[][][][]/|   ",
        r"\__|=========|/__/",
        r"   |__o___o__|    ",
    ],
    [
        r"     ________     ",
        r"    /[][][][]/|   ",
        r" //|=========|/\\ ",
        r"// |__o___o__|  \\",
    ],
];

#[rustfmt::skip]
const WIDE_INK: [[&str; WIDE_H]; 4] = [
    [
        "W    oooooooo    c",
        "WW  ommmmmmmmOO cc",
        " WWOcCCCCCCCsOccc ",
        "   OsscssscssO    ",
    ],
    [
        "     oooooooo     ",
        "W   ommmmmmmmOO  c",
        "WWWOcCCCCCCCsOcccc",
        "   OsscssscssO    ",
    ],
    [
        "     oooooooo     ",
        "    ommmmmmmmOO   ",
        "WWWOcCCCCCCCsOcccc",
        "   OsscssscssO    ",
    ],
    [
        "     oooooooo     ",
        "    ommmmmmmmOO   ",
        " WWOcCCCCCCCsOccc ",
        "WW OsscssscssO  cc",
    ],
];

const TALL_H: usize = 5;

/// The single-slot upright: two thirds the width and a row taller, so its
/// proportion is the opposite of the wide one and it reads as a different
/// object rather than the same one redecorated. Its wings are correspondingly
/// stubbier, and they beat across two body rows instead of one.
#[rustfmt::skip]
const TALL: [[&str; TALL_H]; 4] = [
    [
        r"\   __   /",
        r"\\ /[]/| /",
        r" \|===|/  ",
        r"  |===|/  ",
        r"  |_o_|   ",
    ],
    [
        r"    __    ",
        r"\  /[]/| /",
        r" _|===|/_ ",
        r"  |===|/  ",
        r"  |_o_|   ",
    ],
    [
        r"    __    ",
        r"   /[]/|  ",
        r"\_|===|/_/",
        r"  |===|/  ",
        r"  |_o_|   ",
    ],
    [
        r"    __    ",
        r"   /[]/|  ",
        r" /|===|/\ ",
        r"/ |===|/ \",
        r"  |_o_|   ",
    ],
];

#[rustfmt::skip]
const TALL_INK: [[&str; TALL_H]; 4] = [
    [
        "W   oo   c",
        "WW ommOO c",
        " WOcCsOc  ",
        "  OcCsOc  ",
        "  OscsO   ",
    ],
    [
        "    oo    ",
        "W  ommOO c",
        " WOcCsOcc ",
        "  OcCsOc  ",
        "  OscsO   ",
    ],
    [
        "    oo    ",
        "   ommOO  ",
        "WWOcCsOccc",
        "  OcCsOc  ",
        "  OscsO   ",
    ],
    [
        "    oo    ",
        "   ommOO  ",
        " WOcCsOcc ",
        "W OcCsOc c",
        "  OscsO   ",
    ],
];

const RETRO_H: usize = 4;

/// The rounded one: a domed top face and bowed sides in place of the classic's
/// square box and slanted three-quarter roof, with one long slot instead of
/// two. Same footprint as the classic, deliberately — what it is carrying is
/// shape, and matching the size is what proves shape alone tells two of these
/// apart in flight.
#[rustfmt::skip]
const RETRO: [[&str; RETRO_H]; 4] = [
    [
        r"\   .----.   /",
        r"\\ (::[]::) //",
        r" \\(======)// ",
        r"   (_o__o_)   ",
    ],
    [
        r"    .----.    ",
        r"\  (::[]::)  /",
        r"\\_(======)_//",
        r"   (_o__o_)   ",
    ],
    [
        r"    .----.    ",
        r"   (::[]::)   ",
        r"\__(======)__/",
        r"   (_o__o_)   ",
    ],
    [
        r"    .----.    ",
        r"   (::[]::)   ",
        r" //(======)\\ ",
        r"// (_o__o_) \\",
    ],
];

#[rustfmt::skip]
const RETRO_INK: [[&str; RETRO_H]; 4] = [
    [
        "W   oooooo   c",
        "WW OmmOOmmO cc",
        " WWOcCCCCsOcc ",
        "   OscsscsO   ",
    ],
    [
        "    oooooo    ",
        "W  OmmOOmmO  c",
        "WWWOcCCCCsOccc",
        "   OscsscsO   ",
    ],
    [
        "    oooooo    ",
        "   OmmOOmmO   ",
        "WWWOcCCCCsOccc",
        "   OscsscsO   ",
    ],
    [
        "    oooooo    ",
        "   OmmOOmmO   ",
        " WWOcCCCCsOcc ",
        "WW OscsscsO cc",
    ],
];

pub(super) const TOAST_W: usize = 4;
pub(super) const TOAST_H: usize = 3;

/// Four doneness levels, pale to scorched — the original shipped `toast0`
/// through `toast3` as four separate 64x64 sprites behind a darkness slider,
/// not one slice tinted, so the scorching is drawn as well as coloured.
#[rustfmt::skip]
const TOAST_SPRITE: [[&str; TOAST_H]; 4] = [
    [
        r" __ ",
        r"|  |",
        r"|__|",
    ],
    [
        r" __ ",
        r"|..|",
        r"|__|",
    ],
    [
        r" __ ",
        r"|::|",
        r"|##|",
    ],
    [
        r" __ ",
        r"|##|",
        r"|##|",
    ],
];

/// The doneness ramp, drawn in colour as well as in strokes: each level starts
/// one stop further down the gold -> brown ladder than the last and darkens
/// again from the crumb top to the crust edge, so all eight of the slice's
/// sampled colours are on screen at once across the flock.
#[rustfmt::skip]
const TOAST_INK: [[&str; TOAST_H]; 4] = [
    [
        " PP ",
        "G  G",
        "yYYy",
    ],
    [
        " GG ",
        "GggG",
        "gBBg",
    ],
    [
        " gg ",
        "gBBg",
        "BbbB",
    ],
    [
        " bb ",
        "bddb",
        "dddd",
    ],
];

/// The art, resolved. `render` stamps out of these and nothing else. The cell
/// count is the one literal here: `bake_sprites` divides it by the height to
/// get the row width every art row then has to match, so a wrong count is a
/// build failure and not a sprite baked at the wrong stride.
const CLASSIC_CELLS: [[Cell; 56]; 4] = bake_sprites(&CLASSIC, &CLASSIC_INK);
const WIDE_CELLS: [[Cell; 72]; 4] = bake_sprites(&WIDE, &WIDE_INK);
const TALL_CELLS: [[Cell; 50]; 4] = bake_sprites(&TALL, &TALL_INK);
const RETRO_CELLS: [[Cell; 56]; 4] = bake_sprites(&RETRO, &RETRO_INK);
pub(super) const TOAST_CELLS: [[Cell; 12]; 4] = bake_sprites(&TOAST_SPRITE, &TOAST_INK);

/// The flock's catalogue. Rows are `&'static [Cell]` into the consts above, so
/// picking a model is an index and never a copy.
pub(super) const MODELS: [Model; 4] = [
    model(CLASSIC_H, &CLASSIC_CELLS),
    model(WIDE_H, &WIDE_CELLS),
    model(TALL_H, &TALL_CELLS),
    model(RETRO_H, &RETRO_CELLS),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// A stroke that bakes to the blank glyph is a sprite quietly losing a
    /// character: the ink is there, the colour is there, and nothing draws.
    /// `bake_sprites` cannot see it — it maps the character through
    /// `font::ASCII` and a blank entry is a perfectly valid index — while a
    /// character outside printable ASCII is already `error[E0080]` there.
    ///
    /// This walks `MODELS`, so a model added to the catalogue is covered by
    /// being in it, rather than by someone remembering a second list.
    #[test]
    fn no_stroke_bakes_to_a_blank_glyph() {
        let frames = MODELS
            .iter()
            .enumerate()
            .flat_map(|(m, model)| {
                model
                    .frames
                    .iter()
                    .enumerate()
                    .map(move |(f, c)| (m, f, *c))
            })
            .chain(
                TOAST_CELLS
                    .iter()
                    .enumerate()
                    .map(|(f, c)| (usize::MAX, f, &c[..])),
            );
        for (m, f, cells) in frames {
            for (i, cell) in cells.iter().enumerate() {
                assert!(
                    *cell == Cell::CLEAR || cell.glyph() != font::BLANK as usize,
                    "model {m} frame {f} cell {i}: ink with no glyph"
                );
            }
        }
    }

    /// The subject of the whole palette: four regions, each its own colour, and
    /// two olives that have to stay two. Frame 2 is level flight, the frame the
    /// flock spends most of its time in. Asserting named cells is also the only
    /// thing that sees an ink row shifted within its own width — same length,
    /// blanks in the same places, every stroke a valid key, and the whole body
    /// wrongly coloured — so every model gets its own row of them, and a drift
    /// in one model's ink grid fails on that model alone.
    #[test]
    fn each_region_of_every_model_is_its_own_colour() {
        // (model, row, column, the key that cell must carry, and what that cell
        // IS). The label is the half that tells the next reader whether a
        // failure means the art moved or this table went stale.
        #[rustfmt::skip]
        const REGIONS: &[(usize, usize, usize, u8, &str)] = &[
            (0, 2,  0, b'W', "the near wing is white"),
            (0, 2, 13, b'c', "the far wing is one stop down"),
            (0, 2,  4, b'c', "the front panel is lit at its left edge"),
            (0, 2,  5, b'C', "chrome across its face"),
            (0, 2,  8, b's', "and in shadow at its right"),
            (0, 2,  3, b'O', "the body's side edges are turned away"),
            (0, 2,  9, b'O', "both of them"),
            (0, 0,  5, b'o', "the top face is the lit olive"),
            (0, 1,  5, b'm', "the slot rims are chrome"),

            (1, 2,  0, b'W', "wide: the near wing is white"),
            (1, 2, 17, b'c', "wide: the far wing is one stop down"),
            (1, 2,  4, b'c', "wide: the front panel is lit at its left edge"),
            (1, 2,  7, b'C', "wide: chrome across its face"),
            (1, 2, 12, b's', "wide: and in shadow at its right"),
            (1, 2,  3, b'O', "wide: the body's side edges are turned away"),
            (1, 2, 13, b'O', "wide: both of them"),
            (1, 0,  5, b'o', "wide: the top face is the lit olive"),
            (1, 1,  5, b'm', "wide: the slot rims are chrome"),

            (2, 2,  0, b'W', "tall: the near wing is white"),
            (2, 2,  9, b'c', "tall: the far wing is one stop down"),
            (2, 2,  3, b'c', "tall: the front panel is lit at its left edge"),
            (2, 2,  4, b'C', "tall: chrome across its face"),
            (2, 2,  5, b's', "tall: and in shadow at its right"),
            (2, 2,  2, b'O', "tall: the body's side edges are turned away"),
            (2, 2,  6, b'O', "tall: both of them"),
            (2, 0,  4, b'o', "tall: the top face is the lit olive"),
            (2, 1,  4, b'm', "tall: the slot rim is chrome"),

            (3, 2,  0, b'W', "retro: the near wing is white"),
            (3, 2, 13, b'c', "retro: the far wing is one stop down"),
            (3, 2,  4, b'c', "retro: the front panel is lit at its left edge"),
            (3, 2,  6, b'C', "retro: chrome across its face"),
            (3, 2,  9, b's', "retro: and in shadow at its right"),
            (3, 2,  3, b'O', "retro: the bowed sides are turned away"),
            (3, 2, 10, b'O', "retro: both of them"),
            (3, 0,  4, b'o', "retro: the dome is the lit olive"),
            (3, 1,  4, b'm', "retro: the slot rim is chrome"),
        ];
        for &(m, r, c, key, what) in REGIONS {
            let model = &MODELS[m];
            let got = model.frames[2][r * model.w + c].colour() as u16;
            assert_eq!(got, ink(key), "model {m} ({r},{c}): {what}");
        }

        // The depth cue. One olive and the three-quarter view is a flat box,
        // exactly as one wing colour would make the flap a flat flicker.
        assert_ne!(
            ink(b'O'),
            ink(b'o'),
            "two olives, or there is no near and far"
        );
        assert_ne!(ink(b'W'), ink(b'c'), "two whites, same reason");
    }

    /// Doneness has to darken, in strokes AND in colour. Every slice's crumb
    /// top, against the next.
    #[test]
    fn every_slice_is_darker_than_the_last() {
        let top = |d: usize| PAL[TOAST_CELLS[d][1].colour()];
        for d in 1..4 {
            assert!(
                top(d) < top(d - 1),
                "slice {d} is not darker than {}",
                d - 1
            );
        }
    }

    /// Four models and sixteen wing frames, or the catalogue is decoration: a
    /// frame pasted from one model into another, or a model listed twice, flies
    /// as variety that is not there and nothing else here would notice. Every
    /// frame is compared against every other, cell for cell — size is not
    /// allowed to answer the question early.
    #[test]
    fn every_wing_frame_in_the_catalogue_is_its_own_art() {
        let all: Vec<(usize, usize, &[Cell])> = MODELS
            .iter()
            .enumerate()
            .flat_map(|(m, model)| {
                model
                    .frames
                    .iter()
                    .enumerate()
                    .map(move |(f, c)| (m, f, *c))
            })
            .collect();
        assert_eq!(all.len(), 16);
        for (i, &(ma, fa, a)) in all.iter().enumerate() {
            for &(mb, fb, b) in &all[i + 1..] {
                assert_ne!(a, b, "model {ma} frame {fa} == model {mb} frame {fb}");
            }
        }
    }

    /// `model` derives the width, so the only way the catalogue can lie is a
    /// height that divides the cell count and is still wrong — 2 rows of 28 for
    /// the classic's 4 of 14. Pin the shapes the art is actually drawn at.
    #[test]
    fn every_model_is_the_shape_its_art_was_drawn_at() {
        let shapes: Vec<(usize, usize)> = MODELS.iter().map(|m| (m.w, m.h)).collect();
        assert_eq!(shapes, vec![(14, 4), (18, 4), (10, 5), (14, 4)]);
    }
}
