//! The mayor: grows a city on a block lattice, keeps it powered and policed,
//! and puts back whatever a disaster knocks down.
//!
//! Everything the mayor has decided on is a *want*: a road tile, a building,
//! a park tile, a power line over a road. Each call builds the first want the
//! map does not show yet, so the same pass that lays a new block's roads also
//! rebuilds a burnt-out zone. Only when every want stands does the mayor plan
//! more, one decision at a time, from the engine's demand valves.
//!
//! Blocks are a road ring around two rows of three 3x3 slots, so every zone
//! touches a road and zones in a block touch each other, which carries power.
//! Across a road, power needs a line over it: one per facing pair of slots,
//! and a zone still cut off from every plant gets the cheapest run of wire
//! to one (`power.rs`).

use super::engine::Stats;
use super::power;
use super::tiles::{
    is_clear, is_hazard, is_road, is_water, COMCLR, CONDBIT, FREEZ, HPOWER, INDCLR, LASTPOWER,
    LOMASK, TREEBASE, WOODS5, ZONEBIT,
};
use crate::next_rand;

pub const W: i32 = 120;
pub const H: i32 = 100;
const BW: i32 = 10;
const BH: i32 = 7;
const FOUNTAIN: u16 = 840;
const RESBASE: u16 = 240;
/// Blocks that far from the centre are never opened: the map edge is ugly
/// and the engine's traffic dies off with distance anyway.
const MAX_RING: f32 = 9.0;
/// A want that fails this often is dropped, and its slot left alone.
const MAX_FAILS: u8 = 4;
/// Mayor calls (two city years) after which an empty zone stops counting as
/// room to grow.
const STALE: u32 = 768;
/// How far a fire station's cover reaches, in blocks.
const FIRE_REACH: f32 = 3.0;
const KEPT: u8 = 1;
const SITE: u8 = 2;

/// `EditingTool` in the engine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Res = 0,
    Com = 1,
    Ind = 2,
    Fire = 3,
    Police = 4,
    Wire = 6,
    Bulldozer = 7,
    Road = 9,
    Stadium = 10,
    Park = 11,
    Seaport = 12,
    Coal = 13,
    Nuclear = 14,
    Airport = 15,
}

impl Tool {
    fn cost(self) -> i32 {
        match self {
            Tool::Res | Tool::Com | Tool::Ind => 100,
            Tool::Fire | Tool::Police => 500,
            Tool::Wire => 5,
            Tool::Bulldozer => 1,
            Tool::Road | Tool::Park => 10,
            Tool::Stadium | Tool::Nuclear => 5000,
            Tool::Seaport | Tool::Coal => 3000,
            Tool::Airport => 10000,
        }
    }

    pub fn size(self) -> i32 {
        match self {
            Tool::Stadium | Tool::Seaport | Tool::Coal | Tool::Nuclear => 4,
            Tool::Airport => 6,
            Tool::Wire | Tool::Bulldozer | Tool::Road | Tool::Park => 1,
            _ => 3,
        }
    }
}

/// `ToolResult`.
pub const OK: i32 = 1;
pub const NO_MONEY: i32 = -2;
pub const NEED_BULLDOZE: i32 = -1;

/// What the mayor does to the city: the engine's tools, or a test's fake.
pub trait Hands {
    fn tool(&mut self, tool: Tool, x: i32, y: i32) -> i32;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// Homes and shops together: a trip to the shops stays short, and so
    /// does the trail of traffic smog it leaves.
    Town,
    Ind,
    Utility,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Free,
    Taken,
    /// A slot of trees: housing grows on the land value they lend.
    Park,
    Unusable,
}

struct Block {
    /// The top-left corner of its road ring.
    x: i32,
    y: i32,
    /// Offset from the centre block, in blocks.
    v: (f32, f32),
    kind: Option<Kind>,
    slots: [Slot; 6],
}

impl Block {
    fn slot_centre(&self, i: usize) -> (i32, i32) {
        (
            self.x + 2 + 3 * (i % 3) as i32,
            self.y + 2 + 3 * (i / 3) as i32,
        )
    }

