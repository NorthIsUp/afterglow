//! The terminal host: the same frames, printed into the terminal this runs in.
//!
//! A third host beside DRM and the dump. It drives the same `saver::switch` and
//! `saver::frame` into a pixel buffer nobody reads, then prints the grid that
//! frame flushed: one character per cell where cells are glyph-shaped, two
//! cells per character (`▀`, top cell as the foreground, bottom as the
//! background) where they are square. Truecolor only; every terminal worth
//! running a screensaver in has it.

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::font;
use crate::grid::{Cell, Grid};
use crate::host::pace;
use crate::mirror::Mirror;
use crate::saver::{self, Saver};
use crate::surface::Panel;
use crate::{Config, SIGNALLED};

/// What one terminal character cell stands in for, in framebuffer pixels. The
/// font's own glyph size, so a saver sized for it gets one cell per character.
const CHAR_W: usize = 8;
const CHAR_H: usize = 16;

/// Upscaled panels stop here. A 24px-cell saver in a wide terminal would
/// otherwise ask for a buffer of hundreds of megabytes; past this its grid is
/// simply smaller than the terminal and centred.
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
fn glyph_cells(g: &Grid) -> bool {
    2 * g.cell_h() >= 3 * g.cell_w()
}

/// The panel that gives `g`'s saver about one grid cell per character (or per
/// half character), when the base panel gave it fewer. A saver with 16x32
/// cells built for an 8x16-per-character panel fills a quarter of the
/// terminal; built for twice the panel it fills all of it. None when the grid
/// already fits — growing a grid that is sized off the panel, as the
/// ascii.rest pieces are, gains nothing.
fn upscale(g: &Grid, base: &Panel, cols: usize, rows: usize) -> Option<Panel> {
    let want_rows = if glyph_cells(g) { rows } else { rows * 2 };
    let permille = (cols * 1000 / g.cols()).min(want_rows * 1000 / g.rows());
    let w = (base.w * permille / 1000).min(MAX_PANEL);
    let h = (base.h * permille / 1000).min(MAX_PANEL);
    (permille > 1000 && (w, h) != (base.w, base.h)).then(|| Panel::new(w, h, w))
}

/// Build the selected saver for a `cols` x `rows` terminal.
fn build(mirror: &Mirror, cols: usize, rows: usize, fps: u32) -> (Panel, Box<dyn Saver>) {
    let base = Panel::new(cols * CHAR_W, rows * CHAR_H, cols * CHAR_W);
    let name = saver::name_at(mirror.selected());
    let mut s = saver::make(name, &base, fps);
    let panel = match upscale(s.grid(), &base, cols, rows) {
        Some(p) => {
            s = saver::make(name, &p, fps);
            p
        }
        None => base,
    };
    saver::announce(mirror, s.as_ref(), &panel);
    (panel, s)
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
    out: Vec<u8>,
    /// Nothing on the terminal is known, so every cell is written.
    fresh: bool,
}

