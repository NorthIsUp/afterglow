# `kyoto-dusk`

![`kyoto-dusk`](../media/kyoto-dusk.gif)

A five-storey pagoda dark against an indigo to rose sky with a thin crescent moon, a temple pond holding its reflection, and in front a cherry tree in full bloom lit from below by a stone lantern. Petals fall and drift through the frame, the lantern flickers, the pond shivers.

One of ascii.rest's thirteen halftone scenes: shaded cell by cell and drawn as dots whose size is their brightness, composed for whatever panel it lands on. `ASCII_REST_TOUR=1` sets a camera slowly touring it. The shared engine, the tour and its knobs are in [the ascii.rest ports](../ascii-rest.md#scenes).

## At any size

The cherry tree and lantern keep the left and the pagoda the far bank. On a panel wider than upstream's 2:1 the pagoda, the moon and the hills move out with the frame, and from about 3:1 a temple hall stands beside the pagoda. On a narrower one (16:9, 4:3, square) the tree's limbs draw in toward its trunk and the pagoda moves toward it, so the blossom keeps clear of its roofs; when there is no sky right of the pagoda the moon hangs left of its spire. On a portrait panel the near bank stays on the bottom row and the extra rows are mostly pond, so the pagoda stands above the crown and its reflection runs under it, with a little more sky on top.

## Knobs

None of its own; `ASCII_REST_TOUR*` and `ASCII_REST_TITLE` are in [the ascii.rest ports](../ascii-rest.md#scenes). On upstream's 200x100 grid it is upstream's picture cell for cell.

Source: [`src/ascii_rest/kyoto_dusk/`](../../src/ascii_rest/kyoto_dusk/mod.rs).
