//! tetris — falling-block games played by an AI, as many wells as the panel
//! has room for.
//!
//! # Shape
//!
//! A well is 10x20 and its blocks must land square on the glass, so its
//! height sets the block size and its width is fixed by that. On pine's 3.2:1
//! glass one well is a sixth of the panel. Widening the well would change the
//! game; stretching the blocks would make them bricks. So the panel gets as
//! many independent games side by side (or stacked, in portrait) as fit, at the
//! largest block size that fits that many: `layout` scores every arrangement
//! by the share of the panel it covers, leaning towards bigger blocks, and
//! picks four wells on pine, two on 16:9 and 4:3, one on a square.
//!
//! # Per-frame cost
//!
//! Every cell is rewritten each frame and `Grid::flush` blits only the ones
//! that changed: a falling piece, its ghost and a score. The AI runs once per
//! piece, never in a frame where nothing spawns.

mod game;

use game::{Game, Phase, Timing, H, HIDDEN, SHAPES, W};

use crate::arcade::{self, text_w};
use crate::font;
use crate::grid::{bake, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, saver_seed};

const BASE: u16 = 1;
const LIGHT: u16 = 8;
const DIM: u16 = 15;
const FLASH: u16 = 22;
const LABEL: u16 = 23;
const DIGIT: u16 = 24;
const THEME: u16 = 25;
const THEMES: u16 = 10;
const CURTAIN: u16 = 35;

#[rustfmt::skip]
const PAL_RGB: [[u8; 3]; 36] = [
    [0x00, 0x00, 0x00],
    // I O T S Z J L
    [0x00, 0xD8, 0xF0], [0xF0, 0xD0, 0x00], [0xA8, 0x40, 0xF0], [0x30, 0xD0, 0x40],
    [0xF0, 0x30, 0x30], [0x30, 0x60, 0xF0], [0xF0, 0x90, 0x20],
    [0x9A, 0xF6, 0xFF], [0xFF, 0xF0, 0x90], [0xD8, 0x9C, 0xFF], [0x9C, 0xF5, 0xA5],
    [0xFF, 0x9C, 0x9C], [0x9C, 0xB5, 0xFF], [0xFF, 0xCE, 0x8E],
    [0x00, 0x5A, 0x64], [0x64, 0x57, 0x00], [0x46, 0x1B, 0x64], [0x14, 0x57, 0x1B],
    [0x64, 0x14, 0x14], [0x1B, 0x30, 0x78], [0x64, 0x3C, 0x0E],
    [0xFF, 0xFF, 0xFF], [0x80, 0x80, 0x90], [0xF2, 0xF2, 0xF7],
    // A frame colour per level, cycling every ten.
    [0x40, 0x80, 0xFF], [0xFF, 0x50, 0xA0], [0x50, 0xE0, 0x90], [0xFF, 0xA0, 0x30],
    [0xA0, 0x70, 0xFF], [0x30, 0xD0, 0xE0], [0xFF, 0x60, 0x50], [0xC0, 0xE0, 0x40],
    [0xE0, 0x60, 0xE0], [0x70, 0xA0, 0xC0],
    [0x50, 0x50, 0x60],
];
const PAL: [u32; 36] = bake(&PAL_RGB);

const MAX_BOARDS: usize = 8;

/// The 3x5 font's scale at block size `n`: a digit about a block tall.
fn font_scale(n: usize) -> usize {
    ((n + 2) / 5).max(1)
}

/// Columns the sidebar takes: the next piece, or the widest label.
fn sidebar_w(n: usize) -> usize {
    (4 * n).max(text_w(5) * font_scale(n))
}

/// Cells one game takes at block size `n`: the framed well, a gap, the
/// sidebar; the score above.
fn footprint(n: usize) -> (usize, usize) {
    let s = font_scale(n);
    (10 * n + 4 + sidebar_w(n), 6 * s + 20 * n + 2)
}

/// Block size, wells across and down, and the gap between wells, for a grid
/// of `cols x rows` cells. `forced` pins the count across.
fn layout(cols: usize, rows: usize, forced: usize) -> (usize, usize, usize, usize) {
    let fit = |bx: usize, by: usize| {
        (1..=64)
            .rev()
            .find(|&n| {
                let (w, h) = footprint(n);
                let gap = n.max(2);
                bx * w + (bx - 1) * gap <= cols && by * h + (by - 1) * gap <= rows
            })
            .unwrap_or(1)
    };
    if forced > 0 {
        let n = fit(forced, 1);
        return (n, forced, 1, n.max(2));
    }
    let mut best = (0.0, 1, 1, 1);
    for by in 1..=3 {
        for bx in 1..=MAX_BOARDS / by {
            let n = fit(bx, by);
            let (w, h) = footprint(n);
            let cover = (bx * by * w * h) as f32 / (cols * rows) as f32;
            if cover > 1.0 {
                continue;
            }
            let score = cover * (n as f32).sqrt();
            if score > best.0 {
                best = (score, n, bx, by);
            }
        }
    }
    let (_, n, bx, by) = best;
    (n, bx, by, n.max(2))
}

