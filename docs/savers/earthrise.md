# `earthrise`

![`earthrise`](../media/earthrise.gif)

The Earth coming up over the lunar horizon. The sun is low on the right, so every crater rim and boulder throws a long black shadow across the grey ground, and the same light makes a gibbous Earth with a clean line between day and night. The Earth turns, its clouds drift, it climbs very slowly, and a few bright stars breathe.

One of ascii.rest's thirteen halftone scenes: shaded cell by cell and drawn as dots whose size is their brightness, composed for whatever panel it lands on. `ASCII_REST_TOUR=1` sets a camera slowly touring it. The shared engine, the tour and its knobs are in [the ascii.rest ports](../ascii-rest.md#scenes).

## At any size

The camera stands on the same ground and its view widens or narrows with the panel. On a panel wider than upstream's 2:1 it takes in more of the highlands on the left, two more bright stars and a longer galaxy band, with the Earth 41 columns right of centre. On a narrower one (4:3, square) the Earth draws in toward the centre so it stays whole. On a portrait panel the extra rows are mostly sky over the Earth, and the camera's ground starts nearer so it still reaches the bottom row.

## Knobs

None of its own; `ASCII_REST_TOUR*` and `ASCII_REST_TITLE` are in [the ascii.rest ports](../ascii-rest.md#scenes). On upstream's 200x100 grid it is upstream's picture cell for cell.

Source: [`src/ascii_rest/earthrise.rs`](../../src/ascii_rest/earthrise.rs).
