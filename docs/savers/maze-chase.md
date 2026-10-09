# `maze-chase`

![`maze-chase`](../media/maze-chase.gif)

A Pac-Man-style maze chase on autopilot. An eater clears a maze of pellets
while four ghosts hunt it with the classic personalities: one aims at the
eater, one four tiles ahead of it, one at the far end of a line through the
first ghost and two tiles ahead, and one gives up and retreats to its corner
when it gets close. They alternate scatter and chase on the arcade's
schedule, turn blue and slow after a power pellet, go home as eyes when eaten
(200, 400, 800, 1600), and choose at junctions the way the arcade's do. The
eater dies in a collapsing wedge and the walls flash when a level is cleared.
An extra life comes every `MAZE_BONUS` points.

The maze fills the panel: the tile size comes from the panel's short side
and the maze is as many tiles as fit, about ninety by thirty on pine's 3.2:1
glass. Every level and every new game generates a new one: a corridor lattice
over the left half, cut at random without leaving a dead end or an island, then
mirrored, with a ghost house in the middle and a wrap-around tunnel. Walls are
outlined with rounded box-drawing glyphs, so the border and the house come out
double-lined like the arcade's. Actors are 6x6 bitmaps in half-cell quadrants.

The autopilot scores each way out of a junction by the territory it leads to:
the tiles it reaches before any ghost that is near enough to matter, counting
that a ghost cannot turn back. With room to spare it hunts blue ghosts it can
catch in time, saves power pellets for a ghost on its tail, and otherwise takes
the nearest pellet. Into a pincer there is no room, so it takes the roomiest
way or a power pellet. It grows bolder the longer it goes without eating, and
turns back between tiles when a ghost is about to meet it head on. In tests on
the 16:9 maze it clears about twelve levels an hour.

Source: [`src/maze.rs`](../../src/maze.rs),
[`src/maze/gen.rs`](../../src/maze/gen.rs) and
[`src/maze/play.rs`](../../src/maze/play.rs).

## Knobs

- `MAZE_TILES` (tiles up the panel's short side, 15..80, default 28)
- `MAZE_SPEED` (full speed, tenths of a tile a second, 30..300, default 95)
- `MAZE_CUT` (per cent of the lattice cut away, 0..80, default 45)
- `MAZE_LIVES` (1..9, default 3)
- `MAZE_BONUS` (points per extra life, 0..1000000, default 10000; 0 is none)
- `MAZE_SEED` (0 rolls a new one each build)
