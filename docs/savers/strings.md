# `strings`

![`strings`](../media/strings.gif)

"String Theory", the After Dark module — a polygon whose corners each bounce independently, redrawn every frame over the fading outlines behind it, so a ribbon of lines sweeps and folds. Three ribbons, one hue each.

`src/strings.rs` is After Dark's "String Theory": a polygon whose corners
each bounce around the panel on their own heading, redrawn every frame while the
outlines behind it fade, so the stack reads as one ribbon sweeping and folding
through space. Three independent ribbons, one hue each.

- **The trail is a fading heat buffer, not a ring of polygons.** The classic
  keeps N outlines and erases the oldest as it draws the newest, which needs the
  old vertices kept, the old lines re-walked to erase, and — because ribbons
  overlap — an erase that cannot simply write black. A per-cell brightness that
  decays does all three: a cell's age IS its colour, an overlap is the newer
  stamp winning, and nothing is re-walked.
- **`STRINGS_SPEED` is in SUB-CELLS per second, not a fraction of the panel**,
  and that is what makes this look right at 1920x1080 and at 1280x400. What
  separates the outlines — and so what makes a ribbon read as strings rather
  than as a solid sheet — is the per-frame step in sub-cells and nothing else.
  Scaled to the panel, the 1280x400 one has 2.7x fewer sub-cells down its short
  edge, gets 2.7x less separation, and fills in. The first cut of this did scale
  it, and rendered coloured sheets.
- **`STRINGS_FADE_MS` is the cost knob, and it runs the wrong way round.** A
  cell steps down eight brightness levels over its life, and a level step is a
  re-blit; a SHORTER fade means more steps per frame, not fewer. Measured
  against `lissajous` in the same process: 3.3x at the default 1000 ms, 2.8x at
  2000, 2.4x at 4000. It is 1000 anyway, because a 4000 ms ribbon saturates the
  400-tall panel.
- **The dots carry a freshness guard.** A cell whose heat is below "what a cell
  stamped last frame would have after this frame's decay" has its braille dots
  cleared rather than OR-ed into. Without it, a nearly-faded crossing keeps the
  old pass's dots, which are then redrawn in the new pass's colour lying ACROSS
  the new line — phantom ticks at every crossing. `lissajous` shipped that bug;
  `a_faded_pass_leaves_no_dots_in_a_cell_a_later_pass_relights` pins the fix.

Like `lissajous` it draws through `Grid::flush` and repaints most of the panel
every frame (median 960 of 1080 damaged scanlines at 1920x1080, 400 of 400 at
1280x400). That is not an oversight: the trail fades, so every lit cell changes
colour on the frame it steps down a level, and "what changed" is most of the
trail. Interleaved against the other savers on one machine it measures the same
per frame as `matrix`: 122 vs 116 us over 4,000 frames, 3.8x and 3.6x
`lissajous`. Those are ratios on a laptop and do not convert to milli-cores on
the Pi — `matrix` is the saver to compare it to there, not a number derived from
`lissajous`'s 87m.

Source: [`src/strings.rs`](../../src/strings.rs).

## Knobs

- `STRINGS_RIBBONS` (1..4, default 3)
- `STRINGS_VERTICES` (corners per polygon, 2..8, default 4)
- `STRINGS_FADE_MS` (200..20000, default 1000)
- `STRINGS_SPEED` (sub-cells/sec per corner, 5..1000, default 60)
- `STRINGS_CELL_W` / `STRINGS_CELL_H` (8, 16)
