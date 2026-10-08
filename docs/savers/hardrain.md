# `hardrain`

![`hardrain`](../media/hardrain.webp)

A downpour: steeply slanted streaks under a gusting wind, a mist the sky is veiled in, squalls sweeping across, and standing water at the bottom that ripples where the rain lands. The storm to `rain`'s drizzle.

`src/hardrain.rs` is the storm; `src/rain.rs` is the drizzle. They
share the braille sub-cell trick and nothing else, because two savers that are
hard to tell apart in a rotation are one saver that shows up twice.

- **The wind is a variable.** `rain` has one constant lean. This has a base
  slant nearly three times as steep, plus a gust — two sines that do not share
  a period — swinging it by most of the base again over an eleven-second cycle.
  A streak is re-derived from its head every frame, so a gust re-leans the rain
  that is already falling, not only what spawns next.
- **Four depth tiers, and the furthest is a MIST.** One-to-two sub-rows of
  speck, six-sixteenths of the draw, barely above black: the sky is veiled
  rather than empty. `rain` has three tiers and black between them.
- **Squalls.** About every seventeen seconds a band a third of the panel wide
  sweeps downwind across it. Inside the band every streak is drawn a whole
  depth tier brighter, two and a half times as long, and half of what respawns
  respawns into the band — so it is genuinely more water and not only brighter
  water. Measured at 4-5x the luminance of the sky beside it; a brightness-only
  version measured 1.2x against 1.3x frame-to-frame noise and was invisible.
- **Water, not a splash row.** `rain` reserves the bottom grid row for a
  two-stage ripple. This has a pool 9% of the panel deep carrying a damped 1D
  wave: an impact digs a dip, the dip runs out both ways and reflects off the
  edges, a moving crest catches foam, and near rain throws spray that arcs up
  and falls back. The impulse is spread over three columns on purpose — a
  single-column spike propagates as a spike and the surface renders as a picket
  fence of 8px teeth.
- **Five times the rain.** 55 drops per thousand cells against `rain`'s 11, and
  a fall that crosses the panel in a little under a second.

**Nothing here is measured in rows.** Speed is hundredths of a PANEL HEIGHT per
second, streak length is a percent of panel height, and the pool is a percent of
rows — so the same knobs read the same on 1920x1080 (67 rows) and on the
1280x400 panel (25 rows), where a streak sized in cells would be a quarter of
the screen tall. The steep default slant is also what makes a streak read
ACROSS a panel that wide. Both shapes are rendered by every test in the module.

It is a FULL REPAINT every frame (`Grid::flush`), where `rain` is sparse
(`flush_sparse`). A hand-maintained dirty list can under-report and freeze a
region on the panel forever; a diff cannot, and nothing here is sparse anyway —
the mist, the squall and the pool all touch broad regions every frame. Measured
interleaved at 1920x1080: `hardrain` is 4.8x `rain`, 2.3x `matrix` and 1.0x
`moire` per frame, which puts it in `moire`'s class rather than `rain`'s.

Source: [`src/hardrain.rs`](../../src/hardrain.rs).

## Knobs

- `HARDRAIN_DENSITY` (per 1000 cells, 0..400, default 55)
- `HARDRAIN_SPEED` (hundredths of a panel height per second, 10..1000, default 110)
- `HARDRAIN_WIND` (cells sideways per 100 of fall, -300..300, default 95)
- `HARDRAIN_GUST` (same units, 0..300, default 70)
- `HARDRAIN_GUST_SECS` (1..600, default 11)
- `HARDRAIN_POOL_PCT` (water depth as a percent of rows, 0..40, default 9; 0 = off)
- `HARDRAIN_SQUALL_SECS` (mean seconds between squalls, 0..600, default 17; 0 = off)
- `HARDRAIN_SPRAY` (droplets per impact, 0..4, default 2)
- `HARDRAIN_CELL_W` / `HARDRAIN_CELL_H` (8, 16)
