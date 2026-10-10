//! Getting about Kanto: every map's walkable squares, ledges, warps and
//! edge connections read from the cartridge, a route across maps to any
//! square (Dijkstra over map entrances, each map searched breadth-first),
//! and the next button toward it on the live map, people counted as walls.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use mizu_core::GameBoy;

use super::super::kanto::{self, Kanto};
use super::super::pilot::{DOWN, LEFT, RIGHT, UP};
use super::Ram;

/// A map and a square on it.
pub type Square = (u8, u8, u8);

/// A warp's destination map that means "the map you came in from".
const LAST_MAP: u8 = 0xFF;
const OVERWORLD: u8 = 0;
/// Frames a held direction gets to start a step (a turn on the spot takes
/// a few) before it counts as a bump.
const STEP: u32 = 24;
/// `wJoyIgnore`: buttons the game is ignoring, below `wFontLoaded` so the
/// same address in every revision.
const JOY_IGNORE: u16 = 0xCD6B;
/// Extra steps worth walking around people rather than waiting on them.
const DETOUR: u32 = 6;
/// Bumps into a side of a square before it counts as a wall.
const WALL: u16 = 2;

/// One map, as the planner sees it.
pub struct Grid {
    pub w: usize,
    pub h: usize,
    walk: Vec<bool>,
    /// The tile the game tests on each square.
    pub tile: Vec<u8>,
    pub grass: u8,
    pub tileset: u8,
    /// Square, destination map (resolved), destination warp index.
    pub warps: Vec<(u8, u8, u8, u8)>,
    /// Direction flag, joined map, the player's shift along the edge.
    links: Vec<(u8, u8, i32)>,
}

impl Grid {
    pub fn walkable(&self, x: usize, y: usize) -> bool {
        x < self.w && y < self.h && self.walk[y * self.w + x]
    }
}

/// The ledge table `HandleLedges` reads: facing, tile stood on, ledge tile.
/// The same bytes in all three revisions; found by its first entry.
fn ledges(rom: &[u8]) -> Vec<(u8, u8, u8)> {
    const FIRST: [u8; 4] = [0x00, 0x2C, 0x37, 0x80];
    let Some(at) = rom.windows(4).position(|w| w == FIRST) else {
        return Vec::new();
    };
    rom[at..]
        .chunks(4)
        .take_while(|c| c[0] != 0xFF && c.len() == 4)
        .map(|c| {
            let dir = match c[0] {
                0x00 => DOWN,
                0x04 => UP,
                0x08 => LEFT,
                _ => RIGHT,
            };
            (dir, c[1], c[2])
        })
        .collect()
}

pub const DIRS: [u8; 4] = [UP, DOWN, LEFT, RIGHT];

pub fn delta(dir: u8) -> (isize, isize) {
    match dir {
        UP => (0, -1),
        DOWN => (0, 1),
        LEFT => (-1, 0),
        _ => (1, 0),
    }
}

pub struct Nav {
    kanto: Kanto,
    ram: Ram,
    grids: HashMap<u8, Grid>,
    ledges: Vec<(u8, u8, u8)>,
    /// The map each indoor map's "last map" exits lead to.
    outside: HashMap<u8, u8>,
    /// Bumps per (map, x, y, direction) that went nowhere.
    walls: HashMap<(u8, u8, u8, u8), u16>,
    /// The direction held, frames since, where it started, and whether the
    /// step has begun.
    walking: Option<(u8, u32, Square, bool)>,
    last_dir: u8,
    /// The live route: the square to reach on this map, and the button
    /// that leaves the map from it (`None` for the target itself).
    leg: Option<(Square, Option<u8>)>,
    /// Scratch for the live search: first step toward each square.
    first: Vec<u8>,
    dist: Vec<u32>,
    queue: Vec<(usize, usize)>,
    /// Squares a person stands on, this map.
    people: Vec<(usize, usize)>,
    pub skip_sprite: Option<u16>,
}

