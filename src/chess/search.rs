//! The engine: iterative-deepening alpha-beta with quiescence, a small
//! transposition table, MVV-LVA and killer ordering, and a tapered
//! piece-square evaluation. One per game, on its own thread, so the frame
//! loop only ever polls a mutex it never waits on.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use super::rules::{kind, side, Move, Moves, Pos, BISHOP, EN_PASSANT, KING, MAX_MOVES, PAWN};

pub const MATE: i32 = 30_000;
const INF: i32 = 32_000;
/// Longest game the history arrays hold; `ended` adjudicates before this.
pub const HIST: usize = 512;
const MAX_PLY: usize = 64;
const TT_BITS: u32 = 16;

const VALUE: [i32; 7] = [0, 100, 320, 330, 500, 900, 0];

// Tables read from White's side with rank 8 first, as printed; a white piece
// on `s` reads `[s ^ 0o70]`, a black one `[s]`. Tomasz Michniewski's
// "simplified evaluation function".
#[rustfmt::skip]
const PST: [[i8; 64]; 6] = [
    [ 0,  0,  0,  0,  0,  0,  0,  0,
     50, 50, 50, 50, 50, 50, 50, 50,
     10, 10, 20, 30, 30, 20, 10, 10,
      5,  5, 10, 25, 25, 10,  5,  5,
      0,  0,  0, 20, 20,  0,  0,  0,
      5, -5,-10,  0,  0,-10, -5,  5,
      5, 10, 10,-20,-20, 10, 10,  5,
      0,  0,  0,  0,  0,  0,  0,  0],
    [-50,-40,-30,-30,-30,-30,-40,-50,
     -40,-20,  0,  0,  0,  0,-20,-40,
     -30,  0, 10, 15, 15, 10,  0,-30,
     -30,  5, 15, 20, 20, 15,  5,-30,
     -30,  0, 15, 20, 20, 15,  0,-30,
     -30,  5, 10, 15, 15, 10,  5,-30,
     -40,-20,  0,  5,  5,  0,-20,-40,
     -50,-40,-30,-30,-30,-30,-40,-50],
    [-20,-10,-10,-10,-10,-10,-10,-20,
     -10,  0,  0,  0,  0,  0,  0,-10,
     -10,  0,  5, 10, 10,  5,  0,-10,
     -10,  5,  5, 10, 10,  5,  5,-10,
     -10,  0, 10, 10, 10, 10,  0,-10,
     -10, 10, 10, 10, 10, 10, 10,-10,
     -10,  5,  0,  0,  0,  0,  5,-10,
     -20,-10,-10,-10,-10,-10,-10,-20],
    [ 0,  0,  0,  0,  0,  0,  0,  0,
      5, 10, 10, 10, 10, 10, 10,  5,
     -5,  0,  0,  0,  0,  0,  0, -5,
     -5,  0,  0,  0,  0,  0,  0, -5,
     -5,  0,  0,  0,  0,  0,  0, -5,
     -5,  0,  0,  0,  0,  0,  0, -5,
     -5,  0,  0,  0,  0,  0,  0, -5,
      0,  0,  0,  5,  5,  0,  0,  0],
    [-20,-10,-10, -5, -5,-10,-10,-20,
     -10,  0,  0,  0,  0,  0,  0,-10,
     -10,  0,  5,  5,  5,  5,  0,-10,
      -5,  0,  5,  5,  5,  5,  0, -5,
       0,  0,  5,  5,  5,  5,  0, -5,
     -10,  5,  5,  5,  5,  5,  0,-10,
     -10,  0,  5,  0,  0,  0,  0,-10,
     -20,-10,-10, -5, -5,-10,-10,-20],
    [-30,-40,-40,-50,-50,-40,-40,-30,
     -30,-40,-40,-50,-50,-40,-40,-30,
     -30,-40,-40,-50,-50,-40,-40,-30,
     -30,-40,-40,-50,-50,-40,-40,-30,
     -20,-30,-30,-40,-40,-30,-30,-20,
     -10,-20,-20,-20,-20,-20,-20,-10,
      20, 20,  0,  0,  0,  0, 20, 20,
      20, 30, 10,  0,  0, 10, 30, 20],
];
#[rustfmt::skip]
const KING_END: [i8; 64] = [
    -50,-40,-30,-20,-20,-30,-40,-50,
    -30,-20,-10,  0,  0,-10,-20,-30,
    -30,-10, 20, 30, 30, 20,-10,-30,
    -30,-10, 30, 40, 40, 30,-10,-30,
    -30,-10, 30, 40, 40, 30,-10,-30,
    -30,-10, 20, 30, 30, 20,-10,-30,
    -30,-30,  0,  0,  0,  0,-30,-30,
    -50,-30,-30,-30,-30,-30,-30,-50,
];
/// Endgame pawn push by ranks advanced: what turns a won ending into a queen
/// instead of fifty moves of shuffling.
const PAWN_RUN: [i32; 8] = [0, 0, 10, 20, 35, 60, 100, 0];
const PHASE: [i32; 7] = [0, 0, 1, 1, 2, 4, 0];

