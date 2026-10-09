//! Colours, piece sprites and the text face.

use crate::font;

pub const BG: u16 = 0;
pub const PANEL: u16 = 1;
pub const TEXT: u16 = 2;
pub const DIM: u16 = 3;
pub const ACCENT: u16 = 4;
pub const GOOD: u16 = 5;
pub const RULE: u16 = 7;
pub const BANNER: u16 = 8;
pub const SPARK: u16 = 12;
pub const FIRE: u16 = 17;
pub const FIRE_HOT: u16 = 18;
pub const MAGIC: u16 = 19;
pub const MAGIC_HOT: u16 = 20;
pub const ASH: u16 = 21;
pub const WOOD: u16 = 22;
pub const STEEL: u16 = 23;
/// Per side: outline, fill, shade, highlight.
pub const INK: [[u16; 4]; 2] = [[9, 10, 11, 12], [13, 14, 15, 16]];
/// The captured-piece minis: rim and fill per side, lighter than the board
/// inks because they sit on the dark panel.
pub const MINI: [[u16; 2]; 2] = [[11, 10], [16, 14]];
const THEMES_AT: u16 = 32;
const PER_THEME: u16 = 8;

pub const FIXED: [u32; 24] = [
    0x0d1017, 0x151a25, 0xe9e6df, 0x7d8597, 0xf2c14e, 0x6cc58a, 0xe2645a, 0x262c3a, 0x090b10,
    0x1b1b22, 0xf7f3e9, 0xc9bfac, 0xffffff, 0x0b0b0f, 0x383844, 0x24242c, 0x6a6a82, 0xff7a2a,
    0xffe36b, 0x6fe3ff, 0xe4fbff, 0x4a4a52, 0x9a6a3a, 0xcfd6e0,
];

/// Board colours, light and dark square: lichess brown, tournament green,
/// lichess blue, a dusk purple.
pub const THEMES: [(u32, u32); 4] = [
    (0xf0d9b5, 0xb58863),
    (0xeeeed2, 0x769656),
    (0xdee3e6, 0x8ca2ad),
    (0xe6dcf0, 0x8a76b0),
];

/// What a board square can be lit as.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lit {
    Plain,
    Last,
    Check,
}

/// A square's palette index, and the index of a piece shadow on it.
pub fn square(theme: usize, dark: bool, lit: Lit) -> u16 {
    let base = THEMES_AT + theme as u16 * PER_THEME + u16::from(dark);
    base + match lit {
        Lit::Plain => 0,
        Lit::Last => 2,
        Lit::Check => 4,
    }
}

pub fn shadow(sq_index: u16) -> u16 {
    let t = (sq_index - THEMES_AT) / PER_THEME;
    THEMES_AT + t * PER_THEME + 6 + (sq_index - THEMES_AT) % 2
}

fn mix(a: u32, b: u32, t: u32) -> u32 {
    let ch = |s: u32| {
        let (x, y) = ((a >> s) & 0xff, (b >> s) & 0xff);
        ((x * (256 - t) + y * t) >> 8) << s
    };
    ch(16) | ch(8) | ch(0)
}

pub fn palette() -> Vec<u32> {
    let mut p = vec![0; THEMES_AT as usize];
    p[..FIXED.len()].copy_from_slice(&FIXED);
    for (light, dark) in THEMES {
        let both = [light, dark];
        p.extend(both);
        p.extend(both.map(|c| mix(c, 0xf6e94a, 110)));
        p.extend(both.map(|c| mix(c, 0xff2a2a, 150)));
        p.extend(both.map(|c| mix(c, 0x000000, 60)));
    }
    p
}

pub const SPRITE: usize = 16;

#[rustfmt::skip]
const ART: [[&str; SPRITE]; 6] = [
    [
        "................",
        "................",
        "................",
        "................",
        "......XXXX......",
        ".....XXXXXX.....",
        ".....XXXXXX.....",
        "......XXXX......",
        ".....XXXXXX.....",
        "......XXXX......",
        "......XXXX......",
        ".....XXXXXX.....",
        "....XXXXXXXX....",
        "...XXXXXXXXXX...",
        "...XXXXXXXXXX...",
        "................",
    ],
    [
        "................",
        "................",
        "......X.X.......",
        ".....XXXXX......",
        "....XXXXXXX.....",
        "...XX#XXXXXX....",
        "..XXXXXXXXXXX...",
        ".XXXXXXXXXXXX...",
        ".XXXX.XXXXXXXX..",
        "..XX..XXXXXXXX..",
        ".....XXXXXXXXX..",
        "....XXXXXXXXXX..",
        "...XXXXXXXXXXX..",
        "..XXXXXXXXXXXX..",
        "..XXXXXXXXXXXX..",
        "................",
    ],
    [
        "................",
        ".......XX.......",
        "......XXXX......",
        ".....XXXXXX.....",
        "....XXXX#XXX....",
        "....XXX#XXXX....",
        "....XXXXXXXX....",
        ".....XXXXXX.....",
        "......XXXX......",
        ".....XXXXXX.....",
        "......XXXX......",
        ".....XXXXXX.....",
        "....XXXXXXXX....",
        "..XXXXXXXXXXXX..",
        "..XXXXXXXXXXXX..",
        "................",
    ],
    [
        "................",
        "................",
        "...XXX.XX.XXX...",
        "...XXX.XX.XXX...",
        "...XXXXXXXXXX...",
        "....XXXXXXXX....",
        ".....XXXXXX.....",
        ".....XXXXXX.....",
        ".....XXXXXX.....",
        ".....XXXXXX.....",
        ".....XXXXXX.....",
        "....XXXXXXXX....",
        "...XXXXXXXXXX...",
        "..XXXXXXXXXXXX..",
        "..XXXXXXXXXXXX..",
        "................",
    ],
    [
        ".......XX.......",
        ".X....XXXX....X.",
        ".XX....XX....XX.",
        ".XXX..XXXX..XXX.",
        "..XXXXXXXXXXXX..",
        "..XXXXXXXXXXXX..",
        "...XXXXXXXXXX...",
        "....XXXXXXXX....",
        ".....XXXXXX.....",
        ".....XXXXXX.....",
        "....XXXXXXXX....",
        ".....XXXXXX.....",
        "....XXXXXXXX....",
        "..XXXXXXXXXXXX..",
        "..XXXXXXXXXXXX..",
        "................",
    ],
    [
        ".......##.......",
        "......####......",
        ".......##.......",
        "...XXXX##XXXX...",
        "..XXXXXXXXXXXX..",
        "..XXXXXXXXXXXX..",
        "...XXXXXXXXXX...",
        "....XXXXXXXX....",
        ".....XXXXXX.....",
        ".....XXXXXX.....",
        "....XXXXXXXX....",
        ".....XXXXXX.....",
        "....XXXXXXXX....",
        "..XXXXXXXXXXXX..",
        "..XXXXXXXXXXXX..",
        "................",
    ],
];

