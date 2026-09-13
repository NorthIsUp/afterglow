//! Rain — falling streaks over black, in three depth tiers, with wind and a
//! splash on the ground row.
//!
//! # Why the streaks are braille
//!
//! The obvious draw is ASCII `|`, and it is wrong: unifont's bar stops two
//! pixels short of the top of its cell, so a streak of stacked bars is a column
//! of dashes with a 2px gap every 16px, and a drop moving a whole cell a frame
//! jumps 16px at a time. Braille in this atlas is FILLED 2x4 quadrants, so a
//! cell is four stackable 4x16 quarters: a streak is continuous, it grows and
//! moves in 4px steps, and the wind shifts it half a cell sideways without a
//! second glyph. Everything is drawn in sub-rows (quarter cells) for that
//! reason.
//!
//! # Why this is sparse (Model B)
//!
//! Nothing here is a field: the scene is black except a few hundred cells that
//! are moving. A full repaint would recompute 16k cells to change ~1.5k of
//! them. So the streaks own their cells and hand `flush_sparse` the list.
//!
//! The bug that model can have is a trail — a cell a streak left behind and
//! never erased. Two things stop it:
//!
//! * A streak's cells are DIFFED, not approximated. Each frame it computes
//!   where it is now, erases the old cells that are not in the new set, and
//!   draws the new ones. Nothing repaints "background" over a guess.
//! * Erase runs for EVERY streak before any streak draws. Interleaved, a
//!   streak crossing another would erase a cell its neighbour had already
//!   drawn this frame and punch a hole in it; two passes make the scene the
//!   union of what the streaks stamp, which is what
//!   `the_scene_is_exactly_what_the_drops_stamp` asserts cell for cell.
//!
//! The ground row is RESERVED for splashes and no streak ever draws into it.
//! That is the whole reason splashes need no save-and-restore: the two things
//! that write cells write disjoint regions, so neither can erase the other.
//!
//! Depth is three tiers that differ in speed, length AND brightness together —
//! a far streak that was merely dim but as fast as a near one reads as a dim
//! near streak, not as distance.
//!
//! # Per-frame cost
//!
//! Every streak moves every frame — even the slowest tier covers more than a
//! cell a frame — so this is nowhere near as cheap as city: a frame rewrites
//! about 1.5k of the panel's 16k cells. Measured against matrix on the same
//! machine it is a bit under a third of a full repaint, which puts it around
//! 30-40m of a Pi 5 core at 1920x1080/15fps against matrix's 113m.
//!
//! Damage is a single run covering nearly the whole panel, because rain falls
//! down all of it. That is not a bug and not fixable: what the sparse model
//! buys here is the BLIT, not the shadow-to-hardware copy.
use crate::font;
use crate::grid::{bake, Cell, Grid};
use crate::saver::Saver;
use crate::surface::{Panel, Surface};
use crate::{env_num, next_rand};

/// Cold and blue, two entries per tier: the body of the streak and its head.
/// The head is the brighter one — a streak lit evenly reads as a scratch on the
/// panel, and the bright leading drop is what makes it read as falling.
///
/// The tiers do not overlap in luminance. That separation IS the depth cue, so
/// `the_tiers_read_as_depth` pins it rather than leaving it to taste.
#[rustfmt::skip]
const PAL_RGB: [[u8; 3]; 9] = [
    [0x00, 0x00, 0x00], // 0 background
    [0x1E, 0x2A, 0x3A], // 1 far body — barely there, which is the point
    [0x32, 0x44, 0x5C], // 2 far head
    [0x4A, 0x64, 0x84], // 3 mid body
    [0x76, 0x98, 0xBC], // 4 mid head
    [0x8E, 0xB4, 0xD6], // 5 near body
    [0xE4, 0xF2, 0xFF], // 6 near head — the only near-white in the scene
    [0xB4, 0xD2, 0xEA], // 7 splash, on impact
    [0x4A, 0x62, 0x7A], // 8 splash, settling
];
const PAL: [u32; 9] = bake(&PAL_RGB);

