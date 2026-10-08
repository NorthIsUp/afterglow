//! The block toasters' art: the palette, the two ink keys, and every sprite
//! baked into cells at compile time. No logic, and nothing here runs —
//! `toasters2.rs` reads `TOASTER`, `TOAST_CELLS` and the sizes beside them.

use crate::font;
use crate::grid::{bake, Cell};

/// The same sixteen colours `toasters` sampled off the original sprite sheet,
/// down to the index order — the sheet did not change because the renderer
/// did. What changed is how much of the frame each one covers: line art could
/// only put the olive chassis on the strokes that outlined it, and a filled
/// block puts it on the whole top face and the whole turned-away side, which is
/// the ratio the sheet actually has and the reason this saver exists.
///
/// Index 0 is the background. Nothing paints over it, so an idle region costs
/// zero blits.
#[rustfmt::skip]
pub(super) const PAL_RGB: [[u8; 3]; 16] = [
    [0x00, 0x00, 0x00], //  0        background
    [0x90, 0x90, 0x90], //  1 'C' chrome front panel, the sheet's largest family
    [0xF0, 0xF0, 0xF0], //  2 'W' white — the near wing
    [0xD0, 0xD0, 0xD0], //  3 'c' light chrome — far wing, panel highlight
    [0x30, 0x30, 0x10], //  4 'O' dark olive — the side turned away, the slots
    [0x70, 0x70, 0x30], //  5 'o' lit olive — the top face
    [0x70, 0x70, 0x70], //  6 's' shadow chrome — the panel's lower half
    [0xB0, 0xB0, 0xB0], //  7 'm' chrome midtone
    [0xF0, 0xD0, 0x70], //  8 'G' golden crumb, the slice's largest family
    [0xD0, 0x90, 0x30], //  9 'g' mid gold
    [0x90, 0x70, 0x10], // 10 'b' brown crust
    [0x70, 0x30, 0x10], // 11 'd' dark crust edge
    [0xB0, 0x70, 0x10], // 12 'B' mid brown
    [0xF0, 0xF0, 0x90], // 13 'P' pale highlight
    [0xD0, 0xB0, 0x30], // 14 'y' amber
    [0xF0, 0xD0, 0x50], // 15 'Y' deep gold
];
pub(super) const PAL: [u32; 16] = bake(&PAL_RGB);

/// Ink key -> palette index, exactly as `toasters` keys it, so the two sets of
/// art read the same to anyone who has seen either.
///
/// There is no fallback arm. This runs inside `bake_sprites`, in a const, so a
/// key nobody defined is a build failure and not a stroke silently painted the
/// background colour.
pub(super) const fn ink(k: u8) -> u16 {
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

/// Art key -> glyph. The whole alphabet of this saver: seven Block Elements and
/// the full block, and nothing else — no `/`, no `|`, no `=`. A cell carries
/// ONE colour, so the shape is what buys sub-cell detail: a half block splits
/// the 8x16 cell into two square 8x8 pixels, and a three-quarter block takes a
/// 90-degree corner off a diagonal that would otherwise climb in whole cells.
///
/// Eight keys, and this is the whole list — there are no lowercase quadrant
/// keys, so `p`, `q`, `b` and `d` are const-eval panics and not a silent
/// anything:
///
/// * `#` full, `^` upper half, `v` lower half, `%` 50% shade.
/// * `P` `Q` `B` `D` are the three-quarter blocks, each named for the corner it
///   is MISSING: `P` = ▛ no lower-right, `Q` = ▜ no lower-left, `B` = ▙ no
///   upper-right, `D` = ▟ no upper-left. They appear only at a wing's stair
///   joints, in the pair that bevels one step.
pub(super) const fn shape(k: u8) -> u16 {
    match k {
        b'#' => font::SOLID,
        b'^' => font::UPPER,
        b'v' => font::LOWER,
        b'%' => font::SHADE,
        b'P' => font::NO_LR,
        b'Q' => font::NO_LL,
        b'B' => font::NO_UR,
        b'D' => font::NO_UL,
        _ => panic!("sprite art uses a shape key that is not a block element"),
    }
}

/// Resolve a sprite set's art and ink into cells, at COMPILE time — the same
/// bargain `grid::bake` makes for a palette, for the same reason. The frame
/// loop reads a `&'static [Cell]` out of the binary, so there is no allocation,
/// no `Vec` of `Vec`s to chase, and no construction-time work at all.
///
/// Row width comes from `CELLS / H`, and every row of both grids has to match
/// it: a ragged row, an ink grid that is not blank exactly where its art is, an
/// undefined ink key and an undefined shape key are all `error[E0080]` rather
/// than a sprite that renders wrong. The one drift this cannot see is an ink row
/// shifted within its own width — same length, no blanks moved — which is what
/// the colour assertions in the tests are for.
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
                    out[s][r * w + c] = Cell::new(shape(a[c]), ink(k[c]));
                }
                c += 1;
            }
            r += 1;
        }
        s += 1;
    }
    out
}

