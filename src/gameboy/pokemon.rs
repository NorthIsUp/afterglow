//! The Pokémon bot: plays Red, Blue or Yellow forever from the game's own
//! RAM, laid out by pret's pokered and pokeyellow disassemblies.
//!
//! It is not a speedrun and does not finish the game. It routes over the
//! map's walkable squares (read from the cartridge, `kanto.rs`) to whichever
//! door, staircase or map edge it has used least, so it leaves rooms and
//! roams the routes; it reads every text box, fights every battle with the
//! first move, and when the party faints the game itself sends it back to
//! the last Pokémon Center, healed. The one story beat it is taught is
//! taking a starter from Oak's table. A scripted moment that holds it still
//! for minutes rewinds it to a recent save state.

use std::collections::HashMap;

use mizu_core::GameBoy;

use super::carts::Revision;
use super::kanto::{self, Kanto};
use super::pilot::{A, B, DOWN, LEFT, RIGHT, START, UP};
use crate::next_rand;

/// The WRAM addresses the bot and the world view read. Red and Blue share
/// one layout; Yellow's sits a byte lower from `wFontLoaded` up.
#[derive(Clone, Copy, Debug)]
pub struct Ram {
    pub font_loaded: u16,
    pub walk_counter: u16,
    pub in_battle: u16,
    pub cur_map: u16,
    pub y: u16,
    pub x: u16,
    pub tileset: u16,
    pub party_count: u16,
    pub sprite_data1: u16,
    pub sprite_data2: u16,
    pub num_warps: u16,
    pub warps: u16,
    pub player_name: u16,
    pub rival_name: u16,
    pub moving_direction: u16,
}

impl Ram {
    pub const fn of(r: Revision) -> Self {
        match r {
            Revision::Red | Revision::Blue => Self {
                font_loaded: 0xCFC4,
                walk_counter: 0xCFC5,
                in_battle: 0xD057,
                cur_map: 0xD35E,
                y: 0xD361,
                x: 0xD362,
                tileset: 0xD367,
                party_count: 0xD163,
                sprite_data1: 0xC100,
                sprite_data2: 0xC200,
                num_warps: 0xD3AE,
                warps: 0xD3AF,
                player_name: 0xD158,
                rival_name: 0xD34A,
                moving_direction: 0xD528,
            },
            Revision::Yellow => Self {
                font_loaded: 0xCFC3,
                walk_counter: 0xCFC4,
                in_battle: 0xD056,
                cur_map: 0xD35D,
                y: 0xD360,
                x: 0xD361,
                tileset: 0xD366,
                party_count: 0xD162,
                sprite_data1: 0xC100,
                sprite_data2: 0xC200,
                num_warps: 0xD3AD,
                warps: 0xD3AE,
                player_name: 0xD157,
                rival_name: 0xD349,
                moving_direction: 0xD527,
            },
        }
    }
}

/// A map and a square on it.
type Square = (u8, u8, u8);

/// Oak's lab, and the three Poké Balls on its table (pokered's
/// `data/maps/objects/OaksLab.asm`).
const OAKS_LAB: u8 = 0x28;
const STARTER_BALLS: [(u8, u8); 3] = [(6, 3), (7, 3), (8, 3)];

/// Save states kept for rewinding, one per this many frames of play.
const SNAPSHOTS: usize = 4;
const SNAPSHOT_EVERY: u64 = 60 * 60 * 2;
/// Frames on one square (battles and text boxes included) that count as
/// stuck: longer than any battle the first move cannot finish.
const STILL_LIMIT: u64 = 60 * 60 * 3;

/// Frames a held direction gets before the bot looks whether it moved: a
/// step is 16 frames, a turn on the spot 8.
const STEP: u32 = 18;
/// A text box open this long is a menu the bot is going round in.
const STUCK_TEXT: u32 = 60 * 20;
/// Squares remembered before the memory starts over.
const MAX_SEEN: usize = 1 << 16;

pub struct Bot {
    rev: Revision,
    ram: Ram,
    rng: u32,
    seen: HashMap<Square, u16>,
    /// Times each direction from each square of this map went nowhere.
    walls: HashMap<(u8, u8, u8), u16>,
    /// The direction held and the frames left on it, and where it started.
    walking: Option<(u8, u32, Square)>,
    text_frames: u32,
    steps: u32,
    /// Where it is heading, the direction to press on arrival (through a
    /// door, over a map edge), and the decisions it has left to get there.
    goal: Option<(Square, u8, u32)>,
    /// Press A at the next decision: the goal was something to talk to.
    talk: bool,
    last_here: Option<Square>,
    still: u64,
    snapshots: Vec<Vec<u8>>,
    rewinds: u32,
    last_dir: u8,
    kanto: Kanto,
    /// The current map's walkable squares, row-major, and its size in
    /// squares and joined edges.
    grid_map: Option<u8>,
    walkable: Vec<bool>,
    size: (usize, usize),
    links: u8,
    /// BFS scratch: the first step taken toward each square, and squares
    /// people stand on.
    first: Vec<u8>,
    blocked: Vec<bool>,
    queue: Vec<(usize, usize)>,
}