/// Sub-rows per cell: a braille cell is 2x4 filled quadrants, so a cell row is
/// four stackable quarters and a streak's ends land on 4px boundaries.
const SUB: i32 = 4;

/// The braille bit for (half of the cell, quarter down it). Left column is dots
/// 1,2,3,7 and right column dots 4,5,6,8 — the atlas draws them as filled 4x4
/// rectangles, so an OR of these is exactly the lit sub-cells.
#[rustfmt::skip]
const BITS: [[u8; SUB as usize]; 2] = [
    [0x01, 0x02, 0x04, 0x40],
    [0x08, 0x10, 0x20, 0x80],
];

/// The splash, as braille sub-cells. Impact is the two lowest quadrants (a flat
/// spread across the bottom of the cell); settling is the pair one quarter up,
/// dimmer — a ripple rising and dying rather than a blink.
const SPLASH_HIT: u16 = font::BRAILLE[0xC0];
const SPLASH_FADE: u16 = font::BRAILLE[0x24];

/// Longest streak, in sub-rows, and so the most cells one can occupy: hard wind
/// puts every sub-row in its own cell.
const MAX_SPAN: usize = 24;

/// One depth. Speed, length and brightness move TOGETHER — see the module doc.
struct Tier {
    /// Percent of `RAIN_SPEED`.
    speed_pct: i32,
    /// Lengths, in SUB-ROWS (4px at the default cell): `len_min..len_min +
    /// len_span`, so a tier is a band of streaks and not one repeated length.
    len_min: u8,
    len_span: u8,
    body: u16,
    head: u16,
    /// Far rain does not splash: the ground it lands on is behind the scene.
    splash: bool,
}

#[rustfmt::skip]
const TIERS: [Tier; 3] = [
    Tier { speed_pct: 38,  len_min: 2,  len_span: 3, body: 1, head: 2, splash: false },
    Tier { speed_pct: 68,  len_min: 6,  len_span: 5, body: 3, head: 4, splash: true  },
    Tier { speed_pct: 100, len_min: 12, len_span: 9, body: 5, head: 6, splash: true  },
];

/// Weighted draw: mostly far. A uniform draw gives a curtain of equally near
/// streaks, which is the flat look this is avoiding.
#[rustfmt::skip]
const TIER_DRAW: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 2, 2, 2, 1];

const _: () = {
    let mut i = 0;
    while i < TIERS.len() {
        assert!(
            (TIERS[i].len_min + TIERS[i].len_span) as usize <= MAX_SPAN,
            "a tier draws longer than the per-drop buffer"
        );
        assert!(TIERS[i].len_span >= 1, "a tier has no length spread");
        assert!(
            (TIERS[i].body as usize) < PAL.len() && (TIERS[i].head as usize) < PAL.len(),
            "a tier colours outside the palette"
        );
        // Head brighter than body, and each tier clear of the one behind it.
        assert!(
            PAL_RGB[TIERS[i].head as usize][2] > PAL_RGB[TIERS[i].body as usize][2],
            "a tier's head is no brighter than its body"
        );
        assert!(
            i == 0
                || (PAL_RGB[TIERS[i].body as usize][2] > PAL_RGB[TIERS[i - 1].head as usize][2]
                    && TIERS[i].speed_pct > TIERS[i - 1].speed_pct
                    && TIERS[i].len_min > TIERS[i - 1].len_min),
            "the tiers do not climb in brightness, speed and length together"
        );
        i += 1;
    }
    let mut i = 0;
    while i < TIER_DRAW.len() {
        assert!(
            (TIER_DRAW[i] as usize) < TIERS.len(),
            "a tier draw names a tier that does not exist"
        );
        i += 1;
    }
};

