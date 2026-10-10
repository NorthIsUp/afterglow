//! Pokémon's overworld beyond the screen, drawn from the cartridge's own map
//! data (`kanto.rs`): the map the player is on and the maps its connections
//! join on, outward, with the live tile graphics from VRAM so water and
//! flowers animate — the way vjeux renders all of Kanto from pokered. It
//! shows ground the player has never seen, which a remembered-background
//! view (`view.rs`) cannot.
//!
//! Its colours are learned every frame from the screen itself: each of the
//! four colour indices takes the colour the real screen shows where the
//! drawn world puts that index. That follows palette fades and GBC colours
//! without knowing either, and doubles as the check that the drawing lines
//! up. When it does not (a battle, a menu, a map loading) the view falls
//! back to the plain screen.

use mizu_core::GameBoy;

use super::carts::Revision;
use super::engine::{H, SCREEN_W};
use super::kanto::{decode, Kanto, Placed, TILES};
use super::pokemon::Ram;
use super::view::DIM;

/// Maps further than this from the player, in blocks, are not placed: the
/// widest view is about twelve blocks across.
const REACH: i32 = 24;
const UNKNOWN: u8 = 0xFF;
/// `wPlayerMovingDirection`'s values.
const LEFT: u8 = 2;
const UP: u8 = 8;

pub struct World {
    kanto: Kanto,
    ram: Ram,
    map: Option<u8>,
    placed: Vec<Placed>,
    border: u8,
    cur_tileset: u8,
    vram_tiles: Box<[u8]>,
    /// The world under the view, colour indices.
    idx: Vec<u8>,
    shown: bool,
    /// Last frame's agreement with the screen, per mille.
    pub agree: u32,
}

impl World {
    pub fn new(rev: Revision) -> Self {
        Self {
            kanto: Kanto::new(rev),
            ram: Ram::of(rev),
            map: None,
            placed: Vec::new(),
            border: 0,
            cur_tileset: 0xFF,
            vram_tiles: vec![0; TILES * 64].into_boxed_slice(),
            idx: Vec::new(),
            shown: false,
            agree: 0,
        }
    }

    /// The 8x8 tile at tile coordinates `(tx, ty)` from the current map's
    /// origin, as 64 colour indices; `None` off every placed map's tileset.
    fn tile(&self, rom: &[u8], tx: i32, ty: i32) -> Option<&[u8]> {
        let (bx, by) = (tx.div_euclid(4), ty.div_euclid(4));
        let (block, ts) = match self
            .placed
            .iter()
            .find(|p| (p.bx..p.bx + p.w).contains(&bx) && (p.by..p.by + p.h).contains(&by))
        {
            Some(p) => {
                let i = p.blocks + ((by - p.by) * p.w + (bx - p.bx)) as usize;
                (rom.get(i).copied().unwrap_or(0), p.tileset)
            }
            None => (self.border, self.placed.first()?.tileset),
        };
        let t = self.kanto.tileset(ts)?;
        let (ix, iy) = (tx.rem_euclid(4) as usize, ty.rem_euclid(4) as usize);
        let id = t.blocks[block as usize * 16 + iy * 4 + ix] as usize % TILES;
        let tiles = if ts == self.cur_tileset {
            &self.vram_tiles
        } else {
            &t.tiles
        };
        Some(&tiles[id * 64..][..64])
    }

    pub fn map(&self) -> Option<u8> {
        self.map
    }

    /// The world pixel at the screen's top-left. The player stands on the
    /// square four from the left and four from the top. Mid-step the screen
    /// is part of the way to the next square while the coordinates still
    /// name the last, so the fine scroll counts on from them: back from 16
    /// when the step goes left or up.
    pub fn camera(&self, gb: &mut GameBoy) -> (i32, i32) {
        let r = self.ram;
        let (x, y) = (i32::from(gb.peek(r.x)), i32::from(gb.peek(r.y)));
        let moving = gb.peek(r.moving_direction);
        // The scroll the frame was drawn with, not the register: the game
        // has already written next frame's by the time this runs.
        let [scx, scy] = gb.line_scroll()[0].map(i32::from);
        let fine = |s: i32, back: bool| match s & 15 {
            0 => 0,
            f if back => f - 16,
            f => f,
        };
        // The coordinates move on when the walk counter runs out, a couple
        // of frames before the scroll catches up: from then on the fine
        // scroll counts back from the new square instead.
        let done = gb.peek(r.walk_counter) == 0;
        let (fx, fy) = (
            fine(scx, (moving == LEFT) != done),
            fine(scy, (moving == UP) != done),
        );
        ((x - 4) * 16 + fx, (y - 4) * 16 + fy)
    }

