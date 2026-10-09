//! `chess`: two engines playing each other, one game per board, as many
//! boards as the panel's shape holds.
//!
//! Every logical pixel is a SOLID cell, sized so a board square lands near
//! 32 of them: pieces are 16x16 sprites drawn at a whole multiple, and the
//! text beside them at one cell per font pixel. A landscape panel is split
//! into side-by-side slots (two on pine's 3.2:1); a slot wider than it is
//! tall puts the board left and the record right, a taller one stacks them.
//!
//! The engines (`search.rs`) run on a thread per game; the frame loop hands
//! a position over and polls for the answer through a mutex it only ever
//! `try_lock`s, so a think never costs a frame.
//!
//! A capture is fought out up close (`fight.rs`): the move is played at once
//! so the next think starts, and only showing its answer waits for the fight.

mod art;
mod book;
mod fight;
mod fight_paint;
mod fighters;
mod game;
mod paint;
mod rules;
mod search;
#[cfg(test)]
mod tests;

use std::time::Duration;

use crate::grid::{pixel_aspect, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, next_rand, saver_seed};

use art::BG;
use fight_paint::Span;
use game::{Dirty, Game, Pace, Phase};
use paint::{bar_share, square_colours, Art, Canvas, Painter};

/// Logical pixels a board square aims for: two cells per sprite pixel.
const TARGET_SQ: f32 = 34.0;
/// Narrowest side panel worth drawing beside a board.
const INFO_MIN: i32 = 150;

/// One slot's geometry in logical pixels. Rects are `(x, y, w, h)`.
pub struct Layout {
    slot: (i32, i32, i32, i32),
    bx: i32,
    by: i32,
    sq: i32,
    /// Sprite pixels per design pixel.
    k: i32,
    ev: (i32, i32, i32, i32),
    info: (i32, i32, i32, i32),
}

impl Layout {
    fn new(slot: (i32, i32, i32, i32)) -> Self {
        let (x, y, w, h) = slot;
        let m = (w.min(h) / 48).max(2);
        let gap = (m / 2).max(2);
        let ev_w = (h.min(w) / 24).clamp(3, 14);
        let wide = w * 10 >= h * 12;
        let sq = if wide {
            ((h - 2 * m) / 8).min((w - 2 * m - ev_w - 3 * gap - INFO_MIN) / 8)
        } else {
            ((w - 2 * m - ev_w - gap) / 8).min(h * 62 / 100 / 8)
        }
        .max(2);
        let b = sq * 8;
        let (bx, by, info) = if wide {
            let by = y + (h - b) / 2;
            let bx = x + m;
            let ix = bx + b + ev_w + 3 * gap;
            (bx, by, (ix, y + m, x + w - m - ix, h - 2 * m))
        } else {
            let bx = x + (w - b - ev_w - gap) / 2;
            let by = y + m;
            let iy = by + b + 2 * gap;
            (bx, by, (x + m, iy, w - 2 * m, y + h - m - iy))
        };
        Self {
            slot,
            bx,
            by,
            sq,
            k: (sq / art::SPRITE as i32).max(1),
            ev: (bx + b + gap, by, ev_w, b),
            info,
        }
    }
}

/// How many games, and the cell size in framebuffer pixels across.
fn plan(panel: &Panel, aspect: usize, games: usize) -> (usize, usize, bool) {
    let gw = panel.w as f32;
    let gh = panel.h as f32 * 100.0 / aspect as f32;
    let landscape = gw >= gh * 1.2;
    let n = if games > 0 {
        games
    } else if landscape {
        ((gw / gh / 1.6) as usize).clamp(1, 3)
    } else {
        1
    };
    let (sw, sh) = if landscape {
        (gw / n as f32, gh)
    } else {
        (gw, gh / n as f32)
    };
    let board = if sw >= sh * 1.2 {
        (sh * 0.94).min(sw * 0.6)
    } else {
        (sw * 0.88).min(sh * 0.6)
    };
    let cell = ((board / 8.0 / TARGET_SQ).round() as usize).max(1);
    (n, cell, landscape)
}

struct Knobs {
    think: u64,
    games: usize,
    level: u8,
    result: u64,
    fight_secs: u32,
    seed: u32,
}

pub struct Chess {
    canvas: Canvas,
    pal: Vec<u32>,
    art: Art,
    games: Vec<Game>,
    layouts: Vec<Layout>,
    /// The eval bar's shown share per game, easing toward the engine's.
    shown: Vec<i32>,
    /// What each board's fight drew last frame, for the next to restore.
    drawn: Vec<Option<Span>>,
    now: u64,
    started: bool,
}

