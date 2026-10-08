# `life`

![`life`](../media/life.gif)

Conway's Game of Life on a toroidal board, cells coloured by age — white-hot at birth, cooling to blue — with a fading ash trail. A churn-triggered "meteor" of fresh soup keeps it from settling into still lifes.

Conway's Life is a bad screensaver by default: B3/S23 on a random soup burns
brightly for two hundred generations and then settles into blocks and blinkers
that never change again. Measured on this board at 240x135, churn falls from 49
changes per thousand cells per generation at generation 100 to 9 by generation
2000 and stays there for the next eighteen thousand — a torus keeps a few
gliders circulating, so the population never actually hits zero, which is why
"is anything alive" is not a test of anything.

The rules stay exact; the fix sits outside them. `step` already counts births
and deaths, and that one number catches all three ways a board gets boring —
still lifes churn zero, blinker fields churn a handful, and a nearly-black board
with one glider on it churns ten. A rolling board hash, the textbook stagnation
detector, sees the first two and is blind to the third. So when churn stays
under `LIFE_QUIET` per mille for `LIFE_PATIENCE` consecutive generations, one
disc of fresh soup lands at a random spot — the same `meteor` that seeds the
board at startup, so the panel never shows a transition it did not show at
second zero. At the defaults that settles into an equilibrium of 31 changes per
thousand cells per generation and 52 cells per thousand alive, flat from
generation 600 out to 20000 on both panel shapes, at about one meteor every ten
seconds. With the injector disabled the same measurement over eight seeded
boards reads 6 and 3 per mille; `it_never_dies_down` asserts a floor of 15.

Edges wrap. A dead border is a permanent absorber — every glider that reaches it
dies, and after an hour they all have.

Colour is age: white-hot at birth, cooling through amber and violet to a settled
blue after eight generations, with a dark-red ash that fades for `LIFE_FADE`
generations behind anything that dies. Both come free out of a pass that visits
every cell anyway, and they are what makes an age-saturated still life read as
debris rather than as part of the action.

`LIFE_GPS` paces generations independently of `SAVER_FPS` because Life at 30
generations a second is unreadable. A frame with no generation in it costs one
u32 compare per cell and reports no damage.

Source: [`src/life.rs`](../../src/life.rs).

## Knobs

- `LIFE_GPS` (generations per second, 1..60, default 10)
- `LIFE_DENSITY` (percent alive in fresh soup, 5..80, default 38)
- `LIFE_SEEDS` (startup soup discs, 1..64, default 12)
- `LIFE_QUIET` (churn per MILLE below which a generation is quiet, 1..500, default 30)
- `LIFE_PATIENCE` (quiet generations before a meteor, 1..600, default 12)
- `LIFE_FADE` (generations of ash, 0..6, default 5)
- `LIFE_CELL_W` / `LIFE_CELL_H` (4..64, 8 and 8)
- `LIFE_SEED` (0 = roll one from the clock and pid; any other value reproduces the board exactly — not to be confused with `LIFE_SEEDS`, which counts soup discs)
