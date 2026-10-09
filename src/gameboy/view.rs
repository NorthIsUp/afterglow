//! The view the panel shows: the Game Boy's 160x144 screen filling the
//! height, and the rest of a wider panel filled around it.
//!
//! `frame` fills the sides with each row's edge colour, dimmed: an ambient
//! glow that follows the picture. `wide` shows more of the game's world
//! there instead. For Pokémon that is the overworld drawn from the ROM's own
//! map data (`world.rs`); for anything else it is the trick of `WideGB`,
//! after `WideNES`: remember the background as the screen scrolls past it, and keep
//! what scrolled off where it was. Wherever neither knows, the glow.
//!
//! A view pixel is RGB555, bit 15 marking it dimmed.

use mizu_core::GameBoy;

use super::carts::Pilot;
use super::engine::{H, SCREEN_W};
use super::world::World;

pub const DIM: u16 = 0x8000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Frame,
    Wide,
}

pub struct Composer {
    w: usize,
    mode: Mode,
    stitch: Stitch,
    world: Option<Box<World>>,
    /// What filled the sides last frame, the view as it was then, and the
    /// frames left of a crossfade from that view's sides.
    source: Source,
    last: Vec<u16>,
    from: Vec<u16>,
    fade: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Glow,
    Stitch,
    /// The world view of one map: a new map crossfades too.
    World(Option<u8>),
}

/// About half a second at 59.7 Hz.
const FADE: u32 = 30;

impl Composer {
    pub fn new(w: usize, mode: Mode, pilot: Pilot) -> Self {
        let world = match (mode, pilot) {
            (Mode::Wide, Pilot::Pokemon(r)) => Some(Box::new(World::new(r))),
            _ => None,
        };
        Self {
            w,
            mode,
            stitch: Stitch::new(),
            world,
            source: Source::Glow,
            last: vec![0; w * H],
            from: vec![0; w * H],
            fade: 0,
        }
    }

    /// The world view, for the tests that check it lines up.
    #[cfg(test)]
    pub fn world(&self) -> Option<&World> {
        self.world.as_deref()
    }

    pub fn compose(&mut self, gb: &mut GameBoy, tone: &[u16], out: &mut [u16]) {
        let w = self.w;
        let x0 = (w - SCREEN_W) / 2;
        let lcd = gb.screen_buffer();
        for y in 0..H {
            let row = &mut out[y * w..][..w];
            let src = &lcd[y * SCREEN_W..][..SCREEN_W];
            for (o, &p) in row[x0..x0 + SCREEN_W].iter_mut().zip(src) {
                *o = tone[p as usize];
            }
        }
        glow(out, w, x0);
        if self.mode == Mode::Frame || w == SCREEN_W {
            return;
        }
        // Pokémon has the real map, or the glow: never a remembered
        // background, which in the intro is a blank cream screen slid about.
        let source = match self
            .world
            .as_mut()
            .map(|world| world.compose(gb, tone, out, w))
        {
            Some(true) => Source::World(self.world.as_ref().and_then(|w| w.map())),
            None if self.stitch.compose(gb, tone, out, w) => Source::Stitch,
            Some(false) | None => Source::Glow,
        };
        if source != self.source {
            self.source = source;
            self.from.copy_from_slice(&self.last);
            self.fade = FADE;
        }
        if self.fade > 0 {
            self.fade -= 1;
            let t = FADE - self.fade;
            for y in 0..H {
                let (row, old) = (&mut out[y * w..][..w], &self.from[y * w..][..w]);
                for x in (0..x0).chain(x0 + SCREEN_W..w) {
                    row[x] = blend(old[x], row[x], t, FADE);
                }
            }
        }
        self.last.copy_from_slice(&out[..w * H]);
    }
}

/// `a` toward `b`, `t` of `n` of the way, dimming applied before the mix.
fn blend(a: u16, b: u16, t: u32, n: u32) -> u16 {
    let ch = |c: u16, s: u16| {
        let v = u32::from(c >> s & 31);
        if c & DIM != 0 {
            v * 85 / 256
        } else {
            v
        }
    };
    let mix = |s: u16| ((ch(a, s) * (n - t) + ch(b, s) * t) / n) as u16;
    mix(0) | mix(5) << 5 | mix(10) << 10
}

/// Columns of the screen's edge the glow averages, and rows it blurs over.
const GLOW_COLS: usize = 8;
const GLOW_ROWS: usize = 6;

/// Each side, outside the screen, in the average colour of the screen's
/// edge beside it, blurred down the rows and dimmed: one row's edge pixel
/// alone stripes the sides with every brick and letter.
fn glow(out: &mut [u16], w: usize, x0: usize) {
    if x0 == 0 {
        return;
    }
    let sum = |row: &[u16]| {
        row.iter().fold([0u32; 3], |a, &c| {
            [
                a[0] + u32::from(c & 31),
                a[1] + u32::from(c >> 5 & 31),
                a[2] + u32::from(c >> 10 & 31),
            ]
        })
    };
    let mut edges = [[[0u32; 3]; 2]; H];
    for (y, e) in edges.iter_mut().enumerate() {
        let row = &out[y * w..][..w];
        *e = [
            sum(&row[x0..x0 + GLOW_COLS]),
            sum(&row[x0 + SCREEN_W - GLOW_COLS..x0 + SCREEN_W]),
        ];
    }
    for y in 0..H {
        let (a, b) = (y.saturating_sub(GLOW_ROWS), (y + GLOW_ROWS + 1).min(H));
        let mut acc = [[0u32; 3]; 2];
        for e in &edges[a..b] {
            for side in 0..2 {
                for ch in 0..3 {
                    acc[side][ch] += e[side][ch];
                }
            }
        }
        let n = ((b - a) * GLOW_COLS) as u32;
        let px = |c: [u32; 3]| {
            (c[0] / n) as u16 | ((c[1] / n) as u16) << 5 | ((c[2] / n) as u16) << 10 | DIM
        };
        let row = &mut out[y * w..][..w];
        row[..x0].fill(px(acc[0]));
        row[x0 + SCREEN_W..].fill(px(acc[1]));
    }
}

