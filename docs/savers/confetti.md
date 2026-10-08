# `confetti`

![`confetti`](../media/confetti.webp)

Pieces flutter down, land, and pile up like sand. Confetti falls in a gust band
that sweeps across the panel, so a heap builds under it and topples outward once
the gust has moved on. Between three and seven ledges stand partway up the panel
and collect piles of their own.

The pile is a height map in sub-cell grain units, 25 to the cell by default, so
the 0.48 rise/run angle of repose is something the relaxation can actually land
on. Each frame a few relaxation sweeps move half of any too-steep difference
downhill, so a fresh spike visibly avalanches over several frames; grains are
moved, never created or destroyed.

When the heap fills, the spawner stops and grains erode off random column tops
until it is half drained, then it fills again — a cap would freeze the picture
and a reset would blink. The pile's slope is measured against the square-pixel
cell height, so the incline stays 48% on the glass under
[SAVER_PIXEL_ASPECT](../pixel-aspect.md).

Source: [`src/confetti.rs`](../../src/confetti.rs) — the module doc has the full
design notes.

## Knobs

- `CONFETTI_CELL` (cell width and height, px, 4..64, default 12)
- `CONFETTI_GRAINS` (grain units per cell of pile height, 4..64, default 25)
- `CONFETTI_SLOPE` (angle of repose, rise/run in percent, 5..200, default 48)
- `CONFETTI_DEPOSIT` (grain units a landed piece adds, 1..64, default 12; trades
  airborne density against how fast the heap fills the panel)
- `CONFETTI_MAX` (most pieces in the air at once, 1..4000, default 600)
- `CONFETTI_RATE` (pieces spawned per second, 1..400, default 90)
- `CONFETTI_FALL` (fall speed, px per second, 10..2000, default 260; each piece
  gets 75-125% of it)
- `CONFETTI_SWAY` (peak side-to-side flutter speed, px per second, 0..400,
  default 110; each piece gets 30-100% of it)
- `CONFETTI_FULL_PCT` (heap fill, percent of the panel's grain capacity, at
  which spawning stops and draining starts; draining stops at half this, 5..95,
  default 35)
- `CONFETTI_DRAIN` (grain units eroded per second while draining, 10..100000,
  default 4000)
- `CONFETTI_RELAX` (relaxation sweeps per frame, 1..64, default 8)
- `CONFETTI_GUST_PCT` (width of the band pieces spawn in, percent of panel
  width, 1..100, default 40; 100 is uniform rain)
- `CONFETTI_GUST_SECS` (seconds for the band to sweep the panel once, 2..600,
  default 37)
