# `speeder`

A first-person speeder-bike chase through the forest moon — enormous redwood trunks rush past at parallax while the bike weaves between them on two incommensurate sines, dappled canopy light streams over the mossy floor, and every so often a fallen trunk sweeps up out of frame to be ducked under or another bike flashes across the view. One spawn in twenty is aimed at where the camera WILL be, so the near misses are deliberate; a trunk moving too fast for the eye to hold an edge on is stippled rather than solid. Bark grain is painted in the trunk's own metres, so it slides and swells with its trunk; a trunk too far off to resolve the grain is drawn plain rather than shimmering. Its grid is SQUARE, so all of the perspective is in cells and `SAVER_PIXEL_ASPECT` corrects it for free — the opposite choice to `warp`'s.

Source: [`src/speeder.rs`](../../src/speeder.rs).

## Knobs

- `SPEEDER_CELL` (px, 4..32, default 8, square)
- `SPEEDER_SPEED` (metres/sec, 10..300, default 58)
- `SPEEDER_TRUNKS` (8..400, default 60)
- `SPEEDER_FOV` (focal as a per-cent of COLUMNS, 20..200, default 62 — smaller is wider and faster-looking)
- `SPEEDER_HORIZON` (eye line as a per-cent of rows, 10..80, default 44)
- `SPEEDER_WEAVE` (swing off the path in DECIMETRES, 0..200, default 64; 0 flies straight)
- `SPEEDER_DAPPLE` (per-cent of the floor in a pool of light, 0..100, default 34)
- `SPEEDER_LOG_SECS` (mean seconds between fallen trunks, 0..600, default 16; 0 = off)
- `SPEEDER_RIDER_SECS` (mean seconds between other bikes, 0..600, default 12; 0 = off)
- `SPEEDER_SEED` (0 = roll one from the clock and pid; any other value reproduces the ride exactly)
