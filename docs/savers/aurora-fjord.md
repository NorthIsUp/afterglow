# `aurora-fjord`

![`aurora-fjord`](../media/aurora-fjord.gif)

Curtains of aurora ripple over a fjord between snowy mountains. The still water holds a broken shimmer of them, and a red cabin on the far shore keeps its lamps lit.

One of ascii.rest's thirteen halftone scenes: shaded cell by cell and drawn as dots whose size is their brightness, composed for whatever panel it lands on. `ASCII_REST_TOUR=1` sets a camera slowly touring it. The shared engine, the tour and its knobs are in [the ascii.rest ports](../ascii-rest.md#scenes).

## At any size

The fjord runs in between its two ranges, with the cabin on the shelf to its right, and the curtains span the whole sky. On a panel wider than upstream's 2:1 the fjord moves to the centre and each range raises two more peaks; past 3.2:1 the peaks spread out to the ends. On a narrower one (4:3, square) the ranges and the cabin close in on the fjord, the peaks keeping their height and slope. On a portrait panel the curtains stretch up into a taller sky and the water below gets longer.

## Knobs

None of its own; `ASCII_REST_TOUR*` and `ASCII_REST_TITLE` are in [the ascii.rest ports](../ascii-rest.md#scenes). On upstream's 200x100 grid it is upstream's picture cell for cell.

Source: [`src/ascii_rest/aurora_fjord.rs`](../../src/ascii_rest/aurora_fjord.rs).
