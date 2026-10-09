# `desert-night`

![`desert-night`](../media/desert-night.gif)

A moonless desert under the milky way. The galaxy's bright core sits low over a lone acacia on a dune crest and arches up across the sky, split by its dark dust lane; a distant town warms the far horizon and catches the dunes' faces. Stars twinkle, sand glints along the ridges, and now and then a meteor crosses.

One of ascii.rest's thirteen halftone scenes: shaded cell by cell and drawn as dots whose size is their brightness, composed for whatever panel it lands on. `ASCII_REST_TOUR=1` sets a camera slowly touring it. The shared engine, the tour and its knobs are in [the ascii.rest ports](../ascii-rest.md#scenes).

## At any size

The camera keeps its scale and the panel decides how much of the dune field it sees. On a panel wider than upstream's 2:1 the view widens with the acacia and the galaxy's core held right of centre, the town moves out left, and the milky way sweeps further so its arch spans the sky. On a narrower one (4:3, square) the camera pans so the acacia stays whole near the right edge, and the arch leaves by the left edge sooner. On a portrait panel the extra rows are mostly sky above, with the arch turned steeper to climb it, and nearer sand below.

## Knobs

None of its own; `ASCII_REST_TOUR*` and `ASCII_REST_TITLE` are in [the ascii.rest ports](../ascii-rest.md#scenes). On upstream's 200x100 grid it is upstream's picture cell for cell.

Source: [`src/ascii_rest/desert_night.rs`](../../src/ascii_rest/desert_night.rs).
