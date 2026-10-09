//! maze-chase — an eater clearing a maze of pellets with four ghosts after
//! it, played by an autopilot, in a maze generated to fill the panel.
//!
//! # Shape
//!
//! The arcade's maze is a fixed 28x31 tiles. Here the tile size is set by the
//! panel's short side (`MAZE_TILES` tiles up it, at least 21 across) and the
//! maze is as many tiles as the panel holds, so pine's 3.2:1 glass gets a maze
//! about ninety tiles wide and portrait a tall one. Every level, and every new
//! game, generates a new maze for the same box — see `gen`.
//!
//! A tile is 3x3 cells. Walls are outlined on the cell grid with rounded
//! box-drawing glyphs, which gives the double-lined border and ghost house
//! of the original for free; actors are 6x6 bitmaps drawn in half-cell
//! quadrants, so they move six steps a tile.
//!
//! # Per-frame cost
//!
//! The walls are a static layer built once per maze. A frame copies it, adds
//! pellets and five sprites, and `Grid::flush` blits what changed. The
//! autopilot's floods run when the eater reaches a tile centre, a few times a
//! second, over buffers sized when the saver was built.

mod gen;
mod play;

use gen::{Maze, DOOR, PELLET, POWER, WALL};
use play::{Dir, GState, Game, Phase};

use crate::arcade::{self, text_w, QUADS};
use crate::font;
use crate::glyph;
use crate::grid::{bake, pixel_aspect, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, saver_seed};

const FLASH: u16 = 1;
const DOT: u16 = 2;
const EATER: u16 = 3;
const GHOST: u16 = 4;
const BLUE: u16 = 8;
const PALE: u16 = 9;
const EYES: u16 = 10;
const GATE: u16 = 11;
const TEXT: u16 = 12;
const OVER: u16 = 13;
const POINTS: u16 = 14;
const WALLS: u16 = 15;
const THEMES: u32 = 6;

#[rustfmt::skip]
const PAL_RGB: [[u8; 3]; 21] = [
    [0x00, 0x00, 0x00], [0xF0, 0xF0, 0xFF], [0xFF, 0xB8, 0x97], [0xFF, 0xE0, 0x00],
    // The four ghosts.
    [0xFF, 0x20, 0x20], [0xFF, 0xB8, 0xFF], [0x00, 0xFF, 0xFF], [0xFF, 0xB8, 0x52],
    [0x30, 0x30, 0xFF], [0xF0, 0xF0, 0xFF], [0xFF, 0xFF, 0xFF], [0xFF, 0xB8, 0xDE],
    [0xF2, 0xF2, 0xF7], [0xFF, 0x30, 0x30], [0x00, 0xFF, 0xFF],
    // Wall colour by level.
    [0x30, 0x40, 0xFF], [0xFF, 0x50, 0xB0], [0x20, 0xC8, 0xB0], [0xFF, 0x90, 0x30],
    [0xA0, 0x60, 0xFF], [0x50, 0xD0, 0x40],
];
const PAL: [u32; 21] = bake(&PAL_RGB);

/// The outline glyph for a wall cell joined to its outline neighbours up (1),
/// right (2), down (4) and left (8).
const fn outline(mask: u8) -> u16 {
    glyph::of(match mask {
        5 | 1 | 4 => '│',
        6 => '╭',
        12 => '╮',
        3 => '╰',
        9 => '╯',
        7 => '├',
        13 => '┤',
        14 => '┬',
        11 => '┴',
        15 => '┼',
        _ => '─',
    })
}

/// A ghost's body, a row per quadrant row, bit 5 the left; the skirt flips
/// every few frames.
const BODY: [u8; 5] = [0b011110, 0b111111, 0b111111, 0b111111, 0b111111];
const SKIRT: [u8; 2] = [0b101101, 0b110011];

pub struct MazeChase {
    grid: Grid,
    cols: usize,
    rows: usize,
    ox: usize,
    oy: usize,
    base: Vec<Cell>,
    game: Game,
    frame: u32,
}