struct Well {
    x: usize,
    y: usize,
    game: Game,
}

pub struct Tetris {
    grid: Grid,
    cols: usize,
    rows: usize,
    n: usize,
    s: usize,
    wells: Vec<Well>,
}

impl Tetris {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        let cell = env_num(&["TETRIS_CELL"], 8, 4, 64) as usize;
        let forced = env_num(&["TETRIS_WELLS"], 0, 0, MAX_BOARDS as i64) as usize;
        let start = env_num(&["TETRIS_LEVEL"], 0, 0, 19) as u32;
        let moves = env_num(&["TETRIS_MOVES"], 15, 2, 60) as u32;
        let lookahead = env_num(&["TETRIS_LOOKAHEAD"], 1, 0, 1) == 1;
        let seed = saver_seed(&["TETRIS_SEED"], 0x7E7_0001);
        let grid = Grid::new(panel, cell, cell);
        let (cols, rows) = (grid.cols(), grid.rows());
        let (n, bx, by, gap) = layout(cols, rows, forced);
        let (w, h) = footprint(n);
        let x0 = cols.saturating_sub(bx * w + (bx - 1) * gap) / 2;
        let y0 = rows.saturating_sub(by * h + (by - 1) * gap) / 2;
        let t = Timing::new(fps, moves);
        let mut rng = seed;
        let mut wells = Vec::with_capacity(bx * by);
        for j in 0..by {
            for i in 0..bx {
                let game = Game::new(crate::next_rand(&mut rng) | 1, start, lookahead, t);
                wells.push(Well {
                    x: x0 + i * (w + gap),
                    y: y0 + j * (h + gap),
                    game,
                });
            }
        }
        Self {
            grid,
            cols,
            rows,
            n,
            s: font_scale(n),
            wells,
        }
    }

    #[inline]
    fn put(&mut self, x: usize, y: usize, c: Cell) {
        if x < self.cols && y < self.rows {
            self.grid.set(y * self.cols + x, c);
        }
    }

    /// One block at cell `(x, y)`: `n x n` cells whose last row and column
    /// are half-lit, so neighbouring blocks read as separate tiles.
    fn block(&mut self, x: usize, y: usize, base: u16, light: u16, fill: u16) {
        let n = self.n;
        for j in 0..n {
            for i in 0..n {
                let g = match (n > 1 && i == n - 1, n > 1 && j == n - 1) {
                    (true, true) => crate::glyph::of('▘'),
                    (true, false) => crate::glyph::of('▌'),
                    (false, true) => font::UPPER,
                    (false, false) => fill,
                };
                let c = if j == 0 && n >= 4 { light } else { base };
                self.put(x + i, y + j, Cell::new(g, c));
            }
        }
    }

    fn label(&mut self, x: usize, y: usize, text: &[u8], colour: u16) {
        let sc = self.s;
        let c = Cell::new(font::SOLID, colour);
        for (i, &ch) in text.iter().enumerate() {
            for (py, row) in arcade::glyph3(ch).iter().enumerate() {
                for px in 0..3 {
                    if row & (4 >> px) == 0 {
                        continue;
                    }
                    for dy in 0..sc {
                        for dx in 0..sc {
                            self.put(x + (i * 4 + px) * sc + dx, y + py * sc + dy, c);
                        }
                    }
                }
            }
        }
    }

    fn number(&mut self, x: usize, y: usize, w: usize, v: u64, colour: u16) {
        let s = self.s;
        let mut buf = [0u8; 10];
        let digits = arcade::decimal(v, &mut buf, (w + s) / (4 * s));
        let tw = text_w(digits.len()) * s;
        self.label(x + w.saturating_sub(tw) / 2, y, digits, colour);
    }

    fn draw_well(&mut self, k: usize) {
        let (n, s) = (self.n, self.s);
        let Well { x: ox, y: oy, .. } = self.wells[k];
        let g = &self.wells[k].game;
        let (phase, level, lines, score, next) = (g.phase, g.level, g.lines, g.score, g.next);
        let fy = oy + 6 * s;
        let (ix, iy) = (ox + 1, fy + 1);
        let four = matches!(phase, Phase::Clear { mask, t } if mask.count_ones() == 4 && t % 4 < 2);
        let frame = if four {
            FLASH
        } else {
            THEME + (level % u32::from(THEMES)) as u16
        };
        self.number(ix, oy, 10 * n, score, DIGIT);
        let (fw, fh) = (10 * n + 2, 20 * n + 2);
        let edge = |x: usize, y: usize| match (x, y) {
            (0, 0) => '▗',
            (x, 0) if x == fw - 1 => '▖',
            (0, y) if y == fh - 1 => '▝',
            (x, y) if x == fw - 1 && y == fh - 1 => '▘',
            (_, 0) => '▄',
            (_, y) if y == fh - 1 => '▀',
            (0, _) => '▐',
            _ => '▌',
        };
        for y in 0..fh {
            for x in 0..fw {
                if x == 0 || y == 0 || x == fw - 1 || y == fh - 1 {
                    let c = Cell::new(crate::glyph::of(edge(x, y)), frame);
                    self.put(ox + x, fy + y, c);
                }
            }
        }

        let (clear_mask, wipe) = match phase {
            Phase::Clear { mask, t } => (mask, (t * 5 / self.wells[k].game.t.clear.max(1)) + 1),
            _ => (0, 0),
        };
        for y in HIDDEN..H {
            let vr = y - HIDDEN;
            for x in 0..W {
                let kind = self.wells[k].game.colour[y * W + x];
                if kind == 0 {
                    continue;
                }
                let (bx, by) = (ix + x * n, iy + vr * n);
                if clear_mask & (1 << y) != 0 {
                    if (x.abs_diff(4) + usize::from(x < 5)) as u32 <= wipe {
                        continue;
                    }
                    self.block(bx, by, FLASH, FLASH, font::SOLID);
                } else {
                    let k = u16::from(kind - 1);
                    self.block(bx, by, BASE + k, LIGHT + k, font::SOLID);
                }
            }
        }

        if phase == Phase::Fall {
            let g = &self.wells[k].game;
            let (p, gy) = (g.cur, g.ghost_y());
            let kind = p.kind as u16;
            for (yy, fill, base, light) in [
                (gy, font::SHADE, DIM + kind, DIM + kind),
                (p.y, font::SOLID, BASE + kind, LIGHT + kind),
            ] {
                for (r, &m) in SHAPES[p.kind][p.rot].iter().enumerate() {
                    for c in 0..4 {
                        let (cx, cy) = (p.x + c, yy + r as i32);
                        if m & (1 << c) == 0 || cy < HIDDEN as i32 || cx < 0 {
                            continue;
                        }
                        let vr = cy as usize - HIDDEN;
                        self.block(ix + cx as usize * n, iy + vr * n, base, light, fill);
                    }
                }
            }
        }

        if let Phase::Over(t) = phase {
            let curtain = self.wells[k].game.t.curtain.max(1);
            let down = (t as usize * 20 / curtain as usize).min(20);
            for vr in 0..down {
                for x in 0..W {
                    self.block(ix + x * n, iy + vr * n, CURTAIN, CURTAIN, font::SOLID);
                }
            }
        }

        let sw = sidebar_w(n);
        let sx = ox + 10 * n + 4;
        let mut y = iy;
        self.label(sx + (sw - text_w(4) * s) / 2, y, b"NEXT", LABEL);
        y += 6 * s;
        let m = &SHAPES[next][0];
        let lo = m.iter().map(|r| r.trailing_zeros()).min().unwrap_or(0) as usize;
        let hi = m.iter().map(|r| 8 - r.leading_zeros()).max().unwrap_or(0) as usize;
        let top = m.iter().position(|&r| r != 0).unwrap_or(0);
        let px = sx + (sw - (hi - lo) * n) / 2;
        let nk = next as u16;
        for (r, &row) in m.iter().enumerate().skip(top) {
            for c in lo..hi {
                if row & (1 << c) != 0 {
                    self.block(
                        px + (c - lo) * n,
                        y + (r - top) * n,
                        BASE + nk,
                        LIGHT + nk,
                        font::SOLID,
                    );
                }
            }
        }
        y += 2 * n + 2 * s;
        self.label(sx + (sw - text_w(5) * s) / 2, y, b"LEVEL", LABEL);
        y += 6 * s;
        self.number(sx, y, sw, u64::from(level), DIGIT);
        y += 7 * s;
        self.label(sx + (sw - text_w(5) * s) / 2, y, b"LINES", LABEL);
        y += 6 * s;
        self.number(sx, y, sw, u64::from(lines), DIGIT);
    }
}

