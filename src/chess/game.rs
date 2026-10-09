//! One game: the position, its record, whose turn it is to think, the move
//! being shown, and the pause after the result. No drawing here; `paint`
//! reads this and the flags it raises.

use std::time::Duration;

use super::book::BOOK;
use super::rules::{ended, kind, side, End, Move, Pos, San, EN_PASSANT};
use super::search::{level_limits, Engine, Job, HIST};
use crate::next_rand;

/// Plies before a game is adjudicated drawn.
pub const MAX_PLIES: usize = 400;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// Pause before the next move, then a book move or an engine's turn.
    Wait {
        until: u64,
    },
    Think {
        id: u32,
        asked: Option<u64>,
    },
    Glide {
        m: Move,
        t: u32,
    },
    Over {
        until: u64,
        end: End,
    },
}

/// Frame counts the game runs on, from the frame rate and the knobs.
#[derive(Clone, Copy)]
pub struct Pace {
    pub glide: u32,
    pub book: u64,
    pub min_think: u64,
    pub result: u64,
    pub think: Duration,
    pub fps: u64,
}

/// What `paint` must redraw this frame.
#[derive(Clone, Copy, Default)]
pub struct Dirty {
    pub full: bool,
    pub board: bool,
    pub info: bool,
}

pub struct Game {
    /// The position before the move being shown, during a glide.
    pub pos: Pos,
    pub hist: [u64; HIST],
    pub plies: usize,
    pub sans: [San; HIST],
    /// Pieces each side has taken, by kind.
    pub taken: [[u8; 7]; 2],
    pub levels: [u8; 2],
    pub theme: usize,
    pub opening: usize,
    book_plies: usize,
    pub phase: Phase,
    pub last: Option<Move>,
    /// White's view, centipawns, from the latest answer.
    pub eval: i32,
    pub depth: u8,
    pub end: Option<End>,
    pub dirty: Dirty,
    engine: Option<Engine>,
    next_id: u32,
    fixed_level: u8,
    rng: u32,
    pace: Pace,
    /// Frame the side to move started thinking, for the clock bar.
    pub think_from: u64,
}

impl Game {
    pub fn new(seed: u32, theme: usize, fixed_level: u8, pace: Pace) -> Self {
        let mut g = Self {
            pos: Pos::start(),
            hist: [0; HIST],
            plies: 0,
            sans: [San::default(); HIST],
            taken: [[0; 7]; 2],
            levels: [3, 3],
            theme,
            opening: 0,
            book_plies: 0,
            phase: Phase::Wait { until: 0 },
            last: None,
            eval: 0,
            depth: 0,
            end: None,
            dirty: Dirty::default(),
            engine: None,
            next_id: 1,
            fixed_level,
            rng: seed,
            pace,
            think_from: 0,
        };
        g.reset(0);
        g
    }

    /// Spawned on the first frame, not at construction: the mirror builds a
    /// saver only to read its knobs, and that must not start threads.
    pub fn start_engine(&mut self) {
        if self.engine.is_none() {
            self.engine = Some(Engine::spawn());
        }
    }

    fn roll(&mut self, n: u32) -> u32 {
        next_rand(&mut self.rng) % n.max(1)
    }

    fn reset(&mut self, now: u64) {
        self.pos = Pos::start();
        self.plies = 0;
        self.hist[0] = self.pos.hash;
        self.taken = [[0; 7]; 2];
        self.last = None;
        self.eval = 0;
        self.depth = 0;
        self.end = None;
        self.opening = self.roll(BOOK.len() as u32) as usize;
        let line = BOOK[self.opening].1.split(' ').count();
        self.book_plies = 4 + self.roll(line.saturating_sub(3) as u32) as usize;
        self.levels = if self.fixed_level > 0 {
            [self.fixed_level; 2]
        } else if self.roll(5) < 2 {
            let l = 3 + self.roll(3) as u8;
            [l, l]
        } else {
            [1 + self.roll(5) as u8, 1 + self.roll(5) as u8]
        };
        self.phase = Phase::Wait {
            until: now + self.pace.book,
        };
        self.dirty.full = true;
    }

