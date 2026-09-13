---
title: screensaver
kind: app
namespace: screensaver
url: https://screensaver.<tailnet>.ts.net
icon: monitor
source: k8s/apps/screensaver/
verify: cd k8s/apps/screensaver/image && SAVER_DUMP=/tmp/ss SAVER_DUMP_FRAMES=100000 SAVER_HTTP=127.0.0.1:8099 cargo run --release
---

# screensaver — HDMI screensavers on whichever Pi5 holds the monitor

A deliberately thin workload that paints an animation onto the HDMI display of
the Talos Pi5 carrying the `hardware.homelab/display: "true"` label. Renderer:
`image/` — a static musl Rust binary on a `FROM scratch` image, ~230 KB, which
writes pixels straight into a DRM/KMS dumb buffer.

## Savers

Pick one with `SAVER` (older spelling: `FIRE_STYLE`). Anything unrecognised
falls back to `ascii` — a headless pod must never crash-loop on a typo.

| `SAVER`     | What                                                                                                                                                                                    | Knobs                                                                                                                                                                                                                                                                                                                         |
| ----------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ascii`     | Doom fire as an ASCII ramp (`" .:-=+*#%@"`), one heat sample per character cell, coloured by the 37-step fire palette. The default.                                                     | `FIRE_CELL` (px, 8..64, default 16)                                                                                                                                                                                                                                                                                           |
| `blocks`    | The same fire drawn as chunky pixels — a solid glyph per cell.                                                                                                                          | `FIRE_SCALE` (px, 1..16, default 4)                                                                                                                                                                                                                                                                                           |
| `matrix`    | Digital rain.                                                                                                                                                                           | `MATRIX_CELL_W` (8..64, default 16), `MATRIX_CELL_H` (8..128, default 32)                                                                                                                                                                                                                                                     |
| `toasters`  | Flying toasters, after After Dark's. Four distinct models.                                                                                                                              | `TOASTER_DENSITY` (per 1000 cells, 1..60, default 4), `TOASTER_SPEED` (px/sec, 8..2000, default 170), `TOASTER_TOAST_PCT` (0..100, default 25), `TOASTER_FLAP_FPS` (1..120, default 15), `TOASTER_CELL_W` / `TOASTER_CELL_H`                                                                                                  |
| `toasters2` | The same flock in BLOCK ELEMENTS at 8x16 cells, so the olive chassis is a solid fill rather than edge strokes.                                                                          | `TOASTER2_DENSITY`, `TOASTER2_SPEED`, `TOASTER2_TOAST_PCT`, `TOASTER2_FLAP_FPS`, `TOASTER2_CELL_W` / `TOASTER2_CELL_H`                                                                                                                                                                                                        |
| `toasters3` | The same flock drawn with a BRAILLE-style 2x4 dot matrix per cell, so one 16x6-cell toaster is a 32x24 bitmap — real slot openings, a dial, a lever, barbed wings. One model, not four. | `TOASTER3_DENSITY` (per 1000 cells, 1..60, default 2), `TOASTER3_SPEED`, `TOASTER3_TOAST_PCT`, `TOASTER3_FLAP_FPS`, `TOASTER3_CELL_W` / `TOASTER3_CELL_H`                                                                                                                                                                     |
| `city`      | The After Dark night skyline — lit windows on a black silhouette, scattered stars, a beacon on the tallest tower and the odd shooting star.                                             | `CITY_WINDOW_PCT` (0..100, default 88), `CITY_TWINKLE` (window flips per second, default 40), `CITY_SKY_TWINKLE` (sky re-shades per second, default 12), `CITY_BEACON_MS` (beacon period, default 1500), `CITY_SHOOT_SECS` (mean seconds between shooting stars, 0 = off, default 60), `CITY_CELL_W` / `CITY_CELL_H` (12, 16) |

Common: `SAVER_FPS` (1..120, default 30; older spelling `FIRE_FPS`),
`DRM_DEVICE` (default `/dev/dri/card0`), `RETRY_SECONDS`.

All of these are plain deployment env changes — no image rebuild.

`SAVER` is only the startup choice: the mirror page has a button per saver, and
`POST /select?saver=<name>` does the same thing by hand. An unknown name is a
400 that changes nothing. The switch rebuilds the saver on the render thread and
bumps the mirror's epoch, so viewers reconnect onto the new geometry exactly as
they do for a modeset — and a restart goes back to whatever `SAVER` says.

### About the matrix saver

It copies the _Reloaded/Revolutions_ look, not the literal 1999 one: the first
film's on-screen code is flat-brightness with only the cursor lit, which on a
glyph grid reads as a rendering bug. Three details are what separate it from the
usual imitation, and all three are in `image/src/matrix.rs`:

- **The glyphs are mirrored left to right.** The production designer drew them
  back to front, "as if we were in the code looking at a screen of code from the
  inside". The mirroring is baked into the glyph table, per glyph — katakana
  reversed, digits not.
- **The glyphs never move.** The grid is stationary; what falls is a wave of
  illumination over it.
- **Every column is always raining.** The motion is a closed-form sawtooth with
  two random floats per column, so several drops share a column at different
  speeds and no column is ever idle. Discrete drops with black gaps are the
  giveaway most implementations ship.

### About the city saver

`image/src/city.rs` is the After Dark night skyline, and its palette and layout
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

### About the toasters saver

The art in `image/src/toasters.rs` is this repo's own ASCII, drawn from a
description — no Berkeley Systems bitmap is copied or transcribed. What is
copied is the behaviour, and the research behind each number is in the module
doc. The three that matter:

- **It travels down-and-left, not left.** 5 px across per 2 down, about 22°
  below horizontal. The 45° every web recreation uses comes from Bryan Braun's
  CSS version, not from the original; 2.5:1 is the only slope anyone has taken
  from shipped code (the After Dark 4.0 binary, where the flock drifts
  `(-60, +24)` per loop). The 1990 Mac module appears never to have been
  disassembled, so that is the best evidence there is.
- **Everything moves in lockstep**, one shared step vector in RUN/RISE units so
  the slope is exact and a per-object speed is not expressible. Objects differ
  only in where they entered and where they are in the wing beat.
- **The body is olive, not silver.** Quantising the 256x64 sheet by pixel count
  puts an olive chassis at a fifth of the toaster, behind a chrome front panel
  (`#909090`, the largest family) and white wings (`#F0F0F0`); the slot rims are
  a lighter chrome and the lever lighter still. Every stroke of the art carries
  an ink key naming its region, so a `/` is a white wing in one column and the
  body's receding edge in the next, and the four doneness slices ramp through
  all eight of the toast sprite's sampled golds and browns. The two olives are
  the depth cue — `#707030` lit top face, `#303010` sides turned away — and they
  pair with the near/far wing whites to keep the three-quarter view from
  flattening. Art and ink resolve to cells in a `const fn`, so a ragged row, an
  ink grid misaligned with its art, or an undefined ink key is `error[E0080]` at
  build time rather than a sprite that renders wrong. The sheet's near-black
  `#101010` is the one family with no entry: it outlined the sprite against the
  sheet's background, and here the gaps between glyphs already draw that.