pub(super) const TOASTER_W: usize = 30;
pub(super) const TOASTER_H: usize = 7;

/// Four wing positions of one two-slot toaster in three-quarter view, drawn as
/// filled regions rather than outlines.
///
/// The body is four solid areas and no lines at all: a lit top face receding up
/// and to the right one cell per row, two slots sunk into it, a chrome front
/// panel, and the side the light has turned away from running down the right.
/// Every frame is the same size, so the stamp is a fixed rectangle and the flap
/// never moves the body.
///
/// The wings are a staircase of horizontal runs with the joint columns
/// bevelled — `B`/`Q` where the stair descends rightwards, `D`/`P` where it
/// descends leftwards. Without those the wing is a flight of steps at 16 px a
/// tread, which is the one place the whole-cell grid shows through the
/// illusion.
#[rustfmt::skip]
const TOASTER: [[&str; TOASTER_H]; 4] = [
    // 0 — wings fully up
    [
        r"##B       #############       ",
        r"  Q#B    ##############    D##",
        r"    Q#B ###############  D#P  ",
        r"      Q##################P    ",
        r"        ###############       ",
        r"        ###############       ",
        r"        ^^^^^^^^^^^^^^^       ",
    ],
    // 1 — mid upstroke
    [
        r"          #############       ",
        r"##B      ##############       ",
        r"  Q##B  ###############    D##",
        r"     Q#####################P  ",
        r"        ###############       ",
        r"        ###############       ",
        r"        ^^^^^^^^^^^^^^^       ",
    ],
    // 2 — level, fully extended; no stair to bevel, so the root thickens instead
    [
        r"          #############       ",
        r"         ##############       ",
        r"      vv###############vv     ",
        r"##############################",
        r"      ^^###############^^     ",
        r"        ###############       ",
        r"        ^^^^^^^^^^^^^^^       ",
    ],
    // 3 — wings fully down
    [
        r"          #############       ",
        r"         ##############       ",
        r"        ###############       ",
        r"      D##################B    ",
        r"    D#P ###############  Q#B  ",
        r"  D#P   ###############    Q##",
        r"##P     ^^^^^^^^^^^^^^^       ",
    ],
];

