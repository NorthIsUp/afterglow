//! Pokémon Red, Blue and Yellow's WRAM, as pret's pokered and pokeyellow
//! lay it out, and EvoYellow's, from the `.sym` of its own disassembly:
//! what the bot and the world view read.

use mizu_core::GameBoy;

use super::carts::Revision;

/// The WRAM addresses the bot and the world view read: Red's, moved by
/// [`moves`] in the other revisions.
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
    rev: Revision,
}

impl Ram {
    pub fn of(r: Revision) -> Self {
        let at = |red: u16| shift(r, red);
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
            rev: r,
        }
    }

    /// A Red/Blue WRAM address in this revision.
    pub const fn at(self, red: u16) -> u16 {
        shift(self.rev, red)
    }
}

/// A big-endian word, the way the game stores stats and HP.
pub fn word(gb: &mut GameBoy, at: u16) -> u16 {
    u16::from(gb.peek(at)) << 8 | u16::from(gb.peek(at + 1))
}

/// Where a revision's WRAM moved from Red's: from each Red address on, in
/// order, how far. Yellow sits a byte lower from `wOnSGB` up. EvoYellow
/// then grows at Red's `wPokedexSeen` (a 252-Pokémon Pokédex),
/// `wNumBagItems` (a 30-item bag) and `wPlayerMoney`, and gives 20 unused
/// game-progress flag bytes back from `wGameProgressFlagsEnd`.
const fn moves(r: Revision) -> &'static [(u16, i16)] {
    match r {
        Revision::Red | Revision::Blue => &[],
        Revision::Yellow => &[(0xCF1B, -1)],
        Revision::EvoYellow => &[
            (0xCF1B, -1),
            (0xD30A, 12),
            (0xD31D, 25),
            (0xD347, 45),
            (0xD6B8, 25),
        ],
    }
}

const fn shift(r: Revision, red: u16) -> u16 {
    let (m, mut i, mut by) = (moves(r), 0, 0);
    while i < m.len() && red >= m[i].0 {
        by = m[i].1;
        i += 1;
    }
    red.wrapping_add_signed(by)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Against `evoyellow.sym`, built from EvoYellow's disassembly at the
    /// commit its 3.9 patch was made from.
    #[test]
    fn evoyellow_addresses_match_its_symbols() {
        let r = Ram::of(Revision::EvoYellow);
        let want = [
            (r.at(0xCF7B), 0xCF7A), // wItemList
            (r.font_loaded, 0xCFC3),
            (r.in_battle, 0xD056),
            (r.party_count, 0xD162),
            (r.party_mons, 0xD16A),
            (r.player_name, 0xD157),
            (r.at(0xD31D), 0xD336), // wNumBagItems
            (r.at(0xD31E), 0xD337), // wBagItems
            (r.at(0xD347), 0xD374), // wPlayerMoney
            (r.rival_name, 0xD377),
            (r.options, 0xD382),
            (r.at(0xD356), 0xD383), // wObtainedBadges
            (r.cur_map, 0xD38B),
            (r.y, 0xD38E),
            (r.x, 0xD38F),
            (r.tileset, 0xD394),
            (r.at(0xD3AE), 0xD3DB), // wNumberOfWarps
            (r.moving_direction, 0xD555),
            (r.at(0xD5A6), 0xD5D3), // wMissableObjectFlags
            (r.at(0xD700), 0xD719), // wWalkBikeSurfState
            (r.at(0xD728), 0xD741), // wd728
            (r.at(0xD743), 0xD75C), // wFirstLockTrashCanIndex
            (r.at(0xD747), 0xD760), // wEventFlags
        ];
        for (i, (got, want)) in want.into_iter().enumerate() {
            assert_eq!(got, want, "row {i}: {got:04X} != {want:04X}");
        }
    }

    /// pokeyellow's symbols either side of `wOnSGB`, and Red's own.
    #[test]
    fn yellow_sits_a_byte_lower_from_wonsgb() {
        let (red, y) = (Ram::of(Revision::Red), Ram::of(Revision::Yellow));
        assert_eq!(y.at(0xCD6B), 0xCD6B); // wJoyIgnore
        assert_eq!((y.party_count, y.cur_map), (0xD162, 0xD35D));
        assert_eq!((red.party_count, red.cur_map), (0xD163, 0xD35E));
    }
}
