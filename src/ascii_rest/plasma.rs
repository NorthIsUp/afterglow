//! plasma: the old demo-scene effect. Four sine fields, one a set of rings
//! round a wandering centre, summed and read as soft blobs of density.

use std::f64::consts::PI;

use super::{hex, text, Piece};
use crate::grid::Cell;

const RAMP: [Cell; 12] = text::cells(['.', ',', '-', '~', ':', ';', '=', '+', '*', '#', '%', '@']);
/// Seconds; every term turns a whole number of times in one loop.
const P: f64 = 30.0;
const W: f64 = 2.0 * PI / P;
/// Ramp sweeps per unit of the summed field: low, so the blobs are broad.
const GAIN: f64 = 0.32;

pub struct Plasma {
}

impl Piece for Plasma {
    const NAME: &'static str = "plasma";
    const COLS: usize = 64;
    const ROWS: usize = 22;
    const FPS: u32 = 24;
    const CELL: usize = 2;
    const PALETTE: &'static [u32] = &[hex("#c86bff")];
    const GROUND: u32 = 0;

    fn new() -> Self {
        Self {
        }
    }

    fn frame(&mut self, t: f64, out: &mut [Cell]) {
        let n = RAMP.len();
        let a = W * t;
        let (ca, sa) = (a.cos(), a.sin());
        let (cx, cy) = (14.0 * a.sin(), 9.0 * (2.0 * a).cos());
        for r in 0..Self::ROWS {
            // Cell centres in cell widths, the rows stretched to their true height.
            let y = (r as f64 + 0.5 - Self::ROWS as f64 / 2.0) * 2.0;
            for c in 0..Self::COLS {
                let x = c as f64 + 0.5 - Self::COLS as f64 / 2.0;
                let mut v = (x * 0.11 + 3.0 * a).sin();
                v += (y * 0.13 - 2.0 * a).sin();
                v += ((x * ca + y * sa) * 0.09 + 4.0 * a).sin();
                v += ((x - cx).hypot(y - cy) * 0.17 - 5.0 * a).sin();
                // Up the ramp and back down, evenly, drifting through it once a loop.
                let u = v * GAIN + a / PI;
                let k = (u - 2.0 * (u / 2.0 + 0.5).floor()).abs();
                let i = ((k * n as f64).floor() as usize).min(n - 1);
                out[r * Self::COLS + c] = RAMP[i];
            }
        }
    }
}