impl Bot {
    pub fn new(rev: Revision, seed: u32) -> Self {
        Self {
            rev,
            ram: Ram::of(rev),
            rng: seed | 1,
            seen: HashMap::new(),
            walls: HashMap::new(),
            walking: None,
            text_frames: 0,
            steps: 0,
            goal: None,
            talk: false,
            last_here: None,
            still: 0,
            snapshots: Vec::new(),
            rewinds: 0,
            last_dir: DOWN,
            kanto: Kanto::new(rev),
            grid_map: None,
            walkable: Vec::new(),
            size: (0, 0),
            links: 0,
            first: Vec::new(),
            blocked: Vec::new(),
            queue: Vec::new(),
        }
    }

    pub fn revision(&self) -> Revision {
        self.rev
    }

    pub fn buttons(&mut self, gb: &mut GameBoy, frame: u64) -> u8 {
        self.rewind_if_stuck(gb, frame);
        let r = self.ram;
        // Title screen and Oak's speech, until both names are chosen. Down
        // and A picks the first preset name; if the keyboard opened
        // instead, Start jumps to its END and A takes it.
        if naming_screen(gb) {
            self.walking = None;
            return press(
                frame,
                16,
                if (frame / 16).is_multiple_of(2) {
                    START
                } else {
                    A
                },
            );
        }
        let mut unnamed = |a: u16| matches!(gb.peek(a), 0 | 0x50);
        if unnamed(r.player_name) || unnamed(r.rival_name) {
            self.walking = None;
            let key = [DOWN, A, START, A][(frame / 16 % 4) as usize];
            return press(frame, 16, key);
        }
        if gb.peek(r.in_battle) != 0 {
            self.walking = None;
            self.text_frames = 0;
            // FIGHT and the first move; every fourth press backs out of
            // whatever prompt A alone would loop in (switch, nickname,
            // forgetting a move).
            let n = frame / 10;
            return press(frame, 10, if n % 4 == 3 { B } else { A });
        }
        if gb.peek(r.font_loaded) & 1 != 0 {
            self.walking = None;
            self.text_frames += 1;
            let key = if self.text_frames > STUCK_TEXT / 2 {
                // Round some menu: Start and B get out of most.
                if (frame / 12).is_multiple_of(3) {
                    START
                } else {
                    B
                }
            } else if self.text_frames > 60 * 10 && (frame / 12) % 2 == 1 {
                B
            } else {
                A
            };
            return press(frame, 12, key);
        }
        self.text_frames = 0;
        self.walk(gb, frame)
    }

    /// Some scripted moment it does not understand can hold it in place
    /// for good. Every few minutes of progress it keeps a save state; held
    /// still too long, it goes back to an older one and plays on with
    /// different luck.
    fn rewind_if_stuck(&mut self, gb: &mut GameBoy, frame: u64) {
        // The intro holds still for minutes, and before it ends there is
        // nothing worth going back to.
        if gb.peek(self.ram.party_count) == 0 {
            return;
        }
        let here = self.here(gb);
        if self.last_here == Some(here) {
            self.still += 1;
        } else {
            self.last_here = Some(here);
            self.still = 0;
        }
        if frame.is_multiple_of(SNAPSHOT_EVERY) && self.still < STILL_LIMIT / 4 {
            let mut state = Vec::new();
            if gb.save_state(&mut state).is_ok() {
                if self.snapshots.len() == SNAPSHOTS {
                    self.snapshots.remove(0);
                }
                self.snapshots.push(state);
            }
        }
        if self.still < STILL_LIMIT {
            return;
        }
        self.still = 0;
        let Some(state) = self.snapshots.first() else {
            return;
        };
        if gb.load_state(state.as_slice()).is_ok() {
            self.snapshots.truncate(1);
            self.rewinds += 1;
            self.rng = self
                .rng
                .wrapping_mul(0x9E37_79B9)
                .wrapping_add(self.rewinds);
            self.walking = None;
            self.goal = None;
            self.walls.clear();
        }
    }

    fn here(&self, gb: &mut GameBoy) -> Square {
        let r = self.ram;
        (gb.peek(r.cur_map), gb.peek(r.x), gb.peek(r.y))
    }

