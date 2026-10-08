# `vinyl`

![`vinyl`](../media/vinyl.gif)

A record turning on a turntable, seen from above. The label and a few specks of dust turn at 33 1/3 rpm, the sheen on the grooves stays put, and a J-shaped tonearm rests in the outer grooves.

One of ascii.rest's character pieces, drawn in one ink. No knobs; see [the ascii.rest ports](../ascii-rest.md#character-pieces).

Source: [`src/ascii_rest/vinyl.rs`](../../src/ascii_rest/vinyl.rs).

## `vinyl-wide`

![`vinyl-wide`](../media/vinyl-wide.gif)

A DJ console across the panel: two turntables either side of a mixer. A single deck stretched to 3.2:1 would be one record and a lot of plinth, while two decks and a mixer is the shape that kit really has at that width. The right deck runs 2% fast, so the two labels drift in and out of step. On the mixer, the level meters kick on a 124 bpm beat, the EQ knobs and channel faders get nudged now and then, and the crossfader sweeps from deck to deck every 24 seconds. Each tonearm plays its side: it tracks slowly in to the run-out groove, the cue lever pops it up, it swings back to its rest, waits, then swings out and cues down at the lead-in again. The right deck runs most of a side behind the left, so one changes over while the other plays. On pine each deck is upstream's turntable at upstream's size. Elsewhere the decks scale to the height, or to whatever width two of them and the mixer leave.

In colour: an amber plinth and a brass tonearm. Each record is black with grey sheen on its grooves, a silver rim and spindle, white dust, and a cream print on its label: red on the left deck, blue on the right. The mixer's meters climb from green through yellow to red, under cyan knobs and white fader caps in a grey box. A lifted headshell and a raised cue lever turn white.

Knobs: `VINYL_SIDE_SECS` (default 240, 20..=3600), how long a side plays before the arm lifts; `ASCII_REST_TEXT_CELL_W` / `ASCII_REST_TEXT_CELL_H` and `ASCII_REST_TITLE`; see [full-screen twins](../ascii-rest.md#full-screen-twins).

Source: [`src/ascii_rest/vinyl.rs`](../../src/ascii_rest/vinyl.rs), the original's own module.
