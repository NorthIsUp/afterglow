//! The lamps' light, laid onto a frame's light buffers.

use super::layout::Layout;
use super::{FIG, FLAME, WASH, WATER};
use crate::ascii_rest::math::smooth;

/// A lamp at `[fx, fy]`: a white-hot heart inside an orange halo; the
/// priests are not lit.
pub fn add_glow(
    l: &Layout,
    dynl: &mut [Vec<f32>; 3],
    mat: &[u8],
    [fx, fy]: [f64; 2],
    core: f64,
    halo: f64,
    amp: f64,
) {
    let rr = (halo * 3.2).ceil();
    let mut r = (fy - rr).floor().max(0.0);
    while r < (l.h as f64).min(fy + rr) {
        let mut x = (fx - rr).floor().max(0.0);
        while x < (l.w as f64).min(fx + rr) {
            let k = r as usize * l.w + x as usize;
            let dx = x + 0.5 - fx;
            let dy = (r + 0.5 - fy) * 0.85;
            let d = (dx * dx + dy * dy).sqrt();
            let am = if mat[k] == FIG { amp * 0.15 } else { amp };
            let c = (-(d / core).powi(2)).exp() * 1.6 * am;
            let v = (-d / halo).exp() * 0.35 * am;
            dynl[0][k] = (f64::from(dynl[0][k]) + (c + v * FLAME[0])) as f32;
            dynl[1][k] = (f64::from(dynl[1][k]) + (c * 0.86 + v * FLAME[1])) as f32;
            dynl[2][k] = (f64::from(dynl[2][k]) + (c * 0.55 + v * FLAME[2])) as f32;
            x += 1.0;
        }
        r += 1.0;
    }
}

/// The lamps' wide warm wash on the stone, by how much each surface takes it.
pub fn add_wash(
    l: &Layout,
    dynl: &mut [Vec<f32>; 3],
    alb: &[f32],
    fx: f64,
    fy: f64,
    halo: f64,
    amp: f64,
) {
    let rr = (halo * 2.6).ceil();
    let mut r = (fy - rr).floor().max(0.0);
    while r < (l.h as f64).min(fy + rr) {
        let mut x = (fx - rr).floor().max(0.0);
        while x < (l.w as f64).min(fx + rr) {
            let k = r as usize * l.w + x as usize;
            let a = f64::from(alb[k]);
            if a != 0.0 {
                let (dx, dy) = (x + 0.5 - fx, r + 0.5 - fy);
                let v = (-(dx * dx + dy * dy).sqrt() / halo).exp() * amp * a;
                for c in 0..3 {
                    dynl[c][k] = (f64::from(dynl[c][k]) + v * WASH[c]) as f32;
                }
            }
            x += 1.0;
        }
        r += 1.0;
    }
}

/// A lamp's warm streak on the water below `[fx, wl]`.
pub fn add_streak(
    l: &Layout,
    dref: &mut [f32],
    mat: &[u8],
    [fx, wl]: [f64; 2],
    width: f64,
    len: f64,
    amp: f64,
) {
    let mut r = wl.floor().max(0.0);
    while r < l.h as f64 {
        let dy = r + 0.5 - wl;
        let a = (-dy / len).exp() * amp * smooth(-0.5, 1.0, dy);
        if a < 0.01 {
            break;
        }
        let mut x = (fx - 3.0 * width - 1.0).floor().max(0.0);
        while x < (l.w as f64).min(fx + 3.0 * width + 1.0) {
            let k = r as usize * l.w + x as usize;
            if mat[k] == WATER {
                dref[k] =
                    (f64::from(dref[k]) + a * (-((x + 0.5 - fx) / width).powi(2)).exp()) as f32;
            }
            x += 1.0;
        }
        r += 1.0;
    }
}
