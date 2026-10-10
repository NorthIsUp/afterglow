//! Getting about Kanto: every map's walkable squares, ledges, warps and
//! edge connections read from the cartridge, a route across maps to any
//! square (Dijkstra over map entrances, each map searched breadth-first),
//! and the next button toward it on the live map, people counted as walls.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use mizu_core::GameBoy;

use super::super::carts::Revision;
use super::super::kanto::{self, Kanto};
use super::super::pilot::{A, B, DOWN, LEFT, RIGHT, UP};
use super::grid::{is_tree, Bfs, Grid, Rules, Side, WALL};
use super::{screen, Ram};

/// A map and a square on it.
pub type Square = (u8, u8, u8);

/// A route's step across the live map: the square to reach, the button
/// that leaves the map from it (`None`: the goal is that square), and the
/// map it leaves for.
type Leg = (Square, Option<u8>, u8);

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

/// A warp's destination map that means "the map you came in from".
const LAST_MAP: u8 = 0xFF;
/// Frames a held direction gets to start a step (a turn on the spot takes
/// a few) before it counts as a bump.
const STEP: u32 = 24;
/// `wJoyIgnore`: buttons the game is ignoring, below `wFontLoaded` so the
/// same address in every revision.
pub const JOY_IGNORE: u16 = 0xCD6B;
/// `wSpritePlayerStateData1FacingDirection`.
pub const FACING: u16 = 0xC109;
/// A sprite's first movement byte for one who stands still.
const STAY: u8 = 0xFF;
/// Extra steps worth walking around people rather than waiting on them.
const DETOUR: u32 = 6;
/// Map entrances a route search expands before it gives up.
const ROUTE_LIMIT: u32 = 4000;
/// The tiles a Card Key opens, as `PrintCardKeyText` knows them.
const DOOR_TILES: [u8; 3] = [0x18, 0x24, 0x5E];
/// Warps that never fire: Celadon's into the Mart's fifth floor, Silph
/// Co.'s on 1F and 11F.
const DEAD_WARPS: [Square; 3] = [(0x06, 39, 19), (0xB5, 16, 10), (0xEB, 5, 5)];
/// Saffron's gatehouses: a thirsty guard turns the player back.
const CLOSED: [u8; 4] = [0x46, 0x49, 0x4C, 0x4F];
/// `wNumberOfWarps`, the live map's warps after it.
const NUM_WARPS: u16 = 0xD3AE;
type Floors = &'static [(u8, u8)];
/// Elevators (Rocket Hideout, Celadon Mart, Silph Co.): the map, where to
/// stand at its panel (facing up), and each floor in the panel's order as
/// the map and warp its doors open onto.
const LIFTS: [(u8, (u8, u8), Floors); 3] = [
    (0xCB, (1, 2), &[(0xC7, 4), (0xC8, 4), (0xCA, 2)]),
    (
        0x7F,
        (3, 1),
        &[(0x7A, 5), (0x7B, 2), (0x7C, 2), (0x7D, 2), (0x88, 2)],
    ),
    (
        0xEC,
        (3, 1),
        &[
            (0xB5, 3),
            (0xCF, 2),
            (0xD0, 2),
            (0xD1, 2),
            (0xD2, 2),
            (0xD3, 2),
            (0xD4, 2),
            (0xD5, 2),
            (0xE9, 2),
            (0xEA, 2),
            (0xEB, 1),
        ],
    ),
];
pub const DIRS: [u8; 4] = [UP, DOWN, LEFT, RIGHT];

/// Where the live map's first warp leads: an elevator's, wherever its
/// panel last sent it.
pub fn lift(gb: &mut GameBoy, ram: Ram) -> u8 {
    gb.peek(ram.at(NUM_WARPS) + 4)
}

