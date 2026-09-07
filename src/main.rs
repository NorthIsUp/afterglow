//! fbfire — Doom-fire drawn straight into a DRM/KMS dumb buffer.
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
//! # Environment
//!
//! * `DRM_DEVICE`    — card to open (default `/dev/dri/card0`)
//! * `FIRE_FPS`      — target frames/sec, 1..=120 (default 30)
//! * `FIRE_STYLE`    — `ascii` (default) or `blocks`
//! * `FIRE_SCALE`    — blocks mode: fire grid = panel width / scale, 1..=16 (default 4)
//! * `FIRE_CELL`     — ascii mode: character cell in px, 8..=64 (default 16)
//! * `RETRY_SECONDS` — wait between attempts when no display is present (default 30)

use std::fs::{File, OpenOptions};
use std::os::fd::{AsFd, BorrowedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use drm::buffer::Buffer;
use drm::control::{connector, crtc, Device as ControlDevice};
use drm::Device as BasicDevice;
use drm_fourcc::DrmFourcc;

/// 37-step Doom fire palette (RGB). 0 = cooled black, 36 = white-hot.
const PAL: [[u8; 3]; 37] = [
    [0x07, 0x07, 0x07],
    [0x1F, 0x07, 0x07],
    [0x2F, 0x0F, 0x07],
    [0x47, 0x0F, 0x07],
    [0x57, 0x17, 0x07],
    [0x67, 0x1F, 0x07],
    [0x77, 0x1F, 0x07],
    [0x8F, 0x27, 0x07],
    [0x9F, 0x2F, 0x07],
    [0xAF, 0x3F, 0x07],
    [0xBF, 0x47, 0x07],
    [0xC7, 0x47, 0x07],
    [0xDF, 0x4F, 0x07],
    [0xDF, 0x57, 0x07],
    [0xDF, 0x57, 0x07],
    [0xD7, 0x5F, 0x07],
    [0xD7, 0x5F, 0x07],
    [0xD7, 0x67, 0x0F],
    [0xCF, 0x6F, 0x0F],
    [0xCF, 0x77, 0x0F],
    [0xCF, 0x7F, 0x0F],
    [0xCF, 0x87, 0x17],
    [0xC7, 0x87, 0x17],
    [0xC7, 0x8F, 0x17],
    [0xC7, 0x97, 0x1F],
    [0xBF, 0x9F, 0x1F],
    [0xBF, 0x9F, 0x1F],
    [0xBF, 0xA7, 0x27],
    [0xBF, 0xA7, 0x27],
    [0xBF, 0xAF, 0x2F],
    [0xB7, 0xAF, 0x2F],
    [0xB7, 0xB7, 0x2F],
    [0xB7, 0xB7, 0x37],
    [0xCF, 0xCF, 0x6F],
    [0xDF, 0xDF, 0x9F],
    [0xEF, 0xEF, 0xC7],
    [0xFF, 0xFF, 0xFF],
];

/// Heat ramp, cool/empty -> hot/dense.
const RAMP: [u8; 10] = *b" .:-=+*#%@";

/// Minimal 8x8 font, MSB = leftmost pixel. Only the RAMP glyphs exist; anything
/// else renders blank.
const FONT: [(u8, [u8; 8]); 10] = [
    (b' ', [0, 0, 0, 0, 0, 0, 0, 0]),
    (b'.', [0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x18, 0x00]),
    (b':', [0x00, 0x18, 0x18, 0x00, 0x00, 0x18, 0x18, 0x00]),
    (b'-', [0x00, 0x00, 0x00, 0x7E, 0x00, 0x00, 0x00, 0x00]),
    (b'=', [0x00, 0x00, 0x7E, 0x00, 0x7E, 0x00, 0x00, 0x00]),
    (b'+', [0x00, 0x18, 0x18, 0x7E, 0x18, 0x18, 0x00, 0x00]),
    (b'*', [0x00, 0x66, 0x3C, 0xFF, 0x3C, 0x66, 0x00, 0x00]),
    (b'#', [0x66, 0xFF, 0x66, 0x66, 0x66, 0xFF, 0x66, 0x00]),
    (b'%', [0xC6, 0xCC, 0x18, 0x30, 0x66, 0xC6, 0x00, 0x00]),
    (b'@', [0x3C, 0x42, 0x99, 0xA5, 0xA5, 0x9E, 0x40, 0x3C]),
];

fn glyph(c: u8) -> &'static [u8; 8] {
    for (ch, rows) in FONT.iter() {
        if *ch == c {
            return rows;
        }
    }
    &FONT[0].1
}

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

