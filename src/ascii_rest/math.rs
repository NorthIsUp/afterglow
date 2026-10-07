//! The helpers ascii.rest pieces each carry a copy of, once. All f64, as
//! upstream's JavaScript is, so a port computes the same picture.
//!
//! Upstream has cosmetic variants (`noise(x, y)` with no period is
//! `noise(x, y, 0.0)` here); a piece whose copy really differs keeps its own.

/// Upstream's `hash(x, y)`: integer lattice -> [0, 1). `x | 0` truncates toward
/// zero and wraps to 32 bits, which `as i64 as i32` does for every finite value
/// a piece passes; the arithmetic after it is u32 wrapping, exactly as
/// `Math.imul`, `^` and `>>>` see it.
#[inline]
pub fn hash(x: f64, y: f64) -> f64 {
    let xi = x as i64 as i32 as u32;
    let yi = y as i64 as i32 as u32;
    let mut h = xi
        .wrapping_mul(374_761_393)
        .wrapping_add(yi.wrapping_mul(668_265_263));
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    f64::from(h ^ (h >> 16)) / 4_294_967_296.0
}

/// Value noise, wrapping every `period` lattice cells in x when period > 0.
#[inline]
pub fn noise(x: f64, y: f64, period: f64) -> f64 {
    let (xi, yi) = (x.floor(), y.floor());
    let (fx, fy) = (x - xi, y - yi);
    let u = fx * fx * (3.0 - 2.0 * fx);
    let v = fy * fy * (3.0 - 2.0 * fy);
    let (mut x0, mut x1) = (xi, xi + 1.0);
    if period != 0.0 {
        x0 = js_rem(js_rem(xi, period) + period, period);
        x1 = js_rem(x0 + 1.0, period);
    }
    let a = hash(x0, yi);
    let b = hash(x1, yi);
    let c = hash(x0, yi + 1.0);
    let d = hash(x1, yi + 1.0);
    a + (b - a) * u + (c - a) * v + (a - b - c + d) * u * v
}

/// Octaves of [`noise`], each twice the frequency and half the weight, the
/// period scaled with the frequency so every octave wraps at the same x.
#[inline]
pub fn fbm(x: f64, y: f64, octaves: u32, period: f64) -> f64 {
    let (mut s, mut n, mut amp, mut f) = (0.0, 0.0, 0.5, 1.0);
    for _ in 0..octaves {
        s += amp * noise(x * f, y * f, period * f);
        n += amp;
        amp *= 0.5;
        f *= 2.0;
    }
    s / n
}

#[inline]
pub fn clamp(v: f64) -> f64 {
    v.clamp(0.0, 1.0)
}

/// Smoothstep from `a` to `b`; `a > b` runs it backwards, as upstream relies on.
#[inline]
pub fn smooth(a: f64, b: f64, v: f64) -> f64 {
    let k = clamp((v - a) / (b - a));
    k * k * (3.0 - 2.0 * k)
}

#[inline]
pub fn mix(a: f64, b: f64, k: f64) -> f64 {
    a + (b - a) * k
}

/// JavaScript's `%`: the sign of the dividend, which is Rust's `%` on f64 too.
/// Named so a port reads as the original.
#[inline]
pub fn js_rem(a: f64, b: f64) -> f64 {
    a % b
}

/// JavaScript's `Math.round`: halves toward +infinity, where Rust's `round`
/// goes away from zero.
#[inline]
pub fn js_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

/// The two-round integer hash fractal-tree and lighthouse use instead of
/// [`hash`].
#[inline]
pub fn hash2(x: i64, y: i64) -> f64 {
    let mut h =
        (x as i32 as u32).wrapping_mul(0x27d4_eb2d) ^ (y as i32 as u32).wrapping_mul(0x1656_67b1);
    h = (h ^ (h >> 15)).wrapping_mul(0x85eb_ca6b);
    f64::from(h ^ (h >> 13)) / 4_294_967_296.0
}

/// Upstream's `Math.sign(v || 1)` and `Math.sign(v) || 1`, which agree: 1 for
/// zero and NaN.
#[inline]
pub fn sign_or_one(v: f64) -> f64 {
    if v == 0.0 || v.is_nan() {
        1.0
    } else {
        v.signum()
    }
}

/// `v` scaled to length 1, by [`js_hypot`].
#[inline]
pub fn unit(v: [f64; 3]) -> [f64; 3] {
    let n = js_hypot(&v);
    v.map(|c| c / n)
}

/// Upstream's seeded `mulberry32`, the generator every piece that wants
/// repeatable randomness copies.
pub struct Mulberry32(pub u32);

impl Mulberry32 {
    #[inline]
    pub fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6d2b_79f5);
        let a = self.0;
        let mut t = (a ^ (a >> 15)).wrapping_mul(1 | a);
        t = t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t)) ^ t;
        f64::from(t ^ (t >> 14)) / 4_294_967_296.0
    }
}

/// `Math.hypot` as JavaScriptCore computes it: scaled to the largest argument
/// and Kahan-summed. libm's `hypot` differs in the last bit, which is enough to
/// move a cell across a dither boundary.
#[inline]
pub fn js_hypot(v: &[f64]) -> f64 {
    let max = v.iter().fold(0.0f64, |m, c| m.max(c.abs()));
    if max == 0.0 {
        return 0.0;
    }
    let (mut sum, mut comp) = (0.0f64, 0.0f64);
    for c in v {
        let n = c / max;
        let summand = n * n - comp;
        let pre = sum + summand;
        comp = (pre - sum) - summand;
        sum = pre;
    }
    sum.sqrt() * max
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Values taken from upstream's JavaScript under node, so the port is
    /// pinned to the original rather than to itself.
    #[test]
    fn hash_matches_upstream() {
        assert_eq!(hash(0.0, 0.0), 0.0);
        assert_eq!(hash(3.0, 7.0), JS_HASH_3_7);
        assert_eq!(hash(-5.7, 12.2), JS_HASH_M5_12);
    }

    const JS_HASH_3_7: f64 = 0.060_723_951_552_063_23;
    const JS_HASH_M5_12: f64 = 0.032_864_839_071_407_914;

    #[test]
    fn js_round_rounds_halves_up() {
        assert_eq!(js_round(-0.5), 0.0);
        assert_eq!(js_round(0.5), 1.0);
        assert_eq!(js_round(2.4), 2.0);
    }
}