impl MazeChase {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        let tiles = env_num(&["MAZE_TILES"], 28, 15, 80) as usize;
        let speed = env_num(&["MAZE_SPEED"], 95, 30, 300) as u32;
        let cut = env_num(&["MAZE_CUT"], 45, 0, 80) as usize;
        let lives = env_num(&["MAZE_LIVES"], 3, 1, 9) as u32;
        let bonus = env_num(&["MAZE_BONUS"], 10_000, 0, 1_000_000) as u64;
        let seed = saver_seed(&["MAZE_SEED"], 0x3A2E_0001);
        let aspect = pixel_aspect();
        let tall = panel.h * 100 / (aspect * 3 * (tiles + 1));
        let wide = panel.w / (3 * tiles);
        let want = if wide < tall {
            wide
        } else {
            tall.min(panel.w / 63)
        }
        .max(2);
        // A cell shorter than the glyph keeps only some of its rows, and most
        // heights under 16 drop row 7, where every outline glyph draws its
        // horizontal stroke: take the nearest cell that keeps it.
        let keeps = |c: usize| {
            let h = Grid::shape(panel, c, c, aspect).cell_h;
            (0..h).any(|py| py * font::GLYPH_H / h == 7)
        };
        let cell = (0..=4)
            .flat_map(|d| [want.saturating_sub(d), want + d])
            .find(|&c| c >= 2 && keeps(c))
            .unwrap_or(want);
        let grid = Grid::new(panel, cell, cell);
        let (cols, rows) = (grid.cols(), grid.rows());
        let maze = Maze::new(cols / 3, (rows / 3).saturating_sub(1));
        let ox = cols.saturating_sub(3 * maze.w) / 2;
        let oy = 3 + rows.saturating_sub(3 * maze.h + 3) / 2;
        Self {
            game: Game::new(maze, seed, fps, speed, cut, lives, bonus),
            base: vec![Cell::CLEAR; cols * rows],
            grid,
            cols,
            rows,
            ox,
            oy,
            frame: 0,
        }
    }

    /// Whether cell `(cx, cy)` lies in a wall tile. Off the maze counts as
    /// open, so the border is outlined on both faces.
    fn wall_cell(&self, cx: isize, cy: isize) -> bool {
        let m = &self.game.maze;
        let (x, y) = (cx - self.ox as isize, cy - self.oy as isize);
        if x < 0 || y < 0 || x >= 3 * m.w as isize || y >= 3 * m.h as isize {
            return false;
        }
        m.at(x as usize / 3, y as usize / 3) & WALL != 0
    }

    fn edge_cell(&self, cx: isize, cy: isize) -> bool {
        self.wall_cell(cx, cy)
            && (-1..=1).any(|dy| (-1..=1).any(|dx| !self.wall_cell(cx + dx, cy + dy)))
    }

    /// The walls and the gate, once per maze.
    fn build_base(&mut self) {
        let wall = WALLS + ((self.game.level - 1) % THEMES) as u16;
        for cy in 0..self.rows {
            for cx in 0..self.cols {
                let (x, y) = (cx as isize, cy as isize);
                let c = if self.edge_cell(x, y) {
                    let mask = u8::from(self.edge_cell(x, y - 1))
                        | u8::from(self.edge_cell(x + 1, y)) << 1
                        | u8::from(self.edge_cell(x, y + 1)) << 2
                        | u8::from(self.edge_cell(x - 1, y)) << 3;
                    Cell::new(outline(mask), wall)
                } else {
                    Cell::CLEAR
                };
                self.base[cy * self.cols + cx] = c;
            }
        }
        let m = &self.game.maze;
        for y in 0..m.h {
            for x in 0..m.w {
                if m.at(x, y) & DOOR != 0 {
                    for i in 0..3 {
                        let (cx, cy) = (self.ox + 3 * x + i, self.oy + 3 * y + 1);
                        self.base[cy * self.cols + cx] = Cell::new(glyph::of('─'), GATE);
                    }
                }
            }
        }
        self.game.fresh = false;
    }

    #[inline]
    fn put(&mut self, cx: usize, cy: usize, c: Cell) {
        if cx < self.cols && cy < self.rows {
            self.grid.set(cy * self.cols + cx, c);
        }
    }

    /// Light one quadrant, merging with what this colour already lit in
    /// that cell and replacing anything else.
    fn quad(&mut self, qx: isize, qy: isize, colour: u16) {
        if qx < 0 || qy < 0 {
            return;
        }
        let (cx, cy) = (qx as usize / 2, qy as usize / 2);
        if cx >= self.cols || cy >= self.rows {
            return;
        }
        let i = cy * self.cols + cx;
        let cur = self.grid.cell(i);
        let mut bits = 0;
        if cur.colour() == colour as usize {
            if let Some(b) = QUADS.iter().position(|&g| g as usize == cur.glyph()) {
                bits = b;
            }
        }
        bits |= 1 << ((qx & 1) + 2 * (qy & 1));
        self.grid.set(i, Cell::new(QUADS[bits], colour));
    }

    /// A 6x6 bitmap centred on `(x, y)` in sub-tile units, and again a maze
    /// width away so the tunnel shows it leaving one side and entering the
    /// other; clipped to the maze.
    fn sprite(&mut self, x: i32, y: i32, rows: [u8; 6], colour: u16) {
        let mw = 6 * self.game.maze.w as isize;
        let (lo, hi) = (2 * self.ox as isize, 2 * self.ox as isize + mw);
        let qx0 = lo + (x / 2) as isize - 3;
        let qy0 = 2 * self.oy as isize + (y / 2) as isize - 3;
        for shift in [-mw, 0, mw] {
            for (r, &bits) in rows.iter().enumerate() {
                for c in 0..6 {
                    let qx = qx0 + shift + c as isize;
                    if bits & (0x20 >> c) != 0 && qx >= lo && qx < hi {
                        self.quad(qx, qy0 + r as isize, colour);
                    }
                }
            }
        }
    }

    fn text(&mut self, qx: isize, qy: isize, s: &[u8], colour: u16) {
        arcade::each_pixel(s, |x, y| {
            self.quad(qx + x as isize, qy + y as isize, colour);
        });
    }

    fn number(&mut self, qx: isize, qy: isize, v: u64, colour: u16, centre: bool) {
        let mut buf = [0u8; 10];
        let d = arcade::decimal(v, &mut buf, 8);
        let x = if centre {
            qx - text_w(d.len()) as isize / 2
        } else {
            qx
        };
        self.text(x, qy, d, colour);
    }

    fn draw_pellets(&mut self) {
        let blink = matches!(self.game.phase, Phase::Play) && (self.frame / 6) % 2 == 1;
        let (w, h) = (self.game.maze.w, self.game.maze.h);
        let dot = Cell::new(glyph::of('·'), DOT);
        for y in 0..h {
            for x in 0..w {
                let t = self.game.maze.at(x, y);
                let (cx, cy) = (self.ox + 3 * x + 1, self.oy + 3 * y + 1);
                if t & PELLET != 0 {
                    self.put(cx, cy, dot);
                } else if t & POWER != 0 && !blink {
                    self.put(cx, cy, Cell::new(font::SOLID, DOT));
                    self.put(cx - 1, cy, Cell::new(glyph::of('▐'), DOT));
                    self.put(cx + 1, cy, Cell::new(glyph::of('▌'), DOT));
                    self.put(cx, cy - 1, Cell::new(font::LOWER, DOT));
                    self.put(cx, cy + 1, Cell::new(font::UPPER, DOT));
                }
            }
        }
    }

    fn draw_ghosts(&mut self) {
        let g = &self.game;
        let fps = g.fps;
        let pale = g.fright < 2 * fps && (g.fright / (fps / 5).max(1)) % 2 == 1;
        let skirt = SKIRT[(self.frame / 4) as usize % 2];
        let ghosts = g.ghosts;
        for (k, gh) in ghosts.iter().enumerate() {
            let (ex, ey) = match gh.a.dir {
                Dir::Left => (0, 2),
                Dir::Right => (2, 2),
                Dir::Up => (1, 1),
                Dir::Down => (1, 3),
            };
            let eyes = (0x20 >> ex) | (0x20 >> (ex + 3));
            let mut rows = [BODY[0], BODY[1], BODY[2], BODY[3], BODY[4], skirt];
            if matches!(gh.state, GState::Eyes | GState::Enter) {
                let mut only = [0u8; 6];
                only[ey] = eyes;
                only[ey + 1] = eyes;
                self.sprite(gh.a.x, gh.a.y, only, EYES);
                continue;
            }
            rows[ey] &= !eyes;
            let colour = match (gh.fright, pale) {
                (false, _) => GHOST + k as u16,
                (true, false) => BLUE,
                (true, true) => PALE,
            };
            self.sprite(gh.a.x, gh.a.y, rows, colour);
        }
    }

    fn draw_hud(&mut self) {
        let m_w = self.game.maze.w;
        let qy = 2 * (self.oy as isize - 3);
        let lo = 2 * self.ox as isize;
        let score = self.game.score;
        self.number(lo + 2, qy, score, TEXT, false);
        let mut buf = [0u8; 10];
        let lv = arcade::decimal(u64::from(self.game.level), &mut buf, 3);
        let mut label = *b"LEVEL     ";
        label[6..6 + lv.len()].copy_from_slice(lv);
        let n = 6 + lv.len();
        let mid = lo + 3 * m_w as isize;
        self.text(mid - text_w(n) as isize / 2, qy, &label[..n], TEXT);
        let icon = eater(Dir::Left, 0.7);
        for i in 0..self.game.lives.saturating_sub(1) as i32 {
            self.sprite(2 * (6 * m_w as i32 - 4 - 8 * i), -6, icon, EATER);
        }
    }
}

