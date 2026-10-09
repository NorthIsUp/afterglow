//! One well: SRS pieces and wall kicks, a 7-bag, NES-style gravity and
//! scoring, and the placement AI that plays it.
//!
//! A row is a `u32` bitboard with the ten columns at bits 4..14 and every other
//! bit set as wall, so a collision test is one AND per piece row and the AI's
//! features are popcounts.

use crate::next_rand;

pub const W: usize = 10;
/// Two hidden rows on top, where pieces spawn.
pub const H: usize = 22;
pub const HIDDEN: usize = 2;

const SHIFT: i32 = 4;
const INTERIOR: u32 = 0x3FF << SHIFT;
const WALL: u32 = !INTERIOR;
/// Bit pairs a row transition is read across: the wall bit either side of
/// the well and the ten columns between them.
const ROW_PAIRS: u32 = 0x7FF << (SHIFT - 1);
const SPAWN_X: i32 = 3;

/// Spawn-state cells of I O T S Z J L in their SRS box, the box size, and
/// the x offset that keeps O from moving when it "rotates" in a 2-box.
type Spawn = ([(u8, u8); 4], u8, u8);
const SPAWN: [Spawn; 7] = [
    ([(0, 1), (1, 1), (2, 1), (3, 1)], 4, 0),
    ([(0, 0), (1, 0), (0, 1), (1, 1)], 2, 1),
    ([(1, 0), (0, 1), (1, 1), (2, 1)], 3, 0),
    ([(1, 0), (2, 0), (0, 1), (1, 1)], 3, 0),
    ([(0, 0), (1, 0), (1, 1), (2, 1)], 3, 0),
    ([(0, 0), (0, 1), (1, 1), (2, 1)], 3, 0),
    ([(2, 0), (0, 1), (1, 1), (2, 1)], 3, 0),
];

/// Row masks of every piece in every rotation, `[kind][rot][box row]`.
pub const SHAPES: [[[u8; 4]; 4]; 7] = shapes();

const fn shapes() -> [[[u8; 4]; 4]; 7] {
    let mut out = [[[0u8; 4]; 4]; 7];
    let mut k = 0;
    while k < 7 {
        let (cells, n, off) = SPAWN[k];
        let mut rot = 0;
        while rot < 4 {
            let mut i = 0;
            while i < 4 {
                let (mut x, mut y) = cells[i];
                let mut r = 0;
                while r < rot {
                    (x, y) = (n - 1 - y, x);
                    r += 1;
                }
                out[k][rot][y as usize] |= 1 << (x + off);
                i += 1;
            }
            rot += 1;
        }
        k += 1;
    }
    out
}

/// SRS kick tests, `[from rotation][0 clockwise, 1 counter-clockwise]`, as
/// (dx, dy) with y DOWN — the guideline's tables negated in y.
#[rustfmt::skip]
const KICK_JLSTZ: [[[(i8, i8); 5]; 2]; 4] = [
    [[(0, 0), (-1, 0), (-1, -1), (0, 2), (-1, 2)], [(0, 0), (1, 0), (1, -1), (0, 2), (1, 2)]],
    [[(0, 0), (1, 0), (1, 1), (0, -2), (1, -2)], [(0, 0), (1, 0), (1, 1), (0, -2), (1, -2)]],
    [[(0, 0), (1, 0), (1, -1), (0, 2), (1, 2)], [(0, 0), (-1, 0), (-1, -1), (0, 2), (-1, 2)]],
    [[(0, 0), (-1, 0), (-1, 1), (0, -2), (-1, -2)], [(0, 0), (-1, 0), (-1, 1), (0, -2), (-1, -2)]],
];
#[rustfmt::skip]
const KICK_I: [[[(i8, i8); 5]; 2]; 4] = [
    [[(0, 0), (-2, 0), (1, 0), (-2, 1), (1, -2)], [(0, 0), (-1, 0), (2, 0), (-1, -2), (2, 1)]],
    [[(0, 0), (-1, 0), (2, 0), (-1, -2), (2, 1)], [(0, 0), (2, 0), (-1, 0), (2, -1), (-1, 2)]],
    [[(0, 0), (2, 0), (-1, 0), (2, -1), (-1, 2)], [(0, 0), (1, 0), (-2, 0), (1, 2), (-2, -1)]],
    [[(0, 0), (1, 0), (-2, 0), (1, 2), (-2, -1)], [(0, 0), (-2, 0), (1, 0), (-2, 1), (1, -2)]],
];

