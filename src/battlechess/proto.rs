//! The pipe between the screensaver and `mac-engine`, one exchange per
//! emulated sixtieth of a second: the engine writes the screen as it stands,
//! then waits for the command that runs the next sixtieth.
//!
//! Shared by both binaries (`mac/engine.rs` includes this file by path).
//! It is this repo's own MIT code; the engine around it is GPL-2.0.

pub const W: usize = 512;
pub const H: usize = 342;
/// The Mac Plus screen, 1 bit per pixel, MSB first, 1 = black.
pub const FRAME: usize = W * H / 8;
pub const CMD: usize = 8;

/// A screen's worth of zeros, on the heap: a `FRAME` array is too big for
/// the stack to build first.
pub fn blank() -> Box<[u8; FRAME]> {
    vec![0; FRAME]
        .into_boxed_slice()
        .try_into()
        .expect("FRAME bytes")
}

/// Engine to screensaver: this byte, then `FRAME` bytes after a `CHANGED`.
pub const SAME: u8 = 0;
pub const CHANGED: u8 = 1;

/// Screensaver to engine: the input the Mac sees for the next sixtieth.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Cmd {
    /// Press the reset button; the disks go back in, as the files are.
    pub reset: bool,
    /// Mini vMac's speed: 0 is 1x, each step doubles.
    pub speed: u8,
    pub h: u16,
    pub v: u16,
    pub button: bool,
    /// A Mini vMac key code (`MKC_*` in `mac/minivmac/MYOSGLUE.h`) and
    /// whether it goes down or up.
    pub key: Option<(u8, bool)>,
}

const NO_KEY: u8 = 0xFF;

impl Cmd {
    pub fn encode(&self) -> [u8; CMD] {
        let [h0, h1] = self.h.to_le_bytes();
        let [v0, v1] = self.v.to_le_bytes();
        let (key, down) = self.key.unwrap_or((NO_KEY, false));
        let flags = u8::from(self.reset) | u8::from(self.button) << 1 | u8::from(down) << 2;
        [flags, self.speed, h0, h1, v0, v1, key, 0]
    }

    pub fn decode(b: [u8; CMD]) -> Self {
        Self {
            reset: b[0] & 1 != 0,
            speed: b[1],
            h: u16::from_le_bytes([b[2], b[3]]),
            v: u16::from_le_bytes([b[4], b[5]]),
            button: b[0] & 2 != 0,
            key: (b[6] != NO_KEY).then_some((b[6], b[0] & 4 != 0)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_survive_the_pipe() {
        for c in [
            Cmd::default(),
            Cmd {
                reset: true,
                speed: 3,
                h: 511,
                v: 341,
                button: true,
                key: Some((0x37, true)),
            },
            Cmd {
                key: Some((0, false)),
                ..Cmd::default()
            },
        ] {
            assert_eq!(Cmd::decode(c.encode()), c);
        }
    }
}
