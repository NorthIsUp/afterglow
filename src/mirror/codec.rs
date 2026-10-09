//! One viewer's frame encoder: what changed since this viewer's last frame, as
//! the smaller of two records.
//!
//! * **Sparse** — `u32 n` (top bit clear), then `n * (u32 index, u32 cell)`.
//!   Eight bytes a changed cell: right for a saver that moves a fifth of its
//!   grid, and what a frame of a few changes costs nothing to build.
//! * **Packed** — `u32 PACKED | len`, then `len` bytes: `u32 t`, `t` distinct
//!   cell values, and a raw-deflate stream of one index per cell into them, a
//!   byte each when `t <= 256`, else a `u16`. A cell this viewer already has is
//!   the value [`NEVER`], so a packed record is a delta too.
//!
//! Packed is what a picture costs. Doom at pine's shape is 207k cells with
//! 167k changing every frame: sparse is 1.34 MB a frame, packed is ~50 KB,
//! because its cells are ~140 colours of one glyph and deflate eats the rest.
//! The page inflates with the browser's own `DecompressionStream`.

use miniz_oxide::deflate::core::{compress_to_output, CompressorOxide, TDEFLFlush, TDEFLStatus};
use miniz_oxide::deflate::CompressionLevel;
#[cfg(test)]
use miniz_oxide::inflate::decompress_to_vec;
use miniz_oxide::DataFormat;

use crate::grid::Cell;

use super::http::NEVER;

/// Top bit of a record's first word: packed, and the rest is its byte length.
pub(super) const PACKED: u32 = 1 << 31;

/// Sparse records up to this size are sent without trying packed: building a
/// packed record touches every cell, and below this it cannot win by enough to
/// pay for that.
const PACK_OVER: usize = 4 << 10;

/// More distinct values than a `u16` index addresses: sparse instead.
const MAX_TABLE: usize = 1 << 16;

pub struct Encoder {
    /// What this viewer has; [`NEVER`] until its first frame, so that frame
    /// is a keyframe without a second code path.
    prev: Vec<Cell>,
    out: Vec<u8>,
    table: Table,
    idx: Vec<u16>,
    bytes: Vec<u8>,
    deflate: Box<CompressorOxide>,
}

impl Encoder {
    pub fn new() -> Self {
        Self {
            prev: Vec::new(),
            out: Vec::new(),
            table: Table::default(),
            idx: Vec::new(),
            bytes: Vec::new(),
            deflate: Box::new(CompressorOxide::with_format_and_level(
                DataFormat::Raw,
                CompressionLevel::BestSpeed,
            )),
        }
    }

    /// The record taking this viewer from its last frame to `cur`.
    pub fn encode(&mut self, cur: &[Cell]) -> &[u8] {
        if self.prev.len() != cur.len() {
            self.prev.clear();
            self.prev.resize(cur.len(), NEVER);
        }
        let changed = self.prev.iter().zip(cur).filter(|(a, b)| a != b).count();
        let sparse = 4 + changed * 8;
        if sparse <= PACK_OVER || !self.pack(cur) || self.out.len() >= sparse {
            self.sparse(cur, changed);
        }
        self.prev.copy_from_slice(cur);
        &self.out
    }

    fn sparse(&mut self, cur: &[Cell], changed: usize) {
        self.out.clear();
        self.out.extend_from_slice(&(changed as u32).to_le_bytes());
        for (i, (a, b)) in self.prev.iter().zip(cur).enumerate() {
            if a != b {
                self.out.extend_from_slice(&(i as u32).to_le_bytes());
                self.out.extend_from_slice(&b.raw().to_le_bytes());
            }
        }
    }

    /// Build a packed record in `out`; false when the frame has too many
    /// distinct cells to index.
    fn pack(&mut self, cur: &[Cell]) -> bool {
        self.table.reset(cur.len());
        self.idx.clear();
        for (a, b) in self.prev.iter().zip(cur) {
            let v = if a == b { NEVER } else { *b };
            match self.table.index(v.raw()) {
                Some(i) => self.idx.push(i),
                None => return false,
            }
        }
        self.bytes.clear();
        if self.table.values.len() <= 256 {
            self.bytes.extend(self.idx.iter().map(|&i| i as u8));
        } else {
            self.bytes
                .extend(self.idx.iter().flat_map(|i| i.to_le_bytes()));
        }

        self.out.clear();
        self.out.extend_from_slice(&[0; 4]);
        self.out
            .extend_from_slice(&(self.table.values.len() as u32).to_le_bytes());
        for v in &self.table.values {
            self.out.extend_from_slice(&v.to_le_bytes());
        }
        self.deflate.reset();
        let out = &mut self.out;
        let (status, _) =
            compress_to_output(&mut self.deflate, &self.bytes, TDEFLFlush::Finish, |b| {
                out.extend_from_slice(b);
                true
            });
        debug_assert_eq!(status, TDEFLStatus::Done);
        let len = (self.out.len() - 4) as u32;
        self.out[..4].copy_from_slice(&(PACKED | len).to_le_bytes());
        true
    }
}

/// Cell value to its index in this frame's table: open addressing over a
/// power-of-two slot array, emptied through the list of slots it used, so a
/// frame of 140 colours clears 140 slots rather than the whole array.
#[derive(Default)]
struct Table {
    /// Index + 1 per slot, 0 empty.
    slots: Vec<u32>,
    keys: Vec<u32>,
    used: Vec<u32>,
    values: Vec<u32>,
    /// The last lookup: a picture is mostly runs of one value.
    last: Option<(u32, u16)>,
}