impl Saver for Tetris {
    fn render(&mut self, s: &mut Surface<'_>) {
        for w in &mut self.wells {
            w.game.step();
        }
        for i in 0..self.cols * self.rows {
            self.grid.set(i, Cell::CLEAR);
        }
        for k in 0..self.wells.len() {
            self.draw_well(k);
        }
        self.grid.flush(s, &PAL);
    }

    fn name(&self) -> &'static str {
        "tetris"
    }

    fn grid(&self) -> &Grid {
        &self.grid
    }

    fn palette(&self) -> &[u32] {
        &PAL
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dump;
    use crate::grid::with_test_aspect;
    use crate::saver;
    use crate::testalloc::allocs_during;

    const SHAPES_UNDER_TEST: [(usize, usize, usize, usize); 6] = [
        (1920, 1080, 180, 4),
        (1920, 1080, 100, 2),
        (1024, 768, 100, 2),
        (1080, 1080, 100, 1),
        (1080, 1920, 100, 1),
        (128, 128, 100, 1),
    ];

    fn build(w: usize, h: usize, aspect: usize) -> (Panel, Tetris) {
        let p = Panel::new(w, h, w);
        let t = with_test_aspect(aspect, || Tetris::new(&p, 30));
        (p, t)
    }

    /// Pine gets four wells, 16:9 and 4:3 two, square and portrait one, and
    /// whatever the count, the games span the panel rather than huddling in
    /// its middle.
    #[test]
    fn the_panel_shape_picks_the_number_of_wells() {
        for (w, h, aspect, want) in SHAPES_UNDER_TEST {
            let (p, mut t) = build(w, h, aspect);
            let at = format!("{w}x{h}@{aspect}");
            assert_eq!(t.wells.len(), want, "{at}: wells");
            let mut buf = vec![0u32; p.buf_len()];
            saver::frame(&mut t, &mut buf, &p);
            let (fw, fh) = footprint(t.n);
            for well in &t.wells {
                assert!(well.x + fw <= t.cols || w < 200, "{at}: off the right");
                assert!(well.y + fh <= t.rows || w < 200, "{at}: off the bottom");
            }
            if w >= 200 {
                let lit: Vec<usize> = (0..t.cols)
                    .filter(|&x| (0..t.rows).any(|y| t.grid.cells()[y * t.cols + x] != Cell::CLEAR))
                    .collect();
                let span = lit.last().unwrap() - lit[0] + 1;
                let short = if w >= h { t.cols } else { t.rows };
                assert!(
                    span * 100 >= t.cols * 60,
                    "{at}: {span} of {} columns",
                    t.cols
                );
                let used = (0..t.rows)
                    .filter(|&y| (0..t.cols).any(|x| t.grid.cells()[y * t.cols + x] != Cell::CLEAR))
                    .count();
                assert!(
                    used * 100 >= t.rows * 55 || short == t.rows,
                    "{at}: {used} rows"
                );
            }
        }
    }

    /// Pine's 3.2:1 framebuffer at 180 % aspect and the panel's real 1280x400:
    /// the blocks are square on the glass in both.
    #[test]
    fn blocks_land_square_on_the_glass() {
        let (_, t) = build(1920, 1080, 180);
        let glass_h = t.grid.cell_h() * 100 / 180;
        assert!(t.grid.cell_w().abs_diff(glass_h) <= 1);
        assert!(t.n >= 3, "blocks of {} cells", t.n);
    }

    #[test]
    fn damage_covers_every_changed_pixel() {
        for (w, h, aspect, _) in SHAPES_UNDER_TEST {
            let (p, mut t) = build(w, h, aspect);
            let mut buf = vec![0u32; p.buf_len()];
            let mut prev = buf.clone();
            for n in 0..900 {
                prev.copy_from_slice(&buf);
                let d = saver::frame(&mut t, &mut buf, &p);
                dump::verify(&prev, &buf, &d, &p, n).unwrap();
            }
        }
    }

    /// Long enough for pieces to spawn, lock and clear rows.
    #[test]
    fn render_never_allocates() {
        let (p, mut t) = build(1920, 1080, 180);
        let mut buf = vec![0u32; p.buf_len()];
        saver::frame(&mut t, &mut buf, &p);
        let n = allocs_during(|| {
            for _ in 0..3000 {
                saver::frame(&mut t, &mut buf, &p);
            }
        });
        assert_eq!(n, 0, "render allocated");
        assert!(
            t.wells.iter().any(|w| w.game.lines > 0),
            "no row was cleared"
        );
    }

    /// The AI plays well: games last hundreds of lines and end only where
    /// gravity outruns its hands, around the kill screen.
    #[test]
    fn the_ai_clears_hundreds_of_lines_a_game() {
        let mut g = Game::new(0xBEEF, 0, true, Timing::new(30, 15));
        let mut games = Vec::new();
        let (mut frames, mut top) = (0u64, 0);
        while games.len() < 3 && frames < 3_000_000 {
            let before = g.games;
            top = top.max(g.level);
            g.step();
            frames += 1;
            if g.games > before {
                games.push(g.last_lines);
            }
        }
        eprintln!(
            "tetris AI: lines per game {games:?}, top level {top}, {} min at 30 fps",
            frames / 1800
        );
        assert_eq!(games.len(), 3, "games never end");
        let mean = games.iter().sum::<u32>() / 3;
        assert!(mean >= 150, "mean {mean} lines a game");
    }
}
