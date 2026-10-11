//! Which cartridges the saver plays — the `.gba` files the deployment
//! mounts, none bundled — and which Pokémon a ROM is, by checksum.

use std::fmt::Write;
use std::path::{Path, PathBuf};

use sha1::{Digest, Sha1};

/// The third-generation Pokémon games the pilot knows the RAM of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pokemon {
    FireRed,
    LeafGreen,
    Emerald,
}

/// pret's `roms.sha1` for pokefirered and pokeemerald: the USA/Europe releases.
const POKEMON: &[(&str, Pokemon)] = &[
    ("41cb23d8dccc8ebd7c649cd8fbb58eeace6e2fdc", Pokemon::FireRed),
    ("dd5945db9b930750cb39d00c84da8571feebf417", Pokemon::FireRed),
    (
        "574fa542ffebb14be69902d1d36f1ec0a4afd71e",
        Pokemon::LeafGreen,
    ),
    (
        "7862c67bdecbe21d1d69ce082ce34327e1c6ed5e",
        Pokemon::LeafGreen,
    ),
    ("f3ae088181bf583e55daf962a92bb46f4f1d07b7", Pokemon::Emerald),
];

pub fn sha1_hex(rom: &[u8]) -> String {
    Sha1::digest(rom).iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

pub fn pokemon(sha1: &str) -> Option<Pokemon> {
    POKEMON.iter().find(|(h, _)| *h == sha1).map(|&(_, p)| p)
}

pub struct Cart {
    pub name: String,
    pub rom: Vec<u8>,
    pub sha1: String,
    pub pokemon: Option<Pokemon>,
}

/// `GBA_ROM`'s `.gba` files: the file itself, or a folder's, sorted by name.
pub fn files(rom: &str) -> Vec<PathBuf> {
    if rom.is_empty() {
        return Vec::new();
    }
    let path = Path::new(rom);
    let mut files: Vec<_> = if path.is_dir() {
        std::fs::read_dir(path)
            .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect())
            .unwrap_or_default()
    } else {
        vec![path.to_path_buf()]
    };
    files.retain(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("gba")));
    files.sort();
    files
}

/// Read one ROM; `None` (logged) if it cannot be.
pub fn read(path: &Path) -> Option<Cart> {
    let rom = std::fs::read(path)
        .map_err(|e| eprintln!("[screensaver] gba: {}: {e}", path.display()))
        .ok()?;
    // Once: a 32 MiB cartridge is a noticeable hash on a Pi.
    let sha1 = sha1_hex(&rom);
    let pokemon = pokemon(&sha1);
    let name = path
        .file_stem()
        .map_or(String::new(), |s| s.to_string_lossy().into());
    eprintln!(
        "[screensaver] gba: {name}: sha1 {sha1}{}",
        pokemon.map_or(String::new(), |p| format!(", Pokémon {p:?}"))
    );
    Some(Cart {
        name,
        rom,
        sha1,
        pokemon,
    })
}
