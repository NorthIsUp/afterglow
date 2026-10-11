//! The view the panel shows: the GBA's 240x160 screen filling the height,
//! and each side of a wider panel in the average colour of the screen's
//! edge beside it, blurred down the rows and dimmed — the same ambient glow
//! as `gameboy`'s frame mode.
//!
//! A view pixel is RGB555, bit 15 marking it dimmed.

use super::agb::{H, W};
use crate::rgb555::glow;

/// Rows the glow blurs over.
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
    glow::<H>(out, w, x0, W, GLOW_ROWS);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rgb555::DIM;

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