    pub fn new_game(&mut self, now: u64) {
        self.theme = (self.theme + 1) % super::art::THEMES.len();
        self.reset(now);
    }

    fn book_move(&self) -> Option<Move> {
        if self.plies >= self.book_plies {
            return None;
        }
        let uci = BOOK[self.opening].1.split(' ').nth(self.plies)?;
        self.pos.parse_uci(uci)
    }

    /// The engine's live depth and White's score while it thinks.
    pub fn progress(&self) -> Option<(u8, i32)> {
        match (self.phase, &self.engine) {
            (Phase::Think { asked: Some(_), .. }, Some(e)) => Some(e.progress()),
            _ => None,
        }
    }

    pub fn update(&mut self, now: u64) {
        match self.phase {
            Phase::Wait { until } => {
                if now < until {
                    return;
                }
                if let Some(m) = self.book_move() {
                    self.glide(m);
                } else {
                    self.next_id = self.next_id.wrapping_add(1);
                    self.phase = Phase::Think {
                        id: self.next_id,
                        asked: None,
                    };
                    self.think_from = now;
                    self.dirty.info = true;
                }
            }
            Phase::Think { id, asked: None } => {
                let Some(e) = &self.engine else { return };
                let side = self.pos.stm;
                let seed = u64::from(self.rng) << 1 | side as u64;
                let mut job = Job {
                    id,
                    pos: self.pos,
                    hist: [0; HIST],
                    hist_len: self.plies + 1,
                    lim: level_limits(self.levels[side], self.pace.think, seed),
                };
                job.hist[..=self.plies].copy_from_slice(&self.hist[..=self.plies]);
                if e.ask(&job) {
                    self.phase = Phase::Think {
                        id,
                        asked: Some(now),
                    };
                }
            }
            Phase::Think {
                id,
                asked: Some(at),
            } => {
                if now < at + self.pace.min_think {
                    return;
                }
                let Some(a) = self.engine.as_ref().and_then(|e| e.poll(id)) else {
                    return;
                };
                self.eval = a.score;
                self.depth = a.depth;
                self.glide(a.mv);
            }
            Phase::Glide { m, t } => {
                if t + 1 < self.pace.glide {
                    self.phase = Phase::Glide { m, t: t + 1 };
                    self.dirty.board = true;
                } else {
                    self.land(m, now);
                }
            }
            Phase::Over { until, .. } => {
                if now >= until {
                    self.new_game(now);
                }
            }
        }
    }

    fn glide(&mut self, m: Move) {
        self.phase = Phase::Glide { m, t: 0 };
        self.last = Some(m);
        self.dirty.board = true;
        self.dirty.info = true;
    }

    /// Play the glided move into the record and see whether that ends it.
    fn land(&mut self, m: Move, now: u64) {
        let mover = self.pos.stm;
        let victim = if m.flag == EN_PASSANT {
            1
        } else {
            kind(self.pos.sq[m.to as usize])
        };
        if victim != 0 && side(self.pos.sq[m.from as usize]) == mover {
            self.taken[mover][victim as usize] += 1;
        }
        self.sans[self.plies] = self.pos.san(m);
        self.pos.make(m);
        self.plies += 1;
        self.hist[self.plies] = self.pos.hash;
        self.dirty.board = true;
        self.dirty.info = true;
        self.end = ended(&self.pos, &self.hist[..=self.plies], MAX_PLIES);
        self.phase = match self.end {
            Some(end) => {
                if let End::Mate(w) = end {
                    self.eval = if w == 0 { 30_000 } else { -30_000 };
                }
                Phase::Over {
                    until: now + self.pace.result,
                    end,
                }
            }
            None => Phase::Wait {
                until: now
                    + if self.plies < self.book_plies {
                        self.pace.book
                    } else {
                        0
                    },
            },
        };
    }

    pub fn opening_name(&self) -> &'static str {
        BOOK[self.opening].0
    }
}
