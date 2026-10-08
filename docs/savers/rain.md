# `rain`

![`rain`](../media/rain.webp)

Falling streaks over black, in three depth tiers, with wind and a splash on the
ground row. The tiers differ in speed, length and brightness together, so a far
streak reads as distance rather than as a dim near one.

Streaks are drawn in braille quarters rather than ASCII `|`, so a streak is
continuous, moves in 4px steps, and the wind shifts it half a cell sideways. The
ground row is reserved for splashes, so streaks and splashes never write the
same cells. Only the two nearer tiers splash; far rain lands behind the scene.

A frame rewrites about 1.5k of the panel's 16k cells — a bit under a third of a
full repaint, around 30-40m of a Pi 5 core at 1920x1080/15fps against matrix's
113m.

Source: [`src/rain.rs`](../../src/rain.rs) — the module doc has the full design
notes.

## Knobs

- `RAIN_CELL_W` / `RAIN_CELL_H` (px, 4..64 / 4..128, default 8 / 16; streaks
  move in quarter-cell steps, so the 4px step above is at the default height)
- `RAIN_DENSITY` (streaks per 1000 cells, 0..200, default 11 — about 180 streaks
  at 1920x1080)
- `RAIN_SPEED` (rows per second for the nearest tier; the others are a
  percentage of it, 1..1000, default 50)
- `RAIN_WIND` (cells sideways per 100 cells of fall, signed, -300..300, default 34)
- `RAIN_SPLASH_MS` (splash lifetime per stage, ms, 0..2000, default 160; 0 =
  off)
