//! A maze in the arcade style, generated to any size: mirror-symmetric, no
//! dead ends, a ghost house in the middle and a wrap-around tunnel.
//!
//! Corridors start as a lattice — a corridor every three or four tiles both
//! ways, so the walls between are 2x2 or 3x3 blocks — over the left half and
//! the edges that cross the middle. Edges are then cut at random, each cut
//! kept only if both ends stay junctions or bends (degree two or more) and the
//! lattice stays connected; cut edges merge their neighbouring blocks into the
//! long and L-shaped walls of the real thing. The right half is the mirror.
//!
//! Every buffer is sized once for the panel, so a new level's maze is built
//! inside the frame loop without allocating.

use crate::next_rand;

pub const WALL: u8 = 1;
/// The ghost house's gate: a wall to the eater, open to a ghost.
pub const DOOR: u8 = 2;
pub const HOUSE: u8 = 4;
pub const TUNNEL: u8 = 8;
pub const PELLET: u8 = 16;
pub const POWER: u8 = 32;

const E_NONE: u8 = 0;
const E_OPEN: u8 = 1;
const E_FIXED: u8 = 2;
const E_CUT: u8 = 3;

pub const MAX_POWER: usize = 8;

/// `n` lattice lines spread evenly over `lo..=hi`.
fn spread(lo: usize, hi: usize) -> Vec<usize> {
    let span = hi.saturating_sub(lo);
    let n = (span / 3 + 1).max(2);
    (0..n)
        .map(|k| lo + (k * span + (n - 1) / 2) / (n - 1))
        .collect()
}

pub struct Maze {
    pub w: usize,
    pub h: usize,
    /// Flags per tile, row-major.
    pub t: Vec<u8>,
    xs: Vec<usize>,
    ys: Vec<usize>,
    edges: Vec<u8>,
    order: Vec<u32>,
    seen: Vec<bool>,
    stack: Vec<u32>,
    /// The house interior, inclusive, and the tile above its gate.
    pub house: (usize, usize, usize, usize),
    pub exit: (usize, usize),
    pub start: (usize, usize),
    pub tunnel_y: usize,
    pub pellets: usize,
    pub power: [(usize, usize); MAX_POWER],
    pub npower: usize,
}

impl Maze {
    /// A maze of `w x h` tiles; `w` is rounded down to even for the mirror.
    pub fn new(w: usize, h: usize) -> Self {
        let w = (w.max(18)) & !1;
        let h = h.max(12);
        let xs = spread(1, w / 2 - 2);
        let ys = spread(1, h - 2);
        let nodes = xs.len() * ys.len();
        Self {
            w,
            h,
            t: vec![WALL; w * h],
            edges: vec![E_NONE; nodes * 2],
            order: Vec::with_capacity(nodes * 2),
            seen: vec![false; nodes],
            stack: Vec::with_capacity(nodes),
            xs,
            ys,
            house: (0, 0, 0, 0),
            exit: (0, 0),
            start: (0, 0),
            tunnel_y: 0,
            pellets: 0,
            power: [(0, 0); MAX_POWER],
            npower: 0,
        }
    }

    #[inline]
    pub fn at(&self, x: usize, y: usize) -> u8 {
        self.t[y * self.w + x]
    }

    /// Open to the eater: not wall, not gate, not house.
    #[inline]
    pub fn walkable(&self, x: usize, y: usize) -> bool {
        self.at(x, y) & (WALL | DOOR | HOUSE) == 0
    }

    fn c(&self) -> usize {
        self.xs.len()
    }

    /// Ring rows: the house sits between lattice rows `k` and `k + 2`.
    fn ring_k(&self) -> usize {
        (self.ys.len() / 2).saturating_sub(1).min(self.ys.len() - 3)
    }

    /// Edge ids of node `(i, j)`: rightward (the middle crossing, for the
    /// last column) and downward.
    #[inline]
    fn right(&self, i: usize, j: usize) -> usize {
        (j * self.c() + i) * 2
    }

    #[inline]
    fn down(&self, i: usize, j: usize) -> usize {
        (j * self.c() + i) * 2 + 1
    }

    #[inline]
    fn open(&self, e: usize) -> bool {
        matches!(self.edges[e], E_OPEN | E_FIXED)
    }

    /// Neighbours of node `n` along open edges, as node ids; the middle
    /// crossing and the tunnel lead to the node's own mirror and are left
    /// out, since connectivity is checked on the left half.
    fn links(&self, n: usize, out: &mut [usize; 4]) -> usize {
        let c = self.c();
        let (i, j) = (n % c, n / c);
        let mut k = 0;
        let mut add = |cond: bool, m: usize| {
            if cond {
                out[k] = m;
                k += 1;
            }
        };
        add(i + 1 < c && self.open(self.right(i, j)), n + 1);
        add(i > 0 && self.open(self.right(i - 1, j)), n - 1);
        add(self.open(self.down(i, j)), n + c);
        add(j > 0 && self.open(self.down(i, j - 1)), n - c);
        k
    }

