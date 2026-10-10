//! `battlechess`: Battle Chess (Interplay, 1988) on an emulated Mac Plus,
//! the Mac playing both sides, capture fights and all, one game after
//! another.
//!
//! Only in a `--features mac` build. The Mac is Mini vMac (GPL-2.0 only) in
//! `mac-engine`, a process of its own beside the screensaver: see
//! `engine.rs`, `mac/engine.rs` and `THIRD_PARTY.md`. The ROM, the System
//! disk and the game are the deployment's own files, read in place; without
//! them the saver shows a card saying what to mount.
//!
//! The 512x342 screen fills the panel's height at the Mac's square pixels,
//! a bezel and the Finder's grey desktop pattern beside it, scaled onto the
//! panel as `doom` is (`scaled.rs`). The grid is for the mirror and the
//! terminal: a solid cell per pixel, coloured by palette index.

mod engine;
mod pilot;
// The engine's half of the pipe (`decode`, `SAME`) goes unused here.
#[allow(dead_code)]
mod proto;

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::engine_slot::Claim;
use crate::env_str;
use crate::font;
use crate::glyph;
use crate::grid::{pixel_aspect, Grid};
use crate::saver::Saver;
use crate::scaled::{self, Scaled};
use crate::surface::{Panel, Surface};

use engine::{Engine, State, Want};
use proto::{FRAME, H, W};

/// The widest view: a 4:1 panel.
const MAX_W: usize = 4 * H;
/// Palette indices: the Mac's two, then the art beside it.
const LIT: u8 = 0;
const INK: u8 = 1;
const BEZEL: u8 = 2;
const DESK_DARK: u8 = 3;
const DESK_LIGHT: u8 = 4;
const BEZEL_W: usize = 6;
/// Checksums in the first four bytes of the three Mac Plus ROM revisions.
const PLUS_ROMS: [u32; 3] = [0x4D1E_EEE1, 0x4D1E_EAE1, 0x4D1F_8172];
const ROM_LEN: u64 = 128 * 1024;

pub struct BattleChess {
    grid: Grid,
    palette: Vec<u32>,
    engine: &'static Engine,
    claim: Claim<Engine>,
    /// Showing why there is no Mac, rather than the Mac.
    carded: bool,
    view: Scaled<u8>,
    /// Where the Mac's screen starts in a `pix` row.
    x0: usize,
    pix: Vec<u8>,
    bits: Box<[u8; FRAME]>,
    seq: u32,
    /// `pix` holds what the panel has not shown yet.
    fresh: bool,
}

/// The view's width for a panel, in Mac pixels at the screen's full height,
/// and the share of the panel it fills. Glass narrower than the Mac's 3:2
/// shows it letterboxed.
fn layout(panel: &Panel, aspect: usize) -> (usize, usize, usize) {
    scaled::layout(panel, aspect, H as f32, W, MAX_W)
}

/// Lit and ink for a tint name; anything unknown is `paper`.
fn tint(name: &str) -> (u32, u32) {
    match name {
        "white" => (0xFFFFFF, 0x000000),
        "amber" => (0xFFB52E, 0x1C1000),
        "green" => (0x4AF27A, 0x031A0A),
        "blue" => (0xD8E4FF, 0x0A0E1C),
        _ => (0xEEEBE0, 0x161514),
    }
}

/// The files, and anything wrong with them, one card line each.
pub struct Files {
    pub rom: String,
    pub system: String,
    pub disk: String,
    pub engine: PathBuf,
}

