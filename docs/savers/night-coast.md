# `night-coast`

![`night-coast`](../media/night-coast.gif)

A lighthouse on a wooded headland under moonlit clouds. The beam turns every eight seconds, the clouds drift, and the sea carries the moon's road and the lamp's reflection.

One of ascii.rest's thirteen halftone scenes: shaded cell by cell and drawn as dots whose size is their brightness, composed for whatever panel it lands on. `ASCII_REST_TOUR=1` sets a camera slowly touring it. The shared engine, the tour and its knobs are in [the ascii.rest ports](../ascii-rest.md#scenes).

## At any size

The headland keeps the left and the open sea the rest. On a panel wider than upstream's 2:1 the moon, the far shore and the beam's reach move out with the frame, and a second low hummock rises left of the moon's road. On a narrower one (4:3, square) the headland draws in to the cottage's edge so the moon keeps open water under it. On a portrait panel the extra rows are sky above, where the moon climbs, and sea below.

## Knobs

None of its own; `ASCII_REST_TOUR*` and `ASCII_REST_TITLE` are in [the ascii.rest ports](../ascii-rest.md#scenes). On upstream's 200x100 grid it is upstream's picture cell for cell.

Source: [`src/ascii_rest/night_coast.rs`](../../src/ascii_rest/night_coast.rs).
