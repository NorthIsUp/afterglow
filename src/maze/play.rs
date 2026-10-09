//! The chase: an eater, four ghosts with the arcade's personalities, and the
//! autopilot that plays the eater.
//!
//! Positions are in `SUB`ths of a tile, centre of the actor, so a tile centre
//! is `t * SUB + SUB / 2` and the renderer's half-cell quadrants are two
//! units each. Ghosts choose at tile centres as the arcade's do: never
//! reversing, the open way nearest their target by straight-line distance,
//! ties broken up, left, down, right.
//!
//! The autopilot floods the maze from the ghosts that can hurt it and from
//! itself, and only plans through tiles it reaches before any ghost does. It
//! hunts blue ghosts it can catch in time, saves power pellets for when a
//! ghost is close, otherwise takes the nearest pellet, and with nothing safe
//! in reach runs to wherever its lead over the ghosts is largest.

use super::gen::{Maze, PELLET, POWER, TUNNEL};
use crate::next_rand;

pub const SUB: i32 = 12;
const MID: i32 = SUB / 2;
const FAR: u16 = u16::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    Up,
    Left,
    Down,
    Right,
}

/// The arcade's tie-break order.
const DIRS: [Dir; 4] = [Dir::Up, Dir::Left, Dir::Down, Dir::Right];

impl Dir {
    pub fn dx(self) -> i32 {
        match self {
            Dir::Left => -1,
            Dir::Right => 1,
            _ => 0,
        }
    }

    pub fn dy(self) -> i32 {
        match self {
            Dir::Up => -1,
            Dir::Down => 1,
            _ => 0,
        }
    }

