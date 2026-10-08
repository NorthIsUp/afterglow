# `tv-static`

![`tv-static`](../media/tv-static.gif)

An old set showing snow, with a hum bar rolling through it. The dial clicks over, a test card rolls into place and holds, then is lost.

One of ascii.rest's character pieces. By default the set carries the wide twin's colours (below): the test card's bars in their real colours, a grey cabinet, an amber dial.

Knobs: `TV_STATIC_COLOR` (0..1, default 1) — 0 is upstream's one ink, cell for cell. See [the ascii.rest ports](../ascii-rest.md#character-pieces).

Source: [`src/ascii_rest/tv_static.rs`](../../src/ascii_rest/tv_static.rs).

## `tv-static-wide`

![`tv-static-wide`](../media/tv-static-wide.gif)

The panel is the screen. The tube's rounded corners meet a thin bezel at the panel's edges, and the snow, the hum bar, the rolling and tearing, and the test card fill everything inside it. The channel dial clicks in a small knob set into the bezel's foot. A 4:3 set on a 3.2:1 panel would leave as much black beside it as the original does, and a room around the set would shrink the snow to a third of the glass.

In colour: the snow stays the set's blue-white, as real snow is, while the test card's bars come through in their real colours, white, yellow, cyan, green, magenta, red and blue, with the reversed strip beneath them. The card shows through the snow as it tunes in and rolls. The bezel is grey and the dial amber.

Knobs: `ASCII_REST_TEXT_CELL_W` / `ASCII_REST_TEXT_CELL_H` and `ASCII_REST_TITLE`; see [full-screen twins](../ascii-rest.md#full-screen-twins).

Source: [`src/ascii_rest/tv_static.rs`](../../src/ascii_rest/tv_static.rs), the original's own module.
