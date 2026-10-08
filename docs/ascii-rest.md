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
- `ASCII_REST_TOUR_HOLD_SECS` (1..3600, default 14, each hold drawn from 60%..140% of it)
- `ASCII_REST_TOUR_MAX_ZOOM_PCT` (100..600, default 250, of the cover view's cell)
- `ASCII_REST_TOUR_SEED` (0 = roll one from the clock and pid; any other value reproduces the tour exactly)

## The scene tour

For the halftone scenes the camera moves: it holds a view for 8-20 seconds,
glides to the next over 3-6, and every third to fifth move pulls back to the
cover view or briefly to the whole picture, ground-colour bars and all. Zoom is
cell size, so every picture cell is still one grid cell and the dots just get
bigger: up to 2.5x the cover cell, stepping through each integer width on the
way. Close-ups go where the picture has something in it. Each move scores the
frame on screen in 4x4-cell blocks (the dither's period) by contrast with
their neighbours and by brightness. It frames a block drawn by that score
(the moon, the lamp, the dome), or a corner or edge weighted the same way,
and plays down anything the last three close-ups showed.

A hold costs what the fixed view costs. While the tour holds the cover view it
draws through the same grid, byte for byte. A glide repaints the whole panel
at each new cell width, and on pine with 1 s holds (nearly all glide)
night-coast measures 1.75x matrix against 1.49x untoured. The terminal host
shows the tour; the web mirror always shows the cover view: each zoom step is a new geometry, and
re-describing the mirror for each one would reconnect every viewer a dozen
times per glide.

## `-wide` variants

The same thirteen scenes recomposed at 320x100 (3.2:1), so they fill pine's glass uncropped. Knobs: as the scenes.

**`-wide` variants** are this repo's own: a scene recomposed on a 320x100 grid
so it fills pine's 3.2:1 glass with nothing cropped, rather than stretched. The
original stays untouched and golden-exact; a wide one sets `UPSTREAM = false`,
so its `golden` test skips — there is no upstream output to compare against.

## Character pieces

ascii.rest's character pieces, one ink each. Knobs: none.

## `plasma`

`plasma` started as the twenty-second port and is now its own saver; see [its page](savers/plasma.md).
