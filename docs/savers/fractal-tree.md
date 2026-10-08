# `fractal-tree`

![`fractal-tree`](../media/fractal-tree.gif)

A trunk that forks, and forks again, seven times over, each limb a little shorter than its parent, the outer twigs gathered into lobes of leaves lit from the upper left. Wind bends every limb, the tips most.

One of ascii.rest's character pieces, drawn at whatever size and shape the panel is. See [the ascii.rest ports](../ascii-rest.md#character-pieces).

## At any size

Upstream's tree stands in the middle, scaled to the panel's height, or to its width if that is shorter, so the crown is never cropped. On a panel more than about one and a half of the tree's frames wide (pine's 3.2:1, 16:9) it becomes a grove. Smaller trees grown from other seeds walk out either side until one stands past each edge, all on one ground line that runs edge to edge. The wind reaches each tree a moment after the one to its left, so a gust crosses the grove. On a narrower panel (4:3, square, portrait) the tree stands alone, and its ground thins to dots short of the edges as upstream's does. On a panel taller than the scaled tree, a longer trunk lifts the crown so the top half is not left empty.

In colour: brown trunks darkening into the branches and olive twigs, with leaves in four greens by their light. Every other tree out from the middle is a cherry with a third of its brightest leaves in pink blossom; the rest show a few. The ground line is earth brown.

## Knobs

- `FRACTAL_TREE_COLOR` (0..1, default 1): 0 draws it in upstream's one green ink on black.
- `ASCII_REST_TEXT_CELL_W` / `ASCII_REST_TEXT_CELL_H` and `ASCII_REST_TITLE`; see [character pieces](../ascii-rest.md#character-pieces).

With `FRACTAL_TREE_COLOR` at 0, on upstream's 60x24 grid, it is upstream's picture cell for cell.

Source: [`src/ascii_rest/fractal_tree.rs`](../../src/ascii_rest/fractal_tree.rs).
