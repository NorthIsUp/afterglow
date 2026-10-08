# `satori`

A slow colour-field composition: the panel is split into a handful of
rectangles, each holding one muted tone, all of it moving far slower than a
glance. Tones walk a closed ribbon of 24 hues, so every intermediate colour on
the way to a target is itself a colour the composition could have had, and the
split lines glide a cell at a time so the layout recomposes without a cut.

The scene changes on only a couple of frames in a hundred, and frames that
changed nothing skip the fill and the diff entirely.

Source: [`src/satori.rs`](../../src/satori.rs) — the module doc has the full
design notes.

## Knobs

- `SATORI_CELL_W` / `SATORI_CELL_H` (px, 4..64 / 4..64, default 16 / 16)
- `SATORI_FIELDS` (rectangles the panel is split into, 2..32, default 9)
- `SATORI_SEED` (seed for the composition and its moves, 1..4294967295, default
  `0x5A701234`; unlike most savers there is no clock-rolled default, so every
  start draws the same composition unless this is set)
- `SATORI_FADE_SEC` (seconds between each step a tone takes toward its target,
  1..60, default 2)
- `SATORI_TONE_SEC` (seconds between picking one field and giving it a new
  target tone, 1..600, default 12)
- `SATORI_GLIDE_SEC` (seconds per one-cell step of a moving split line, 1..60,
  default 3)
- `SATORI_DRIFT_SEC` (seconds between giving a split line a new target
  position, 1..3600, default 30)