/// The eater as a 6x6 disc with a wedge of half-angle `mouth` cut out
/// facing `dir`; at a half-angle of pi it is gone.
fn eater(dir: Dir, mouth: f32) -> [u8; 6] {
    let (ux, uy) = (dir.dx() as f32, dir.dy() as f32);
    let cut = mouth.cos();
    let mut rows = [0u8; 6];
    for (y, row) in rows.iter_mut().enumerate() {
        for x in 0..6 {
            let (dx, dy) = (x as f32 - 2.5, y as f32 - 2.5);
            let r = (dx * dx + dy * dy).sqrt();
            if r > 3.0 || (mouth > 0.0 && (dx * ux + dy * uy) / r > cut) {
                continue;
            }
            *row |= 0x20 >> x;
        }
    }
    rows
}

impl Saver for MazeChase {
    fn render(&mut self, s: &mut Surface<'_>) {
        self.frame = self.frame.wrapping_add(1);
        self.game.step();
        if self.game.fresh {
            self.build_base();
        }
        let flash = match self.game.phase {
            Phase::Clear(t) => (t / (self.game.fps / 4).max(1)) % 2 == 1,
            _ => false,
        };
        for i in 0..self.cols * self.rows {
            let c = self.base[i];
            let c = if flash && c.colour() >= WALLS as usize {
                Cell::new(c.glyph() as u16, FLASH)
            } else {
                c
            };
            self.grid.set(i, c);
        }
        self.draw_pellets();
        let phase = self.game.phase;
        let fps = self.game.fps;
        let show_ghosts = match phase {
            Phase::Dying(t) => t < fps,
            Phase::Clear(_) | Phase::Over(_) => false,
            _ => true,
        };
        if show_ghosts {
            self.draw_ghosts();
        }
        let pac = self.game.pac;
        let rows = match phase {
            Phase::Dying(t) if t >= fps => {
                let p = (t - fps) as f32 / (fps * 3 / 2) as f32;
                Some(eater(Dir::Up, 0.3 + p * std::f32::consts::PI))
            }
            Phase::Freeze(_) | Phase::Over(_) => None,
            Phase::Ready(_) => Some(eater(pac.dir, 0.0)),
            _ => {
                const OPEN: [f32; 8] = [0.0, 0.35, 0.7, 1.0, 1.0, 0.7, 0.35, 0.0];
                Some(eater(pac.dir, OPEN[(self.game.chomp % 8) as usize]))
            }
        };
        if let Some(r) = rows {
            self.sprite(pac.x, pac.y, r, EATER);
        }
        self.draw_hud();
        let (sx, sy) = self.game.maze.start;
        let (qx, qy) = (
            2 * self.ox as isize + 6 * sx as isize + 6,
            2 * (self.oy + 3 * sy) as isize,
        );
        match phase {
            Phase::Ready(_) => self.text(qx - text_w(5) as isize / 2, qy, b"READY", EATER),
            Phase::Over(_) => self.text(qx - text_w(9) as isize / 2, qy, b"GAME OVER", OVER),
            Phase::Freeze(_) => {
                let (x, y, pts) = self.game.popup;
                let at = (
                    2 * self.ox as isize + (x / 2) as isize,
                    2 * self.oy as isize + (y / 2) as isize - 2,
                );
                self.number(at.0, at.1, pts, POINTS, true);
            }
            _ => {}
        }
        self.grid.flush(s, &PAL);
    }