    fn degree(&self, n: usize) -> usize {
        let c = self.c();
        let (i, j) = (n % c, n / c);
        let mut out = [0; 4];
        let mut d = self.links(n, &mut out);
        d += usize::from(i + 1 == c && self.open(self.right(i, j)));
        d += usize::from(i == 0 && self.ys[j] == self.tunnel_y);
        d
    }

    fn connected(&mut self) -> bool {
        let nodes = self.seen.len();
        self.seen.fill(false);
        self.stack.clear();
        let Some(first) = (0..nodes).find(|&n| self.degree(n) > 0) else {
            return false;
        };
        self.seen[first] = true;
        self.stack.push(first as u32);
        let mut reached = 1;
        let mut out = [0; 4];
        while let Some(n) = self.stack.pop() {
            let k = self.links(n as usize, &mut out);
            for &m in &out[..k] {
                if !self.seen[m] {
                    self.seen[m] = true;
                    reached += 1;
                    self.stack.push(m as u32);
                }
            }
        }
        reached == (0..nodes).filter(|&n| self.degree(n) > 0).count()
    }

    /// Build a new maze for `seed`, cutting about `cut_pct` per cent of the
    /// lattice's removable edges.
    pub fn generate(&mut self, seed: u32, cut_pct: usize) {
        let (c, r) = (self.xs.len(), self.ys.len());
        let k = self.ring_k();
        self.tunnel_y = self.ys[k + 1];
        for j in 0..r {
            for i in 0..c {
                let (rt, dn) = (self.right(i, j), self.down(i, j));
                self.edges[rt] = E_OPEN;
                self.edges[dn] = if j + 1 < r { E_OPEN } else { E_NONE };
            }
        }
        // The ring around the house is fixed open, the node inside it gone.
        for i in [c - 2, c - 1] {
            for j in [k, k + 2] {
                let e = self.right(i, j);
                self.edges[e] = E_FIXED;
            }
        }
        for j in [k, k + 1] {
            let e = self.down(c - 2, j);
            self.edges[e] = E_FIXED;
            let e = self.down(c - 1, j);
            self.edges[e] = E_CUT;
        }
        for i in [c - 2, c - 1] {
            let e = self.right(i, k + 1);
            self.edges[e] = E_CUT;
        }
        let sj = (k + 3).min(r - 1);
        let e = self.right(c - 1, sj);
        self.edges[e] = E_FIXED;

        let mut rng = seed.max(1);
        self.order.clear();
        self.order
            .extend((0..self.edges.len() as u32).filter(|&e| self.edges[e as usize] == E_OPEN));
        for i in (1..self.order.len()).rev() {
            let j = next_rand(&mut rng) as usize % (i + 1);
            self.order.swap(i, j);
        }
        let target = self.order.len() * cut_pct / 100;
        let mut cut = 0;
        for o in 0..self.order.len() {
            if cut >= target {
                break;
            }
            let e = self.order[o] as usize;
            let n = e / 2;
            let m = if e % 2 == 1 {
                n + c
            } else if n % c + 1 < c {
                n + 1
            } else {
                n
            };
            if self.degree(n) < 3 || self.degree(m) < 3 {
                continue;
            }
            self.edges[e] = E_CUT;
            if self.degree(n) < 2 || self.degree(m) < 2 || !self.connected() {
                self.edges[e] = E_OPEN;
            } else {
                cut += 1;
            }
        }
        self.carve(k, sj);
    }

    fn set_both(&mut self, x: usize, y: usize, v: u8) {
        let w = self.w;
        self.t[y * w + x] = v;
        self.t[y * w + (w - 1 - x)] = v;
    }

