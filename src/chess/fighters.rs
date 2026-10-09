//! The fight's actors: each piece redrawn at twice the board sprites'
//! resolution, facing right, with stubby legs and an arm drawn on top so one
//! body serves every pose.

use super::art::{classify, CLEAR};

pub const BODY: usize = 32;
/// Body rows; the legs fill the rest of the 32.
pub const TORSO: usize = 27;

#[rustfmt::skip]
const ART: [[&str; TORSO]; 6] = [
    [
        "................................",
        "................................",
        "................................",
        "................................",
        "................................",
        "..............XXXX..............",
        "............XXXXXXXX............",
        "...........XXXXXXXXXX...........",
        "..........XXXXXXXXXXXX..........",
        "..........XXXXXXXXX#XX..........",
        "..........XXXXXXXXXXXX..........",
        "...........XXXXXXXXXX...........",
        "............XXXXXXXX............",
        "..........XXXXXXXXXXXX..........",
        ".........XXXXXXXXXXXXXX.........",
        "..........XXXXXXXXXXXX..........",
        "...........XXXXXXXXXX...........",
        "...........XXXXXXXXXX...........",
        "..........XXXXXXXXXXXX..........",
        ".........XXXXXXXXXXXXXX.........",
        "........XXXXXXXXXXXXXXXX........",
        ".......XXXXXXXXXXXXXXXXXX.......",
        "......XXXXXXXXXXXXXXXXXXXX......",
        "......XXXXXXXXXXXXXXXXXXXX......",
        "......X##################X......",
        "......XXXXXXXXXXXXXXXXXXXX......",
        "......XXXXXXXXXXXXXXXXXXXX......",
    ],
    [
        "................................",
        "...............X..X.............",
        "..............XX.XX.............",
        ".............XXXXXXX............",
        "............XXXXXXXXX...........",
        "...........XXXXXXXXXXX..........",
        "..........XXXXXXXXX#XXX.........",
        ".........XXXXXXXXXXXXXXX........",
        ".........XXXXXXXXXXXXXXXXX......",
        "........XXXXXXXXXXXXXXXXXXX.....",
        "........XXXXXXXXXXXXXXXXXXXX....",
        "........XXXXXXXXXXXXXXXXXXX#....",
        ".......XXXXXXXXXX.XXXXXXXXX.....",
        ".......XXXXXXXXXX...XXXXXX......",
        ".......XXXXXXXXXXX..............",
        "......XXXXXXXXXXXX..............",
        "......XXXXXXXXXXXXX.............",
        "......XXXXXXXXXXXXXX............",
        "......XXXXXXXXXXXXXXX...........",
        "......XXXXXXXXXXXXXXXX..........",
        ".......XXXXXXXXXXXXXXXX.........",
        "........XXXXXXXXXXXXXXX.........",
        "......XXXXXXXXXXXXXXXXXXXX......",
        "......XXXXXXXXXXXXXXXXXXXX......",
        "......X##################X......",
        "......XXXXXXXXXXXXXXXXXXXX......",
        "......XXXXXXXXXXXXXXXXXXXX......",
    ],
    [
        "...............XX...............",
        "..............XXXX..............",
        ".............XXXXXX.............",
        "............XXXXXXXX............",
        "...........XXXXX#XXXX...........",
        "...........XXXX#XXXXX...........",
        "..........XXXX#XXXXXXX..........",
        "..........XXXXXXXXX#XX..........",
        "..........XXXXXXXXXXXX..........",
        "...........XXXXXXXXXX...........",
        "............XXXXXXXX............",
        "..........XXXXXXXXXXXX..........",
        ".........XXXXXXXXXXXXXX.........",
        "..........XXXXXXXXXXXX..........",
        "...........XXXXXXXXXX...........",
        "...........XXXXXXXXXX...........",
        "...........XXXXXXXXXX...........",
        "..........XXXXXXXXXXXX..........",
        "..........XXXXXXXXXXXX..........",
        ".........XXXXXXXXXXXXXX.........",
        "........XXXXXXXXXXXXXXXX........",
        ".......XXXXXXXXXXXXXXXXXX.......",
        "......XXXXXXXXXXXXXXXXXXXX......",
        "......XXXXXXXXXXXXXXXXXXXX......",
        "......X##################X......",
        "......XXXXXXXXXXXXXXXXXXXX......",
        "......XXXXXXXXXXXXXXXXXXXX......",
    ],
    [
        "................................",
        "................................",
        ".......XXXXX..XXXXXX..XXXXX.....",
        ".......XXXXX..XXXXXX..XXXXX.....",
        ".......XXXXXXXXXXXXXXXXXXXX.....",
        ".......XXXXXXXXXXXXXXXXXXXX.....",
        "........XXXXXXXXXXXXXXXXXX......",
        ".........XXXXXXXXXXXXXXXX.......",
        ".........XXXXXXXXXXXXXXXX.......",
        ".........XXXXXXXXX#XX#XXX.......",
        ".........XXXXXXXXXXXXXXXX.......",
        ".........X#######XXXXXXXX.......",
        ".........XXXXXXXXXXXXXXXX.......",
        ".........XXXX#######XXXXX.......",
        ".........XXXXXXXXXXXXXXXX.......",
        ".........X#######XXXXXXXX.......",
        ".........XXXXXXXXXXXXXXXX.......",
        ".........XXXX#######XXXXX.......",
        ".........XXXXXXXXXXXXXXXX.......",
        ".........X#######XXXXXXXX.......",
        "........XXXXXXXXXXXXXXXXXX......",
        ".......XXXXXXXXXXXXXXXXXXXX.....",
        "......XXXXXXXXXXXXXXXXXXXXXX....",
        "......XXXXXXXXXXXXXXXXXXXXXX....",
        "......X####################X....",
        "......XXXXXXXXXXXXXXXXXXXXXX....",
        "......XXXXXXXXXXXXXXXXXXXXXX....",
    ],
    [
        "..............XXXX..............",
        "....XX........XXXX........XX....",
        "....XX.........XX.........XX....",
        ".....X...XX....XX....XX...X.....",
        ".....XX..XX...XXXX...XX..XX.....",
        ".....XXX.XXX..XXXX..XXX.XXX.....",
        "......XXXXXXXXXXXXXXXXXXXX......",
        "......X##################X......",
        ".......XXXXXXXXXXXXXXXXXX.......",
        ".........XXXXXXXXXXXXXX.........",
        "..........XXXXXXXXX#XX..........",
        "...........XXXXXXXXXX...........",
        "............XXXXXXXX............",
        "...........XXXXXXXXXX...........",
        "..........XXXXXXXXXXXX..........",
        "...........XXXXXXXXXX...........",
        "...........XXXXXXXXXX...........",
        "..........XXXXXXXXXXXX..........",
        ".........XXXXXXXXXXXXXX.........",
        "........XXXXXXXXXXXXXXXX........",
        ".......XXXXXXXXXXXXXXXXXX.......",
        "......XXXXXXXXXXXXXXXXXXXX......",
        ".....XXXXXXXXXXXXXXXXXXXXXX.....",
        ".....XXXXXXXXXXXXXXXXXXXXXX.....",
        ".....X####################X.....",
        ".....XXXXXXXXXXXXXXXXXXXXXX.....",
        ".....XXXXXXXXXXXXXXXXXXXXXX.....",
    ],
    [
        "..............####..............",
        "..............####..............",
        "...........##########...........",
        "...........##########...........",
        "..............####..............",
        "......XXXXXXXX####XXXXXXXX......",
        ".....XXXXXXXXXXXXXXXXXXXXXX.....",
        ".....XXXXXXXXXXXXXXXXXXXXXX.....",
        ".....X####################X.....",
        "......XXXXXXXXXXXXXXXXXXXX......",
        ".........XXXXXXXXX#XXX..........",
        "..........XXXXXXXXXXXX..........",
        "...........XXXXXXXXXX...........",
        "...........XXXXXXXXXX...........",
        "..........XXXXXXXXXXXX..........",
        "...........XXXXXXXXXX...........",
        "...........XXXXXXXXXX...........",
        "..........XXXXXXXXXXXX..........",
        ".........XXXXXXXXXXXXXX.........",
        "........XXXXXXXXXXXXXXXX........",
        ".......XXXXXXXXXXXXXXXXXX.......",
        "......XXXXXXXXXXXXXXXXXXXX......",
        ".....XXXXXXXXXXXXXXXXXXXXXX.....",
        ".....XXXXXXXXXXXXXXXXXXXXXX.....",
        ".....X####################X.....",
        ".....XXXXXXXXXXXXXXXXXXXXXX.....",
        ".....XXXXXXXXXXXXXXXXXXXXXX.....",
    ],
];

