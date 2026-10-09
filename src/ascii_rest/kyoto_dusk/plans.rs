//! The far bank's buildings and the stone lantern, as plans on upstream's
//! grid: a shade code for a cell's offset from the axis and its row.

use super::{LB, SHORE};
use crate::ascii_rest::math::{fbm, hash};

/// Where each roof's eave sits.
const EAVE: [f64; 5] = [68.5, 58.0, 48.5, 40.0, 32.5];
/// Each roof's half-width.
const ROOF: [f64; 5] = [14.5, 13.4, 12.3, 11.2, 10.1];
const BODY: [f64; 5] = [6.4, 5.9, 5.4, 4.9, 4.4];

/// The pagoda, in cells about its axis: a shade code or 0 for air. 1 body,
/// 2 roof top, 3 roof underside, 4 roof rim, 5 spire, 6 lit doorway.
pub fn pagoda(dx: f64, y: f64) -> u8 {
    let ax = dx.abs();
    // stone base
    if (76.0..SHORE + 0.5).contains(&y) {
        return u8::from(ax <= 9.5 - if y < 77.0 { 1.0 } else { 0.0 });
    }
    for i in 0..5 {
        let (e, r) = (EAVE[i], ROOF[i]);
        // a roof: thin at its upturned tips, rising in a shallow concave curve to the body
        let u = ax / r;
        if u <= 1.0 {
            let lift = 3.5 * u * u * u * u;
            let bottom = e - lift + 0.6;
            let top = e - lift - 1.2 - 3.6 * (1.0 - u).powf(1.7);
            if y >= top && y < bottom {
                if y < top + 0.9 {
                    return 4;
                }
                if y > bottom - 1.0 {
                    return 3;
                }
                return 2;
            }
        }
        // the storey beneath this roof, up from the roof below it (or the base)
        let floor = if i == 0 { 76.0 } else { EAVE[i - 1] - 3.6 };
        if y >= e + 0.6 && y < floor && ax <= BODY[i] {
            if i == 0 && ax <= 1.0 && y > 72.0 && y < 75.5 {
                return 6;
            }
            // a railed balcony under each roof
            if y < e + 2.0 && ax <= BODY[i] + 1.2 {
                return 3;
            }
            return 1;
        }
        if i > 0 && y >= e + 0.6 && y < e + 2.0 && ax <= BODY[i] + 1.2 {
            return 3;
        }
    }
    // the spire: a mast with nine rings and a flame-shaped finial
    let top = EAVE[4] - 1.2 - 3.6 - 0.2;
    if y < top && y >= 9.0 {
        if y >= top - 1.5 {
            return if ax <= 2.4 { 3 } else { 0 }; // the roof box
        }
        if y >= 15.0 && y < top - 1.5 {
            let ring = ((y - 15.0) / 1.2).floor() as i32 & 1;
            return if ax <= if ring != 0 { 1.4 } else { 0.55 } {
                5
            } else {
                0
            };
        }
        if y >= 12.0 {
            return if ax <= 1.3 - (y - 12.0) * 0.2 { 5 } else { 0 };
        }
        return if ax <= 0.6 { 5 } else { 0 };
    }
    0
}

/// The main hall beside the pagoda, in the pagoda's shade codes: a low body of
/// lit shoji under one deep hipped roof.
pub fn hall(dx: f64, y: f64) -> u8 {
    let ax = dx.abs();
    if (77.5..SHORE + 0.5).contains(&y) {
        return u8::from(ax <= 13.0 - if y < 78.5 { 1.0 } else { 0.0 });
    }
    let (e, r) = (70.5, 16.5);
    let u = ax / r;
    if u <= 1.0 {
        let lift = 2.6 * u * u * u * u;
        let bottom = e - lift + 0.6;
        let top = e - lift - 1.2 - 5.4 * (1.0 - u).powf(1.3);
        if y >= top.max(e - 6.2) && y < bottom {
            if y < top.max(e - 6.2) + 0.9 {
                return 4;
            }
            if y > bottom - 1.0 {
                return 3;
            }
            return 2;
        }
    }
    if y >= e + 0.6 && y < 77.5 && ax <= 10.5 {
        if y < e + 1.8 {
            return 3;
        }
        // shoji between the posts, lit from inside
        let bay = ((dx + 10.5) / 3.0).floor();
        if ax <= 9.5 && y > 72.6 && y < 76.4 && (dx + 10.5) % 3.0 > 0.9 && hash(bay, 23.0) > 0.35 {
            return 6;
        }
        return 1;
    }
    0
}

/// The stone lantern in its plan: 1 stone, 2 lit opening, 3 roof, 0 air.
pub fn lantern(dx: f64, y: f64) -> u8 {
    let ax = dx.abs();
    if (93.5..LB).contains(&y) {
        return u8::from(ax <= 4.2 - if y < 94.5 { 0.8 } else { 0.0 }); // foot
    }
    if (87.0..93.5).contains(&y) {
        return u8::from(ax <= 1.4); // post
    }
    if (85.5..87.0).contains(&y) {
        return u8::from(ax <= 3.6 - if y < 86.2 { 0.6 } else { 0.0 }); // platform
    }
    if (80.0..85.5).contains(&y) {
        if ax <= 1.7 && (80.8..84.8).contains(&y) {
            return 2; // lit opening
        }
        return u8::from(ax <= 2.9);
    }
    if (76.5..80.0).contains(&y) {
        // the roof, flaring out with upturned corners
        let u = (80.0 - y) / 3.5;
        let hw = 5.6 - 4.1 * u.powf(0.8) + if y > 79.2 { 0.6 } else { 0.0 };
        return if ax <= hw { 3 } else { 0 };
    }
    if (73.5..76.5).contains(&y) {
        // finial
        return if ax <= 1.3 - (y - 75.0).abs() * 0.25 || ax <= 0.5 {
            3
        } else {
            0
        };
    }
    0
}

pub fn hill_b(x: f64) -> f64 {
    77.0 - 2.5 * fbm(x * 0.04 + 9.0, 2.0, 4, 0.0)
}

/// Low tiled roofs along the far bank, a few lit.
pub fn town(x: f64) -> f64 {
    let i = ((x + 3.0) / 11.0).floor();
    let f = (x + 3.0) / 11.0 - i;
    // a hipped roof: a short level ridge, sloping ends, a gap between houses
    let h = 2.0 + hash(i, 5.0) * 2.5;
    let e = (f - 0.5).abs() * 2.0;
    if e > 0.86 {
        SHORE
    } else {
        SHORE - 1.0 - h + (e - 0.35).max(0.0) * 6.0
    }
}