/// NES frames per row at 60 Hz, by level; 29 and up is the kill screen.
const FRAMES_PER_ROW: [u32; 30] = [
    48, 43, 38, 33, 28, 23, 18, 13, 8, 6, 5, 5, 5, 4, 4, 4, 3, 3, 3, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
    1,
];
const LINE_SCORE: [u64; 5] = [0, 40, 100, 300, 1200];

/// El-Tetris's weights (Yiyuan Lee, after Dellacherie), tuned by a genetic
/// search: landing height, rows cleared, row and column transitions, holes,
/// cumulative wells.
const WEIGHTS: [f32; 6] = [
    -4.500_158, 3.418_127, -3.217_888, -9.348_695, -7.899_265, -3.385_597,
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Board {
    pub rows: [u32; H],
}

impl Board {
    pub const fn empty() -> Self {
        Self { rows: [WALL; H] }
    }

    #[inline]
    fn row(&self, y: i32) -> u32 {
        if y < 0 {
            WALL
        } else if y >= H as i32 {
            u32::MAX
        } else {
            self.rows[y as usize]
        }
    }

    #[inline]
    pub fn fits(&self, kind: usize, rot: usize, x: i32, y: i32) -> bool {
        let sh = x + SHIFT;
        if !(0..=24).contains(&sh) {
            return false;
        }
        SHAPES[kind][rot]
            .iter()
            .enumerate()
            .all(|(r, &m)| m == 0 || self.row(y + r as i32) & (u32::from(m) << sh) == 0)
    }

    pub fn drop_y(&self, kind: usize, rot: usize, x: i32, mut y: i32) -> i32 {
        while self.fits(kind, rot, x, y + 1) {
            y += 1;
        }
        y
    }

    fn place(&mut self, kind: usize, rot: usize, x: i32, y: i32) {
        for (r, &m) in SHAPES[kind][rot].iter().enumerate() {
            let yy = y + r as i32;
            if m != 0 && (0..H as i32).contains(&yy) {
                self.rows[yy as usize] |= u32::from(m) << (x + SHIFT);
            }
        }
    }

    /// The full rows, as a bit per row.
    pub fn full(&self) -> u32 {
        (0..H).fold(0, |m, y| m | (u32::from(self.rows[y] == u32::MAX) << y))
    }

    /// Remove the rows in `mask`, everything above falling into their place.
    fn remove(&mut self, mask: u32) {
        let mut w = H;
        for y in (0..H).rev() {
            if mask & (1 << y) == 0 {
                w -= 1;
                self.rows[w] = self.rows[y];
            }
        }
        for r in &mut self.rows[..w] {
            *r = WALL;
        }
    }

    /// El-Tetris's score for the board after a move that landed at
    /// `landing2` (twice the height) and cleared `cleared` rows.
    fn eval(&self, landing2: i32, cleared: u32) -> f32 {
        let (mut row_t, mut col_t, mut holes, mut wells) = (0u32, 0u32, 0u32, 0u32);
        let mut above = 0u32;
        let mut run = [0u32; W];
        for y in 0..H {
            let r = self.rows[y];
            row_t += ((r ^ (r >> 1)) & ROW_PAIRS).count_ones();
            if y + 1 < H {
                col_t += ((r ^ self.rows[y + 1]) & INTERIOR).count_ones();
            } else {
                col_t += (!r & INTERIOR).count_ones();
            }
            let inner = r & INTERIOR;
            above |= inner;
            holes += (above & !inner).count_ones();
            let well = !r & (r << 1) & (r >> 1) & INTERIOR;
            for (c, n) in run.iter_mut().enumerate() {
                if well & (1 << (c as i32 + SHIFT)) != 0 {
                    *n += 1;
                    wells += *n;
                } else {
                    *n = 0;
                }
            }
        }
        let f = [
            landing2 as f32 / 2.0,
            cleared as f32,
            row_t as f32,
            col_t as f32,
            holes as f32,
            wells as f32,
        ];
        f.iter().zip(WEIGHTS).map(|(a, w)| a * w).sum()
    }
}

/// How many distinct rotations a piece needs searching: O has one.
const fn rotations(kind: usize) -> usize {
    if kind == 1 {
        1
    } else {
        4
    }
}

/// Every resting place reachable by rotating at the spawn, sliding along
/// the spawn row, then dropping: `f(rot, x, y)`.
fn placements(b: &Board, kind: usize, mut f: impl FnMut(usize, i32, i32)) {
    for rot in 0..rotations(kind) {
        if !b.fits(kind, rot, SPAWN_X, 0) {
            continue;
        }
        for dir in [-1, 1] {
            let mut x = if dir < 0 { SPAWN_X } else { SPAWN_X + 1 };
            while b.fits(kind, rot, x, 0) {
                f(rot, x, b.drop_y(kind, rot, x, 0));
                x += dir;
            }
        }
    }
}

/// The board after `kind` lands at `(rot, x, y)`, the rows it cleared, and
/// twice its landing height.
fn land(b: &Board, kind: usize, rot: usize, x: i32, y: i32) -> (Board, u32, i32) {
    let mut nb = *b;
    nb.place(kind, rot, x, y);
    let m = &SHAPES[kind][rot];
    let top = y + m.iter().position(|&r| r != 0).unwrap_or(0) as i32;
    let bot = y + m.iter().rposition(|&r| r != 0).unwrap_or(0) as i32;
    let landing2 = (H as i32 - top) + (H as i32 - bot);
    let full = nb.full();
    nb.remove(full);
    (nb, full.count_ones(), landing2)
}

/// The best place for `kind`, looking one piece ahead at `next` when given:
/// each placement is scored by the best the next piece can then do.
pub fn plan(b: &Board, kind: usize, next: Option<usize>) -> Option<(usize, i32)> {
    let mut best: Option<(f32, usize, i32)> = None;
    placements(b, kind, |rot, x, y| {
        let (nb, cleared, landing2) = land(b, kind, rot, x, y);
        let score = match next {
            None => nb.eval(landing2, cleared),
            Some(nk) => {
                let mut b2 = f32::NEG_INFINITY;
                placements(&nb, nk, |r2, x2, y2| {
                    let (nb2, c2, l2) = land(&nb, nk, r2, x2, y2);
                    b2 = b2.max(nb2.eval(l2, c2));
                });
                let own = WEIGHTS[0] * landing2 as f32 / 2.0 + WEIGHTS[1] * cleared as f32;
                own + b2
            }
        };
        if best.is_none_or(|(s, _, _)| score > s) {
            best = Some((score, rot, x));
        }
    });
    best.map(|(_, r, x)| (r, x))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Piece {
    pub kind: usize,
    pub rot: usize,
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// Entry delay before the next piece appears.
    Spawn(u32),
    Fall,
    /// Rows in the mask are being wiped; `t` frames in.
    Clear {
        mask: u32,
        t: u32,
    },
    /// The curtain, then a held board; `t` frames in.
    Over(u32),
}

/// Frame counts at the saver's rate, worked out once.
#[derive(Clone, Copy)]
pub struct Timing {
    fps: u32,
    pub move_every: u32,
    pub are: u32,
    pub clear: u32,
    pub curtain: u32,
    pub over: u32,
}

impl Timing {
    pub fn new(fps: u32, moves_per_sec: u32) -> Self {
        let fps = fps.max(1);
        let f = |ms: u32| (ms * fps / 1000).max(1);
        Self {
            fps,
            move_every: (fps / moves_per_sec.max(1)).max(1),
            are: f(170),
            clear: f(500),
            curtain: f(1500),
            over: f(3500),
        }
    }

    /// Gravity in 1/256 rows per frame.
    fn gravity(&self, level: u32) -> u32 {
        60 * 256 / (self.fps * FRAMES_PER_ROW[level.min(29) as usize])
    }

    /// Soft drop: 30 rows a second, NES's one row per two frames.
    fn soft(&self) -> u32 {
        30 * 256 / self.fps
    }
}

pub struct Game {
    pub board: Board,
    /// Piece kind + 1 per settled cell, 0 empty, row-major.
    pub colour: [u8; W * H],
    pub cur: Piece,
    pub next: usize,
    pub phase: Phase,
    pub level: u32,
    pub lines: u32,
    pub score: u64,
    pub start_level: u32,
    /// Games finished, and the lines the last one cleared.
    pub games: u32,
    pub last_lines: u32,
    bag: [u8; 7],
    bag_n: usize,
    rng: u32,
    target: (usize, i32),
    think: u32,
    cooldown: u32,
    acc: u32,
    lookahead: bool,
    pub t: Timing,
}

impl Game {
    pub fn new(seed: u32, start_level: u32, lookahead: bool, t: Timing) -> Self {
        let mut g = Self {
            board: Board::empty(),
            colour: [0; W * H],
            cur: Piece {
                kind: 0,
                rot: 0,
                x: SPAWN_X,
                y: 0,
            },
            next: 0,
            phase: Phase::Spawn(1),
            level: start_level,
            lines: 0,
            score: 0,
            start_level,
            games: 0,
            last_lines: 0,
            bag: [0; 7],
            bag_n: 0,
            rng: seed.max(1),
            target: (0, SPAWN_X),
            think: 0,
            cooldown: 0,
            acc: 0,
            lookahead,
            t,
        };
        g.next = g.draw();
        g
    }

    fn draw(&mut self) -> usize {
        if self.bag_n == 0 {
            self.bag = [0, 1, 2, 3, 4, 5, 6];
            for i in (1..7).rev() {
                let j = next_rand(&mut self.rng) as usize % (i + 1);
                self.bag.swap(i, j);
            }
            self.bag_n = 7;
        }
        self.bag_n -= 1;
        self.bag[self.bag_n] as usize
    }

    fn reset(&mut self) {
        self.games += 1;
        self.last_lines = self.lines;
        self.board = Board::empty();
        self.colour = [0; W * H];
        (self.level, self.lines, self.score) = (self.start_level, 0, 0);
        self.phase = Phase::Spawn(self.t.are);
    }

    fn spawn(&mut self) {
        self.cur = Piece {
            kind: self.next,
            rot: 0,
            x: SPAWN_X,
            y: 0,
        };
        self.next = self.draw();
        if !self.board.fits(self.cur.kind, 0, SPAWN_X, 0) {
            self.phase = Phase::Over(0);
            return;
        }
        let ahead = self.lookahead.then_some(self.next);
        self.target = plan(&self.board, self.cur.kind, ahead).unwrap_or((0, SPAWN_X));
        self.think = 8u32.saturating_sub(self.level / 2) * self.t.fps / 30;
        (self.acc, self.cooldown) = (0, 0);
        self.phase = Phase::Fall;
    }

    fn rotate(&mut self, dir: usize) {
        let Piece { kind, rot, x, y } = self.cur;
        let to = if dir == 0 {
            (rot + 1) % 4
        } else {
            (rot + 3) % 4
        };
        let kicks = match kind {
            0 => &KICK_I[rot][dir],
            1 => &[(0, 0); 5],
            _ => &KICK_JLSTZ[rot][dir],
        };
        for &(dx, dy) in kicks {
            let (nx, ny) = (x + i32::from(dx), y + i32::from(dy));
            if self.board.fits(kind, to, nx, ny) {
                self.cur = Piece {
                    kind,
                    rot: to,
                    x: nx,
                    y: ny,
                };
                return;
            }
        }
    }

    fn aligned(&self) -> bool {
        (self.cur.rot % rotations(self.cur.kind), self.cur.x) == self.target
    }

    /// One action toward the plan: rotate, then slide.
    fn act(&mut self) {
        let Piece { kind, rot, x, y } = self.cur;
        let want = self.target.0;
        if rot % rotations(kind) != want {
            self.rotate(usize::from((want + 4 - rot) % 4 == 3));
        } else if x != self.target.1 {
            let nx = x + (self.target.1 - x).signum();
            if self.board.fits(kind, rot, nx, y) {
                self.cur.x = nx;
            }
        }
    }

    fn fall(&mut self) {
        if self.think > 0 {
            self.think -= 1;
        } else if self.cooldown > 0 {
            self.cooldown -= 1;
        } else if !self.aligned() {
            self.act();
            self.cooldown = self.t.move_every - 1;
        }
        let aligned = self.aligned() && self.think == 0;
        let g = self.t.gravity(self.level);
        self.acc += if aligned { g.max(self.t.soft()) } else { g };
        let Piece { kind, rot, x, .. } = self.cur;
        while self.acc >= 256 {
            self.acc -= 256;
            if !self.board.fits(kind, rot, x, self.cur.y + 1) {
                self.lock();
                return;
            }
            self.cur.y += 1;
        }
    }

    fn lock(&mut self) {
        let Piece { kind, rot, x, y } = self.cur;
        self.board.place(kind, rot, x, y);
        let mut visible = false;
        for (r, &m) in SHAPES[kind][rot].iter().enumerate() {
            for c in 0..4 {
                if m & (1 << c) != 0 {
                    let (cx, cy) = (x + c, y + r as i32);
                    visible |= cy >= HIDDEN as i32;
                    self.colour[cy as usize * W + cx as usize] = kind as u8 + 1;
                }
            }
        }
        let mask = self.board.full();
        self.phase = if !visible {
            Phase::Over(0)
        } else if mask != 0 {
            Phase::Clear { mask, t: 0 }
        } else {
            Phase::Spawn(self.t.are)
        };
    }

    fn finish_clear(&mut self, mask: u32) {
        self.board.remove(mask);
        let mut w = H;
        for y in (0..H).rev() {
            if mask & (1 << y) == 0 {
                w -= 1;
                self.colour.copy_within(y * W..y * W + W, w * W);
            }
        }
        self.colour[..w * W].fill(0);
        let n = mask.count_ones();
        self.score += LINE_SCORE[n.min(4) as usize] * u64::from(self.level + 1);
        self.lines += n;
        self.level = self.level.max(self.start_level + self.lines / 10);
    }

    pub fn step(&mut self) {
        self.phase = match self.phase {
            Phase::Spawn(0) => {
                self.spawn();
                return;
            }
            Phase::Spawn(n) => Phase::Spawn(n - 1),
            Phase::Fall => {
                self.fall();
                return;
            }
            Phase::Clear { mask, t } if t + 1 >= self.t.clear => {
                self.finish_clear(mask);
                Phase::Spawn(self.t.are)
            }
            Phase::Clear { mask, t } => Phase::Clear { mask, t: t + 1 },
            Phase::Over(t) if t + 1 >= self.t.over => {
                self.reset();
                return;
            }
            Phase::Over(t) => Phase::Over(t + 1),
        };
    }

    /// Where the piece in play would land, for the ghost.
    pub fn ghost_y(&self) -> i32 {
        let Piece { kind, rot, x, y } = self.cur;
        self.board.drop_y(kind, rot, x, y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(kind: usize, rot: usize) -> usize {
        SHAPES[kind][rot]
            .iter()
            .map(|m| m.count_ones() as usize)
            .sum()
    }

    #[test]
    fn every_rotation_is_four_cells_and_four_turns_come_home() {
        for k in 0..7 {
            for r in 0..4 {
                assert_eq!(cells(k, r), 4, "kind {k} rot {r}");
            }
        }
        // O never moves; T's right state points right.
        assert!(SHAPES[1].iter().all(|s| *s == SHAPES[1][0]));
        assert_eq!(SHAPES[2][1], [0b010, 0b110, 0b010, 0]);
        // I's vertical states sit in columns 2 and 1.
        assert_eq!(SHAPES[0][1], [4, 4, 4, 4]);
        assert_eq!(SHAPES[0][3], [2, 2, 2, 2]);
    }

    /// A T against the left wall rotates by kicking right, which a game
    /// without SRS kicks refuses.
    #[test]
    fn a_wall_kick_lets_a_piece_rotate_off_the_wall() {
        let mut g = Game::new(1, 0, false, Timing::new(30, 15));
        g.cur = Piece {
            kind: 2,
            rot: 1,
            x: -1,
            y: 10,
        };
        assert!(g.board.fits(2, 1, -1, 10));
        assert!(!g.board.fits(2, 2, -1, 10));
        g.rotate(0);
        assert_eq!((g.cur.rot, g.cur.x), (2, 0));
    }

    #[test]
    fn a_bag_deals_each_piece_once() {
        let mut g = Game::new(9, 0, false, Timing::new(30, 15));
        g.bag_n = 0;
        for _ in 0..10 {
            let mut seen = [false; 7];
            for _ in 0..7 {
                seen[g.draw()] = true;
            }
            assert!(seen.iter().all(|&s| s));
        }
    }

    #[test]
    fn full_rows_clear_and_the_stack_falls() {
        let mut b = Board::empty();
        b.rows[H - 1] = u32::MAX;
        b.rows[H - 2] = WALL | (1 << SHIFT);
        assert_eq!(b.full(), 1 << (H - 1));
        b.remove(b.full());
        assert_eq!(b.rows[H - 1], WALL | (1 << SHIFT));
        assert!(b.rows[..H - 1].iter().all(|&r| r == WALL));
    }

    /// The AI fills a well-shaped hole with the piece that fits it rather
    /// than burying it.
    #[test]
    fn the_ai_drops_an_i_into_a_well() {
        let mut b = Board::empty();
        for y in H - 4..H {
            b.rows[y] = !(1 << (SHIFT + 9));
        }
        let (rot, x) = plan(&b, 0, None).unwrap();
        let y = b.drop_y(0, rot, x, 0);
        let (_, cleared, _) = land(&b, 0, rot, x, y);
        assert_eq!(cleared, 4, "rot {rot} x {x}");
    }
}
