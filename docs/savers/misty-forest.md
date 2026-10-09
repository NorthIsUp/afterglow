# `misty-forest`

![`misty-forest`](../media/misty-forest.gif)

Morning in a pine forest. Ridge after ridge of pines recedes into fog, each paler than the one in front, with mist lying in sheets in the valleys between them. A low sun sits behind the farthest trees and sends beams slanting down through the fog to a clearing on the forest floor. The fog drifts, the beams shimmer, and motes of dust float in the light.

One of ascii.rest's thirteen halftone scenes: shaded cell by cell and drawn as dots whose size is their brightness, composed for whatever panel it lands on. `ASCII_REST_TOUR=1` sets a camera slowly touring it. The shared engine, the tour and its knobs are in [the ascii.rest ports](../ascii-rest.md#scenes).

## At any size

The sun and its clearing keep the right third and the two giant pines frame the edges. On a panel wider than upstream's 2:1 the ridges and fog run on west, and from about 2.6:1 a young pine stands off-centre between the giants so the middle is not empty. On a narrower one (4:3, square) the giants step further out of the frame so they frame it rather than wall it in. On a portrait panel half the extra rows are sky above and the rest push the ridges apart, so they recede down the frame to a deeper forest floor.

## Knobs

None of its own; `ASCII_REST_TOUR*` and `ASCII_REST_TITLE` are in [the ascii.rest ports](../ascii-rest.md#scenes). On upstream's 200x100 grid it is upstream's picture cell for cell.

Source: [`src/ascii_rest/misty_forest.rs`](../../src/ascii_rest/misty_forest.rs).