/// The colour of every block above, cell for cell. Five regions, and they are
/// what the palette buys: the near wing is white and the far one a stop down so
/// the flap reads as depth, the top face is the LIT olive with its two slots
/// punched out in the dark one, the front panel runs light chrome to shadow
/// left to right, and the side under the top face's overhang is the dark olive
/// again — a whole face of it, not an edge stroke.
#[rustfmt::skip]
const TOASTER_INK: [[&str; TOASTER_H]; 4] = [
    [
        "WWW       ooooooooooooo       ",
        "  WWW    ooOOOOoOOOOooO    ccc",
        "    WWW oooooooooooooOO  ccc  ",
        "      WWccCCCCCCCCsssOOccc    ",
        "        cCCCCCCCCssssOO       ",
        "        sssssssssssssOO       ",
        "        OOOOOOOOOOOOOOO       ",
    ],
    [
        "          ooooooooooooo       ",
        "WWW      ooOOOOoOOOOooO       ",
        "  WWWW  oooooooooooooOO    ccc",
        "     WWWccCCCCCCCCsssOOccccc  ",
        "        cCCCCCCCCssssOO       ",
        "        sssssssssssssOO       ",
        "        OOOOOOOOOOOOOOO       ",
    ],
    [
        "          ooooooooooooo       ",
        "         ooOOOOoOOOOooO       ",
        "      WWoooooooooooooOOcc     ",
        "WWWWWWWWccCCCCCCCCsssOOccccccc",
        "      WWcCCCCCCCCssssOOcc     ",
        "        sssssssssssssOO       ",
        "        OOOOOOOOOOOOOOO       ",
    ],
    [
        "          ooooooooooooo       ",
        "         ooOOOOoOOOOooO       ",
        "        oooooooooooooOO       ",
        "      WWccCCCCCCCCsssOOccc    ",
        "    WWW cCCCCCCCCssssOO  ccc  ",
        "  WWW   sssssssssssssOO    ccc",
        "WWW     OOOOOOOOOOOOOOO       ",
    ],
];

pub(super) const TOAST_W: usize = 14;
pub(super) const TOAST_H: usize = 7;

/// Four doneness levels, pale to scorched. The scorch is DRAWN as well as
/// coloured — it creeps up from the bottom a row a level as a 50% shade over
/// the crumb, which is what a slice left in too long actually looks like and
/// what the original's four separate sprites were doing behind its darkness
/// slider. The dome is drawn in half blocks, which is the shape a whole-cell
/// grid could not hold: 14 cells of art over 7 rows is a 14x14 pixel slice.
#[rustfmt::skip]
const TOAST_SPRITE: [[&str; TOAST_H]; 4] = [
    [
        r"  vv######vv  ",
        r"v############v",
        r"##############",
        r"##############",
        r"##############",
        r"##############",
        r"##############",
    ],
    [
        r"  vv######vv  ",
        r"v############v",
        r"##############",
        r"##############",
        r"##############",
        r"##############",
        r"#%%%%%%%%%%%%#",
    ],
    [
        r"  vv######vv  ",
        r"v############v",
        r"##############",
        r"##############",
        r"##############",
        r"#%%%%%%%%%%%%#",
        r"#%%%%%%%%%%%%#",
    ],
    [
        r"  vv######vv  ",
        r"v############v",
        r"##############",
        r"##############",
        r"#%%%%%%%%%%%%#",
        r"#%%%%%%%%%%%%#",
        r"#%%%%%%%%%%%%#",
    ],
];

/// The doneness ramp in colour: each level starts one stop further down the
/// gold -> brown ladder than the last, crust darker than crumb throughout, so
/// all eight of the slice's sampled colours are on screen at once across the
/// flock.
#[rustfmt::skip]
const TOAST_INK: [[&str; TOAST_H]; 4] = [
    [
        "  YYYYYYYYYY  ",
        "YYYYYYYYYYYYYY",
        "YPPPPPPPPPPPPY",
        "YPPPPPPPPPPPPY",
        "YPPPPPPPPPPPPY",
        "YPPPPPPPPPPPPY",
        "YYYYYYYYYYYYYY",
    ],
    [
        "  yyyyyyyyyy  ",
        "yyyyyyyyyyyyyy",
        "yGGGGGGGGGGGGy",
        "yGGGGGGGGGGGGy",
        "yGGGGGGGGGGGGy",
        "yGGGGGGGGGGGGy",
        "yyyyyyyyyyyyyy",
    ],
    [
        "  BBBBBBBBBB  ",
        "BBBBBBBBBBBBBB",
        "BggggggggggggB",
        "BggggggggggggB",
        "BggggggggggggB",
        "BBBBBBBBBBBBBB",
        "BBBBBBBBBBBBBB",
    ],
    [
        "  dddddddddd  ",
        "dddddddddddddd",
        "dbbbbbbbbbbbbd",
        "dbbbbbbbbbbbbd",
        "dddddddddddddd",
        "dddddddddddddd",
        "dddddddddddddd",
    ],
];

