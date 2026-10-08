# `warp`

Flying forward through a starfield. Points stream out of a vanishing point at
the centre, accelerating and brightening as they pass the camera.

Nothing moves in screen space: each star holds a fixed position in the tunnel's
cross-section and a depth that only decreases, and the outward acceleration, the
lengthening streaks and the sparse centre all fall out of the perspective
divide. A streak is the segment between where the star was last frame and where
it is now — motion blur, not a decay trail — so nothing fades.

Stars are drawn into braille, 2x4 dots per cell, so at the default 8x8 cell a
star is a 4x2 pixel dot and streaks stay thin. The projection is in panel
pixels rather than cells, so `warp` applies
[SAVER_PIXEL_ASPECT](../pixel-aspect.md) to its vertical focal length itself;
otherwise a round tunnel would arrive on a stretched panel as a wide ellipse.

Source: [`src/warp.rs`](../../src/warp.rs) — the module doc has the full design
notes.

## Knobs

- `WARP_CELL` (cell width and height, px, 4..32, default 8)
- `WARP_STARS` (stars in the field, 16..8000, default 650)
- `WARP_SPEED` (flight speed, depth units per second over a 2..1000 tunnel,
  10..5000, default 420)
- `WARP_SPREAD` (half-width in px of the tunnel rim at depth 100 — how fast the
  field opens out, 64..8000, default 700)
- `WARP_STREAK` (most dots one streak is drawn with; a longer streak is drawn
  sparser, 1..2048, default 256)
