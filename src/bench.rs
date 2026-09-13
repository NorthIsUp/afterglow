//! Interleaved per-saver microbench, in-process.
//!
//! `/usr/bin/time` on the dump binary measures the PPM writer (~300 us/frame)
//! more than it measures the saver, and a number from a separate run drifts
//! with whatever the CPU's thermal state was that minute. So every saver is
//! timed in the SAME process as the first name in the list, in short
//! alternating rounds: drift lands on both sides and the ratio survives it.
//!
//! The timed frame is `saver::frame` PLUS the shadow-to-hardware copy of
//! exactly the rects the frame reported — what simpledrm does inside
//! `dirty_framebuffer`, and where most of an object-sparse saver's cost lives.
//! `Mpx` is that copy in megapixels, and unlike the timings it is exact: the
//! same number on any machine, so it is the one to compare across commits.
//!
//! ```text
//! cargo test --release bench_savers -- --ignored --nocapture
//! BENCH_SAVERS=matrix,satori BENCH_ROUNDS=40 cargo test --release bench_savers -- --ignored --nocapture
//! ```

use std::time::{Duration, Instant};

use drm::control::ClipRect;

use crate::saver;
use crate::surface::{Panel, MAX_RUNS};

/// Frames per saver per round. Long enough to swamp the `Instant` pair, short
/// enough that a round is a fair slice of the same thermal conditions.
const CHUNK: usize = 50;

#[test]
#[ignore = "a benchmark, not a check: run it explicitly"]
fn bench_savers() {
    let names = crate::env_str(&["BENCH_SAVERS"], "matrix,satori,toasters2");
    let names: Vec<&str> = names.split(',').filter(|n| !n.is_empty()).collect();
    let rounds = crate::env_num(&["BENCH_ROUNDS"], 20, 1, 10_000) as usize;

    let w = crate::env_num(&["BENCH_W"], 1920, 64, 4096) as usize;
    let h = crate::env_num(&["BENCH_H"], 1080, 64, 4096) as usize;
    let panel = Panel::new(w, h, w);
    let mut savers: Vec<_> = names.iter().map(|n| saver::make(n, &panel, 30)).collect();
    let mut bufs: Vec<Vec<u32>> = names.iter().map(|_| vec![0u32; panel.buf_len()]).collect();
    // One per saver, standing in for the buffer simpledrm scans out of.
    let mut hw: Vec<Vec<u32>> = names.iter().map(|_| vec![0u32; panel.buf_len()]).collect();
    let mut total = vec![Duration::ZERO; names.len()];
    let mut px = vec![0usize; names.len()];
    let mut rects = [ClipRect::new(0, 0, 0, 0); MAX_RUNS];

    // Frame 0 is a full repaint for every saver and is not what runs for weeks.
    for (s, b) in savers.iter_mut().zip(bufs.iter_mut()) {
        saver::frame(s.as_mut(), b, &panel);
    }

    for _ in 0..rounds {
        for i in 0..names.len() {
            let t0 = Instant::now();
            for _ in 0..CHUNK {
                let d = saver::frame(savers[i].as_mut(), &mut bufs[i], &panel);
                let n = d.rects(&mut rects);
                for r in &rects[..n] {
                    let (x0, x1) = (r.x1() as usize, r.x2() as usize);
                    for y in r.y1() as usize..r.y2() as usize {
                        let o = y * panel.w;
                        hw[i][o + x0..o + x1].copy_from_slice(&bufs[i][o + x0..o + x1]);
                    }
                }
                px[i] += d.px();
            }
            total[i] += t0.elapsed();
        }
    }

    let frames = (rounds * CHUNK) as f64;
    let base = total[0].as_secs_f64() / frames * 1e6;
    println!(
        "\n{} frames/saver, {rounds} interleaved rounds",
        frames as u64
    );
    for ((name, t), p) in names.iter().zip(total.iter()).zip(px.iter()) {
        let us = t.as_secs_f64() / frames * 1e6;
        println!(
            "  {name:<12} {us:8.3} us/frame   {:6.3}x {}   {:6.3} Mpx/frame",
            us / base,
            names[0],
            *p as f64 / frames / 1e6,
        );
    }
}