/// The art, resolved. `render` stamps out of these and nothing else. The cell
/// count is the one literal here: `bake_sprites` divides it by the height to
/// get the row width every art row then has to match, so a wrong count is a
/// build failure and not a sprite baked at the wrong stride.
pub(super) const TOASTER_CELLS: [[Cell; TOASTER_W * TOASTER_H]; 4] =
    bake_sprites(&TOASTER, &TOASTER_INK);
pub(super) const TOAST_CELLS: [[Cell; TOAST_W * TOAST_H]; 4] =
    bake_sprites(&TOAST_SPRITE, &TOAST_INK);

#[cfg(test)]
mod tests {
    use super::*;

    /// The constraint the whole saver rests on: every character either of the
    /// two grids uses has to be a Block Element that is actually IN the atlas
    /// with pixels lit. `shape` already rejects an unknown key at build time,
    /// but it cannot see a key that maps to a glyph slot Unifont left blank —
    /// the ink would be right, the colour would be right, and nothing would
    /// draw. Walking the baked cells covers both grids and every future sprite.
    #[test]
    fn every_sprite_character_has_a_non_blank_glyph() {
        let all = TOASTER_CELLS
            .iter()
            .map(|c| &c[..])
            .chain(TOAST_CELLS.iter().map(|c| &c[..]));
        for (f, cells) in all.enumerate() {
            for (i, cell) in cells.iter().enumerate() {
                assert!(
                    *cell == Cell::CLEAR || cell.glyph() != font::BLANK as usize,
                    "sprite {f} cell {i}: ink with no glyph"
                );
                assert!(
                    *cell == Cell::CLEAR || font::GLYPHS[cell.glyph()].iter().any(|&row| row != 0),
                    "sprite {f} cell {i}: glyph {} is blank in the atlas",
                    cell.glyph()
                );
            }
        }
        // And the alphabet really is blocks: no glyph a sprite uses may be an
        // ASCII letterform, which is what a copy-paste from `toasters` would
        // leave behind.
        let all = TOASTER_CELLS
            .iter()
            .map(|c| &c[..])
            .chain(TOAST_CELLS.iter().map(|c| &c[..]));
        for cells in all {
            for cell in cells.iter().filter(|c| **c != Cell::CLEAR) {
                assert!(
                    !font::ASCII.contains(&(cell.glyph() as u16)),
                    "glyph {} is a letterform, not a block",
                    cell.glyph()
                );
            }
        }
    }

