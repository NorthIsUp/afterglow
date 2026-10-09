//! The rules: a mailbox board, legal move generation, make/unmake, Zobrist
//! hashing, SAN and the ways a game ends. Everything is fixed-size so the
//! render thread can play a move and name it without allocating.

/// Piece kinds; a black piece is the kind with `BLACK` set.
pub const PAWN: u8 = 1;
pub const KNIGHT: u8 = 2;
pub const BISHOP: u8 = 3;
pub const ROOK: u8 = 4;
pub const QUEEN: u8 = 5;
pub const KING: u8 = 6;
pub const BLACK: u8 = 8;

pub const fn kind(p: u8) -> u8 {
    p & 7
}

/// 0 white, 1 black.
pub const fn side(p: u8) -> usize {
    (p >> 3) as usize
}

pub const fn file(s: u8) -> i8 {
    (s & 7) as i8
}

pub const fn rank(s: u8) -> i8 {
    (s >> 3) as i8
}

const NO_EP: u8 = 64;

pub const QUIET: u8 = 0;
pub const DOUBLE: u8 = 1;
pub const EN_PASSANT: u8 = 2;
pub const CASTLE: u8 = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Move {
    pub from: u8,
    pub to: u8,
    /// The kind promoted to, or 0.
    pub promo: u8,
    pub flag: u8,
}

impl Move {
    pub const NULL: Self = Self {
        from: 0,
        to: 0,
        promo: 0,
        flag: 0,
    };

    /// For a castle: the rook's from and to squares.
    pub fn rook_hop(self) -> (u8, u8) {
        if self.to > self.from {
            (self.from + 3, self.from + 1)
        } else {
            (self.from - 4, self.from - 1)
        }
    }
}

pub const MAX_MOVES: usize = 256;

pub struct Moves {
    pub m: [Move; MAX_MOVES],
    pub n: usize,
}

impl Moves {
    pub const fn new() -> Self {
        Self {
            m: [Move::NULL; MAX_MOVES],
            n: 0,
        }
    }

    #[inline]
    fn push(&mut self, m: Move) {
        self.m[self.n] = m;
        self.n += 1;
    }

    pub fn as_slice(&self) -> &[Move] {
        &self.m[..self.n]
    }
}

/// Splitmix, at compile time, so the Zobrist keys are a table in the binary.
const fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

const fn key(i: u64) -> u64 {
    mix(i
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(0x1234_5678))
}

const PIECE_KEYS: [[u64; 64]; 16] = {
    let mut t = [[0; 64]; 16];
    let mut p = 0;
    while p < 16 {
        let mut s = 0;
        while s < 64 {
            t[p][s] = key((p * 64 + s) as u64 + 1);
            s += 1;
        }
        p += 1;
    }
    t
};
const SIDE_KEY: u64 = key(5000);
const CASTLE_KEYS: [u64; 16] = {
    let mut t = [0; 16];
    let mut i = 0;
    while i < 16 {
        t[i] = key(6000 + i as u64);
        i += 1;
    }
    t
};
const EP_KEYS: [u64; 8] = {
    let mut t = [0; 8];
    let mut i = 0;
    while i < 8 {
        t[i] = key(7000 + i as u64);
        i += 1;
    }
    t
};

/// Castling rights a move off or onto each square leaves standing: a king or
/// rook moving, or a rook being captured, clears its bits.
const CASTLE_MASK: [u8; 64] = {
    let mut t = [15u8; 64];
    t[0] = 0b1101;
    t[4] = 0b1100;
    t[7] = 0b1110;
    t[56] = 0b0111;
    t[60] = 0b0011;
    t[63] = 0b1011;
    t
};