/// The tile on screen in `dir` from the player, where
/// `GetTileAndCoordsInFrontOfPlayer` reads it.
fn tile_ahead(gb: &mut GameBoy, dir: u8) -> u8 {
    let (x, y) = match dir {
        UP => (8, 7),
        DOWN => (8, 11),
        LEFT => (6, 9),
        _ => (10, 9),
    };
    gb.peek(screen::TILE_MAP + y * screen::COLS as u16 + x)
}

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
    leg: Option<Leg>,
    /// Scratch for the live search.
    bfs: Bfs,
    /// Squares a person stands on, this map.
    people: Vec<(usize, usize)>,
    /// Yellow's Pikachu walks behind the player in this sprite slot.
    skip_sprite: Option<u16>,
    /// The Poké Ball picture's id, and the squares of the ones on this map:
    /// an item lying in a corridor is picked up, not waited out.
    ball: u8,
    balls: Vec<(usize, usize)>,
    /// Squares of people who never move by themselves, this map's, kept
    /// per sprite slot while they are off screen.
    still: Vec<(usize, usize)>,
    stills: (u8, [Option<(usize, usize)>; 16]),
    /// Trees cut on the map the player is on: they grow back when it
    /// leaves.
    cut: Vec<Square>,
    /// A tree in the way, faced, and already counted as cut: the bot
    /// should cut it.
    pub tree: Option<Square>,
    /// At an elevator's panel, facing it: the floor the route wants, as
    /// its place in the panel's list and its map.
    pub floor: Option<(u8, u8)>,
    /// The guards at Saffron's gates let the player by.
    pub saffron: bool,
    /// Shut doors on the live map that open when faced and pressed.
    pub doors: Vec<Square>,
    /// The live route goes through people who stand still for good:
    /// there was no other.
    pushy: bool,
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
            ball: if rev == Revision::Yellow { 0x47 } else { 0x3D },
            balls: Vec::new(),
            still: Vec::new(),
            stills: (0xFF, [None; 16]),
            cut: Vec::new(),
            tree: None,
            floor: None,
            saffron: false,
            doors: Vec::new(),
            pushy: false,
        }
    }

    /// Learns the ledge table and which outdoor map every building opens
    /// onto, once, found or not.
    fn learn(&mut self, rom: &[u8]) {
        if self.learned {
            return;
        }
        self.learned = true;
        let cut = self.rules.cut;
        self.rules = Rules::of(rom);
        self.rules.cut = cut;
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
        let lift = LIFTS.iter().find(|l| l.0 == from.0).map(|l| l.2);
        for (x, y, to, id) in warps {
            let d = dist
                .get(usize::from(y) * w + usize::from(x))
                .copied()
                .unwrap_or(u32::MAX);
            if d == u32::MAX || DEAD_WARPS.contains(&(from.0, x, y)) {
                continue;
            }
            // An elevator's doors open onto whichever floor its panel picks.
            for &(to, id) in lift.unwrap_or(&[(to, id)]) {
                if CLOSED.contains(&to) && !self.saffron {
                    continue;
                }
                if let Some(land) = self.landing(rom, to, id) {
                    out.push(((from.0, x, y), edge_dir(x.into(), y.into()), land, d));
                }
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
        still: &[(usize, usize)],
    ) -> Option<Leg> {
        let mut heap = BinaryHeap::new();
        let mut best: HashMap<Square, u32> = HashMap::new();
        // Per entrance: the first leg out of `from`'s map.
        let mut via: HashMap<Square, Leg> = HashMap::new();
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
                let (walls, still) = if node == from {
                    (&self.walls, still)
                } else {
                    (&none, &[][..])
                };
                bfs.run(
                    g,
                    &self.rules,
                    walls,
                    node.0,
                    (node.1.into(), node.2.into()),
                    still,
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
                    return Some(if node == from {
                        (s, None, s.0)
                    } else {
                        via[&node]
                    });
                }
            }
            for (sq, dir, land, d) in self.exits(rom, node, &bfs.dist) {
                let c = cost + d + 2;
                if best.get(&land).is_none_or(|&b| c < b) {
                    best.insert(land, c);
                    let leg = if node == from {
                        (sq, Some(dir), land.0)
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

    /// The player stands where the map allows: not mid-warp, where the new
    /// map's number shows a frame or more before the square it lands on.
    pub fn settled(&mut self, gb: &mut GameBoy) -> bool {
        let here = self.here(gb);
        let rom = gb.rom();
        let cut = self.cut.contains(&here);
        self.grid(rom, here.0)
            .is_none_or(|g| cut || g.walkable(here.1.into(), here.2.into()))
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

    /// Squares a script opens or shuts by changing the map's blocks in RAM,
    /// which the cartridge's copy never shows.
    pub fn set_open(&mut self, rom: &[u8], map: u8, squares: &[(u8, u8)], open: bool) {
        let Some(g) = self.grid(rom, map) else {
            return;
        };
        let w = g.w;
        let changed = squares.iter().any(|&(x, y)| {
            g.walk
                .get(usize::from(y) * w + usize::from(x))
                .is_some_and(|&s| s != open)
        });
        if changed {
            for &(x, y) in squares {
                self.set_walk((map, x, y), open);
            }
            self.leg = None;
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

    fn read_people(&mut self, gb: &mut GameBoy, map: u8, w: usize, h: usize) {
        let r = self.ram;
        self.people.clear();
        self.balls.clear();
        if self.stills.0 != map {
            self.stills = (map, [None; 16]);
        }
        for n in 1..16u16 {
            if Some(n) == self.skip_sprite {
                continue;
            }
            let base = r.sprite_data2 + n * 16;
            let (y, x) = (gb.peek(base + 4), gb.peek(base + 5));
            let picture = gb.peek(r.sprite_data1 + n * 16);
            let slot = &mut self.stills.1[usize::from(n)];
            if picture == 0 {
                *slot = None;
            }
            // On screen: hidden people keep their slots, and so do people
            // off screen, so those who stand still are remembered.
            let shown = picture != 0 && gb.peek(r.sprite_data1 + n * 16 + 2) != 0xFF;
            if shown && x >= 4 && y >= 4 {
                let (x, y) = (usize::from(x - 4), usize::from(y - 4));
                if x < w && y < h {
                    self.people.push((x, y));
                    let ball = picture == self.ball;
                    if ball {
                        self.balls.push((x, y));
                    }
                    *slot = (!ball && gb.peek(base + 6) == STAY).then_some((x, y));
                }
            }
        }
        self.still.clear();
        self.still.extend(self.stills.1.iter().flatten());
        for &s in &self.still {
            if !self.people.contains(&s) {
                self.people.push(s);
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
        self.read_people(gb, here.0, w, h);
        let people = std::mem::take(&mut self.people);
        let free = |g: &Grid, s: Square| {
            goal(g, s) && (s.0 != here.0 || !people.contains(&(s.1.into(), s.2.into())))
        };
        if let Some((t, None, _)) = self.leg {
            if t.0 == here.0 && self.grids.get(&t.0).is_some_and(|g| !free(g, t)) {
                self.leg = None;
            }
        }
        // Planned once per map: the live search below walks the leg.
        if self.leg.is_none_or(|(t, ..)| t.0 != here.0) {
            // Around anyone who stands still for good, by another floor if
            // need be, unless there is no other way.
            let still = std::mem::take(&mut self.still);
            self.leg = self.route(gb.rom(), here, &free, &still);
            // Walls learned from a person who has since moved, or a trap
            // in the only corridor, can cut the only way; forget them and
            // look again.
            if self.leg.is_none() && !(self.walls.is_empty() && self.traps.is_empty()) {
                self.walls.clear();
                for s in std::mem::take(&mut self.traps) {
                    self.set_walk(s, true);
                }
                self.leg = self.route(gb.rom(), here, &free, &still);
            }
            self.pushy = self.leg.is_none() && !still.is_empty();
            if self.pushy {
                self.leg = self.route(gb.rom(), here, &free, &[]);
            }
            self.still = still;
        }
        self.people = people;
        let (target, leave, to) = self.leg?;
        if let Some(b) = self.panel(gb, here, leave, to) {
            return Some(b);
        }
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
        let door = self.doors.contains(&ahead) && DOOR_TILES.contains(&tile_ahead(gb, dir));
        if self.balls.contains(&(ahead.1.into(), ahead.2.into())) || door {
            // Turned to face it, then A held: the overworld loop polls the
            // joypad slower than a frame, so a one-frame tap can fall
            // between polls.
            return Some(if facing(gb.peek(FACING)) == dir {
                A
            } else {
                dir
            });
        }
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

    /// In an elevator going the wrong way: to its panel, facing it, and
    /// then `floor` names the floor to pick.
    fn panel(&mut self, gb: &mut GameBoy, here: Square, leave: Option<u8>, to: u8) -> Option<u8> {
        let &(_, at, floors) = LIFTS.iter().find(|l| l.0 == here.0)?;
        if leave.is_none() || lift(gb, self.ram) == to {
            return None;
        }
        if (here.1, here.2) != at {
            let dir = self.step_toward(gb, here, at)?;
            return Some(self.hold(gb, dir));
        }
        if facing(gb.peek(FACING)) != UP {
            return Some(self.hold(gb, UP));
        }
        let i = floors.iter().position(|f| f.0 == to)?;
        self.floor = Some((i as u8, to));
        // Not 0, which callers take for "arrived": B does nothing here.
        Some(B)
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
        // Never waiting on one who stands still for good, unless there is
        // no other way.
        let mut steps = [(u32::MAX, 0u8); 3];
        let blocks: [&[(usize, usize)]; 3] = [&self.people, &self.still, &[]];
        for (k, blocked) in blocks.into_iter().enumerate() {
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
        let [(around, a), (past, p), (through, t)] = steps;
        if around != u32::MAX && around <= past.min(through).saturating_add(DETOUR) {
            Some(a)
        } else if past != u32::MAX {
            Some(p)
        } else {
            // Through someone who never moves only when the route found no
            // way around them: otherwise the route is stale.
            (through != u32::MAX && self.pushy).then_some(t)
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