    /// The subject of the whole palette, and the thing this version has that
    /// the line-art one could not: the olive is a FILL. Asserting named cells
    /// is also the only thing that sees an ink row shifted within its own width
    /// — same length, blanks in the same places, every stroke a valid key, and
    /// the whole body wrongly coloured.
    #[test]
    fn each_region_is_its_own_colour() {
        // (row, column, the key that cell must carry, and what that cell IS).
        // Frame 2 is level flight, the frame the flock spends most of its time
        // in. The label is the half that tells the next reader whether a
        // failure means the art moved or this table went stale.
        #[rustfmt::skip]
        const REGIONS: &[(usize, usize, u8, &str)] = &[
            (3,  0, b'W', "the near wing is white"),
            (3, 29, b'c', "the far wing is one stop down"),
            (3,  8, b'c', "the front panel is lit at its left edge"),
            (3, 12, b'C', "chrome across its face"),
            (3, 20, b's', "and in shadow at its right"),
            (5, 10, b's', "the panel's lower half is all shadow"),
            (0, 16, b'o', "the top face is the lit olive"),
            (2, 10, b'o', "and it is a fill, two rows of it"),
            (1, 12, b'O', "a slot is punched out of it in the dark olive"),
            (1, 17, b'O', "both of them"),
            (1, 15, b'o', "with lit olive between"),
            (3, 21, b'O', "the turned-away side is a whole face of dark olive"),
            (5, 22, b'O', "top to bottom"),
            (6, 10, b'O', "and the base under it"),
        ];
        let f = &TOASTER_CELLS[2];
        for &(r, c, key, what) in REGIONS {
            let got = f[r * TOASTER_W + c].colour() as u16;
            assert_eq!(got, ink(key), "({r},{c}): {what}");
        }

        // The depth cue. One olive and the three-quarter view is a flat box,
        // exactly as one wing colour would make the flap a flat flicker.
        assert_ne!(
            ink(b'O'),
            ink(b'o'),
            "two olives, or there is no near and far"
        );
        assert_ne!(ink(b'W'), ink(b'c'), "two whites, same reason");

        // The claim in the module doc and `docs/savers/toasters.md`, as the one number all
        // three quote: half of the toaster's lit cells are olive, which is
        // what "the chassis is a fill and not an outline" means in figures.
        // Pinned as a band around the measured 50..=53%. Measured sensitivity:
        // it fails once five of the 63 olive cells stop being olive and
        // tolerates three moving, so it is the FILL claim and not a pin on
        // individual cells — `REGIONS` above is the pin. EVERY frame, because
        // the wings are what changes between them and the chassis is what
        // must not.
        for (i, f) in TOASTER_CELLS.iter().enumerate() {
            let lit: Vec<u16> = f
                .iter()
                .filter(|c| **c != Cell::CLEAR)
                .map(|c| c.colour() as u16)
                .collect();
            let olive = lit
                .iter()
                .filter(|&&c| c == ink(b'o') || c == ink(b'O'))
                .count();
            let pct = olive * 100 / lit.len();
            assert!(
                (48..=55).contains(&pct),
                "frame {i}: olive is {olive} of {} lit cells ({pct}%), not the fill \
                 the docs claim",
                lit.len()
            );
        }
    }

    /// Doneness has to darken, in drawn scorch AND in colour, or the four
    /// slices are one slice tinted four ways.
    #[test]
    fn every_slice_is_darker_than_the_last() {
        // Both regions, because they ramp independently: sampling only the
        // crust let a slice with a PALER crumb than the slice before it pass.
        let at = |d: usize, r: usize, c: usize| PAL[TOAST_CELLS[d][r * TOAST_W + c].colour()];
        for (r, c, what) in [(1, 6, "crust"), (2, 6, "crumb")] {
            for d in 1..4 {
                assert!(
                    at(d, r, c) < at(d - 1, r, c),
                    "slice {d}'s {what} is not darker than {}'s",
                    d - 1
                );
            }
        }
        let scorched = |d: usize| {
            TOAST_CELLS[d]
                .iter()
                .filter(|c| c.glyph() == font::SHADE as usize)
                .count()
        };
        for d in 1..4 {
            assert!(
                scorched(d) > scorched(d - 1),
                "slice {d} has no more scorch drawn on it than {}",
                d - 1
            );
        }
    }

    /// Four wing frames, or the flap is decoration: a frame pasted from another
    /// flies as motion that is not there and nothing else here would notice.
    #[test]
    fn every_wing_frame_is_its_own_art() {
        for (i, a) in TOASTER_CELLS.iter().enumerate() {
            for (j, b) in TOASTER_CELLS.iter().enumerate().skip(i + 1) {
                assert_ne!(a, b, "frame {i} == frame {j}");
            }
        }
        for (i, a) in TOAST_CELLS.iter().enumerate() {
            for (j, b) in TOAST_CELLS.iter().enumerate().skip(i + 1) {
                assert_ne!(a, b, "slice {i} == slice {j}");
            }
        }
    }
}