/// One streak. Positions are 8.8 fixed point in CELLS: a drop that moved a
/// whole number of cells a frame would quantise every speed to the same few
/// values and the tiers would stop differing.
struct Drop {
    x: i32,
    y: i32,
    /// Cells per frame, down. Always at least 1/256 so nothing ever stalls.
    vy: i32,
    /// Length in sub-rows.
    len: u8,
    tier: u8,
    /// Where it is NOW, head cell first, with the braille pattern each cell
    /// holds. `next` is this frame's answer, kept in the struct rather than on
    /// the stack because the erase pass computes it and the draw pass — a whole
    /// loop later — consumes it.
    cells: [u32; MAX_SPAN],
    n: u8,
    next: [u32; MAX_SPAN],
    next_pat: [u8; MAX_SPAN],
    nn: u8,
    /// Whether `next[0]` is the head cell, i.e. the head is on screen. Without
    /// it a streak whose head has gone below the ground row would paint its
    /// second cell in the head colour.
    head_vis: bool,
    /// Splashed already, so a streak that takes several frames to clear the
    /// ground row makes one splash and not five.
    landed: bool,
}

/// A ripple on the ground row: one cell, two stages, then gone.
struct Splash {
    i: u32,
    stage: u8,
    t: u8,
    live: bool,
}

pub struct Rain {
    grid: Grid,
    drops: Vec<Drop>,
    splashes: Vec<Splash>,
    /// Reserved in `new`, only ever `clear`ed — CLAUDE.md makes the frame loop
    /// the top constraint in this repo and `render_never_allocates` pins it.
    dirty: Vec<u32>,
    cols: i32,
    /// The row splashes live on. Streaks draw in `0..ground` and nothing else
    /// writes there, which is why neither has to save what the other covered.
    ground: i32,
    /// Cells sideways per SUB-ROW of fall, 8.8. Signed: wind blows either way.
    slant: i32,
    /// Frames a splash spends in each of its two stages.
    splash_frames: u8,
    splash_on: bool,
    vy_base: i32,
    rng: u32,
}

impl Rain {
    pub fn new(panel: &Panel, fps: u32) -> Self {
        // 8x16 is the braille cell's own size: anything else stretches the
        // quadrants and the streak stops being 4px wide.
        let cell_w = env_num(&["RAIN_CELL_W"], 8, 4, 64) as usize;
        let cell_h = env_num(&["RAIN_CELL_H"], 16, 4, 128) as usize;
        let grid = Grid::new(panel, cell_w, cell_h);
        let (cols, rows) = (grid.cols() as i32, grid.rows() as i32);

        // The calibration knob. Density is per THOUSAND CELLS rather than an
        // absolute count, so the same number reads the same on a 1080p panel
        // and on whatever else this ends up plugged into. 11/1000 is about 180
        // streaks at 1920x1080: heavy enough to read as rain, open enough that
        // the black between streaks is still black.
        let per_mille = env_num(&["RAIN_DENSITY"], 11, 0, 200);
        // Rows per second for the NEAREST tier; the others are a percentage of
        // it. At 50 a near streak crosses a 67-row panel in about 1.3s, which
        // is rain rather than sleet.
        let speed = env_num(&["RAIN_SPEED"], 50, 1, 1000) as i32;
        // Wind, as cells sideways per HUNDRED cells of fall. Signed, so a
        // negative value blows the other way. 34 is a light slant — the streak
        // leans about a third of a cell per cell, which at braille resolution
        // is a visible lean rather than a staircase.
        let wind = env_num(&["RAIN_WIND"], 34, -300, 300) as i32;
        // Splash lifetime in milliseconds, per stage. 0 turns splashes off.
        let splash_ms = env_num(&["RAIN_SPLASH_MS"], 160, 0, 2000) as i32;

        let fps = fps.max(1) as i32;
        let n = ((cols as i64 * rows as i64) * per_mille / 1000).max(1) as usize;
        let vy_base = ((speed * 256) / fps).max(1);

        let mut rain = Self {
            grid,
            drops: Vec::with_capacity(n),
            // A splash per drop is the ceiling: every streak could land on the
            // same frame. Cheaper to reserve than to decide what to drop.
            splashes: Vec::with_capacity(n),
            // Each drop can erase its whole old streak and draw its whole new
            // one in one frame, and every splash can change.
            dirty: Vec::with_capacity(n * 2 * MAX_SPAN + n + 8),
            cols,
            ground: rows - 1,
            slant: wind * 256 / 100 / SUB,
            splash_frames: ((splash_ms * fps) / 1000).clamp(1, 255) as u8,
            splash_on: splash_ms > 0,
            vy_base,
            rng: 0x5a1e_1c0d,
        };
        for _ in 0..n {
            let mut d = Drop {
                x: 0,
                y: 0,
                vy: vy_base,
                len: 1,
                tier: 0,
                cells: [0; MAX_SPAN],
                n: 0,
                next: [0; MAX_SPAN],
                next_pat: [0; MAX_SPAN],
                nn: 0,
                head_vis: false,
                landed: false,
            };
            rain.respawn(&mut d);
            // Spread over the whole panel, so frame 0 is rain already falling
            // rather than an empty sky that fills in over the first second.
            d.y = (next_rand(&mut rain.rng) as i32 % rows.max(1)) << 8;
            rain.drops.push(d);
        }
        for _ in 0..n {
            rain.splashes.push(Splash {
                i: 0,
                stage: 0,
                t: 0,
                live: false,
            });
        }
        // Nothing stamps the scene here: `render` draws every streak before
        // the flush, so frame 0's blit-everything already has the whole field
        // to paint. A separate frame-0 stamp was written and deleted — nothing
        // observed it, which is what the frame-0 lit-pixel assertion says.
        rain
    }