pub type Body = [[u8; BODY]; BODY];

/// Where the arm hangs from and where the head is, facing right, in body
/// pixels: `(shoulder x, shoulder y, head x, head y)`.
pub const JOINTS: [(i32, i32, i32, i32); 6] = [
    (20, 15, 16, 9),
    (17, 16, 18, 7),
    (20, 13, 16, 6),
    (22, 12, 16, 8),
    (20, 14, 16, 10),
    (20, 14, 16, 10),
];

/// Two legs under the base, a walk frame lifting one, then the other.
const LEGS: [(i32, i32); 2] = [(11, 3), (18, 3)];

pub fn bodies() -> [Body; 6] {
    let mut out = [[[CLEAR; BODY]; BODY]; 6];
    for (k, art) in ART.iter().enumerate() {
        let mut b = [[0u8; BODY]; BODY];
        for (y, row) in art.iter().enumerate() {
            for (x, c) in row.bytes().enumerate() {
                b[y][x] = match c {
                    b'X' => 1,
                    b'#' => 2,
                    _ => 0,
                };
            }
        }
        out[k] = classify(&b);
    }
    out
}

/// Leg pixels for walk frame `step` (0 stands): `(x, y)` with `y` in
/// `TORSO..BODY`, as class `LINE` around a `FILL` core.
pub fn leg_px(step: u8, x: i32, y: i32) -> u8 {
    for (i, &(lx, w)) in LEGS.iter().enumerate() {
        let lift = match step {
            1 if i == 0 => 2,
            3 if i == 1 => 2,
            _ => 0,
        };
        let bottom = BODY as i32 - 1 - lift;
        if x >= lx && x < lx + w && y >= TORSO as i32 && y <= bottom {
            let edge = x == lx || x == lx + w - 1 || y == bottom;
            return if edge { 1 } else { 2 };
        }
    }
    CLEAR
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_is_body_wide() {
        for (k, art) in ART.iter().enumerate() {
            for (y, row) in art.iter().enumerate() {
                assert_eq!(row.len(), BODY, "kind {k} row {y}");
            }
        }
    }
}
