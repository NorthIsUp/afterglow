//! screensaver — animated pixels drawn straight into a DRM/KMS dumb buffer.
//!
//! # Why DRM and not /dev/fb0
//!
//! Talos v1.14.0 builds its kernel with `# CONFIG_FB is not set`, so `/dev/fbN`
//! does not exist on any node — verified on both pine and fir.
//! `CONFIG_DRM_FBDEV_EMULATION` is enabled, but it only feeds the in-kernel
//! console (`fbcon`), which is precisely the bare Linux terminal an otherwise
//! idle HDMI port displays. No device tree overlay can bring fbdev back: it is
//! compiled out, not merely unbound.
//!
//! The predecessor to this program opened `/dev/fb0`, so after that upgrade it
//! idled forever, logging "framebuffer not present" while the monitor showed a
//! console. There is deliberately **no fbdev fallback** here: no node has one,
//! and an untestable fallback path is worse than none.
//!
//! # Why Rust, and `FROM scratch`
//!
//! This runs privileged with the host's `/dev` mounted. The C version needed a
//! Debian userland — 33.5 MB of it, plus libdrm and fbset — to host a 72 KB
//! program. The `drm` crate issues raw ioctls, so a musl build links nothing but
//! libc, and the image is the binary alone: 207 KB, measured, a 166x reduction
//! with no package surface at all.
//!
//! The retry/idle loop lives here rather than in a shell wrapper, because
//! `FROM scratch` has no shell. That is not a loss: the loop was thirty lines
//! of sh whose only real job was printing diagnostics this can print better.
//!
//! # Layout
//!
//! * `surface` — the mapped frame and the damage contract. Read that first.
//! * `mirror` — the web mirror: the same cells the panel shows, over HTTP.
//! * `grid` / `font` — the character grid and the one glyph blitter.
//! * `fire` / `matrix` / `toasters` / `toasters3` / `city` — the savers. `saver` is the trait and the name -> saver
//!   dispatch; adding one is a module plus a row in `saver::SAVERS`.
//! * `ascii_rest` — ports of ascii.rest's pieces: one generic `Play` saver,
//!   each piece only its drawing code.
//! * `host` — DRM: modeset, mapping, dirty, teardown.
//! * `dump` — the same frame code rendered to PPM on a machine with no display,
//!   with the damage self-check that no monitor can perform.
//! * `term` — the same frame code printed into the terminal it runs in.
//!
//! # Environment
//!
//! * `DRM_DEVICE`     — card to open (default `/dev/dri/card0`)
//! * `SAVER`          — which saver starts, one of the names in
//!   `saver::SAVERS` (`ascii` is both the default and the fallback for an
//!   unrecognised name). The startup choice only: `POST /select?saver=<name>`
//!   on the web mirror switches it live, and a restart goes back to this.
//! * `SAVER_ROTATE_SECS` — seconds each saver holds the panel before another
//!   one is picked at random, 0..=86400. **0, the default, is off.** Never the
//!   saver already showing, and a `/select` gives the saver it picked a full
//!   interval before rotation moves on again.
//! * `SAVER_FPS`      — target frames/sec, 1..=120 (default 30)
//! * `SAVER_PANEL_MM`   — the panel's visible width in mm, 0 (default) = unknown.
//!   Not discoverable (this monitor's EDID is 0 bytes); someone measures it. Only
//!   the mirror page reads it, to offer a canvas the same PHYSICAL size as the
//!   panel. Nothing in the render path touches it.
//! * `SAVER_PIXEL_ASPECT` — how much taller than wide one framebuffer pixel
//!   lands on the panel, in per-cent, 25..=400. **100, the default, is off and
//!   is a byte-for-byte no-op.** Pine's monitor is a 1280x400 panel the firmware
//!   drives at 1920x1080 and which then rescales 1.5x across and 2.7x down, so
//!   everything reaches the glass squashed by 1.8: set it to 180 there. Applied
//!   once, in `Grid::new`, by making the cell that much taller — every saver
//!   draws in cells or in sub-cells of one, so the correction reaches all but
//!   the five that measure something in framebuffer pixels (`warp`, `moire`,
//!   `toasters*`, `confetti`, `podracer`), which carry it explicitly. Process-wide rather
//!   than per-saver: it is a property of the monitor, and a knob 25 savers each
//!   have to remember is a knob 25 savers get wrong. See the README.
//! * `RETRY_SECONDS`  — wait between attempts when no display is present (default 30)
//! * `SAVER_HTTP`     — address the web mirror listens on (default
//!   `127.0.0.1:8080`, which is the `tailscale-auth` sidecar's default upstream;
//!   `off` disables it). See `mirror.rs`.
//! * `SAVER_DUMP`     — render to PPM files in this directory instead of to a
//!   display, then exit. Also honours `SAVER_DUMP_FRAMES`, `SAVER_DUMP_EVERY`,
//!   `SAVER_WIDTH`, `SAVER_HEIGHT`.
//! * `SAVER_TERM`     — 1 animates the saver in this terminal instead of on a
//!   display, until Ctrl-C or `q`. Any OS with a truecolor terminal, no card
//!   needed. See `term.rs`.
//!
//! Per-saver knobs are documented in `README.md`, one row
//! per saver — it is the only complete list, and a second copy here goes stale.
//!
//! `FIRE_FPS` and `FIRE_STYLE` remain accepted as the older spellings of
//! `SAVER_FPS` and `SAVER` — the live deployment sets them, and its image digest
//! is bumped in a separate commit, so a new binary always runs against the old
//! env block first.

