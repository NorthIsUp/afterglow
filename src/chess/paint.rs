//! Drawing: a pixel canvas over the grid that records exactly which cells
//! changed, and the board, eval bar and side panel drawn onto it.

use super::art::{
    self, glyph_row, ink, mini_ink, shadow, square, Lit, Mini, Sprite, ACCENT, ADVANCE, BANNER, BG,
    DIM, GOOD, PANEL, RULE, SPRITE, TEXT, TEXT_H,
};
use super::game::{Game, Phase};
use super::rules::{kind, side, Move, CASTLE};
use super::search::MATE;
use super::Layout;
use crate::font;
use crate::grid::{Cell, Grid};
use crate::surface::Surface;

/// Every logical pixel is one SOLID cell. Writes land in the grid's next
/// frame at once; `dirty` lists each changed cell once, and `flush` drops
/// those a later write put back the way the panel already shows them.
pub struct Canvas {
    pub grid: Grid,
    dirty: Vec<u32>,
    marked: Vec<bool>,
    clip: (i32, i32, i32, i32),
}

impl Canvas {
    pub fn new(grid: Grid) -> Self {
        let n = grid.cols() * grid.rows();
        let clip = (0, 0, grid.cols() as i32, grid.rows() as i32);
        Self {
            grid,
            dirty: Vec::with_capacity(n),
            marked: vec![false; n],
            clip,
        }
    }

    pub fn unclip(&mut self) {
        self.clip = (0, 0, self.grid.cols() as i32, self.grid.rows() as i32);
    }

    pub fn clip_to(&mut self, x: i32, y: i32, w: i32, h: i32) {
        let (c, r) = (self.grid.cols() as i32, self.grid.rows() as i32);
        self.clip = (x.max(0), y.max(0), (x + w).min(c), (y + h).min(r));
    }

    #[inline]
    pub fn put(&mut self, x: i32, y: i32, c: u16) {
        let (x0, y0, x1, y1) = self.clip;
        if x < x0 || y < y0 || x >= x1 || y >= y1 {
            return;
        }
        let i = y as usize * self.grid.cols() + x as usize;
        let cell = Cell::new(font::SOLID, c);
        if self.grid.cell(i) != cell {
            self.grid.set(i, cell);
            if !self.marked[i] {
                self.marked[i] = true;
                self.dirty.push(i as u32);
            }
        }
    }

    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: u16) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.put(xx, yy, c);
            }
        }
    }

    /// `s` at `scale`, top-left at `x, y`; returns the width drawn.
    pub fn text(&mut self, x: i32, y: i32, s: &[u8], c: u16, scale: i32) -> i32 {
        for (n, &ch) in s.iter().enumerate() {
            let gx = x + (n * ADVANCE) as i32 * scale;
            for gy in 0..TEXT_H {
                let bits = glyph_row(ch, gy);
                for bx in 0..ADVANCE {
                    if bits & (0x80 >> bx) != 0 {
                        let (px, py) = (gx + bx as i32 * scale, y + gy as i32 * scale);
                        self.rect(px, py, scale, scale, c);
                    }
                }
            }
        }
        (s.len() * ADVANCE) as i32 * scale
    }

    pub fn flush(&mut self, s: &mut Surface<'_>, pal: &[u32]) {
        let Self {
            grid,
            dirty,
            marked,
            ..
        } = self;
        for &i in dirty.iter() {
            marked[i as usize] = false;
        }
        let drawn = grid.cells();
        let mut keep = 0;
        for j in 0..dirty.len() {
            let i = dirty[j] as usize;
            if grid.cell(i) != drawn[i] {
                dirty[keep] = dirty[j];
                keep += 1;
            }
        }
        dirty.truncate(keep);
        grid.flush_sparse(s, pal, dirty);
        dirty.clear();
    }
}

/// Glyphs for a number, into `buf`; returns the used tail.
pub fn digits(mut n: u32, buf: &mut [u8; 10]) -> &[u8] {
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 || i == 0 {
            break;
        }
    }
    &buf[i..]
}