/// Static evaluation from the side to move's view, plus `noise` centipawns
/// of a hash-seeded wobble: a weaker player misjudges positions, and the
/// same position misjudged differently game to game is what keeps games
/// apart.
pub fn eval(p: &Pos, noise: i32, seed: u64) -> i32 {
    let (mut mg, mut eg) = ([0i32; 2], [0i32; 2]);
    let mut phase = 0;
    let mut big = [0i32; 2];
    let mut bishops = [0; 2];
    for (s, &pc) in p.sq.iter().enumerate() {
        if pc == 0 {
            continue;
        }
        let (k, c) = (kind(pc) as usize, side(pc));
        let at = if c == 0 { s ^ 0o70 } else { s };
        phase += PHASE[k];
        let v = VALUE[k];
        if k != PAWN as usize && k != KING as usize {
            big[c] += v;
        }
        if k == BISHOP as usize {
            bishops[c] += 1;
        }
        let t = i32::from(PST[k - 1][at]);
        if k == KING as usize {
            mg[c] += t;
            eg[c] += i32::from(KING_END[at]);
        } else {
            mg[c] += v + t;
            eg[c] += v + t;
            if k == PAWN as usize {
                let advanced = if c == 0 { s / 8 } else { 7 - s / 8 };
                eg[c] += PAWN_RUN[advanced];
            }
        }
    }
    for c in 0..2 {
        if bishops[c] >= 2 {
            mg[c] += 30;
            eg[c] += 40;
        }
    }
    let phase = phase.min(24);
    let mut score = ((mg[0] - mg[1]) * phase + (eg[0] - eg[1]) * (24 - phase)) / 24;
    score += mop_up(p, big, phase);
    if p.stm == 1 {
        score = -score;
    }
    if noise > 0 {
        let z = (p.hash ^ seed).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 40;
        score += (z % (2 * noise as u64 + 1)) as i32 - noise;
    }
    score + 8
}

/// A side well ahead in a thin endgame drives the other king to the edge and
/// walks its own over: piece-square tables alone cannot find a K+R mate.
fn mop_up(p: &Pos, big: [i32; 2], phase: i32) -> i32 {
    if phase > 8 {
        return 0;
    }
    let lead = big[0] - big[1];
    if lead.abs() < 300 {
        return 0;
    }
    let (win, lose) = if lead > 0 { (0, 1) } else { (1, 0) };
    let (wk, lk) = (p.kings[win], p.kings[lose]);
    let fr = |s: u8| (i32::from(s & 7), i32::from(s >> 3));
    let (lf, lr) = fr(lk);
    let (wf, wr) = fr(wk);
    let edge = (2 * lf - 7).abs() + (2 * lr - 7).abs();
    let near = 14 - ((lf - wf).abs() + (lr - wr).abs());
    let bonus = 6 * edge + 4 * near;
    if win == 0 {
        bonus
    } else {
        -bonus
    }
}

#[derive(Clone, Copy, Default)]
struct Entry {
    key: u64,
    mv: Move,
    score: i16,
    depth: i8,
    bound: u8,
}
const EXACT: u8 = 0;
const LOWER: u8 = 1;
const UPPER: u8 = 2;

#[derive(Clone, Copy)]
pub struct Limits {
    pub depth: u8,
    pub budget: Duration,
    pub noise: i32,
    pub seed: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct Answer {
    pub id: u32,
    pub mv: Move,
    /// Centipawns from White's side; past `MATE - 1000` is a forced mate.
    pub score: i32,
    pub depth: u8,
}

pub struct Searcher {
    tt: Vec<Entry>,
    killers: [[Move; 2]; MAX_PLY],
    /// Hashes from the game's start through the current search node, for
    /// repetitions.
    path: Vec<u64>,
    nodes: u64,
    deadline: Instant,
    stopped: bool,
    lim: Limits,
}

impl Searcher {
    pub fn new() -> Self {
        Self {
            tt: vec![Entry::default(); 1 << TT_BITS],
            killers: [[Move::NULL; 2]; MAX_PLY],
            path: Vec::with_capacity(HIST + MAX_PLY + 8),
            nodes: 0,
            deadline: Instant::now(),
            stopped: false,
            lim: Limits {
                depth: 1,
                budget: Duration::ZERO,
                noise: 0,
                seed: 0,
            },
        }
    }