    fn name(&self) -> &'static str {
        "maze-chase"
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

    const SHAPES: [(usize, usize, usize); 6] = [
        (1920, 1080, 180),
        (1920, 1080, 100),
        (1024, 768, 100),
        (1080, 1080, 100),
        (1080, 1920, 100),
        (128, 128, 100),
    ];

    fn build(w: usize, h: usize, aspect: usize) -> (Panel, MazeChase) {
        let p = Panel::new(w, h, w);
        let m = with_test_aspect(aspect, || MazeChase::new(&p, 30));
        (p, m)
    }

    /// The maze is the panel's shape: it spans nearly all of both axes, so
    /// pine's is about three times as wide as it is tall and portrait's
    /// taller than wide.
    #[test]
    fn the_maze_fills_the_panel_at_any_shape() {
        for (w, h, aspect) in SHAPES {
            let (p, mut m) = build(w, h, aspect);
            let mut buf = vec![0u32; p.buf_len()];
            saver::frame(&mut m, &mut buf, &p);
            let at = format!("{w}x{h}@{aspect}");
            let (mw, mh) = (m.game.maze.w, m.game.maze.h);
            if w >= 200 {
                assert!(
                    3 * mw * 100 >= m.cols * 94,
                    "{at}: {mw} tiles across {} cols",
                    m.cols
                );
                assert!(
                    3 * mh * 100 >= (m.rows - 3) * 90,
                    "{at}: {mh} tiles down {} rows",
                    m.rows
                );
            }
            let glass =
                (mw * m.grid.cell_w()) as f32 / (mh * m.grid.cell_h() * 100 / aspect) as f32;
            let panel = w as f32 / (h * 100 / aspect) as f32;
            assert!(
                (glass / panel - 1.0).abs() < 0.25 || w < 200,
                "{at}: maze {glass:.2} on a {panel:.2} panel"
            );
        }
        let (_, pine) = build(1920, 1080, 180);
        for (w, h, aspect) in SHAPES {
            let (_, m) = build(w, h, aspect);
            let ch = m.grid.cell_h();
            assert!(
                w < 200 || (0..ch).any(|py| py * font::GLYPH_H / ch == 7),
                "{w}x{h}: a {ch}px cell loses the outline's stroke"
            );
        }
        assert!(
            pine.game.maze.w >= 80,
            "pine maze only {} wide",
            pine.game.maze.w
        );
    }

