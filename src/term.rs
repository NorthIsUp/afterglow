//! The terminal host: the same frames, printed into the terminal this runs in.
//!
//! A third host beside DRM and the dump. It drives the same `saver::Driver`
//! into a pixel buffer nobody reads, then prints the grid that frame flushed,
//! scaled as the panel shows it: a character per cell where cells are
//! glyph-shaped, two per character (`▀`, top as the foreground, bottom as the
//! background) where they are square. Truecolor only; every terminal worth
//! running a screensaver in has it.

use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use crate::font;
use crate::grid::{Cell, Grid, Shape};
use crate::mirror::Mirror;
use crate::saver::{self, Driver, Saver};
use crate::surface::Panel;
use crate::{Config, SIGNALLED};

/// What one terminal character cell stands in for, in framebuffer pixels. The
/// font's own glyph size, so a saver sized for it gets one cell per character.
const CHAR_W: usize = 8;
const CHAR_H: usize = 16;

/// Upscaled panels stop here. A 24px-cell saver in a wide terminal would
/// otherwise ask for a buffer of hundreds of megabytes; past this its cells
/// are simply bigger than a character.
const MAX_PANEL: usize = 4096;

const ENTER: &[u8] = b"\x1b[?1049h\x1b[?25l\x1b[2J";
const LEAVE: &[u8] = b"\x1b[0m\x1b[?25h\x1b[?1049l";

/// Terminal size in characters, from the tty on stdout.
fn size() -> Option<(usize, usize)> {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    let ok = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) } == 0;
    (ok && ws.ws_col > 0 && ws.ws_row > 0).then(|| (ws.ws_col.into(), ws.ws_row.into()))
}

/// A cell at least 1.5x taller than wide reads as a character; anything
/// squarer is a pixel, and two of those stack into one character.
fn glyph_cells(s: Shape) -> bool {
    2 * s.cell_h >= 3 * s.cell_w
}

/// The panel that gives `g`'s saver about one grid cell per character (or per
/// half character), when the base panel gave it fewer. A saver with 16x32
/// cells built for an 8x16-per-character panel fills a quarter of the
/// terminal; built for twice the panel it fills all of it. None when the grid
/// already fits — growing a grid that is sized off the panel, as the
/// ascii.rest pieces are, gains nothing.
fn upscale(g: &Grid, base: &Panel, cols: usize, rows: usize) -> Option<Panel> {
    let want_rows = if glyph_cells(g.shape_of()) {
        rows
    } else {
        rows * 2
    };
    let permille = (cols * 1000 / g.cols()).min(want_rows * 1000 / g.rows());
    let w = (base.w * permille / 1000).min(MAX_PANEL);
    let h = (base.h * permille / 1000).min(MAX_PANEL);
    (permille > 1000 && (w, h) != (base.w, base.h)).then(|| Panel::new(w, h, w))
}

/// Build saver `name` for a `cols` x `rows` terminal: `saver::Driver`'s
/// `place`. A saver whose cells are bigger than a character at the base panel
/// is built again for a bigger one; only that build is kept and announced.
fn place(name: &str, cols: usize, rows: usize, fps: u32) -> (Panel, Box<dyn Saver>) {
    let base = Panel::new(cols * CHAR_W, rows * CHAR_H, cols * CHAR_W);
    let s = saver::make(name, &base, fps);
    match upscale(s.grid(), &base, cols, rows) {
        Some(p) => (p, saver::make(name, &p, fps)),
        None => (base, s),
    }
}

/// One terminal character: what it prints and in which colours.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Tc {
    ch: char,
    fg: u32,
    bg: u32,
}

/// The terminal's cells, this frame and the one before, and the bytes that
/// turn one into the other.
pub struct Screen {
    cols: usize,
    rows: usize,
    cur: Vec<Tc>,
    prev: Vec<Tc>,
    /// The grid column under each character's centre, and the grid row under
    /// each character's (or half character's) centre; `u32::MAX` off the grid.
    xs: Vec<u32>,
    ys: Vec<u32>,
    out: Vec<u8>,
    /// Nothing on the terminal is known, so every cell is written.
    fresh: bool,
}

