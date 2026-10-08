# `vinyl`

A record turning on a turntable, seen from above. The label and a few specks of dust turn at 33 1/3 rpm, the sheen on the grooves stays put, and a J-shaped tonearm rests in the outer grooves.

One of ascii.rest's character pieces, drawn in one ink. No knobs; see [the ascii.rest ports](../ascii-rest.md#character-pieces).

Source: [`src/ascii_rest/vinyl.rs`](../../src/ascii_rest/vinyl.rs).

## `vinyl-wide`

A DJ console across the panel: two turntables either side of a mixer. A single deck stretched to 3.2:1 would be one record and a lot of plinth, while two decks and a mixer is the shape that kit really has at that width. The right deck runs 2% fast, so the two labels drift in and out of step. On the mixer, the level meters kick on a 124 bpm beat, the EQ knobs and channel faders get nudged now and then, and the crossfader sweeps from deck to deck every 24 seconds. On pine each deck is upstream's turntable at upstream's size. Elsewhere the decks scale to the height, or to whatever width two of them and the mixer leave.

Knobs: `ASCII_REST_TEXT_CELL_W` / `ASCII_REST_TEXT_CELL_H` and `ASCII_REST_TITLE`; see [full-screen twins](../ascii-rest.md#full-screen-twins).

Source: [`src/ascii_rest/vinyl.rs`](../../src/ascii_rest/vinyl.rs), the original's own module.
