//! Pokémon Red, Blue and Yellow's WRAM, as pret's pokered and pokeyellow
//! lay it out: what the bot and the world view read.

use mizu_core::GameBoy;

use super::carts::Revision;

/// The WRAM addresses the bot and the world view read. Red and Blue share
/// one layout; Yellow's sits a byte lower from `wFontLoaded` up.
#[derive(Clone, Copy, Debug)]
pub struct Ram {
    pub font_loaded: u16,
    pub walk_counter: u16,
    pub in_battle: u16,
    pub cur_map: u16,
    pub y: u16,
    pub x: u16,
    pub tileset: u16,
    pub party_count: u16,
    /// `wPartyMons`, `party_struct` in pokered.
    pub party_mons: u16,
    pub sprite_data1: u16,
    pub sprite_data2: u16,
    pub player_name: u16,
    pub rival_name: u16,
    pub moving_direction: u16,
    pub options: u16,
    yellow: bool,
}

impl Ram {
    pub fn of(r: Revision) -> Self {
        let yellow = matches!(r, Revision::Yellow);
        let at = |red: u16| shift(yellow, red);
        Self {
            font_loaded: at(0xCFC4),
            walk_counter: at(0xCFC5),
            in_battle: at(0xD057),
            cur_map: at(0xD35E),
            y: at(0xD361),
            x: at(0xD362),
            tileset: at(0xD367),
            party_count: at(0xD163),
            party_mons: at(0xD16B),
            sprite_data1: 0xC100,
            sprite_data2: 0xC200,
            player_name: at(0xD158),
            rival_name: at(0xD34A),
            moving_direction: at(0xD528),
            options: at(0xD355),
            yellow,
        }
    }

    /// A Red/Blue WRAM address in this revision.
    pub const fn at(self, red: u16) -> u16 {
        shift(self.yellow, red)
    }
}

/// A big-endian word, the way the game stores stats and HP.
pub fn word(gb: &mut GameBoy, at: u16) -> u16 {
    u16::from(gb.peek(at)) << 8 | u16::from(gb.peek(at + 1))
}

const fn shift(yellow: bool, red: u16) -> u16 {
    if yellow && red >= 0xCFC4 {
        red - 1
    } else {
        red
    }
}