- **Four wing positions, ping-ponged.** Up, mid, level, down and back, a full
  beat in 0.4 s. The original's sheet is four 64x64 frames — a half-stroke —
  and playing it 0,1,2,3 and snapping back draws only the downstroke.

Toast is the other quarter of the flock (the original spawner holds roughly
three toasters per slice) and comes in four doneness levels, each its own
sprite rather than one slice tinted, as the original's `toast0`..`toast3` were.
Entry is the original's "reverse L": lanes down the top edge and in from the
right, snapped to cell boundaries. Background is solid black and nothing paints
over it, which is why an idle region costs no blits at all.

### About the toasters3 saver

`toasters3` is `toasters`' behaviour at eight times the shape resolution. The
slope, the lockstep, the ping-pong beat, the 3:1 toast ratio and the sampled
palette are the same values, shared where it matters: `toasters3/art.rs` imports
`toasters::art`'s `PAL_RGB` and `ink`, so the two savers cannot drift to
different colours.

**Braille here is this repo's own bitmap, not Unifont's.** Unifont draws
U+2800..28FF as a _reading_ font: a set dot is a 2x2 pip and an UNSET dot is a
1x1 pip, so U+2800 is not blank and a fully-set U+28FF lights 16 of a cell's 128
pixels. As sub-cell graphics that is a 12.5%-coverage ghost with a permanent pip
grid under it, and the non-blank U+2800 would break the transparency the sprite
stamp depends on. `tools/genfont.py` therefore generates the 256 cells as FILLED
4x4 quadrants — the same 2x4 addressing, drawn solid. Pattern `0x00` interns
onto `BLANK` and `0xFF` onto `SOLID`, so a dotless cell is transparent for free.
Cost: the atlas goes from 144 glyphs (2304 B) to 398 (6368 B), `font.rs` from
185 lines to 446. The glyph index is already `u16`, so nothing widened.