    fn respawn(&mut self, d: &mut Drop) {
        let tier = TIER_DRAW[next_rand(&mut self.rng) as usize % TIER_DRAW.len()];
        let t = &TIERS[tier as usize];
        d.tier = tier;
        d.len = t.len_min + (next_rand(&mut self.rng) % t.len_span as u32) as u8;
        // +/-20% on top of the tier, so two near streaks are not in lockstep.
        let jitter = 80 + (next_rand(&mut self.rng) % 41) as i32;
        d.vy = ((self.vy_base * t.speed_pct / 100) * jitter / 100).max(1);
        d.x = (next_rand(&mut self.rng) as i32 % self.cols.max(1)) << 8;
        // Start the whole streak above the top edge, plus a random gap, or
        // every drop that respawns on the same frame enters in one rank.
        d.y = -((d.len as i32) << 6) - ((next_rand(&mut self.rng) as i32 % 24) << 8);
        d.landed = false;
    }

    /// Head sub-row: quarter-cells, floored, so a streak straddling a cell
    /// boundary is drawn where it actually is rather than snapped to a row.
    #[inline]
    fn head_sub(d: &Drop) -> i32 {
        d.y >> 6
    }

    /// Where the streak's cells are and which quadrants of each it lights, from
    /// its head position. Head first, and only what is on screen — a streak
    /// entering at the top or leaving at the ground row is clipped, not wrapped
    /// vertically.
    fn plan(&self, d: &mut Drop) {
        d.nn = 0;
        d.head_vis = false;
        let head = Self::head_sub(d);
        for k in 0..d.len as i32 {
            let s = head - k;
            let r = s >> 2;
            if r < 0 || r >= self.ground {
                continue;
            }
            let xs = d.x - k * self.slant;
            let c = (xs >> 8).rem_euclid(self.cols.max(1));
            // Bit 7 of the fraction: which half of the cell the streak is in.
            // This is what makes the wind a lean and not a staircase.
            let bit = BITS[((xs >> 7) & 1) as usize][(s & 3) as usize];
            let i = (r * self.cols + c) as u32;
            let last = d.nn as usize;
            // The sub-rows walk one way, so a cell can only be the one just
            // pushed — no search, and a cell never appears twice.
            if last > 0 && d.next[last - 1] == i {
                d.next_pat[last - 1] |= bit;
            } else if last < MAX_SPAN {
                d.next[last] = i;
                d.next_pat[last] = bit;
                d.nn += 1;
                d.head_vis |= k == 0;
            }
        }
    }

