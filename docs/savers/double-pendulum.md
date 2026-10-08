# `double-pendulum`

![`double-pendulum`](../media/double-pendulum.gif)

Two equal rods hung end to end from one pivot, stepped with RK4. Chaotic, so it never repeats; the lower bob leaves a fading trail.

One of ascii.rest's character pieces, drawn at whatever size and shape the panel is. See [the ascii.rest ports](../ascii-rest.md#character-pieces).

## At any size

The panel is shared out into slots of upstream's shape, and a pendulum hangs in each, as large as its slot allows without either rod clipping. A wide panel gets them side by side: three on pine, two at 16:9 and at 4:3, where one would leave wide bare margins. A square panel gets one. A tall panel stacks them: two, one above the other, in portrait. Each is let go from a slightly different angle, so they start out nearly in step and soon have nothing in common.

In colour: each pendulum's trail fades through its own family by age, blue, magenta or gold, from deep at the oldest end to bright at the bob. The rods are pale, the bobs white and the pivots grey.

## Knobs

- `DOUBLE_PENDULUM_COLOR` (0..1, default 1): 0 draws it in upstream's one cyan ink on black.
- `ASCII_REST_TEXT_CELL_W` / `ASCII_REST_TEXT_CELL_H` and `ASCII_REST_TITLE`; see [character pieces](../ascii-rest.md#character-pieces).

With `DOUBLE_PENDULUM_COLOR` at 0, on upstream's 48x25 grid, it is upstream's picture cell for cell.

Source: [`src/ascii_rest/double_pendulum.rs`](../../src/ascii_rest/double_pendulum.rs).
