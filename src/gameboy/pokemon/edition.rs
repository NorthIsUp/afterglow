//! What each revision changes about the bot's game, as data: the story and
//! the walking read a field, never ask which game this is.

use super::super::carts::Revision;
use super::story::{ERIKA, SABRINA};

pub struct Edition {
    /// The sprite slot Yellow's follower walks behind the player in.
    pub follower: Option<u16>,
    /// An item ball's sprite picture.
    pub ball_sprite: u8,
    /// The Poké Ball in Oak's lab to take, when it is the same whatever the
    /// starter: Yellow's Pikachu.
    pub starter_ball: Option<u8>,
    /// The lead's level before each gym when the starter is fixed, else the
    /// starter's own.
    pub levels: Option<[u8; 6]>,
    /// The gym, by index in badge order, the story stops before.
    pub stop_before: Option<usize>,
    /// Rocket Hideout B4F's door: Yellow's Jessie and James stand there.
    pub hideout_door: bool,
    /// Species that can learn Cut: the starters that can, then the grass
    /// Pokémon on Route 24, Oddish and Bellsprout's lines.
    pub cutters: [u8; 10],
    /// Route 24's two of them, worth a Poké Ball for Cut.
    pub catch: [u8; 2],
}

const RED: Edition = Edition {
    follower: None,
    ball_sprite: 0x3D,
    starter_ball: None,
    levels: None,
    stop_before: None,
    hideout_door: true,
    cutters: [0x99, 0x09, 0x9A, 0xB0, 0xB2, 0xB4, 0xB9, 0xBA, 0xBC, 0xBD],
    catch: [0xB9, 0xBC],
};

const YELLOW: Edition = Edition {
    follower: Some(15),
    ball_sprite: 0x47,
    starter_ball: Some(7),
    levels: Some([16, 18, 30, 40, 48, 0]),
    // Pikachu keeps only electric moves, and Silph Co.'s Giovanni leads
    // with ground types.
    stop_before: Some(SABRINA),
    hideout_door: false,
    ..RED
};

/// EvoYellow renumbers the species. Its Celadon Gym fields ground types,
/// and Erika six grass Pokémon at levels 46 to 50.
const EVO_YELLOW: Edition = Edition {
    stop_before: Some(ERIKA),
    cutters: [0xE7, 0xE8, 0xE9, 0xE5, 0xEA, 0xED, 0x87, 0x88, 0x8A, 0x8B],
    catch: [0x87, 0x8A],
    ..YELLOW
};

impl Edition {
    pub fn of(r: Revision) -> &'static Self {
        match r {
            Revision::Red | Revision::Blue => &RED,
            Revision::Yellow => &YELLOW,
            Revision::EvoYellow => &EVO_YELLOW,
        }
    }
}
