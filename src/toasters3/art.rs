//! toasters3's art: one toaster drawn at eight times the detail, baked into
//! braille cells at compile time. No logic, and nothing here runs —
//! `toasters3.rs` reads `FRAMES` and `TOAST_CELLS` and nothing else.
//!
//! # Why the art is a dot bitmap and not a row of braille characters
//!
//! Every sprite here is written as its DOT bitmap: `#` for a lit dot, a space
//! for background, two characters across and four rows down per cell.
//! `bake_braille` packs each 2x4 block into the U+2800..28FF pattern byte and
//! looks up `font::BRAILLE`. Writing the braille characters themselves would be
//! the same data in a form nobody can edit — and would break the byte-indexed
//! const parser, since U+28xx is three bytes of UTF-8 and a column is one byte.
//!
//! # What braille buys, and what it does not
//!
//! A cell carries eight dots and ONE colour. So the shape is eight times finer
//! than the palette: the slot, the curve of the chrome, the feather barbs and
//! the crumb pores are all dot-resolution, while every colour region — chrome,
//! shadow, the two olives, each wing — is still a whole cell wide. The regions
//! are therefore drawn two dots wide on purpose; a one-dot-wide highlight would
//! be a colour the cell grid cannot hold.
//!
//! The palette is `toasters`' own, sampled from the original sprite sheet, so
//! the two savers are the same machine at two resolutions rather than two
//! different toasters.

use crate::font;
use crate::grid::Cell;
use crate::toasters::art::{ink, PAL_RGB};

/// Re-exported so `toasters3` addresses the same colours `toasters` does —
/// see `toasters::art::PAL_RGB` for where each one was sampled from.
pub(super) const PAL: [u32; 16] = crate::grid::bake(&PAL_RGB);

/// Braille dot -> (column, row) in the 2x4 cell, by bit position. Dots 1..6
/// fill the first three rows column-major and 7/8 were bolted on underneath,
/// which is why the bottom row is bits 6 and 7 rather than 3 and 7. Same order
/// `tools/genfont.py` draws the glyphs in; disagreeing with it would transpose
/// every sprite.
const DOT_AT: [(usize, usize); 8] = [
    (0, 0),
    (0, 1),
    (0, 2),
    (1, 0),
    (1, 1),
    (1, 2),
    (0, 3),
    (1, 3),
];

/// Resolve a sprite set's dots and ink into cells, at COMPILE time. The frame
/// loop reads a `&'static [Cell]` out of the binary: no allocation, no runtime
/// bake, nothing to chase.
///
/// Width comes from `CELLS / IH`, and every row of both grids has to match it:
/// a ragged row, a dot character that is not `#` or a space, ink where no dot
/// is lit, and a lit cell with no ink are all `error[E0080]` rather than a
/// sprite that renders wrong.
const fn bake_braille<const AH: usize, const IH: usize, const N: usize, const CELLS: usize>(
    dots: &[[&str; AH]; N],
    ink_rows: &[[&str; IH]; N],
) -> [[Cell; CELLS]; N] {
    let w = CELLS / IH;
    assert!(w * IH == CELLS, "sprite cell count is not width x height");
    assert!(AH == IH * 4, "a braille cell is four dot rows tall");
    let mut out = [[Cell::CLEAR; CELLS]; N];
    let mut s = 0;
    while s < N {
        let mut r = 0;
        while r < AH {
            assert!(
                dots[s][r].len() == w * 2,
                "dot row is not twice the sprite's width"
            );
            r += 1;
        }
        let mut r = 0;
        while r < IH {
            let k = ink_rows[s][r].as_bytes();
            assert!(k.len() == w, "ink row is not the sprite's width");
            let mut c = 0;
            while c < w {
                let mut bits = 0usize;
                let mut d = 0;
                while d < 8 {
                    let (dc, dr) = DOT_AT[d];
                    let ch = dots[s][r * 4 + dr].as_bytes()[c * 2 + dc];
                    assert!(ch == b'#' || ch == b' ', "a dot is '#' or blank");
                    if ch == b'#' {
                        bits |= 1 << d;
                    }
                    d += 1;
                }
                if bits == 0 {
                    assert!(k[c] == b' ', "ink on a cell with no dots");
                } else {
                    assert!(k[c] != b' ', "dots with no ink");
                    out[s][r * w + c] = Cell::new(font::BRAILLE[bits], ink(k[c]));
                }
                c += 1;
            }
            r += 1;
        }
        s += 1;
    }
    out
}

pub(super) const TOASTER_W: usize = 16;
pub(super) const TOASTER_H: usize = 6;
const TOASTER_DOT_H: usize = TOASTER_H * 4;

