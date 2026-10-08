# `podracer`

![`podracer`](../media/podracer.gif)

First-person Boonta Eve: two podracer engines hang ahead of you on their cables, flaring and yawing independently as you turn, while an ochre canyon rips past on both sides. One ray per cell column finds the wall; the floor and sky fall out of the ground-plane solve. Arches you fly through, rock spires, slot canyons barely wider than the pod, heat shimmer over the rim, and every so often a rival's engine wash crossing the view. Full repaint — it damages most of the panel every frame, because most of the panel is moving.

Source: [`src/podracer.rs`](../../src/podracer.rs).

## Knobs

- `PODRACER_CELL` (px, 4..32, default 8)
- `PODRACER_SPEED` (course m/s, 40..900, default 300)
- `PODRACER_FOV` (focal as a percent of panel width, 30..200, default 78)
- `PODRACER_WIDTH` (canyon half-width in metres, 6..90, default 30)
- `PODRACER_PINCH` (how far a slot closes, percent, 0..90, default 64)
- `PODRACER_SPREAD` (engine separation, percent of panel width, 10..90, default 46)
- `PODRACER_ENGINE` (engine radius, percent of panel width, 3..30, default 7, capped at 22% of the aspect-corrected height)
- `PODRACER_SHIMMER` (0..100, default 70)
- `PODRACER_FEATURES` (arches and spires alive at once, 0..24, default 7)
- `PODRACER_WASH_SECS` (mean seconds between a rival's wash, 0 = off..600, default 9)
- `PODRACER_SEED` (0 = roll one from the clock and pid; any other value reproduces the run exactly)
