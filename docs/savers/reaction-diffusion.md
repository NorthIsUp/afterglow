# `reaction-diffusion`

![`reaction-diffusion`](../media/reaction-diffusion.gif)

A Gray-Scott reaction whose spots on the left give way to stripes on the right. Every few seconds the kill rate rises, the pattern dies back to a few survivors, and they grow out again.

One of ascii.rest's character pieces, drawn in one ink. No knobs; see [the ascii.rest ports](../ascii-rest.md#character-pieces).

Source: [`src/ascii_rest/reaction_diffusion.rs`](../../src/ascii_rest/reaction_diffusion.rs).

## `reaction-diffusion-wide`

![`reaction-diffusion-wide`](../media/reaction-diffusion-wide.gif)

The same reaction in a dish the size of the panel, two cells to a character as upstream's is. Spots at the left edge turn to stripes at the right, and the seeds scale with the dish's area. It steps at upstream's 1,000 a second up to 7,000 cells and slower above that, so a bigger dish evolves more slowly rather than costing more: pine's 8,000 cells take 875 steps a second and 1080p's 14,400 take 486, so a die-back comes round every 9 and 16 seconds instead of 8. Upstream's arithmetic is f64 cell by cell. The twin steps in f32 over whole rows, which the compiler vectorises, so a dish 2.8 times upstream's area on pine costs less than the original does. A grid smaller than upstream's runs upstream's dish and shows the middle of it, because a smaller dish cannot hold a spot.

In colour, by concentration: thin violet edges warm through rose and orange to a pale-gold core.

Knobs: `ASCII_REST_TEXT_CELL_W` / `ASCII_REST_TEXT_CELL_H` and `ASCII_REST_TITLE`; see [full-screen twins](../ascii-rest.md#full-screen-twins).

Source: [`src/ascii_rest/reaction_diffusion.rs`](../../src/ascii_rest/reaction_diffusion.rs), the original's own module.
