# The ascii.rest ports

Twenty-one pieces from [ascii.rest](https://ascii.rest) ([source](https://github.com/bas3line/ascii),
MIT, by @bas3line), ported line for line to `src/ascii_rest/`. Upstream a piece
is `frame(t) -> string` over a fixed grid; here it writes cells, and one generic
saver, `Play<P: Piece>`, does the rest — the clock, upstream's own frame rate (a
15 fps scene shades 15 times a second at `SAVER_FPS=30`), centring, the ground
colour and the flush. A port is only its drawing code.

- **Scenes** (`cell: 1`, a palette) share `halftone::Dots`: the 4x4 ordered
  dither, the " ·•●" dot glyphs (drawn round on a square-glass cell, sized to
  upstream's coverage) and the cached nearest-palette lookup.
- **Text pieces** (`cell: 2`) have no palette upstream, so each gets one ink
  picked to suit it. Their box-drawing and block characters are in the glyph
  table as `font::TEXT`.

Pictures are drawn 1:1, one piece cell per grid cell, never resampled: that
would smear the dither. Scenes fill the panel and crop the overflow, keeping a
band of rows chosen per scene (`Fit::Cover { anchor }`) so the horizon, the moon
or the Taj's dome stays in frame; on pine's 3.2:1 glass that is about 60% of
each scene's height. Text pieces keep `Fit::Contain` and sit whole over their
ground, since a cropped one loses words.

Every port is checked cell for cell against upstream's own output — glyph and
palette index, four frames each, stateful pieces stepped through every tick
between. All 21 match exactly. The math is f64 like JavaScript, `Float32Array`
storage stays f32, and `math.rs` reproduces JavaScriptCore where it differs from
libm (`Math.hypot`, `Math.round`, integer `Math.pow`):

```sh
git clone https://github.com/bas3line/ascii /tmp/ascii
bun tools/ascii-rest-golden.ts /tmp/ascii /tmp/golden night-coast aurora   # any slugs
ASCII_REST_GOLDEN=/tmp/golden cargo test --release ascii_rest -- --ignored --nocapture
```

Cost: night-coast measures 2.3x matrix per panel frame.

## Scenes

ascii.rest's thirteen halftone scenes — landscapes shaded cell by cell and drawn as dots whose size is their brightness. Each has its own page, linked from the
[README](../README.md#asciirest-halftone-scenes).

### Knobs

- `ASCII_REST_TOUR` (0..1, default 1; 0 is the fixed cover view)
- `ASCII_REST_TOUR_SHOT_SECS` (4..600, default 20, each shot drawn from 75%..150% of it; a shot with little to move ends sooner)
- `ASCII_REST_TOUR_MAX_ZOOM_PCT` (100..600, default 250, of the cover view's cell)
- `ASCII_REST_TOUR_CUTS` (0..1, default 0; 1 cuts between framings and drifts slowly within each, instead of one continuous move)
- `ASCII_REST_TOUR_SEED` (0 = roll one from the clock and pid; any other value reproduces the tour exactly)
- `ASCII_REST_TITLE` (0..1, default 0) — the scene's name (`night coast`) in the
  panel's bottom-left corner, on a band of ground. Every ascii.rest piece reads it.

The title is stamped onto the grid after the picture is mapped, so a piece's
`frame` — and the golden test — never sees it; off, it costs one `Option` check
a frame. A scene's cell is a few pixels wide, so its title is the font's glyphs
drawn in dots, rows paired and blank columns trimmed. A tour close-up's cells
are several times wider, so once the dots would take more than half the width
the title switches to one glyph per cell, as a character piece's always is.

## The scene tour

For the halftone scenes the camera moves Ken Burns style. Each shot is a slow
push in or pull out with a gentle pan, 15-30 seconds at the default, eased at
both ends with a half-second settle, and the next shot carries on from where
it ended. Every third to fifth shot pulls back to the cover view or to the
whole picture, ground-colour bars and all. A pan covers at most half the
panel. Close-ups go where the picture has something in it: each shot scores
the frame on screen in 4x4-cell blocks (the dither's period) by contrast with
their neighbours and by brightness, aims at a block drawn by that score (the
moon, the lamp, the dome), and plays down anything the last three close-ups
showed.

Zoom is cell size, so every picture cell is still one grid cell and the dots
just get bigger: up to 2.5x the cover cell. The cell width steps a pixel at a
time, evenly through the shot, and each step re-places the picture so the
shot's focus stays put. The pan is pixel-precise: the grid is drawn shifted
by the part of a cell the camera has passed, with the partial cells at the
edges clipped, and a column and row of bleed past the panel's edge to slide
in. A shot with few zoom steps and little pan is cut short so it never sits
still for long. Near the whole picture only zoom can move, and its steps are
a fifth of the picture each, so that is where the camera pauses longest,
about three seconds.

Every move repaints the whole panel, so the camera moves only on frames the
piece draws (15 a second for the scenes): between them nothing else changes,
and moving there too would double the cost. Over 100 s on pine's geometry
night-coast measures 1.56x matrix, against 1.23x untoured and 1.43x with
`ASCII_REST_TOUR_CUTS=1`. The terminal host and the web mirror show the tour
too. The mirror keeps the cover view's grid — each zoom step is a new
geometry, and re-describing the mirror for each would reconnect every viewer
a dozen times a shot — and fills it with the panel's cell under each of its
cells' centres, pixel shift included, the whole picture's bars too. The
mapping is redone only when the view moves, and only while someone watches.

## `-wide` variants

The same thirteen scenes recomposed at 320x100 (3.2:1), so they fill pine's glass uncropped. Knobs: as the scenes.
The mirror page lists each scene once, with an `expanded` toggle between the two.

**`-wide` variants** are this repo's own: a scene recomposed on a 320x100 grid
so it fills pine's 3.2:1 glass with nothing cropped, rather than stretched. Each
shares its original's module: one `Scene<IS_WIDE>` drawn from a `Layout` (width,
sun, landmarks, extra peaks and props), with an `ORIGINAL` that keeps upstream's
literals and a `WIDE` beside it. The original stays golden-exact; the wide one
is marked `#[no_upstream]` in `each_piece!`, so it gets no `golden` test — there
is no upstream output to compare against.

## Character pieces

ascii.rest's character pieces, one ink each. Knobs: `ASCII_REST_TITLE`.

## Full-screen twins

Each character piece has a `-wide` twin that fills the whole panel at any size or shape, with no bars. A twin is not a fixed picture fitted to the panel, the way a scene's `-wide` variant is. It is a `Canvas`: it is given the grid's own `cols x rows` and draws every cell, and `Fill` is its saver, as `Play` is a piece's. The grid's cells stay glyph-shaped, twice as tall as wide. Each twin keeps its original's ink, motion and frame rate, and shares its original's module and drawing code, parameterised by size. The original stays golden-exact. The twin is marked `#[fill]` in `each_piece!`, which gives it its own exercise test (every panel shape down to 128px, both sides reached, no allocation) and no golden test. The mirror page pairs each piece with its twin behind the same `expanded` toggle the scenes use.

What full screen means for each is on its page. Knobs:

- `ASCII_REST_TEXT_CELL_W` / `ASCII_REST_TEXT_CELL_H` (glass px, 4..64 / 8..128, default 12 / 24): 160x25 cells on pine, 160x45 at 1080p
- `ASCII_REST_TITLE` (0..1, default 0)

## `plasma`

`plasma` started as the twenty-second port and is now its own saver; see [its page](savers/plasma.md).
