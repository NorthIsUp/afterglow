//! The view the panel shows: the GBA's 240x160 screen filling the height,
//! and each side of a wider panel in the average colour of the screen's
//! edge beside it, blurred down the rows and dimmed — the same ambient glow
//! as `gameboy`'s frame mode.
//!
//! A view pixel is RGB555, bit 15 marking it dimmed.

use super::agb::{H, W};

pub const DIM: u16 = 0x8000;

/// Columns of the screen's edge the glow averages, and rows it blurs over.
const GLOW_COLS: usize = 8;
const GLOW_ROWS: usize = 8;

/// mGBA's XBGR8888 (red low) to RGB555 (red low).
fn rgb555(p: u32) -> u16 {
    ((p >> 3 & 31) | (p >> 11 & 31) << 5 | (p >> 19 & 31) << 10) as u16
}

/// The screen into the middle of `out`, `w` wide, and the glow either side.
pub fn compose(screen: &[u32], w: usize, out: &mut [u16]) {
    let x0 = (w - W) / 2;
    for (y, src) in screen.as_chunks::<W>().0.iter().enumerate() {
        for (o, &p) in out[y * w + x0..][..W].iter_mut().zip(src) {
            *o = rgb555(p);
        }
    }
    if x0 == 0 {
        return;
    }
    let sum = |row: &[u16]| {
        row.iter().fold([0u32; 3], |a, &c| {
            [
                a[0] + u32::from(c & 31),
                a[1] + u32::from(c >> 5 & 31),
                a[2] + u32::from(c >> 10 & 31),
            ]
        })
    };
    let mut edges = [[[0u32; 3]; 2]; H];
    for (y, e) in edges.iter_mut().enumerate() {
        let row = &out[y * w..][..w];
        *e = [
            sum(&row[x0..x0 + GLOW_COLS]),
            sum(&row[x0 + W - GLOW_COLS..x0 + W]),
        ];
    }
    for y in 0..H {
        let (a, b) = (y.saturating_sub(GLOW_ROWS), (y + GLOW_ROWS + 1).min(H));
        let mut acc = [[0u32; 3]; 2];
        for e in &edges[a..b] {
            for side in 0..2 {
                for ch in 0..3 {
                    acc[side][ch] += e[side][ch];
                }
            }
        }
        let n = ((b - a) * GLOW_COLS) as u32;
        let px = |c: [u32; 3]| {
            (c[0] / n) as u16 | ((c[1] / n) as u16) << 5 | ((c[2] / n) as u16) << 10 | DIM
        };
        let row = &mut out[y * w..][..w];
        row[..x0].fill(px(acc[0]));
        row[x0 + W..].fill(px(acc[1]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The screen lands in the middle untouched, and each side is its edge's
    /// colour, dimmed.
    #[test]
    fn the_screen_sits_centred_with_a_glow_of_its_edges() {
        let w = 284;
        let x0 = (w - W) / 2;
        let red = 0x0000_00F8;
        let blue = 0x00F8_0000;
        let screen: Vec<u32> = (0..W * H)
            .map(|i| if i % W < W / 2 { red } else { blue })
            .collect();
        let mut out = vec![0u16; w * H];
        compose(&screen, w, &mut out);
        for y in [0, H / 2, H - 1] {
            let row = &out[y * w..][..w];
            assert_eq!(row[x0], 31);
            assert_eq!(row[x0 + W - 1], 31 << 10);
            assert_eq!(row[0], 31 | DIM);
            assert_eq!(row[w - 1], 31 << 10 | DIM);
        }
    }

    #[test]
    fn a_screen_exactly_as_wide_as_the_view_has_no_sides() {
        let screen = vec![0x00FF_FFFFu32; W * H];
        let mut out = vec![0u16; W * H];
        compose(&screen, W, &mut out);
        assert!(out.iter().all(|&p| p == 0x7FFF));
    }
}
