//! mizu-core 1.3.0 (MIT, Amjad Alsharafi), vendored and trimmed for afterglow.
//! Every change from upstream is marked `afterglow`; see THIRD_PARTY.md.
#![allow(warnings, clippy::all, clippy::pedantic)]

mod apu;
mod cartridge;
pub mod cpu;
mod joypad;
pub mod memory;
mod ppu;
mod save_error;
mod serial;
mod timer;

use std::cell::RefCell;
use std::io::{Read, Write};
use std::rc::Rc;

use save_state::Savable;

use cartridge::Cartridge;
use cpu::Cpu;
use memory::Bus;

pub use apu::AudioBuffers;
pub use cartridge::CartridgeError;
pub use cpu::RunError;
pub use joypad::JoypadButton;
pub use save_error::SaveError;
pub use serial::SerialDevice;

pub const SAVE_STATE_VERSION: usize = 2;
const SAVE_STATE_MAGIC: &[u8; 4] = b"MST\xee";

/// Custom configuration for the [`GameBoy`] emulation inner workings
#[derive(Debug, Default, Clone, Copy, Savable)]
pub struct GameBoyConfig {
    /// Should the gameboy run in DMG mode? default is in CGB mode
    pub is_dmg: bool,
}

impl GameBoyConfig {
    pub fn boot_rom_len(&self) -> usize {
        if self.is_dmg {
            0x100
        } else {
            0x900
        }
    }
}

/// The GameBoy is the main interface to the emulator.
pub struct GameBoy {
    cpu: Cpu,
    bus: Bus,
}

impl GameBoy {
    /// afterglow: replaces the file-based builder. No boot ROM: the CPU and
    /// I/O start in the state one leaves behind. `sram` seeds battery RAM.
    pub fn from_rom(
        rom: Vec<u8>,
        sram: Option<&[u8]>,
        config: GameBoyConfig,
    ) -> Result<Self, CartridgeError> {
        let cartridge = Cartridge::from_bytes(rom, sram)?;
        let is_cartridge_color = cartridge.is_cartridge_color();
        Ok(Self {
            bus: Bus::new_without_boot_rom(cartridge, config),
            cpu: Cpu::new_without_boot_rom(config, is_cartridge_color),
        })
    }

    /// Clocks the Gameboy clock for the duration of one PPU frame.
    ///
    /// afterglow: an illegal opcode is returned rather than unwrapped.
    pub fn clock_for_frame(&mut self) -> Result<(), RunError> {
        const PPU_CYCLES_PER_FRAME: u32 = 456 * 154;
        let mut cycles = 0u32;
        while cycles < PPU_CYCLES_PER_FRAME {
            self.cpu.next_instruction(&mut self.bus)?;
            cycles += self.bus.elapsed_ppu_cycles();
        }
        Ok(())
    }

    /// Return the game title string extracted from the cartridge.
    pub fn game_title(&self) -> &str {
        self.bus.cartridge().game_title()
    }

    /// afterglow: whether the cartridge header asks for CGB features.
    pub fn is_cartridge_color(&self) -> bool {
        self.bus.cartridge().is_cartridge_color()
    }

    /// afterglow: battery RAM, as a power cycle would keep it.
    pub fn sram(&self) -> &[u8] {
        self.bus.cartridge().sram()
    }

    /// afterglow: the ROM, for reading banked data the game points at.
    pub fn rom(&self) -> &[u8] {
        self.bus.cartridge().rom()
    }

    /// afterglow: RGB555 per pixel, 160x144, the last finished frame.
    pub fn screen_buffer(&self) -> &[u16] {
        self.bus.screen_buffer()
    }

    /// afterglow: the background layer of the same frame (bit 15: window).
    pub fn bg_buffer(&self) -> &[u16] {
        self.bus.bg_buffer()
    }

    /// afterglow: SCX, SCY per line of the same frame.
    pub fn line_scroll(&self) -> &[[u8; 2]] {
        self.bus.line_scroll()
    }

    /// afterglow: LCDC's display enable.
    pub fn lcd_on(&self) -> bool {
        self.bus.lcd_on()
    }

    /// afterglow: a side-effect-free read of the address space.
    pub fn peek(&mut self, addr: u16) -> u8 {
        self.bus.peek(addr)
    }

    /// afterglow: both VRAM banks.
    pub fn vram(&self) -> &[u8] {
        self.bus.vram()
    }

    /// Return the audio buffer of the APU at the current state.
    pub fn audio_buffers(&mut self) -> AudioBuffers<'_> {
        self.bus.audio_buffers()
    }

    /// afterglow: hold exactly the buttons in `mask` (bit 7..0: Start,
    /// Select, B, A, Down, Up, Left, Right), as the joypad register lays
    /// them out.
    pub fn set_buttons(&mut self, mask: u8) {
        const ALL: [(u8, fn() -> JoypadButton); 8] = [
            (0x80, || JoypadButton::Start),
            (0x40, || JoypadButton::Select),
            (0x20, || JoypadButton::B),
            (0x10, || JoypadButton::A),
            (0x08, || JoypadButton::Down),
            (0x04, || JoypadButton::Up),
            (0x02, || JoypadButton::Left),
            (0x01, || JoypadButton::Right),
        ];
        for (bit, button) in ALL {
            if mask & bit != 0 {
                self.bus.press_joypad(button());
            } else {
                self.bus.release_joypad(button());
            }
        }
    }

    /// Change the state of the joypad button to `pressed`.
    pub fn press_joypad(&mut self, button: JoypadButton) {
        self.bus.press_joypad(button);
    }

    /// Change the state of the joypad button to `released`.
    pub fn release_joypad(&mut self, button: JoypadButton) {
        self.bus.release_joypad(button);
    }

    pub fn connect_device(&mut self, device: Rc<RefCell<dyn SerialDevice>>) {
        self.bus.connect_device(device);
    }

    pub fn disconnect_device(&mut self) {
        self.bus.disconnect_device();
    }

    /// afterglow: uncompressed (upstream wraps the body in zstd, which is C).
    pub fn save_state<W: Write>(&self, mut writer: W) -> Result<(), SaveError> {
        SAVE_STATE_MAGIC.save(&mut writer)?;
        SAVE_STATE_VERSION.save(&mut writer)?;
        let cartridge_hash: &[u8; 32] = self.bus.cartridge().hash();
        cartridge_hash.save(&mut writer)?;
        self.cpu.save(&mut writer)?;
        self.bus.save(&mut writer)?;
        Ok(())
    }

    /// afterglow: reads what `save_state` writes. A failed load leaves the
    /// machine in an undefined state; the caller resets it.
    pub fn load_state<R: Read>(&mut self, mut reader: R) -> Result<(), SaveError> {
        let mut magic = [0u8; 4];
        let mut version = 0usize;
        let mut hash = [0u8; 32];
        magic.load(&mut reader)?;
        if &magic != SAVE_STATE_MAGIC {
            return Err(SaveError::InvalidSaveStateHeader);
        }
        version.load(&mut reader)?;
        if version != SAVE_STATE_VERSION {
            return Err(SaveError::UnmatchedSaveErrorVersion(version));
        }
        hash.load(&mut reader)?;
        if &hash != self.bus.cartridge().hash() {
            return Err(SaveError::InvalidCartridgeHash);
        }
        self.cpu.load(&mut reader)?;
        self.bus.load(&mut reader)?;
        Ok(())
    }
}