/// The grid cell under the centre of slot `t` of `n` spread across `len`
/// panel pixels, for cells `cell` pixels wide, drawn `shift` pixels before
/// the panel's edge, of which there are `count`; `u32::MAX` in the margin past
/// the last.
#[inline]
fn sample(t: usize, n: usize, len: usize, (cell, shift): (usize, usize), count: usize) -> u32 {
    let g = ((2 * t + 1) * len / (2 * n) + shift) / cell;
    if g < count {
        g as u32
    } else {
        u32::MAX
    }
}

impl Screen {
    pub fn new(cols: usize, rows: usize) -> Self {
        let blank = Tc {
            ch: ' ',
            fg: 0,
            bg: 0,
        };
        Self {
            cols,
            rows,
            cur: vec![blank; cols * rows],
            prev: vec![blank; cols * rows],
            xs: Vec::with_capacity(cols),
            ys: Vec::with_capacity(rows * 2),
            // Room for a full repaint with a cursor move and both colours on
            // every cell, so a busy frame does not grow it.
            out: Vec::with_capacity(cols * rows * 48 + 64),
            fresh: true,
        }
    }

    /// Lay a flushed grid's cells onto the terminal as the panel shows them:
    /// each character, or each half of one, shows the cell under its centre.
    /// A close-up's cells span several characters and a wide view's are
    /// sampled, so a zoom reads as a zoom here as on the glass.
    pub fn compose(&mut self, g: &Grid, panel: &Panel, pal: &[u32]) {
        let shape = (g.shape_of(), g.shift_of());
        self.compose_cells(g.cells(), shape, (panel.w, panel.h), g.ground(), pal);
    }

    fn compose_cells(
        &mut self,
        cells: &[Cell],
        (g, (dx, dy)): (Shape, (usize, usize)),
        (pw, ph): (usize, usize),
        ground: u32,
        pal: &[u32],
    ) {
        let lit = |c: Cell| {
            if c.glyph() == usize::from(font::BLANK) {
                ground
            } else {
                pal[c.colour()]
            }
        };
        let glyphs = glyph_cells(g);
        let vrows = if glyphs { self.rows } else { self.rows * 2 };
        let (cols, rows) = (self.cols, self.rows);
        self.xs.clear();
        self.xs
            .extend((0..cols).map(|x| sample(x, cols, pw, (g.cell_w, dx), g.cols)));
        self.ys.clear();
        self.ys
            .extend((0..vrows).map(|y| sample(y, vrows, ph, (g.cell_h, dy), g.rows)));
        let at = |gx: u32, gy: u32| cells[gy as usize * g.cols + gx as usize];
        let off = |v: u32| v == u32::MAX;
        for y in 0..rows {
            for x in 0..cols {
                let gx = self.xs[x];
                let tc = if glyphs {
                    let gy = self.ys[y];
                    if off(gx) || off(gy) {
                        None
                    } else {
                        let c = at(gx, gy);
                        Some(Tc {
                            ch: font::CHARS[c.glyph()],
                            fg: pal[c.colour()],
                            bg: ground,
                        })
                    }
                } else {
                    let (top, bot) = (self.ys[2 * y], self.ys[2 * y + 1]);
                    if off(gx) || off(top) {
                        None
                    } else {
                        Some(Tc {
                            ch: '▀',
                            fg: lit(at(gx, top)),
                            bg: if off(bot) { ground } else { lit(at(gx, bot)) },
                        })
                    }
                };
                self.cur[y * cols + x] = tc.unwrap_or(Tc {
                    ch: ' ',
                    fg: ground,
                    bg: ground,
                });
            }
        }
    }

