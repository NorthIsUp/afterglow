# `dvd`

![`dvd`](../media/dvd.gif)

The bouncing DVD logo, and the corner hit it exists for. When the logo lands
exactly in a corner it stops dead and strobes through the palette for
`DVD_CORNER_MS` — the only time it is ever still.

Both axes advance by a whole number of pixels per frame and the travel range is
trimmed to a multiple of that step, so the logo moves on a finite lattice and a
corner is guaranteed to be reachable rather than approached and missed forever.
Nothing else is fudged toward the corner; at 1920x1080 / 15 fps the wait is a
few minutes.

The logo is cell art, so the cell is the pixel size of the wordmark: the default
24x24 cell puts it at 480x144 on a 1080p panel. The bounce arithmetic reads the
cell height back from the grid, so it stays exact under
[SAVER_PIXEL_ASPECT](../pixel-aspect.md).

Source: [`src/dvd.rs`](../../src/dvd.rs) — the module doc has the full design
notes.

## Knobs

- `DVD_CELL_W` / `DVD_CELL_H` (px, 4..64 / 4..128, default 24 / 24)
- `DVD_SPEED` (px per second, rounded to a whole px per frame, 8..2000, default 180)
- `DVD_CORNER_MS` (how long the logo holds and strobes after a corner hit, ms,
  0..10000, default 1500)
