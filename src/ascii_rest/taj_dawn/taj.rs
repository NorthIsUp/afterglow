//! The Taj and the mosque as shapes: where marble or sandstone is, and how it
//! faces the light.

use std::f64::consts::PI;

use crate::ascii_rest::math::{clamp, smooth};

/// The onion dome's profile: swelling out past the drum, then drawn to a point.
fn onion(h: f64) -> f64 {
    if h < 0.3 {
        0.8 + 0.2 * ((h / 0.3) * PI * 0.5).sin()
    } else {
        (((h - 0.3) / 0.7) * PI * 0.5).cos().powf(1.45)
    }
}

/// Pointed arch: half-width at height h above the springline, for an arch of
/// half-width w; zero above the apex.
fn arch(w: f64, h: f64) -> f64 {
    if h <= 0.0 {
        return w;
    }
    let c = w * 0.5;
    let r = w + c;
    let q = r * r - h * h;
    if q > 0.0 {
        (q.sqrt() - c).max(0.0)
    } else {
        0.0
    }
}

/// The Taj in metres: X across from the axis, Y up from the garden. Returns
/// (lit, recess) for marble, or None for air. lit runs 0 (shadow, facing away
/// from the sun) to 1 (facing it); recess darkens arches and niches.
pub fn taj(x: f64, y: f64) -> Option<(f64, f64)> {
    let ax = x.abs();
    let left = if x < 0.0 { 1.0 } else { -1.0 }; // +1 on the sun's side
                                                 // finial
    if (72.0..80.5).contains(&y)
        && ax
            <= 0.75
                + if (y - 74.5).abs() < 0.9 { 0.6 } else { 0.0 }
                + if (y - 77.0).abs() < 0.7 { 0.4 } else { 0.0 }
    {
        return Some((0.75 + 0.2 * left, 0.0));
    }
    // the great dome
    if (47.5..72.0).contains(&y) {
        let h = (y - 47.5) / 24.5;
        let hw = 15.0 * onion(h);
        if ax <= hw {
            // a sphere lit from the left and a little above
            let nx = x / (hw + 0.01);
            let nz = (1.0 - nx * nx).max(0.0).sqrt();
            return Some((clamp(0.3 + 0.5 * (-0.8 * nx + 0.4 * nz) + 0.2 * h), 0.0));
        }
    }
    // the four chhatris on the roof, two in view
    let cx = ax - 17.5;
    let sx = if x < 0.0 { -cx } else { cx };
    if cx.abs() <= 4.4 && (39.5..55.5).contains(&y) {
        let nx = sx / 4.4;
        if y < 41.0 {
            return Some((clamp(0.5 - 0.4 * nx * left), 0.0));
        }
        if y < 46.5 {
            if cx.abs() > 3.6 {
                return None;
            }
            let open = cx.abs() < 2.6 && cx.abs() > 0.6 && y < 45.5;
            return Some((
                clamp(0.5 - 0.45 * sx / 4.0 * left),
                if open { 0.75 } else { 0.0 },
            ));
        }
        if y < 47.3 {
            return Some((clamp(0.55 - 0.4 * nx * left), 0.0));
        }
        let h = (y - 47.3) / 6.5;
        if h < 1.0 && cx.abs() <= 4.0 * onion(h) {
            return Some((
                clamp(0.55 - 0.6 * (sx / (4.0 * onion(h) + 0.01)) * left + 0.1 * h),
                0.0,
            ));
        }
        if h >= 1.0 && cx.abs() < 0.6 {
            return Some((0.6, 0.0));
        }
    }
    // the drum under the dome
    if (40.0..47.5).contains(&y) && ax <= 12.2 {
        let nx = x / 12.2;
        let band = if y > 45.8 { 0.15 } else { 0.0 };
        return Some((clamp(0.45 - 0.55 * nx + band), 0.0));
    }
    // slender pinnacles at the corners of the portal and of the building
    if ((ax - 8.8).abs() < 0.75 && (40.0..47.5).contains(&y))
        || ((ax - 28.2).abs() < 0.75 && (38.0..44.5).contains(&y))
    {
        return Some((0.55 + 0.25 * left, 0.0));
    }
    // the main building
    if ax <= 28.5 && (7.0..40.0).contains(&y) {
        // the portal rises a little above the parapet
        if y >= 38.5 && ax > 9.0 && ((ax + 0.5) / 1.5).floor() as i64 & 1 != 0 {
            return None;
        }
        if ax <= 9.0 {
            // central portal: a calligraphy band round a deep pointed arch
            let hw = arch(6.0, y - 25.0);
            if ax <= hw && y < 25.0 + 9.0 {
                let door = ax <= 3.2 && ax <= arch(3.2, y - 15.5);
                return Some((
                    0.35 + 0.15 * left,
                    if door {
                        0.82
                    } else {
                        0.6 + 0.12 * (1.0 - smooth(25.0, 33.0, y))
                    },
                ));
            }
            if ax <= hw + 0.8 && y < 25.0 + 10.2 {
                return Some((0.42, 0.35));
            }
            if ax > 7.6 && ax <= 9.0 && y < 40.0 {
                return Some((0.5 + 0.12 * left, 0.08));
            }
            return Some((0.5 + 0.08 * left, 0.0));
        }
        if ax <= 21.0 {
            // front face either side of the portal: two storeys of arched niches
            let nx = ax - 15.0;
            for (y0, y1) in [(9.0, 21.5), (24.5, 37.0)] {
                if y >= y0 && y < y1 && nx.abs() <= arch(3.6, y - (y1 - 4.5)) {
                    return Some((0.4 + 0.1 * left, 0.5));
                }
                if y >= y0 - 0.6
                    && y < y1 + 0.6
                    && nx.abs() <= arch(4.3, y - (y1 - 4.2))
                    && nx.abs() > 3.6
                {
                    return Some((0.45, 0.22));
                }
            }
            return Some((0.5 + 0.1 * left, 0.0));
        }
        // chamfered corners, turned toward the sun on the left and away on the right
        let lit = 0.5 + 0.48 * left;
        let nx = ax - 24.7;
        for (y0, y1) in [(9.0, 21.5), (24.5, 37.0)] {
            if y >= y0 && y < y1 && nx.abs() <= arch(2.2, y - (y1 - 3.0)) {
                return Some((lit * 0.7, 0.45));
            }
        }
        return Some((lit, 0.0));
    }
    // minarets at the corners of the plinth, tapering, with three galleries
    let mx = ax - 44.0;
    if (7.0..57.0).contains(&y) {
        let s = if x < 0.0 { -mx } else { mx }; // across the shaft, toward the sun negative
        let hw = 2.9 - 0.6 * (y - 7.0) / 40.0;
        for g in [19.5, 32.0, 44.5] {
            if y >= g && y < g + 1.4 && mx.abs() <= hw + 1.1 {
                return Some((
                    clamp(0.5 - 0.42 * s / (hw + 1.1) * left),
                    if y < g + 0.5 { 0.35 } else { 0.0 },
                ));
            }
        }
        if y < 46.0 && mx.abs() <= hw {
            return Some((clamp(0.5 - 0.48 * (s / hw) * left), 0.0));
        }
        if (45.9..49.5).contains(&y) && mx.abs() <= 2.2 {
            return Some((
                clamp(0.5 - 0.4 * s / 2.2 * left),
                if mx.abs() < 1.4 && mx.abs() > 0.3 {
                    0.7
                } else {
                    0.0
                },
            ));
        }
        if y >= 49.5 {
            let h = (y - 49.5) / 4.5;
            if h < 1.0 && mx.abs() <= 2.5 * onion(h) {
                return Some((clamp(0.55 - 0.55 * s / (2.5 * onion(h) + 0.01) * left), 0.0));
            }
            if h >= 1.0 && y < 56.0 && mx.abs() < 0.5 {
                return Some((0.6, 0.0));
            }
        }
    }
    // the plinth, with a row of shallow niches
    if ax <= 47.5 && (0.0..7.0).contains(&y) {
        if y > 6.2 {
            return Some((0.62, 0.0));
        }
        let k = (ax % 5.2) - 2.6;
        if y > 1.5 && y < 5.2 && k.abs() < 1.1 {
            return Some((0.42, 0.3));
        }
        return Some((0.52, 0.0));
    }
    None
}

