# `tactiles`

![`tactiles`](../media/tactiles.gif)

After Dark's TacTiles — a grid of square tiles carrying one geometric glyph each (bar, diagonal, corner, arc). Shape, rotation and colour are three travelling sine waves quantised into bands, so the tiling rearranges itself continuously without ever looking rolled.

`src/tactiles.rs` reports the WHOLE panel as damaged every frame, and
that is not a bug (compare the damaged-scanline table on the
[toasters page](toasters.md#toasters3)). The three waves cross the entire tile
grid, so every 48px band of scanlines has some tile flipping in it and the runs
merge into one. What stays small is the blit: about 1% of cells a frame, since
`Grid::flush` skips every cell whose packed `Cell` did not change. Measured
interleaved against `moire` at 1920x1080, it costs 0.76x `moire` per frame.

The glyphs are baked once in the constructor — sixteen shape-by-rotation
bitmaps rasterised into braille cells — so the frame loop is four array reads
and a multiply per cell, with the trig per TILE rather than per cell.

Source: [`src/tactiles.rs`](../../src/tactiles.rs).

## Knobs

- `TACTILES_CELL_W` / `TACTILES_CELL_H` (px, 4..32, default 8 each)
- `TACTILES_TILE` (tile side in CELLS, 2..24, default 6 — so a 48px tile)
- `TACTILES_SPEED` (wave travel, milli-rad/sec, 10..5000, default 700)
- `TACTILES_SCALE` (wave spatial frequency, milli-rad per tile, 10..3000, default 430)
- `TACTILES_STROKE` (glyph stroke as a percent of the tile, 5..50, default 24)
- `TACTILES_SEED` (0 = roll one from the clock and pid; any other value reproduces the pattern exactly)