    /// Fill `idx` with the world under a `w`-wide view whose screen's
    /// top-left is world pixel `(cx, cy)`. A tile row at a time.
    fn render(&mut self, rom: &[u8], cx: i32, cy: i32, w: usize) {
        let mut idx = std::mem::take(&mut self.idx);
        idx.clear();
        idx.resize(w * H, UNKNOWN);
        let x0 = (w - SCREEN_W) as i32 / 2;
        let left = cx - x0;
        for y in 0..H {
            let wy = cy + y as i32;
            let (ty, py) = (wy.div_euclid(8), wy.rem_euclid(8) as usize);
            let mut vx = 0usize;
            while vx < w {
                let wx = left + vx as i32;
                let (tx, px) = (wx.div_euclid(8), wx.rem_euclid(8) as usize);
                let n = (8 - px).min(w - vx);
                if let Some(t) = self.tile(rom, tx, ty) {
                    let src = &t[py * 8 + px..][..n];
                    idx[y * w + vx..][..n].copy_from_slice(src);
                }
                vx += n;
            }
        }
        self.idx = idx;
    }

    /// Agreement with the screen's background, per mille, and the colour
    /// each index shows there.
    fn fit(&self, bg: &[u16], w: usize) -> (u32, [u16; 4]) {
        let x0 = (w - SCREEN_W) / 2;
        let mut votes = [[(0u16, 0u32); 4]; 4];
        let mut total = 0u32;
        for y in 0..H {
            for x in 0..SCREEN_W {
                let (p, i) = (bg[y * SCREEN_W + x], self.idx[y * w + x0 + x]);
                if p & DIM != 0 || i > 3 {
                    continue;
                }
                total += 1;
                // Majority vote in four slots: the screen's background has
                // four colours per palette, a few more mid-fade.
                let slots = &mut votes[i as usize];
                if let Some(s) = slots.iter_mut().find(|s| s.1 > 0 && s.0 == p) {
                    s.1 += 1;
                } else if let Some(s) = slots.iter_mut().min_by_key(|s| s.1) {
                    if s.1 == 0 {
                        *s = (p, 1);
                    } else {
                        s.1 -= 1;
                    }
                }
            }
        }
        let colour = votes.map(|s| s.iter().max_by_key(|s| s.1).map_or(0, |s| s.0));
        let mut hits = 0u32;
        for y in 0..H {
            for x in 0..SCREEN_W {
                let (p, i) = (bg[y * SCREEN_W + x], self.idx[y * w + x0 + x]);
                if p & DIM == 0 && i <= 3 && colour[i as usize] == p {
                    hits += 1;
                }
            }
        }
        (hits * 1000 / total.max(1), colour)
    }

    /// Bring the placed maps and the live tiles up to date. False when
    /// there is no world to draw.
    fn update(&mut self, gb: &mut GameBoy) -> bool {
        if !gb.lcd_on() || gb.peek(self.ram.in_battle) != 0 {
            return false;
        }
        let map = gb.peek(self.ram.cur_map);
        if self.map != Some(map) {
            self.map = Some(map);
            // A new map must line up from scratch, not ride on the old one's
            // standing: the number changes a moment before the screen does.
            self.shown = false;
            self.border = self
                .kanto
                .place(gb.rom(), map, REACH, &mut self.placed)
                .unwrap_or(0);
        }
        if self.placed.is_empty() {
            return false;
        }
        self.cur_tileset = gb.peek(self.ram.tileset);
        let vram = gb.vram();
        decode(&mut self.vram_tiles, |i| {
            vram.get(0x1000 + i).copied().unwrap_or(0)
        });
        true
    }

    /// Draw the world into the view's sides. False, drawing nothing, when
    /// it does not match what the screen shows.
    pub fn compose(&mut self, gb: &mut GameBoy, tone: &[u16], out: &mut [u16], w: usize) -> bool {
        if !self.update(gb) {
            self.shown = false;
            self.agree = 0;
            return false;
        }
        let (cx, cy) = self.camera(gb);
        self.render(gb.rom(), cx, cy, w);
        let (agree, colour) = self.fit(gb.bg_buffer(), w);
        self.agree = agree;
        let need = if self.shown { 600 } else { 850 };
        self.shown = agree >= need;
        if !self.shown {
            return false;
        }
        self.npcs(gb, cx, cy, w);
        let x0 = (w - SCREEN_W) / 2;
        let colour = colour.map(|c| tone[c as usize]);
        for y in 0..H {
            let row = &mut out[y * w..][..w];
            let idx = &self.idx[y * w..][..w];
            for (vx, (o, &i)) in row.iter_mut().zip(idx).enumerate() {
                if i <= 3 && !(x0..x0 + SCREEN_W).contains(&vx) {
                    *o = colour[i as usize];
                }
            }
        }
        true
    }

