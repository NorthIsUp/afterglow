# `reaction-diffusion`

![`reaction-diffusion`](../media/reaction-diffusion.gif)

A Gray-Scott reaction whose spots give way to stripes. Every few seconds the kill rate rises, the pattern dies back to a few survivors, and they grow out again.

One of ascii.rest's character pieces, drawn at whatever size and shape the panel is. See [the ascii.rest ports](../ascii-rest.md#character-pieces).

## At any size

The dish is the panel, two cells to a character as upstream's is. Spots turn to stripes along its long side: left to right on a landscape panel, top to bottom on a portrait one. The seeds scale with the dish's area, so a big dish fills as fast as a small one. A grid smaller than upstream's runs upstream's dish and shows the middle of it, because a smaller dish cannot hold a spot.

It steps at upstream's 1,000 a second up to 7,000 cells and slower above that, so a bigger dish evolves more slowly rather than costing more: pine's 8,000 cells take 875 steps a second and 1080p's 14,400 take 486, so a die-back comes round every 9 and 16 seconds instead of 8. A dish no bigger than upstream's steps in upstream's f64 arithmetic, cell by cell. A bigger one steps in f32 over whole rows, which the compiler vectorises, so a dish 2.8 times upstream's area on pine costs less than upstream's does.

In colour, by concentration: thin violet edges warm through rose and orange to a pale-gold core.

## Knobs

- `REACTION_DIFFUSION_COLOR` (0..1, default 1): 0 draws it in upstream's one orange ink on black.
- `ASCII_REST_TEXT_CELL_W` / `ASCII_REST_TEXT_CELL_H` and `ASCII_REST_TITLE`; see [character pieces](../ascii-rest.md#character-pieces).

With `REACTION_DIFFUSION_COLOR` at 0, on upstream's 60x24 grid, it is upstream's picture cell for cell.

Source: [`src/ascii_rest/reaction_diffusion.rs`](../../src/ascii_rest/reaction_diffusion.rs).
