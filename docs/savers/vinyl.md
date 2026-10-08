# `vinyl`

![`vinyl`](../media/vinyl.gif)

A record turning on a turntable, seen from above. The label and a few specks of dust turn at 33 1/3 rpm, the sheen on the grooves stays put, and a J-shaped tonearm plays the record.

One of ascii.rest's character pieces, drawn at whatever size and shape the panel is. See [the ascii.rest ports](../ascii-rest.md#character-pieces).

## At any size

On a narrow or squarish panel (4:3, square, portrait) it is one turntable, as large as fits and centred, with the plinth's edge running round the whole panel.

On a wide panel (pine's 3.2:1, 16:9) it becomes a DJ console: two turntables either side of a mixer. One deck stretched that wide would be one record and a lot of plinth, and two decks with a mixer is the shape that kit really has at that width. It switches to two decks once each would still be at least 60% of the size one deck alone would be. The right deck runs 2% fast, so the two labels drift in and out of step. On the mixer, which is as tall as the decks, the level meters kick on a 124 bpm beat, the EQ knobs and channel faders get nudged now and then, and the crossfader sweeps from deck to deck every 24 seconds. On pine each deck is upstream's turntable at upstream's size.

Each tonearm plays its side. It tracks slowly in to the run-out groove, the cue lever pops it up, and it swings back to its rest. After a wait it swings out and cues down at the lead-in again. The right deck runs most of a side behind the left, so one changes over while the other plays. Upstream's arm rests in the outer grooves.

In colour: an amber plinth and a brass tonearm. Each record is black with grey sheen on its grooves, a silver rim and spindle, white dust, and a cream print on its label, red on the left deck and blue on the right. The mixer's meters climb from green through yellow to red, under cyan knobs and white fader caps in a grey box. A lifted headshell and a raised cue lever turn white.

## Knobs

- `VINYL_SIDE_SECS` (0..3600, default 240): how long a side plays before the arm lifts. 0 leaves the arm resting in the outer grooves, as upstream does.
- `VINYL_COLOR` (0..1, default 1): 0 draws it in upstream's one amber ink on black.
- `ASCII_REST_TEXT_CELL_W` / `ASCII_REST_TEXT_CELL_H` and `ASCII_REST_TITLE`; see [character pieces](../ascii-rest.md#character-pieces).

With both of its own knobs at 0, on upstream's 64x25 grid, it is upstream's picture cell for cell.

Source: [`src/ascii_rest/vinyl.rs`](../../src/ascii_rest/vinyl.rs).
