//! The cities a new game may start from instead of empty land, and names
//! for the ones that start empty.

use crate::next_rand;

/// Sample cities from the Micropolis release that keep going when left to
/// run, by the names it gave them.
pub const CITIES: &[(&str, &[u8])] = &[
    (
        "Haight",
        include_bytes!("../../micropolis/cities/haight.cty"),
    ),
    (
        "Joffburg",
        include_bytes!("../../micropolis/cities/joffburg.cty"),
    ),
    (
        "Kamakura",
        include_bytes!("../../micropolis/cities/kamakura.cty"),
    ),
    ("Kobe", include_bytes!("../../micropolis/cities/kobe.cty")),
    (
        "Kowloon",
        include_bytes!("../../micropolis/cities/kowloon.cty"),
    ),
    ("Kyoto", include_bytes!("../../micropolis/cities/kyoto.cty")),
    (
        "Radial",
        include_bytes!("../../micropolis/cities/radial.cty"),
    ),
    (
        "Yokohama",
        include_bytes!("../../micropolis/cities/yokohama.cty"),
    ),
];

const FIRST: &[&str] = &[
    "Ash", "Bay", "Birch", "Bright", "Cedar", "Clear", "Cliff", "Elm", "Fair", "Fox", "Glen",
    "Green", "High", "Iron", "Lake", "Maple", "Mill", "North", "Oak", "Pine", "Red", "River",
    "Rock", "Silver", "Spring", "Stone", "Sun", "West", "Willow", "Wood",
];
const LAST: &[&str] = &[
    "brook", "burg", "dale", "field", "ford", "haven", "mont", "port", "ridge", "side", "stead",
    "ton", "view", "ville", "water", "wood",
];

/// A city's name in two parts, so a made-up one needs no allocation.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Name(pub &'static str, pub &'static str);

/// A made-up town name.
pub fn name(rng: &mut u32) -> Name {
    let a = FIRST[next_rand(rng) as usize % FIRST.len()];
    let b = LAST[next_rand(rng) as usize % LAST.len()];
    Name(a, b)
}
