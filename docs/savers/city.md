# `city`

![`city`](../media/city.gif)

The After Dark night skyline — lit windows on a black silhouette, scattered stars, a beacon on the tallest tower and the odd shooting star.

`src/city.rs` is the After Dark night skyline, and its palette and layout
are sampled off a reference frame rather than invented. What makes the look:

- **Sky, windows and the warm light are different colour families, not one ramp
  dimmed.** Sky lights are neutral grey with a one-stop blue shift (`g == r`,
  dimmest `#686878`); windows are cyan-white and hotter (`g > r`, hottest
  `#D8F8F8`). Paint both from one grey ramp and the skyline stops reading as lit
  rooms. The warm ramp — `#F8E0A8`, `#D0B070`, `#A08048` — is the window ramp
  turned over (`r > g > b`) at the same luminances, so a warm window reads as
  the same room under a different bulb rather than as a different kind of light.
- **Warm light happens at two scales, and the scales are the point.** A LAMP is
  one window left on in somebody else's cold tower, at 8 per thousand slots;
  a HOTEL is a whole silhouette on the same yellow bulb, and it is the only
  thing in the scene that changes the colour of a whole building. Both counts
  are guarantees rather than averages — the lamps are chosen exactly rather than
  rolled per cell (a per-cell roll at the same nominal rate landed anywhere
  between 0.35% and 1.3% of the panel depending on how the RNG lined up with the
  cell loop), and the warm buildings are capped at two however the roll goes,
  because at four of the two dozen a panel generates it stops being a cold
  skyline with exceptions in it and becomes a two-colour one. Measured: 145 of
  1,385 lit windows warm, 10.5%, in two buildings plus a scatter of lamps. The
  family is fixed per window and survives every twinkle: one that re-rolled it
  each time would flicker between white and yellow, which reads as a broken
  pixel and not as a lamp someone left on.
- **The silhouette is made of windows, not of an outline.** A building is a
  stack of one to three boxes of window slots and nothing draws an edge.
  Buildings share one baseline and overlap, so a nearer one punches its own
  silhouette through the one behind.
- **Six window SHAPES, not one.** This is the thing the saver kept coming back
  on. Pitch, dark floors and fill rate vary which cells are lit; none of them
  varies what a window is, so a skyline built only out of them is the same 6x4
  square at a dozen phases and neighbouring buildings read as one texture
  however differently they are clocked. Real facades differ far more by window
  PROPORTION than by grid pitch, so the shapes span it: a punched square, a
  floor-to-ceiling slot twice as tall as it is wide, a pane twice as wide as it
  is tall, two small windows stacked in one cell (an apartment block beside an
  office slab — twice the storeys on the same grid), an unbroken spandrel course
  that runs into its neighbour, and a curtain-wall strip. Four of them are
  quadrant masks over `font::BRAILLE`, which is this repo's own filled 2x4
  quadrants rather than reading pips, so a window can be a sixth of a cell
  without adding a glyph to the atlas. Measured on one panel: 666 square, 329
  slot, 232 twin, 229 pane, 205 strip, 201 band.
- **Seventeen lighting styles and four silhouettes**, both assigned once and
  stable for the life of the scene. On top of the shape, the styles vary column
  pitch, floor pitch, a dark service floor, a checkerboard, a dark service core
  up the middle of the facade, a tower lit only in its top third, and simply
  mostly dark. The silhouettes are plain, a setback that steps in for the upper
  two thirds, a podium wider than the tower above it, and a narrow mast standing
  clear of the roof. Without that variety, two buildings that touch are one wall
  of lights with no edge in it — the silhouette is there but nothing inside it
  says where one ends and the next begins.
- **Buildings differ in BRIGHTNESS, not only in geometry.** Three profiles over
  the window ramp — mixed, an office floor still at it (narrow and at the hot
  end, every room on the same circuit), and a block where almost nothing is on
  and what is on is barely on. One global ramp gives a skyline that shimmers at
  the same rate in the same colour whatever its pitch, which was most of why the
  old one read as one wall. The profile is per SLOT, like the colour family and
  for the same reason: a dim block that relit off the mixed ramp would walk to
  the district average over a few minutes with nothing failing while it
  happened. Colour family and brightness are ONE index into one table, because
  "which distribution does this room come back at" is the property that has to
  be stable and family is only one axis of it.
- **The roofline is four height classes, not one spread**: low blocks,
  mid-rise, towers, and a spire on one draw in sixteen, with the taller classes
  drawn narrower. Towers standing clear of the crowd are what makes the shape
  read as a skyline — the first cut of this saver used one narrow uniform range
  and rendered a flat band with no towers in it.
- **A red aircraft warning light on the tallest thing in the scene**, centred on
  it and one row above its roof, flashing at 0.67 Hz (1.5 s period, lit for a
  quarter of it) — the rate real ones run at. `#E01818` is the only red in the
  palette and the const block enforces that, so nothing else on the panel can be
  mistaken for it; it is saturated rather than bright, and deliberately dimmer
  than the hottest window, so the one red thing on screen never becomes the
  brightest thing on it. A tie for tallest picks the leftmost, so the beacon
  does not hop between two equal towers.