    /// People and items off the screen, standing on their squares, drawn
    /// into `idx` from their graphics in VRAM: the game keeps every one's
    /// map position and sprite slot even while it hides them. Sprite
    /// colours go through OBP0, then back to the background index that
    /// shows the same shade, so they take the learned colours too.
    fn npcs(&mut self, gb: &mut GameBoy, cx: i32, cy: i32, w: usize) {
        let (bgp, obp) = (gb.peek(0xFF47), gb.peek(0xFF48));
        let shade_index = |c: u8| {
            let shade = obp >> (2 * c) & 3;
            (0..4u8).find(|&i| bgp >> (2 * i) & 3 == shade)
        };
        let to_index = [None, shade_index(1), shade_index(2), shade_index(3)];
        let x0 = (w - SCREEN_W) as i32 / 2;
        let r = self.ram;
        for n in 1..16u16 {
            let (d1, d2) = (r.sprite_data1 + n * 16, r.sprite_data2 + n * 16);
            if gb.peek(d1) == 0 {
                continue;
            }
            let (my, mx, base) = (gb.peek(d2 + 4), gb.peek(d2 + 5), gb.peek(d2 + 0xE));
            if my < 4 || mx < 4 || base == 0 {
                continue;
            }
            let wx = (i32::from(mx) - 4) * 16;
            let wy = (i32::from(my) - 4) * 16 - 4;
            let (vx, vy) = (wx - cx + x0, wy - cy);
            // On the screen the real sprite is already there, mid-step too.
            if vx + 16 > x0 && vx < x0 + SCREEN_W as i32 && vy + 16 > 0 && vy < H as i32 {
                continue;
            }
            if vx + 16 <= 0 || vx >= w as i32 || vy + 16 <= 0 || vy >= H as i32 {
                continue;
            }
            let slot = u16::from(base - 1);
            let (first, facing, flip) = match slot {
                0xA => (0xA * 12, 0, false),
                0xB => (0xA * 12 + 4, 0, false),
                _ => match gb.peek(d1 + 9) {
                    4 => (slot * 12, 4, false),
                    8 => (slot * 12, 8, false),
                    0xC => (slot * 12, 8, true),
                    _ => (slot * 12, 0, false),
                },
            };
            let vram = gb.vram();
            for (k, (ty, tx)) in [(0, 0), (0, 8), (8, 0), (8, 8)].into_iter().enumerate() {
                let tile = usize::from(first + facing + k as u16);
                let tx = if flip { 8 - tx } else { tx };
                for row in 0..8 {
                    let at = tile * 16 + row * 2;
                    let (lo, hi) = (
                        vram.get(at).copied().unwrap_or(0),
                        vram.get(at + 1).copied().unwrap_or(0),
                    );
                    let py = vy + ty + row as i32;
                    if !(0..H as i32).contains(&py) {
                        continue;
                    }
                    for col in 0..8 {
                        let bit = if flip { col } else { 7 - col };
                        let c = (lo >> bit & 1) | (hi >> bit & 1) << 1;
                        let px = vx + tx + col;
                        if let (Some(i), true) = (to_index[c as usize], (0..w as i32).contains(&px))
                        {
                            self.idx[py as usize * w + px as usize] = i;
                        }
                    }
                }
            }
        }
    }

    /// The camera offset from `camera()` that fits the screen best, within
    /// `r` pixels, and its agreement: for the test that checks `camera()`.
    #[cfg(test)]
    pub fn best_offset(
        &mut self,
        gb: &mut GameBoy,
        w: usize,
        r: i32,
    ) -> Option<((i32, i32), u32, u32)> {
        if !self.update(gb) {
            return None;
        }
        let (cx, cy) = self.camera(gb);
        self.render(gb.rom(), cx, cy, w);
        let here = self.fit(gb.bg_buffer(), w).0;
        let mut best = ((0, 0), 0);
        for dy in -r..=r {
            for dx in -r..=r {
                self.render(gb.rom(), cx + dx, cy + dy, w);
                let a = self.fit(gb.bg_buffer(), w).0;
                if a > best.1 {
                    best = ((dx, dy), a);
                }
            }
        }
        Some((best.0, best.1, here))
    }
}