    #[test]
    fn damage_covers_every_changed_pixel() {
        for (w, h, aspect) in SHAPES {
            let (p, mut m) = build(w, h, aspect);
            let mut buf = vec![0u32; p.buf_len()];
            let mut prev = buf.clone();
            for n in 0..600 {
                prev.copy_from_slice(&buf);
                let d = saver::frame(&mut m, &mut buf, &p);
                dump::verify(&prev, &buf, &d, &p, n).unwrap();
            }
        }
    }

    /// Steady play, deaths, level clears and new mazes, all without an
    /// allocation: the small square panel gets through levels quickly.
    #[test]
    fn render_never_allocates() {
        let (p, mut m) = build(640, 400, 100);
        let mut buf = vec![0u32; p.buf_len()];
        saver::frame(&mut m, &mut buf, &p);
        let n = allocs_during(|| {
            for _ in 0..40_000 {
                saver::frame(&mut m, &mut buf, &p);
            }
        });
        assert_eq!(n, 0, "render allocated");
        let g = &m.game;
        assert!(g.cleared > 0, "no level cleared");
        assert!(g.deaths > 0 || g.eaten > 0, "nothing happened");
    }

    /// The autopilot clears levels: an hour of play on the 16:9 maze gets
    /// through several; bonus lives make up for the deaths along the way.
    #[test]
    fn the_pilot_clears_levels() {
        let (_, mut m) = build(1920, 1080, 100);
        let g = &mut m.game;
        let per_level = g.left;
        let (mut eaten, mut left) = (0, g.left);
        for _ in 0..108_000 {
            g.step();
            eaten += left.saturating_sub(g.left);
            left = g.left;
        }
        eprintln!(
            "maze-chase {}x{} tiles: {} levels cleared, {} deaths, {} ghosts eaten, {} pellets a life",
            g.maze.w, g.maze.h, g.cleared, g.deaths, g.eaten, eaten / g.deaths.max(1) as usize
        );
        assert!(g.cleared >= 6, "{} levels in an hour", g.cleared);
        assert!(eaten / g.deaths.max(1) as usize >= per_level / 4);
    }
}
