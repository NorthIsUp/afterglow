# `sakura`

![`sakura`](../media/sakura.gif)

A cherry tree at night, shedding blossom on a slow wind. The tree is grown from
a seed that changes every time the pod starts, and it stands in one of three
places: beside a pond, on a mountain spur, or in a rock garden. One start in
three grows it windswept instead of upright — the bonsai fukinagashi, laid over
by a wind that never stops.

The tree and its setting are rasterised once and never redrawn; after that only
the cells petals occupy are touched, so per-frame work scales with petals, not
cells. The wind is one field sampled per petal, so a gust reaches every petal at
once and the fall reads as weather rather than noise.

Night, not daylight: the panel's unlit pixels are black, so every lit thing is a
silhouette against it. Blossom runs `#FFD9E8` to `#9E5070`, the trunk is
`#584437`, and the pond reflection is the same hues at roughly 40% luminance.

Source: [`src/sakura.rs`](../../src/sakura.rs) — the module doc has the full
design notes.

## Knobs

- `SAKURA_SEED` (0 = roll one from the clock and pid; any other value reproduces
  the tree, setting and style exactly)
- `SAKURA_SCENE` (`pond`, `mountain` or `garden` / `rock`; anything else,
  including unset, rolls one)
- `SAKURA_TREE` (`windswept` / `swept` / `bonsai` or `upright` / `straight`;
  anything else, including unset, rolls windswept one start in three)
- `SAKURA_CELL_W` / `SAKURA_CELL_H` (px, 4..64 / 4..128, default 12 / 16)
- `SAKURA_PETALS` (petals in play, 0..4000, default 150)
- `SAKURA_FALL` (fall speed, hundredths of a cell-row per second, 10..20000,
  default 380)
- `SAKURA_WIND_BASE` (steady wind, hundredths of a cell-column per second,
  signed, -20000..20000, default 180)
- `SAKURA_WIND` (how far a gust swings either side of the base wind, same units,
  0..20000, default 900)
- `SAKURA_GUST_SECS` (gust cycle, tenths of a second, 5..6000, default 110)
- `SAKURA_FLUTTER` (petal sway amplitude, hundredths of a cell, 0..10000,
  default 95)
- `SAKURA_FLUTTER_RATE` (sway frequency, tenths of a cycle per second, 1..600,
  default 9)
- `SAKURA_TUMBLE_MS` (one full tumble of a petal, ms, 60..60000, default 1300)
- `SAKURA_REST_SECS` (how long a landed petal lies before it is recycled to the
  crown, seconds, 1..3600, default 5; a long rest empties the sky)
- `SAKURA_RIPPLE_MS` (how long a petal landing on the pond shows as a ripple,
  ms, 0..10000, default 420; pond only)
- `SAKURA_STARS` (stars in the sky, 0..20000, default 110)
- `SAKURA_HORIZON_PCT` (percent of the panel above the ground line, 10..95,
  default per setting: pond 64, mountain 56, garden 58; `SAKURA_WATER_PCT` is
  read as a fallback name)
- `SAKURA_BLOOM` (blossom density, percent of the default, 0..400, default 100)
