# `double-pendulum`

Two equal rods hung end to end from one pivot, stepped with RK4. Chaotic, so it never repeats; the lower bob leaves a fading trail.

One of ascii.rest's character pieces, drawn in one ink. No knobs; see [the ascii.rest ports](../ascii-rest.md#character-pieces).

Source: [`src/ascii_rest/double_pendulum.rs`](../../src/ascii_rest/double_pendulum.rs).

## `double-pendulum-wide`

As many pendulums as fit side by side, each scaled to the panel's height: three on pine, two at 16:9. Each is let go from a slightly different angle, so they start out nearly in step and soon have nothing in common.

In colour: each pendulum's trail fades through its own family by age, blue, magenta or gold, from deep at the oldest end to bright at the bob. The rods are pale, the bobs white and the pivots grey.

Knobs: `ASCII_REST_TEXT_CELL_W` / `ASCII_REST_TEXT_CELL_H` and `ASCII_REST_TITLE`; see [full-screen twins](../ascii-rest.md#full-screen-twins).

Source: [`src/ascii_rest/double_pendulum.rs`](../../src/ascii_rest/double_pendulum.rs), the original's own module.
