# `storm-plains`

![`storm-plains`](../media/storm-plains.gif)

An anvil thunderhead at dusk over open wheat country. The sun has just set behind the farmhouse, so the storm's top and western flank still catch its light while the base sinks into slate shadow. Lightning flickers inside the cloud, now and then a bolt reaches the ground, rain curtains drift under the base and the wheat moves in gusts.

One of ascii.rest's thirteen halftone scenes: shaded cell by cell and drawn as dots whose size is their brightness, composed for whatever panel it lands on. `ASCII_REST_TOUR=1` sets a camera slowly touring it. The shared engine, the tour and its knobs are in [the ascii.rest ports](../ascii-rest.md#scenes).

## At any size

The farm keeps the western corner and the storm stands whole to the east of it. On a panel wider than upstream's 2:1 the storm moves east with the frame (50 columns at 3.2:1) and its anvil streams on across the extra sky, trailing thinner rain. On a narrower one (16:9, 4:3, square) the storm draws in over the farm until its flanking towers hang above the yard, the farm steps up to 14 columns toward the edge, and the road bends less so it still reaches the bottom of the frame. On a portrait panel the extra rows split between sky above the anvil and wheat below the horizon.

## Knobs

None of its own; `ASCII_REST_TOUR*` and `ASCII_REST_TITLE` are in [the ascii.rest ports](../ascii-rest.md#scenes). On upstream's 200x100 grid it is upstream's picture cell for cell.

Source: [`src/ascii_rest/storm_plains/mod.rs`](../../src/ascii_rest/storm_plains/mod.rs).