    fn rev(self) -> Self {
        match self {
            Dir::Up => Dir::Down,
            Dir::Down => Dir::Up,
            Dir::Left => Dir::Right,
            Dir::Right => Dir::Left,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Actor {
    pub x: i32,
    pub y: i32,
    pub dir: Dir,
    acc: u32,
}

impl Actor {
    fn at(tx: usize, ty: usize, dir: Dir) -> Self {
        Self {
            x: tx as i32 * SUB + MID,
            y: ty as i32 * SUB + MID,
            dir,
            acc: 0,
        }
    }

    pub fn tile(&self) -> (usize, usize) {
        ((self.x / SUB) as usize, (self.y / SUB) as usize)
    }

    fn centred(&self) -> bool {
        self.x % SUB == MID && self.y % SUB == MID
    }
}

/// What one way out of a junction leads to; see `region`.
struct Region {
    size: usize,
    pellet: u16,
    power: u16,
    hunt: u16,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GState {
    House,
    Leave,
    Walk,
    Eyes,
    Enter,
}

#[derive(Clone, Copy, Debug)]
pub struct Ghost {
    pub a: Actor,
    pub state: GState,
    pub fright: bool,
    home: (i32, i32),
    corner: (i32, i32),
    release: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Ready(u32),
    Play,
    /// A ghost was just eaten: everything holds while its points show.
    Freeze(u32),
    Dying(u32),
    Clear(u32),
    Over(u32),
}

/// The tile one step from `(x, y)`, wrapping only sideways.
#[inline]
fn step_in(m: &Maze, x: usize, y: usize, d: Dir) -> Option<(usize, usize)> {
    let w = m.w as i32;
    let ny = y as i32 + d.dy();
    if ny < 0 || ny >= m.h as i32 {
        return None;
    }
    Some((((x as i32 + d.dx() + w) % w) as usize, ny as usize))
}

/// Walking distance from `start` to every tile, into `dist`.
fn distances(m: &Maze, start: usize, dist: &mut [u16], queue: &mut Vec<u32>) {
    dist.fill(FAR);
    queue.clear();
    dist[start] = 0;
    queue.push(start as u32);
    let mut head = 0;
    while head < queue.len() {
        let i = queue[head] as usize;
        head += 1;
        for d in DIRS {
            if let Some((x, y)) = step_in(m, i % m.w, i / m.w, d) {
                let j = y * m.w + x;
                if dist[j] == FAR && m.walkable(x, y) {
                    dist[j] = dist[i] + 1;
                    queue.push(j as u32);
                }
            }
        }
    }
}

/// The open tile nearest `(tx, ty)`, which may be off the maze: a ghost's
/// scatter corner, or a target projected past the wall.
fn nearest_open(m: &Maze, tx: i32, ty: i32) -> usize {
    let (cx, cy) = (tx.clamp(0, m.w as i32 - 1), ty.clamp(0, m.h as i32 - 1));
    for r in 0..m.w.max(m.h) as i32 {
        for dy in -r..=r {
            for dx in -r..=r {
                let (x, y) = (cx + dx, cy + dy);
                if dx.abs().max(dy.abs()) == r
                    && (0..m.w as i32).contains(&x)
                    && (0..m.h as i32).contains(&y)
                    && m.walkable(x as usize, y as usize)
                {
                    return y as usize * m.w + x as usize;
                }
            }
        }
    }
    0
}

/// Seconds of blue time by level, the arcade's table.
const FRIGHT_SECS: [u32; 19] = [6, 5, 4, 3, 2, 5, 2, 2, 1, 5, 2, 1, 1, 3, 1, 1, 0, 1, 0];

pub struct Game {
    pub maze: Maze,
    pub pac: Actor,
    pub ghosts: [Ghost; 4],
    pub phase: Phase,
    pub level: u32,
    pub score: u64,
    pub lives: u32,
    pub fright: u32,
    chain: u32,
    mode_t: u32,
    chase: bool,
    pub left: usize,
    pub chomp: u32,
    /// Frames since the eater last ate, which wears its caution down.
    hungry: u32,
    /// Ghost distance past which a tile is not threatened, whoever is
    /// nearer; shorter as hunger wears caution down.
    reach: u16,
    /// Points and where to show them, while frozen.
    pub popup: (i32, i32, u64),
    pub cleared: u32,
    pub deaths: u32,
    pub games: u32,
    pub last_levels: u32,
    pub eaten: u32,
    /// Set when the maze changed, for the renderer's static layer.
    pub fresh: bool,
    pub fps: u32,
    rng: u32,
    cut: usize,
    speed10: u32,
    max_lives: u32,
    bonus: u64,
    next_bonus: u64,
    gd: Vec<u16>,
    pd: Vec<u16>,
    exit_d: Vec<u16>,
    td: Vec<u16>,
    queue: Vec<u32>,
}

impl Game {
    pub fn new(
        maze: Maze,
        seed: u32,
        fps: u32,
        speed10: u32,
        cut: usize,
        lives: u32,
        bonus: u64,
    ) -> Self {
        let n = maze.w * maze.h;
        let blank = Ghost {
            a: Actor::at(0, 0, Dir::Left),
            state: GState::House,
            fright: false,
            home: (0, 0),
            corner: (0, 0),
            release: 0,
        };
        let mut g = Self {
            maze,
            pac: Actor::at(0, 0, Dir::Left),
            ghosts: [blank; 4],
            phase: Phase::Ready(0),
            level: 1,
            score: 0,
            lives,
            fright: 0,
            chain: 0,
            mode_t: 0,
            chase: false,
            left: 0,
            chomp: 0,
            hungry: 0,
            reach: 12,
            popup: (0, 0, 0),
            cleared: 0,
            deaths: 0,
            games: 0,
            last_levels: 0,
            eaten: 0,
            fresh: true,
            fps: fps.max(1),
            rng: seed.max(1),
            cut,
            speed10,
            max_lives: lives,
            bonus,
            next_bonus: bonus,
            gd: vec![FAR; n],
            pd: vec![FAR; n],
            exit_d: vec![FAR; n],
            td: vec![FAR; n],
            queue: Vec::with_capacity(n),
        };
        g.new_maze();
        g
    }

    fn new_maze(&mut self) {
        let seed = next_rand(&mut self.rng) | 1;
        self.maze.generate(seed, self.cut);
        self.left = self.maze.pellets + self.maze.npower;
        let (ex, ey) = self.maze.exit;
        distances(
            &self.maze,
            ex + ey * self.maze.w,
            &mut self.exit_d,
            &mut self.queue,
        );
        self.fresh = true;
        self.reset_actors();
    }

    fn reset_actors(&mut self) {
        let m = &self.maze;
        let (ex, ey) = m.exit;
        let (_, y0, _, y1) = m.house;
        let hy = ((y0 + y1) as i32 * SUB) / 2 + MID;
        let hx = ex as i32 * SUB + MID;
        let (w, h) = (m.w as i32, m.h as i32);
        let corners = [(w - 3, -3), (2, -3), (w - 1, h + 1), (0, h + 1)];
        let homes = [(hx, hy), (hx, hy), (hx - 2 * SUB, hy), (hx + 2 * SUB, hy)];
        for (k, g) in self.ghosts.iter_mut().enumerate() {
            *g = Ghost {
                a: Actor {
                    x: homes[k].0,
                    y: homes[k].1,
                    dir: if k == 2 { Dir::Up } else { Dir::Down },
                    acc: 0,
                },
                state: GState::House,
                fright: false,
                home: homes[k],
                corner: corners[k],
                release: [0, 1, 4, 7][k] * self.fps,
            };
        }
        self.ghosts[0].a = Actor::at(ex, ey, Dir::Left);
        self.ghosts[0].state = GState::Walk;
        let (sx, sy) = m.start;
        self.pac = Actor::at(sx, sy, Dir::Left);
        (self.fright, self.chain, self.mode_t, self.chase) = (0, 0, 0, false);
        self.phase = Phase::Ready(0);
    }

    fn fright_frames(&self) -> u32 {
        FRIGHT_SECS[(self.level as usize - 1).min(FRIGHT_SECS.len() - 1)] * self.fps
    }

    /// 1/256 sub-units per frame at `pct` of full speed.
    fn speed(&self, pct: u32) -> u32 {
        pct * self.speed10 * SUB as u32 * 256 / (100 * 10 * self.fps)
    }

    fn tier(&self) -> usize {
        match self.level {
            1 => 0,
            2..=4 => 1,
            _ => 2,
        }
    }

    /// The tile one step from `(x, y)`, wrapping only sideways.
    #[inline]
    fn step_tile(&self, x: usize, y: usize, d: Dir) -> Option<(usize, usize)> {
        step_in(&self.maze, x, y, d)
    }

    #[inline]
    fn can(&self, x: usize, y: usize, d: Dir) -> bool {
        self.step_tile(x, y, d)
            .is_some_and(|(a, b)| self.maze.walkable(a, b))
    }

    /// Move one unit along `a.dir`, wrapping through the tunnel.
    fn nudge(a: &mut Actor, wsub: i32) {
        a.x = (a.x + a.dir.dx() + wsub) % wsub;
        a.y += a.dir.dy();
    }

    /// Whether tile `j`, reached in `d` steps, is beyond every ghost's
    /// reach by then. Only ghosts near the tile count: one across the maze
    /// is not heading for it.
    #[inline]
    fn clear_of(&self, j: usize, d: u16) -> bool {
        self.gd[j] > d + d / 2 || self.gd[j] >= self.reach
    }

    /// The tiles the eater would reach before any ghost if it stepped to
    /// `n` now, never passing back through `here`: their count and the
    /// distance to the nearest pellet, power pellet and catchable blue ghost.
    fn region(&mut self, here: usize, n: usize, margin: u16) -> Region {
        let w = self.maze.w;
        let mut r = Region {
            size: 0,
            pellet: FAR,
            power: FAR,
            hunt: FAR,
        };
        if !self.clear_of(n, 1 + margin) {
            return r;
        }
        self.pd.fill(FAR);
        self.queue.clear();
        self.pd[here] = 0;
        self.pd[n] = 1;
        self.queue.push(n as u32);
        let mut head = 0;
        while head < self.queue.len() {
            let i = self.queue[head] as usize;
            head += 1;
            let d = self.pd[i];
            r.size += 1;
            let t = self.maze.t[i];
            if t & PELLET != 0 {
                r.pellet = r.pellet.min(d);
            }
            if t & POWER != 0 {
                r.power = r.power.min(d);
            }
            for dir in DIRS {
                let Some((x, y)) = self.step_tile(i % w, i / w, dir) else {
                    continue;
                };
                let j = y * w + x;
                if self.pd[j] == FAR && self.maze.walkable(x, y) && self.clear_of(j, d + 1 + margin)
                {
                    self.pd[j] = d + 1;
                    self.queue.push(j as u32);
                }
            }
        }
        if self.fright > 0 {
            let frames_per_tile = (SUB as u32 * 256) / self.speed(90).max(1);
            for g in &self.ghosts {
                let (gx, gy) = g.a.tile();
                let d = self.pd[gy * w + gx];
                let reach = u32::from(d) * frames_per_tile + self.fps / 2;
                if g.fright && g.state == GState::Walk && d != FAR && reach < self.fright {
                    r.hunt = r.hunt.min(d);
                }
            }
        }
        r
    }

    /// Distance from the nearest ghost that can hurt, into `gd`.
    fn flood_ghosts(&mut self) {
        let w = self.maze.w;
        self.gd.fill(FAR);
        self.queue.clear();
        // Blue time about to run out counts as no blue time at all.
        let ending = self.fright < self.fps;
        for g in &self.ghosts {
            if g.state != GState::Walk || (g.fright && !ending) {
                continue;
            }
            // A ghost cannot turn back, so it starts from its tile and the
            // ways ahead of it; what is behind it is reached the long way.
            let (x, y) = g.a.tile();
            let j = y * w + x;
            if !self.maze.walkable(x, y) {
                continue;
            }
            self.gd[j] = 0;
            for d in DIRS {
                if d == g.a.dir.rev() {
                    continue;
                }
                if let Some((nx, ny)) = self.step_tile(x, y, d) {
                    let k = ny * w + nx;
                    if self.maze.walkable(nx, ny) && self.gd[k] > 1 {
                        self.gd[k] = 1;
                        self.queue.push(k as u32);
                    }
                }
            }
        }
        let mut head = 0;
        while head < self.queue.len() {
            let i = self.queue[head] as usize;
            head += 1;
            let d = self.gd[i] + 1;
            for dir in DIRS {
                if let Some((x, y)) = self.step_tile(i % w, i / w, dir) {
                    let j = y * w + x;
                    if self.gd[j] == FAR && self.maze.walkable(x, y) {
                        self.gd[j] = d;
                        self.queue.push(j as u32);
                    }
                }
            }
        }
    }

    /// The autopilot's choice at a tile centre: each way out is scored by
    /// the territory it leads into. Enough room, and it hunts, saves power
    /// pellets for a ghost on its tail, or takes the nearest pellet; a way
    /// into a pincer has almost none, so with every way cramped it takes the
    /// roomiest, or a power pellet if one is in reach.
    fn pilot(&mut self) -> Dir {
        let w = self.maze.w;
        let (px, py) = self.pac.tile();
        let here = py * w + px;
        self.flood_ghosts();
        let starving = self.hungry > 12 * self.fps;
        let (room, margin, reach) = if starving { (6, 0, 4) } else { (20, 0, 8) };
        self.reach = reach;
        let threat = self.gd[here] < 8;
        let room = if self.gd[here] < 2 * reach { room } else { 0 };
        let mut best = (i64::MIN, self.pac.dir);
        for d in DIRS {
            let Some((x, y)) = self.step_tile(px, py, d) else {
                continue;
            };
            if !self.maze.walkable(x, y) {
                continue;
            }
            let r = self.region(here, y * w + x, margin);
            let near = |v: u16| i64::from(FAR - v);
            let score = if r.hunt != FAR {
                4_000_000 + near(r.hunt)
            } else if r.power != FAR && (threat || r.size < room) {
                3_000_000 + near(r.power)
            } else if r.size >= room && r.pellet != FAR {
                2_000_000 + near(r.pellet)
            } else if r.size >= room && r.power != FAR {
                1_000_000 + near(r.power)
            } else {
                r.size as i64
            };
            if score > best.0 {
                best = (score, d);
            }
        }
        best.1
    }

    /// Between tiles, turn back if a ghost is about to meet the eater head on.
    fn flinch(&mut self) {
        let (px, py) = self.pac.tile();
        let w = self.maze.w;
        let near = self.ghosts.iter().any(|g| {
            let (gx, gy) = g.a.tile();
            let dx = gx.abs_diff(px).min(w - gx.abs_diff(px));
            g.state == GState::Walk && !g.fright && dx + gy.abs_diff(py) <= 3
        });
        if !near {
            return;
        }
        self.flood_ghosts();
        let d = self.pac.dir;
        let along = (self.pac.x - (px as i32 * SUB + MID)) * d.dx()
            + (self.pac.y - (py as i32 * SUB + MID)) * d.dy();
        let (ahead, behind) = if along > 0 {
            (self.step_tile(px, py, d), Some((px, py)))
        } else {
            (Some((px, py)), self.step_tile(px, py, d.rev()))
        };
        let gd = |t: Option<(usize, usize)>| t.map_or(FAR, |(x, y)| self.gd[y * w + x]);
        if gd(ahead) <= 1 && gd(behind) > gd(ahead) {
            self.pac.dir = d.rev();
        }
    }

    fn eat(&mut self) {
        let (x, y) = self.pac.tile();
        let i = y * self.maze.w + x;
        let t = self.maze.t[i];
        if t & (PELLET | POWER) != 0 {
            self.hungry = 0;
        }
        if t & PELLET != 0 {
            self.maze.t[i] &= !PELLET;
            self.score += 10;
            self.left -= 1;
        } else if t & POWER != 0 {
            self.maze.t[i] &= !POWER;
            self.score += 50;
            self.left -= 1;
            self.fright = self.fright_frames();
            self.chain = 0;
            for g in &mut self.ghosts {
                if g.state == GState::Walk {
                    g.a.dir = g.a.dir.rev();
                }
                g.fright = self.fright > 0 && g.state != GState::Eyes && g.state != GState::Enter;
            }
        }
    }

    fn move_pac(&mut self) {
        let pct = match (self.fright > 0, self.tier()) {
            (false, 0) => 80,
            (false, 1) | (true, 0) => 90,
            (true, 1) => 95,
            _ => 100,
        };
        let wsub = self.maze.w as i32 * SUB;
        self.pac.acc += self.speed(pct);
        let mut moved = false;
        if !self.pac.centred() {
            self.flinch();
        }
        while self.pac.acc >= 256 {
            self.pac.acc -= 256;
            if self.pac.centred() {
                self.eat();
                let (x, y) = self.pac.tile();
                let d = self.pilot();
                if self.can(x, y, d) {
                    self.pac.dir = d;
                } else if !self.can(x, y, self.pac.dir) {
                    self.pac.acc = 0;
                    break;
                }
            }
            Self::nudge(&mut self.pac, wsub);
            moved = true;
        }
        if moved {
            self.chomp += 1;
        }
    }

    fn target(&self, k: usize) -> (i32, i32) {
        let g = &self.ghosts[k];
        if !self.chase {
            return g.corner;
        }
        let (px, py) = self.pac.tile();
        let (px, py) = (px as i32, py as i32);
        let (dx, dy) = (self.pac.dir.dx(), self.pac.dir.dy());
        match k {
            0 => (px, py),
            1 => (px + 4 * dx, py + 4 * dy),
            2 => {
                let (bx, by) = self.ghosts[0].a.tile();
                let (ax, ay) = (px + 2 * dx, py + 2 * dy);
                (2 * ax - bx as i32, 2 * ay - by as i32)
            }
            _ => {
                let (cx, cy) = g.a.tile();
                let (ex, ey) = (cx as i32 - px, cy as i32 - py);
                if ex * ex + ey * ey > 64 {
                    (px, py)
                } else {
                    g.corner
                }
            }
        }
    }

    /// A walking ghost at a tile centre picks its next direction.
    fn steer(&mut self, k: usize) {
        let g = self.ghosts[k];
        let (x, y) = g.a.tile();
        let eyes = g.state == GState::Eyes;
        let (tx, ty) = self.target(k);
        let walk = !eyes && !g.fright && next_rand(&mut self.rng).is_multiple_of(8);
        if walk {
            let t = nearest_open(&self.maze, tx, ty);
            distances(&self.maze, t, &mut self.td, &mut self.queue);
        }
        let mut best: Option<(u32, Dir)> = None;
        let mut options = 0u32;
        for d in DIRS {
            if d == g.a.dir.rev() || !self.can(x, y, d) {
                continue;
            }
            let Some((nx, ny)) = self.step_tile(x, y, d) else {
                continue;
            };
            options += 1;
            let j = ny * self.maze.w + nx;
            let cost = if eyes {
                u32::from(self.exit_d[j])
            } else if g.fright {
                next_rand(&mut self.rng) % 64
            } else if walk {
                u32::from(self.td[j])
            } else {
                let (ex, ey) = (nx as i32 - tx, ny as i32 - ty);
                (ex * ex + ey * ey) as u32
            };
            if best.is_none_or(|(c, _)| cost < c) {
                best = Some((cost, d));
            }
        }
        self.ghosts[k].a.dir = match best {
            Some((_, d)) if options > 0 => d,
            _ => g.a.dir.rev(),
        };
    }

    fn move_ghost(&mut self, k: usize) {
        let t = self.tier();
        let g = self.ghosts[k];
        let (tx, ty) = g.a.tile();
        let tunnel = self.maze.at(tx, ty) & TUNNEL != 0;
        let pct = match g.state {
            GState::Eyes | GState::Enter => 150,
            GState::House | GState::Leave => 50,
            _ if tunnel => [40, 45, 50][t],
            _ if g.fright => [50, 55, 60][t],
            GState::Walk => [75, 85, 95][t],
        };
        let wsub = self.maze.w as i32 * SUB;
        let (ex, ey) = self.maze.exit;
        let (ex, ey) = (ex as i32 * SUB + MID, ey as i32 * SUB + MID);
        let (_, hy0, _, hy1) = self.maze.house;
        self.ghosts[k].a.acc += self.speed(pct);
        while self.ghosts[k].a.acc >= 256 {
            self.ghosts[k].a.acc -= 256;
            let state = self.ghosts[k].state;
            if matches!(state, GState::Walk | GState::Eyes) {
                let a = self.ghosts[k].a;
                if a.centred() {
                    if state == GState::Eyes && (a.x, a.y) == (ex, ey) {
                        self.ghosts[k].state = GState::Enter;
                        continue;
                    }
                    self.steer(k);
                }
                Self::nudge(&mut self.ghosts[k].a, wsub);
                continue;
            }
            let g = &mut self.ghosts[k];
            match state {
                GState::House if g.release > 0 => {
                    let (top, bot) = (hy0 as i32 * SUB + MID, hy1 as i32 * SUB + MID);
                    if g.a.y <= top.max(g.home.1 - MID) {
                        g.a.dir = Dir::Down;
                    } else if g.a.y >= bot.min(g.home.1 + MID) {
                        g.a.dir = Dir::Up;
                    }
                    g.a.y += g.a.dir.dy();
                }
                GState::Leave if g.a.x != ex => g.a.x += (ex - g.a.x).signum(),
                GState::Leave if g.a.y != ey => g.a.y -= 1,
                GState::Leave => {
                    g.state = GState::Walk;
                    g.a.dir = Dir::Left;
                }
                GState::Enter if g.a.y != g.home.1 => g.a.y += (g.home.1 - g.a.y).signum(),
                GState::Enter if g.a.x != g.home.0 => g.a.x += (g.home.0 - g.a.x).signum(),
                GState::House | GState::Enter | GState::Walk | GState::Eyes => {
                    g.state = GState::Leave;
                }
            }
        }
    }

    fn collide(&mut self) {
        let wsub = self.maze.w as i32 * SUB;
        for k in 0..4 {
            let g = self.ghosts[k];
            if g.state != GState::Walk {
                continue;
            }
            let dx = (g.a.x - self.pac.x).abs();
            let dx = dx.min(wsub - dx);
            if dx + (g.a.y - self.pac.y).abs() >= SUB * 2 / 3 {
                continue;
            }
            if g.fright {
                let pts = 200u64 << self.chain.min(3);
                self.chain += 1;
                self.score += pts;
                self.eaten += 1;
                self.ghosts[k].state = GState::Eyes;
                self.ghosts[k].fright = false;
                self.popup = (g.a.x, g.a.y, pts);
                self.phase = Phase::Freeze(0);
                return;
            }
            self.phase = Phase::Dying(0);
            return;
        }
    }

    fn mode_tick(&mut self) {
        if self.fright > 0 {
            self.fright -= 1;
            if self.fright == 0 {
                for g in &mut self.ghosts {
                    g.fright = false;
                }
            }
            return;
        }
        let secs: &[u32] = if self.level == 1 {
            &[7, 20, 7, 20, 5, 20, 5]
        } else {
            &[5, 20, 5, 20, 5]
        };
        self.mode_t += 1;
        let mut t = self.mode_t;
        let mut chase = true;
        for (i, &s) in secs.iter().enumerate() {
            if t < s * self.fps {
                chase = i % 2 == 1;
                break;
            }
            t -= s * self.fps;
        }
        if chase != self.chase {
            self.chase = chase;
            for g in &mut self.ghosts {
                if g.state == GState::Walk {
                    g.a.dir = g.a.dir.rev();
                }
            }
        }
    }

    fn play(&mut self) {
        self.mode_tick();
        self.hungry += 1;
        for g in &mut self.ghosts {
            g.release = g.release.saturating_sub(1);
        }
        self.move_pac();
        self.collide();
        if self.phase != Phase::Play {
            return;
        }
        for k in 0..4 {
            self.move_ghost(k);
        }
        self.collide();
        if self.bonus > 0 && self.score >= self.next_bonus {
            self.lives = (self.lives + 1).min(9);
            self.next_bonus += self.bonus;
        }
        if self.phase == Phase::Play && self.left == 0 {
            self.phase = Phase::Clear(0);
        }
    }

    fn new_game(&mut self) {
        self.games += 1;
        self.last_levels = self.level - 1;
        (self.level, self.score, self.lives) = (1, 0, self.max_lives);
        self.next_bonus = self.bonus;
        self.new_maze();
    }

    pub fn step(&mut self) {
        let s = self.fps;
        self.phase = match self.phase {
            Phase::Ready(t) if t >= 2 * s => Phase::Play,
            Phase::Ready(t) => Phase::Ready(t + 1),
            Phase::Play => {
                self.play();
                return;
            }
            Phase::Freeze(t) if t >= s / 2 => Phase::Play,
            Phase::Freeze(t) => Phase::Freeze(t + 1),
            Phase::Dying(t) if t >= 5 * s / 2 => {
                self.deaths += 1;
                self.lives -= 1;
                if self.lives == 0 {
                    Phase::Over(0)
                } else {
                    self.reset_actors();
                    return;
                }
            }
            Phase::Dying(t) => Phase::Dying(t + 1),
            Phase::Clear(t) if t >= 2 * s => {
                self.level += 1;
                self.cleared += 1;
                self.new_maze();
                return;
            }
            Phase::Clear(t) => Phase::Clear(t + 1),
            Phase::Over(t) if t >= 3 * s => {
                self.new_game();
                return;
            }
            Phase::Over(t) => Phase::Over(t + 1),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(w: usize, h: usize) -> Game {
        Game::new(Maze::new(w, h), 0xC0FFEE, 30, 95, 45, 3, 10_000)
    }

    /// The four personalities aim where the arcade's do.
    #[test]
    fn each_ghost_aims_by_its_own_rule() {
        let mut g = game(52, 29);
        g.chase = true;
        g.pac = Actor::at(10, 4, Dir::Right);
        g.ghosts[0].a = Actor::at(20, 4, Dir::Left);
        assert_eq!(g.target(0), (10, 4));
        assert_eq!(g.target(1), (14, 4));
        // Inky doubles the vector from Blinky to two ahead of the eater.
        assert_eq!(g.target(2), (2 * 12 - 20, 4));
        g.ghosts[3].a = Actor::at(12, 4, Dir::Left);
        assert_eq!(g.target(3), g.ghosts[3].corner);
        g.ghosts[3].a = Actor::at(30, 20, Dir::Left);
        assert_eq!(g.target(3), (10, 4));
        g.chase = false;
        assert_eq!(g.target(1), g.ghosts[1].corner);
    }

    /// The pellet in the eater's own corridor is its next move when no
    /// ghost is near.
    #[test]
    fn the_pilot_takes_the_nearest_pellet() {
        let mut g = game(52, 29);
        for gh in &mut g.ghosts {
            gh.state = GState::House;
        }
        let (sx, sy) = g.maze.start;
        let d = g.pilot();
        let (nx, ny) = g.step_tile(sx, sy, d).unwrap();
        assert!(g.maze.walkable(nx, ny));
        assert!(matches!(d, Dir::Left | Dir::Right), "{d:?}");
    }
}
