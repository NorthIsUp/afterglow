# `deep-reef`

![`deep-reef`](../media/deep-reef.gif)

Looking along a coral reef from a few metres down. The sun is a bright blaze in the rippled surface, shafts of light fan down from it, kelp sways in the swell, a school of fish wheels through the dark water, bubbles rise and caustics crawl over the sand.

One of ascii.rest's thirteen halftone scenes: shaded cell by cell and drawn as dots whose size is their brightness, composed for whatever panel it lands on. `ASCII_REST_TOUR=1` sets a camera slowly touring it. The shared engine, the tour and its knobs are in [the ascii.rest ports](../ascii-rest.md#scenes).

## At any size

The two reefs keep the sides, the kelp frames them, the open sand runs down the middle and the sun stays above it. On a panel wider than upstream's 2:1 both reefs broaden with more brain corals and branching coral, the sand widens with more bommies, and a third, hazier kelp stands out on it. On a narrower one (16:9, 4:3, square) the reefs draw in, coral heads that would crowd each other drop out, and the side bommies go as the sand narrows, down to the big one alone on a square. On a portrait panel most of the extra rows are open water between the surface and the reef, where the school swims; the rest bring the sand nearer.

## Knobs

None of its own; `ASCII_REST_TOUR*` and `ASCII_REST_TITLE` are in [the ascii.rest ports](../ascii-rest.md#scenes). On upstream's 200x100 grid it is upstream's picture cell for cell.

Source: [`src/ascii_rest/deep_reef/`](../../src/ascii_rest/deep_reef/mod.rs).