    /// Best move at `pos` within `lim`, `history` being the game's hashes up
    /// to and including `pos`. `progress` gets each finished depth and score
    /// for the thinking display. `pos` must have a legal move.
    pub fn think(
        &mut self,
        pos: &Pos,
        history: &[u64],
        lim: Limits,
        abort: &AtomicBool,
        progress: &AtomicU64,
    ) -> (Move, i32, u8) {
        let start = Instant::now();
        self.deadline = start + lim.budget;
        self.lim = lim;
        self.stopped = false;
        self.nodes = 0;
        self.killers = [[Move::NULL; 2]; MAX_PLY];
        self.path.clear();
        self.path.extend_from_slice(history);
        let mut root = Moves::new();
        pos.legal(&mut root);
        let (mut best, mut best_score, mut reached) = (root.m[0], 0, 0u8);
        let mut p = *pos;
        for depth in 1..=lim.depth.max(1) {
            let (mv, score) = self.root(&mut p, &mut root, i32::from(depth), abort, best);
            if let Some(mv) = mv {
                best = mv;
                best_score = score;
            }
            if self.stopped {
                break;
            }
            reached = depth;
            let white = if pos.stm == 0 {
                best_score
            } else {
                -best_score
            };
            progress.store(pack(depth, white), Ordering::Relaxed);
            if best_score.abs() > MATE - 1000 || start.elapsed() * 2 > lim.budget {
                break;
            }
        }
        let white = if pos.stm == 0 {
            best_score
        } else {
            -best_score
        };
        (best, white, reached.max(1))
    }

    /// One iteration at the root, previous best first. A move that beat
    /// alpha is trusted even if the clock stops the iteration after it.
    fn root(
        &mut self,
        p: &mut Pos,
        root: &mut Moves,
        depth: i32,
        abort: &AtomicBool,
        prev: Move,
    ) -> (Option<Move>, i32) {
        if let Some(i) = root.as_slice().iter().position(|&m| m == prev) {
            root.m[..=i].rotate_right(1);
        }
        let (mut alpha, beta) = (-INF, INF);
        let mut best = None;
        for i in 0..root.n {
            let m = root.m[i];
            let u = p.make(m);
            self.path.push(p.hash);
            let score = -self.negamax(p, depth - 1, -beta, -alpha, 1, abort);
            self.path.pop();
            p.unmake(m, &u);
            if self.stopped {
                break;
            }
            if score > alpha {
                alpha = score;
                best = Some(m);
            }
        }
        (best, alpha)
    }

    fn tick(&mut self, abort: &AtomicBool) {
        self.nodes += 1;
        if self.nodes & 1023 == 0
            && (Instant::now() >= self.deadline || abort.load(Ordering::Relaxed))
        {
            self.stopped = true;
        }
    }

    /// `path` ends with `p` itself; earlier positions with the same side to
    /// move since the last capture or pawn move are every other one before.
    fn repeated(&self, p: &Pos) -> bool {
        let n = self.path.len();
        let lo = n.saturating_sub(1 + p.half as usize);
        (lo..n.saturating_sub(2))
            .rev()
            .step_by(2)
            .any(|i| self.path[i] == p.hash)
    }

