# `moire`

Moire interference from overlapping line families. Two gratings — straight,
concentric or radial — drift and rotate against each other, and what you are
meant to see is the beat: fringes that sweep across the panel as the gratings
turn. A cell's colour is its overlap depth, so one grating draws a mid blue and
two crossing draw near-white.

This saver is not cheap. Measured against matrix on the same machine, the
default `sc` costs 2.1x matrix per frame and `ss` 1.4x; `MOIRE_KINDS=r` is
roughly five straight families, which is why the default has no radial. It is a
full repaint every frame.

The cell-size floors are a CPU limit: on the Pi the default 8x32 is ~255m
against the pod's 500m limit and 8x24 is ~330m. Geometry is in panel pixels
rather than cells, so `moire` applies [SAVER_PIXEL_ASPECT](../pixel-aspect.md)
itself to keep rings circular on a stretched panel.

Source: [`src/moire.rs`](../../src/moire.rs) — the module doc has the full
design notes.

## Knobs

- `MOIRE_KINDS` (one letter per grating: `s` straight, `c` concentric, `r`
  radial; up to four used, other characters ignored, default `sc`; nothing valid
  falls back to `sc`)
- `MOIRE_CELL_W` / `MOIRE_CELL_H` (px, 8..64 / 24..128, default 8 / 32)
- `MOIRE_SPACING` (line pitch of the first grating, px, 6..400, default 64;
  each further grating is 11% wider so they beat)
- `MOIRE_DUTY` (percent of each period that is ink, 2..90, default 22)
- `MOIRE_SPIN` (rotation, millidegrees per second, 0..60000, default 400;
  alternate gratings turn opposite ways)
- `MOIRE_DRIFT` (sideways slide of the lines, px per second, 0..600, default 9)
