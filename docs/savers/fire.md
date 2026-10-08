# Fire: `ascii` and `blocks`

Both draw the same Doom fire from `src/fire.rs`; they differ only in the brush.

## `ascii`

![`ascii`](../media/ascii.webp)

Doom fire as an ASCII ramp (`" .:-=+*#%@"`), one heat sample per character cell, coloured by the 37-step fire palette. The default.

### Knobs

- `FIRE_CELL` (px, 8..64, default 16)

## `blocks`

![`blocks`](../media/blocks.webp)

The same fire drawn as chunky pixels — a solid glyph per cell.

### Knobs

- `FIRE_SCALE` (px, 1..16, default 4)

Source: [`src/fire.rs`](../../src/fire.rs).