impl Table {
    fn reset(&mut self, cells: usize) {
        // Twice the most entries it can hold, so a probe stays short.
        let want = (2 * cells.min(MAX_TABLE)).next_power_of_two().max(64);
        if self.slots.len() == want {
            for &s in &self.used {
                self.slots[s as usize] = 0;
            }
        } else {
            self.slots = vec![0; want];
            self.keys = vec![0; want];
        }
        self.used.clear();
        self.values.clear();
        self.last = None;
    }

    fn index(&mut self, v: u32) -> Option<u16> {
        if let Some((k, i)) = self.last {
            if k == v {
                return Some(i);
            }
        }
        let mask = self.slots.len() - 1;
        let mut s = (v.wrapping_mul(0x9E37_79B9) >> 7) as usize & mask;
        loop {
            match self.slots[s] {
                0 => {
                    if self.values.len() == MAX_TABLE {
                        return None;
                    }
                    self.values.push(v);
                    self.slots[s] = self.values.len() as u32;
                    self.keys[s] = v;
                    self.used.push(s as u32);
                    let i = (self.values.len() - 1) as u16;
                    self.last = Some((v, i));
                    return Some(i);
                }
                n if self.keys[s] == v => {
                    let i = (n - 1) as u16;
                    self.last = Some((v, i));
                    return Some(i);
                }
                _ => s = (s + 1) & mask,
            }
        }
    }
}

/// What the page does with a record, in Rust: apply it to `have`.
#[cfg(test)]
pub(super) fn apply(have: &mut [Cell], rec: &[u8]) {
    let word = |o: usize| u32::from_le_bytes(rec[o..o + 4].try_into().unwrap());
    let head = word(0);
    if head & PACKED == 0 {
        assert_eq!(rec.len(), 4 + head as usize * 8);
        for k in 0..head as usize {
            have[word(4 + k * 8) as usize] = cell(word(8 + k * 8));
        }
        return;
    }
    assert_eq!(rec.len(), 4 + (head & !PACKED) as usize);
    let t = word(4) as usize;
    let values: Vec<u32> = (0..t).map(|k| word(8 + k * 4)).collect();
    let idx = decompress_to_vec(&rec[8 + t * 4..]).unwrap();
    let wide = t > 256;
    assert_eq!(idx.len(), have.len() * if wide { 2 } else { 1 });
    for (i, c) in have.iter_mut().enumerate() {
        let k = if wide {
            u16::from_le_bytes([idx[2 * i], idx[2 * i + 1]]) as usize
        } else {
            idx[i] as usize
        };
        if values[k] != NEVER.raw() {
            *c = cell(values[k]);
        }
    }
}

#[cfg(test)]
fn cell(raw: u32) -> Cell {
    Cell::new(raw as u16, (raw >> 16) as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A doom-shaped frame: one glyph, `colours` colours, every cell moving.
    fn picture(n: usize, colours: u32, seed: u32) -> Vec<Cell> {
        let mut r = seed | 1;
        (0..n)
            .map(|i| {
                if i % 7 == 0 {
                    r ^= r << 13;
                    r ^= r >> 17;
                    r ^= r << 5;
                }
                Cell::new(3, (r % colours) as u16)
            })
            .collect()
    }

    #[test]
    fn records_round_trip_whichever_kind_is_chosen() {
        let n = 960 * 216;
        let mut enc = Encoder::new();
        let mut have = vec![NEVER; n];
        for f in 0..4 {
            let cur = picture(n, 140, 2 * f + 1);
            let rec = enc.encode(&cur).to_vec();
            assert!(rec[3] & 0x80 != 0, "frame {f}: {} bytes", rec.len());
            apply(&mut have, &rec);
            assert!(have.iter().eq(&cur), "frame {f}");
        }
        // Two cells move: sparse, 4 + 2 * 8 bytes.
        let mut cur = have.clone();
        cur[5] = Cell::new(4, 1);
        cur[n - 1] = Cell::new(4, 2);
        let rec = enc.encode(&cur).to_vec();
        assert_eq!(rec.len(), 20);
        apply(&mut have, &rec);
        assert!(have.iter().eq(&cur));
        // Unchanged: the empty record that doubles as the keepalive.
        assert_eq!(enc.encode(&cur), &0u32.to_le_bytes());
    }

    /// More than 256 distinct values switches the indices to u16; more than
    /// a u16 can address falls back to sparse rather than truncating.
    #[test]
    fn wide_tables_use_u16_and_huge_ones_fall_back_to_sparse() {
        let n = 400 * 300;
        let mut enc = Encoder::new();
        let mut have = vec![NEVER; n];
        let cur = picture(n, 3000, 9);
        let rec = enc.encode(&cur).to_vec();
        assert!(rec[3] & 0x80 != 0);
        apply(&mut have, &rec);
        assert!(have.iter().eq(&cur));

        let mut enc = Encoder::new();
        let mut have = vec![NEVER; n];
        let cur: Vec<Cell> = (0..n)
            .map(|i| Cell::new(i as u16, (i >> 16) as u16))
            .collect();
        let rec = enc.encode(&cur).to_vec();
        assert_eq!(rec.len(), 4 + n * 8, "sparse");
        apply(&mut have, &rec);
        assert!(have.iter().eq(&cur));
    }

    /// The point of all this, at doom's real numbers.
    #[test]
    fn a_moving_picture_packs_far_below_sparse() {
        let n = 960 * 216;
        let mut enc = Encoder::new();
        enc.encode(&picture(n, 140, 1));
        let packed = enc.encode(&picture(n, 140, 2)).len();
        assert!(packed * 4 < n * 8, "packed {packed} bytes");
    }
}