/// Four wing positions of one toaster in three-quarter view, 32x24 dots each:
/// a domed top face receding up and to the right with two slots cut out of it
/// (the slot is BACKGROUND, which is what an opening looks like and what the
/// line-art version could only suggest with `[]`), a chrome front panel whose
/// left edge bows out and whose right falls through shadow into the
/// turned-away olive side, a browning dial, a lever, two feet, and a pair of
/// swept wings whose trailing edge is barbed every third quill.
///
/// The BODY is identical in all four frames — only the wings move — so the flap
/// never makes the machine wobble.
#[rustfmt::skip]
const TOASTER: [[&str; TOASTER_DOT_H]; 4] = [
    // wings fully up
    [
        "                                ",
        "                                ",
        "             ################   ",
        "            ####   ##   ####    ",
        "#          ####   ##   ####     ",
        "##        ####   ##   ####     #",
        "###      ################     ##",
        "###     ################      ##",
        "  ##     ###############     ## ",
        "  ###    ################   ### ",
        "  ####  #################  #### ",
        "   #### ############# ########  ",
        "    ######  ######### #######   ",
        "     #####   ######## ######    ",
        "     ###### ######### ######    ",
        "      ############### #####     ",
        "      #####         #######     ",
        "       ###################      ",
        "       ###################      ",
        "        ####        ######      ",
        "        #################       ",
        "         ################       ",
        "         ################       ",
        "         ###        ###         ",
    ],
    // mid upstroke
    [
        "                                ",
        "                                ",
        "             ################   ",
        "            ####   ##   ####    ",
        "           ####   ##   ####     ",
        "          ####   ##   ####      ",
        "         ################       ",
        "        ################        ",
        "#        ###############        ",
        "###      ################     ##",
        "#####   #################   ####",
        "####### ############# ##########",
        "  ########  ######### ######### ",
        "  ########   ######## ######### ",
        "     ###### ######### ######    ",
        "     ################ ######    ",
        "      #####         #######     ",
        "       ###################      ",
        "       ###################      ",
        "        ####        ######      ",
        "        #################       ",
        "         ################       ",
        "         ################       ",
        "         ###        ###         ",
    ],
    // level, fully extended
    [
        "                                ",
        "                                ",
        "             ################   ",
        "            ####   ##   ####    ",
        "           ####   ##   ####     ",
        "          ####   ##   ####      ",
        "         ################       ",
        "        ################        ",
        "         ###############        ",
        "         ################       ",
        "        #################       ",
        "        ############# ####      ",
        "##########  ######### ##########",
        "##########   ######## ##########",
        "########### ######### ##########",
        "# ################### ######### ",
        "   # ######         ######## #  ",
        "      #####################     ",
        "       ###################      ",
        "        ####        ######      ",
        "        #################       ",
        "         ################       ",
        "         ################       ",
        "         ###        ###         ",
    ],
    // wings fully down
    [
        "                                ",
        "                                ",
        "             ################   ",
        "            ####   ##   ####    ",
        "           ####   ##   ####     ",
        "          ####   ##   ####      ",
        "         ################       ",
        "        ################        ",
        "         ###############        ",
        "         ################       ",
        "        #################       ",
        "        ############# ####      ",
        "       ###  ######### ####      ",
        "      ####   ######## #####     ",
        "     ###### ######### ######    ",
        "    ################# #######   ",
        "  #########         ########### ",
        " ###############################",
        "################################",
        "####    ####        ######   ###",
        "# #     #################     # ",
        "#        ################       ",
        "         ################       ",
        "         ###        ###         ",
    ],
];

/// The colour of every cell above. Four regions and they are what the palette
/// buys: the near wing white and the far wing one stop down so the flap reads
/// as depth, the front panel running lit -> chrome -> shadow left to right
/// between two olive side edges, and the lit olive top face with chrome slot
/// rims. A space here must line up with a cell that has no dots at all.
#[rustfmt::skip]
const TOASTER_INK: [[&str; TOASTER_H]; 4] = [
    // wings fully up
    [
        "      ooooooooo ",
        "WW  moommmmooo c",
        " WWWWcCCCCsOOccc",
        "  WWWcCCCCCsOcc ",
        "   WWcCCCCCsOc  ",
        "    WcCCCCmOO   ",
    ],
    // mid upstroke
    [
        "      ooooooooo ",
        "    moommmmooo  ",
        "WWWWWcCCCCsOOccc",
        " WWWWcCCCCCsOccc",
        "   WWcCCCCCsOc  ",
        "    WcCCCCmOO   ",
    ],
    // level, fully extended
    [
        "      ooooooooo ",
        "    moommmmooo  ",
        "    WcCCCCsOO   ",
        "WWWWWcCCCCCsOccc",
        " WWWWcCCCCCsOcc ",
        "    WcCCCCmOO   ",
    ],
    // wings fully down
    [
        "      ooooooooo ",
        "    moommmmooo  ",
        "    WcCCCCsOO   ",
        "  WWWcCCCCCsOcc ",
        "WWWWWcCCCCCsOccc",
        "WW  WcCCCCmOO  c",
    ],
];

