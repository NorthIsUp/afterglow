//! Getting about Kanto: every map's walkable squares, ledges, warps and
//! edge connections read from the cartridge, a route across maps to any
//! square (Dijkstra over map entrances, each map searched breadth-first),
//! and the next button toward it on the live map, people counted as walls.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use mizu_core::GameBoy;

use super::super::carts::Revision;
use super::super::kanto::{self, Kanto};
use super::super::pilot::{DOWN, LEFT, RIGHT, UP};
use super::Ram;

/// A map and a square on it.
pub type Square = (u8, u8, u8);

/// A step under way: the direction, frames since it was pressed, where
/// it started, whether it has begun, and whether a script held the
/// joypad meanwhile.
#[derive(Clone, Copy)]
struct Walk {
    dir: u8,
    frames: u32,
    from: Square,
    started: bool,
    held: bool,
}

/// One side of a square: where a step in that direction would leave it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Side(Square, u8);

/// A warp's destination map that means "the map you came in from".
const LAST_MAP: u8 = 0xFF;
const OVERWORLD: u8 = 0;
/// Frames a held direction gets to start a step (a turn on the spot takes
/// a few) before it counts as a bump.
const STEP: u32 = 24;
/// `wJoyIgnore`: buttons the game is ignoring, below `wFontLoaded` so the
/// same address in every revision.
pub const JOY_IGNORE: u16 = 0xCD6B;
/// `wSpritePlayerStateData1FacingDirection`.
pub const FACING: u16 = 0xC109;
/// Extra steps worth walking around people rather than waiting on them.
const DETOUR: u32 = 6;
/// Bumps into a side of a square before it counts as a wall.
const WALL: u16 = 2;
/// Map entrances a route search expands before it gives up.
const ROUTE_LIMIT: u32 = 4000;
/// Saffron's gatehouses: a thirsty guard turns the player back, and the
/// bot never brings him a drink.
const CLOSED: [u8; 4] = [0x46, 0x49, 0x4C, 0x4F];
/// The Snorlax asleep on Routes 12 and 16: no Poké Flute, no way past.
const SNORLAX: [Square; 2] = [(0x17, 10, 62), (0x1B, 26, 10)];
/// The tiles `UsedCut` cuts: a tree outdoors, and a gym's.
const TREE: u8 = 0x3D;
const GYM: u8 = 7;
const GYM_TREE: u8 = 0x50;

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

/// Movement rules beyond the walkable tiles, from tables the game reads:
/// ledges (`HandleLedges`: facing, tile stood on, ledge tile) and tile
/// pairs no step may cross (`TilePairCollisionsLand`: a cave's raised
/// floor and its edge). The same bytes in all three revisions, found by
/// their first entries.
#[derive(Default)]
struct Rules {
    ledges: Vec<(u8, u8, u8)>,
    pairs: Vec<(u8, u8, u8)>,
    /// The party can cut trees, so they are a way through.
    cut: bool,
}

impl Rules {
    fn of(rom: &[u8]) -> Self {
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
            cut: false,
        }
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

pub const DIRS: [u8; 4] = [UP, DOWN, LEFT, RIGHT];

/// A direction as the game stores a facing (`SPRITE_FACING_*`).
pub fn facing(b: u8) -> u8 {
    match b {
        0x04 => UP,
        0x08 => LEFT,
        0x0C => RIGHT,
        _ => DOWN,
    }
}

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
    rules: Rules,
    learned: bool,
    /// Squares taken out of their grids by `trap`.
    traps: Vec<Square>,
    /// The map each indoor map's "last map" exits lead to.
    outside: HashMap<u8, u8>,
    /// Bumps per (map, x, y, direction) that went nowhere.
    walls: HashMap<Side, u16>,
    walking: Option<Walk>,
    last_dir: u8,
    /// The live route: the square to reach on this map, and the button
    /// that leaves the map from it (`None` for the target itself).
    leg: Option<(Square, Option<u8>)>,
    /// Scratch for the live search.
    bfs: Bfs,
    /// Squares a person stands on, this map.
    people: Vec<(usize, usize)>,
    /// Yellow's Pikachu walks behind the player in this sprite slot.
    skip_sprite: Option<u16>,
    /// Trees cut on the map the player is on: they grow back when it
    /// leaves.
    cut: Vec<Square>,
    /// A tree in the way, faced, and already counted as cut: the bot
    /// should cut it.
    pub tree: Option<Square>,
}

/// One breadth-first search's result, kept to reuse its buffers: the steps
/// to each square of a grid and the first direction toward it.
#[derive(Default)]
struct Bfs {
    dist: Vec<u32>,
    first: Vec<u8>,
    queue: Vec<(usize, usize)>,
}