impl Nav {
    pub fn new(kanto: Kanto, ram: Ram) -> Self {
        Self {
            kanto,
            ram,
            grids: HashMap::new(),
            ledges: Vec::new(),
            outside: HashMap::new(),
            walls: HashMap::new(),
            walking: None,
            last_dir: DOWN,
            leg: None,
            first: Vec::new(),
            dist: Vec::new(),
            queue: Vec::new(),
            people: Vec::new(),
            skip_sprite: None,
        }
    }

    /// Learns the ledge table and which outdoor map every building opens
    /// onto, once.
    fn learn(&mut self, rom: &[u8]) {
        if !self.ledges.is_empty() {
            return;
        }
        self.ledges = ledges(rom);
        for m in 0..=0xF7u8 {
            let Some(h) = self.kanto.header(rom, m) else {
                continue;
            };
            for (_, _, _, to) in raw_warps(rom, h.objects) {
                if to != LAST_MAP && to != m {
                    self.outside.entry(to).or_insert(m);
                }
            }
        }
    }

    pub fn grid(&mut self, rom: &[u8], map: u8) -> Option<&Grid> {
        self.learn(rom);
        if !self.grids.contains_key(&map) {
            let g = self.build(rom, map)?;
            self.grids.insert(map, g);
        }
        self.grids.get(&map)
    }

    fn build(&mut self, rom: &[u8], map: u8) -> Option<Grid> {
        let h = self.kanto.header(rom, map)?;
        let mut placed = Vec::new();
        self.kanto.place(rom, map, 0, &mut placed);
        let p = *placed.first()?;
        let (w, hh) = ((p.w * 2) as usize, (p.h * 2) as usize);
        let mut walk = Vec::with_capacity(w * hh);
        let mut tile = Vec::with_capacity(w * hh);
        for y in 0..hh {
            for x in 0..w {
                walk.push(self.kanto.walkable(rom, &p, x as i32, y as i32));
                tile.push(self.kanto.tile(rom, &p, x as i32, y as i32).unwrap_or(0));
            }
        }
        let outside = self.outside.get(&map).copied();
        let warps: Vec<_> = raw_warps(rom, h.objects)
            .map(|(x, y, id, to)| {
                (
                    x,
                    y,
                    if to == LAST_MAP {
                        outside.unwrap_or(to)
                    } else {
                        to
                    },
                    id,
                )
            })
            .collect();
        // Door tiles are not in the walkable list; the game warps the
        // player as they step on.
        for &(x, y, ..) in &warps {
            if let Some(s) = walk.get_mut(usize::from(y) * w + usize::from(x)) {
                *s = true;
            }
        }
        Some(Grid {
            w,
            h: hh,
            walk,
            tile,
            grass: self.kanto.tileset(p.tileset).map_or(0xFF, |t| t.grass),
            tileset: p.tileset,
            warps,
            links: h.links,
        })
    }

    /// Where a warp lands: its destination's warp of that index.
    fn landing(&mut self, rom: &[u8], to: u8, id: u8) -> Option<Square> {
        let g = self.grid(rom, to)?;
        let &(x, y, ..) = g.warps.get(id as usize)?;
        Some((to, x, y))
    }

