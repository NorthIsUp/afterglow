//! How a scene's picture departs from the two shapes its `Layout` is exact
//! at: upstream's 200x100 and the 320x100 of the old `-wide` recompositions.

use super::math::clamp;

#[derive(Clone, Copy)]
pub struct Stretch {
    pub w: f64,
    /// 0 at 200 columns, 1 at 320; unclamped, so it runs on past 3.2:1 and
    /// below zero under 200.
    pub wide: f64,
    /// 0 from 200 columns up, 1 at 100: a square picture.
    pub narrow: f64,
    /// Rows past upstream's 100.
    pub tall: f64,
}

impl Stretch {
    pub fn new(w: usize, h: usize) -> Self {
        let wf = w as f64;
        Self {
            w: wf,
            wide: (wf - 200.0) / 120.0,
            narrow: clamp((200.0 - wf) / 100.0),
            tall: h as f64 - 100.0,
        }
    }

    /// A value that is `at` at 200 columns, `by_wide` more at 320 and
    /// `by_narrow` less at 100.
    #[inline]
    pub fn grow(&self, at: f64, by_wide: f64, by_narrow: f64) -> f64 {
        if self.w >= 200.0 {
            at + by_wide * self.wide
        } else {
            at - by_narrow * self.narrow
        }
    }
}
