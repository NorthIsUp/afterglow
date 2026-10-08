# `tv-static`

![`tv-static`](../media/tv-static.gif)

An old set showing snow, with a hum bar rolling through it. The dial clicks over, a test card rolls into place and holds, then is lost.

One of ascii.rest's character pieces, drawn at whatever size and shape the panel is. See [the ascii.rest ports](../ascii-rest.md#character-pieces).

## At any size

By default the panel is the screen. The tube's rounded corners meet a thin bezel at the panel's edges, and the snow, the hum bar, the rolling and tearing, and the test card fill everything inside it, whatever its shape: the card's bars, circle and crosshair are drawn for the screen's own width and height. The channel dial clicks in a small knob set into the bezel's foot.

With `TV_STATIC_SET=1` it is the old set instead: upstream's cabinet, rounded screen, channel and volume knobs, speaker grille, legs and antenna, scaled to the largest set of its shape the panel holds and centred on black. On pine's 3.2:1 glass and at 16:9 it fills the height with black either side; on a square or portrait panel it fills the width with black above and below. The snow, the hum bar and the test card play inside its screen. A panel too small for a legible set gets the full-screen form.

In colour: the snow stays the set's blue-white, as real snow is, while the test card's bars come through in their real colours, white, yellow, cyan, green, magenta, red and blue, with the reversed strip beneath them. The card shows through the snow as it tunes in and rolls. The cabinet or bezel is grey and the dial amber.

## Knobs

- `TV_STATIC_SET` (0..1, default 0): 1 draws the old set, scaled to fit, rather than making the panel the screen.
- `TV_STATIC_COLOR` (0..1, default 1): 0 draws it in upstream's one blue-white ink on black.
- `ASCII_REST_TEXT_CELL_W` / `ASCII_REST_TEXT_CELL_H` and `ASCII_REST_TITLE`; see [character pieces](../ascii-rest.md#character-pieces).

With `TV_STATIC_SET` at 1 and `TV_STATIC_COLOR` at 0, on upstream's 58x26 grid, it is upstream's picture cell for cell.

Source: [`src/ascii_rest/tv_static.rs`](../../src/ascii_rest/tv_static.rs).