    /// The escapes that bring the terminal from the last emitted frame to the
    /// composed one: only cells that changed, a cursor move only where the
    /// writes are not contiguous, a colour only where it differs from the last.
    pub fn emit(&mut self) -> &[u8] {
        self.out.clear();
        let mut at = usize::MAX;
        let (mut fg, mut bg) = (None, None);
        for (i, (&c, &p)) in self.cur.iter().zip(&self.prev).enumerate() {
            if !self.fresh && c == p {
                continue;
            }
            let (x, y) = (i % self.cols, i / self.cols);
            if at != i {
                let _ = write!(self.out, "\x1b[{};{}H", y + 1, x + 1);
            }
            if fg != Some(c.fg) {
                let _ = write!(
                    self.out,
                    "\x1b[38;2;{};{};{}m",
                    c.fg >> 16 & 0xFF,
                    c.fg >> 8 & 0xFF,
                    c.fg & 0xFF
                );
                fg = Some(c.fg);
            }
            if bg != Some(c.bg) {
                let _ = write!(
                    self.out,
                    "\x1b[48;2;{};{};{}m",
                    c.bg >> 16 & 0xFF,
                    c.bg >> 8 & 0xFF,
                    c.bg & 0xFF
                );
                bg = Some(c.bg);
            }
            self.out
                .extend_from_slice(c.ch.encode_utf8(&mut [0; 4]).as_bytes());
            // The last column leaves the cursor in the terminal's pending-wrap
            // state, where the next character lands on the next row on some
            // terminals and overwrites this one on others. Re-address instead.
            at = if x + 1 == self.cols {
                usize::MAX
            } else {
                i + 1
            };
        }
        std::mem::swap(&mut self.cur, &mut self.prev);
        self.fresh = false;
        &self.out
    }
}

/// What `enter` took from the terminal, for `restore` to give back.
struct Tty {
    termios: Option<libc::termios>,
    /// The real stderr, and the file fd 2 writes into meanwhile.
    stderr: Option<(OwnedFd, File)>,
}

/// The one static, because the panic hook can reach nothing else and
/// `panic = "abort"` runs no destructor that could.
static TTY: Mutex<Option<Tty>> = Mutex::new(None);

/// Take the terminal: alternate screen, no cursor, no echo, keys readable
/// without Enter. Returns whether stdin is a tty whose keys can be read.
fn enter() -> bool {
    let termios = unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        (libc::tcgetattr(libc::STDIN_FILENO, &raw mut t) == 0).then_some(t)
    };
    let keys = termios.is_some_and(|mut t| {
        // ISIG stays on: Ctrl-C is still SIGINT, which main already turns
        // into a clean stop.
        t.c_lflag &= !(libc::ECHO | libc::ICANON);
        t.c_cc[libc::VMIN] = 0;
        t.c_cc[libc::VTIME] = 0;
        unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw const t) == 0 }
    });
    let tty = Tty {
        termios,
        stderr: hold_stderr(),
    };
    *TTY.lock().unwrap_or_else(PoisonError::into_inner) = Some(tty);
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(ENTER).and_then(|()| out.flush());
    keys
}

/// Point fd 2 at an unlinked file until `restore` replays it. Anything written
/// to the terminal while the alternate screen is up lands on top of the
/// picture, and the diff never repaints cells it thinks are unchanged — the
/// mirror thread's "listening" line would sit there for good. None, leaving
/// stderr alone, when it is not a terminal or no file can be made.
fn hold_stderr() -> Option<(OwnedFd, File)> {
    if unsafe { libc::isatty(libc::STDERR_FILENO) } != 1 {
        return None;
    }
    let path = std::env::temp_dir().join(format!("afterglow-stderr-{}", std::process::id()));
    // create_new so a planted symlink is refused; std opens it O_CLOEXEC.
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .ok()?;
    let _ = std::fs::remove_file(&path);
    let real = unsafe { libc::fcntl(libc::STDERR_FILENO, libc::F_DUPFD_CLOEXEC, 3) };
    if real < 0 {
        return None;
    }
    let real = unsafe { OwnedFd::from_raw_fd(real) };
    (unsafe { libc::dup2(file.as_raw_fd(), libc::STDERR_FILENO) } >= 0).then_some((real, file))
}