    /// Read the current map's walkable squares from the cartridge.
    fn learn_map(&mut self, gb: &mut GameBoy, map: u8) {
        self.grid_map = Some(map);
        let mut placed = Vec::new();
        self.kanto.place(gb.rom(), map, 0, &mut placed);
        let Some(p) = placed.first() else {
            self.size = (0, 0);
            return;
        };
        let (w, h) = ((p.w * 2) as usize, (p.h * 2) as usize);
        self.size = (w, h);
        self.links = self
            .kanto
            .header(gb.rom(), map)
            .map_or(0, |h| h.links.iter().fold(0, |a, l| a | l.0));
        self.walkable.clear();
        for y in 0..h {
            for x in 0..w {
                let ok = self.kanto.walkable(gb.rom(), p, x as i32, y as i32);
                self.walkable.push(ok);
            }
        }
    }

    /// Doors, stairs and cave mouths, and the walkable squares along an
    /// edge another map joins on, each with the direction to press there.
    fn exits(&self, gb: &mut GameBoy, map: u8) -> Vec<(Square, u8)> {
        let r = self.ram;
        let mut out = Vec::new();
        // No Pokémon yet: Oak's lab won't let it leave until it takes a
        // ball from the table, from below.
        if map == OAKS_LAB && gb.peek(r.party_count) == 0 {
            return STARTER_BALLS
                .iter()
                .map(|&(x, y)| ((map, x, y + 1), UP))
                .collect();
        }
        for i in 0..gb.peek(r.num_warps).min(32) {
            let e = r.warps + 4 * u16::from(i);
            out.push(((map, gb.peek(e + 1), gb.peek(e)), self.last_dir));
        }
        let (w, h) = self.size;
        let ok = |x: usize, y: usize| self.walkable.get(y * w + x).copied().unwrap_or(false);
        for (flag, dir) in [
            (kanto::NORTH, UP),
            (kanto::SOUTH, DOWN),
            (kanto::WEST, LEFT),
            (kanto::EAST, RIGHT),
        ] {
            if self.links & flag == 0 {
                continue;
            }
            let squares: Vec<(usize, usize)> = match dir {
                UP => (0..w).map(|x| (x, 0)).collect(),
                DOWN => (0..w).map(|x| (x, h - 1)).collect(),
                LEFT => (0..h).map(|y| (0, y)).collect(),
                _ => (0..h).map(|y| (w - 1, y)).collect(),
            };
            for (x, y) in squares.into_iter().filter(|&(x, y)| ok(x, y)).step_by(3) {
                out.push(((map, x as u8, y as u8), dir));
            }
        }
        out
    }

    /// Breadth-first over walkable squares from `from`, people in the way
    /// counted as walls: fills `first` with the first step toward each
    /// reachable square (0 for unreached).
    fn flood(&mut self, gb: &mut GameBoy, from: (usize, usize)) {
        let (w, h) = self.size;
        self.first.clear();
        self.first.resize(w * h, 0);
        self.blocked.clear();
        self.blocked.resize(w * h, false);
        let r = self.ram;
        for n in 1..16u16 {
            let base = r.sprite_data2 + n * 16;
            let (y, x) = (gb.peek(base + 4), gb.peek(base + 5));
            if gb.peek(r.sprite_data1 + n * 16) != 0 && x >= 4 && y >= 4 {
                let (x, y) = (usize::from(x - 4), usize::from(y - 4));
                if x < w && y < h {
                    self.blocked[y * w + x] = true;
                }
            }
        }
        self.queue.clear();
        self.queue.push(from);
        let mut head = 0;
        while head < self.queue.len() {
            let (x, y) = self.queue[head];
            head += 1;
            for (dir, nx, ny) in [
                (UP, x, y.wrapping_sub(1)),
                (DOWN, x, y + 1),
                (LEFT, x.wrapping_sub(1), y),
                (RIGHT, x + 1, y),
            ] {
                if nx >= w || ny >= h {
                    continue;
                }
                let i = ny * w + nx;
                if self.first[i] != 0 || !self.walkable[i] || self.blocked[i] || (nx, ny) == from {
                    continue;
                }
                let wall = self
                    .walls
                    .get(&(x as u8, y as u8, dir))
                    .copied()
                    .unwrap_or(0);
                if wall > 2 {
                    continue;
                }
                self.first[i] = if (x, y) == from {
                    dir
                } else {
                    self.first[y * w + x]
                };
                self.queue.push((nx, ny));
            }
        }
    }