/// `+1.25`, `-0.40`, `#3`, `-#2` for White's score.
fn eval_text(score: i32, buf: &mut [u8; 12]) -> &[u8] {
    let mut n = 0;
    let mut push = |b: u8, n: &mut usize| {
        buf[*n] = b;
        *n += 1;
    };
    if score.abs() > MATE - 1000 {
        if score < 0 {
            push(b'-', &mut n);
        }
        push(b'#', &mut n);
        if score.abs() >= MATE {
            return &buf[..n];
        }
        let moves = ((MATE - score.abs()) as u32).div_ceil(2).max(1);
        let mut d = [0u8; 10];
        for &c in digits(moves, &mut d) {
            push(c, &mut n);
        }
    } else {
        push(if score < 0 { b'-' } else { b'+' }, &mut n);
        let cp = score.unsigned_abs().min(9999);
        let mut d = [0u8; 10];
        for &c in digits(cp / 100, &mut d) {
            push(c, &mut n);
        }
        push(b'.', &mut n);
        push(b'0' + (cp / 10 % 10) as u8, &mut n);
        push(b'0' + (cp % 10) as u8, &mut n);
    }
    &buf[..n]
}

/// White's share of the eval bar, 0..=1000.
pub fn bar_share(score: i32) -> i32 {
    if score.abs() > MATE - 1000 {
        return if score > 0 { 1000 } else { 0 };
    }
    let x = score.clamp(-2000, 2000) as f32 / 300.0;
    (1000.0 / (1.0 + (-x).exp())) as i32
}

pub struct Art {
    pub big: [Sprite; 6],
    pub mini: [Mini; 6],
}

impl Layout {
    fn square_xy(&self, s: u8) -> (i32, i32) {
        let (f, r) = (i32::from(s & 7), i32::from(s >> 3));
        (self.bx + f * self.sq, self.by + (7 - r) * self.sq)
    }

    /// Top-left of a sprite standing on square `s`.
    fn sprite_xy(&self, s: u8) -> (i32, i32) {
        let (x, y) = self.square_xy(s);
        let p = SPRITE as i32 * self.k;
        (x + (self.sq - p) / 2, y + (self.sq - p + 1) / 2)
    }

    fn square_at(&self, x: i32, y: i32) -> Option<u8> {
        let (f, r) = (
            (x - self.bx).div_euclid(self.sq),
            (y - self.by).div_euclid(self.sq),
        );
        ((0..8).contains(&f) && (0..8).contains(&r)).then(|| ((7 - r) * 8 + f) as u8)
    }
}

/// Every square's colour this frame: the check test is a scan of the board,
/// too dear to repeat per shadow pixel.
pub fn square_colours(g: &Game) -> [u16; 64] {
    let gliding = matches!(g.phase, Phase::Glide { .. });
    let check = (!gliding && g.pos.in_check()).then_some(g.pos.kings[g.pos.stm]);
    std::array::from_fn(|i| {
        let s = i as u8;
        let lit = if check == Some(s) {
            Lit::Check
        } else if g.last.is_some_and(|m| m.from == s || m.to == s) {
            Lit::Last
        } else {
            Lit::Plain
        };
        square(g.theme, (s / 8 + s % 8).is_multiple_of(2), lit)
    })
}

/// The pieces a glide moves, from where to where: the mover, and the rook
/// in a castle.
fn gliders(m: Move) -> [(u8, u8); 2] {
    let rook = if m.flag == CASTLE {
        m.rook_hop()
    } else {
        (64, 64)
    };
    [(m.from, m.to), rook]
}

fn ease(t: u32, n: u32) -> f32 {
    let x = (t + 1) as f32 / n.max(1) as f32;
    x * x * (3.0 - 2.0 * x)
}

pub struct Painter<'a> {
    pub c: &'a mut Canvas,
    pub l: &'a Layout,
    pub art: &'a Art,
    pub g: &'a Game,
    pub colours: [u16; 64],
}

