# `alpine-dawn`

![`alpine-dawn`](../media/alpine-dawn.gif)

Jagged snow peaks catch the first pink light on their east faces while their flanks stay in blue shadow. Mist pools along the far shore and a still lake mirrors it all. The light warms toward gold as the sun clears the ridge, the mist drifts, and slow ripples cross the water.

One of ascii.rest's thirteen halftone scenes: shaded cell by cell and drawn as dots whose size is their brightness, composed for whatever panel it lands on. `ASCII_REST_TOUR=1` sets a camera slowly touring it. The shared engine, the tour and its knobs are in [the ascii.rest ports](../ascii-rest.md#scenes).

## At any size

The camera keeps upstream's scale and the range stays centred. On a panel wider than upstream's 2:1 the view takes in more of the valley: three more peaks rise out of the lake at the flanks by 3:1, and past 3.2:1 more fill the gaps between them and the range. On a narrower one (16:9, 4:3, square) the summits close in toward the middle, and the sun, whose low notch the closer peaks would hide, stands a little higher over the ridge. On a portrait panel the extra rows split between sky above, where the stars linger, and lake below.

## Knobs

None of its own; `ASCII_REST_TOUR*` and `ASCII_REST_TITLE` are in [the ascii.rest ports](../ascii-rest.md#scenes). On upstream's 200x100 grid it is upstream's picture cell for cell.

Source: [`src/ascii_rest/alpine_dawn.rs`](../../src/ascii_rest/alpine_dawn.rs).