    fn dist(&self) -> f32 {
        self.v.0.hypot(self.v.1)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Want {
    Road(i32, i32),
    Building(Tool, i32, i32),
    Park(i32, i32),
    /// A power line over the road tile here.
    Crossing(i32, i32),
    /// A power line on its own, in a loaded city.
    Wire(i32, i32),
}

pub struct Mayor {
    blocks: Vec<Block>,
    /// What the mayor has decided on, how often building it failed, and the
    /// call it was decided on.
    wants: Vec<(Want, u8, u32)>,
    calls: u32,
    /// The way industry lies from the centre, as a unit vector; housing
    /// spreads the other way.
    ind_dir: (f32, f32),
    /// Per map cell: `KEPT` for land a building or road may yet want, which
    /// a power line should go round; `SITE` under a wanted building, where
    /// one must not go.
    keep: Vec<u8>,
    pub actions: u32,
}

fn at(map: &[u16], x: i32, y: i32) -> u16 {
    if (0..W).contains(&x) && (0..H).contains(&y) {
        map[(x * H + y) as usize]
    } else {
        u16::MAX
    }
}

fn tile(map: &[u16], x: i32, y: i32) -> u16 {
    at(map, x, y) & LOMASK
}

/// Land a block may be built on: not water, not off the map, and in a loaded
/// city not already somebody else's.
fn free(map: &[u16], x: i32, y: i32) -> bool {
    let t = tile(map, x, y);
    at(map, x, y) != u16::MAX && !is_water(t) && is_clear(t)
}

impl Mayor {
    /// A mayor for `map`: a fresh terrain, or a loaded city whose roads,
    /// buildings and power lines become wants, so the mayor repairs them.
    pub fn new(map: &[u16], seed: u32) -> Self {
        let mut rng = seed | 1;
        let a = (next_rand(&mut rng) % 6283) as f32 / 1000.0;
        let mut m = Self {
            blocks: Vec::new(),
            wants: Vec::new(),
            calls: 0,
            ind_dir: (a.cos(), a.sin()),
            keep: vec![0; (W * H) as usize],
            actions: 0,
        };
        m.adopt(map);
        m.lay_lattice(map);
        m.update_keep();
        m
    }

    fn update_keep(&mut self) {
        for k in &mut self.keep {
            *k = (*k == SITE) as u8 * SITE;
        }
        for b in &self.blocks {
            let (x0, x1, y0, y1) = if b.kind.is_none() {
                (b.x, b.x + BW, b.y, b.y + BH)
            } else {
                (b.x + 1, b.x + BW - 1, b.y + 1, b.y + BH - 1)
            };
            for x in x0..=x1 {
                for y in y0..=y1 {
                    let k = &mut self.keep[(x * H + y) as usize];
                    *k = (*k).max(KEPT);
                }
            }
        }
    }

    fn adopt(&mut self, map: &[u16]) {
        for x in 0..W {
            for y in 0..H {
                let v = at(map, x, y);
                let t = v & LOMASK;
                if v & ZONEBIT != 0 {
                    if let Some(tool) = building_of(t) {
                        self.want_building(tool, x, y);
                    }
                } else if is_road(t) {
                    self.wants.push((Want::Road(x, y), 0, self.calls));
                } else if (HPOWER..=LASTPOWER - 2).contains(&t) {
                    self.wants.push((Want::Wire(x, y), 0, self.calls));
                }
            }
        }
    }

    /// The lattice offset and centre with the most free blocks near the
    /// centre, which for a loaded city is the middle of what is built.
    fn lay_lattice(&mut self, map: &[u16]) {
        let built: Vec<(i32, i32)> = (0..W)
            .flat_map(|x| (0..H).map(move |y| (x, y)))
            .filter(|&(x, y)| {
                let t = tile(map, x, y);
                !is_clear(t) && !is_water(t)
            })
            .collect();
        let mut best = (f32::MIN, 0, 0, (0, 0));
        for ox in 0..BW {
            for oy in 0..BH {
                let blocks = lattice(ox, oy);
                let land: Vec<bool> = blocks.iter().map(|&(x, y)| block_free(map, x, y)).collect();
                let centres: Vec<(i32, i32)> = if built.len() > 40 {
                    let n = built.len() as i32;
                    let (sx, sy) = built.iter().fold((0, 0), |s, p| (s.0 + p.0, s.1 + p.1));
                    vec![(sx / n, sy / n)]
                } else {
                    blocks
                        .iter()
                        .map(|&(x, y)| (x + BW / 2, y + BH / 2))
                        .collect()
                };
                for &(cx, cy) in &centres {
                    let score: f32 = blocks
                        .iter()
                        .zip(&land)
                        .filter(|(_, &l)| l)
                        .map(|(&(x, y), _)| {
                            let d = (((x + BW / 2 - cx) as f32 / BW as f32).powi(2)
                                + ((y + BH / 2 - cy) as f32 / BH as f32).powi(2))
                            .sqrt();
                            (4.0 - d).max(0.0)
                        })
                        .sum();
                    if score > best.0 {
                        best = (score, ox, oy, (cx, cy));
                    }
                }
            }
        }
        let (_, ox, oy, (cx, cy)) = best;
        self.blocks = lattice(ox, oy)
            .into_iter()
            .filter(|&(x, y)| block_free(map, x, y))
            .map(|(x, y)| Block {
                x,
                y,
                v: (
                    (x + BW / 2 - cx) as f32 / BW as f32,
                    (y + BH / 2 - cy) as f32 / BH as f32,
                ),
                kind: None,
                slots: std::array::from_fn(|i| {
                    let (sx, sy) = (x + 2 + 3 * (i % 3) as i32, y + 2 + 3 * (i / 3) as i32);
                    let ok = (-1..=1).all(|d| (-1..=1).all(|e| free(map, sx + d, sy + e)));
                    if ok {
                        Slot::Free
                    } else {
                        Slot::Unusable
                    }
                }),
            })
            .filter(|b| b.dist() <= MAX_RING)
            .collect();
    }

    /// Build the first want the map lacks, or plan one more. Returns whether
    /// it did anything; at most one tool succeeds per call.
    pub fn act(&mut self, map: &[u16], s: &Stats, hands: &mut impl Hands) -> bool {
        self.calls = self.calls.wrapping_add(1);
        match self.build(map, s, hands) {
            Some(done) => done,
            None => self.plan(map, s),
        }
    }

    /// `None` when every want stands; otherwise whether a tool succeeded.
    fn build(&mut self, map: &[u16], s: &Stats, hands: &mut impl Hands) -> Option<bool> {
        // Most urgent first: after a disaster, power before roads before
        // homes, and the savings wait for the plant rather than going on
        // roads to a dark city.
        let (i, (tool, x, y)) = self
            .wants
            .iter()
            .enumerate()
            .filter_map(|(i, &(w, ..))| next_step(map, w).map(|st| (i, st)))
            .min_by_key(|&(i, _)| urgency(self.wants[i].0))?;
        let (want, fails, _) = self.wants[i];
        if s.funds < tool.cost() {
            return Some(false);
        }
        let r = hands.tool(tool, x, y);
        if r == OK {
            self.actions += 1;
            return Some(true);
        }
        if r == NO_MONEY {
            return Some(false);
        }
        if r == NEED_BULLDOZE {
            if let Want::Building(t, cx, cy) = want {
                if clear_site(map, t, cx, cy, hands) {
                    self.actions += 1;
                    return Some(true);
                }
            }
        }
        self.wants[i].1 = fails + 1;
        if fails + 1 >= MAX_FAILS {
            self.wants.remove(i);
        }
        Some(false)
    }

    fn plan(&mut self, map: &[u16], s: &Stats) -> bool {
        let cap = 700 * s.coal + 2000 * s.nuclear;
        let load = load(map);
        if self.needs_power(map, s, cap, load) {
            let tool = if s.funds >= 12000 && s.pop > 30000 && s.nuclear < 2 {
                Tool::Nuclear
            } else {
                Tool::Coal
            };
            return s.funds >= tool.cost() && self.big(tool);
        }
        if s.funds < 300 {
            return false;
        }
        if let Some(path) = power::route(map, |i| match self.keep[i] {
            0 => Some(0),
            KEPT => Some(12),
            _ => None,
        }) {
            for i in path {
                let (x, y) = ((i / H as usize) as i32, (i % H as usize) as i32);
                self.wants.push((Want::Wire(x, y), 0, self.calls));
            }
            return true;
        }
        // A plant's worth kept back once the grid is two-thirds drawn, so
        // growth never outruns power it cannot then afford; and, except from
        // a stadium, seaport or airport, once the city is a town, so a quake
        // that topples a plant is not the end.
        let near_cap = load * 3 > cap * 2;
        let mut reserve = 800 + if near_cap { Tool::Coal.cost() } else { 0 };
        // A capped zone type grows no more until its stadium, seaport or
        // airport stands: the best thing the money can buy, so save for it.
        let mut saving = 0;
        for (cap, have, tool) in [
            (s.res_cap, s.stadium, Tool::Stadium),
            (s.ind_cap, s.seaport, Tool::Seaport),
            (s.com_cap, s.airport, Tool::Airport),
        ] {
            if cap != 0 && have == 0 && self.count(|t| t == tool) == 0 {
                if s.funds >= tool.cost() + reserve {
                    return self.big(tool);
                }
                saving += tool.cost();
            }
        }
        reserve += saving;
        if s.pop > 10_000 && !near_cap {
            reserve += Tool::Coal.cost();
        }
        if s.funds < reserve {
            return false;
        }
        let zones = self.count(|t| matches!(t, Tool::Res | Tool::Com | Tool::Ind));
        let police = self.count(|t| t == Tool::Police);
        // A station costs a hundred a year, as much as a whole block's
        // roads: one each early on, more only as the city outgrows them.
        if police * 40 + 20 <= zones || (s.crime > 120 && police * 25 < zones) {
            return self.zone(Tool::Police, Kind::Town);
        }
        if zones >= 30 {
            if let Some(bi) = self.uncovered() {
                return self.zone_in(Tool::Fire, bi);
            }
        }
        let want = 1 + zones / 40;
        let mut demand = [
            (s.res_valve as f32 / 2000.0, Tool::Res, Kind::Town),
            (s.com_valve as f32 / 1500.0, Tool::Com, Kind::Town),
            (s.ind_valve as f32 / 1500.0, Tool::Ind, Kind::Ind),
        ];
        demand.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (d, tool, kind) in demand {
            // An empty city's valves start near zero; seed all three.
            let starting = zones < 3 && self.count(|t| t == tool) == 0;
            let (all, fresh) = self.vacant(map, tool);
            if (d > 0.0 || starting) && fresh < want && all < want + 2 + zones / 20 {
                return self.zone(tool, kind);
            }
        }
        false
    }

    /// The tax rate for these funds: up when broke, down when flush.
    pub fn tax(s: &Stats) -> i32 {
        if s.funds < 1500 {
            9
        } else if s.funds < 5000 {
            8
        } else if s.funds > 30000 {
            6
        } else {
            7
        }
    }

    fn count(&self, f: impl Fn(Tool) -> bool) -> usize {
        self.wants
            .iter()
            .filter(|(w, ..)| matches!(*w, Want::Building(t, ..) if f(t)))
            .count()
    }

    /// Zones of `tool` nobody has moved into yet: all of them, and those
    /// zoned in the last two city years. One left empty longer may never
    /// fill (the smog or the commute is wrong there), so it should not stall
    /// the city; but a lot of them means more lots would only sit empty too.
    fn vacant(&self, map: &[u16], tool: Tool) -> (usize, usize) {
        let empty: Vec<u32> = self
            .wants
            .iter()
            .filter(|(w, ..)| match *w {
                Want::Building(t, x, y) if t == tool => {
                    let c = tile(map, x, y);
                    match tool {
                        Tool::Res => {
                            c == FREEZ
                                && (-1..=1).all(|d| {
                                    (-1..=1).all(|e| {
                                        (RESBASE..=RESBASE + 8).contains(&tile(map, x + d, y + e))
                                    })
                                })
                        }
                        Tool::Com => c == COMCLR,
                        _ => c == INDCLR,
                    }
                }
                _ => false,
            })
            .map(|&(_, _, born)| born)
            .collect();
        let fresh = empty
            .iter()
            .filter(|&&born| self.calls.wrapping_sub(born) < STALE)
            .count();
        (empty.len(), fresh)
    }

    /// Whether what is built and wanted would draw more than the plants
    /// make, with a plant's worth of slack for what is still to come.
    fn needs_power(&self, map: &[u16], s: &Stats, cap: i32, load: i32) -> bool {
        let planned = self.count(|t| matches!(t, Tool::Coal | Tool::Nuclear));
        if planned as i32 > s.coal + s.nuclear {
            return false;
        }
        let unbuilt: i32 = self
            .wants
            .iter()
            .filter_map(|(w, ..)| match *w {
                Want::Building(t, x, y) if at(map, x, y) & ZONEBIT == 0 => {
                    Some(t.size() * t.size())
                }
                _ => None,
            })
            .sum();
        cap == 0 || load + unbuilt + 40 > cap * 9 / 10
    }

    /// An opened block with room for a fire station and none near: a fire
    /// left to burn takes the district with it, plants and all.
    fn uncovered(&self) -> Option<usize> {
        let stations: Vec<(i32, i32)> = self
            .wants
            .iter()
            .filter_map(|(w, ..)| match *w {
                Want::Building(Tool::Fire, x, y) => Some((x, y)),
                _ => None,
            })
            .collect();
        let near = |b: &Block| {
            stations.iter().any(|&(x, y)| {
                let dx = (x - b.x - BW / 2) as f32 / BW as f32;
                let dy = (y - b.y - BH / 2) as f32 / BH as f32;
                dx.hypot(dy) <= FIRE_REACH
            })
        };
        (0..self.blocks.len()).find(|&bi| {
            let b = &self.blocks[bi];
            b.kind.is_some() && b.slots.contains(&Slot::Free) && !near(b)
        })
    }

    /// Put `tool` in a free slot of block `bi`, which has one.
    fn zone_in(&mut self, tool: Tool, bi: usize) -> bool {
        let Some(i) = (0..6).find(|&i| self.blocks[bi].slots[i] == Slot::Free) else {
            return false;
        };
        self.blocks[bi].slots[i] = Slot::Taken;
        let (x, y) = self.blocks[bi].slot_centre(i);
        self.want_building(tool, x, y);
        true
    }

    /// Put `tool` (a 3x3) in a free slot of a `kind` block, opening one if
    /// none is free.
    fn zone(&mut self, tool: Tool, kind: Kind) -> bool {
        let found = self
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| b.kind == Some(kind))
            .flat_map(|(bi, b)| {
                (0..6)
                    .filter(move |&i| b.slots[i] == Slot::Free)
                    .map(move |i| (bi, i))
            })
            .min_by(|a, b| self.blocks[a.0].dist().total_cmp(&self.blocks[b.0].dist()));
        let (bi, i) = match found {
            Some(f) => f,
            None => match self.open(kind) {
                Some(bi) => match (0..6).find(|&i| self.blocks[bi].slots[i] == Slot::Free) {
                    Some(i) => (bi, i),
                    None => return false,
                },
                None => return false,
            },
        };
        let b = &mut self.blocks[bi];
        b.slots[i] = Slot::Taken;
        let (x, y) = b.slot_centre(i);
        self.want_building(tool, x, y);
        if kind == Kind::Town && tool == Tool::Res && !self.blocks[bi].slots.contains(&Slot::Park) {
            if let Some(j) = [4, 1, 3, 5, 0, 2]
                .into_iter()
                .find(|&j| self.blocks[bi].slots[j] == Slot::Free)
            {
                self.blocks[bi].slots[j] = Slot::Park;
                let (px, py) = self.blocks[bi].slot_centre(j);
                for d in -1..=1 {
                    for e in -1..=1 {
                        self.wants.push((Want::Park(px + d, py + e), 0, self.calls));
                    }
                }
            }
        }
        true
    }

    /// A plant, stadium, seaport or airport in the top-left of a new utility
    /// block; its other two slots stay free for zones and the rest is park.
    fn big(&mut self, tool: Tool) -> bool {
        let Some(bi) = (0..self.blocks.len())
            .filter(|&bi| {
                let b = &self.blocks[bi];
                b.kind.is_none() && b.slots.iter().all(|&s| s == Slot::Free)
            })
            .filter(|&bi| self.touches_open(bi))
            .min_by(|&a, &b| {
                self.score(a, Kind::Utility)
                    .total_cmp(&self.score(b, Kind::Utility))
            })
        else {
            return false;
        };
        self.open_block(bi, Kind::Utility);
        let b = &mut self.blocks[bi];
        for i in [0, 1, 3, 4] {
            b.slots[i] = Slot::Taken;
        }
        let (x, y) = (b.x, b.y);
        self.want_building(tool, x + 2, y + 2);
        let n = tool.size();
        for py in y + 1..y + 7 {
            for px in x + 1..x + 7 {
                if px > x + n || py > y + n {
                    self.wants.push((Want::Park(px, py), 0, self.calls));
                }
            }
        }
        true
    }

    fn touches_open(&self, bi: usize) -> bool {
        let b = &self.blocks[bi];
        let none_open = self.blocks.iter().all(|o| o.kind.is_none());
        none_open && b.dist() < 1.5
            || self
                .blocks
                .iter()
                .any(|o| o.kind.is_some() && (o.x - b.x).abs() + (o.y - b.y).abs() <= BW.max(BH))
    }

    /// Lower is better. Industry and its plants sit on one side of the
    /// centre, housing on the other, commerce between: smoke drifts over
    /// nobody's house, and every home still reaches shops within the
    /// engine's trip length.
    fn score(&self, bi: usize, kind: Kind) -> f32 {
        let b = &self.blocks[bi];
        let p = b.v.0 * self.ind_dir.0 + b.v.1 * self.ind_dir.1;
        b.dist()
            + match kind {
                Kind::Ind => 4.0 * (1.0 - p).max(0.0),
                Kind::Town => 4.0 * (p + 0.5).max(0.0),
                Kind::Utility => 4.0 * (3.0 - p).max(0.0),
            }
    }

    /// The best unopened block beside the city for `kind`, opened.
    fn open(&mut self, kind: Kind) -> Option<usize> {
        let bi = (0..self.blocks.len())
            .filter(|&bi| {
                let b = &self.blocks[bi];
                b.kind.is_none() && b.slots.contains(&Slot::Free)
            })
            .filter(|&bi| self.touches_open(bi))
            .min_by(|&a, &b| self.score(a, kind).total_cmp(&self.score(b, kind)))?;
        self.open_block(bi, kind);
        Some(bi)
    }

    /// Want the block's road ring and the power lines over it.
    fn open_block(&mut self, bi: usize, kind: Kind) {
        let b = &mut self.blocks[bi];
        b.kind = Some(kind);
        let (x, y) = (b.x, b.y);
        let ring = (x..=x + BW)
            .flat_map(|px| [(px, y), (px, y + BH)])
            .chain((y + 1..y + BH).flat_map(|py| [(x, py), (x + BW, py)]));
        for (px, py) in ring {
            let w = Want::Road(px, py);
            if !self.wants.iter().any(|(o, ..)| *o == w) {
                self.wants.push((w, 0, self.calls));
            }
        }
        for c in [2, 5, 8] {
            for py in [y, y + BH] {
                self.crossing(x + c, py);
            }
        }
        for r in [2, 5] {
            for px in [x, x + BW] {
                self.crossing(px, y + r);
            }
        }
    }

    #[cfg(test)]
    pub fn top(&self, map: &[u16]) -> String {
        let mut v: Vec<(u8, String)> = self
            .wants
            .iter()
            .filter_map(|&(w, f, _)| {
                next_step(map, w).map(|st| (urgency(w), format!("{w:?} f{f} {st:?}")))
            })
            .collect();
        v.sort();
        format!("{} pending: {:?}", v.len(), &v[..v.len().min(3)])
    }

    /// Want a building, its site closed to power lines, and any line the
    /// mayor meant to run across it forgotten.
    fn want_building(&mut self, tool: Tool, x: i32, y: i32) {
        let n = tool.size();
        let site =
            |px: i32, py: i32| (x - 1..x - 1 + n).contains(&px) && (y - 1..y - 1 + n).contains(&py);
        self.wants
            .retain(|(w, ..)| !matches!(*w, Want::Wire(px, py) if site(px, py)));
        for px in x - 1..x - 1 + n {
            for py in y - 1..y - 1 + n {
                if (0..W).contains(&px) && (0..H).contains(&py) {
                    self.keep[(px * H + py) as usize] = SITE;
                }
            }
        }
        self.wants.push((Want::Building(tool, x, y), 0, self.calls));
    }

    fn crossing(&mut self, x: i32, y: i32) {
        let w = Want::Crossing(x, y);
        if !self.wants.iter().any(|(o, ..)| *o == w) {
            self.wants.push((w, 0, self.calls));
        }
    }
}

/// Lower builds first.
fn urgency(w: Want) -> u8 {
    match w {
        Want::Building(Tool::Coal | Tool::Nuclear, ..) => 0,
        Want::Wire(..) | Want::Crossing(..) => 1,
        Want::Road(..) => 2,
        Want::Building(Tool::Fire | Tool::Police, ..) => 3,
        Want::Building(Tool::Res | Tool::Com | Tool::Ind, ..) => 4,
        Want::Building(..) => 5,
        Want::Park(..) => 6,
    }
}

/// Power the grid draws: one unit per conductive tile, as the engine counts.
fn load(map: &[u16]) -> i32 {
    map.iter().filter(|&&v| v & CONDBIT != 0).count() as i32
}

/// Block corners for lattice offset `ox`, `oy`, whole blocks on the map only.
fn lattice(ox: i32, oy: i32) -> Vec<(i32, i32)> {
    let mut v = Vec::new();
    let mut y = oy;
    while y + BH < H {
        let mut x = ox;
        while x + BW < W {
            v.push((x, y));
            x += BW;
        }
        y += BH;
    }
    v
}

/// A block whose road ring is free land and that has a usable slot.
fn block_free(map: &[u16], x: i32, y: i32) -> bool {
    let ring = (x..=x + BW).all(|px| free(map, px, y) && free(map, px, y + BH))
        && (y..=y + BH).all(|py| free(map, x, py) && free(map, x + BW, py));
    ring && (0..6).any(|i| {
        let (sx, sy) = (x + 2 + 3 * (i % 3), y + 2 + 3 * (i / 3));
        (-1..=1).all(|d| (-1..=1).all(|e| free(map, sx + d, sy + e)))
    })
}

/// The tool that rebuilds the building whose centre tile is `t`.
fn building_of(t: u16) -> Option<Tool> {
    Some(match t {
        240..=422 | 956..=1018 => Tool::Res,
        423..=611 => Tool::Com,
        612..=692 => Tool::Ind,
        693..=708 => Tool::Seaport,
        709..=744 => Tool::Airport,
        745..=760 => Tool::Coal,
        761..=769 => Tool::Fire,
        770..=778 => Tool::Police,
        779..=810 => Tool::Stadium,
        811..=826 => Tool::Nuclear,
        _ => return None,
    })
}

/// The tool use that brings `want` about, or `None` if the map already
/// shows it, or cannot yet (a fire or a flood is in the way).
fn next_step(map: &[u16], want: Want) -> Option<(Tool, i32, i32)> {
    match want {
        Want::Road(x, y) => {
            let t = tile(map, x, y);
            (!is_road(t) && !is_hazard(t) && (is_clear(t) || is_water(t))).then_some((
                Tool::Road,
                x,
                y,
            ))
        }
        Want::Building(tool, x, y) => {
            if at(map, x, y) & ZONEBIT != 0 {
                return None;
            }
            let n = tool.size();
            let hazard = (x - 1..x - 1 + n)
                .any(|px| (y - 1..y - 1 + n).any(|py| is_hazard(tile(map, px, py))));
            (!hazard).then_some((tool, x, y))
        }
        Want::Park(x, y) => {
            let t = tile(map, x, y);
            if (TREEBASE..=WOODS5).contains(&t) || t == FOUNTAIN || !is_clear(t) {
                return None;
            }
            Some(if t == 0 {
                (Tool::Park, x, y)
            } else {
                (Tool::Bulldozer, x, y)
            })
        }
        Want::Crossing(x, y) => {
            let t = tile(map, x, y);
            if !(66..=67).contains(&t) {
                return None;
            }
            // 66 runs left-right, so the line crosses it top to bottom.
            let (a, b) = if t == 66 {
                ((x, y - 1), (x, y + 1))
            } else {
                ((x - 1, y), (x + 1, y))
            };
            let conducts = |(px, py): (i32, i32)| {
                let v = at(map, px, py);
                v != u16::MAX && v & CONDBIT != 0 && !is_road(v & LOMASK)
            };
            (conducts(a) && conducts(b)).then_some((Tool::Wire, x, y))
        }
        Want::Wire(x, y) => {
            let t = tile(map, x, y);
            let open = (is_clear(t) && !is_water(t)) || (66..=67).contains(&t);
            (open && !is_hazard(t) && at(map, x, y) & CONDBIT == 0).then_some((Tool::Wire, x, y))
        }
    }
}

/// Bulldoze whatever stands in a wanted building's footprint that the
/// engine will not clear itself: the stumps of a zone a quake broke up.
fn clear_site(map: &[u16], tool: Tool, x: i32, y: i32, hands: &mut impl Hands) -> bool {
    let n = tool.size();
    for py in y - 1..y - 1 + n {
        for px in x - 1..x - 1 + n {
            let t = tile(map, px, py);
            if !is_clear(t)
                && !is_road(t)
                && !is_water(t)
                && hands.tool(Tool::Bulldozer, px, py) == OK
            {
                return true;
            }
        }
    }
    false
}