const KNIGHT_D: [(i8, i8); 8] = [
    (1, 2),
    (2, 1),
    (2, -1),
    (1, -2),
    (-1, -2),
    (-2, -1),
    (-2, 1),
    (-1, 2),
];
const KING_D: [(i8, i8); 8] = [
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
    (0, -1),
    (1, -1),
];
const ROOK_D: [(i8, i8); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
const BISHOP_D: [(i8, i8); 4] = [(1, 1), (1, -1), (-1, 1), (-1, -1)];

#[inline]
fn step(s: u8, (df, dr): (i8, i8)) -> Option<u8> {
    let (f, r) = (file(s) + df, rank(s) + dr);
    ((0..8).contains(&f) && (0..8).contains(&r)).then_some((r * 8 + f) as u8)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Pos {
    pub sq: [u8; 64],
    /// 0 white to move, 1 black.
    pub stm: usize,
    /// Bits: 1 white O-O, 2 white O-O-O, 4 black O-O, 8 black O-O-O.
    pub castle: u8,
    pub ep: u8,
    pub half: u16,
    pub hash: u64,
    pub kings: [u8; 2],
}

pub struct Undo {
    captured: u8,
    castle: u8,
    ep: u8,
    half: u16,
    hash: u64,
}

impl Undo {
    /// What stood on the target square, 0 for none (en passant included).
    pub fn captured(&self) -> u8 {
        self.captured
    }
}

impl Pos {
    pub fn start() -> Self {
        let back = [ROOK, KNIGHT, BISHOP, QUEEN, KING, BISHOP, KNIGHT, ROOK];
        let mut sq = [0u8; 64];
        for f in 0..8 {
            sq[f] = back[f];
            sq[8 + f] = PAWN;
            sq[48 + f] = PAWN | BLACK;
            sq[56 + f] = back[f] | BLACK;
        }
        let mut p = Self {
            sq,
            stm: 0,
            castle: 15,
            ep: NO_EP,
            half: 0,
            hash: 0,
            kings: [4, 60],
        };
        p.hash = p.compute_hash();
        p
    }

    pub fn compute_hash(&self) -> u64 {
        let mut h = CASTLE_KEYS[self.castle as usize];
        for (s, &p) in self.sq.iter().enumerate() {
            if p != 0 {
                h ^= PIECE_KEYS[p as usize][s];
            }
        }
        if self.stm == 1 {
            h ^= SIDE_KEY;
        }
        if self.ep != NO_EP {
            h ^= EP_KEYS[file(self.ep) as usize];
        }
        h
    }

    /// Is `s` attacked by a piece of side `by`?
    pub fn attacked(&self, s: u8, by: usize) -> bool {
        let col = (by as u8) << 3;
        let pawn_from: [(i8, i8); 2] = if by == 0 {
            [(-1, -1), (1, -1)]
        } else {
            [(-1, 1), (1, 1)]
        };
        for d in pawn_from {
            if step(s, d).is_some_and(|t| self.sq[t as usize] == PAWN | col) {
                return true;
            }
        }
        for d in KNIGHT_D {
            if step(s, d).is_some_and(|t| self.sq[t as usize] == KNIGHT | col) {
                return true;
            }
        }
        for d in KING_D {
            if step(s, d).is_some_and(|t| self.sq[t as usize] == KING | col) {
                return true;
            }
        }
        for (dirs, slider) in [(ROOK_D, ROOK), (BISHOP_D, BISHOP)] {
            for d in dirs {
                let mut t = s;
                while let Some(n) = step(t, d) {
                    t = n;
                    let p = self.sq[t as usize];
                    if p == 0 {
                        continue;
                    }
                    if side(p) == by && (kind(p) == slider || kind(p) == QUEEN) {
                        return true;
                    }
                    break;
                }
            }
        }
        false
    }

    pub fn in_check(&self) -> bool {
        self.attacked(self.kings[self.stm], self.stm ^ 1)
    }

    /// Pseudo-legal moves: the mover's king may be left in check. With
    /// `noisy`, only captures and queen promotions, for quiescence.
    pub fn gen(&self, out: &mut Moves, noisy: bool) {
        out.n = 0;
        let us = self.stm;
        for s in 0..64u8 {
            let p = self.sq[s as usize];
            if p == 0 || side(p) != us {
                continue;
            }
            match kind(p) {
                PAWN => self.gen_pawn(s, out, noisy),
                KNIGHT => self.gen_steps(s, &KNIGHT_D, out, noisy),
                BISHOP => self.gen_rays(s, &BISHOP_D, out, noisy),
                ROOK => self.gen_rays(s, &ROOK_D, out, noisy),
                QUEEN => {
                    self.gen_rays(s, &BISHOP_D, out, noisy);
                    self.gen_rays(s, &ROOK_D, out, noisy);
                }
                _ => {
                    self.gen_steps(s, &KING_D, out, noisy);
                    if !noisy {
                        self.gen_castles(s, out);
                    }
                }
            }
        }
    }

    fn target(&self, t: u8) -> Option<bool> {
        let q = self.sq[t as usize];
        if q == 0 {
            Some(false)
        } else if side(q) != self.stm {
            Some(true)
        } else {
            None
        }
    }

    fn gen_steps(&self, s: u8, dirs: &[(i8, i8)], out: &mut Moves, noisy: bool) {
        for &d in dirs {
            if let Some(t) = step(s, d) {
                if let Some(cap) = self.target(t) {
                    if cap || !noisy {
                        out.push(mv(s, t, 0, QUIET));
                    }
                }
            }
        }
    }

    fn gen_rays(&self, s: u8, dirs: &[(i8, i8)], out: &mut Moves, noisy: bool) {
        for &d in dirs {
            let mut t = s;
            while let Some(n) = step(t, d) {
                t = n;
                match self.target(t) {
                    Some(false) => {
                        if !noisy {
                            out.push(mv(s, t, 0, QUIET));
                        }
                    }
                    Some(true) => {
                        out.push(mv(s, t, 0, QUIET));
                        break;
                    }
                    None => break,
                }
            }
        }
    }

    fn gen_pawn(&self, s: u8, out: &mut Moves, noisy: bool) {
        let (dr, home, last) = if self.stm == 0 { (1, 1, 7) } else { (-1, 6, 0) };
        let push = |out: &mut Moves, t: u8, flag: u8| {
            if rank(t) == last {
                if noisy {
                    out.push(mv(s, t, QUEEN, flag));
                } else {
                    for k in [QUEEN, KNIGHT, ROOK, BISHOP] {
                        out.push(mv(s, t, k, flag));
                    }
                }
            } else {
                out.push(mv(s, t, 0, flag));
            }
        };
        if let Some(t) = step(s, (0, dr)) {
            if self.sq[t as usize] == 0 {
                if !noisy || rank(t) == last {
                    push(out, t, QUIET);
                }
                if !noisy && rank(s) == home {
                    let t2 = step(t, (0, dr)).unwrap();
                    if self.sq[t2 as usize] == 0 {
                        out.push(mv(s, t2, 0, DOUBLE));
                    }
                }
            }
        }
        for df in [-1, 1] {
            if let Some(t) = step(s, (df, dr)) {
                if t == self.ep {
                    out.push(mv(s, t, 0, EN_PASSANT));
                } else if self.target(t) == Some(true) {
                    push(out, t, QUIET);
                }
            }
        }
    }

    fn gen_castles(&self, s: u8, out: &mut Moves) {
        let (k_bit, q_bit) = if self.stm == 0 { (1, 2) } else { (4, 8) };
        let them = self.stm ^ 1;
        if self.castle & (k_bit | q_bit) == 0 || self.attacked(s, them) {
            return;
        }
        let empty = |a: u8| self.sq[a as usize] == 0;
        if self.castle & k_bit != 0 && empty(s + 1) && empty(s + 2) && !self.attacked(s + 1, them) {
            out.push(mv(s, s + 2, 0, CASTLE));
        }
        if self.castle & q_bit != 0
            && empty(s - 1)
            && empty(s - 2)
            && empty(s - 3)
            && !self.attacked(s - 1, them)
        {
            out.push(mv(s, s - 2, 0, CASTLE));
        }
    }

    pub fn make(&mut self, m: Move) -> Undo {
        let undo = Undo {
            captured: self.sq[m.to as usize],
            castle: self.castle,
            ep: self.ep,
            half: self.half,
            hash: self.hash,
        };
        let p = self.sq[m.from as usize];
        let mut h = self.hash ^ CASTLE_KEYS[self.castle as usize] ^ SIDE_KEY;
        if self.ep != NO_EP {
            h ^= EP_KEYS[file(self.ep) as usize];
        }
        h ^= PIECE_KEYS[p as usize][m.from as usize];
        if undo.captured != 0 {
            h ^= PIECE_KEYS[undo.captured as usize][m.to as usize];
        }
        self.half = if kind(p) == PAWN || undo.captured != 0 {
            0
        } else {
            self.half + 1
        };
        self.sq[m.from as usize] = 0;
        let placed = if m.promo != 0 {
            m.promo | (p & BLACK)
        } else {
            p
        };
        self.sq[m.to as usize] = placed;
        h ^= PIECE_KEYS[placed as usize][m.to as usize];
        self.ep = NO_EP;
        match m.flag {
            DOUBLE => {
                self.ep = (m.from + m.to) / 2;
                h ^= EP_KEYS[file(self.ep) as usize];
            }
            EN_PASSANT => {
                let victim = if self.stm == 0 { m.to - 8 } else { m.to + 8 };
                h ^= PIECE_KEYS[self.sq[victim as usize] as usize][victim as usize];
                self.sq[victim as usize] = 0;
                self.half = 0;
            }
            CASTLE => {
                let (rf, rt) = m.rook_hop();
                let r = self.sq[rf as usize];
                self.sq[rf as usize] = 0;
                self.sq[rt as usize] = r;
                h ^= PIECE_KEYS[r as usize][rf as usize] ^ PIECE_KEYS[r as usize][rt as usize];
            }
            _ => {}
        }
        if kind(p) == KING {
            self.kings[self.stm] = m.to;
        }
        self.castle &= CASTLE_MASK[m.from as usize] & CASTLE_MASK[m.to as usize];
        h ^= CASTLE_KEYS[self.castle as usize];
        self.hash = h;
        self.stm ^= 1;
        undo
    }

    pub fn unmake(&mut self, m: Move, u: &Undo) {
        self.stm ^= 1;
        let placed = self.sq[m.to as usize];
        let p = if m.promo != 0 {
            PAWN | (placed & BLACK)
        } else {
            placed
        };
        self.sq[m.from as usize] = p;
        self.sq[m.to as usize] = u.captured;
        match m.flag {
            EN_PASSANT => {
                let victim = if self.stm == 0 { m.to - 8 } else { m.to + 8 };
                self.sq[victim as usize] = PAWN | if self.stm == 0 { BLACK } else { 0 };
            }
            CASTLE => {
                let (rf, rt) = m.rook_hop();
                self.sq[rf as usize] = self.sq[rt as usize];
                self.sq[rt as usize] = 0;
            }
            _ => {}
        }
        if kind(p) == KING {
            self.kings[self.stm] = m.from;
        }
        self.castle = u.castle;
        self.ep = u.ep;
        self.half = u.half;
        self.hash = u.hash;
    }

    /// Make `m` if it leaves the mover's king safe; otherwise leave the
    /// position as it was.
    pub fn try_make(&mut self, m: Move) -> Option<Undo> {
        let u = self.make(m);
        if self.attacked(self.kings[self.stm ^ 1], self.stm) {
            self.unmake(m, &u);
            None
        } else {
            Some(u)
        }
    }

    pub fn legal(&self, out: &mut Moves) {
        let mut pseudo = Moves::new();
        self.gen(&mut pseudo, false);
        out.n = 0;
        let mut p = *self;
        for &m in pseudo.as_slice() {
            if let Some(u) = p.try_make(m) {
                p.unmake(m, &u);
                out.push(m);
            }
        }
    }

    pub fn has_legal(&self) -> bool {
        let mut pseudo = Moves::new();
        self.gen(&mut pseudo, false);
        let mut p = *self;
        pseudo
            .as_slice()
            .iter()
            .any(|&m| p.try_make(m).map(|u| p.unmake(m, &u)).is_some())
    }

    /// Neither side can mate with what is left: kings alone, or one minor.
    pub fn dead(&self) -> bool {
        let mut minors = 0;
        for &p in &self.sq {
            match kind(p) {
                0 | KING => {}
                KNIGHT | BISHOP => minors += 1,
                _ => return false,
            }
        }
        minors <= 1
    }

    /// The legal move from `from` to `to` (promoting to `promo` if a pawn
    /// reaches the last rank), if there is one.
    pub fn find(&self, from: u8, to: u8, promo: u8) -> Option<Move> {
        let mut l = Moves::new();
        self.legal(&mut l);
        l.as_slice()
            .iter()
            .copied()
            .find(|m| m.from == from && m.to == to && (m.promo == promo || m.promo == 0))
    }

    /// `e2e4`, `e7e8q`.
    pub fn parse_uci(&self, s: &str) -> Option<Move> {
        let b = s.as_bytes();
        if b.len() < 4 {
            return None;
        }
        let sq = |f: u8, r: u8| -> Option<u8> {
            ((b'a'..=b'h').contains(&f) && (b'1'..=b'8').contains(&r))
                .then(|| (r - b'1') * 8 + (f - b'a'))
        };
        let promo = match b.get(4) {
            Some(b'q') => QUEEN,
            Some(b'r') => ROOK,
            Some(b'b') => BISHOP,
            Some(b'n') => KNIGHT,
            _ => 0,
        };
        self.find(sq(b[0], b[1])?, sq(b[2], b[3])?, promo)
    }

    /// Standard algebraic notation for legal `m`, check and mate marks
    /// included.
    pub fn san(&self, m: Move) -> San {
        let mut out = San::default();
        let p = self.sq[m.from as usize];
        if m.flag == CASTLE {
            out.push_str(if m.to > m.from { "O-O" } else { "O-O-O" });
        } else {
            let capture = self.sq[m.to as usize] != 0 || m.flag == EN_PASSANT;
            if kind(p) == PAWN {
                if capture {
                    out.push(b'a' + file(m.from) as u8);
                }
            } else {
                out.push(b" PNBRQK"[kind(p) as usize]);
                let mut l = Moves::new();
                self.legal(&mut l);
                let (mut clash, mut same_file, mut same_rank) = (false, false, false);
                for o in l.as_slice() {
                    if o.to == m.to && o.from != m.from && self.sq[o.from as usize] == p {
                        clash = true;
                        same_file |= file(o.from) == file(m.from);
                        same_rank |= rank(o.from) == rank(m.from);
                    }
                }
                if clash {
                    if !same_file {
                        out.push(b'a' + file(m.from) as u8);
                    } else if !same_rank {
                        out.push(b'1' + rank(m.from) as u8);
                    } else {
                        out.push(b'a' + file(m.from) as u8);
                        out.push(b'1' + rank(m.from) as u8);
                    }
                }
            }
            if capture {
                out.push(b'x');
            }
            out.push(b'a' + file(m.to) as u8);
            out.push(b'1' + rank(m.to) as u8);
            if m.promo != 0 {
                out.push(b'=');
                out.push(b" PNBRQK"[m.promo as usize]);
            }
        }
        let mut after = *self;
        after.make(m);
        if after.in_check() {
            out.push(if after.has_legal() { b'+' } else { b'#' });
        }
        out
    }
}

const fn mv(from: u8, to: u8, promo: u8, flag: u8) -> Move {
    Move {
        from,
        to,
        promo,
        flag,
    }
}

/// A move's SAN, at most `Qa1xb2=Q#`-long, in place.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct San {
    b: [u8; 10],
    n: u8,
}

impl San {
    fn push(&mut self, c: u8) {
        if (self.n as usize) < self.b.len() {
            self.b[self.n as usize] = c;
            self.n += 1;
        }
    }

    fn push_str(&mut self, s: &str) {
        for &c in s.as_bytes() {
            self.push(c);
        }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.b[..self.n as usize]
    }
}

/// How a game ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum End {
    /// The side that delivered mate.
    Mate(usize),
    Stalemate,
    Fifty,
    Repetition,
    Material,
    /// Adjudicated: a screensaver game has to finish.
    Length,
}