**Detail is 8x; colour is 1x.** A `Cell` is one glyph and one palette index, so
every dot inside a cell is the same colour. The art is drawn around that: colour
regions are at least a cell (two dots) wide, and everything finer than a cell is
drawn as NEGATIVE space — the slots, the lever track, the dial recess, the two
chrome ribs, the crumb-tray seam and the gap between the feet are all unlit dots
reading black against chrome. An interior detail drawn as a second _shade_ of
chrome disappears at cell resolution; the same detail drawn as holes does not.
The first draft of this sprite had a solid interior and rendered as a grey blob.

**The art is a dot bitmap, not a row of braille characters.** Sprites in
`toasters3/art.rs` are written as `#` and spaces, two characters across and four
rows down per cell; `bake_braille` packs each block into its pattern byte at
compile time. Braille characters would be the same data in a form nobody can
edit, and would break the byte-indexed const parser — U+28xx is three bytes of
UTF-8 where a column is one byte.

**One model, not four.** `toasters` flies four silhouettes because a sky of one
14x4 line-art shape is repetitive. A 32x24 bitmap already carries more
information than all four of those, so this flies one toaster drawn properly
rather than four drawn four times as expensively.

Measured over 599 frames at 1920x1080, damaged scanlines per frame:

| saver       | median | mean | max  |
| ----------- | ------ | ---- | ---- |
| `city`      | 16     | 24   | 144  |
| `toasters3` | 288    | 304  | 672  |
| `toasters`  | 672    | 626  | 960  |
| `matrix`    | 1056   | 1056 | 1056 |

`city`'s max is its one moving thing: a shooting star fires about once a minute
and costs a median 112 scanlines for the ~25 frames it crosses in. Its quiet
frames — 3,553 of 3,599 in a two-minute run — top out at 64.

`toasters3` is _cheaper_ than `toasters` despite the bigger sprite: the default
density is 2 per 1000 cells rather than 4, because the original sized its flock
by area under sprite (~22% of the screen) and this sprite is 96 cells where the
classic is 56. Nine bigger objects at nine heights merge into fewer damage runs
than fifteen smaller ones.

### About the toasters2 saver

Same scene, different brush: `image/src/toasters2.rs` flies the flock above
drawn in Block Elements — `█ ▀ ▄ ▒ ▛ ▜ ▙ ▟` — instead of in `/`, `|` and `=`.
Every behavioural number is `toasters`' and is not re-argued there: the 2.5:1
diagonal, the shared step vector, the six-step ping-pong, a quarter of the
flock as toast.

Three things are the difference:

- **The olive is a fill, not an outline.** Line art can only put the chassis
  colour on the strokes that outline it; a filled block puts it on the whole
  top face and the whole turned-away side, with the two slots punched out of it
  in the dark olive. Olive is **half the toaster's lit cells** — 63 of 125 in
  level flight, and within a cell of that in all four wing frames — where the
  line-art version could only outline it. That is the whole reason this saver
  exists, and `each_region_is_its_own_colour` asserts the figure so the claim
  cannot rot. (Across a whole panel it measures lower, 40%: the toast and the
  wings dilute it. The sprite is the population the claim is about.)
