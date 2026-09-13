//! Interleaved per-saver microbench, in-process.
//!
//! `/usr/bin/time` on the dump binary measures the PPM writer (~300 us/frame)
//! more than it measures the saver, and a number from a separate run drifts
//! with whatever the CPU's thermal state was that minute. So every saver is
//! timed in the SAME process as the first name in the list, in short
//! alternating rounds: drift lands on both sides and the ratio survives it.
//!
//! ```text
//! cargo test --release bench_savers -- --ignored --nocapture
//! BENCH_SAVERS=matrix,satori BENCH_ROUNDS=40 cargo test --release bench_savers -- --ignored --nocapture
//! ```

use std::time::{Duration, Instant};

use crate::saver;
use crate::surface::Panel;

/// Frames per saver per round. Long enough to swamp the `Instant` pair, short
/// enough that a round is a fair slice of the same thermal conditions.
const CHUNK: usize = 50;

#[test]
#[ignore = "a benchmark, not a check: run it explicitly"]
fn bench_savers() {
    let names = crate::env_str(&["BENCH_SAVERS"], "matrix,satori,toasters2");
    let names: Vec<&str> = names.split(',').filter(|n| !n.is_empty()).collect();
    let rounds = crate::env_num(&["BENCH_ROUNDS"], 20, 1, 10_000) as usize;

    let panel = Panel::new(1920, 1080, 1920);
    let mut savers: Vec<_> = names.iter().map(|n| saver::make(n, &panel, 30)).collect();
    let mut bufs: Vec<Vec<u32>> = names.iter().map(|_| vec![0u32; panel.buf_len()]).collect();
    let mut total = vec![Duration::ZERO; names.len()];

    // Frame 0 is a full repaint for every saver and is not what runs for weeks.
    for (s, b) in savers.iter_mut().zip(bufs.iter_mut()) {
        saver::frame(s.as_mut(), b, &panel);
    }

    for _ in 0..rounds {
        for i in 0..names.len() {
            let t0 = Instant::now();
            for _ in 0..CHUNK {
                saver::frame(savers[i].as_mut(), &mut bufs[i], &panel);
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
    for (name, t) in names.iter().zip(total.iter()) {
        let us = t.as_secs_f64() / frames * 1e6;
        println!(
            "  {name:<12} {us:8.3} us/frame   {:6.3}x {}",
            us / base,
            names[0]
        );
    }
}
