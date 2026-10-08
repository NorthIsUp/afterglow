# `worms`

![`worms`](../media/worms.webp)

Segmented crawlers wandering a toroidal grid, head bright, body trailing behind
it in alternating light and dark bands. Each worm turns by a damped random walk
on its angular velocity rather than its heading, so it commits to a curve and
comes out of it instead of jittering.

Moving a worm touches three cells whatever its length, and `saver::frame`
measures about 2us at 1920x1080/15fps. The real cost is damage: a dozen worms
are a dozen short scanline runs, a measured median of 384 of the panel's 1072
covered rows at 15fps, so `WORMS_COUNT` is the knob that moves it.

Source: [`src/worms.rs`](../../src/worms.rs) — the module doc has the full
design notes.

## Knobs

- `WORMS_CELL_W` / `WORMS_CELL_H` (px, 4..64 / 4..128, default 16 / 16)
- `WORMS_COUNT` (worms, 1..64, default 12)
- `WORMS_LEN` (body length, cells, 2..400, default 44)
- `WORMS_SPEED` (cells per second, 1..200, default 14)
- `WORMS_TURN` (fastest a worm can turn, degrees per second, 1..2000, default 150)
- `WORMS_BAND` (segments per light or dark body band, 1..40, default 3)