- **Half the cell, twice the resolution.** 8x16 against the line-art version's
  16x32, so a half block is a square 8x8 pixel and the toaster is 30x7 cells
  where the classic is 14x4 — the same 240 px on the panel, at twice the
  detail. Wing diagonals use the three-quarter blocks at their joints, without
  which a wing is a flight of steps 16 px to a tread.
- **One toaster, four slices.** The original flew one machine; `toasters` flies
  four models as an acknowledged departure. At this resolution the flap and the
  four doneness sprites already carry the variety, so this one is the
  original's — and the slices' scorch is drawn as a 50% shade creeping up from
  the bottom a row a level, not only tinted.

The flock's mix is COUNTED, not rolled per object. A 25% coin flip over sixteen
objects lands on a single slice about one seed in twenty, and nothing re-rolls
an object's kind, so that seed's sky holds one slice for the life of the pod.

Cost scales with the OBJECTS and never with the grid: the scene lives in the
grid's own buffer between frames, each object clears the rectangle it last
stamped and stamps a new one, and `Grid::flush_sparse` blits exactly the cells
named. The dirty list is sorted and deduped before the blit — row-major order is
what lets `Damage` merge marks into runs (out of order it reported 2832
scanlines of a 1072-line grid), and the dedup drops the second blit of every
cell an object cleared and then repainted. It is not literally O(objects): that
sort is `k log k` over the 4237 cells the objects touched, deduping to 2701
blits, and is the largest single cost in the renderer. `k` follows the object
count, not the panel. Measured over 599 frames at 1920x1080: median 800
damaged scanlines of the 1072 the grid owns, against `toasters`' 672 of 1056 —
more because sixteen sprites at a 16 px row pitch make more distinct bands than
`MAX_RUNS` can hold apart — and against 1056 for `ascii` and `matrix`, which
repaint everything every frame.

## Gotchas

- **The mirror looks perfect while the panel is wrong** — the mirror publishes `saver.grid().cells()`, the frame we just _wrote_, not a read-back of the scanout. Anything that clobbers the panel downstream of that write (fbcon, another DRM client) is invisible to it, which is why it sat green for four days while the monitor showed console text. Fix: trust the mirror for "is the renderer running", never for "is this what the screen shows".
- **The monitor shows console text, not the saver** — Talos boots `console=tty0 consoleblank=0`, so fbcon owns the framebuffer and repaints over every frame the renderer flips in. Nothing in the DRM path can see it: our ioctls all succeed, so the pod looks perfect at the right CPU for days. Fix: the `release-fbcon` initContainer writes 0 into the vtconsole bind before the renderer starts. Confirmed 2026-09-12.
- **The panel is black and there is no login prompt** — that is the cost of the above: the display node has no HDMI console while this runs. Fix: nothing, by design. A reboot restores it, or `echo 1 > /sys/class/vtconsole/vtcon1/bind` rebinds it by hand.
- **`Forbidden: a valid Tailscale identity is required` (403)** — the nginx auth sidecar 403s any request without a matching `Tailscale-User-Login` header. Fix: reach it over the tailnet at the URL above, not by port-forward.
- **The mirror page says "no display yet (503)" and retries** — `/meta` is written at modeset, and the pod idles rather than crash-looping when the node holds no monitor. Fix: nothing to fix on the mirror; check `kubectl logs` for the DRM failure, which is the real problem.
- **The mirror is frames behind, or arrives in bursts** — an nginx in front buffers a proxied response by default. Fix: keep the `X-Accel-Buffering: no` header `/stream` sets; don't strip it, and don't "fix" it by adding a streaming exception to the shared `tailscale-auth` component.
- **The mirror shows the wrong colours after `SAVER` changes** — palette and geometry belong to a modeset, and a viewer holding the old ones would mis-colour every cell. Deliberate: the stream closes on modeset. Fix: none, the page reconnects and re-reads `/meta` within two seconds.
- **A dump takes 6 seconds instead of finishing instantly** — the dump path drives the mirror, so it is paced at `SAVER_FPS` whenever the mirror is live. Fix: `SAVER_HTTP=off` for a dump you only want the PPMs from.

## How it works

`image/src/main.rs` opens `/dev/dri/card0`, modesets the connector's preferred
mode, creates an XRGB8888 dumb buffer, and maps it. Each frame the active saver
draws into that mapping and the host tells the driver which scanlines changed.

