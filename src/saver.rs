//! The one thing a screensaver is, and the one per-frame call around it.

use crate::fire::Fire;
use crate::grid::Grid;
use crate::matrix::Matrix;
use crate::surface::{Damage, Panel, Surface};

/// A screensaver. One frame, one call. Dispatch happens here and NOWHERE below
/// it: no `&dyn Palette`, no `fn cell(&self, x, y) -> Cell`, no `&mut dyn FnMut`
/// handed to the blit. Those convert 15 indirect calls a second into ~120k
/// (per cell) or ~31M (per pixel) and would blow the 500m limit on their own.
pub trait Saver {
    /// Advance the simulation and draw the frame. Damage is whatever `s`
    /// accrued while being written — a saver cannot report it and cannot
    /// under-report it.
    fn render(&mut self, s: &mut Surface<'_>);

    /// For the startup log line.
    fn name(&self) -> &'static str;

    /// The grid this saver draws through, for the web mirror. Every saver here
    /// has one — a saver that painted pixels directly could not be mirrored as
    /// cells, and would need its own answer rather than an `Option` here that
    /// every caller has to defend against.
    fn grid(&self) -> &Grid;

    /// Palette the cells' colour indices address, as XRGB8888.
    fn palette(&self) -> &[u32];
}

/// Construction is not a trait method: grid geometry is only known after
/// modeset, and `fn new(&Panel) -> Self` is not object-safe. Each saver reads
/// its own env vars in its own constructor, so no central struct enumerates
/// every saver's knobs.
///
/// Unknown names fall through to fire-ascii. This pod is headless on a remote
/// node: a typo must never crash-loop it. Adding a saver is one arm here.
pub fn make(name: &str, panel: &Panel, fps: u32) -> Box<dyn Saver> {
    match name {
        "blocks" => Box::new(Fire::blocks(panel)),
        "matrix" => Box::new(Matrix::new(panel, fps)),
        _ => Box::new(Fire::ascii(panel)),
    }
}

/// The per-frame body, shared verbatim by the DRM path and the dump path. A
/// dump that used its own loop would prove nothing about what runs on hardware.
#[inline]
pub fn frame(saver: &mut dyn Saver, buf: &mut [u32], panel: &Panel) -> Damage {
    let mut s = Surface::new(buf, panel);
    saver.render(&mut s);
    s.finish()
}