    fn negamax(
        &mut self,
        p: &mut Pos,
        mut depth: i32,
        mut alpha: i32,
        beta: i32,
        ply: usize,
        abort: &AtomicBool,
    ) -> i32 {
        self.tick(abort);
        if self.stopped {
            return 0;
        }
        if p.half >= 100 || self.repeated(p) {
            return 0;
        }
        let check = p.in_check();
        if check {
            depth += 1;
        }
        if depth <= 0 || ply >= MAX_PLY - 1 {
            return self.quiesce(p, alpha, beta, ply, abort);
        }
        let slot = (p.hash >> (64 - TT_BITS)) as usize;
        let e = self.tt[slot];
        let mut tt_move = Move::NULL;
        if e.key == p.hash {
            tt_move = e.mv;
            if i32::from(e.depth) >= depth {
                let s = from_tt(e.score, ply);
                match e.bound {
                    EXACT => return s,
                    LOWER if s >= beta => return s,
                    UPPER if s <= alpha => return s,
                    _ => {}
                }
            }
        }
        let mut moves = Moves::new();
        p.gen(&mut moves, false);
        let mut keys = [0i32; MAX_MOVES];
        for (k, &m) in keys.iter_mut().zip(moves.as_slice()) {
            *k = self.order(p, m, tt_move, ply);
        }
        let alpha0 = alpha;
        let (mut best, mut best_move, mut legal) = (-INF, Move::NULL, 0);
        for i in 0..moves.n {
            let m = pick(&mut moves, &mut keys, i);
            let Some(u) = p.try_make(m) else { continue };
            legal += 1;
            self.path.push(p.hash);
            let score = -self.negamax(p, depth - 1, -beta, -alpha, ply + 1, abort);
            self.path.pop();
            let quiet = u.captured() == 0 && m.promo == 0 && m.flag != EN_PASSANT;
            p.unmake(m, &u);
            if self.stopped {
                return 0;
            }
            if score > best {
                best = score;
                best_move = m;
            }
            if score > alpha {
                alpha = score;
            }
            if alpha >= beta {
                if quiet && self.killers[ply][0] != m {
                    self.killers[ply] = [m, self.killers[ply][0]];
                }
                break;
            }
        }
        if legal == 0 {
            return if check { -MATE + ply as i32 } else { 0 };
        }
        let bound = if best >= beta {
            LOWER
        } else if best > alpha0 {
            EXACT
        } else {
            UPPER
        };
        self.tt[slot] = Entry {
            key: p.hash,
            mv: best_move,
            score: to_tt(best, ply),
            depth: depth as i8,
            bound,
        };
        best
    }

    fn quiesce(
        &mut self,
        p: &mut Pos,
        mut alpha: i32,
        beta: i32,
        ply: usize,
        abort: &AtomicBool,
    ) -> i32 {
        self.tick(abort);
        if self.stopped {
            return 0;
        }
        let stand = eval(p, self.lim.noise, self.lim.seed);
        if stand >= beta {
            return stand;
        }
        alpha = alpha.max(stand);
        if ply >= MAX_PLY - 1 {
            return stand;
        }
        let mut moves = Moves::new();
        p.gen(&mut moves, true);
        let mut keys = [0i32; MAX_MOVES];
        for (k, &m) in keys.iter_mut().zip(moves.as_slice()) {
            *k = mvv_lva(p, m);
        }
        for i in 0..moves.n {
            let m = pick(&mut moves, &mut keys, i);
            let Some(u) = p.try_make(m) else { continue };
            let score = -self.quiesce(p, -beta, -alpha, ply + 1, abort);
            p.unmake(m, &u);
            if self.stopped {
                return 0;
            }
            if score >= beta {
                return score;
            }
            alpha = alpha.max(score);
        }
        alpha
    }

