//! Button timing: a tap is two frames held and a gap released, so the game
//! sees each press as a fresh edge; text is answered a set time after it
//! stops printing.

use mizu_core::GameBoy;

use super::screen;

/// Frames a tap holds its button.
const HOLD: u8 = 2;
/// Released frames after a menu tap: enough for any menu to take the press.
pub const GAP: u8 = 4;

#[derive(Default)]
pub struct Keys {
    key: u8,
    hold: u8,
    gap: u8,
    /// The text box's tiles last frame and how many frames they have held.
    text: u32,
    still: u32,
    /// Frames text must sit printed before A, from `POKEMON_TEXT_MS`.
    pub text_frames: u32,
}

impl Keys {
    pub fn new(text_ms: u32) -> Self {
        Self {
            text_frames: (text_ms * 60).div_ceil(1000),
            ..Self::default()
        }
    }

    /// What to hold this frame if a tap is still under way.
    pub fn busy(&mut self) -> Option<u8> {
        if self.hold > 0 {
            self.hold -= 1;
            return Some(self.key);
        }
        if self.gap > 0 {
            self.gap -= 1;
            return Some(0);
        }
        None
    }

    pub fn tap(&mut self, key: u8, gap: u8) -> u8 {
        self.key = key;
        self.hold = HOLD - 1;
        self.gap = gap;
        key
    }

    /// Tracks the text box once a frame; true once it has sat unchanged
    /// for the text delay, and then starts counting again.
    pub fn text_ready(&mut self, gb: &mut GameBoy) -> bool {
        let h = screen::text_hash(gb);
        if h == self.text {
            self.still += 1;
        } else {
            self.text = h;
            self.still = 0;
        }
        if self.still >= self.text_frames {
            self.still = 0;
            return true;
        }
        false
    }
}