impl Painter<'_> {
    fn piece(&mut self, x: i32, y: i32, p: u8) {
        let spr = &self.art.big[kind(p) as usize - 1];
        let (k, sd) = (self.l.k, side(p));
        for (sy, row) in spr.iter().enumerate() {
            for (sx, &c) in row.iter().enumerate() {
                if c == art::CLEAR {
                    continue;
                }
                let (px, py) = (x + sx as i32 * k + k, y + sy as i32 * k + k);
                for dy in 0..k {
                    for dx in 0..k {
                        if let Some(s) = self.l.square_at(px + dx, py + dy) {
                            let sh = shadow(self.colours[s as usize]);
                            self.c.put(px + dx, py + dy, sh);
                        }
                    }
                }
            }
        }
        for (sy, row) in spr.iter().enumerate() {
            for (sx, &c) in row.iter().enumerate() {
                if c != art::CLEAR {
                    let (px, py) = (x + sx as i32 * k, y + sy as i32 * k);
                    self.c.rect(px, py, k, k, ink(sd, c));
                }
            }
        }
    }

    fn square(&mut self, s: u8, skip: [(u8, u8); 2]) {
        let (x, y) = self.l.square_xy(s);
        let sq = self.l.sq;
        self.c.rect(x, y, sq, sq, self.colours[s as usize]);
        let p = self.g.pos.sq[s as usize];
        if p != 0 && !skip.iter().any(|&(f, _)| f == s) {
            let (px, py) = self.l.sprite_xy(s);
            self.piece(px, py, p);
        }
    }

    /// Where a glider's sprite is at step `t`.
    fn glider_xy(&self, from: u8, to: u8, t: u32) -> (i32, i32) {
        let e = ease(t, self.l.glide);
        let (ax, ay) = self.l.sprite_xy(from);
        let (bx, by) = self.l.sprite_xy(to);
        let x = ax + ((bx - ax) as f32 * e).round() as i32;
        let y = ay + ((by - ay) as f32 * e).round() as i32;
        // Lifted a little mid-flight, so the glide reads as a hand carrying
        // the piece rather than a slide.
        let lift = ((e * std::f32::consts::PI).sin() * self.l.sq as f32 * 0.12) as i32;
        (x, y - lift)
    }

    /// Squares under a sprite (and its shadow) at `x, y`, as a bit set.
    fn under(&self, x: i32, y: i32) -> u64 {
        let p = SPRITE as i32 * self.l.k + self.l.k;
        let mut set = 0u64;
        for (cx, cy) in [
            (x, y),
            (x + p - 1, y),
            (x, y + p - 1),
            (x + p - 1, y + p - 1),
        ] {
            if let Some(s) = self.l.square_at(cx, cy) {
                set |= 1 << s;
            }
        }
        set
    }

    /// The whole board, or mid-glide only the squares the gliders left or
    /// now cover: every other square is already right on the panel.
    pub fn board(&mut self, whole: bool) {
        let mut only = u64::MAX;
        let skip = match self.g.phase {
            Phase::Glide { m, t } => {
                if !whole && t > 0 {
                    only = 0;
                    for (from, to) in gliders(m) {
                        if from != 64 {
                            let (x0, y0) = self.glider_xy(from, to, t - 1);
                            let (x1, y1) = self.glider_xy(from, to, t);
                            only |= self.under(x0, y0) | self.under(x1, y1);
                        }
                    }
                }
                gliders(m)
            }
            _ => [(64, 64); 2],
        };
        for s in 0..64 {
            if only & (1 << s) != 0 {
                self.square(s, skip);
            }
        }
        if let Phase::Glide { m, t } = self.g.phase {
            for (from, to) in gliders(m) {
                if from != 64 {
                    let (x, y) = self.glider_xy(from, to, t);
                    self.piece(x, y, self.g.pos.sq[from as usize]);
                }
            }
        }
        if let Phase::Over { end, .. } = self.g.phase {
            self.banner(end.score().as_bytes(), end.reason().as_bytes());
        }
    }

    fn banner(&mut self, score: &[u8], reason: &[u8]) {
        let b = self.l.sq * 8;
        let big = if b >= 200 { 2 } else { 1 };
        let h = TEXT_H as i32 * (big + 1) + 14;
        let y = self.l.by + (b - h) / 2;
        self.c.rect(self.l.bx, y, b, h, BANNER);
        self.c.rect(self.l.bx, y, b, 1, ACCENT);
        self.c.rect(self.l.bx, y + h - 1, b, 1, ACCENT);
        let w = (score.len() * ADVANCE) as i32 * big;
        self.c
            .text(self.l.bx + (b - w) / 2, y + 5, score, ACCENT, big);
        let w = (reason.len() * ADVANCE) as i32;
        let ry = y + 7 + TEXT_H as i32 * big;
        self.c.text(self.l.bx + (b - w) / 2, ry, reason, TEXT, 1);
    }

    pub fn eval_bar(&mut self, share: i32) {
        let (x, y, w, h) = self.l.ev;
        let white = h * share / 1000;
        self.c.rect(x, y, w, h - white, INK_DARK);
        self.c.rect(x, y + h - white, w, white, INK_LIGHT);
        self.c.rect(x, y + h / 2, w, 1, DIM);
    }

    /// The side panel: players at its ends, the record between.
    pub fn info(&mut self, now: u64) {
        let (x, y, w, h) = self.l.info;
        self.c.rect(x, y, w, h, PANEL);
        let (t1, t0, top, bottom) = self.bands();
        self.players(now);
        self.taken(1, x + 4, t1);
        self.taken(0, x + 4, t0);
        if bottom - top < TEXT_H as i32 {
            return;
        }
        self.c.rect(x + 4, top - 3, w - 8, 1, RULE);
        self.c.rect(x + 4, bottom + 1, w - 8, 1, RULE);
        let name = self.g.opening_name().as_bytes();
        let fit = ((w - 8) as usize / ADVANCE)
            .saturating_sub(7)
            .min(name.len());
        self.c.text(x + 4, top, &name[..fit], DIM, 1);
        let row = TEXT_H as i32 + 4;
        self.moves(x + 4, top + row, w - 8, bottom - top - row);
    }

    /// Inside the side panel: the y of each side's captured row (black's,
    /// white's), and the record's top and bottom.
    fn bands(&self) -> (i32, i32, i32, i32) {
        let (_, y, _, h) = self.l.info;
        let row = TEXT_H as i32 + 4;
        let mini = 8 * self.l.k;
        let t1 = y + row + 5;
        let t0 = y + h - row - mini - 5;
        (t1, t0, t1 + mini + 7, t0 - 7)
    }

    /// Both player rows: names, levels, and who is thinking, every frame.
    pub fn players(&mut self, now: u64) {
        let (x, y, w, h) = self.l.info;
        let row = TEXT_H as i32 + 4;
        self.player(1, x + 4, y + 3, w - 8, now);
        self.player(0, x + 4, y + h - row + 1, w - 8, now);
    }

    fn player(&mut self, sd: usize, x: i32, y: i32, w: i32, now: u64) {
        self.c.clip_to(x, y - 2, w, TEXT_H as i32 + 6);
        self.c.rect(x, y - 2, w, TEXT_H as i32 + 6, PANEL);
        let to_move = self.g.pos.stm == sd && self.g.end.is_none();
        let d = TEXT_H as i32 - 2;
        self.c.rect(x, y + 1, d, d, art::INK[sd][0]);
        self.c.rect(x + 1, y + 2, d - 2, d - 2, art::INK[sd][1]);
        let name: &[u8] = if sd == 0 { b"WHITE" } else { b"BLACK" };
        let nx = x + d + 5;
        let tw = self
            .c
            .text(nx, y, name, if to_move { TEXT } else { DIM }, 1);
        let level = self.g.levels[sd];
        for i in 0..5 {
            let px = nx + tw + 6 + i * 5;
            let c = if i < i32::from(level) { ACCENT } else { RULE };
            self.c.rect(px, y + 4, 3, 3, c);
            self.c.rect(px, y + 8, 3, 1, c);
        }
        if to_move {
            if let Some((depth, _)) = self.g.progress() {
                let mut b = [0u8; 10];
                let ds = digits(u32::from(depth), &mut b);
                let mut label = [b'd', 0, 0, 0];
                let n = ds.len().min(3);
                label[1..=n].copy_from_slice(&ds[..n]);
                let lw = ((n + 1) * ADVANCE) as i32;
                if depth > 0 {
                    self.c.text(x + w - lw - 22, y, &label[..=n], DIM, 1);
                }
            }
            let thinking = matches!(self.g.phase, Phase::Think { .. });
            for i in 0..3u64 {
                let on = thinking && (now / (self.l.fps / 6).max(1)) % 3 == i;
                let c = if on {
                    GOOD
                } else if thinking {
                    RULE
                } else {
                    PANEL
                };
                self.c.rect(x + w - 16 + i as i32 * 6, y + 5, 3, 3, c);
            }
            let elapsed = now.saturating_sub(self.g.think_from);
            let full = (self.l.think_frames).max(1);
            let bw = ((w as u64 * elapsed.min(full)) / full) as i32;
            let c = if thinking { GOOD } else { PANEL };
            self.c.rect(x, y + TEXT_H as i32 + 2, bw, 1, c);
        }
        self.c.unclip();
    }

    fn taken(&mut self, sd: usize, x: i32, y: i32) {
        let them = sd ^ 1;
        let z = self.l.k;
        let mut px = x;
        let mut material = [0i32; 2];
        for (who, m) in material.iter_mut().enumerate() {
            for k in 1..6 {
                *m += i32::from(self.g.taken[who][k]) * [0, 1, 3, 3, 5, 9][k];
            }
        }
        let counts = &self.g.taken[sd];
        let n: i32 = counts[1..6].iter().map(|&c| i32::from(c)).sum();
        let kinds = counts[1..6].iter().filter(|&&c| c > 0).count() as i32;
        let (ix, _, iw, _) = self.l.info;
        let room = ix + iw - 4 - 4 * ADVANCE as i32 - x;
        // A long game takes more than the panel holds piece by piece: then
        // each kind is drawn once with its count.
        let grouped = n * 6 * z + kinds * 3 * z + 2 * z > room;
        self.c
            .clip_to(ix, y - TEXT_H as i32, iw - 4, 8 * z + 2 * TEXT_H as i32);
        let ty = y + 4 * z - TEXT_H as i32 / 2 - 1;
        for (k, &count) in counts.iter().enumerate().take(6).skip(1) {
            let shown = if grouped { count.min(1) } else { count };
            for _ in 0..shown {
                let spr = &self.art.mini[k - 1];
                for (sy, row) in spr.iter().enumerate() {
                    for (sx, &c) in row.iter().enumerate() {
                        if c != art::CLEAR {
                            let (mx, my) = (px + sx as i32 * z, y + sy as i32 * z);
                            self.c.rect(mx, my, z, z, mini_ink(them, c));
                        }
                    }
                }
                px += 6 * z;
            }
            if grouped && count > 1 {
                let mut b = [0u8; 10];
                let ds = digits(u32::from(count), &mut b);
                px += 2 * z;
                px += self.c.text(px, ty, ds, DIM, 1);
            }
            if count > 0 {
                px += 3 * z;
            }
        }
        px += 2 * z;
        let lead = material[sd] - material[them];
        if lead > 0 {
            let mut b = [0u8; 10];
            let ds = digits(lead as u32, &mut b);
            let tx = px + 4;
            self.c.text(tx, ty, b"+", DIM, 1);
            self.c.text(tx + ADVANCE as i32, ty, ds, DIM, 1);
        }
        self.c.unclip();
    }

    /// The record, a full move a row, newest at the bottom; flows into more
    /// columns when the panel is wide.
    fn moves(&mut self, x: i32, y: i32, w: i32, h: i32) {
        let adv = ADVANCE as i32;
        let pitch = TEXT_H as i32 + 1;
        let col_w = 20 * adv;
        let cols = (w / col_w).max(1);
        let rows_fit = (h / pitch).max(1);
        let plies = self.g.plies as i32;
        let mut full_rows = (plies + 1) / 2;
        let result = self.g.end.map(|e| e.score().as_bytes());
        if result.is_some() {
            full_rows += 1;
        }
        let shown = rows_fit * cols;
        let first = (full_rows - shown).max(0);
        let showing = plies - 1;
        for r in first..full_rows {
            let slot = r - first;
            let cx = x + (slot / rows_fit) * col_w;
            let cy = y + (slot % rows_fit) * pitch;
            if r * 2 >= plies {
                if let Some(res) = result {
                    self.c.text(cx + 4 * adv, cy, res, ACCENT, 1);
                }
                continue;
            }
            let mut b = [0u8; 10];
            let num = digits((r + 1) as u32, &mut b);
            let nx = cx + (3 - num.len() as i32).max(0) * adv;
            self.c.text(nx, cy, num, DIM, 1);
            self.c.text(nx + num.len() as i32 * adv, cy, b".", DIM, 1);
            for half in 0..2 {
                let ply = r * 2 + half;
                if ply >= plies {
                    break;
                }
                let san = self.g.sans[ply as usize];
                let c = if ply == showing { ACCENT } else { TEXT };
                let sx = cx + 4 * adv + half * 8 * adv;
                if sx + 3 * adv <= x + w {
                    self.c.text(sx, cy, san.bytes(), c, 1);
                }
            }
        }
    }

    /// The eval in figures, at the top of the record.
    pub fn eval_figure(&mut self, score: i32) {
        let (x, _, w, _) = self.l.info;
        let (_, _, top, _) = self.bands();
        let mut b = [0u8; 12];
        let s = eval_text(score, &mut b);
        let tw = (7 * ADVANCE) as i32;
        let tx = x + w - 4 - tw;
        self.c.rect(tx, top, tw, TEXT_H as i32, PANEL);
        let sw = (s.len() * ADVANCE) as i32;
        let colour = if score >= 0 { TEXT } else { DIM };
        self.c.text(x + w - 4 - sw, top, s, colour, 1);
    }

    pub fn clear(&mut self) {
        let (x, y, w, h) = self.l.slot;
        self.c.rect(x, y, w, h, BG);
    }
}

const INK_LIGHT: u16 = art::INK[0][1];
const INK_DARK: u16 = art::INK[1][1];
