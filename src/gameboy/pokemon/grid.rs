//! One map as the planner sees it, from the cartridge: walkable squares,
//! warps, edge links, and the movement rules beyond them (ledges, a cave's
//! raised floor, spinner arrows, trees once Cut is known), searched
//! breadth-first.

use std::collections::HashMap;

use super::nav::{delta, facing, Square, DIRS};

/// One side of a square: where a step in that direction would leave it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Side(pub Square, pub u8);
const OVERWORLD: u8 = 0;
/// Bumps into a side of a square before it counts as a wall.
pub const WALL: u16 = 2;
/// Each spinner floor's arrow table (`DecodeArrowMovementRLE` reads it),
/// found by its first two squares as the table stores them, y first:
/// Rocket Hideout B2F and B3F, and Viridian Gym.
const SPINNERS: [(u8, [u8; 2], [u8; 2]); 3] = [
    (0xC8, [9, 4], [11, 4]),
    (0xC9, [13, 10], [19, 10]),
    (0x2D, [11, 19], [1, 19]),
];
/// The tiles `UsedCut` cuts: a tree outdoors, and a gym's.
const TREE: u8 = 0x3D;
const GYM: u8 = 7;
const GYM_TREE: u8 = 0x50;

/// One map, as the planner sees it.
pub struct Grid {
    pub w: usize,
    pub h: usize,
    pub walk: Vec<bool>,
    /// The tile the game tests on each square.
    pub tile: Vec<u8>,
    pub grass: u8,
    pub tileset: u8,
    /// Square, destination map (resolved), destination warp index.
    pub warps: Vec<(u8, u8, u8, u8)>,
    /// Direction flag, joined map, the player's shift along the edge.
    pub links: Vec<(u8, u8, i32)>,
}

impl Grid {
    pub fn walkable(&self, x: usize, y: usize) -> bool {
        x < self.w && y < self.h && self.walk[y * self.w + x]
    }

    /// A warp off the map's edge: stepping on it is enough.
    fn inner_warp(&self, x: usize, y: usize) -> bool {
        (1..self.w - 1).contains(&x)
            && (1..self.h - 1).contains(&y)
            && self
                .warps
                .iter()
                .any(|w| (usize::from(w.0), usize::from(w.1)) == (x, y))
    }
}

/// Movement rules beyond the walkable tiles, from tables the game reads:
/// ledges (`HandleLedges`: facing, tile stood on, ledge tile) and tile
/// pairs no step may cross (`TilePairCollisionsLand`: a cave's raised
/// floor and its edge). The same bytes in all three revisions, found by
/// their first entries.
#[derive(Default)]
pub struct Rules {
    ledges: Vec<(u8, u8, u8)>,
    pairs: Vec<(u8, u8, u8)>,
    /// Arrow squares and the square each throws the player to.
    spins: HashMap<Square, (u8, u8)>,
    /// The party can cut trees, so they are a way through.
    pub cut: bool,
}

impl Rules {
    pub fn of(rom: &[u8]) -> Self {
        let table = |first: &[u8], width: usize| -> Vec<&[u8]> {
            let Some(at) = rom.windows(first.len()).position(|w| w == first) else {
                return Vec::new();
            };
            rom[at..]
                .chunks(width)
                .take_while(|c| c[0] != 0xFF && c.len() == width)
                .collect()
        };
        let ledges = table(&[0x00, 0x2C, 0x37, 0x80], 4)
            .into_iter()
            .map(|c| (facing(c[0]), c[1], c[2]))
            .collect();
        let pairs = table(&[0x11, 0x20, 0x05, 0x11, 0x41, 0x05], 3)
            .into_iter()
            .map(|c| (c[0], c[1], c[2]))
            .collect();
        Self {
            ledges,
            pairs,
            spins: spins(rom),
            cut: false,
        }
    }

    /// Where stepping onto `to` leaves the player, and the extra steps:
    /// an arrow throws them on, maybe onto another.
    fn spin(&self, map: u8, mut to: (usize, usize)) -> ((usize, usize), u32) {
        let mut cost = 0;
        for _ in 0..8 {
            let Some(&(x, y)) = self.spins.get(&(map, to.0 as u8, to.1 as u8)) else {
                break;
            };
            let (x, y) = (usize::from(x), usize::from(y));
            cost += (to.0.abs_diff(x) + to.1.abs_diff(y)) as u32;
            to = (x, y);
        }
        (to, cost)
    }

    fn ledge(&self, g: &Grid, dir: u8, on: u8, next: u8) -> bool {
        g.tileset == OVERWORLD
            && self
                .ledges
                .iter()
                .any(|&(d, o, l)| d == dir && o == on && l == next)
    }

    fn tree(&self, g: &Grid, tile: u8) -> bool {
        self.cut && is_tree(g, tile)
    }