/// Map terminal position `t` onto a picture `len` wide centred in `n`; None
/// off the picture. A picture wider than the terminal loses both edges.
#[inline]
fn centred(t: usize, n: usize, len: usize) -> Option<usize> {
    let p = t as isize - (n as isize - len as isize) / 2;
    (p >= 0 && (p as usize) < len).then_some(p as usize)
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
            // Room for a full repaint with a cursor move and both colours on
            // every cell, so a busy frame does not grow it.
            out: Vec::with_capacity(cols * rows * 48 + 64),
            fresh: true,
        }
    }

    /// Lay a flushed grid's cells onto the terminal.
    pub fn compose(&mut self, g: &Grid, pal: &[u32]) {
        self.compose_cells(
            g.cells(),
            g.cols(),
            g.rows(),
            glyph_cells(g),
            g.ground(),
            pal,
        );
    }

    fn compose_cells(
        &mut self,
        cells: &[Cell],
        gcols: usize,
        grows: usize,
        glyphs: bool,
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
        let vrows = if glyphs { grows } else { grows.div_ceil(2) };
        for y in 0..self.rows {
            let gy = centred(y, self.rows, vrows);
            for x in 0..self.cols {
                let tc = match (gy, centred(x, self.cols, gcols)) {
                    (Some(gy), Some(gx)) if glyphs => {
                        let c = cells[gy * gcols + gx];
                        Tc {
                            ch: font::CHARS[c.glyph()],
                            fg: pal[c.colour()],
                            bg: ground,
                        }
                    }
                    (Some(gy), Some(gx)) => {
                        let (top, bot) = (2 * gy, 2 * gy + 1);
                        Tc {
                            ch: '▀',
                            fg: lit(cells[top * gcols + gx]),
                            bg: if bot < grows {
                                lit(cells[bot * gcols + gx])
                            } else {
                                ground
                            },
                        }
                    }
                    _ => Tc {
                        ch: ' ',
                        fg: ground,
                        bg: ground,
                    },
                };
                self.cur[y * self.cols + x] = tc;
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

static ACTIVE: AtomicBool = AtomicBool::new(false);
static TERMIOS: OnceLock<libc::termios> = OnceLock::new();
/// The real stderr while fd 2 is muted, or -1.
static STDERR: AtomicI32 = AtomicI32::new(-1);

/// Take the terminal: alternate screen, no cursor, no echo, keys readable
/// without Enter. Returns whether stdin is a tty whose keys can be read.
fn enter() -> bool {
    let keys = unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        libc::tcgetattr(libc::STDIN_FILENO, &mut t) == 0 && {
            let _ = TERMIOS.set(t);
            // ISIG stays on: Ctrl-C is still SIGINT, which main already turns
            // into a clean stop.
            t.c_lflag &= !(libc::ECHO | libc::ICANON);
            t.c_cc[libc::VMIN] = 0;
            t.c_cc[libc::VTIME] = 0;
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &t) == 0
        }
    };
    // Anything written to stderr while the alternate screen is up lands on top
    // of the picture, and the diff never repaints cells it thinks are
    // unchanged — the mirror thread's "listening" line would sit there for
    // good. Muted rather than lost: a panic restores fd 2 before it prints.
    unsafe {
        if libc::isatty(libc::STDERR_FILENO) == 1 {
            let saved = libc::dup(libc::STDERR_FILENO);
            let null = libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY);
            if saved >= 0 && null >= 0 {
                libc::dup2(null, libc::STDERR_FILENO);
                STDERR.store(saved, Ordering::SeqCst);
            }
            if null >= 0 {
                libc::close(null);
            }
        }
    }
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(ENTER).and_then(|()| out.flush());
    ACTIVE.store(true, Ordering::SeqCst);
    keys
}

