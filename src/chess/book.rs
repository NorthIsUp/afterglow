//! A small opening book: a named line per entry, played before the engines
//! take over, so no two games start alike.

pub const BOOK: &[(&str, &str)] = &[
    (
        "Ruy Lopez",
        "e2e4 e7e5 g1f3 b8c6 f1b5 a7a6 b5a4 g8f6 e1g1 f8e7",
    ),
    (
        "Ruy Lopez, Berlin",
        "e2e4 e7e5 g1f3 b8c6 f1b5 g8f6 e1g1 f6e4 d2d4 e4d6",
    ),
    (
        "Italian Game",
        "e2e4 e7e5 g1f3 b8c6 f1c4 f8c5 c2c3 g8f6 d2d3 d7d6",
    ),
    (
        "Two Knights",
        "e2e4 e7e5 g1f3 b8c6 f1c4 g8f6 d2d3 f8e7 e1g1 e8g8",
    ),
    (
        "Scotch Game",
        "e2e4 e7e5 g1f3 b8c6 d2d4 e5d4 f3d4 g8f6 d4c6 b7c6",
    ),
    (
        "Petrov Defence",
        "e2e4 e7e5 g1f3 g8f6 f3e5 d7d6 e5f3 f6e4 d2d4 d6d5",
    ),
    (
        "King's Gambit",
        "e2e4 e7e5 f2f4 e5f4 g1f3 g7g5 h2h4 g5g4 f3e5",
    ),
    ("Vienna Game", "e2e4 e7e5 b1c3 g8f6 f2f4 d7d5 f4e5 f6e4"),
    (
        "Sicilian, Najdorf",
        "e2e4 c7c5 g1f3 d7d6 d2d4 c5d4 f3d4 g8f6 b1c3 a7a6",
    ),
    (
        "Sicilian, Dragon",
        "e2e4 c7c5 g1f3 d7d6 d2d4 c5d4 f3d4 g8f6 b1c3 g7g6",
    ),
    (
        "Sicilian, Sveshnikov",
        "e2e4 c7c5 g1f3 b8c6 d2d4 c5d4 f3d4 g8f6 b1c3 e7e5",
    ),
    (
        "Sicilian, Alapin",
        "e2e4 c7c5 c2c3 g8f6 e4e5 f6d5 d2d4 c5d4 g1f3",
    ),
    (
        "French, Winawer",
        "e2e4 e7e6 d2d4 d7d5 b1c3 f8b4 e4e5 c7c5 a2a3 b4c3 b2c3",
    ),
    (
        "French, Advance",
        "e2e4 e7e6 d2d4 d7d5 e4e5 c7c5 c2c3 b8c6 g1f3 d8b6",
    ),
    (
        "Caro-Kann",
        "e2e4 c7c6 d2d4 d7d5 b1c3 d5e4 c3e4 c8f5 e4g3 f5g6",
    ),
    (
        "Scandinavian",
        "e2e4 d7d5 e4d5 d8d5 b1c3 d5a5 d2d4 g8f6 g1f3 c8f5",
    ),
    (
        "Pirc Defence",
        "e2e4 d7d6 d2d4 g8f6 b1c3 g7g6 f2f4 f8g7 g1f3 e8g8",
    ),
    (
        "Alekhine's Defence",
        "e2e4 g8f6 e4e5 f6d5 d2d4 d7d6 g1f3 c8g4",
    ),
    (
        "Queen's Gambit Declined",
        "d2d4 d7d5 c2c4 e7e6 b1c3 g8f6 c1g5 f8e7 e2e3 e8g8",
    ),
    (
        "Queen's Gambit Accepted",
        "d2d4 d7d5 c2c4 d5c4 g1f3 g8f6 e2e3 e7e6 f1c4 c7c5",
    ),
    (
        "Slav Defence",
        "d2d4 d7d5 c2c4 c7c6 g1f3 g8f6 b1c3 d5c4 a2a4 c8f5",
    ),
    (
        "King's Indian",
        "d2d4 g8f6 c2c4 g7g6 b1c3 f8g7 e2e4 d7d6 g1f3 e8g8",
    ),
    (
        "Grunfeld",
        "d2d4 g8f6 c2c4 g7g6 b1c3 d7d5 c4d5 f6d5 e2e4 d5c3 b2c3",
    ),
    (
        "Nimzo-Indian",
        "d2d4 g8f6 c2c4 e7e6 b1c3 f8b4 e2e3 e8g8 f1d3 d7d5",
    ),
    (
        "Queen's Indian",
        "d2d4 g8f6 c2c4 e7e6 g1f3 b7b6 g2g3 c8b7 f1g2 f8e7",
    ),
    (
        "Benoni",
        "d2d4 g8f6 c2c4 c7c5 d4d5 e7e6 b1c3 e6d5 c4d5 d7d6",
    ),
    (
        "Dutch Defence",
        "d2d4 f7f5 g2g3 g8f6 f1g2 g7g6 g1f3 f8g7 e1g1 e8g8",
    ),
    (
        "London System",
        "d2d4 d7d5 c1f4 g8f6 e2e3 c7c5 c2c3 b8c6 g1f3 e7e6",
    ),
    (
        "English Opening",
        "c2c4 e7e5 b1c3 g8f6 g1f3 b8c6 g2g3 d7d5 c4d5 f6d5",
    ),
    (
        "Reti Opening",
        "g1f3 d7d5 c2c4 e7e6 g2g3 g8f6 f1g2 f8e7 e1g1 e8g8",
    ),
    (
        "Bird's Opening",
        "f2f4 d7d5 g1f3 g8f6 e2e3 g7g6 b2b3 f8g7 c1b2",
    ),
    (
        "Catalan",
        "d2d4 g8f6 c2c4 e7e6 g2g3 d7d5 f1g2 f8e7 g1f3 e8g8",
    ),
];
