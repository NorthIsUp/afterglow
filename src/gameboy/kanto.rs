//! Pokémon Red, Blue and Yellow's map data, read from the cartridge in
//! pret's pokered layout: map headers and their connections, tileset
//! blocks and graphics, and which tiles can be walked on. The world view
//! draws from it and the bot plans its routes on it.

use std::collections::HashMap;

use super::carts::Revision;

/// Where a revision keeps its tables, as bank and address.
#[derive(Clone, Copy)]
struct Tables {
    /// Two-byte pointers to each map's header.
    headers: (u8, u16),
    /// One byte per map: the bank its header, blocks and objects are in.
    banks: (u8, u16),
    /// Twelve bytes per tileset.
    tilesets: (u8, u16),
    /// The bank the tilesets' walkable-tile lists are in.
    collision_bank: u8,
}

impl Tables {
    const fn of(r: Revision) -> Self {
        match r {
            Revision::Red | Revision::Blue => Self {
                headers: (0x00, 0x01AE),
                banks: (0x03, 0x423D),
                tilesets: (0x03, 0x47BE),
                collision_bank: 0x00,
            },
            Revision::Yellow => Self {
                headers: (0x3F, 0x41F2),
                banks: (0x3F, 0x43E4),
                tilesets: (0x03, 0x4558),
                collision_bank: 0x01,
            },
        }
    }
}

pub fn at(bank: u8, addr: u16) -> usize {
    if addr < 0x4000 {
        addr as usize
    } else {
        bank as usize * 0x4000 + (addr as usize - 0x4000)
    }
}

fn byte(rom: &[u8], bank: u8, addr: u16) -> u8 {
    rom.get(at(bank, addr)).copied().unwrap_or(0)
}

fn word(rom: &[u8], bank: u8, addr: u16) -> u16 {
    u16::from_le_bytes([byte(rom, bank, addr), byte(rom, bank, addr.wrapping_add(1))])
}

pub const NORTH: u8 = 8;
pub const SOUTH: u8 = 4;
pub const WEST: u8 = 2;
pub const EAST: u8 = 1;

/// A map header's fields, and its connections.
pub struct Header {
    pub tileset: u8,
    pub h: i32,
    pub w: i32,
    /// ROM offset of the map's blocks, `w * h` bytes.
    pub blocks: usize,
    pub border: u8,
    /// ROM offset of the map's object data: border block, then warps.
    pub objects: usize,
    /// Direction flag, map, offset along the edge in blocks.
    pub links: Vec<(u8, u8, i32)>,
}

/// A map placed on the world grid, in blocks from the current map's origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub map: u8,
    pub bx: i32,
    pub by: i32,
    pub w: i32,
    pub h: i32,
    pub tileset: u8,
    pub blocks: usize,
}

/// A tileset: its 256 blocks of 4x4 tile ids, its tiles decoded to colour
/// indices, and which tile ids can be walked on.
pub struct Tileset {
    pub blocks: Box<[u8]>,
    pub tiles: Box<[u8]>,
    pub walkable: [bool; 256],
    /// The tile wild Pokémon hide in, `0xFF` for none.
    pub grass: u8,
}

pub const TILES: usize = 0x80;

pub struct Kanto {
    tables: Tables,
    tilesets: HashMap<u8, Tileset>,
}

impl Kanto {
    pub fn new(r: Revision) -> Self {
        Self {
            tables: Tables::of(r),
            tilesets: HashMap::new(),
        }
    }

    pub fn header(&self, rom: &[u8], map: u8) -> Option<Header> {
        let t = self.tables;
        let ptr = word(rom, t.headers.0, t.headers.1 + 2 * u16::from(map));
        let bank = byte(rom, t.banks.0, t.banks.1 + u16::from(map));
        if ptr < 0x4000 || bank == 0 {
            return None;
        }
        let b = |o: u16| byte(rom, bank, ptr + o);
        let flags = b(9);
        let mut links = Vec::new();
        let mut o = 10;
        for dir in [NORTH, SOUTH, WEST, EAST] {
            if flags & dir == 0 {
                continue;
            }
            // The player's coordinate shift on crossing is minus twice the
            // offset of the joined map along this edge.
            let shift = if dir == NORTH || dir == SOUTH {
                b(o + 8)
            } else {
                b(o + 7)
            } as i8;
            links.push((dir, b(o), -i32::from(shift) / 2));
            o += 11;
        }
        let objects = word(rom, bank, ptr + o);
        Some(Header {
            tileset: b(0),
            h: i32::from(b(1)),
            w: i32::from(b(2)),
            blocks: at(bank, word(rom, bank, ptr + 3)),
            border: byte(rom, bank, objects),
            objects: at(bank, objects),
            links,
        })
    }

