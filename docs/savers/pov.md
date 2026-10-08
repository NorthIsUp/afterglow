# `pov`

Points of View — a rotating platonic solid drawn as a grid of dots on its own surface, changing to the next of the five every ten seconds in a burst that throws the points outward and lands them on the new shape with an overshoot.

Five platonic solids in a fixed cycle — tetrahedron, cube, octahedron,
dodecahedron, icosahedron — each held for `POV_HOLD_SECS` and then **burst**
into the next one. The surface is sampled as a triangular lattice of dots rather
than drawn as a wireframe: an edge is shared by two faces so both lattices land
on it and it comes out twice as dense, which is what keeps the faces and edges
legible while the figure turns. Back faces are drawn too — there is no cull, and
depth shading does the work instead, so the far side shows through dim and the
near side bright.

The burst is the point of the saver. Each point is kicked outward along its own
direction by a bump that peaks about a third of the way through and is exactly
zero at both ends, while an `easeOutBack` carries it to its new position — so
the cloud expands, the new solid emerges out of it over the back half of the
transition, and the figure snaps a little past its final shape before settling.
`POV_BURST` is how far the kick throws a point; 0 turns it into a plain morph.

The solids do not have the same number of samples, so the pool is sized to the
largest and a point beyond a smaller solid's count doubles up on an existing
sample. Nothing fades in or out: on a shrink several points converge and merge,
on a grow several leave one site and split.

Sizing is off the SHORTER panel side, so the figure is whole on the 1280x400
panel with empty width either side rather than running off the top and bottom,
and `POV_SPACING` is in dots, so the point count falls with the panel instead of
packing a fixed count into a quarter of the area. Each solid is inflated to the
same mid-radius (the mean of its in- and circumradius) so the five read as one
object changing shape rather than as the figure growing and shrinking — a
tetrahedron inscribed in the same sphere as an icosahedron looks half the size.

A braille dot is `cell_w/2` by `cell_h/4`, square at the default 8x16. Keep that
ratio if you change `POV_CELL_W` / `POV_CELL_H`, or the solid comes out as an
ellipsoid. `SAVER_PIXEL_ASPECT` stretches `cell_h` on top of whatever you set, so
the dot is square on the GLASS rather than in the framebuffer — set the pair as
if the pixels were square and let the knob do the rest.

Source: [`src/pov.rs`](../../src/pov.rs).

## Knobs

- `POV_HOLD_SECS` (1..600, default 10)
- `POV_BURST_MS` (100..5000, default 1200)
- `POV_SPACING` (dots between surface samples, 2..24, default 6)
- `POV_SCALE` (figure radius in thousandths of the SHORTER panel side, 50..600, default 420)
- `POV_BURST` (outward scatter in thousandths of the figure radius, 0..2000, default 450)
- `POV_Z_DIST` (2000..40000, default 6000)
- `POV_RATE_XY` / `POV_RATE_XZ` / `POV_RATE_YZ` (milli-revolutions per second, default 7 / 23 / 13)
- `POV_CELL_W` / `POV_CELL_H` (8, 16)