    fn crossing(&self, g: &Grid, a: u8, b: u8) -> bool {
        self.pairs
            .iter()
            .any(|&(ts, x, y)| ts == g.tileset && ((x, y) == (a, b) || (x, y) == (b, a)))
    }
}

/// One breadth-first search's result, kept to reuse its buffers: the steps
/// to each square of a grid and the first direction toward it.
#[derive(Default)]
pub struct Bfs {
    pub dist: Vec<u32>,
    pub first: Vec<u8>,
    queue: Vec<(usize, usize)>,
}

impl Bfs {
    /// Squares reachable from `(x, y)` on a grid and the steps to each,
    /// ledges hopped one way, `blocked` squares and learned walls avoided;
    /// `first` holds the first direction toward each.
    pub fn run(
        &mut self,
        g: &Grid,
        rules: &Rules,
        walls: &HashMap<Side, u16>,
        map: u8,
        from: (usize, usize),
        blocked: &[(usize, usize)],
    ) {
        let Self { dist, first, queue } = self;
        dist.clear();
        dist.resize(g.w * g.h, u32::MAX);
        first.clear();
        first.resize(g.w * g.h, 0);
        queue.clear();
        if from.0 >= g.w || from.1 >= g.h {
            return;
        }
        dist[from.1 * g.w + from.0] = 0;
        queue.push(from);
        let mut head = 0;
        while head < queue.len() {
            let (x, y) = queue[head];
            head += 1;
            let here = y * g.w + x;
            // Stairs and pads inside a map take the player the moment they
            // step on: a way out, never a way across.
            if (x, y) != from && g.inner_warp(x, y) {
                continue;
            }
            for dir in DIRS {
                if walls
                    .get(&Side((map, x as u8, y as u8), dir))
                    .copied()
                    .unwrap_or(0)
                    > WALL
                {
                    continue;
                }
                let (dx, dy) = delta(dir);
                let (nx, ny) = (x.wrapping_add_signed(dx), y.wrapping_add_signed(dy));
                if nx >= g.w || ny >= g.h {
                    continue;
                }
                let mut to = (nx, ny);
                let mut cost = 1;
                let next = g.tile[ny * g.w + nx];
                if rules.crossing(g, g.tile[here], next) {
                    continue;
                }
                if !g.walk[ny * g.w + nx] && !rules.tree(g, next) {
                    let ledge = rules.ledge(g, dir, g.tile[here], next);
                    let (jx, jy) = (nx.wrapping_add_signed(dx), ny.wrapping_add_signed(dy));
                    if !ledge || !g.walkable(jx, jy) {
                        continue;
                    }
                    to = (jx, jy);
                    cost = 2;
                }
                let (to, spun) = rules.spin(map, to);
                cost += spun;
                if blocked.contains(&to) || spun > 0 && !g.walkable(to.0, to.1) {
                    continue;
                }
                let i = to.1 * g.w + to.0;
                if dist[i] != u32::MAX {
                    continue;
                }
                dist[i] = dist[here] + cost;
                first[i] = if (x, y) == from { dir } else { first[here] };
                queue.push(to);
            }
        }
    }
}

pub fn is_tree(g: &Grid, tile: u8) -> bool {
    (g.tileset == OVERWORLD && tile == TREE) || (g.tileset == GYM && tile == GYM_TREE)
}

/// `SPINNERS`' tables: each arrow square (4 bytes: y, x, a pointer into the
/// same bank to `(direction, count)` pairs ending in `0xFF`) and the square
/// its moves end on.
fn spins(rom: &[u8]) -> HashMap<Square, (u8, u8)> {
    let mut out = HashMap::new();
    for (map, first, second) in SPINNERS {
        // Into the switchable bank, both: the squares alone match elsewhere.
        let banked = |hi: u8| (0x40..0x80).contains(&hi);
        let Some(at) = rom
            .windows(8)
            .position(|w| w[..2] == first && w[4..6] == second && banked(w[3]) && banked(w[7]))
        else {
            continue;
        };
        let bank = at / 0x4000 * 0x4000;
        for e in rom[at..]
            .as_chunks::<4>()
            .0
            .iter()
            .take_while(|e| e[0] != 0xFF)
        {
            let ptr = usize::from(u16::from_le_bytes([e[2], e[3]]));
            if !(0x4000..0x8000).contains(&ptr) {
                break;
            }
            let (mut x, mut y) = (i32::from(e[1]), i32::from(e[0]));
            for m in rom[bank + ptr - 0x4000..]
                .as_chunks::<2>()
                .0
                .iter()
                .take_while(|m| m[0] != 0xFF)
            {
                let (dx, dy) = delta(m[0] >> 4);
                x += dx as i32 * i32::from(m[1]);
                y += dy as i32 * i32::from(m[1]);
            }
            if let (Ok(x), Ok(y)) = (u8::try_from(x), u8::try_from(y)) {
                out.insert((map, e[1], e[0]), (x, y));
            }
        }
    }
    out
}