- **Four star shapes, paired to the brightness ramp**: a small dot at the dim
  end (`#686878`), a taller dot, a sparkle, and a cross at the bright end
  (`#B8B8C8`). Shape and brightness tier are fixed per star and a twinkle
  re-shades only inside its own tier — a star that changed shape, or jumped from
  the dimmest grey to the brightest, reads as noise rather than as a sky.
- **A shooting star about once a minute**, jittered (the spawn is a per-frame
  draw, so the interval is exponential and never metronomic). A five-cell streak
  crosses the sky over about 0.8 s, drawn with the slash that matches its
  direction so the cells join into one line — a tail of dots in a sky made of
  dots is five more stars, which is how the first cut of it read. It is the only
  thing in the scene that moves, so it is the only thing that saves what it
  covered and puts it back exactly; `a_shooting_star_crosses_and_leaves_nothing_behind`
  pins that as an exact cell-for-cell invariant rather than a ratio.
- **Density per tenth of the frame, top to bottom, is 0/14/20/15/11/10/43/63/47/0.**
  Both edges are empty; the sky thins toward the top; street level is darker
  than the floors above it. Those numbers are a guide, not ground truth — they
  come off one compressed screenshot whose anti-aliasing halos count as lit
  pixels — so where they and the art disagree the art wins. The SKY half of the
  table is half what was first read off that screenshot (29/40/31/22/20), which
  rendered a speckle where the reference is a scattered field with plenty of
  black in it. The rendered frame measures 0/11/20/13/10/15/30/49/40/0 in lit
  CELLS; the skyline bands run under the table because the darker styles — floor
  pitch, checkerboard, the tower lit only up top — take slots out of it. Lit
  PIXELS are about double what they were before the window shapes went in, since
  a floor-to-ceiling slot or a spandrel band fills several times the cell a
  punched square does; the cell figures are within a point of the old ones,
  which is the check that the shapes changed the facades and not the layout. The
  tests assert the ordering, a loose envelope, and an absolute ceiling on the
  sky, never the figures.

The twinkle is deliberately slow — 40 window flips and 12 sky re-shades per
second, against the 1,862 window slots a 1920x1080 panel generates, so a given
window turns over about once a minute and the scene reads as a calm shimmer
rather than a busy one. Both are rates in flips per second, not divisors. A
window is RE-DRAWN at its own building's odds rather than toggled: a toggle has
a fixed point at half lit, so a city generated at 88% would quietly fade to 50%
over a few minutes. Re-drawing is memoryless in one step, so the stationary
distribution is exactly the generating one — no drift, not merely slow drift,
and `the_scene_is_still_and_does_not_drain` holds every band to within two
points over 300,000 frames.

It is also by a wide margin the cheapest saver here, and by construction rather
than by luck. The scene is built once and lives in the grid between frames; a
frame rewrites only the cell or two that twinkle and hands `Grid::flush_sparse`
their indices, so there is no per-cell scan, no rebuild and no allocation in the
render path. Measured over 3,599 frames after frame 0 at 1920x1080: median 16
damaged scanlines, mean 23.7, and 64 at the worst quiet frame — against
toasters' median 672 and ascii/matrix's 1056, the whole grid every frame. The
beacon costs one cell twice per flash. The shooting star is the one exception
and it is priced: 46 of those 3,599 frames carried a streak, median 112 damaged
scanlines and 144 at worst, still an eighth of the panel.

None of the window shapes, brightness profiles or warm buildings cost anything
per frame, and that is not luck either: they are all properties of a cell the
generator wrote once, so a frame still rewrites the cell or two that twinkle.
Damage is per CELL and not per lit pixel, so a slot that fills half its cell
reports exactly what a punched square reports. Re-measured after they went in:
median 16, mean 21.7, 64 at worst — the same frame, cell for cell.

The invariant under those numbers is in `Grid::flush_sparse`: `cur` and `prev`
are identical after every flush, so a cell written but left out of `dirty` is a
test failure. That is checked against the grid's two buffers and NOT against the
framebuffer — an unreported write is never blitted, so the framebuffer never
changes and a framebuffer diff cannot see it. Nothing here is special-cased
around that path: the beacon and the streak report their cells like everything
else, which is why `damage_covers_every_changed_scanline` can catch them.

Source: [`src/city.rs`](../../src/city.rs).

## Knobs

- `CITY_WINDOW_PCT` (0..100, default 88)
- `CITY_TWINKLE` (window flips per second, default 40)
- `CITY_SKY_TWINKLE` (sky re-shades per second, default 12)
- `CITY_BEACON_MS` (beacon period, default 1500)
- `CITY_SHOOT_SECS` (mean seconds between shooting stars, 0 = off, default 60)
- `CITY_CELL_W` / `CITY_CELL_H` (12, 16)