    fn splash_at(&mut self, col: i32) {
        if !self.splash_on || self.ground < 0 {
            return;
        }
        let i = (self.ground * self.cols + col.rem_euclid(self.cols.max(1))) as u32;
        // One splash per cell: a second on the same cell would expire first and
        // blank a ripple the other still claims, which is a dark cell the scene
        // says is lit. Restarting the one that is there is also what a second
        // drop landing in the same puddle looks like.
        //
        // Reuse a dead slot otherwise; a full table simply drops the splash.
        // The table holds one per streak, so "full" means every streak landed
        // on the same frame.
        let mut free = None;
        for (k, s) in self.splashes.iter().enumerate() {
            if s.live && s.i == i {
                free = Some(k);
                break;
            }
            if !s.live && free.is_none() {
                free = Some(k);
            }
        }
        if let Some(k) = free {
            let s = &mut self.splashes[k];
            s.i = i;
            s.stage = 0;
            s.t = 0;
            s.live = true;
        }
    }
}

/// A placeholder to `mem::replace` a drop with while it is borrowed out of the
/// Vec — `plan` needs `&self` for the geometry and `&mut Drop` for the answer,
/// and the two cannot both come through `self.drops[k]`.
const DEAD: Drop = Drop {
    x: 0,
    y: 0,
    vy: 1,
    len: 0,
    tier: 0,
    cells: [0; MAX_SPAN],
    n: 0,
    next: [0; MAX_SPAN],
    next_pat: [0; MAX_SPAN],
    nn: 0,
    head_vis: false,
    landed: false,
};

impl Saver for Rain {
    fn render(&mut self, s: &mut Surface<'_>) {
        self.dirty.clear();

        // 1. Move, and work out where every streak will be. Splashes are
        //    spawned here so they are live for step 4 in the same frame.
        for k in 0..self.drops.len() {
            let mut d = std::mem::replace(&mut self.drops[k], DEAD);
            d.y += d.vy;
            d.x += (d.vy * self.slant * SUB) >> 8;
            let head = Self::head_sub(&d);
            if !d.landed && (head >> 2) >= self.ground && TIERS[d.tier as usize].splash {
                d.landed = true;
                self.splash_at(d.x >> 8);
            }
            // Gone when the whole streak is at or below the ground row.
            if (head - d.len as i32 + 1) >> 2 >= self.ground {
                self.respawn(&mut d);
            }
            self.plan(&mut d);
            self.drops[k] = d;
        }

        // 2. Erase, for EVERY streak, before anything draws — see the module
        //    doc: interleaved, a crossing streak punches a hole in its
        //    neighbour and the hole persists until that neighbour moves.
        for k in 0..self.drops.len() {
            let (cells, n, next, nn) = {
                let d = &self.drops[k];
                (d.cells, d.n, d.next, d.nn)
            };
            for &i in &cells[..n as usize] {
                if next[..nn as usize].contains(&i) {
                    continue;
                }
                if self.grid.cell(i as usize) != Cell::CLEAR {
                    self.grid.set(i as usize, Cell::CLEAR);
                    self.dirty.push(i);
                }
            }
        }

        // 3. Draw.
        for k in 0..self.drops.len() {
            let (next, next_pat, nn, head_vis, tier) = {
                let d = &self.drops[k];
                (d.next, d.next_pat, d.nn, d.head_vis, d.tier)
            };
            let t = &TIERS[tier as usize];
            for j in 0..nn as usize {
                let i = next[j];
                let col = if j == 0 && head_vis { t.head } else { t.body };
                let c = Cell::new(font::BRAILLE[next_pat[j] as usize], col);
                if c != self.grid.cell(i as usize) {
                    self.grid.set(i as usize, c);
                    self.dirty.push(i);
                }
            }
            let d = &mut self.drops[k];
            d.cells = next;
            d.n = nn;
        }

        // 4. Splashes, on their own row. Re-asserted every frame rather than
        //    only on a stage change, so the ground row is always exactly what
        //    the live splashes say it is.
        for k in 0..self.splashes.len() {
            if !self.splashes[k].live {
                continue;
            }
            let (i, want) = {
                let sp = &mut self.splashes[k];
                sp.t += 1;
                if sp.t >= self.splash_frames {
                    sp.t = 0;
                    sp.stage += 1;
                }
                if sp.stage >= 2 {
                    sp.live = false;
                }
                (
                    sp.i,
                    match sp.stage {
                        0 => Cell::new(SPLASH_HIT, 7),
                        1 => Cell::new(SPLASH_FADE, 8),
                        _ => Cell::CLEAR,
                    },
                )
            };
            if want != self.grid.cell(i as usize) {
                self.grid.set(i as usize, want);
                self.dirty.push(i);
            }
        }

        // Row-major, so two cells in the same scanline collapse into one damage
        // run — `Damage::mark` merges only into the LAST run, and rain writes
        // its cells in drop order, which is scattered by construction.
        self.dirty.sort_unstable();
        self.grid.flush_sparse(s, &PAL, &self.dirty);
    }