mod ascii_rest;
#[cfg(test)]
mod bench;
mod city;
mod confetti;
mod doodles;
mod dump;
mod dvd;
mod fire;
mod font;
mod fractal;
mod glyph;
mod grid;
mod hardrain;
mod host;
mod hypercube;
mod life;
mod lissajous;
mod marble;
mod matrix;
mod mirror;
mod moire;
mod plasma;
mod podracer;
mod pov;
mod rain;
mod sakura;
mod satori;
mod saver;
mod speeder;
mod strings;
mod surface;
mod tactiles;
mod term;
#[cfg(test)]
mod testalloc;
mod toasters;
mod toasters2;
mod toasters3;
mod warp;
mod worms;
mod xwing;
mod zot;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// First key PRESENT wins; present-but-unparseable or out-of-range falls back to
/// `default`. Clamp-or-default rather than fail-fast is the right shape for a
/// headless pod: a typo in an env var must never crash-loop it.
pub fn env_num(keys: &[&str], default: i64, lo: i64, hi: i64) -> i64 {
    for key in keys {
        let Ok(raw) = std::env::var(key) else {
            continue;
        };
        return match raw.parse::<i64>() {
            Ok(v) if v >= lo && v <= hi => v,
            _ => {
                eprintln!("[screensaver] {key}={raw:?} is not {lo}..={hi}; using {default}");
                default
            }
        };
    }
    default
}

/// A saver's RNG seed: `<SAVER>_SEED` if set, otherwise one rolled from the
/// clock and the pid so a pod restart shows a new scene.
///
/// The knob is not a convenience. A clock-seeded saver cannot be dumped and
/// compared frame-for-frame against another build, and that comparison is how
/// every rendering change in this tree is shown to be a no-op — four savers
/// shipped without one and were simply unverifiable. `sakura` had it right
/// first; this is that code, shared.
///
/// The clock alone is a poor seed (two pods starting in the same second draw
/// the same scene), so the pid mixes in, and the pair is stirred rather than
/// used raw.
pub fn saver_seed(keys: &[&str], fallback: u32) -> u32 {
    let pinned = env_num(keys, 0, 0, u32::MAX as i64) as u32;
    if pinned != 0 {
        return pinned;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() ^ d.as_secs() as u32)
        .unwrap_or(fallback);
    let mut seed = nanos ^ std::process::id().wrapping_mul(0x9E37_79B9);
    next_rand(&mut seed);
    seed.max(1)
}

/// splitmix32, not the LCG fire and matrix use. Those draw one number per cell,
/// where the LCG's correlation between successive outputs is invisible; a saver
/// that draws a PAIR from consecutive outputs gets a visible diagonal clump out
/// of it. Toasters found that in a dump, which is why this exists.
#[inline]
pub fn next_rand(rng: &mut u32) -> u32 {
    *rng = rng.wrapping_add(0x9E37_79B9);
    let mut z = *rng;
    z = (z ^ (z >> 16)).wrapping_mul(0x85EB_CA6B);
    z = (z ^ (z >> 13)).wrapping_mul(0xC2B2_AE35);
    (z ^ (z >> 16)) >> 1
}

/// As `env_num`, for the values that are names rather than numbers.
pub fn env_str(keys: &[&str], default: &str) -> String {
    keys.iter()
        .find_map(|k| std::env::var(k).ok())
        .unwrap_or_else(|| default.to_string())
}