impl Files {
    fn problems(&self) -> Vec<String> {
        let mut out = vec![];
        for (key, path, what) in [
            ("MAC_ROM", &self.rom, "Mac Plus ROM"),
            ("MAC_SYSTEM", &self.system, "System disk"),
            ("MAC_DISK", &self.disk, "Battle Chess disk"),
        ] {
            if !Path::new(path).is_file() {
                out.push(format!("{key}: no {what} at"));
                out.push(path_line(path));
            }
        }
        if out.is_empty() {
            if !plus_rom(Path::new(&self.rom)) {
                out.push("MAC_ROM is not a Mac Plus ROM:".into());
                out.push(path_line(&self.rom));
            }
            if volume_name(Path::new(&self.disk)).is_none() {
                out.push("MAC_DISK is not an HFS disk image:".into());
                out.push(path_line(&self.disk));
            }
            if !self.engine.is_file() {
                out.push("mac-engine is missing beside the screensaver:".into());
                out.push(path_line(&self.engine.to_string_lossy()));
            }
        }
        out
    }
}

/// A path on a card line of its own, indented, its start cut to fit.
fn path_line(s: &str) -> String {
    const FIT: usize = 58;
    let count = s.chars().count();
    if count <= FIT {
        format!("  {s}")
    } else {
        format!(
            "  ...{}",
            s.chars().skip(count - FIT + 3).collect::<String>()
        )
    }
}

fn plus_rom(path: &Path) -> bool {
    let mut head = [0; 4];
    std::fs::File::open(path)
        .and_then(|mut f| {
            let len = f.metadata()?.len();
            f.read_exact(&mut head)?;
            Ok(len >= ROM_LEN)
        })
        .is_ok_and(|long| long && PLUS_ROMS.contains(&u32::from_be_bytes(head)))
}

/// An HFS volume's name, from its master directory block.
fn volume_name(path: &Path) -> Option<String> {
    let mut mdb = [0; 64];
    let mut f = std::fs::File::open(path).ok()?;
    f.seek(SeekFrom::Start(1024)).ok()?;
    f.read_exact(&mut mdb).ok()?;
    let len = usize::from(mdb[36]).min(27);
    (&mdb[..2] == b"BD").then(|| mdb[37..37 + len].iter().map(|&b| char::from(b)).collect())
}

/// The Mac's screen as an alert on the Finder's grey desktop, `lines` in
/// it: into `pix`, rows `stride` apart, the screen starting at column `x0`.
fn card<S: AsRef<str>>(lines: &[S], pix: &mut [u8], stride: usize, x0: usize) {
    let mut put = |x: usize, y: usize, v: u8| pix[y * stride + x0 + x] = v;
    for y in 0..H {
        for x in 0..W {
            put(x, y, if (x + y) % 2 == 0 { INK } else { LIT });
        }
    }
    let rows = lines.len() * font::GLYPH_H;
    let (bx, by, bw, bh) = (24, (H - rows) / 2 - 16, W - 48, rows + 32);
    for y in by - 1..=by + bh {
        for x in bx - 1..=bx + bw {
            let edge = y == by - 1 || y == by + bh || x == bx - 1 || x == bx + bw;
            put(x, y, if edge { INK } else { LIT });
        }
    }
    for (row, line) in lines.iter().enumerate() {
        let top = by + 16 + row * font::GLYPH_H;
        for (col, c) in line
            .as_ref()
            .chars()
            .take((bw - 32) / font::GLYPH_W)
            .enumerate()
        {
            let c = if c == ' ' || c.is_ascii_graphic() {
                c
            } else {
                '?'
            };
            let glyph = font::GLYPHS[glyph::of(c) as usize];
            for (gy, bits) in glyph.iter().enumerate() {
                for gx in 0..font::GLYPH_W {
                    if bits << gx & 0x80 != 0 {
                        put(bx + 16 + col * font::GLYPH_W + gx, top + gy, INK);
                    }
                }
            }
        }
    }
}

const FAILED: [&str; 4] = [
    "The Mac would not start.",
    "",
    "Its files are there, but the emulator gave up",
    "on them three times running: see the pod's log.",
];

