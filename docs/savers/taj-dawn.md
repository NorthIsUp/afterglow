# `taj-dawn`

![`taj-dawn`](../media/taj-dawn.gif)

The Taj Mahal at first light, seen down its long reflecting canal between rows of cypress. The sun has just cleared the red sandstone mosque on the left; the haze and the clouds drift, the canal's reflection ripples, light glints along its far end, and a few birds cross.

One of ascii.rest's thirteen halftone scenes: shaded cell by cell and drawn as dots whose size is their brightness, composed for whatever panel it lands on. `ASCII_REST_TOUR=1` sets a camera slowly touring it. The shared engine, the tour and its knobs are in [the ascii.rest ports](../ascii-rest.md#scenes).

## At any size

The Taj stays at the canal's vanishing point with the mosque on its left and the sun just over it. On a panel wider than upstream's 2:1 the garden opens out either side, and from about 2.6:1 the jawab, the mosque's twin, answers it on the right. On a narrower one (4:3, square) the mosque draws in behind the Taj's left minaret and the cypress rows, running off the left edge on a square, with the sun climbing over its middle. On a portrait panel the extra rows are sky above, where the last stars hang, and canal below.

## Knobs

None of its own; `ASCII_REST_TOUR*` and `ASCII_REST_TITLE` are in [the ascii.rest ports](../ascii-rest.md#scenes). On upstream's 200x100 grid it is upstream's picture cell for cell.

Source: [`src/ascii_rest/taj_dawn/mod.rs`](../../src/ascii_rest/taj_dawn/mod.rs), with the Taj and the mosque's shapes in [`taj.rs`](../../src/ascii_rest/taj_dawn/taj.rs).