pub struct Config {
    device: String,
    fps: u32,
    saver: String,
    rotate_secs: u64,
    retry: Duration,
    dump: Option<String>,
    term: bool,
    http: String,
}

impl Config {
    fn from_env() -> Self {
        Self {
            device: std::env::var("DRM_DEVICE").unwrap_or_else(|_| "/dev/dri/card0".into()),
            fps: env_num(&["SAVER_FPS", "FIRE_FPS"], 30, 1, 120) as u32,
            // Unrecognised names land on fire-ascii, which is what FIRE_STYLE
            // has always done — it tested `!= "blocks"`. The only value whose
            // meaning changes is "matrix", which used to mean ascii.
            saver: env_str(&["SAVER", "FIRE_STYLE"], "ascii"),
            rotate_secs: env_num(&["SAVER_ROTATE_SECS"], 0, 0, 86_400) as u64,
            retry: Duration::from_secs(env_num(&["RETRY_SECONDS"], 30, 1, 3600) as u64),
            dump: std::env::var("SAVER_DUMP").ok(),
            term: env_num(&["SAVER_TERM"], 0, 0, 1) == 1,
            // Loopback by default: the only thing that should reach the mirror
            // is the tailscale-auth gate sharing this pod's netns.
            http: env_str(&["SAVER_HTTP"], "127.0.0.1:8080"),
        }
    }
}

fn main() {
    let cfg = Config::from_env();

    // SIGTERM/SIGINT set the flag so the render loop exits and the CRTC is
    // restored. Kubernetes sends SIGTERM on pod shutdown; without this the last
    // frame would stay frozen on the panel.
    install_signal_handlers();

    // Started before anything else and outliving every retry of the display
    // loop, so the mirror answers (with a 503 from /meta) on a node whose
    // monitor is absent — which is exactly when someone is asking why. A dump
    // drives it too, which is how it is testable with no card at all.
    let mirror = mirror::Mirror::new(cfg.fps);
    // The env var is the startup default; after that the web UI owns the choice,
    // and both loops build their saver from this selection rather than reading
    // cfg.saver again. An unrecognised name is refused here and leaves the
    // selection at row 0 — the same fallback `make` has always had, now reached
    // through the one validation point instead of a second path beside it.
    if !mirror.select(&cfg.saver) {
        eprintln!(
            "[screensaver] SAVER={} is not a saver, using {}",
            cfg.saver,
            saver::name_at(0)
        );
    }
    // Same deal as SAVER: the env var is the startup default and the web UI
    // owns it after that, so the render loop has one place to read it from
    // rather than an env lookup and a live value that can disagree.
    mirror.set_rotate_secs(cfg.rotate_secs);
    if cfg.http != "off" {
        let (m, addr) = (Arc::clone(&mirror), cfg.http.clone());
        std::thread::spawn(move || mirror::serve(m, &addr));
    }

    // Checked before anything touches DRM, so a dump runs on a laptop with no
    // card at all.
    if let Some(dir) = &cfg.dump {
        if let Err(e) = dump::run_dump(dir, &cfg, &mirror) {
            eprintln!("[screensaver] {e}");
            std::process::exit(1);
        }
        return;
    }
    if cfg.term {
        if let Err(e) = term::run(&cfg, &mirror) {
            eprintln!("[screensaver] {e}");
            std::process::exit(1);
        }
        return;
    }

    eprintln!("[screensaver] starting; DRM_DEVICE={}", cfg.device);

    // Talos is headless and immutable: there is no console login, just this pod.
    // A missing display is not fatal — idle and retry so the pod stays Running
    // and its logs stay readable.
    loop {
        if SIGNALLED.load(Ordering::Relaxed) {
            return;
        }
        match host::run(&cfg, &mirror) {
            Ok(()) => return,
            Err(e) => {
                eprintln!("[screensaver] {e}");
                eprintln!(
                    "[screensaver] no usable display. Expected when no monitor is on this \
                     node, or when it was connected after boot — the framebuffer is handed \
                     over by U-Boot at boot time. Retrying in {}s.",
                    cfg.retry.as_secs()
                );
                std::thread::sleep(cfg.retry);
            }
        }
    }
}

/// Set by the signal handler, read by the render loop. A plain static rather
/// than a shared Arc so the handler touches nothing but an atomic store, which
/// is the only thing that is async-signal-safe.
pub static SIGNALLED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_sig: libc::c_int) {
    SIGNALLED.store(true, Ordering::Relaxed);
}

fn install_signal_handlers() {
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
    }
}