fn env_num(key: &str, default: i64, lo: i64, hi: i64) -> i64 {
    match std::env::var(key).ok().and_then(|v| v.parse::<i64>().ok()) {
        Some(v) if v >= lo && v <= hi => v,
        _ => default,
    }
}

struct Config {
    device: String,
    fps: u32,
    ascii: bool,
    scale: u32,
    cell: u32,
    retry: Duration,
}

impl Config {
    fn from_env() -> Self {
        Self {
            device: std::env::var("DRM_DEVICE").unwrap_or_else(|_| "/dev/dri/card0".into()),
            fps: env_num("FIRE_FPS", 30, 1, 120) as u32,
            ascii: std::env::var("FIRE_STYLE")
                .map(|s| s != "blocks")
                .unwrap_or(true),
            scale: env_num("FIRE_SCALE", 4, 1, 16) as u32,
            cell: env_num("FIRE_CELL", 16, 8, 64) as u32,
            retry: Duration::from_secs(env_num("RETRY_SECONDS", 30, 1, 3600) as u64),
        }
    }
}

/// Fire simulation over a low-res grid, propagated upward Doom-style.
struct Fire {
    w: usize,
    h: usize,
    cells: Vec<u8>,
    rng: u32,
}

impl Fire {
    fn new(w: usize, h: usize) -> Self {
        let w = w.max(1);
        let h = h.max(1);
        let mut cells = vec![0u8; w * h];
        // White-hot source row along the bottom.
        for x in 0..w {
            cells[(h - 1) * w + x] = 36;
        }
        Self {
            w,
            h,
            cells,
            rng: 0x9e37_79b9,
        }
    }

    fn step(&mut self) {
        for x in 0..self.w {
            for y in 1..self.h {
                let src = y * self.w + x;
                let v = self.cells[src];
                if v == 0 {
                    self.cells[src - self.w] = 0;
                    continue;
                }
                self.rng = self.rng.wrapping_mul(1103515245).wrapping_add(12345);
                let rnd = (self.rng >> 16) & 3;
                let drift = (rnd & 1) as usize;
                let dst = src.saturating_sub(self.w + drift);
                self.cells[dst] = v.saturating_sub((rnd & 1) as u8);
            }
        }
    }

    fn at(&self, x: usize, y: usize) -> u8 {
        self.cells[y.min(self.h - 1) * self.w + x.min(self.w - 1)]
    }
}

/// Everything needed to draw one frame into a mapped XRGB8888 buffer.
struct Panel {
    w: u32,
    h: u32,
    pitch: u32,
}

impl Panel {
    /// Blocks mode: scale the fire grid straight to the panel.
    fn draw_blocks(&self, buf: &mut [u8], fire: &Fire, pal: &[u32; 37]) {
        for y in 0..self.h as usize {
            let fy = y * fire.h / self.h as usize;
            let row = &mut buf[y * self.pitch as usize..][..self.w as usize * 4];
            for x in 0..self.w as usize {
                let fx = x * fire.w / self.w as usize;
                let px = pal[fire.at(fx, fy) as usize];
                row[x * 4..x * 4 + 4].copy_from_slice(&px.to_ne_bytes());
            }
        }
    }