    fn carve(&mut self, k: usize, sj: usize) {
        let (c, r, w, h) = (self.xs.len(), self.ys.len(), self.w, self.h);
        self.t.fill(WALL);
        for j in 0..r {
            for i in 0..c {
                let n = j * c + i;
                if self.degree(n) == 0 {
                    continue;
                }
                let (x, y) = (self.xs[i], self.ys[j]);
                self.set_both(x, y, 0);
                if self.open(self.right(i, j)) {
                    let x1 = if i + 1 < c { self.xs[i + 1] } else { w / 2 };
                    for xx in x..=x1 {
                        self.set_both(xx, y, 0);
                    }
                }
                if j + 1 < r && self.open(self.down(i, j)) {
                    for yy in y..=self.ys[j + 1] {
                        self.set_both(x, yy, 0);
                    }
                }
            }
        }
        self.tunnel_y = self.ys[k + 1];
        for x in 0..self.xs[0] {
            self.set_both(x, self.tunnel_y, TUNNEL);
        }
        let (x0, x1) = (self.xs[c - 2] + 2, w - 3 - self.xs[c - 2]);
        let (y0, y1) = (self.ys[k] + 2, self.ys[k + 2] - 2);
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.t[y * w + x] = HOUSE;
            }
        }
        self.t[(y0 - 1) * w + w / 2 - 1] = DOOR;
        self.t[(y0 - 1) * w + w / 2] = DOOR;
        self.house = (x0, y0, x1, y1);
        self.exit = (w / 2 - 1, self.ys[k]);
        self.start = (w / 2 - 1, self.ys[sj]);

        self.pellets = 0;
        for i in 0..w * h {
            if self.t[i] == 0 {
                self.t[i] = PELLET;
                self.pellets += 1;
            }
        }
        let (sx, sy) = self.start;
        for x in [sx, sx + 1] {
            self.t[sy * w + x] &= !PELLET;
            self.pellets -= 1;
        }
        self.npower = 0;
        let spots = [(0, 1), (0, r - 2), (c / 2, 0), (c / 2, r - 1)];
        let n = if c >= 10 { 4 } else { 2 };
        for &(i, j) in &spots[..n] {
            let (x, y) = (self.xs[i], self.ys[j]);
            for xx in [x, w - 1 - x] {
                if self.t[y * w + xx] & PELLET != 0 {
                    self.t[y * w + xx] = POWER;
                    self.pellets -= 1;
                    self.power[self.npower] = (xx, y);
                    self.npower += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs(m: &Maze, x: usize, y: usize) -> usize {
        let (w, h) = (m.w, m.h);
        [
            (x, (y + h - 1) % h),
            (x, (y + 1) % h),
            ((x + w - 1) % w, y),
            ((x + 1) % w, y),
        ]
        .iter()
        .filter(|&&(a, b)| m.walkable(a, b))
        .count()
    }

    /// Every size the saver builds: symmetric, no dead end, all of it
    /// reachable from the start, a tunnel, and a house with a gate.
    #[test]
    fn mazes_are_symmetric_connected_and_have_no_dead_ends() {
        for (w, h) in [
            (90, 26),
            (52, 29),
            (36, 27),
            (30, 29),
            (20, 36),
            (20, 20),
            (18, 12),
        ] {
            let mut m = Maze::new(w, h);
            for seed in 1..20 {
                m.generate(seed, 45);
                let at = format!("{w}x{h} seed {seed}");
                for y in 0..m.h {
                    for x in 0..m.w {
                        assert_eq!(m.at(x, y), m.at(m.w - 1 - x, y), "{at}: asymmetric");
                        if m.walkable(x, y) {
                            assert!(dirs(&m, x, y) >= 2, "{at}: dead end at {x},{y}");
                        }
                    }
                }
                let mut seen = vec![false; m.w * m.h];
                let mut stack = vec![m.start];
                seen[m.start.1 * m.w + m.start.0] = true;
                while let Some((x, y)) = stack.pop() {
                    for (a, b) in [
                        (x, (y + m.h - 1) % m.h),
                        (x, (y + 1) % m.h),
                        ((x + m.w - 1) % m.w, y),
                        ((x + 1) % m.w, y),
                    ] {
                        if m.walkable(a, b) && !seen[b * m.w + a] {
                            seen[b * m.w + a] = true;
                            stack.push((a, b));
                        }
                    }
                }
                let open = (0..m.w * m.h)
                    .filter(|&i| m.walkable(i % m.w, i / m.w))
                    .count();
                assert_eq!(seen.iter().filter(|&&s| s).count(), open, "{at}: islands");
                assert!(m.at(0, m.tunnel_y) & TUNNEL != 0, "{at}: no tunnel");
                assert!(m.at(m.exit.0, m.exit.1 + 1) & DOOR != 0, "{at}: no gate");
                assert!(m.pellets > 50 && m.npower >= 2, "{at}: pellets");
            }
        }
    }

    /// Cuts are what make it a maze rather than a grid of 2x2 blocks.
    #[test]
    fn a_fair_share_of_the_lattice_is_cut() {
        let mut m = Maze::new(90, 26);
        m.generate(7, 45);
        let (mut cut, mut open) = (0, 0);
        for &e in &m.edges {
            cut += usize::from(e == E_CUT);
            open += usize::from(e == E_OPEN);
        }
        assert!(
            cut * 100 / (cut + open) >= 30,
            "{cut} cut of {}",
            cut + open
        );
    }

    #[test]
    fn a_new_maze_never_allocates() {
        let mut m = Maze::new(90, 26);
        m.generate(1, 45);
        let n = crate::testalloc::allocs_during(|| {
            for s in 2..10 {
                m.generate(s, 45);
            }
        });
        assert_eq!(n, 0);
    }
}
