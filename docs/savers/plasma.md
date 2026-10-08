# `plasma`

![`plasma`](../media/plasma.gif)

The demo-scene plasma, full screen: four sine fields summed into soft blobs of density, drawn with ascii.rest's ramp `.,-~:;=+*#%@` at the panel's own resolution, any size or shape. Colour is a second, slower field sweeping a deep blue → violet → magenta → coral → orange → gold → mint → sky wheel across the panel, so the blobs swim through bands of hue; brightness follows density, so the cores glow. Loops every 30 s.

`plasma` started as the twenty-second port and is now its own saver in
`src/plasma.rs`: upstream's fixed 64x22 picture sat small and boxed on pine's
glass, and a field this simple can be evaluated at any resolution. It keeps
upstream's ramp, four terms and 30 s loop, scaled so a panel of any shape shows
as many blobs per unit area as upstream's picture did. It changes ~17% of its
cells a frame and measures ~1.15x matrix at 1920x1080 and ~0.8x on pine's
geometry (`SAVER_PIXEL_ASPECT=180`).

Source: [`src/plasma.rs`](../../src/plasma.rs).

## Knobs

- `PLASMA_CELL_W` / `PLASMA_CELL_H` (glass px, 4..64 / 4..128, default 12 / 24)
