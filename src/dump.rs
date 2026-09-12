//! Headless rendering to PPM, plus the damage self-check.
//!
//! This exists for one bug: a saver that writes pixels it does not report. On
//! simpledrm that region shows a stale frame forever, and it reproduces on
//! hardware and nowhere else — there is no monitor in CI and no way to look at
//! the panel from a laptop. So the check runs here, against the same
//! `saver::frame` the DRM host calls, and fails the process.

use std::io::Write;
use std::time::{Duration, Instant};

use crate::mirror::Mirror;
use crate::saver;
use crate::surface::{Damage, Panel};
use crate::{env_num, Config};

/// Binary PPM (P6). No dependency, and every image viewer and ffmpeg reads it.
fn write_ppm(dir: &str, n: usize, buf: &[u32], p: &Panel) -> Result<(), String> {
    let path = format!("{dir}/frame-{n:05}.ppm");
    let mut out = Vec::with_capacity(p.w * p.h * 3 + 32);
    out.extend_from_slice(format!("P6\n{} {}\n255\n", p.w, p.h).as_bytes());
    for row in buf.chunks(buf.len() / p.h) {
        for &v in &row[..p.w] {
            out.push((v >> 16) as u8);
            out.push((v >> 8) as u8);
            out.push(v as u8);
        }
    }
    std::fs::write(&path, &out).map_err(|e| format!("write {path}: {e}"))
}

/// Any scanline whose visible pixels changed but which no run covers is an
/// UNDER-REPORT: on simpledrm that region would show a stale frame forever.
fn verify(prev: &[u32], cur: &[u32], d: &Damage, p: &Panel, n: usize) -> Result<(), String> {
    let stride = cur.len() / p.h;
    for y in 0..p.h {
        let a = &prev[y * stride..][..p.w];
        let b = &cur[y * stride..][..p.w];
        if a == b {
            continue;
        }
        let covered = d.runs().iter().any(|&(y0, y1)| {
            let y = y as u16;
            y >= y0 && y < y1
        });
        if !covered {
            return Err(format!(
                "[dump] frame {n}: scanline {y} changed but not reported"
            ));
        }
    }
    Ok(())
}

pub fn run_dump(dir: &str, cfg: &Config, mirror: &Mirror) -> Result<(), String> {
    let frames = env_num(&["SAVER_DUMP_FRAMES"], 30, 1, 100_000) as usize;
    let every = env_num(&["SAVER_DUMP_EVERY"], 10, 1, 100_000) as usize;
    let w = env_num(&["SAVER_WIDTH"], 1920, 64, 4096) as usize;
    let h = env_num(&["SAVER_HEIGHT"], 1080, 64, 4096) as usize;

    // stride32 == w: a dump has no pitch alignment to honour.
    let panel = Panel::new(w, h, w);
    // Zeroed, exactly as create_dumb_buffer hands it over — so a saver that
    // fails to paint frame 0 in full shows up here as black, same as on the panel.
    let mut buf = vec![0u32; panel.buf_len()];
    let mut check = vec![0u32; buf.len()];
    // Built from the mirror's selection, not cfg.saver: one validated path,
    // so make's fallback arm stops being load-bearing for user input.
    let mut saver = saver::make(saver::name_at(mirror.selected()), &panel, cfg.fps);

    // The mirror is fed from here too, so it is exercisable on a laptop with no
    // card — the same argument that put the damage self-check in this file.
    // Paced at SAVER_FPS when it is live, so a dump of many frames is a live
    // mirror rather than a burst; `SAVER_HTTP=off` keeps a dump instant.
    let paced =
        (cfg.http != "off").then(|| Duration::from_nanos(1_000_000_000 / u64::from(cfg.fps)));
    saver::announce(mirror, saver.as_ref());
    let mut selected = mirror.selected();

    std::fs::create_dir_all(dir).map_err(|e| format!("mkdir {dir}: {e}"))?;
    let log_path = format!("{dir}/damage.txt");
    let mut log =
        std::fs::File::create(&log_path).map_err(|e| format!("create {log_path}: {e}"))?;
    eprintln!(
        "[screensaver] dump saver={} {}x{} frames={frames} every={every} -> {dir}",
        saver.name(),
        w,
        h
    );

    for n in 0..frames {
        let t0 = Instant::now();
        // The same switch the DRM host honours, so /select is exercisable on a
        saver::switch(&mut saver, &mut selected, mirror, &panel, cfg.fps);
        check.copy_from_slice(&buf);
        let damage = saver::frame(saver.as_mut(), &mut buf, &panel);
        mirror.publish(saver.grid().cells());
        verify(&check, &buf, &damage, &panel, n)?;
        writeln!(
            log,
            "frame {n} runs={} rows={} {:?}",
            damage.runs().len(),
            damage.rows(),
            damage.runs()
        )
        .map_err(|e| format!("write {log_path}: {e}"))?;
        if n % every == 0 {
            write_ppm(dir, n, &buf, &panel)?;
        }
        if let Some(rem) = paced.and_then(|d| d.checked_sub(t0.elapsed())) {
            std::thread::sleep(rem);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::Surface;

    /// The check is only worth having if it fires, and nothing that goes through
    /// `Surface` can make it fire — that is the contract. So drive it directly.
    #[test]
    fn an_unreported_scanline_is_an_error() {
        let panel = Panel::new(4, 4, 4);
        let prev = vec![0u32; panel.buf_len()];
        let mut cur = prev.clone();
        cur[2 * 4 + 1] = 0xFF;

        let mut scratch = vec![0u32; panel.buf_len()];
        let mut s = Surface::new(&mut scratch, &panel);
        for row in s.cell_rows(0, 0, 4, 1) {
            row.fill(0);
        }
        // Row 0 reported, row 2 changed: the frozen-region bug, caught.
        assert!(verify(&prev, &cur, &s.finish(), &panel, 7).is_err());

        let mut s = Surface::new(&mut scratch, &panel);
        for row in s.cell_rows(0, 2, 4, 1) {
            row.fill(0);
        }
        assert!(verify(&prev, &cur, &s.finish(), &panel, 7).is_ok());
    }

    #[test]
    fn an_unchanged_frame_needs_no_damage() {
        let panel = Panel::new(4, 4, 4);
        let buf = vec![0u32; panel.buf_len()];
        assert!(verify(&buf, &buf, &Damage::new(), &panel, 0).is_ok());
    }
}
