//! The DRM host: open the card, modeset, map the dumb buffer, hand it to a
//! saver once per frame, report the damage, and give the console back.

use std::fs::{File, OpenOptions};
use std::os::fd::{AsFd, BorrowedFd};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use drm::buffer::Buffer;
use drm::control::{connector, crtc, ClipRect, Device as ControlDevice};
use drm::Device as BasicDevice;
use drm_fourcc::DrmFourcc;

use crate::mirror::Mirror;
use crate::saver;
use crate::surface::{Panel, MAX_RUNS};
use crate::{Config, SIGNALLED};

/// A DRM card. The `drm` crate's traits are blanket-implemented for anything
/// that can hand over a borrowed fd.
struct Card(File);

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}
impl BasicDevice for Card {}
impl ControlDevice for Card {}

/// Sleep out what is left of the frame budget, or count the overrun. A
/// separate function only because the card-bound render loop around it cannot
/// be run in CI, and this is the branch that has to be right before anyone
/// raises `SAVER_FPS` against the pod's 500m CFS quota.
fn pace(mirror: &Mirror, frame_dur: Duration, t0: Instant) {
    match frame_dur.checked_sub(t0.elapsed()) {
        Some(rem) => std::thread::sleep(rem),
        // The panel missed its rate. COUNTED, not logged: a log line per frame
        // at 30fps is its own outage. Read it from /stat.
        None => mirror.overran(),
    }
}

