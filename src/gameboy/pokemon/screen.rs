//! What the game shows, read from `wTileMap` (the same address in all three
//! revisions) in the game's own character codes, and the menu the cursor is
//! in, read from the menu variables `HandleMenuInput` keeps.

use mizu_core::GameBoy;

use super::super::pilot::{A, DOWN, UP};

const TILE_MAP: u16 = 0xC3A0;
pub const COLS: usize = 20;
pub const ROWS: usize = 18;
pub const CURSOR: u8 = 0xED;
/// The blinking "more text" arrow at the text box's corner.
const MORE: (usize, usize) = (18, 16);

const TOP_ITEM_X: u16 = 0xCC25;
const CURRENT_ITEM: u16 = 0xCC26;
const MAX_ITEM: u16 = 0xCC28;

/// One tile code as ASCII, `.` for anything that is not a letter, digit or
/// the few signs menus use.
pub const fn ascii(c: u8) -> u8 {
    match c {
        0x80..=0x99 => c - 0x80 + b'A',
        0xA0..=0xB9 => c - 0xA0 + b'a',
        0xF6..=0xFF => c - 0xF6 + b'0',
        0xBA => b'e',
        0xE1 => b'P',
        0xE2 => b'M',
        0xE3 => b'-',
        0xE6 => b'?',
        0xE7 => b'!',
        0xE8 => b'.',
        0x7F => b' ',
        CURSOR => b'>',
        _ => b'.',
    }
}

pub fn row(gb: &mut GameBoy, y: usize) -> [u8; COLS] {
    let mut out = [0; COLS];
    for (x, o) in out.iter_mut().enumerate() {
        *o = ascii(gb.peek(TILE_MAP + (y * COLS + x) as u16));
    }
    out
}

/// Where `text` first shows on screen.
pub fn find(gb: &mut GameBoy, text: &[u8]) -> Option<(usize, usize)> {
    (0..ROWS).find_map(|y| {
        let r = row(gb, y);
        r.windows(text.len())
            .position(|w| w == text)
            .map(|x| (x, y))
    })
}

pub fn shows(gb: &mut GameBoy, text: &[u8]) -> bool {
    find(gb, text).is_some()
}

/// The solid menu cursor's square, if one is on screen.
pub fn cursor(gb: &mut GameBoy) -> Option<(usize, usize)> {
    (0..ROWS * COLS)
        .find(|&i| gb.peek(TILE_MAP + i as u16) == CURSOR)
        .map(|i| (i % COLS, i / COLS))
}

/// The text box's tiles folded into one number, minus the blinking arrow:
/// unchanged from frame to frame means the text has finished printing.
pub fn text_hash(gb: &mut GameBoy) -> u32 {
    let mut h = 0x811C_9DC5u32;
    for y in 12..ROWS {
        for x in 0..COLS {
            if (x, y) != MORE {
                h = (h ^ u32::from(gb.peek(TILE_MAP + (y * COLS + x) as u16)))
                    .wrapping_mul(0x0100_0193);
            }
        }
    }
    h
}

/// Debug: the whole screen as text.
#[cfg(test)]
pub fn dump(gb: &mut GameBoy) -> String {
    (0..ROWS)
        .map(|y| String::from_utf8_lossy(&row(gb, y)).into_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The open menu's cursor: the item it is on, the last item, and the
/// column its top item sits in (the battle menu's two columns differ only
/// there).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Menu {
    pub item: u8,
    pub max: u8,
    pub x: u8,
}

pub fn menu(gb: &mut GameBoy) -> Menu {
    Menu {
        item: gb.peek(CURRENT_ITEM),
        max: gb.peek(MAX_ITEM),
        x: gb.peek(TOP_ITEM_X),
    }
}

/// The button that moves a one-column menu's cursor toward `want`, A once
/// it is there.
pub fn toward(m: Menu, want: u8) -> u8 {
    match m.item.cmp(&want) {
        std::cmp::Ordering::Less => DOWN,
        std::cmp::Ordering::Greater => UP,
        std::cmp::Ordering::Equal => A,
    }
}

/// A name in RAM, `0x50`-terminated, as ASCII.
pub fn name(gb: &mut GameBoy, at: u16) -> [u8; 11] {
    let mut out = [0; 11];
    for (i, o) in out.iter_mut().enumerate() {
        let c = gb.peek(at + i as u16);
        if c == 0x50 {
            break;
        }
        *o = ascii(c);
    }
    out
}