/// Give the terminal back. Idempotent, and called from the panic hook as well
/// as on the way out: `panic = "abort"` skips every destructor.
fn restore() {
    if !ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(LEAVE).and_then(|()| out.flush());
    unsafe {
        if let Some(t) = TERMIOS.get() {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, t);
        }
        let fd = STDERR.swap(-1, Ordering::SeqCst);
        if fd >= 0 {
            libc::dup2(fd, libc::STDERR_FILENO);
            libc::close(fd);
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
    let (mut panel, mut saver) = build(mirror, cols, rows, cfg.fps);
    let mut buf = vec![0u32; panel.buf_len()];
    let mut screen = Screen::new(cols, rows);
    let mut selected = mirror.selected();
    let mut rot = saver::Rotate::new(Instant::now());
    let frame_dur = Duration::from_nanos(1_000_000_000 / u64::from(cfg.fps));

    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        hook(info);
    }));
    let keys = enter();
    let _restore = Restore;

    while !SIGNALLED.load(Ordering::Relaxed) && !(keys && quit_key()) {
        let t0 = Instant::now();
        // Polled rather than SIGWINCH: one ioctl a frame, and no second signal
        // handler racing the first.
        if let Some((c, r)) = size().filter(|&s| s != (cols, rows)) {
            (cols, rows) = (c, r);
            (panel, saver) = build(mirror, cols, rows, cfg.fps);
            buf = vec![0u32; panel.buf_len()];
            screen = Screen::new(cols, rows);
        }
        let base = Panel::new(cols * CHAR_W, rows * CHAR_H, cols * CHAR_W);
        if saver::switch(
            &mut saver,
            &mut selected,
            &mut rot,
            t0,
            mirror,
            &base,
            cfg.fps,
        ) {
            // Switched at the base panel; this sizes it for the terminal.
            (panel, saver) = build(mirror, cols, rows, cfg.fps);
            buf = vec![0u32; panel.buf_len()];
        }
        saver::frame(saver.as_mut(), &mut buf, &panel);
        if mirror.watched() {
            mirror.publish(saver.mirror_cells());
        }
        screen.compose(saver.shown(), saver.palette());
        let mut out = std::io::stdout().lock();
        out.write_all(screen.emit())
            .and_then(|()| out.flush())
            .map_err(|e| format!("write terminal: {e}"))?;
        drop(out);
        pace(mirror, frame_dur, t0);
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

    #[test]
    fn glyph_cells_print_one_char_each_centred() {
        let mut s = Screen::new(4, 3);
        let cells = [Cell::new(ascii(b'A'), 1), Cell::new(ascii(b'B'), 2)];
        s.compose_cells(&cells, 2, 1, true, 0x111111, &PAL);
        let row: Vec<char> = s.cur[4..8].iter().map(|t| t.ch).collect();
        assert_eq!(row, [' ', 'A', 'B', ' ']);
        assert_eq!(s.cur[5].fg, 0xFF0000);
        assert_eq!(s.cur[6].fg, 0x00FF00);
        assert!(s.cur[5..7].iter().all(|t| t.bg == 0x111111));
        // Outside the grid is ground, not black.
        assert_eq!(s.cur[0].bg, 0x111111);
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
        s.compose_cells(&cells, 2, 3, false, 0x111111, &PAL);
        let t = |i: usize| (s.cur[i].ch, s.cur[i].fg, s.cur[i].bg);
        assert_eq!(t(0), ('▀', 0xFF0000, 0x111111));
        // A blank glyph is ground whatever its colour; any lit one is its colour.
        assert_eq!(t(1), ('▀', 0x111111, 0x00FF00));
        assert_eq!(t(2), ('▀', 0x00FF00, 0x111111));
        assert_eq!(t(3), ('▀', 0xFF0000, 0x111111));
    }

    #[test]
    fn a_wider_grid_is_cropped_to_its_centre() {
        let mut s = Screen::new(2, 1);
        let cells: Vec<Cell> = b"WXYZ".iter().map(|&c| Cell::new(ascii(c), 1)).collect();
        s.compose_cells(&cells, 4, 1, true, 0, &PAL);
        assert_eq!((s.cur[0].ch, s.cur[1].ch), ('X', 'Y'));
    }

    #[test]
    fn an_unchanged_frame_writes_no_cells() {
        let mut s = Screen::new(3, 2);
        let cells = [Cell::new(ascii(b'A'), 1); 6];
        s.compose_cells(&cells, 3, 2, true, 0, &PAL);
        let first = s.emit().len();
        assert!(first > 0);
        s.compose_cells(&cells, 3, 2, true, 0, &PAL);
        assert!(s.emit().is_empty());

        let mut moved = cells;
        moved[4] = Cell::new(ascii(b'B'), 1);
        s.compose_cells(&moved, 3, 2, true, 0, &PAL);
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
        s.compose_cells(&cells, 2, 2, true, 0, &PAL);
        let out = String::from_utf8(s.emit().to_vec()).unwrap();
        assert_eq!(out.matches('H').count(), 2, "{out:?}");
    }

    #[test]
    fn emit_never_allocates() {
        let mut s = Screen::new(40, 10);
        let mut cells = vec![Cell::new(ascii(b'A'), 1); 400];
        s.compose_cells(&cells, 40, 10, true, 0, &PAL);
        s.emit();
        for (i, c) in cells.iter_mut().enumerate() {
            *c = Cell::new(ascii(b'B'), (i % 3) as u16);
        }
        let n = allocs_during(|| {
            s.compose_cells(&cells, 40, 10, true, 0x123456, &PAL);
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