    fn name(&self) -> &'static str {
        "rain"
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
    use crate::saver;

    /// 1080 is not a multiple of 16: 67 rows and an 8-line strip below them
    /// that belongs to no cell, which is the strip frame 0 has to paint.
    fn panel() -> Panel {
        Panel::new(1920, 1080, 1920)
    }

    fn rain() -> Rain {
        Rain::new(&panel(), 30)
    }

    /// What the scene SHOULD be: the union of the cells the streaks and the
    /// live splashes hold, and nothing else. Exact, not a ratio — a trail
    /// saturates rather than growing without bound, so a threshold waves it
    /// through (see the contract's TRAP 2).
    fn expected(r: &Rain) -> Vec<bool> {
        let mut want = vec![false; r.grid.cols() * r.grid.rows()];
        for d in &r.drops {
            for j in 0..d.n as usize {
                want[d.cells[j] as usize] = true;
            }
        }
        for s in r.splashes.iter().filter(|s| s.live) {
            want[s.i as usize] = true;
        }
        want
    }

    /// The tiers have to differ in speed, length and brightness at once, or
    /// "depth" is three shades of the same rain.
    #[test]
    fn the_tiers_read_as_depth() {
        let r = rain();
        for w in TIERS.windows(2) {
            assert!(w[0].speed_pct < w[1].speed_pct, "tiers do not speed up");
            assert!(w[0].len_min < w[1].len_min, "tiers do not lengthen");
            let lum = |c: u16| PAL_RGB[c as usize].iter().map(|&v| v as u32).sum::<u32>();
            assert!(lum(w[0].head) < lum(w[1].body), "tiers do not brighten");
        }
        // And all three have to actually be on screen: a draw table that never
        // rolled a near streak would pass everything above.
        let mut seen = [0usize; TIERS.len()];
        for d in &r.drops {
            seen[d.tier as usize] += 1;
        }
        for (t, &n) in seen.iter().enumerate() {
            assert!(n > r.drops.len() / 20, "tier {t} has only {n} streaks");
        }
    }

    /// T1 + T2. Frame 0 covers the whole panel including the bottom strip, and
    /// after that every scanline that changed is reported — plus the check a
    /// framebuffer diff is structurally blind to: a cell written into the grid
    /// but left out of `dirty` is never blitted, so the panel freezes there
    /// forever and no pixel comparison can see it.
    #[test]
    fn damage_covers_every_changed_scanline() {
        let p = panel();
        let mut r = rain();
        let mut buf = vec![0u32; p.buf_len()];
        let mut prev = buf.clone();

        let d = saver::frame(&mut r, &mut buf, &p);
        assert_eq!(d.rows(), p.h, "frame 0 must paint the whole panel");
        assert!(
            buf.iter().filter(|&&px| px != 0).count() > 10_000,
            "frame 0 painted nothing"
        );

        let cells = r.grid.cols() * r.grid.rows();
        let mut rows = Vec::new();
        for n in 1..1500 {
            prev.copy_from_slice(&buf);
            let d = saver::frame(&mut r, &mut buf, &p);
            for y in 0..p.h {
                if prev[y * p.w..][..p.w] == buf[y * p.w..][..p.w] {
                    continue;
                }
                assert!(
                    dump::row_reported(&prev[y * p.w..][..p.w], &buf[y * p.w..][..p.w], y, &d),
                    "frame {n}: scanline {y} changed outside every reported rect"
                );
            }
            for i in 0..cells {
                assert_eq!(
                    r.grid.cell(i),
                    r.grid.cells()[i],
                    "frame {n}: cell {i} was written but not reported"
                );
            }
            rows.push(d.rows());
        }
        // Non-vacuous: rain that never moved would satisfy everything above.
        rows.sort_unstable();
        assert!(rows[rows.len() / 2] > 0, "no damage at all after frame 0");
    }