/// Open the card, modeset the connector's preferred mode, and run until stopped.
/// Returns Ok(()) when asked to stop, Err on any setup failure so the caller can
/// idle and retry rather than crash-looping.
pub fn run(cfg: &Config, mirror: &Mirror) -> Result<(), String> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&cfg.device)
        .map_err(|e| format!("open {}: {e}", cfg.device))?;
    let card = Card(file);

    // Best effort: fbcon owns the console, and modesetting needs master. A
    // container handed the device is normally granted it.
    let _ = card.acquire_master_lock();

    let res = card
        .resource_handles()
        .map_err(|e| format!("resource_handles: {e}"))?;

    let conn = res
        .connectors()
        .iter()
        .filter_map(|h| card.get_connector(*h, false).ok())
        .find(|c| c.state() == connector::State::Connected && !c.modes().is_empty())
        .ok_or_else(|| "no connected connector with modes".to_string())?;

    // modes()[0] is the driver's preferred/native mode.
    let mode = conn.modes()[0];
    let (w, h) = mode.size();

    // Prefer the CRTC already driving this connector, else the first its
    // encoders can reach.
    let crtc_handle: crtc::Handle = conn
        .current_encoder()
        .and_then(|e| card.get_encoder(e).ok())
        .and_then(|e| e.crtc())
        .or_else(|| {
            // possible_crtcs() is an opaque CrtcListFilter, not a bitmask we can
            // index ourselves; filter_crtcs resolves it against the resource list.
            conn.encoders()
                .iter()
                .filter_map(|h| card.get_encoder(*h).ok())
                .flat_map(|e| res.filter_crtcs(e.possible_crtcs()))
                .next()
        })
        .ok_or_else(|| "no usable CRTC".to_string())?;

    let mut db = card
        .create_dumb_buffer((w as u32, h as u32), DrmFourcc::Xrgb8888, 32)
        .map_err(|e| format!("create_dumb_buffer: {e}"))?;
    let fb = card
        .add_framebuffer(&db, 24, 32)
        .map_err(|e| format!("add_framebuffer: {e}"))?;

    let saved = card.get_crtc(crtc_handle).ok();
    card.set_crtc(crtc_handle, Some(fb), (0, 0), &[conn.handle()], Some(mode))
        .map_err(|e| format!("set_crtc: {e}"))?;

    let pitch = db.pitch();
    // A 32bpp pitch is always a multiple of 4, so this division is exact.
    let panel = Panel::new(w as usize, h as usize, pitch as usize / 4);
    // Built from the mirror's selection, not cfg.saver: one validated path,
    // so make's fallback arm stops being load-bearing for user input.
    let mut saver = saver::make(saver::name_at(mirror.selected()), &panel, cfg.fps);
    eprintln!(
        "[screensaver] drm {} {}x{}@{}Hz crtc={:?} pitch={} saver={} fps={}",
        cfg.device,
        w,
        h,
        mode.vrefresh(),
        crtc_handle,
        pitch,
        saver.name(),
        cfg.fps
    );

    // Geometry, palette and glyph table are fixed until the saver changes, so
    // the mirror is told once and every frame after it is only cells.
    saver::announce(mirror, saver.as_ref(), &panel);
    let mut selected = mirror.selected();
    // The interval is the mirror's; this is only the deadline it implies. Built
    // here, not once in main, so a modeset retry gives the current saver a full
    // turn — and it picks up whatever `/rotate` has been set to since.
    let mut rot = saver::Rotate::new(Instant::now());

    let frame_dur = Duration::from_nanos(1_000_000_000 / u64::from(cfg.fps));
    // simpledrm — the driver U-Boot hands over on a Pi5 — scans out of a SHADOW
    // buffer. Writing into the mapping is not enough: the driver only copies to
    // the hardware when told which regions changed. Without this the initial
    // set_crtc displays frame 0 (a freshly zeroed buffer, i.e. black) and every
    // frame after it lands in memory nothing ever reads. That is precisely the
    // "screen went blank and never animated" symptom. See src/surface.rs for the
    // contract that keeps a saver from under-reporting.
    let mut rects = [ClipRect::new(0, 0, 0, 0); MAX_RUNS];
    let mut dirty_unsupported = false;

    while !SIGNALLED.load(Ordering::Relaxed) {
        let t0 = Instant::now();
        // One relaxed load per frame, same as the mirror's viewer count. The
        // new saver's grid geometry and palette differ, so announcing it bumps
        if saver::switch(
            &mut saver,
            &mut selected,
            &mut rot,
            t0,
            mirror,
            &panel,
            cfg.fps,
        ) {
            eprintln!("[screensaver] now drawing {}", saver.name());
        }
        let damage = {
            let mut map = card
                .map_dumb_buffer(&mut db)
                .map_err(|e| format!("map_dumb_buffer: {e}"))?;
            // One safe cast per frame, replacing a 4-byte slice copy per pixel.
            let buf: &mut [u32] = bytemuck::cast_slice_mut(map.as_mut());
            saver::frame(saver.as_mut(), buf, &panel)
        };

        // After the flush, so `cells()` is the frame that just went to the
        // panel. Costs one atomic load with nobody watching; see mirror.rs for
        // why this can never make the display wait.
        mirror.publish(saver.grid().cells());

        // Drivers that scan out directly have no need for this and answer
        // ENOSYS/EINVAL; note it once and stop asking rather than logging per
        // frame at 30fps. A frame where nothing moved dirties nothing.
        if !dirty_unsupported && !damage.is_empty() {
            let n = damage.rects(&mut rects);
            if let Err(e) = card.dirty_framebuffer(fb, &rects[..n]) {
                eprintln!(
                    "[screensaver] dirty_framebuffer unsupported ({e}); assuming direct scanout"
                );
                dirty_unsupported = true;
            }
        }
        pace(mirror, frame_dur, t0);
    }

    // Blank, then hand the CRTC back so fbcon returns instead of a frozen frame.
    // The fill is never dirtied and so never reaches a shadow-buffer driver; the
    // set_crtc restore below is the load-bearing half.
    if let Ok(mut map) = card.map_dumb_buffer(&mut db) {
        map.as_mut().fill(0);
    }
    if let Some(s) = saved {
        let _ = card.set_crtc(
            crtc_handle,
            s.framebuffer(),
            s.position(),
            &[conn.handle()],
            s.mode(),
        );
    }
    let _ = card.destroy_framebuffer(fb);
    let _ = card.destroy_dumb_buffer(db);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of the counter: a frame that ran past its budget is
    /// counted, and one that did not is not. CI has no card, so the render loop
    /// itself never runs here — this branch is the testable half of it.
    #[test]
    fn a_late_frame_is_counted_and_an_early_one_is_not() {
        let m = Mirror::new(15);
        let budget = Duration::from_millis(20);

        // Frame that took longer than the budget: nothing left to sleep.
        let late = Instant::now() - Duration::from_millis(50);
        pace(&m, budget, late);
        assert_eq!(m.overruns(), 1);
        pace(&m, budget, late);
        assert_eq!(m.overruns(), 2);

        // Frame that finished inside the budget: sleeps out the remainder and
        // counts nothing. The elapsed check is what fails if the arms are
        // swapped — a swapped `pace` returns instantly here.
        let t0 = Instant::now();
        pace(&m, budget, t0);
        assert_eq!(m.overruns(), 2);
        assert!(t0.elapsed() >= budget, "did not sleep: {:?}", t0.elapsed());
    }
}