pub(super) const TOAST_W: usize = 8;
pub(super) const TOAST_H: usize = 4;
const TOAST_DOT_H: usize = TOAST_H * 4;

/// Four doneness levels, 16x16 dots each. The dots draw what a cell of solid
/// colour cannot: the two humps of the loaf's top, the crust line all the way
/// round, and the crumb pores. The browning is the part braille cannot draw —
/// a cell is one colour — so it is per cell, walking down the gold -> crust
/// ladder from a starting stop set by the doneness.
#[rustfmt::skip]
const TOAST_SPRITE: [[&str; TOAST_DOT_H]; 4] = [
    // doneness 0
    [
        "    ###  ###    ",
        "   ##########   ",
        "  ## #########  ",
        "  #### #######  ",
        "  ###### #####  ",
        "  ######## ###  ",
        "  ########## #  ",
        "  # ##########  ",
        "  ### ########  ",
        "  ##### ######  ",
        "  ####### ####  ",
        "  ######### ##  ",
        "  ############  ",
        "  ## #########  ",
        "   ##########   ",
        "                ",
    ],
    // doneness 1
    [
        "    ###  ###    ",
        "   ##########   ",
        "  ## #########  ",
        "  #### #######  ",
        "  ###### #####  ",
        "  ######## ###  ",
        "  ########## #  ",
        "  # ##########  ",
        "  ### ########  ",
        "  ##### ######  ",
        "  ####### ####  ",
        "  ######### ##  ",
        "  ############  ",
        "  ## #########  ",
        "   ##########   ",
        "                ",
    ],
    // doneness 2
    [
        "    ###  ###    ",
        "   ##########   ",
        "  ## #########  ",
        "  #### #######  ",
        "  ###### #####  ",
        "  ######## ###  ",
        "  ########## #  ",
        "  # ##########  ",
        "  ### ########  ",
        "  ##### ######  ",
        "  ####### ####  ",
        "  ######### ##  ",
        "  ############  ",
        "  ## #########  ",
        "   ##########   ",
        "                ",
    ],
    // doneness 3
    [
        "    ###  ###    ",
        "   ##########   ",
        "  ## #########  ",
        "  #### #######  ",
        "  ###### #####  ",
        "  ######## ###  ",
        "  ########## #  ",
        "  # ##########  ",
        "  ### ########  ",
        "  ##### ######  ",
        "  ####### ####  ",
        "  ######### ##  ",
        "  ############  ",
        "  ## #########  ",
        "   ##########   ",
        "                ",
    ],
];

#[rustfmt::skip]
const TOAST_INK: [[&str; TOAST_H]; 4] = [
    // doneness 0
    [
        " PPPPPP ",
        " gGGGGg ",
        " BYYYYB ",
        " byyyyb ",
    ],
    // doneness 1
    [
        " GGGGGG ",
        " BYYYYB ",
        " byyyyb ",
        " dggggd ",
    ],
    // doneness 2
    [
        " YYYYYY ",
        " byyyyb ",
        " dggggd ",
        " dBBBBd ",
    ],
    // doneness 3
    [
        " yyyyyy ",
        " dggggd ",
        " dBBBBd ",
        " dbbbbd ",
    ],
];

/// The art, resolved. `render` stamps out of these and nothing else. The cell
/// count is the one literal: `bake_braille` divides it by the height to get the
/// width every row then has to match, so a wrong count is a build failure and
/// not a sprite baked at the wrong stride.
const TOASTER_CELLS: [[Cell; 96]; 4] = bake_braille(&TOASTER, &TOASTER_INK);
pub(super) const TOAST_CELLS: [[Cell; 32]; 4] = bake_braille(&TOAST_SPRITE, &TOAST_INK);

/// The four wing positions, as slices into the const above — picking one is an
/// index and never a copy.
pub(super) const FRAMES: [&[Cell]; 4] = [
    &TOASTER_CELLS[0],
    &TOASTER_CELLS[1],
    &TOASTER_CELLS[2],
    &TOASTER_CELLS[3],
];