    /// Squares reachable from `(x, y)` on a grid and the steps to each,
    /// ledges hopped one way, `blocked` squares and learned walls avoided;
    /// `first` gets the first direction toward each.
    #[allow(clippy::too_many_arguments)]
    fn search(
        g: &Grid,
        ledges: &[(u8, u8, u8)],
        walls: &HashMap<(u8, u8, u8, u8), u16>,
        map: u8,
        from: (usize, usize),
        blocked: &[(usize, usize)],
        dist: &mut Vec<u32>,
        first: &mut Vec<u8>,
        queue: &mut Vec<(usize, usize)>,
    ) {
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
            for dir in DIRS {
                if walls
                    .get(&(map, x as u8, y as u8, dir))
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
                if !g.walk[ny * g.w + nx] {
                    let ledge = g.tileset == OVERWORLD
                        && ledges.iter().any(|&(d, on, l)| {
                            d == dir && on == g.tile[here] && l == g.tile[ny * g.w + nx]
                        });
                    let (jx, jy) = (nx.wrapping_add_signed(dx), ny.wrapping_add_signed(dy));
                    if !ledge || !g.walkable(jx, jy) {
                        continue;
                    }
                    to = (jx, jy);
                    cost = 2;
                }
                if blocked.contains(&to) {
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

    /// The exits a square can reach: the warp or edge square, the button
    /// that leaves from it, where it lands, and the steps.
    fn exits(&mut self, rom: &[u8], from: Square, dist: &[u32]) -> Vec<(Square, u8, Square, u32)> {
        let mut out = Vec::new();
        let Some(g) = self.grid(rom, from.0) else {
            return out;
        };
        let (w, h) = (g.w, g.h);
        let warps = g.warps.clone();
        let links = g.links.clone();
        let edge_dir = |x: usize, y: usize| {
            if y + 1 == h {
                DOWN
            } else if y == 0 {
                UP
            } else if x == 0 {
                LEFT
            } else if x + 1 == w {
                RIGHT
            } else {
                // A door or stairs inside the map: walking onto it is enough.
                0
            }
        };
        for (x, y, to, id) in warps {
            let d = dist
                .get(usize::from(y) * w + usize::from(x))
                .copied()
                .unwrap_or(u32::MAX);
            if d == u32::MAX {
                continue;
            }
            if let Some(land) = self.landing(rom, to, id) {
                out.push(((from.0, x, y), edge_dir(x.into(), y.into()), land, d));
            }
        }
        for (flag, to, off) in links {
            let Some(tg) = self.grid(rom, to) else {
                continue;
            };
            let (tw, th) = (tg.w as i32, tg.h as i32);
            let squares: Vec<(usize, usize)> = match flag {
                kanto::NORTH => (0..w).map(|x| (x, 0)).collect(),
                kanto::SOUTH => (0..w).map(|x| (x, h - 1)).collect(),
                kanto::WEST => (0..h).map(|y| (0, y)).collect(),
                _ => (0..h).map(|y| (w - 1, y)).collect(),
            };
            for (x, y) in squares {
                let d = dist[y * w + x];
                if d == u32::MAX {
                    continue;
                }
                let (lx, ly, dir) = match flag {
                    kanto::NORTH => (x as i32 - 2 * off, th - 1, UP),
                    kanto::SOUTH => (x as i32 - 2 * off, 0, DOWN),
                    kanto::WEST => (tw - 1, y as i32 - 2 * off, LEFT),
                    _ => (0, y as i32 - 2 * off, RIGHT),
                };
                if (0..tw).contains(&lx) && (0..th).contains(&ly) {
                    out.push(((from.0, x as u8, y as u8), dir, (to, lx as u8, ly as u8), d));
                }
            }
        }
        out
    }

    /// The cheapest route from `from` to any square `goal` accepts, as the
    /// square to reach on `from`'s map and the button that leaves the map
    /// there (`None`: the goal is on this map). Searched over map
    /// entrances; `limit` caps the entrances expanded.
    pub fn route(
        &mut self,
        rom: &[u8],
        from: Square,
        goal: &dyn Fn(&Grid, Square) -> bool,
    ) -> Option<(Square, Option<u8>)> {
        let mut heap = BinaryHeap::new();
        let mut best: HashMap<Square, u32> = HashMap::new();
        // Per entrance: the first leg out of `from`'s map.
        let mut via: HashMap<Square, (Square, Option<u8>)> = HashMap::new();
        heap.push(Reverse((0u32, from)));
        best.insert(from, 0);
        let (mut dist, mut first, mut queue) = (Vec::new(), Vec::new(), Vec::new());
        let mut expanded = 0;
        while let Some(Reverse((cost, node))) = heap.pop() {
            if best.get(&node).is_some_and(|&c| c < cost) {
                continue;
            }
            expanded += 1;
            if expanded > 4000 {
                return None;
            }
            {
                self.grid(rom, node.0)?;
                let g = &self.grids[&node.0];
                // Only the live square avoids learned walls: elsewhere they
                // are stale.
                let none = HashMap::new();
                let walls = if node == from { &self.walls } else { &none };
                Self::search(
                    g,
                    &self.ledges,
                    walls,
                    node.0,
                    (node.1.into(), node.2.into()),
                    &[],
                    &mut dist,
                    &mut first,
                    &mut queue,
                );
                // The goal on this map: done if it is the cheapest thing
                // left (the heap is ordered, and no exit can make it
                // cheaper than walking to it here).
                let w = g.w;
                let mut near: Option<(u32, Square)> = None;
                for (i, &d) in dist.iter().enumerate() {
                    if d == u32::MAX {
                        continue;
                    }
                    let s = (node.0, (i % w) as u8, (i / w) as u8);
                    if near.is_none_or(|(nd, _)| d < nd) && goal(g, s) {
                        near = Some((d, s));
                    }
                }
                if let Some((_, s)) = near {
                    return Some(if node == from { (s, None) } else { via[&node] });
                }
            }
            for (sq, dir, land, d) in self.exits(rom, node, &dist) {
                let c = cost + d + 2;
                if best.get(&land).is_none_or(|&b| c < b) {
                    best.insert(land, c);
                    let leg = if node == from {
                        (sq, Some(dir))
                    } else {
                        via[&node]
                    };
                    via.insert(land, leg);
                    heap.push(Reverse((c, land)));
                }
            }
        }
        None
    }

    pub fn here(&self, gb: &mut GameBoy) -> Square {
        let r = self.ram;
        (gb.peek(r.cur_map), gb.peek(r.x), gb.peek(r.y))
    }

    /// Mid-step: what to hold this frame, or `None` once the step is over
    /// (and a step that never started counted as a bump). The direction
    /// is held only until the step starts: the game takes the next step's
    /// direction from whatever is held the frame this one ends, before the
    /// new square shows in RAM, so the key is released and the next step
    /// decided from where this one lands.
    pub fn stepping(&mut self, gb: &mut GameBoy) -> Option<u8> {
        let (dir, frames, from, started) = self.walking?;
        let moving = gb.peek(self.ram.walk_counter) != 0;
        if started {
            if moving && frames < STEP * 3 {
                self.walking = Some((dir, frames + 1, from, true));
                return Some(0);
            }
            self.walking = None;
            return None;
        }
        if moving {
            self.walking = Some((dir, frames + 1, from, true));
            return Some(0);
        }
        if frames < STEP {
            self.walking = Some((dir, frames + 1, from, false));
            return Some(dir);
        }
        self.walking = None;
        let ahead = Self::ahead(from, dir);
        let person = self.people.contains(&(ahead.1.into(), ahead.2.into()));
        // A script holding the joypad is not a wall.
        let held = gb.peek(JOY_IGNORE) != 0;
        if self.here(gb) == from && !person && !held {
            *self.walls.entry((from.0, from.1, from.2, dir)).or_insert(0) += 1;
        }
        None
    }

    pub fn stop(&mut self) {
        self.walking = None;
    }

    /// Forget the live route and every learned wall: after a rewind, or a
    /// new goal.
    pub fn reset(&mut self) {
        self.walking = None;
        self.leg = None;
        self.walls.clear();
    }

    pub fn hold(&mut self, gb: &mut GameBoy, dir: u8) -> u8 {
        let here = self.here(gb);
        self.last_dir = dir;
        self.walking = Some((dir, 0, here, false));
        dir
    }

    fn read_people(&mut self, gb: &mut GameBoy, w: usize, h: usize) {
        let r = self.ram;
        self.people.clear();
        for n in 1..16u16 {
            if Some(n) == self.skip_sprite {
                continue;
            }
            let base = r.sprite_data2 + n * 16;
            let (y, x) = (gb.peek(base + 4), gb.peek(base + 5));
            // A picture, and on screen: hidden people keep their slots.
            let shown = gb.peek(r.sprite_data1 + n * 16) != 0
                && gb.peek(r.sprite_data1 + n * 16 + 2) != 0xFF;
            if shown && x >= 4 && y >= 4 {
                let (x, y) = (usize::from(x - 4), usize::from(y - 4));
                if x < w && y < h {
                    self.people.push((x, y));
                }
            }
        }
    }

    /// The next button toward any square `goal` accepts: `Some(0)` when
    /// standing on one, `None` when nothing it accepts can be reached.
    pub fn toward(&mut self, gb: &mut GameBoy, goal: &dyn Fn(&Grid, Square) -> bool) -> Option<u8> {
        let here = self.here(gb);
        let rom = gb.rom();
        if let Some(g) = self.grid(rom, here.0) {
            if goal(g, here) {
                self.leg = None;
                return Some(0);
            }
        }
        // Planned once per map: the live search below walks the leg.
        if self.leg.is_none_or(|(t, _)| t.0 != here.0) {
            self.leg = self.route(gb.rom(), here, goal);
            // Walls learned from a person who has since moved can cut the
            // only way; forget them and look again.
            if self.leg.is_none() && !self.walls.is_empty() {
                self.walls.clear();
                self.leg = self.route(gb.rom(), here, goal);
            }
        }
        if std::env::var("POKEBOT_TRACE_NAV").is_ok() {
            eprintln!(
                "nav here {here:?} leg {:?} walls {:?}",
                self.leg, self.walls
            );
        }
        let (target, leave) = self.leg?;
        if (target.1, target.2) == (here.1, here.2) {
            let Some(leave) = leave else {
                // The goal moved off this square (a person stood on it).
                self.leg = None;
                return None;
            };
            let dir = if leave == 0 { self.last_dir } else { leave };
            // A way out that did not take: try the side least bumped.
            let bumps = |d: u8| {
                self.walls
                    .get(&(here.0, here.1, here.2, d))
                    .copied()
                    .unwrap_or(0)
            };
            let dir = if bumps(dir) > WALL {
                DIRS.into_iter().min_by_key(|&d| bumps(d)).unwrap_or(dir)
            } else {
                dir
            };
            return Some(self.hold(gb, dir));
        }
        let Some(dir) = self.step_toward(gb, here, (target.1, target.2)) else {
            self.leg = None;
            return None;
        };
        Some(self.hold(gb, dir))
    }

    /// The first step toward `to` on the live map, people avoided; failing
    /// that, ignoring them (they move).
    fn step_toward(&mut self, gb: &mut GameBoy, here: Square, to: (u8, u8)) -> Option<u8> {
        let rom = gb.rom();
        self.grid(rom, here.0)?;
        let g = &self.grids[&here.0];
        let (w, h) = (g.w, g.h);
        self.read_people(gb, w, h);
        let g = &self.grids[&here.0];
        let i = usize::from(to.1) * w + usize::from(to.0);
        // Around people, unless that is far longer than waiting for them
        // to move: the long way round can be a one-way loop over a ledge.
        let mut steps = [(u32::MAX, 0u8); 2];
        for (k, people) in [true, false].into_iter().enumerate() {
            let blocked: &[(usize, usize)] = if people { &self.people } else { &[] };
            Self::search(
                g,
                &self.ledges,
                &self.walls,
                here.0,
                (here.1.into(), here.2.into()),
                blocked,
                &mut self.dist,
                &mut self.first,
                &mut self.queue,
            );
            if let (Some(&d), Some(&f)) = (self.dist.get(i), self.first.get(i)) {
                steps[k] = (d, f);
            }
        }
        let [(around, a), (through, t)] = steps;
        if std::env::var("POKEBOT_TRACE_NAV").is_ok() {
            eprintln!("  steps {steps:?} people {:?}", self.people);
        }
        match (around, through) {
            (u32::MAX, u32::MAX) => None,
            (d, t_) if d != u32::MAX && d <= t_.saturating_add(DETOUR) => Some(a),
            _ => Some(t),
        }
    }

    /// The square in `dir` from `s`.
    pub fn ahead(s: Square, dir: u8) -> Square {
        let (dx, dy) = delta(dir);
        (
            s.0,
            s.1.wrapping_add_signed(dx as i8),
            s.2.wrapping_add_signed(dy as i8),
        )
    }
}

/// A map's warps from its object data: square, destination warp index,
/// destination map.
fn raw_warps(rom: &[u8], objects: usize) -> impl Iterator<Item = (u8, u8, u8, u8)> + '_ {
    let n = rom.get(objects + 1).copied().unwrap_or(0).min(32) as usize;
    (0..n).filter_map(move |i| {
        let e = rom.get(objects + 2 + 4 * i..objects + 6 + 4 * i)?;
        Some((e[1], e[0], e[2], e[3]))
    })
}