    /// Ascii mode: one fire sample per character cell, blitted as a glyph.
    fn draw_ascii(&self, buf: &mut [u8], fire: &Fire, pal: &[u32; 37], cell: u32) {
        let cols = (self.w / cell) as usize;
        let rows = (self.h / cell) as usize;
        let cell = cell as usize;
        for cy in 0..rows {
            for cx in 0..cols {
                let heat = fire.at(cx, cy);
                let ch = RAMP[(heat as usize * (RAMP.len() - 1)) / 36];
                let colour = pal[heat as usize].to_ne_bytes();
                let bits = glyph(ch);
                let (ox, oy) = (cx * cell, cy * cell);
                for py in 0..cell {
                    let gy = py * 8 / cell;
                    let line = bits[gy];
                    let start = (oy + py) * self.pitch as usize + ox * 4;
                    let row = &mut buf[start..start + cell * 4];
                    for px in 0..cell {
                        let gx = px * 8 / cell;
                        let lit = ch != b' ' && (line & (0x80 >> gx)) != 0;
                        let v = if lit { colour } else { [0, 0, 0, 0] };
                        row[px * 4..px * 4 + 4].copy_from_slice(&v);
                    }
                }
            }
        }
    }
}

/// Open the card, modeset the connector's preferred mode, and run until stopped.
/// Returns Ok(()) when asked to stop, Err on any setup failure so the caller can
/// idle and retry rather than crash-looping.
fn run(cfg: &Config) -> Result<(), String> {
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
    let panel = Panel {
        w: w as u32,
        h: h as u32,
        pitch,
    };
    eprintln!(
        "[screensaver] drm {} {}x{}@{}Hz crtc={:?} pitch={} style={} fps={}",
        cfg.device,
        w,
        h,
        mode.vrefresh(),
        crtc_handle,
        pitch,
        if cfg.ascii { "ascii" } else { "blocks" },
        cfg.fps
    );

    let pal: [u32; 37] = {
        let mut p = [0u32; 37];
        for (i, c) in PAL.iter().enumerate() {
            // XRGB8888 little-endian: 0x00RRGGBB
            p[i] = (u32::from(c[0]) << 16) | (u32::from(c[1]) << 8) | u32::from(c[2]);
        }
        p
    };

    let (fw, fh) = if cfg.ascii {
        (
            (w as u32 / cfg.cell) as usize,
            (h as u32 / cfg.cell) as usize,
        )
    } else {
        (
            (w as u32 / cfg.scale) as usize,
            (h as u32 / cfg.scale) as usize,
        )
    };
    let mut fire = Fire::new(fw, fh);

    let frame = Duration::from_nanos(1_000_000_000 / u64::from(cfg.fps));
    while !SIGNALLED.load(Ordering::Relaxed) {
        let t0 = Instant::now();
        fire.step();
        {
            let mut map = card
                .map_dumb_buffer(&mut db)
                .map_err(|e| format!("map_dumb_buffer: {e}"))?;
            let buf = map.as_mut();
            if cfg.ascii {
                panel.draw_ascii(buf, &fire, &pal, cfg.cell);
            } else {
                panel.draw_blocks(buf, &fire, &pal);
            }
        }
        if let Some(rem) = frame.checked_sub(t0.elapsed()) {
            std::thread::sleep(rem);
        }
    }

    // Blank, then hand the CRTC back so fbcon returns instead of a frozen frame.
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

fn main() {
    let cfg = Config::from_env();

    // SIGTERM/SIGINT set the flag so the render loop exits and the CRTC is
    // restored. Kubernetes sends SIGTERM on pod shutdown; without this the last
    // frame would stay frozen on the panel.
    install_signal_handlers();

    eprintln!("[screensaver] starting; DRM_DEVICE={}", cfg.device);

    // Talos is headless and immutable: there is no console login, just this pod.
    // A missing display is not fatal — idle and retry so the pod stays Running
    // and its logs stay readable.
    loop {
        if SIGNALLED.load(Ordering::Relaxed) {
            return;
        }
        match run(&cfg) {
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
static SIGNALLED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_sig: libc::c_int) {
    SIGNALLED.store(true, Ordering::Relaxed);
}

fn install_signal_handlers() {
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
    }
}