/// Give the terminal back, then replay what stderr said meanwhile. Idempotent,
/// and called from the panic hook as well as on the way out: `panic = "abort"`
/// skips every destructor.
fn restore() {
    let Some(tty) = TTY.lock().unwrap_or_else(PoisonError::into_inner).take() else {
        return;
    };
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(LEAVE).and_then(|()| out.flush());
    if let Some(t) = tty.termios {
        unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw const t) };
    }
    if let Some((real, mut held)) = tty.stderr {
        unsafe { libc::dup2(real.as_raw_fd(), libc::STDERR_FILENO) };
        if held.seek(SeekFrom::Start(0)).is_ok() {
            let _ = std::io::copy(&mut held, &mut std::io::stderr());
        }
    }
}

struct Restore;

impl Drop for Restore {
    fn drop(&mut self) {
        restore();
    }
}

/// `q` was pressed. Never blocks: VMIN=0 makes an empty read return at once.
fn quit_key() -> bool {
    let mut buf = [0u8; 16];
    let n = unsafe { libc::read(libc::STDIN_FILENO, buf.as_mut_ptr().cast(), buf.len()) };
    n > 0 && buf[..n as usize].contains(&b'q')
}

pub fn run(cfg: &Config, mirror: &Mirror) -> Result<(), String> {
    let (mut cols, mut rows) =
        size().ok_or("SAVER_TERM=1 needs a terminal on stdout (TIOCGWINSZ failed)")?;
    let fps = cfg.fps;
    let mut d = Driver::new(mirror, fps, |n| place(n, cols, rows, fps));
    let mut buf = vec![0u32; d.panel().buf_len()];
    let mut screen = Screen::new(cols, rows);

    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        hook(info);
    }));
    let keys = enter();
    let _restore = Restore;

    while !(SIGNALLED.load(Ordering::Relaxed) || (keys && quit_key())) {
        let t0 = Instant::now();
        // Polled rather than SIGWINCH: one ioctl a frame, and no second signal
        // handler racing the first.
        let resized = size().filter(|&s| s != (cols, rows));
        if let Some((c, r)) = resized {
            (cols, rows) = (c, r);
            d.rebuild(mirror, |n| place(n, cols, rows, fps));
            screen = Screen::new(cols, rows);
        }
        if d.switch(t0, mirror, |n| place(n, cols, rows, fps)) || resized.is_some() {
            buf.resize(d.panel().buf_len(), 0);
        }
        d.frame(&mut buf, mirror);
        screen.compose(d.saver().grid(), d.panel(), d.saver().palette());
        let mut out = std::io::stdout().lock();
        out.write_all(screen.emit())
            .and_then(|()| out.flush())
            .map_err(|e| format!("write terminal: {e}"))?;
        drop(out);
        d.pace(mirror, t0);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testalloc::allocs_during;

    const PAL: [u32; 3] = [0x000000, 0xFF0000, 0x00FF00];

    #[test]
    fn every_glyph_has_a_char() {
        assert_eq!(font::CHARS.len(), font::GLYPHS.len());
        assert_eq!(font::CHARS[usize::from(font::BLANK)], ' ');
        assert_eq!(font::CHARS[usize::from(font::SOLID)], '█');
        let dots: Vec<char> = font::HALFTONE
            .iter()
            .map(|&g| font::CHARS[usize::from(g)])
            .collect();
        assert_eq!(dots, [' ', '·', '•', '●']);
        for (i, &c) in font::ASCII.iter().enumerate() {
            assert_eq!(font::CHARS[usize::from(c)], char::from(0x20 + i as u8));
        }
        // A braille pattern that is a half block prints as the block: both are
        // filled quadrants on the panel, and the block is what reads as one.
        assert_eq!(font::CHARS[usize::from(font::UPPER)], '▀');
    }

    fn ascii(c: u8) -> u16 {
        font::ASCII[usize::from(c - 0x20)]
    }

    /// A grid of `cols x rows` cells `cw x ch` on a panel exactly that size.
    fn fitted(cols: usize, rows: usize, cw: usize, ch: usize) -> (Shape, (usize, usize)) {
        let s = Shape {
            cols,
            rows,
            cell_w: cw,
            cell_h: ch,
        };
        (s, (cols * cw, rows * ch))
    }

    #[test]
    fn glyph_cells_print_one_char_each() {
        let mut s = Screen::new(4, 3);
        let cells = [Cell::new(ascii(b'A'), 1), Cell::new(ascii(b'B'), 2)];
        // Two cells on a panel four characters wide: the rest is margin.
        let (g, _) = fitted(2, 1, CHAR_W, CHAR_H);
        s.compose_cells(
            &cells,
            (g, (0, 0)),
            (4 * CHAR_W, 3 * CHAR_H),
            0x111111,
            &PAL,
        );
        let row: Vec<char> = s.cur[0..4].iter().map(|t| t.ch).collect();
        assert_eq!(row, ['A', 'B', ' ', ' ']);
        assert_eq!(s.cur[0].fg, 0xFF0000);
        assert_eq!(s.cur[1].fg, 0x00FF00);
        assert!(s.cur[0..2].iter().all(|t| t.bg == 0x111111));
        // Outside the grid is ground, not black.
        assert_eq!(s.cur[4].bg, 0x111111);
    }

    #[test]
    fn square_cells_stack_two_rows_per_char() {
        let mut s = Screen::new(2, 2);
        // A 2x3 grid: the odd last row pairs with the ground.
        let cells = [
            Cell::new(font::SOLID, 1),
            Cell::CLEAR,
            Cell::new(font::BLANK, 2),
            Cell::new(font::HALFTONE[1], 2),
            Cell::new(font::SOLID, 2),
            Cell::new(font::SOLID, 1),
        ];
        let (g, _) = fitted(2, 3, CHAR_W, CHAR_H / 2);
        s.compose_cells(
            &cells,
            (g, (0, 0)),
            (2 * CHAR_W, 2 * CHAR_H),
            0x111111,
            &PAL,
        );
        let t = |i: usize| (s.cur[i].ch, s.cur[i].fg, s.cur[i].bg);
        assert_eq!(t(0), ('▀', 0xFF0000, 0x111111));
        // A blank glyph is ground whatever its colour; any lit one is its colour.
        assert_eq!(t(1), ('▀', 0x111111, 0x00FF00));
        assert_eq!(t(2), ('▀', 0x00FF00, 0x111111));
        assert_eq!(t(3), ('▀', 0xFF0000, 0x111111));
    }

    /// The ascii.rest tour zooms by cell size. A close-up's cells are wider
    /// than a character and must span several; a wide view's are narrower and
    /// must be sampled across the whole picture. Composing cells 1:1 showed
    /// the first as a small window in the middle of the terminal and cropped
    /// the second to its centre.
    #[test]
    fn a_zoom_reads_as_a_zoom() {
        let panel = (4 * CHAR_W, CHAR_H);
        let row = |s: &Screen| s.cur.iter().map(|t| t.ch).collect::<String>();
        let mut s = Screen::new(4, 1);

        let close: Vec<Cell> = b"WX".iter().map(|&c| Cell::new(ascii(c), 1)).collect();
        let (g, _) = fitted(2, 1, 2 * CHAR_W, 2 * CHAR_H);
        s.compose_cells(&close, (g, (0, 0)), panel, 0, &PAL);
        assert_eq!(row(&s), "WWXX");

        let wide: Vec<Cell> = b"ABCDEFGH"
            .iter()
            .map(|&c| Cell::new(ascii(c), 1))
            .collect();
        let (g, _) = fitted(8, 1, CHAR_W / 2, CHAR_H);
        s.compose_cells(&wide, (g, (0, 0)), panel, 0, &PAL);
        assert_eq!(row(&s), "BDFH");
    }

    #[test]
    fn an_unchanged_frame_writes_no_cells() {
        let mut s = Screen::new(3, 2);
        let cells = [Cell::new(ascii(b'A'), 1); 6];
        let (g, p) = fitted(3, 2, CHAR_W, CHAR_H);
        s.compose_cells(&cells, (g, (0, 0)), p, 0, &PAL);
        let first = s.emit().len();
        assert!(first > 0);
        s.compose_cells(&cells, (g, (0, 0)), p, 0, &PAL);
        assert_eq!(s.emit(), []);

        let mut moved = cells;
        moved[4] = Cell::new(ascii(b'B'), 1);
        s.compose_cells(&moved, (g, (0, 0)), p, 0, &PAL);
        // A cursor move and the colours, which are re-sent once per frame.
        let out = String::from_utf8(s.emit().to_vec()).unwrap();
        assert!(out.starts_with("\x1b[2;2H"), "{out:?}");
        assert!(out.ends_with('B'), "{out:?}");
        assert_eq!(out.matches('\x1b').count(), 3, "{out:?}");
    }

    #[test]
    fn contiguous_writes_skip_the_cursor_move_but_not_past_a_row_end() {
        let mut s = Screen::new(2, 2);
        let cells = [Cell::new(ascii(b'A'), 1); 4];
        let (g, p) = fitted(2, 2, CHAR_W, CHAR_H);
        s.compose_cells(&cells, (g, (0, 0)), p, 0, &PAL);
        let out = String::from_utf8(s.emit().to_vec()).unwrap();
        assert_eq!(out.matches('H').count(), 2, "{out:?}");
    }

    #[test]
    fn compose_and_emit_never_allocate() {
        let mut s = Screen::new(40, 10);
        let mut cells = vec![Cell::new(ascii(b'A'), 1); 400];
        let (g, p) = fitted(40, 10, CHAR_W, CHAR_H);
        s.compose_cells(&cells, (g, (0, 0)), p, 0, &PAL);
        s.emit();
        for (i, c) in cells.iter_mut().enumerate() {
            *c = Cell::new(ascii(b'B'), (i % 3) as u16);
        }
        let n = allocs_during(|| {
            s.compose_cells(&cells, (g, (0, 0)), p, 0x123456, &PAL);
            std::hint::black_box(s.emit());
        });
        assert_eq!(n, 0);
    }

    #[test]
    fn a_big_celled_saver_is_upscaled_to_fill_the_terminal() {
        let base = Panel::new(160 * CHAR_W, 50 * CHAR_H, 160 * CHAR_W);
        let g = Grid::with_aspect(&base, 16, 32, 100);
        let p = upscale(&g, &base, 160, 50).expect("80x25 grid in a 160x50 terminal");
        let g = Grid::with_aspect(&p, 16, 32, 100);
        assert_eq!((g.cols(), g.rows()), (160, 50));
        // Square cells fill two grid rows per terminal row.
        let g = Grid::with_aspect(&base, 16, 16, 100);
        let p = upscale(&g, &base, 160, 50).unwrap();
        let g = Grid::with_aspect(&p, 16, 16, 100);
        assert_eq!((g.cols(), g.rows()), (160, 100));
        // Already one cell per character: left alone.
        let g = Grid::with_aspect(&base, 8, 16, 100);
        assert!(upscale(&g, &base, 160, 50).is_none());
    }
}