impl Bfs {
    /// Squares reachable from `(x, y)` on a grid and the steps to each,
    /// ledges hopped one way, `blocked` squares and learned walls avoided;
    /// `first` holds the first direction toward each.
    fn run(
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
}

impl Nav {
    pub fn new(rev: Revision) -> Self {
        Self {
            kanto: Kanto::new(rev),
            ram: Ram::of(rev),
            grids: HashMap::new(),
            rules: Rules::default(),
            learned: false,
            traps: Vec::new(),
            outside: HashMap::new(),
            walls: HashMap::new(),
            walking: None,
            last_dir: DOWN,
            leg: None,
            bfs: Bfs::default(),
            people: Vec::new(),
            skip_sprite: (rev == Revision::Yellow).then_some(15),
            cut: Vec::new(),
            tree: None,
        }
    }

    /// Learns the ledge table and which outdoor map every building opens
    /// onto, once, found or not.
    fn learn(&mut self, rom: &[u8]) {
        if self.learned {
            return;
        }
        self.learned = true;
        self.rules = Rules {
            cut: self.rules.cut,
            ..Rules::of(rom)
        };
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
        for &(m, x, y) in &SNORLAX {
            if m == map {
                walk[usize::from(y) * w + usize::from(x)] = false;
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
            if CLOSED.contains(&to) {
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
            let along = if flag == kanto::NORTH || flag == kanto::SOUTH {
                w
            } else {
                h
            };
            for k in 0..along {
                let (x, y) = match flag {
                    kanto::NORTH => (k, 0),
                    kanto::SOUTH => (k, h - 1),
                    kanto::WEST => (0, k),
                    _ => (w - 1, k),
                };
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
                // The game tests the square stepped onto in the joined map.
                if (0..tw).contains(&lx)
                    && (0..th).contains(&ly)
                    && tg.walkable(lx as usize, ly as usize)
                {
                    out.push(((from.0, x as u8, y as u8), dir, (to, lx as u8, ly as u8), d));
                }
            }
        }
        out
    }

    /// The cheapest route from `from` to any square `goal` accepts, as the
    /// square to reach on `from`'s map and the button that leaves the map
    /// there (`None`: the goal is on this map). Searched over map
    /// entrances, at most `ROUTE_LIMIT` of them.
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
        let mut bfs = Bfs::default();
        let mut expanded = 0;
        while let Some(Reverse((cost, node))) = heap.pop() {
            if best.get(&node).is_some_and(|&c| c < cost) {
                continue;
            }
            expanded += 1;
            if expanded > ROUTE_LIMIT {
                return None;
            }
            {
                self.grid(rom, node.0)?;
                let g = &self.grids[&node.0];
                // Only the live square avoids learned walls: elsewhere they
                // are stale.
                let none = HashMap::new();
                let walls = if node == from { &self.walls } else { &none };
                bfs.run(
                    g,
                    &self.rules,
                    walls,
                    node.0,
                    (node.1.into(), node.2.into()),
                    &[],
                );
                // The goal on this map: done if it is the cheapest thing
                // left (the heap is ordered, and no exit can make it
                // cheaper than walking to it here).
                let w = g.w;
                let mut near: Option<(u32, Square)> = None;
                for (i, &d) in bfs.dist.iter().enumerate() {
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
            for (sq, dir, land, d) in self.exits(rom, node, &bfs.dist) {
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
        let mut w = self.walking?;
        w.frames += 1;
        // A script holding the joypad, even for a frame, is not a wall.
        w.held |= gb.peek(JOY_IGNORE) != 0;
        let moving = gb.peek(self.ram.walk_counter) != 0;
        if w.started {
            if moving && w.frames < STEP * 3 {
                self.walking = Some(w);
                return Some(0);
            }
            self.walking = None;
            return None;
        }
        if moving {
            w.started = true;
            self.walking = Some(w);
            return Some(0);
        }
        if w.frames < STEP {
            self.walking = Some(w);
            return Some(w.dir);
        }
        self.walking = None;
        let ahead = Self::ahead(w.from, w.dir);
        let person = self.people.contains(&(ahead.1.into(), ahead.2.into()));
        if self.here(gb) == w.from && !person && !w.held {
            *self.walls.entry(Side(w.from, w.dir)).or_insert(0) += 1;
        }
        None
    }

    /// A square the game hangs on (a script that never ends): never
    /// stand on it again.
    pub fn trap(&mut self, s: Square) {
        self.set_walk(s, false);
        self.traps.push(s);
    }

    fn set_walk(&mut self, s: Square, walk: bool) {
        if let Some(g) = self.grids.get_mut(&s.0) {
            let i = usize::from(s.2) * g.w + usize::from(s.1);
            if let Some(w) = g.walk.get_mut(i) {
                *w = walk;
            }
        }
    }

    /// Whether trees are a way through: the party knows Cut.
    pub fn set_cut(&mut self, cut: bool) {
        if self.rules.cut != cut {
            self.rules.cut = cut;
            self.leg = None;
        }
    }

    /// Squares a script opens by changing the map's blocks in RAM, which
    /// the cartridge's copy never shows.
    pub fn open(&mut self, rom: &[u8], map: u8, squares: &[(u8, u8)]) {
        if self.grid(rom, map).is_some() {
            for &(x, y) in squares {
                self.set_walk((map, x, y), true);
            }
        }
    }

    /// Someone stood on this square of the live map, last time it looked.
    pub fn person(&self, s: Square) -> bool {
        self.people.contains(&(s.1.into(), s.2.into()))
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
        self.walking = Some(Walk {
            dir,
            frames: 0,
            from: here,
            started: false,
            held: false,
        });
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
        self.cut.retain(|s| s.0 == here.0);
        let rom = gb.rom();
        if let Some(g) = self.grid(rom, here.0) {
            if goal(g, here) {
                self.leg = None;
                return Some(0);
            }
        }
        // A trainer who walked up to battle stays where he stopped: a goal
        // square with someone on it is no goal.
        let (w, h) = self.grids.get(&here.0).map_or((0, 0), |g| (g.w, g.h));
        self.read_people(gb, w, h);
        let people = std::mem::take(&mut self.people);
        let free = |g: &Grid, s: Square| {
            goal(g, s) && (s.0 != here.0 || !people.contains(&(s.1.into(), s.2.into())))
        };
        if let Some((t, None)) = self.leg {
            if t.0 == here.0 && self.grids.get(&t.0).is_some_and(|g| !free(g, t)) {
                self.leg = None;
            }
        }
        // Planned once per map: the live search below walks the leg.
        if self.leg.is_none_or(|(t, _)| t.0 != here.0) {
            self.leg = self.route(gb.rom(), here, &free);
            // Walls learned from a person who has since moved, or a trap
            // in the only corridor, can cut the only way; forget them and
            // look again.
            if self.leg.is_none() && !(self.walls.is_empty() && self.traps.is_empty()) {
                self.walls.clear();
                for s in std::mem::take(&mut self.traps) {
                    self.set_walk(s, true);
                }
                self.leg = self.route(gb.rom(), here, &free);
            }
        }
        self.people = people;
        let (target, leave) = self.leg?;
        if (target.1, target.2) == (here.1, here.2) {
            let Some(leave) = leave else {
                // The goal moved off this square (a person stood on it).
                self.leg = None;
                return None;
            };
            let dir = if leave == 0 { self.last_dir } else { leave };
            // A way out that did not take: try the side least bumped.
            let bumps = |d: u8| self.walls.get(&Side(here, d)).copied().unwrap_or(0);
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
        let ahead = Self::ahead(here, dir);
        if self.standing_tree(gb.rom(), ahead) {
            // Turned to face it first: Cut takes the tile in front. Pushing
            // into the tree meanwhile is harmless.
            if facing(gb.peek(FACING)) == dir {
                self.tree = Some(ahead);
                self.cut.push(ahead);
            }
            return Some(dir);
        }
        Some(self.hold(gb, dir))
    }

    /// A tree on this square that has not been cut since the player came.
    fn standing_tree(&mut self, rom: &[u8], s: Square) -> bool {
        if !self.rules.cut || self.cut.contains(&s) {
            return false;
        }
        self.grid(rom, s.0).is_some_and(|g| {
            let i = usize::from(s.2) * g.w + usize::from(s.1);
            g.walk.get(i) == Some(&false) && g.tile.get(i).is_some_and(|&t| is_tree(g, t))
        })
    }

    /// The first step toward `to` on the live map, people (as `toward` just
    /// read them) avoided; failing that, ignoring them (they move).
    fn step_toward(&mut self, gb: &mut GameBoy, here: Square, to: (u8, u8)) -> Option<u8> {
        let rom = gb.rom();
        self.grid(rom, here.0)?;
        let g = &self.grids[&here.0];
        let i = usize::from(to.1) * g.w + usize::from(to.0);
        // Around people, unless that is far longer than waiting for them
        // to move: the long way round can be a one-way loop over a ledge.
        let mut steps = [(u32::MAX, 0u8); 2];
        for (k, people) in [true, false].into_iter().enumerate() {
            let blocked: &[(usize, usize)] = if people { &self.people } else { &[] };
            self.bfs.run(
                g,
                &self.rules,
                &self.walls,
                here.0,
                (here.1.into(), here.2.into()),
                blocked,
            );
            if let (Some(&d), Some(&f)) = (self.bfs.dist.get(i), self.bfs.first.get(i)) {
                steps[k] = (d, f);
            }
        }
        let [(around, a), (through, t)] = steps;
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

fn is_tree(g: &Grid, tile: u8) -> bool {
    (g.tileset == OVERWORLD && tile == TREE) || (g.tileset == GYM && tile == GYM_TREE)
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
