//! The one thing a screensaver is, and the one per-frame call around it.

use crate::city::City;
use crate::fire::Fire;
use crate::grid::Grid;
use crate::matrix::Matrix;
use crate::mirror::Mirror;
use crate::surface::{Damage, Panel, Surface};
use crate::toasters::Toasters;
use crate::toasters3::Toasters3;

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

/// Every saver, in the order the web mirror offers them. ONE table, because a
/// name list beside a dispatch match is a drift the compiler cannot see: a
/// saver added to the match but not the list is unreachable from the picker and
/// silently means ascii, and no test can enumerate a match's arms to catch it.
///
/// `SAVERS[0]` is the fallback for an unrecognised name. This pod is headless on
/// a remote node — a typo in `SAVER` must never crash-loop it.
///
/// Construction is not a trait method: grid geometry is only known after
/// modeset, and `fn new(&Panel) -> Self` is not object-safe. Each saver reads
/// its own env vars in its own constructor, so no central struct enumerates
/// every saver's knobs. Adding a saver is one row here.
/// Named because clippy is right that the bare tuple is a mouthful, and this is
/// the shape every row shares.
type Build = fn(&Panel, u32) -> Box<dyn Saver>;

const SAVERS: &[(&str, Build)] = &[
    ("ascii", |p, _| Box::new(Fire::ascii(p))),
    ("blocks", |p, _| Box::new(Fire::blocks(p))),
    ("matrix", |p, fps| Box::new(Matrix::new(p, fps))),
    ("toasters", |p, fps| Box::new(Toasters::new(p, fps))),
    ("toasters3", |p, fps| Box::new(Toasters3::new(p, fps))),
    ("city", |p, fps| Box::new(City::new(p, fps))),
];

/// The name at an index the render loop is holding. Panics on an index no
/// `index_of` produced, which is unreachable: the only writer is `select`.
pub fn name_at(i: usize) -> &'static str {
    SAVERS[i].0
}

/// Names in picker order, for `/meta`.
pub fn names() -> impl Iterator<Item = &'static str> {
    SAVERS.iter().map(|(n, _)| *n)
}

/// Position in `SAVERS`, or None for a name no saver answers to. The one place
/// a user-supplied name is validated — `make` cannot report a bad name.
pub fn index_of(name: &str) -> Option<usize> {
    SAVERS.iter().position(|(n, _)| *n == name)
}

pub fn make(name: &str, panel: &Panel, fps: u32) -> Box<dyn Saver> {
    let (_, build) = index_of(name).map_or(SAVERS[0], |i| SAVERS[i]);
    build(panel, fps)
}

/// Swap the saver if the mirror's selection moved. Shared verbatim by the DRM
/// loop and the dump loop for the same reason `frame` is: a dump that ran its
/// own copy of this would prove nothing about what runs on hardware.
///
/// Returns true when it switched, so the caller can log it on the thread that
/// actually draws — the HTTP thread cannot know whether the render loop is
/// running or parked in its no-monitor retry.
pub fn switch(
    saver: &mut Box<dyn Saver>,
    selected: &mut usize,
    mirror: &Mirror,
    panel: &Panel,
    fps: u32,
) -> bool {
    // One relaxed load per frame, same as the mirror's viewer count, and free
    // for the same reason: adjacent field, already-hot cache line, a plain load
    // with no barrier. Relaxed is right because the atomic publishes no data —
    // it indexes a const table that has existed since program start. Everything
    // a viewer observes travels through the meta and frame mutexes.
    let now = mirror.selected();
    if now == *selected {
        return false;
    }
    *selected = now;
    *saver = make(name_at(now), panel, fps);
    // The new grid geometry and palette differ, so this bumps the mirror's
    // epoch and every viewer reconnects onto the new /meta.
    announce(mirror, saver.as_ref());
    true
}

/// Tell the mirror what this saver draws through. Every construction of a saver
/// is followed by one of these — a viewer holding the previous saver's geometry
/// and palette would mis-draw every cell.
pub fn announce(mirror: &Mirror, s: &dyn Saver) {
    let g = s.grid();
    mirror.describe(
        s.name(),
        g.cols(),
        g.rows(),
        g.cell_w(),
        g.cell_h(),
        s.palette(),
    );
}

/// The per-frame body, shared verbatim by the DRM path and the dump path. A
/// dump that used its own loop would prove nothing about what runs on hardware.
#[inline]
pub fn frame(saver: &mut dyn Saver, buf: &mut [u32], panel: &Panel) -> Damage {
    let mut s = Surface::new(buf, panel);
    saver.render(&mut s);
    s.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One table makes NAMES-vs-make drift impossible, so what is left to check
    /// is a copy-paste inside a row: a name paired with the constructor for a
    /// different saver. That still renders the wrong thing for a valid name.
    /// Small panel, because only the dispatch is under test.
    #[test]
    fn every_row_builds_the_saver_it_names() {
        let panel = Panel::new(128, 128, 128);
        for (i, name) in names().enumerate() {
            assert_eq!(index_of(name), Some(i));
            assert_eq!(name_at(i), name);
            assert_eq!(make(name, &panel, 30).name(), name);
        }
        assert_eq!(index_of("nope"), None);
        // An unrecognised name must land on the first row, not panic.
        assert_eq!(make("nope", &panel, 30).name(), name_at(0));
    }
}
