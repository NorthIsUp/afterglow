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
//! * `fire` / `matrix` — the savers. `saver` is the trait and the name -> saver
//!   dispatch; adding one is a module plus an arm in `saver::make`.
//! * `host` — DRM: modeset, mapping, dirty, teardown.
//! * `dump` — the same frame code rendered to PPM on a machine with no display,
//!   with the damage self-check that no monitor can perform.
//!
//! # Environment
//!
//! * `DRM_DEVICE`     — card to open (default `/dev/dri/card0`)
//! * `SAVER`          — `ascii` (default), `blocks`, or `matrix`
//! * `SAVER_FPS`      — target frames/sec, 1..=120 (default 30)
//! * `FIRE_CELL`      — ascii fire: character cell in px, 8..=64 (default 16)
//! * `FIRE_SCALE`     — blocks fire: cell in px, 1..=16 (default 4)
//! * `MATRIX_CELL_W`  — matrix: cell width in px, 8..=64 (default 16)
//! * `MATRIX_CELL_H`  — matrix: cell height in px, 8..=128 (default 32)
//! * `RETRY_SECONDS`  — wait between attempts when no display is present (default 30)
//! * `SAVER_HTTP`     — address the web mirror listens on (default
//!   `127.0.0.1:8080`, which is the `tailscale-auth` sidecar's default upstream;
//!   `off` disables it). See `mirror.rs`.
//! * `SAVER_DUMP`     — render to PPM files in this directory instead of to a
//!   display, then exit. Also honours `SAVER_DUMP_FRAMES`, `SAVER_DUMP_EVERY`,
//!   `SAVER_WIDTH`, `SAVER_HEIGHT`.
//!
//! `FIRE_FPS` and `FIRE_STYLE` remain accepted as the older spellings of
//! `SAVER_FPS` and `SAVER` — the live deployment sets them, and its image digest
//! is bumped in a separate commit, so a new binary always runs against the old
//! env block first.

mod dump;
mod fire;
mod font;
mod grid;
mod host;
mod matrix;
mod mirror;
mod saver;
mod surface;

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
    retry: Duration,
    dump: Option<String>,
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
            retry: Duration::from_secs(env_num(&["RETRY_SECONDS"], 30, 1, 3600) as u64),
            dump: std::env::var("SAVER_DUMP").ok(),
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
    let mirror = mirror::Mirror::new();
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