    fn order(&self, p: &Pos, m: Move, tt: Move, ply: usize) -> i32 {
        if m == tt {
            return 1 << 20;
        }
        if p.sq[m.to as usize] != 0 || m.promo != 0 {
            return (1 << 16) + mvv_lva(p, m);
        }
        if self.killers[ply][0] == m {
            return 1 << 15;
        }
        if self.killers[ply][1] == m {
            return (1 << 15) - 1;
        }
        0
    }
}

fn mvv_lva(p: &Pos, m: Move) -> i32 {
    let victim = if m.flag == EN_PASSANT {
        VALUE[PAWN as usize]
    } else {
        VALUE[kind(p.sq[m.to as usize]) as usize]
    };
    let attacker = VALUE[kind(p.sq[m.from as usize]) as usize];
    victim * 10 - attacker / 10 + VALUE[m.promo as usize]
}

/// Mate scores count plies from the root; the table stores them counted from
/// the node, so a transposition at another ply reads the right distance.
fn to_tt(s: i32, ply: usize) -> i16 {
    let ply = ply as i32;
    let s = if s > MATE - 1000 {
        s + ply
    } else if s < -MATE + 1000 {
        s - ply
    } else {
        s
    };
    s.clamp(-INF, INF) as i16
}

fn from_tt(s: i16, ply: usize) -> i32 {
    let (s, ply) = (i32::from(s), ply as i32);
    if s > MATE - 1000 {
        s - ply
    } else if s < -MATE + 1000 {
        s + ply
    } else {
        s
    }
}

/// Selection sort, one step: the best remaining move to slot `i`.
fn pick(moves: &mut Moves, keys: &mut [i32], i: usize) -> Move {
    let mut b = i;
    for j in i + 1..moves.n {
        if keys[j] > keys[b] {
            b = j;
        }
    }
    moves.m.swap(i, b);
    keys.swap(i, b);
    moves.m[i]
}

/// Depth and White's score in one word, so the render thread reads both with
/// one relaxed load.
fn pack(depth: u8, score: i32) -> u64 {
    (u64::from(depth) << 32) | u64::from(score as u32)
}

pub fn unpack(w: u64) -> (u8, i32) {
    ((w >> 32) as u8, w as u32 as i32)
}

/// What the render thread hands the engine: everything copied in, so the
/// engine never reads the game while it moves.
#[derive(Clone, Copy)]
pub struct Job {
    pub id: u32,
    pub pos: Pos,
    pub hist: [u64; HIST],
    pub hist_len: usize,
    pub lim: Limits,
}

struct Slot {
    job: Option<Job>,
    answer: Option<Answer>,
    quit: bool,
}

struct Shared {
    slot: Mutex<Slot>,
    wake: Condvar,
    abort: AtomicBool,
    progress: AtomicU64,
}

/// One engine thread. Dropping it stops the search and lets the thread exit
/// without being joined: a saver switch must not wait out a think.
pub struct Engine {
    shared: Arc<Shared>,
}

impl Engine {
    pub fn spawn() -> Self {
        let shared = Arc::new(Shared {
            slot: Mutex::new(Slot {
                job: None,
                answer: None,
                quit: false,
            }),
            wake: Condvar::new(),
            abort: AtomicBool::new(false),
            progress: AtomicU64::new(0),
        });
        let theirs = Arc::clone(&shared);
        let _ = std::thread::Builder::new()
            .name("chess".into())
            .spawn(move || run(&theirs));
        Self { shared }
    }

    /// Hand over a job; false if the engine's lock was busy this instant, so
    /// the caller tries again next frame instead of waiting.
    pub fn ask(&self, job: &Job) -> bool {
        let Ok(mut s) = self.shared.slot.try_lock() else {
            return false;
        };
        s.job = Some(*job);
        s.answer = None;
        self.shared.progress.store(0, Ordering::Relaxed);
        drop(s);
        self.shared.wake.notify_one();
        true
    }

    /// The answer to job `id`, once, if it is in.
    pub fn poll(&self, id: u32) -> Option<Answer> {
        let mut s = self.shared.slot.try_lock().ok()?;
        match s.answer {
            Some(a) if a.id == id => s.answer.take(),
            _ => None,
        }
    }

    /// The depth finished and its score, while thinking.
    pub fn progress(&self) -> (u8, i32) {
        unpack(self.shared.progress.load(Ordering::Relaxed))
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shared.abort.store(true, Ordering::Relaxed);
        if let Ok(mut s) = self.shared.slot.lock() {
            s.quit = true;
        }
        self.shared.wake.notify_one();
    }
}

fn run(sh: &Shared) {
    let mut searcher = Searcher::new();
    loop {
        let job = {
            let Ok(mut s) = sh.slot.lock() else { return };
            loop {
                if s.quit {
                    return;
                }
                if let Some(j) = s.job.take() {
                    break j;
                }
                s = match sh.wake.wait(s) {
                    Ok(s) => s,
                    Err(_) => return,
                };
            }
        };
        let (mv, score, depth) = searcher.think(
            &job.pos,
            &job.hist[..job.hist_len],
            job.lim,
            &sh.abort,
            &sh.progress,
        );
        let Ok(mut s) = sh.slot.lock() else { return };
        if s.job.is_none() {
            s.answer = Some(Answer {
                id: job.id,
                mv,
                score,
                depth,
            });
        }
    }
}

/// Engine strength by level, 1 (weakest) to 5: depth cap, share of the think
/// budget, and evaluation noise in centipawns.
pub fn level_limits(level: u8, think: Duration, seed: u64) -> Limits {
    let i = usize::from(level.clamp(1, 5) - 1);
    const DEPTH: [u8; 5] = [2, 3, 4, 6, 32];
    const SHARE: [u32; 5] = [30, 50, 70, 85, 100];
    const NOISE: [i32; 5] = [90, 50, 25, 10, 0];
    Limits {
        depth: DEPTH[i],
        budget: think * SHARE[i] / 100,
        noise: NOISE[i],
        seed,
    }
}