impl BattleChess {
    pub fn new(panel: &Panel, _fps: u32) -> Self {
        let files = Files {
            rom: env_str(&["MAC_ROM"], "/roms/mac/Mac-Plus.ROM"),
            system: env_str(&["MAC_SYSTEM"], "/roms/mac/System7_5_3.img"),
            disk: env_str(&["MAC_DISK"], "/roms/mac/BattleChess.img"),
            engine: std::env::current_exe()
                .map(|p| p.with_file_name("mac-engine"))
                .unwrap_or_default(),
        };
        let tint = env_str(&["BATTLECHESS_TINT"], "paper");
        Self::build(panel, pixel_aspect(), &files, &tint, Engine::get())
    }

    fn build(
        panel: &Panel,
        aspect: usize,
        files: &Files,
        tint_name: &str,
        engine: &'static Engine,
    ) -> Self {
        let (grid, view) = Scaled::new(panel, aspect, layout(panel, aspect), H, MAX_W);
        let (lit, ink) = tint(tint_name);
        let problems = files.problems();
        let want = problems.is_empty().then(|| Want {
            engine: files.engine.clone(),
            rom: files.rom.clone().into(),
            disks: vec![files.system.clone().into(), files.disk.clone().into()],
            volume: volume_name(Path::new(&files.disk)).unwrap_or_default(),
        });
        let w = view.width();
        let mut s = Self {
            grid,
            palette: vec![lit, ink, 0x2B2A27, 0x141414, 0x1E1E1E],
            engine,
            claim: Claim::new(want),
            carded: false,
            view,
            x0: (w - W) / 2,
            pix: vec![INK; MAX_W * H],
            bits: proto::blank(),
            seq: 0,
            fresh: true,
        };
        s.sides();
        if !problems.is_empty() {
            let mut lines = vec![
                "Battle Chess needs the deployment's files.".into(),
                String::new(),
            ];
            lines.extend(problems);
            lines.extend([String::new(), "See docs/savers/battlechess.md".into()]);
            s.show_card(&lines);
        }
        s
    }

    /// The desktop pattern beside the screen, and a bezel against it.
    fn sides(&mut self) {
        let (w, x0) = (self.view.width(), self.x0);
        for y in 0..H {
            for x in (0..x0).chain(x0 + W..w) {
                let bezel = x + BEZEL_W >= x0 && x < x0 + W + BEZEL_W;
                self.pix[y * w + x] = if bezel {
                    BEZEL
                } else if (x + y) % 2 == 0 {
                    DESK_DARK
                } else {
                    DESK_LIGHT
                };
            }
        }
    }

    fn show_card<S: AsRef<str>>(&mut self, lines: &[S]) {
        card(lines, &mut self.pix, self.view.width(), self.x0);
        self.carded = true;
        self.fresh = true;
    }

    /// The engine's 1-bit screen into the Mac's part of `pix`.
    fn unpack(&mut self) {
        let w = self.view.width();
        for (y, row) in self.bits.as_chunks::<{ W / 8 }>().0.iter().enumerate() {
            let out = &mut self.pix[y * w + self.x0..][..W];
            for (px, byte) in out.as_chunks_mut::<8>().0.iter_mut().zip(row) {
                for (i, p) in px.iter_mut().enumerate() {
                    *p = byte >> (7 - i) & 1;
                }
            }
        }
        self.fresh = true;
    }
}

impl Saver for BattleChess {
    fn render(&mut self, s: &mut Surface<'_>) {
        let engine = self.engine;
        if let Some(e) = self.claim.engine(|| engine) {
            if e.state() == State::Failed {
                if !self.carded {
                    self.show_card(&FAILED);
                }
            } else if let Some(seq) = e.latest(self.seq, &mut self.bits) {
                self.seq = seq;
                self.carded = false;
                self.unpack();
            }
        }
        if !self.fresh {
            return;
        }
        self.fresh = false;
        let palette = &self.palette;
        self.view.draw(
            s,
            &mut self.grid,
            &self.pix,
            false,
            |p| palette[p as usize],
            u16::from,
        );
    }

    fn name(&self) -> &'static str {
        "battlechess"
    }

    fn grid(&self) -> &Grid {
        &self.grid
    }

    fn palette(&self) -> &[u32] {
        &self.palette
    }
}

#[cfg(test)]
mod tests;
