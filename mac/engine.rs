//! `mac-engine`: Mini vMac running a Mac Plus, as a process of its own.
//!
//! Copyright (C) 2026 Adam Hitchcock. GPL-2.0, like the emulator it links
//! (`mac/minivmac`, GPL-2.0 only). It is a separate program so that the
//! screensaver, GPL-3.0 as a whole in the `-gpl` image, never links it; the
//! two only share a pipe (`src/battlechess/proto.rs`). A fault in the
//! emulator ends this process and the screensaver starts another.
//!
//! `mac-engine ROM DISK...`: the first 128 KiB of ROM, the disks in drive
//! order, boot disk first. Screens go to stdout, commands come from stdin; a
//! closed stdin ends the run.

// The screensaver uses the rest of it.
#[allow(dead_code)]
#[path = "../src/battlechess/proto.rs"]
mod proto;

use std::cell::RefCell;
use std::ffi::{c_char, c_int, CString};
use std::io::{BufWriter, Read, Stdin, Stdout, Write};
use std::process::ExitCode;

use proto::{blank, Cmd, CHANGED, CMD, FRAME, SAME};

const ROM: usize = 128 * 1024;

extern "C" {
    fn mvx_init(rom: *const u8, disks: *const *const c_char, n: c_int) -> c_int;
    fn mvx_run() -> c_int;
    fn mvx_speed(speed: c_int);
    fn mvx_mouse(h: c_int, v: c_int, down: c_int);
    fn mvx_key(mkc: c_int, down: c_int);
}

struct Io {
    input: Stdin,
    out: BufWriter<Stdout>,
    last: Box<[u8; FRAME]>,
    sent: bool,
}

thread_local! {
    static IO: RefCell<Io> = RefCell::new(Io {
        input: std::io::stdin(),
        out: BufWriter::with_capacity(FRAME + 1, std::io::stdout()),
        last: blank(),
        sent: false,
    });
}

/// Called by `mac/afterglow_mac.c` once per emulated sixtieth. A panic must
/// not unwind into C, so it stops the Mac instead.
///
/// # Safety
///
/// `screen` points at `FRAME` bytes, live for the call.
#[no_mangle]
pub unsafe extern "C" fn afg_mac_tick(screen: *const u8) -> c_int {
    // SAFETY: as the caller promises.
    let screen = unsafe { std::slice::from_raw_parts(screen, FRAME) };
    std::panic::catch_unwind(|| IO.with(|io| exchange(&mut io.borrow_mut(), screen))).unwrap_or(2)
}

/// Hands the screen over, then applies the next command: 0 runs on, 1 resets,
/// 2 stops (the screensaver has gone).
fn exchange(io: &mut Io, screen: &[u8]) -> c_int {
    let changed = !io.sent || io.last[..] != *screen;
    let sent = if changed {
        io.last.copy_from_slice(screen);
        io.out
            .write_all(&[CHANGED])
            .and_then(|()| io.out.write_all(screen))
    } else {
        io.out.write_all(&[SAME])
    };
    let mut b = [0; CMD];
    if sent.and_then(|()| io.out.flush()).is_err() || io.input.read_exact(&mut b).is_err() {
        return 2;
    }
    io.sent = true;
    let c = Cmd::decode(b);
    // SAFETY: inside afg_mac_tick, the one place the glue takes input.
    unsafe {
        mvx_speed(c.speed.into());
        mvx_mouse(c.h.into(), c.v.into(), c.button.into());
        if let Some((k, down)) = c.key {
            mvx_key(k.into(), down.into());
        }
    }
    c_int::from(c.reset)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((rom, disks)) = args.split_first().filter(|(_, d)| !d.is_empty()) else {
        eprintln!("usage: mac-engine ROM DISK...");
        return ExitCode::from(2);
    };
    let rom = match std::fs::read(rom) {
        Ok(r) if r.len() >= ROM => r,
        Ok(_) => {
            eprintln!("mac-engine: {rom}: shorter than a 128 KiB Mac Plus ROM");
            return ExitCode::FAILURE;
        }
        Err(e) => {
            eprintln!("mac-engine: {rom}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let Ok(disks) = disks
        .iter()
        .map(|d| CString::new(d.as_str()))
        .collect::<Result<Vec<_>, _>>()
    else {
        eprintln!("mac-engine: a disk path holds a NUL");
        return ExitCode::FAILURE;
    };
    let ptrs: Vec<*const c_char> = disks.iter().map(|d| d.as_ptr()).collect();
    // SAFETY: the ROM is at least the 128 KiB the glue copies; the paths
    // outlive the call.
    if unsafe { mvx_init(rom.as_ptr(), ptrs.as_ptr(), ptrs.len() as c_int) } != 0 {
        eprintln!("mac-engine: a disk would not open, or out of memory");
        return ExitCode::FAILURE;
    }
    // SAFETY: initialised above; runs on this thread until a stop.
    if unsafe { mvx_run() } == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
