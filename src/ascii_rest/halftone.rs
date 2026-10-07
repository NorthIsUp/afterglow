//! What every ascii.rest halftone scene shares: the 4x4 ordered dither, the
//! " ·•●" dot steps and their coverage, and the nearest-palette lookup.
//!
//! The scenes differ in how they turn a shaded colour into a dot `level` (each
//! has its own floor, gamma and fade), so that stays in the scene. What comes
//! after — level to dot step to palette index — is [`Dots::step`] and
//! [`Dots::ink`].

use super::math::{clamp, js_round};
use crate::font;
use crate::grid::Cell;

/// Upstream's `BAYER`, `v / 16 - 0.47`.
pub const BAYER: [f64; 16] = {
    let m = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];
    let mut out = [0.0; 16];
    let mut i = 0;
    while i < 16 {
        out[i] = m[i] as f64 / 16.0 - 0.47;
        i += 1;
    }
    out
};

/// How much of the largest dot each step fills.
pub const COVER: [f64; 4] = [0.0, 0.3, 0.6, 1.0];

/// The dither offset for picture row `r`, column `x`.
#[inline]
pub fn bayer(r: usize, x: usize) -> f64 {
    BAYER[(r & 3) * 4 + (x & 3)]
}

/// A scene's palette as linear 0..1 RGB plus upstream's lazily filled 32-step
/// cube of nearest indices. Lazy because a scene touches a few hundred of the
/// 32768 entries; filling it up front would cost a second per saver switch.
pub struct Dots {
    pal: Vec<[f64; 3]>,
    lut: Vec<u8>,
}

impl Dots {
    pub fn new(palette: &[u32]) -> Self {
        assert!(palette.len() < 255, "255 is the lut's empty marker");
        let ch = |v: u32, s: u32| f64::from((v >> s) & 0xFF) / 255.0;
        Self {
            pal: palette
                .iter()
                .map(|&v| [ch(v, 16), ch(v, 8), ch(v, 0)])
                .collect(),
            lut: vec![255; 32768],
        }
    }

    /// The palette index nearest `(r, g, b)`, each 0..1, weighted 0.3/0.5/0.2.
    #[inline]
    pub fn nearest(&mut self, r: f64, g: f64, b: f64) -> u8 {
        let q = |v: f64| ((v * 31.99) as i32).min(31) as usize;
        let k = (q(r) << 10) | (q(g) << 5) | q(b);
        if self.lut[k] != 255 {
            return self.lut[k];
        }
        let (mut best, mut bd) = (0, 1e9);
        for (i, p) in self.pal.iter().enumerate() {
            let (dr, dg, db) = (p[0] - r, p[1] - g, p[2] - b);
            let d = 0.3 * dr * dr + 0.5 * dg * dg + 0.2 * db * db;
            if d < bd {
                bd = d;
                best = i;
            }
        }
        self.lut[k] = best as u8;
        best as u8
    }

    /// Upstream's `Math.max(0, Math.min(3, Math.round(level * 3 + dither)))`.
    #[inline]
    pub fn step(level: f64, dither: f64) -> usize {
        js_round(level * 3.0 + dither).clamp(0.0, 3.0) as usize
    }

    /// How much brighter the colour must be to make up what dot `step` lacks
    /// in coverage, 0..1; 0 for no dot. `lift` is upstream's `+ 0.06`.
    #[inline]
    pub fn want(step: usize, level: f64, lift: f64) -> f64 {
        if step > 0 {
            ((level + lift) / COVER[step]).min(1.0)
        } else {
            0.0
        }
    }

    /// Dot `step` in the palette colour nearest `rgb * s`. Scenes differ only
    /// in how they scale `s`, so that stays in the scene.
    #[inline]
    pub fn dot(&mut self, step: usize, rgb: [f64; 3], s: f64) -> Cell {
        let c = self.nearest(clamp(rgb[0] * s), clamp(rgb[1] * s), clamp(rgb[2] * s));
        Cell::new(font::HALFTONE[step], u16::from(c))
    }

    /// Upstream's usual tail: a small dot drawn brighter and a large one
    /// dimmer, so dot size and colour together carry `level`. `peak` is
    /// `max(r, g, b, 1e-4)`.
    #[inline]
    pub fn ink(&mut self, step: usize, level: f64, rgb: [f64; 3], peak: f64) -> Cell {
        self.dot(
            step,
            rgb,
            (0.3 + 0.7 * Self::want(step, level, 0.06)) / peak,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_rounds_like_javascript() {
        assert_eq!(Dots::step(0.0, -0.47), 0);
        assert_eq!(Dots::step(0.5, 0.0), 2); // 1.5 rounds up, not to even
        assert_eq!(Dots::step(2.0, 0.0), 3);
    }

    #[test]
    fn nearest_is_cached_and_exact() {
        let mut d = Dots::new(&[0x00_00_00, 0xff_ff_ff, 0xff_00_00]);
        assert_eq!(d.nearest(0.9, 0.1, 0.1), 2);
        assert_eq!(d.nearest(0.9, 0.1, 0.1), 2);
        assert_eq!(d.nearest(0.95, 0.95, 0.9), 1);
    }
}