    /// T3. `render` allocates nothing: `dirty` is the only per-frame buffer and
    /// a Vec that never grows past its reserve never reallocates. The drop and
    /// splash tables are fixed-length for the life of the saver, so their
    /// capacity is checked too — a `push` in the frame loop would move them.
    #[test]
    fn render_never_allocates() {
        let p = panel();
        let mut r = rain();
        let mut buf = vec![0u32; p.buf_len()];
        let reserved = r.dirty.capacity();
        assert!(reserved > 0, "nothing was reserved for the frame loop");
        let (nd, ns) = (r.drops.capacity(), r.splashes.capacity());

        let mut worst = 0;
        for _ in 0..50_000 {
            saver::frame(&mut r, &mut buf, &p);
            worst = worst.max(r.dirty.len());
            assert_eq!(
                r.dirty.capacity(),
                reserved,
                "`dirty` grew past its reserve: the render path allocated"
            );
            assert_eq!((r.drops.capacity(), r.splashes.capacity()), (nd, ns));
        }
        // Non-vacuous: a reserve nothing ever fills proves nothing. A frame has
        // to hold more than a whole near streak's worth of cells.
        assert!(worst > MAX_SPAN, "`dirty` never held a whole streak");
        assert!(
            worst <= reserved,
            "{worst} entries in a reserve of {reserved}"
        );
    }

    /// T4, first half: the scene is EXACTLY the union of what the streaks and
    /// splashes stamp. A cell lit that no streak claims is a trail; a cell dark
    /// that one does claim is a hole punched by a crossing streak.
    #[test]
    fn the_scene_is_exactly_what_the_drops_stamp() {
        let p = panel();
        let mut r = rain();
        let mut buf = vec![0u32; p.buf_len()];
        let mut splashed = 0;
        for n in 0..3000 {
            saver::frame(&mut r, &mut buf, &p);
            let want = expected(&r);
            splashed += r.splashes.iter().filter(|s| s.live).count();
            for (i, &w) in want.iter().enumerate() {
                let lit = r.grid.cell(i).glyph() != font::BLANK as usize;
                assert_eq!(lit, w, "frame {n}: cell {i} lit={lit} want={w}");
            }
        }
        // Non-vacuous on both halves: streaks have to be on screen, and
        // splashes have to have happened at all.
        assert!(
            expected(&r).iter().filter(|&&w| w).count() > r.drops.len(),
            "the rain drained away"
        );
        assert!(splashed > 0, "nothing ever splashed");
    }

    /// T4, second half: it falls DOWNWARD. Flip one sign and you get rain
    /// rising off the floor, which every other test here passes.
    #[test]
    fn the_rain_falls_downward() {
        let p = panel();
        let mut r = rain();
        let mut buf = vec![0u32; p.buf_len()];
        let mut y: Vec<i32> = r.drops.iter().map(|d| d.y).collect();
        let mut respawns = 0;
        for n in 0..2000 {
            saver::frame(&mut r, &mut buf, &p);
            for (k, d) in r.drops.iter().enumerate() {
                if d.y < y[k] {
                    // The only legal way back up the screen is a respawn, which
                    // starts the whole streak above the top edge.
                    assert!(
                        d.y < 0,
                        "frame {n}: drop {k} moved up from {} to {}",
                        y[k],
                        d.y
                    );
                    respawns += 1;
                } else {
                    assert!(d.y > y[k], "frame {n}: drop {k} stalled at {}", d.y);
                }
                y[k] = d.y;
            }
        }
        // And they have to reach the bottom and come back, or "downward" is
        // one long fall nothing ever completes.
        assert!(respawns > r.drops.len(), "only {respawns} respawns");
    }
}