/// The pawn's mini, drawn rather than pooled: pooling flattens its head
/// into the collar.
const PAWN_MINI: [&str; 8] = [
    "........", "...XX...", "..XXXX..", "...XX...", "...XX...", "..XXXX..", ".XXXXXX.", "........",
];

/// Pixel classes: 0 clear, then an index into a side's `INK`.
pub const CLEAR: u8 = 0;
const LINE: u8 = 1;
const FILL: u8 = 2;
const SHADE: u8 = 3;
const HI: u8 = 4;

pub type Sprite = [[u8; SPRITE]; SPRITE];
pub type Mini = [[u8; 8]; 8];

/// Outline wherever the body meets the outside, so the art is drawn as a
/// silhouette; a light edge on the left and a shade on the right of the fill.
pub fn classify<const N: usize>(body: &[[u8; N]; N]) -> [[u8; N]; N] {
    let at = |x: isize, y: isize| -> u8 {
        if x < 0 || y < 0 || x >= N as isize || y >= N as isize {
            0
        } else {
            body[y as usize][x as usize]
        }
    };
    let mut out = [[CLEAR; N]; N];
    for y in 0..N as isize {
        for x in 0..N as isize {
            out[y as usize][x as usize] = match at(x, y) {
                0 => CLEAR,
                2 => LINE,
                _ if [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .any(|&(dx, dy)| at(x + dx, y + dy) == 0) =>
                {
                    LINE
                }
                _ => FILL,
            };
        }
    }
    let lined = out;
    for y in 0..N {
        for x in 0..N {
            if lined[y][x] != FILL {
                continue;
            }
            let right = x + 1 < N && lined[y][x + 1] == LINE;
            let left = x > 0 && lined[y][x - 1] == LINE;
            out[y][x] = match (left, right) {
                (false, true) => SHADE,
                (true, false) => HI,
                _ => FILL,
            };
        }
    }
    out
}

/// Every piece's sprite, by kind (`sprites[kind - 1]`), and its 8x8 mini.
pub fn sprites() -> ([Sprite; 6], [Mini; 6]) {
    let mut big = [[[CLEAR; SPRITE]; SPRITE]; 6];
    let mut small = [[[CLEAR; 8]; 8]; 6];
    for (k, art) in ART.iter().enumerate() {
        let mut body = [[0u8; SPRITE]; SPRITE];
        for (y, row) in art.iter().enumerate() {
            for (x, c) in row.bytes().enumerate() {
                body[y][x] = match c {
                    b'X' => 1,
                    b'#' => 2,
                    _ => 0,
                };
            }
        }
        big[k] = classify(&body);
        let mut pooled = [[0u8; 8]; 8];
        for (y, row) in pooled.iter_mut().enumerate() {
            for (x, v) in row.iter_mut().enumerate() {
                let any = (0..2).any(|dy| (0..2).any(|dx| body[2 * y + dy][2 * x + dx] != 0));
                *v = u8::from(any);
            }
        }
        if k == 0 {
            for (row, art) in pooled.iter_mut().zip(PAWN_MINI) {
                for (v, c) in row.iter_mut().zip(art.bytes()) {
                    *v = u8::from(c == b'X');
                }
            }
        }
        small[k] = classify(&pooled);
    }
    (big, small)
}

/// The colour of class `c` of a piece of `side`; minis have only rim and
/// fill.
pub fn ink(side: usize, c: u8) -> u16 {
    INK[side][c as usize - 1]
}

pub fn mini_ink(side: usize, c: u8) -> u16 {
    MINI[side][usize::from(c != LINE)]
}

/// The text face: the font's glyphs without their empty top four rows, one
/// column trimmed off the left.
pub const TEXT_H: usize = 13;
pub const ADVANCE: usize = 7;
const TOP: usize = 3;

/// Row `y` (`0..TEXT_H`) of `c`'s glyph, bit 7 the leftmost column.
#[inline]
pub fn glyph_row(c: u8, y: usize) -> u8 {
    let i = if (0x20..0x7f).contains(&c) {
        font::ASCII[(c - 0x20) as usize]
    } else {
        font::ASCII[(b'?' - 0x20) as usize]
    };
    font::GLYPHS[i as usize][TOP + y] << 1
}