impl End {
    /// `1-0`, `0-1` or `½-½`, spelled for a font with no `½`.
    pub fn score(self) -> &'static str {
        match self {
            End::Mate(0) => "1-0",
            End::Mate(_) => "0-1",
            _ => "1/2-1/2",
        }
    }

    pub fn reason(self) -> &'static str {
        match self {
            End::Mate(_) => "checkmate",
            End::Stalemate => "stalemate",
            End::Fifty => "fifty-move rule",
            End::Repetition => "threefold repetition",
            End::Material => "insufficient material",
            End::Length => "move limit",
        }
    }
}

/// The game is over at `pos`, reached after the positions hashed in
/// `history` (which includes `pos` itself last)? `max_plies` adjudicates.
pub fn ended(pos: &Pos, history: &[u64], max_plies: usize) -> Option<End> {
    if !pos.has_legal() {
        return Some(if pos.in_check() {
            End::Mate(pos.stm ^ 1)
        } else {
            End::Stalemate
        });
    }
    if pos.dead() {
        return Some(End::Material);
    }
    if pos.half >= 100 {
        return Some(End::Fifty);
    }
    if history.iter().filter(|&&h| h == pos.hash).count() >= 3 {
        return Some(End::Repetition);
    }
    (history.len() > max_plies).then_some(End::Length)
}