/// The red sandstone mosque that flanks the Taj, in cells, on axis `mx` with
/// its foot on row `mb`: three domes over a
/// five-bay front with a tall central portal. Returns 0 for wall, 1 for dome,
/// 2 for a recess, or -1 for air. It stands against the sun, so it is mostly
/// silhouette.
pub fn mosque(xc: f64, y: f64, mx: f64, mb: f64) -> i8 {
    let dx = xc - mx;
    let ax = dx.abs();
    let odd = |v: f64| v.floor() as i64 & 1 != 0;
    // plinth
    if (mb - 2.5..mb).contains(&y) && ax <= 22.0 {
        return 0;
    }
    // end towers, each with a small kiosk on top
    let tx = (ax - 19.5).abs();
    if tx <= 1.3 && (mb - 13.0..mb - 2.5).contains(&y) {
        return 0;
    }
    if (mb - 15.5..mb - 13.0).contains(&y) {
        let h = (mb - 13.0 - y) / 2.5;
        if tx <= 1.8 * onion(h) {
            return 1;
        }
    }
    if tx < 0.35 && (mb - 16.5..mb - 15.5).contains(&y) {
        return 1;
    }
    // central portal, rising above the front
    if ax <= 5.5 && (mb - 15.0..mb - 2.5).contains(&y) {
        if y < mb - 14.3 && odd(xc) {
            return -1;
        }
        if ax <= arch(3.4, mb - 9.5 - y) && y >= mb - 13.5 {
            return 2;
        }
        return 0;
    }
    // the five-bay front and its parapet
    if ax <= 18.0 && (mb - 10.0..mb - 2.5).contains(&y) {
        let bay = ((ax - 5.5) % 4.2) - 2.1;
        if ax > 6.0 && y >= mb - 8.5 && bay.abs() <= arch(1.4, mb - 6.2 - y) {
            return 2;
        }
        return 0;
    }
    if ax <= 18.0 && (mb - 10.8..mb - 10.0).contains(&y) && odd(xc * 0.75) {
        return 0;
    }
    // side domes on drums
    let sx = (ax - 11.5).abs();
    if sx <= 2.6 && (mb - 12.0..mb - 10.0).contains(&y) {
        return 0;
    }
    if (mb - 17.0..mb - 12.0).contains(&y) {
        let h = (mb - 12.0 - y) / 5.0;
        if sx <= 3.6 * onion(h) {
            return 1;
        }
    }
    if sx < 0.35 && (mb - 18.5..mb - 17.0).contains(&y) {
        return 1;
    }
    // the great central dome
    if ax <= 4.0 && (mb - 16.5..mb - 15.0).contains(&y) {
        return 0;
    }
    if (mb - 23.5..mb - 16.5).contains(&y) {
        let h = (mb - 16.5 - y) / 7.0;
        if ax <= 5.2 * onion(h) {
            return 1;
        }
    }
    if ax < 0.4 && (mb - 25.5..mb - 23.5).contains(&y) {
        return 1;
    }
    -1
}
