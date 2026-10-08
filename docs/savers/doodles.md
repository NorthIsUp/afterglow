# `doodles`

![`doodles`](../media/doodles.gif)

After Dark's scribbler: pens wander the panel leaving one continuous freehand line each, looping back over themselves until the sheet is full, then it fades and a new one starts. Each pen's hue sweeps across the life of a doodle, so the scribble shows its own history.

The point is that it reads as hand-drawn rather than mathematical, which is the
whole difference from `lissajous`. Lissajous evaluates a closed-form curve —
position is a function of t, the figure is periodic, and it looks like
mathematics because it is. A doodle pen carries a heading and a turn rate, and
only the TURN RATE is driven, by a damped random walk. Position is the double
integral of noise: the line commits to an arc, curls out of it and wanders off,
and no frame of it can be reproduced from a formula.

Two knobs bracket the degenerate ends, and `DOODLES_INERTIA` is the one that
matters: too little and the heading is uncorrelated between steps, which is a
fuzzy blob rather than a line; too much and the turn rate never changes sign,
which is a circle retracing itself forever. The default 960 puts the turn rate's
correlation at about 25 steps and the typical radius at ~17 sub-cells, so
curvature persists for roughly one loop's arc.

`DOODLES_SPEED` is quoted for a 1080p panel and is scaled down with panel AREA
(to a floor of a third). A doodle lasts as long as the pen needs to fill the
sheet, so without that the same hand speed finishes a 1280x400 panel four times
sooner — measured 5s against 19s. With it, a doodle is ~16s at 1280x400 and
~20s at 1920x1080.

Source: [`src/doodles.rs`](../../src/doodles.rs).

## Knobs

- `DOODLES_PENS` (1..4, default 3)
- `DOODLES_SPEED` (pen steps/sec at 1080p, 30..8000, default 420)
- `DOODLES_INERTIA` (per-mille of the turn rate carried to the next step, 500..999, default 960)
- `DOODLES_WANDER` (milli-rad of turn-rate noise per step, 1..300, default 20)
- `DOODLES_CURL` (milli-rad/step, tightest curl, 5..500, default 90)
- `DOODLES_FILL_PCT` (1..90, default 28)
- `DOODLES_MAX_S` (5..900, default 90)
- `DOODLES_FADE_MS` (200..10000, default 1600)
- `DOODLES_CELL_W` / `DOODLES_CELL_H` (8, 16)
- `DOODLES_SEED` (0 = roll one from the clock and pid; any other value reproduces the doodle exactly)
