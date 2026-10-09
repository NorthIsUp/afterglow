//! Which cartridges the saver plays: the bundled homebrew, or ROMs the
//! deployment mounts, and which Pokémon revision a ROM is by checksum.

use std::fmt::Write;
use std::path::Path;

use sha1::{Digest, Sha1};

/// A bundled ROM. Every one is free to redistribute; `THIRD_PARTY.md` has the
/// licences.
pub struct Bundled {
    pub name: &'static str,
    pub rom: &'static [u8],
    pub pilot: Pilot,
}

pub const BUNDLED: &[Bundled] = &[
    Bundled {
        name: "tobu-tobu-girl-deluxe",
        rom: include_bytes!("../../gameboy/tobu-tobu-girl-deluxe.gb"),
        pilot: Pilot::Tobu,
    },
    Bundled {
        name: "rebound",
        rom: include_bytes!("../../gameboy/rebound.gbc"),
        pilot: Pilot::Rebound,
    },
    Bundled {
        name: "life",
        rom: include_bytes!("../../gameboy/life.gb"),
        pilot: Pilot::Mash,
    },
];

/// How a cartridge is played.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pilot {
    /// Start, then A now and then: gets most games past their title screen.
    Mash,
    /// Tobu Tobu Girl Deluxe, steered from its own RAM.
    Tobu,
    /// Rebound: run right and jump.
    Rebound,
    /// Pokémon Red, Blue or Yellow, by the RAM-map bot.
    Pokemon(Revision),
}

/// The Pokémon revisions the bot and the world view know the memory map of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Revision {
    Red,
    Blue,
    Yellow,
}

/// pret's `roms.sha1` for pokered and pokeyellow: the USA/Europe releases.
const POKEMON: &[(&str, Revision)] = &[
    ("ea9bcae617fdf159b045185467ae58b2e4a48b9a", Revision::Red),
    ("d7037c83e1ae5b39bde3c30787637ba1d4c48ce2", Revision::Blue),
    ("cc7d03262ebfaf2f06772c1a480c7d9d5f4a38e1", Revision::Yellow),
];

pub fn sha1_hex(rom: &[u8]) -> String {
    let digest = Sha1::digest(rom);
    digest.iter().fold(String::with_capacity(40), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

pub fn revision(rom: &[u8]) -> Option<Revision> {
    let hex = sha1_hex(rom);
    POKEMON.iter().find(|(h, _)| *h == hex).map(|&(_, r)| r)
}

/// A cartridge ready to boot: its bytes, a name for the log, how to play it,
/// and battery RAM to start from.
pub struct Cart {
    pub name: String,
    pub rom: Vec<u8>,
    pub pilot: Pilot,
    pub sram: Option<Vec<u8>>,
}

/// `GAMEBOY_ROM`: empty for the bundled homebrew, else a ROM file or a
/// directory of them (`.gb`, `.gbc`), sorted by name. A Pokémon ROM is
/// recognised by checksum; any other plays with [`Pilot::Mash`]. `sav` seeds
/// the battery RAM of a single ROM, read once and never written.
pub fn load(rom: &str, sav: &str) -> Vec<Cart> {
    if rom.is_empty() {
        return BUNDLED
            .iter()
            .map(|b| Cart {
                name: b.name.into(),
                rom: b.rom.to_vec(),
                pilot: b.pilot,
                sram: None,
            })
            .collect();
    }
    let path = Path::new(rom);
    let mut files: Vec<_> = if path.is_dir() {
        std::fs::read_dir(path)
            .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect())
            .unwrap_or_default()
    } else {
        vec![path.to_path_buf()]
    };
    files.retain(|p| p.extension().is_some_and(|x| x == "gb" || x == "gbc"));
    files.sort();
    let sram = (!sav.is_empty() && files.len() == 1)
        .then(|| std::fs::read(sav).ok())
        .flatten();
    let carts: Vec<Cart> = files
        .iter()
        .filter_map(|p| {
            let bytes = std::fs::read(p)
                .map_err(|e| eprintln!("[screensaver] gameboy: {}: {e}", p.display()))
                .ok()?;
            let pilot = match revision(&bytes) {
                Some(r) => Pilot::Pokemon(r),
                None => Pilot::Mash,
            };
            let name = p
                .file_stem()
                .map_or(String::new(), |s| s.to_string_lossy().into());
            eprintln!(
                "[screensaver] gameboy: {name}: sha1 {}{}",
                sha1_hex(&bytes),
                match pilot {
                    Pilot::Pokemon(r) => format!(", Pokémon {r:?}: bot and world view"),
                    _ => String::new(),
                }
            );
            Some(Cart {
                name,
                rom: bytes,
                pilot,
                sram: sram.clone(),
            })
        })
        .collect();
    if carts.is_empty() {
        eprintln!("[screensaver] gameboy: no ROM at {rom:?}; playing the bundled homebrew");
        return load("", "");
    }
    carts
}

/// Four DMG shades, lightest first, as RGB555.
pub type Shades = [u16; 4];

const fn rgb(c: u32) -> u16 {
    let (r, g, b) = ((c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF);
    ((r >> 3) | (g >> 3) << 5 | (b >> 3) << 10) as u16
}

/// `GAMEBOY_PALETTE` for a monochrome cartridge. `auto` picks by game: Red
/// and Blue in their cover colours, anything else the original green.
pub fn shades(name: &str, pilot: Pilot) -> Shades {
    let pick = match (name, pilot) {
        ("auto", Pilot::Pokemon(Revision::Red)) => "red",
        ("auto", Pilot::Pokemon(Revision::Blue)) => "blue",
        ("auto", _) => "green",
        (n, _) => n,
    };
    match pick {
        "grey" => [rgb(0xF8F8F8), rgb(0xA8A8A8), rgb(0x585858), rgb(0x101010)],
        "pocket" => [rgb(0xC4CFA1), rgb(0x8B956D), rgb(0x4D533C), rgb(0x1F1F1F)],
        "red" => [rgb(0xFFEFE6), rgb(0xF0907A), rgb(0x9C3A3A), rgb(0x2A0E10)],
        "blue" => [rgb(0xEEF4FF), rgb(0x8AA8E8), rgb(0x34509C), rgb(0x0E1430)],
        _ => [rgb(0xE0F8D0), rgb(0x88C070), rgb(0x346856), rgb(0x081820)],
    }
}

/// The greys the core paints a monochrome game in, lightest first.
pub const DMG_GREYS: Shades = [0x7FFF, 0x56B5, 0x294A, 0x0000];

#[cfg(test)]
mod tests {
    use mizu_core::GameBoyConfig;

    use super::*;

    #[test]
    fn bundled_roms_boot_and_are_not_pokemon() {
        for b in BUNDLED {
            assert_eq!(revision(b.rom), None, "{}", b.name);
            assert!(
                mizu_core::GameBoy::from_rom(b.rom.to_vec(), None, GameBoyConfig::default())
                    .is_ok(),
                "{} does not load",
                b.name
            );
        }
    }

    #[test]
    fn a_missing_rom_falls_back_to_the_bundle() {
        let carts = load("/nonexistent/rom.gb", "");
        assert_eq!(carts.len(), BUNDLED.len());
    }

    #[test]
    fn greys_match_the_core() {
        let g = |v: u16| v | v << 5 | v << 10;
        assert_eq!(DMG_GREYS, [g(31), g(21), g(10), g(0)]);
    }
}
