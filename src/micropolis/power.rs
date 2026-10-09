//! Power-line routing: from the grid a plant feeds to zones it does not
//! reach, the cheapest run of wire.
//!
//! The engine conducts power between any two neighbouring tiles that both
//! carry `CONDBIT`, so the grid is the 4-connected pieces of such tiles, and
//! a piece with a plant in it is live. A road takes a power line across it,
//! which conducts like any other, so a run may cross or follow roads.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use super::mayor::{H, W};
use super::tiles::{is_clear, is_hazard, is_water, CONDBIT, LOMASK, ZONEBIT};

const COAL: std::ops::RangeInclusive<u16> = 745..=760;
const NUCLEAR: std::ops::RangeInclusive<u16> = 811..=826;
/// Straight road tiles, the only ones a line can cross.
const ROAD_STRAIGHT: std::ops::RangeInclusive<u16> = 66..=67;
const NONE: u32 = u32::MAX;

/// The tiles to lay wire on so that some unpowered zone joins a live grid,
/// nearest first; `None` if every zone is live, or no plant stands, or no
/// run reaches. `extra(i)` is what laying wire on tile `i` costs beyond the
/// tile itself, `None` where wire must not go: a wanted building's site.
pub fn route(map: &[u16], extra: impl Fn(usize) -> Option<u32>) -> Option<Vec<usize>> {
    let n = (W * H) as usize;
    let mut comp = vec![NONE; n];
    let mut live = Vec::new();
    let mut dead_zone = Vec::new();
    let mut stack = Vec::new();
    for start in 0..n {
        if map[start] & CONDBIT == 0 || comp[start] != NONE {
            continue;
        }
        let id = live.len() as u32;
        let (mut plant, mut zone) = (false, false);
        comp[start] = id;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let t = map[i] & LOMASK;
            plant |= COAL.contains(&t) || NUCLEAR.contains(&t);
            zone |= map[i] & ZONEBIT != 0;
            for j in neighbours(i) {
                if map[j] & CONDBIT != 0 && comp[j] == NONE {
                    comp[j] = id;
                    stack.push(j);
                }
            }
        }
        live.push(plant);
        dead_zone.push(zone && !plant);
    }
    if !live.contains(&true) || !dead_zone.contains(&true) {
        return None;
    }
    let mut dist = vec![u32::MAX; n];
    let mut from = vec![usize::MAX; n];
    let mut heap = BinaryHeap::new();
    for i in 0..n {
        if comp[i] != NONE && live[comp[i] as usize] {
            dist[i] = 0;
            heap.push(Reverse((0, i)));
        }
    }
    while let Some(Reverse((d, i))) = heap.pop() {
        if d > dist[i] {
            continue;
        }
        if comp[i] != NONE && dead_zone[comp[i] as usize] {
            let mut path = Vec::new();
            let mut k = from[i];
            while k != usize::MAX {
                if map[k] & CONDBIT == 0 {
                    path.push(k);
                }
                k = from[k];
            }
            return (!path.is_empty()).then_some(path);
        }
        for j in neighbours(i) {
            let cost = if comp[j] == NONE {
                let t = map[j] & LOMASK;
                let base = if ROAD_STRAIGHT.contains(&t) {
                    2
                } else if is_clear(t) && !is_water(t) && !is_hazard(t) {
                    3
                } else {
                    continue;
                };
                let Some(e) = extra(j) else { continue };
                base + e
            } else if live[comp[j] as usize] {
                continue;
            } else {
                u32::from(!dead_zone[comp[j] as usize])
            };
            let nd = d + cost;
            if nd < dist[j] {
                dist[j] = nd;
                from[j] = i;
                heap.push(Reverse((nd, j)));
            }
        }
    }
    None
}

/// The 4-neighbours of map index `i` (column-major, `x * H + y`).
fn neighbours(i: usize) -> impl Iterator<Item = usize> {
    let (x, y) = ((i / H as usize) as i32, (i % H as usize) as i32);
    [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)]
        .into_iter()
        .filter(|&(x, y)| (0..W).contains(&x) && (0..H).contains(&y))
        .map(|(x, y)| (x * H + y) as usize)
}
