# `fractal`

Escape-time fractals: nineteen families in rotation, each zooming continuously
into a point known to sit on its boundary. Each cell is one sample drawn solid,
so the fractal reads as chunky pixel art. Between families the image dips
through dimmer palette tiers into near-darkness and cuts at the dim tier, rather
than crossfading.

Sampling per cell rather than per pixel is what makes it affordable: a 20px
cell is 96x54 = 5,184 orbits, 400x fewer than one per pixel at 1920x1080. The
four-family version was measured on the Pi 5 at 125.7 milli-cores (range
111-153m); the 19-family version has not been measured on the Pi.

Iteration depth rises with zoom depth but is capped, and past the cap the panel
goes flat, so the total zoom of a cycle is clamped to 16 octaves whatever
`FRACTAL_SECONDS` and `FRACTAL_ZOOM_PCT` ask for.

Source: [`src/fractal.rs`](../../src/fractal.rs) — the module doc has the full
design notes.

## Knobs

- `FRACTAL_CELL` (cell width and height, px, 4..64, default 20; orbits scale
  with its square)
- `FRACTAL_SECONDS` (length of one family's cycle, seconds, 5..45, default 26)
- `FRACTAL_FADE_MS` (dip to dark around each cut, ms, 0..5000, default 900;
  capped at 40% of the cycle)
- `FRACTAL_ZOOM_PCT` (percent of the view width the zoom eats each second,
  1..90, default 22; held so a cycle zooms at most 16 octaves)
- `FRACTAL_JULIA_DRIFT` (how fast the Julia constant's argument turns,
  milli-radians per second, 0..2000, default 90)
- `FRACTAL_ITER` (iteration budget at the start of a zoom, 16..512, default 40;
  rises by 8 per octave of zoom)
- `FRACTAL_ITER_MAX` (cap on that budget, 32..2000, default 110)
