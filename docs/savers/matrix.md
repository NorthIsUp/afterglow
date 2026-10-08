# `matrix`

![`matrix`](../media/matrix.gif)

Digital rain.

It copies the _Reloaded/Revolutions_ look, not the literal 1999 one: the first
film's on-screen code is flat-brightness with only the cursor lit, which on a
glyph grid reads as a rendering bug. Three details are what separate it from the
usual imitation, and all three are in `src/matrix.rs`:

- **The glyphs are mirrored left to right.** The production designer drew them
  back to front, "as if we were in the code looking at a screen of code from the
  inside". The mirroring is baked into the glyph table, per glyph — katakana
  reversed, digits not.
- **The glyphs never move.** The grid is stationary; what falls is a wave of
  illumination over it.
- **Every column is always raining.** The motion is a closed-form sawtooth with
  two random floats per column, so several drops share a column at different
  speeds and no column is ever idle. Discrete drops with black gaps are the
  giveaway most implementations ship.

Source: [`src/matrix.rs`](../../src/matrix.rs).

## Knobs

- `MATRIX_CELL_W` (8..64, default 16)
- `MATRIX_CELL_H` (8..128, default 32)