**Why DRM and not `/dev/fb0`:** Talos v1.14.0 builds its kernel with
`# CONFIG_FB is not set`, so `/dev/fbN` exists on no node — verified on pine and
fir. `CONFIG_DRM_FBDEV_EMULATION` is on, but it only feeds the in-kernel console
(`fbcon`), which is exactly the bare terminal an unconfigured HDMI port shows. No
device tree overlay can bring fbdev back: it is compiled out, not unbound. There
is deliberately **no fbdev fallback** — no node has one, and an untestable
fallback path is worse than none.

**Why the dirty call is load-bearing:** simpledrm — the driver U-Boot hands over
on a Pi 5 — scans out of a _shadow_ buffer. A pixel written into the mapping
reaches the panel only if the driver is told its scanline changed. That is what
`image/src/surface.rs` is about, and its module doc is the contract every saver
is held to; read it before writing a new one. A region written but never reported
shows the previous frame forever, and that bug reproduces on hardware and
nowhere else.

**Layout** (`image/src/`): `surface.rs` (the mapped frame + damage), `grid.rs`
and `font.rs` (character grid + the one glyph blitter), `fire.rs`, `matrix.rs`
`toasters.rs`, `toasters3.rs` and `city.rs` (the savers), `saver.rs` (the trait and the name → saver
dispatch), `host.rs` (DRM), `dump.rs` (headless PPM rendering). Adding a saver
is a module plus one row in `saver::SAVERS`.

## The web mirror

`https://screensaver.<tailnet>.ts.net` shows what the panel is drawing,
live. Identity is the Tailscale-injected `Tailscale-User-Login` header enforced
by the `tailscale-auth` sidecar — there is no login and there must never be one.

**What crosses the wire is cells, not pixels.** Every saver paints through
`Grid`, so the panel's whole state is `cols * rows` of a `Cell` — a glyph index
and a palette index packed into one `u32` — over a palette and a glyph table
that are both fixed for the modeset. The mirror sends the changed cells. It is
not compression; it is sending the thing the renderer already has.

Measured at 1920x1080, `SAVER_FPS=15`:

| `SAVER`  | grid    | cells  | changed/frame | damaged scanlines/frame |
| -------- | ------- | ------ | ------------- | ----------------------- |
| `matrix` | 120x33  | 3960   | 792 (20.0%)   | 1056 — the whole panel  |
| `ascii`  | 120x67  | 8040   | 2546 (31.7%)  | 1056                    |
| `blocks` | 480x270 | 129600 | 11022 (8.5%)  | ~620                    |

The right-hand column is why every pixel-shaped answer loses. Matrix dirties
every scanline every frame, so "ship the damaged rows" ships 8.1 MB per frame;
the same frame is 6.4 KB of cells. Re-encoding the panel as JPEG or PNG instead
costs the Pi tens of milliseconds per frame, against a renderer that measures
113m of one core in total.

**What it costs the renderer.** With nobody watching: one relaxed atomic load
per frame. With a viewer: one `memcpy` of the cell array (15.8 KB for matrix)
under a `try_lock` that is _skipped_ rather than waited on — the display can
never be made to wait for the web path, and a frame the mirror misses is just a
frame the mirror misses. The diff, the encode and the socket are all on the
viewer's own thread.

`GET /stream` is an HTTP/1.1 chunked binary stream — one-way server → client,
which `fetch` and a stream reader already do, so a WebSocket would buy nothing
for a hand-rolled SHA-1 and a frame codec. Records are self-describing
(`u32 count`, then `count * (u32 index, u32 cell)`), which is what makes them
survive nginx re-chunking them on the way through the gate. `GET /meta` is the
geometry, palette and glyph table; `GET /` is the page. `SAVER_HTTP=off`
removes all of it.

## Looking at a saver without a monitor

`SAVER_DUMP` renders to PPM files and exits, on any machine, with no display:

```sh
cd k8s/apps/screensaver/image
cargo build --release
SAVER=matrix SAVER_DUMP=/tmp/mx SAVER_DUMP_FRAMES=30 ./target/release/screensaver
```

