# `xwing`

![`xwing`](../media/xwing.webp)

The Death Star run from the cockpit, in three acts on a loop: the station swelling out of a starfield, a low pass over its greebled surface, then the trench — walls closing in and the targeting computer swinging down over the view. Green fire comes in and red goes out in every act, TIE fighters cross, chase and pass the canopy, and anything a red bolt reaches explodes.

`src/xwing.rs` is the only saver here with a beginning, a middle and an
end. The other forward-motion savers are steady states you can join at any
moment; this one builds — open space, then a surface, then a trench that closes
in — and then cuts back and does it again.

- **Black and cold grey, and the only saver with stars in it.** Every grey in
  the palette is blue-shifted (`b > g > r`); the only colours are green turret
  fire, red cannon fire, and the targeting computer's amber. If a frame of this
  could be recoloured into a desert or a forest, the palette has drifted.
- **The plating is generated, not tiled.** One function, `greeble`, textures the
  station, the plain and the trench walls from their own two surface
  coordinates. It hashes at two scales — a block that may be sunk into the
  surface (a trench within the trench) or raised into a housing, and a
  third-size detail inside it (a vent, a nub) — so there is no period to catch.
  The equatorial trench and the superlaser dish are cut in the same coordinates,
  which is why they swell with the station instead of being drawn onto it.
- **Panel seams are NEGATIVE space.** A cell carries one colour, so a seam drawn
  as a darker shade would have to be a whole cell wide. Drawn as unlit braille
  dots punched out of a full block it is a quarter of one and lands where the
  seam actually is. Seams stop being drawn once a cell is wider than a block —
  past that they are finer than the panel can resolve, and drawing them anyway
  is moire. That cut-off doubles as the distance LOD, and without it the horizon
  — where every surface here converges — is a band of static.
- **The joins are flight, not edits.** Act 1 ends with the plating covering
  the frame and act 2 opens NOSE-DOWN, which is also plating covering the
  frame, so the cut lands on matching pixels; the horizon then sweeps down into
  place over `XWING_PITCH_MS` — a dive being pulled out of. Act 3 opens on the
  plain act 2 ended on and the walls GROW out of it over `XWING_RISE_MS` before
  they start closing in. The only join that is still a cut is the last one, and
  it is behind the explosion that ends act 3.
- **Every act is under fire, and only the targeting computer is trench-only.**
  Green comes in from a gun emplacement on the station's face, then a surface
  battery, then a wall turret; red goes out from the wingtips throughout. TIEs
  fly one of three sorties — a crossing shot, a pursuit ahead of the camera, a
  pass across the canopy — and the silhouette is drawn from its proportions
  (two cut-cornered hexagons at `|dx| ~ 1.5`, a ball, the struts), because at
  eight cells across that outline is the only thing it could be and two
  rectangles would be a barbell. A red bolt that reaches one takes it, and the
  explosion is flash, expanding shell, debris, fade — a one-frame white blob
  reads as a dropped frame, which is what `an_explosion_flashes_expands_and_ends`
  is there to prevent.
- **The geometry is in glass units, not cells.** `SAVER_PIXEL_ASPECT=180`
  nearly halves `rows`, so a trench whose walls were placed at "a quarter of
  `cols`" would close at a different rate on the panel than in a 1080p dump.
  Everything projects into units of the glass width of one framebuffer pixel;
  the panel squashes vertically by the aspect, so a framebuffer pixel is
  `100 / aspect` units TALL and `row_v` DIVIDES the already-stretched `cell_h`
  back out — the same conversion `marble` makes, and it comes back equal to
  `col_v`, since `Grid` made the cell square on the glass in the first place.
  It shipped inverted, which composed a 1:1 picture onto the panel's 3.2:1
  glass and squashed the whole run into a third of its vertical extent;
  `the_shot_is_the_same_at_both_pixel_aspects` could not see it because both of
  its arms were derived from the same inverted convention, so
  `the_cell_is_square_on_the_glass` now pins the operator directly. **The live
  panel is 1920x1080 at 180, which is 1920x600 — 3.2:1 — on the glass**, and
  that is the geometry to dump at; 1280x400 at 180 is a much narrower slice of
  glass than the panel actually is.
- **Cost.** A full repaint with a couple of divides and two hashes per cell.
  Measured at 1920x1080 against `moire`: act 1 (approach) 1.55x, act 2
  (surface) 1.24x, act 3 (trench) 1.92x — the trench resolves a wall AND a
  floor per cell and takes the nearer. TIEs, explosions and the extra fire cost
  5-13% of an act each, because a sprite is a bounded box where the scene is
  every cell. On the live panel the whole cycle is 1.43x `moire`, around 200m
  of the pod's 500m.

Source: [`src/xwing.rs`](../../src/xwing.rs).

## Knobs

- `XWING_SEED`
- `XWING_APPROACH_SECS` / `XWING_SURFACE_SECS` / `XWING_TRENCH_SECS` (1..600, default 11 / 9 / 13)
- `XWING_SPEED` (world units/sec, 50..20000, default 900)
- `XWING_GREEBLE` (plating block size, 4..2000, default 60)
- `XWING_FOV` (focal length in thousandths of the visual panel width, 200..3000, default 800)
- `XWING_STARS` (0..4000, default 170)
- `XWING_TOWERS` (0..400, default 16)
- `XWING_BOLTS` (0..400, default 28)
- `XWING_TIE_SECS` (mean seconds between TIE sorties, 0..600, default 7; 0 = none)
- `XWING_TIES` (0..200, default 6)
- `XWING_BOOM_SECS` (mean seconds between surface explosions, 0..600, default 9; 0 = none)
- `XWING_BOOMS` (0..200, default 6)
- `XWING_PITCH_MS` (act 2's nose coming up, 0..10000, default 1600)
- `XWING_RISE_MS` (act 3's walls rising, 0..10000, default 1400)
- `XWING_CELL_W` / `XWING_CELL_H` (4..32, default 8 / 8)