    fn walk(&mut self, gb: &mut GameBoy, frame: u64) -> u8 {
        let here = self.here(gb);
        if let Some((dir, left, from)) = self.walking {
            if left > 0 {
                self.walking = Some((dir, left - 1, from));
                return dir;
            }
            self.walking = None;
            if here == from {
                *self.walls.entry((from.1, from.2, dir)).or_insert(0) += 1;
                if let Some(g) = &mut self.goal {
                    g.2 = g.2.saturating_sub(10);
                }
            }
        }
        if self.seen.len() > MAX_SEEN {
            self.seen.clear();
        }
        let m = here.0;
        if self.grid_map != Some(m) {
            self.walls.clear();
            self.goal = None;
            self.learn_map(gb, m);
        }
        *self.seen.entry(here).or_insert(0) += 1;
        self.steps += 1;
        // Every so often face something and press A: signs, people, items.
        if std::mem::take(&mut self.talk)
            || (self.steps.is_multiple_of(9) && frame.is_multiple_of(2))
        {
            return A;
        }
        // Not moving for seconds with no text box the bot knows of: some
        // speech or prompt is waiting on A.
        if self.still > 60 * 5 && self.steps.is_multiple_of(2) {
            return A;
        }
        let dir = self.route(gb, here).unwrap_or_else(|| self.wander(here));
        self.last_dir = dir;
        self.walking = Some((dir, STEP, here));
        dir
    }

    /// The next step toward the least-visited exit it can reach.
    fn route(&mut self, gb: &mut GameBoy, here: Square) -> Option<u8> {
        let (m, x, y) = here;
        let (w, h) = self.size;
        if usize::from(x) >= w || usize::from(y) >= h {
            return None;
        }
        if let Some((g, dir, _)) = self.goal {
            if g == here {
                self.goal = None;
                self.talk = m == OAKS_LAB;
                return Some(dir);
            }
        }
        self.flood(gb, (usize::from(x), usize::from(y)));
        let reach = |s: &Square| {
            self.first
                .get(usize::from(s.2) * w + usize::from(s.1))
                .copied()
                .unwrap_or(0)
        };
        // A goal it gave up on counts as visited, or it would pick it again.
        if let Some((g, _, 0)) = self.goal {
            *self.seen.entry(g).or_insert(0) += 4;
        }
        let keep = self
            .goal
            .filter(|&(g, _, left)| left > 0 && g.0 == m && reach(&g) != 0);
        let goal = if let Some((g, d, left)) = keep {
            (g, d, left - 1)
        } else {
            let exits = self.exits(gb, m);
            let pick = exits
                .into_iter()
                .filter(|(s, _)| reach(s) != 0)
                .min_by_key(|(s, _)| {
                    let v = u32::from(self.seen.get(s).copied().unwrap_or(0));
                    v * 64 + next_rand(&mut self.rng) % 48
                })?;
            (pick.0, pick.1, 150)
        };
        self.goal = Some(goal);
        Some(reach(&goal.0))
    }

    /// No exit in reach: toward the least-trodden neighbour.
    fn wander(&mut self, here: Square) -> u8 {
        let (m, x, y) = here;
        let mut best = (u32::MAX, UP);
        for (dir, to) in [
            (UP, (m, x, y.wrapping_sub(1))),
            (DOWN, (m, x, y.wrapping_add(1))),
            (LEFT, (m, x.wrapping_sub(1), y)),
            (RIGHT, (m, x.wrapping_add(1), y)),
        ] {
            let wall = u32::from(self.walls.get(&(x, y, dir)).copied().unwrap_or(0)) * 40;
            let visits = u32::from(self.seen.get(&to).copied().unwrap_or(0));
            let score = visits * 3 + wall + next_rand(&mut self.rng) % 6;
            if score < best.0 {
                best = (score, dir);
            }
        }
        best.1
    }
}

/// The naming keyboard is up: its last line reads `UPPER CASE` or
/// `lower case`, in the game's own character codes, in `wTileMap` (the same
/// address in all three revisions).
fn naming_screen(gb: &mut GameBoy) -> bool {
    const TILE_MAP: u16 = 0xC3A0;
    const AT: u16 = TILE_MAP + 15 * 20 + 8;
    let word = [0, 1, 2, 3].map(|i| gb.peek(AT + i));
    word == [0x82, 0x80, 0x92, 0x84] || word == [0xA2, 0xA0, 0xB2, 0xA4]
}

/// `key` for the first two frames of every `every`, so each press is a
/// fresh edge the game sees.
fn press(frame: u64, every: u64, key: u8) -> u8 {
    if frame % every < 2 {
        key
    } else {
        0
    }
}