Also honours `SAVER_DUMP_EVERY` (write every Nth frame, default 10),
`SAVER_WIDTH`, `SAVER_HEIGHT`. It renders through the exact `saver::frame` call
the DRM host makes, so it is not a mock, and it runs the **damage self-check**:
any scanline whose pixels changed without being reported exits non-zero. That is
the one bug a monitor cannot help with and a laptop can.

`<dir>/damage.txt` gets a line per frame with the reported runs — the headless
read on whether a saver is quietly flushing the whole panel.

The dump also drives the **web mirror**, so http://127.0.0.1:8080 shows the same
saver in a browser with no card at all — the one way the mirror is testable off
the hardware. Pass a large `SAVER_DUMP_FRAMES` and it runs at `SAVER_FPS`
indefinitely.

View with `magick frame-00000.ppm out.png`, or
`ffmpeg -i 'frame-%05d.ppm' out.gif`.

## The glyph table

`image/src/font.rs` is **generated and committed** so the image build stays a
pure `cargo build --locked` with no Python in the build stage. Regenerate with:

```sh
cd k8s/apps/screensaver/image
python3 tools/genfont.py tools/unifont-subset.hex -o src/font.rs && cargo fmt
```

CI re-runs exactly that and diffs, so the table cannot drift from its source.
Glyphs are 8x16, one byte per row, from a vendored 6 KB subset of GNU Unifont's
`.hex` — already a bitmap, so there is no rasteriser and no font crate. The SIL
OFL 1.1 arm of Unifont's dual licence is elected explicitly (`tools/LICENSE.unifont`);
the derived table is not called Unifont. Fire's ten ramp glyphs are this repo's
own 8x8 bitmaps, row-doubled, which is why fire renders pixel-identically to the
pre-refactor build. `ASCII` indexes U+0020..=U+007E by `c - 0x20`, which is what
lets a saver write its sprites as plain string literals; identical bitmaps are
interned, so a character another set already pulled in costs no extra slot.

## Building / publishing the image

Built and pushed by CI (`.github/workflows/screensaver-image.yml`) on any change
under `image/`: arm64-native, running `cargo fmt --check`, `clippy -D warnings`,
the tests and a dump render, then publishing as `latest` and `sha-<commit>`. The
image is **private**; the `ghcr` pull secret is delivered to the `screensaver`
namespace via `k8s/secrets/ghcr-screensaver.sops.yaml`.

Then bump the `image:` digest in `deployment.yaml` — in its own commit, with no
env changes in it, so the new binary always runs against the old env block first.

<!-- gen:facts -->

|              |                                                                                                                                                                    |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Namespace    | screensaver                                                                                                                                                        |
| Image        | `busybox:1.36`, `ghcr.io/northisup/screensaver@sha256:ad224264cf0e24a35671772ee5ba435e4c028f800d041a0685e3f7567463b384`, `nginxinc/nginx-unprivileged:1.27-alpine` |
| Ports        | `screensaver 8080`, `ts-auth 8085`                                                                                                                                 |
| Storage      | —                                                                                                                                                                  |
| Memory limit | `screensaver 128Mi`, `ts-auth 64Mi`                                                                                                                                |
| Strategy     | `Recreate`                                                                                                                                                         |
| nodeSelector | `hardware.homelab/display=true`                                                                                                                                    |
| Components   | `tailscale-auth`                                                                                                                                                   |
| Depends on   | —                                                                                                                                                                  |

<!-- /gen:facts -->

## Debugging

There is no shell in the image, so `kubectl exec … ls /dev/dri` is gone — that is
how the missing `CONFIG_FB` was found in the first place. The binary prints its
own device diagnostics on failure and idles rather than crash-looping when no
display is present, so `kubectl logs` is the first stop; `kubectl debug` with an
ephemeral container covers the rest.

Two failures only the panel can show, both of which look correct in review:

- The first frame must paint the whole panel (the buffer arrives zeroed and
  `set_crtc` has already scanned that black frame out).
- SIGTERM must hand the console back. The teardown's `map.fill(0)` is never
  dirtied and so never reaches hardware; the `set_crtc` restore is the
  load-bearing half.
