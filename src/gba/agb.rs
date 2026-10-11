//! mGBA behind `gba/afterglow_gba.c`: one machine, owned, freed on drop.

use std::ffi::{c_int, c_long};
use std::ptr::NonNull;

pub const W: usize = 240;
pub const H: usize = 160;

#[repr(C)]
struct Agb {
    _opaque: [u8; 0],
}

extern "C" {
    fn agb_new(rom: *const u8, rom_len: usize, bios: *const u8, bios_len: usize) -> *mut Agb;
    fn agb_frame(a: *mut Agb, keys: u32) -> c_int;
    fn agb_pixels(a: *const Agb) -> *const u32;
    fn agb_read8(a: *mut Agb, addr: u32) -> u8;
    fn agb_save(a: *mut Agb, out: *mut u8, cap: usize) -> c_long;
    fn agb_load(a: *mut Agb, state: *const u8, len: usize) -> c_int;
    fn agb_free(a: *mut Agb);
}

/// KEYINPUT bits, 1 = held.
pub mod keys {
    pub const A: u32 = 1;
    pub const B: u32 = 1 << 1;
    pub const START: u32 = 1 << 3;
    pub const RIGHT: u32 = 1 << 4;
    pub const LEFT: u32 = 1 << 5;
    pub const UP: u32 = 1 << 6;
    pub const DOWN: u32 = 1 << 7;
}

/// A machine and the ROM it reads in place, dropped after it.
pub struct Core {
    agb: NonNull<Agb>,
    _rom: Box<[u8]>,
}

// The machine is plain heap state with no thread affinity; the engine thread
// is the only one that ever holds it.
unsafe impl Send for Core {}

impl Core {
    /// `None` if mGBA will not load `rom`. An empty `bios` is mGBA's HLE BIOS.
    pub fn new(rom: Box<[u8]>, bios: &[u8]) -> Option<Self> {
        let (bp, bl) = if bios.is_empty() {
            (std::ptr::null(), 0)
        } else {
            (bios.as_ptr(), bios.len())
        };
        // SAFETY: the BIOS is copied; the ROM lives in `Self`, which frees
        // the machine before it.
        let agb = NonNull::new(unsafe { agb_new(rom.as_ptr(), rom.len(), bp, bl) })?;
        Some(Self { agb, _rom: rom })
    }

    /// One frame with `keys` held. False if the core aborted: the machine is
    /// done and must be dropped.
    pub fn frame(&mut self, keys: u32) -> bool {
        // SAFETY: a live machine, used from one thread.
        unsafe { agb_frame(self.agb.as_ptr(), keys) == 0 }
    }

    /// The last frame, XBGR8888 with red in the low byte.
    pub fn pixels(&self) -> &[u32] {
        // SAFETY: the glue's buffer is W * H and lives as long as the machine.
        unsafe { std::slice::from_raw_parts(agb_pixels(self.agb.as_ptr()), W * H) }
    }

    pub fn read8(&mut self, addr: u32) -> u8 {
        // SAFETY: as `frame`.
        unsafe { agb_read8(self.agb.as_ptr(), addr) }
    }

    pub fn read32(&mut self, addr: u32) -> u32 {
        (0..4).fold(0, |v, i| v | u32::from(self.read8(addr + i)) << (8 * i))
    }
}

impl Core {
    /// Append a save state, battery save included, to `out`. False if the
    /// core aborted.
    pub fn save(&mut self, out: &mut Vec<u8>) -> bool {
        loop {
            let at = out.len();
            let cap = out.capacity() - at;
            // SAFETY: as `frame`; the glue writes at most `cap` bytes into
            // the spare capacity.
            let n = unsafe { agb_save(self.agb.as_ptr(), out.as_mut_ptr().add(at), cap) };
            let Ok(n) = usize::try_from(n) else {
                return false;
            };
            if n <= cap {
                // SAFETY: the glue wrote all `n`.
                unsafe { out.set_len(at + n) };
                return true;
            }
            out.reserve(n);
        }
    }

    /// Load what `save` wrote. False if mGBA rejects it: the machine is done
    /// and must be dropped.
    pub fn load(&mut self, state: &[u8]) -> bool {
        // SAFETY: as `frame`; the state is only read.
        unsafe { agb_load(self.agb.as_ptr(), state.as_ptr(), state.len()) == 0 }
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        // SAFETY: owned, freed once.
        unsafe { agb_free(self.agb.as_ptr()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wild pointer in core code fails that call instead of killing the
    /// screensaver: the C boundary catches SIGSEGV as it catches an abort.
    #[test]
    fn a_segfault_in_the_core_fails_the_call_not_the_process() {
        // Any machine: the first one installs the fault handler.
        let _core = Core::new(vec![0; 0xC0].into_boxed_slice(), &[]);
        // SAFETY: deliberately not; the boundary is what is under test.
        assert_eq!(unsafe { agb_read8(std::ptr::null_mut(), 0) }, 0);
        assert_eq!(
            unsafe { agb_read8(std::ptr::null_mut(), 0) },
            0,
            "and again"
        );
    }
}
