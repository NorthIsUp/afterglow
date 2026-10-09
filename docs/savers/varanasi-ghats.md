# `varanasi-ghats`

![`varanasi-ghats`](../media/varanasi-ghats.gif)

Dusk on the Ganga. Stepped ghats run along the bank and away toward the afterglow, temple spires black against an indigo to amber sky. Priests raise the aarti lamps on the steps, diyas drift downstream on the dark water, and a boatman rows slowly across the bright reach.

One of ascii.rest's thirteen halftone scenes: shaded cell by cell and drawn as dots whose size is their brightness, composed for whatever panel it lands on. `ASCII_REST_TOUR=1` sets a camera slowly touring it. The shared engine, the tour and its knobs are in [the ascii.rest ports](../ascii-rest.md#scenes).

## At any size

The ghats always run from the near left away to the glow, and everything on them (spires, umbrellas, lamps) is placed along that run. On a panel wider than upstream's 2:1 the ghats run further before they meet the far bank, with two more spires, a fifth priest and two more umbrellas, and a broad reach of river opens beyond the glow. On a narrower one (16:9, 4:3, square) the glow comes in and the ghats recede faster, and a spire, priest or umbrella that would crowd its neighbour drops out. On a portrait panel the extra rows are sky above the far bank and river below it, where the boatman rows.

## Knobs

None of its own; `ASCII_REST_TOUR*` and `ASCII_REST_TITLE` are in [the ascii.rest ports](../ascii-rest.md#scenes). On upstream's 200x100 grid it is upstream's picture cell for cell.

Source: [`src/ascii_rest/varanasi_ghats/`](../../src/ascii_rest/varanasi_ghats/mod.rs).
