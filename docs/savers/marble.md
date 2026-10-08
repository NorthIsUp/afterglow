# `marble`

![`marble`](../media/marble.webp)

Marble Madness — an isometric course floating in black space, a chrome marble rolling down it on autopilot, and hazards trying to stop it. Ramps, narrow catwalks over nothing, acid pools, a hammer and a leashed black hunter marble. Falling off costs a respawn at the last checkpoint; reaching the goal generates a new course.

`src/marble.rs` is Atari's Marble Madness, not a marble run: an isometric
course seen from a fixed three-quarter view, a marble worked down it by a very
simple autopilot, and the void underneath everything.

**The projection is the whole look, and it is computed in GLASS units.** A tile
is `gx = (x - y) * tw`, `gy = (x + y) * tw/2 - z * zs` — a diamond exactly twice
as wide as it is tall. That 2:1 has to hold **on the panel**, not in the
framebuffer, and `SAVER_PIXEL_ASPECT=180` is precisely the difference between
the two. So there is one conversion from glass to sub-cells:

```text
ux = cell_w / 2                       px per sub-cell across
uy = cell_h / 4 * 100 / pixel_aspect  px per sub-cell down, un-stretched
```

`uy` divides the aspect back out — at 100 it is a no-op, at 180 it is the only
thing keeping the diamonds 2:1. `diamonds_are_two_to_one_on_the_glass_at_both_aspects`
measures the drawn tile, in glass units, at both aspects; it does not assert the
constants, because the constants are right in both worlds and the bug is not.

Tiles are painted back to front by increasing `x + y`, each a top diamond plus a
skirt down its two lower edges to whatever the neighbour's height is. The skirt
is what turns a heightfield into cliffs and catwalks instead of a flat mosaic.
The camera follows the marble, so most frames move every tile on the panel and
the damage model is a full repaint through `Grid::flush`.

**Courses are generated, then validated, then rejected.** A route of descending
straight segments is carved first — decks one to five tiles wide, some walled,
some slick and unwalled — and the geometry is built around it. Five criteria:
a flood fill from the start that may drop any distance but never climb more than
`CLIMB` must reach the goal (acid counts as solid, so the dry line past a pool
has to exist); every hazard's tile must border that reachable set; the route's
bounding box must span at least 11 tiles in both axes and descend at least 9
height units; the deck must be between 60 tiles and a third of the field; and
then the candidate is handed to a **physics probe** that runs the same step and
the same autopilot, hazards off, and must reach the goal. Measured over 400
candidates at 1080p: 359 pass the first four, 316 of those pass the probe —
about 1.3 candidates per accepted course, with the goal-unreachable check and
the probe doing essentially all the rejecting. Fourteen candidates in, the last
one ships anyway: a headless pod must never stall the frame loop over taste.

**Progress, not churn, is the stagnation measure.** `advance` already knows the
furthest waypoint reached. A fall or a hazard respawns the marble at the last
checkpoint; `MARBLE_PATIENCE` steps with no progress at all respawns it with a
shove; and a respawn that fails to beat the previous one four times running
blames the course and regenerates it. Everything that respawns routes through
one function, so there is one place to get that ladder right. Measured over
60 000 steps with the defaults: about 133 goals, 141 falls, 104 hazard deaths
and 19 no-progress respawns, of which roughly 15 escalate to a new course —
that is one fall per goal, which is what "someone playing reasonably well and
occasionally losing it" measures out as.

Two numbers in there were found the hard way. A hunter marble that chases
without a leash follows you the length of the course and shoves you off the same
catwalk forever: 40 falls per goal, measured. And a checkpoint beside an acid
pool is an infinite death loop — 1 992 deaths in 60 000 steps on one seed —
which is why a respawn buys 45 steps of grace.

Tunnelling is swept, not capped by inspection: a step is split into
`ceil(speed / 0.22)` substeps so nothing moves more than 0.22 of a tile at a
time, against a one-tile catwalk, and the per-step speed is clamped to the
substep budget independently of `MARBLE_SPEED` and of `SAVER_FPS`. Walls block
by tile KIND as well as by height, because `height` deliberately refuses to
blend into a wall — without that the bilinear smoothing builds a ramp up the
side of the thing that is there to stop you, and a fast marble simply drives
over it.

Cost: 342 us/frame at 1920x1080 with `SAVER_PIXEL_ASPECT=180`, against `moire`
at 277 in the same process — 1.23x the most expensive saver measured, which
puts it around 170m of the 500m limit. Most of that is the fill: the camera
moves, so every visible tile is redrawn every frame.

Source: [`src/marble.rs`](../../src/marble.rs).

## Knobs

- `MARBLE_TILE` (tile width in GLASS px, 0 = derive from the panel, else 16..160)
- `MARBLE_SPEED` (milli-tiles/sec, 500..20000, default 4200)
- `MARBLE_STEER` (autopilot thrust, milli-tiles/sec², 100..40000, default 5200 — how well the invisible player plays)
- `MARBLE_PATIENCE` (steps with no route progress before a respawn, 30..4000, default 260)
- `MARBLE_COURSE_S` (10..3600, default 150)
- `MARBLE_HAZARDS` (0..6, default 3)
- `MARBLE_CELL_W` / `MARBLE_CELL_H` (4..32 / 4..64, 8 and 8)
- `MARBLE_SEED` (0 = roll one from the clock and pid; any other value reproduces the course exactly)
