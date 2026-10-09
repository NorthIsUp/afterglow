# `ocean-sunset`

![`ocean-sunset`](../media/ocean-sunset.gif)

Golden hour at sea. The sun rests on the horizon beyond a dark pine headland, heaped cloud overhead lit from below, a glitter path running across the water toward us, and long swells rolling in, their crests catching the light.

One of ascii.rest's thirteen halftone scenes: shaded cell by cell and drawn as dots whose size is their brightness, composed for whatever panel it lands on. `ASCII_REST_TOUR=1` sets a camera slowly touring it. The shared engine, the tour and its knobs are in [the ascii.rest ports](../ascii-rest.md#scenes).

## At any size

The headland keeps the left and the sun sits on the horizon right of centre, with the sloop just left of it. On a panel wider than upstream's 2:1 the headland is drawn larger to hold its weight across the wider sea, the sun and the sloop move out with the frame, and from about 2.3:1 a far, low island rises right of the sun. On a narrower one (4:3, square) the headland is drawn smaller and the sun comes in. On a portrait panel the extra rows are split between sky above, where the cloud deck and the first stars climb with it, and sea below.

## Knobs

None of its own; `ASCII_REST_TOUR*` and `ASCII_REST_TITLE` are in [the ascii.rest ports](../ascii-rest.md#scenes). On upstream's 200x100 grid it is upstream's picture cell for cell.

Source: [`src/ascii_rest/ocean_sunset.rs`](../../src/ascii_rest/ocean_sunset.rs).
