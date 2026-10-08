# `hypercube`

![`hypercube`](../media/hypercube.webp)

A rotating, inverting tesseract wireframe. The 4D perspective divide renders the
tesseract's two cubes as one cube inside another, and any rotation that touches
W makes them swap places — the inner cube swells through the outer one and
becomes it. That turning inside out is the saver.

A tesseract turned only in XY, XZ and YZ is a spinning cube in a box, so if
every W-plane rate is set to 0 the saver logs a warning and turns ZW at 29
anyway. The default rates are pairwise coprime, so the figure never visibly
returns to a pose it held a minute ago; XW is the fastest and turns the figure
inside out every 21 seconds.

Edges are drawn in braille, 2x4 dots per cell, and depth-cued along their
length from W and Z; where edges share a cell the nearer shade wins.

Source: [`src/hypercube.rs`](../../src/hypercube.rs) — the module doc has the
full design notes.

## Knobs

- `HYPERCUBE_STYLE` (`braille` draws lines in 2x4 dots, `ascii` a text ramp by
  cell coverage, `block` whole lit cells; anything else is `braille`; default
  `braille`)
- `HYPERCUBE_CELL_W` / `HYPERCUBE_CELL_H` (px, 4..64 / 8..128, default 8 / 16)
- `HYPERCUBE_RATE_XY` / `HYPERCUBE_RATE_XZ` / `HYPERCUBE_RATE_YZ` /
  `HYPERCUBE_RATE_XW` / `HYPERCUBE_RATE_YW` / `HYPERCUBE_RATE_ZW` (rotation in
  each plane, milli-revolutions per second, signed, -2000..2000, default 0 / 13
  / 29 / 47 / 0 / 0; at least one W plane is forced non-zero)
- `HYPERCUBE_W_DIST` (4D camera distance, thousandths of the tesseract's
  half-edge, 1200..20000, default 2400; smaller swells the inner cube harder)
- `HYPERCUBE_Z_DIST` (3D camera distance, same units, 2000..40000, default 6000)
- `HYPERCUBE_SCALE` (figure size, thousandths of the shorter side of the dot
  grid, 20..400, default 130)
