# `synthwave`

![`synthwave`](../media/synthwave.gif)

The eighties horizon. A grid floor rolls toward the viewer under a setting sun cut by thinning stripes, behind a ridge of mountains.

One of ascii.rest's character pieces, drawn at whatever size and shape the panel is. See [the ascii.rest ports](../ascii-rest.md#character-pieces).

## At any size

The sky and the floor keep upstream's shares of the height, and the perspective is worked out again for the panel: the cross lines are spaced for its floor, and as many rails fan out from the vanishing point as it takes to reach both sides. The ridge runs edge to edge and stays low under the sun.

The sun and the ridge grow with the height of the sky. On a square or portrait panel that would make the sun wider than the panel, so there they scale with the width instead. The sun stays a disc that fits, with stars filling the taller sky above it.

In colour: the sun shades from gold at its crown to hot pink at the horizon, over purple mountains and white stars. The floor is magenta crossed by cyan rails, on a deep purple night.

## Knobs

- `SYNTHWAVE_COLOR` (0..1, default 1): 0 draws it in upstream's one pink ink on black.
- `ASCII_REST_TEXT_CELL_W` / `ASCII_REST_TEXT_CELL_H` and `ASCII_REST_TITLE`; see [character pieces](../ascii-rest.md#character-pieces).

With `SYNTHWAVE_COLOR` at 0, on upstream's 65x28 grid, it is upstream's picture cell for cell.

Source: [`src/ascii_rest/synthwave.rs`](../../src/ascii_rest/synthwave.rs).