const CW: usize = 1024;
const CH: usize = 512;
const UNKNOWN: u16 = 0xFFFF;

/// The background the screen has scrolled past, in a ring of world pixels
/// addressed by where the camera has been.
pub struct Stitch {
    canvas: Vec<u16>,
    cam: (usize, usize),
    prev: Option<(u8, u8)>,
    /// Frames since the memory last started over; it is shown only once
    /// a scene has held a second, so a one-frame fill never flashes up.
    steady: u32,
}

const STEADY: u32 = 60;

impl Stitch {
    fn new() -> Self {
        Self {
            canvas: vec![UNKNOWN; CW * CH],
            cam: (0, 0),
            prev: None,
            steady: 0,
        }
    }

    fn forget(&mut self) {
        self.canvas.fill(UNKNOWN);
        self.prev = None;
        self.steady = 0;
    }

    /// The scroll most lines were drawn at. Lines at another one are a
    /// status bar or a raster effect, not the world.
    fn scroll(lines: &[[u8; 2]]) -> (u8, u8) {
        let mut best = (0, (0, 0));
        for (i, l) in lines.iter().enumerate() {
            if lines[..i].contains(l) {
                continue;
            }
            let n = lines[i..].iter().filter(|m| *m == l).count();
            if n > best.0 {
                best = (n, (l[0], l[1]));
            }
        }
        best.1
    }

    /// True when it drew anything remembered.
    pub fn compose(&mut self, gb: &GameBoy, tone: &[u16], out: &mut [u16], w: usize) -> bool {
        if !gb.lcd_on() {
            self.forget();
            return false;
        }
        let (bg, lines) = (gb.bg_buffer(), gb.line_scroll());
        let (sx, sy) = Self::scroll(lines);
        if let Some((px, py)) = self.prev {
            let dx = i32::from(sx.wrapping_sub(px) as i8);
            let dy = i32::from(sy.wrapping_sub(py) as i8);
            self.cam.0 = self.cam.0.wrapping_add_signed(dx as isize) % CW;
            self.cam.1 = self.cam.1.wrapping_add_signed(dy as isize) % CH;
        }
        self.prev = Some((sx, sy));
        let at =
            |cam: (usize, usize), x: usize, y: usize| ((cam.1 + y) % CH) * CW + (cam.0 + x) % CW;
        // A screen that disagrees with most of what is remembered under it
        // is a new room, not a scroll.
        let (mut known, mut differ) = (0usize, 0usize);
        for y in (0..H).step_by(3) {
            if lines[y] != [sx, sy] {
                continue;
            }
            for x in (0..SCREEN_W).step_by(3) {
                let p = bg[y * SCREEN_W + x];
                let c = self.canvas[at(self.cam, x, y)];
                if p & DIM == 0 && c != UNKNOWN {
                    known += 1;
                    differ += usize::from(c != tone[p as usize]);
                }
            }
        }
        // A blank or faded-out screen says nothing about the world: neither
        // a new scene nor anything to remember.
        let first = bg[lines.iter().position(|l| *l == [sx, sy]).unwrap_or(0) * SCREEN_W];
        let blank = (0..H)
            .filter(|&y| lines[y] == [sx, sy])
            .all(|y| bg[y * SCREEN_W..][..SCREEN_W].iter().all(|&p| p == first));
        if blank {
            return self.draw(out, w);
        }
        if known > 200 && differ * 3 > known {
            self.canvas.fill(UNKNOWN);
            self.steady = 0;
        }
        self.steady = self.steady.saturating_add(1);
        for y in 0..H {
            if lines[y] != [sx, sy] {
                continue;
            }
            for x in 0..SCREEN_W {
                let p = bg[y * SCREEN_W + x];
                if p & DIM == 0 {
                    self.canvas[at(self.cam, x, y)] = tone[p as usize];
                }
            }
        }
        self.draw(out, w)
    }

    /// The remembered background into the sides, once the scene is steady.
    fn draw(&self, out: &mut [u16], w: usize) -> bool {
        if self.steady < STEADY {
            return false;
        }
        let x0 = (w - SCREEN_W) / 2;
        let mut any = false;
        for y in 0..H {
            let row = &mut out[y * w..][..w];
            for (vx, o) in row.iter_mut().enumerate() {
                if (x0..x0 + SCREEN_W).contains(&vx) {
                    continue;
                }
                let wx = (self.cam.0 + CW * 4 + vx).wrapping_sub(x0) % CW;
                let c = self.canvas[((self.cam.1 + y) % CH) * CW + wx];
                if c != UNKNOWN {
                    *o = c;
                    any = true;
                }
            }
        }
        any
    }
}