    /// `map` and every map joined to it, transitively, within `reach`
    /// blocks of its origin; the current map first.
    pub fn place(&mut self, rom: &[u8], map: u8, reach: i32, out: &mut Vec<Placed>) -> Option<u8> {
        out.clear();
        let cur = self.header(rom, map)?;
        let border = cur.border;
        let mut queue = vec![(map, 0i32, 0i32, cur)];
        while let Some((m, bx, by, h)) = queue.pop() {
            if out.iter().any(|p| p.map == m) {
                continue;
            }
            out.push(Placed {
                map: m,
                bx,
                by,
                w: h.w,
                h: h.h,
                tileset: h.tileset,
                blocks: h.blocks,
            });
            self.load_tileset(rom, h.tileset);
            for &(dir, t, off) in &h.links {
                let Some(th) = self.header(rom, t) else {
                    continue;
                };
                let (nx, ny) = match dir {
                    NORTH => (bx + off, by - th.h),
                    SOUTH => (bx + off, by + h.h),
                    WEST => (bx - th.w, by + off),
                    _ => (bx + h.w, by + off),
                };
                if nx.abs() <= reach && ny.abs() <= reach {
                    queue.push((t, nx, ny, th));
                }
            }
        }
        Some(border)
    }

    pub fn tileset(&self, ts: u8) -> Option<&Tileset> {
        self.tilesets.get(&ts)
    }

    fn load_tileset(&mut self, rom: &[u8], ts: u8) {
        if self.tilesets.contains_key(&ts) {
            return;
        }
        let t = self.tables;
        let base = t.tilesets.1 + 12 * u16::from(ts);
        let b = |o: u16| byte(rom, t.tilesets.0, base + o);
        let w = |o: u16| word(rom, t.tilesets.0, base + o);
        let bank = b(0);
        let (blocks, gfx) = (at(bank, w(1)), at(bank, w(3)));
        let mut walkable = [false; 256];
        let coll = at(t.collision_bank, w(5));
        for &id in rom
            .get(coll..)
            .unwrap_or(&[])
            .iter()
            .take_while(|&&id| id != 0xFF)
        {
            walkable[id as usize] = true;
        }
        let mut tiles = vec![0u8; TILES * 64].into_boxed_slice();
        decode(&mut tiles, |i| rom.get(gfx + i).copied().unwrap_or(0));
        self.tilesets.insert(
            ts,
            Tileset {
                blocks: (0..256 * 16)
                    .map(|i| rom.get(blocks + i).copied().unwrap_or(0))
                    .collect(),
                tiles,
                walkable,
                grass: b(10),
            },
        );
    }

    /// The tile the game tests at square `(sx, sy)` of a placed map: the
    /// square's lower-left.
    pub fn tile(&self, rom: &[u8], p: &Placed, sx: i32, sy: i32) -> Option<u8> {
        if sx < 0 || sy < 0 || sx >= p.w * 2 || sy >= p.h * 2 {
            return None;
        }
        let ts = self.tilesets.get(&p.tileset)?;
        let block = rom
            .get(p.blocks + (sy / 2 * p.w + sx / 2) as usize)
            .copied()
            .unwrap_or(0);
        Some(
            ts.blocks
                [block as usize * 16 + ((sy % 2) * 2 + 1) as usize * 4 + (sx % 2 * 2) as usize],
        )
    }

    /// Whether the player can stand on square `(sx, sy)` of a placed map:
    /// its tile is in its tileset's walkable list.
    pub fn walkable(&self, rom: &[u8], p: &Placed, sx: i32, sy: i32) -> bool {
        let Some(tile) = self.tile(rom, p, sx, sy) else {
            return false;
        };
        self.tilesets
            .get(&p.tileset)
            .is_some_and(|ts| ts.walkable[tile as usize])
    }
}

/// 2bpp tiles to one colour index per pixel.
pub fn decode(out: &mut [u8], src: impl Fn(usize) -> u8) {
    for t in 0..TILES {
        for row in 0..8 {
            let (lo, hi) = (src(t * 16 + row * 2), src(t * 16 + row * 2 + 1));
            for px in 0..8 {
                let bit = 7 - px;
                out[t * 64 + row * 8 + px] = (lo >> bit & 1) | (hi >> bit & 1) << 1;
            }
        }
    }
}
