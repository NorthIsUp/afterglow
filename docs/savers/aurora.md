# `aurora`

![`aurora`](../media/aurora.gif)

A curtain of light over a spruce treeline at night. Its lower hem is brightest and ripples; streaks rise from it and fade out below a clear sky of stars, and light drifts along it in slow surges.

One of ascii.rest's character pieces, drawn at whatever size and shape the panel is. See [the ascii.rest ports](../ascii-rest.md#character-pieces).

## At any size

The curtain folds on across the whole width and the treeline runs edge to edge. The hem, the streaks and the trees are scaled to the panel's height, and the ground line and hem keep upstream's place above the bottom. On a panel wider than upstream's the hem also rises and sags in a slow swell along it. On a narrow panel (square, portrait) they scale to the width instead, so the trees stay spruces rather than towers, and the rows left over become sky: the streaks reach up into it and the stars fill it as densely as upstream's.

In colour: the curtain is green and teal low, trading places in slow bands along it, violet at the top of its streaks and magenta at its hem. The stars are white and blue-white, the spruces dark teal, and the sky a deep night blue.

## Knobs

- `AURORA_COLOR` (0..1, default 1): 0 draws it in upstream's one green ink on black.
- `ASCII_REST_TEXT_CELL_W` / `ASCII_REST_TEXT_CELL_H` and `ASCII_REST_TITLE`; see [character pieces](../ascii-rest.md#character-pieces).

With `AURORA_COLOR` at 0, on upstream's 64x20 grid, it is upstream's picture cell for cell.

Source: [`src/ascii_rest/aurora.rs`](../../src/ascii_rest/aurora.rs).