impl Chess {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        let knobs = Knobs {
            think: env_num(&["CHESS_THINK_MS"], 1500, 100, 20_000) as u64,
            games: env_num(&["CHESS_GAMES"], 0, 0, 4) as usize,
            level: env_num(&["CHESS_LEVEL"], 0, 0, 5) as u8,
            result: env_num(&["CHESS_RESULT_SECS"], 6, 1, 60) as u64,
            fight_secs: if env_num(&["CHESS_FIGHTS"], 1, 0, 1) == 1 {
                env_num(&["CHESS_FIGHT_SECS"], 3, 1, 10) as u32
            } else {
                0
            },
            seed: saver_seed(&["CHESS_SEED"], 7),
        };
        Self::build(panel, fps, &knobs)
    }

    fn build(panel: &Panel, fps: u32, k: &Knobs) -> Self {
        let (games, level, mut seed) = (k.games, k.level, k.seed);
        let fps = u64::from(fps.max(1));
        let pace = Pace {
            glide: (fps * 9 / 20).max(2) as u32,
            book: (fps * 7 / 10).max(1),
            min_think: (fps / 2).max(1),
            result: fps * k.result,
            think: Duration::from_millis(k.think),
            fps,
            fight: fps as u32 * k.fight_secs,
        };
        let (n, cell, landscape) = plan(panel, pixel_aspect(), games);
        let grid = Grid::new(panel, cell, cell).with_ground(art::FIXED[BG as usize]);
        let (cols, rows) = (grid.cols() as i32, grid.rows() as i32);
        let layouts = (0..n as i32)
            .map(|i| {
                let slot = if landscape {
                    let (x0, x1) = (cols * i / n as i32, cols * (i + 1) / n as i32);
                    (x0, 0, x1 - x0, rows)
                } else {
                    let (y0, y1) = (rows * i / n as i32, rows * (i + 1) / n as i32);
                    (0, y0, cols, y1 - y0)
                };
                Layout::new(slot)
            })
            .collect();
        let theme0 = next_rand(&mut seed) as usize;
        let games = (0..n)
            .map(|i| {
                Game::new(
                    next_rand(&mut seed),
                    (theme0 + i) % art::THEMES.len(),
                    level,
                    pace,
                )
            })
            .collect();
        let (big, mini) = art::sprites();
        Self {
            canvas: Canvas::new(grid),
            pal: art::palette(),
            art: Art {
                big,
                mini,
                bodies: fighters::bodies(),
            },
            games,
            layouts,
            shown: vec![500; n],
            drawn: vec![None; n],
            now: 0,
            started: false,
        }
    }
}

impl Saver for Chess {
    fn render(&mut self, s: &mut Surface<'_>) {
        if !self.started {
            self.started = true;
            for g in &mut self.games {
                g.start_engine();
            }
        }
        self.now += 1;
        let now = self.now;
        for (((g, l), shown), drawn) in self
            .games
            .iter_mut()
            .zip(&self.layouts)
            .zip(&mut self.shown)
            .zip(&mut self.drawn)
        {
            g.update(now);
            let Dirty { full, board, info } = std::mem::take(&mut g.dirty);
            let score = g
                .progress()
                .map_or(g.eval, |(d, sc)| if d > 0 { sc } else { g.eval });
            let target = bar_share(score);
            let was = *shown;
            *shown += (target - *shown) / 8 + (target - *shown).signum();
            let mut p = Painter {
                c: &mut self.canvas,
                l,
                art: &self.art,
                g,
                colours: square_colours(g),
            };
            if full {
                p.clear();
                *drawn = None;
            }
            if let Some(f) = &g.fight {
                p.fight(f, drawn);
            } else if full || board {
                *drawn = None;
                p.board(full || !matches!(g.phase, Phase::Glide { t, .. } if t > 0));
            }
            if full || info {
                p.info(now);
            } else {
                p.players(now);
            }
            if full || info || was != *shown || g.progress().is_some() {
                p.eval_bar(*shown);
                p.eval_figure(score);
            }
        }
        self.canvas.flush(s, &self.pal);
    }

    fn name(&self) -> &'static str {
        "chess"
    }

    fn grid(&self) -> &Grid {
        &self.canvas.grid
    }

    fn palette(&self) -> &[u32] {
        &self.pal
    }
}
