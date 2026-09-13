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

| `SAVER`     | What                                                                                                                                                                                                                                                                                                                                                                                                                  | Knobs                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| ----------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ascii`     | Doom fire as an ASCII ramp (`" .:-=+*#%@"`), one heat sample per character cell, coloured by the 37-step fire palette. The default.                                                                                                                                                                                                                                                                                   | `FIRE_CELL` (px, 8..64, default 16)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `blocks`    | The same fire drawn as chunky pixels — a solid glyph per cell.                                                                                                                                                                                                                                                                                                                                                        | `FIRE_SCALE` (px, 1..16, default 4)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `matrix`    | Digital rain.                                                                                                                                                                                                                                                                                                                                                                                                         | `MATRIX_CELL_W` (8..64, default 16), `MATRIX_CELL_H` (8..128, default 32)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `toasters`  | Flying toasters, after After Dark's. Four distinct models.                                                                                                                                                                                                                                                                                                                                                            | `TOASTER_DENSITY` (per 1000 cells, 1..60, default 4), `TOASTER_SPEED` (px/sec, 8..2000, default 170), `TOASTER_TOAST_PCT` (0..100, default 25), `TOASTER_FLAP_FPS` (1..120, default 15), `TOASTER_CELL_W` / `TOASTER_CELL_H`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| `toasters2` | The same flock in BLOCK ELEMENTS at 8x16 cells, so the olive chassis is a solid fill rather than edge strokes.                                                                                                                                                                                                                                                                                                        | `TOASTER2_DENSITY`, `TOASTER2_SPEED`, `TOASTER2_TOAST_PCT`, `TOASTER2_FLAP_FPS`, `TOASTER2_CELL_W` / `TOASTER2_CELL_H`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `toasters3` | The same flock drawn with a BRAILLE-style 2x4 dot matrix per cell, so one 16x6-cell toaster is a 32x24 bitmap — real slot openings, a dial, a lever, barbed wings. One model, not four.                                                                                                                                                                                                                               | `TOASTER3_DENSITY` (per 1000 cells, 1..60, default 2), `TOASTER3_SPEED`, `TOASTER3_TOAST_PCT`, `TOASTER3_FLAP_FPS`, `TOASTER3_CELL_W` / `TOASTER3_CELL_H`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `city`      | The After Dark night skyline — lit windows on a black silhouette, scattered stars, a beacon on the tallest tower and the odd shooting star.                                                                                                                                                                                                                                                                           | `CITY_WINDOW_PCT` (0..100, default 88), `CITY_TWINKLE` (window flips per second, default 40), `CITY_SKY_TWINKLE` (sky re-shades per second, default 12), `CITY_BEACON_MS` (beacon period, default 1500), `CITY_SHOOT_SECS` (mean seconds between shooting stars, 0 = off, default 60), `CITY_CELL_W` / `CITY_CELL_H` (12, 16)                                                                                                                                                                                                                                                                                                                                                                                                                |
| `tactiles`  | After Dark's TacTiles — a grid of square tiles carrying one geometric glyph each (bar, diagonal, corner, arc). Shape, rotation and colour are three travelling sine waves quantised into bands, so the tiling rearranges itself continuously without ever looking rolled.                                                                                                                                             | `TACTILES_CELL_W` / `TACTILES_CELL_H` (px, 4..32, default 8 each), `TACTILES_TILE` (tile side in CELLS, 2..24, default 6 — so a 48px tile), `TACTILES_SPEED` (wave travel, milli-rad/sec, 10..5000, default 700), `TACTILES_SCALE` (wave spatial frequency, milli-rad per tile, 10..3000, default 430), `TACTILES_STROKE` (glyph stroke as a percent of the tile, 5..50, default 24), `TACTILES_SEED` (0 = roll one from the clock and pid; any other value reproduces the pattern exactly)                                                                                                                                                                                                                                                  |
| `zot`       | Lightning. A leader crosses the panel and forks, each fork forking again; the channel strobes over a white core in a blue halo, goes out, and usually fires again down the same channel a beat later — a double or triple flash — before the panel goes dark for a second or two. Endpoints are drawn on the perimeter by arc length, so the bolt crosses the panel at any aspect — 1920x1080 or the native 1280x400. | `ZOT_BOLT_MS` (one stroke, 60..3000, default 380, ±25% per stroke), `ZOT_RESTRIKE_PCT` (chance of another stroke down the same channel, 0..100, default 70, up to three strokes), `ZOT_STROKE_GAP_MS` (dark between those strokes, 10..1000, default 110), `ZOT_GAP_MIN_MS` / `ZOT_GAP_MAX_MS` (100..60000, default 700 / 2400), `ZOT_FORK_PCT` (0..100, default 9), `ZOT_AIR_PCT` (bolts ending in mid-air, 0..100, default 30), `ZOT_JITTER` (milli-radians of wander per step, 10..3000, default 900), `ZOT_GLOW_PCT` (afterglow wash peak, 0..100, default 45), `ZOT_HALO_PCT` (0..100, default 70), `ZOT_CELL_W` / `ZOT_CELL_H` (8, 16), `ZOT_SEED` (0 = roll one from the clock and pid; any other value reproduces the storm exactly) |
| `strings`   | "String Theory", the After Dark module — a polygon whose corners each bounce independently, redrawn every frame over the fading outlines behind it, so a ribbon of lines sweeps and folds. Three ribbons, one hue each.                                                                                                                                                                                               | `STRINGS_RIBBONS` (1..4, default 3), `STRINGS_VERTICES` (corners per polygon, 2..8, default 4), `STRINGS_FADE_MS` (200..20000, default 1000), `STRINGS_SPEED` (sub-cells/sec per corner, 5..1000, default 60), `STRINGS_CELL_W` / `STRINGS_CELL_H` (8, 16)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| `life`      | Conway's Game of Life on a toroidal board, cells coloured by age — white-hot at birth, cooling to blue — with a fading ash trail. A churn-triggered "meteor" of fresh soup keeps it from settling into still lifes.                                                                                                                                                                                                   | `LIFE_GPS` (generations per second, 1..60, default 10), `LIFE_DENSITY` (percent alive in fresh soup, 5..80, default 38), `LIFE_SEEDS` (startup soup discs, 1..64, default 12), `LIFE_QUIET` (churn per MILLE below which a generation is quiet, 1..500, default 30), `LIFE_PATIENCE` (quiet generations before a meteor, 1..600, default 12), `LIFE_FADE` (generations of ash, 0..6, default 5), `LIFE_CELL_W` / `LIFE_CELL_H` (4..64, 8 and 8), `LIFE_SEED` (0 = roll one from the clock and pid; any other value reproduces the board exactly — not to be confused with `LIFE_SEEDS`, which counts soup discs)                                                                                                                             |

| `marble` | Marble Madness — an isometric course floating in black space, a chrome marble rolling down it on autopilot, and hazards trying to stop it. Ramps, narrow catwalks over nothing, acid pools, a hammer and a leashed black hunter marble. Falling off costs a respawn at the last checkpoint; reaching the goal generates a new course. | `MARBLE_TILE` (tile width in GLASS px, 0 = derive from the panel, else 16..160), `MARBLE_SPEED` (milli-tiles/sec, 500..20000, default 4200), `MARBLE_STEER` (autopilot thrust, milli-tiles/sec², 100..40000, default 5200 — how well the invisible player plays), `MARBLE_PATIENCE` (steps with no route progress before a respawn, 30..4000, default 260), `MARBLE_COURSE_S` (10..3600, default 150), `MARBLE_HAZARDS` (0..6, default 3), `MARBLE_CELL_W` / `MARBLE_CELL_H` (4..32 / 4..64, 8 and 8), `MARBLE_SEED` (0 = roll one from the clock and pid; any other value reproduces the course exactly) |
| `hardrain` | A downpour: steeply slanted streaks under a gusting wind, a mist the sky is veiled in, squalls sweeping across, and standing water at the bottom that ripples where the rain lands. The storm to `rain`'s drizzle. | `HARDRAIN_DENSITY` (per 1000 cells, 0..400, default 55), `HARDRAIN_SPEED` (hundredths of a panel height per second, 10..1000, default 110), `HARDRAIN_WIND` (cells sideways per 100 of fall, -300..300, default 95), `HARDRAIN_GUST` (same units, 0..300, default 70), `HARDRAIN_GUST_SECS` (1..600, default 11), `HARDRAIN_POOL_PCT` (water depth as a percent of rows, 0..40, default 9; 0 = off), `HARDRAIN_SQUALL_SECS` (mean seconds between squalls, 0..600, default 17; 0 = off), `HARDRAIN_SPRAY` (droplets per impact, 0..4, default 2), `HARDRAIN_CELL_W` / `HARDRAIN_CELL_H` (8, 16) |
| `doodles` | After Dark's scribbler: pens wander the panel leaving one continuous freehand line each, looping back over themselves until the sheet is full, then it fades and a new one starts. Each pen's hue sweeps across the life of a doodle, so the scribble shows its own history. | `DOODLES_PENS` (1..4, default 3), `DOODLES_SPEED` (pen steps/sec at 1080p, 30..8000, default 420), `DOODLES_INERTIA` (per-mille of the turn rate carried to the next step, 500..999, default 960), `DOODLES_WANDER` (milli-rad of turn-rate noise per step, 1..300, default 20), `DOODLES_CURL` (milli-rad/step, tightest curl, 5..500, default 90), `DOODLES_FILL_PCT` (1..90, default 28), `DOODLES_MAX_S` (5..900, default 90), `DOODLES_FADE_MS` (200..10000, default 1600), `DOODLES_CELL_W` / `DOODLES_CELL_H` (8, 16), `DOODLES_SEED` (0 = roll one from the clock and pid; any other value reproduces the doodle exactly) |
| `pov` | Points of View — a rotating platonic solid drawn as a grid of dots on its own surface, changing to the next of the five every ten seconds in a burst that throws the points outward and lands them on the new shape with an overshoot. | `POV_HOLD_SECS` (1..600, default 10), `POV_BURST_MS` (100..5000, default 1200), `POV_SPACING` (dots between surface samples, 2..24, default 6), `POV_SCALE` (figure radius in thousandths of the SHORTER panel side, 50..600, default 420), `POV_BURST` (outward scatter in thousandths of the figure radius, 0..2000, default 450), `POV_Z_DIST` (2000..40000, default 6000), `POV_RATE_XY` / `POV_RATE_XZ` / `POV_RATE_YZ` (milli-revolutions per second, default 7 / 23 / 13), `POV_CELL_W` / `POV_CELL_H` (8, 16) |
| `podracer` | First-person Boonta Eve: two podracer engines hang ahead of you on their cables, flaring and yawing independently as you turn, while an ochre canyon rips past on both sides. One ray per cell column finds the wall; the floor and sky fall out of the ground-plane solve. Arches you fly through, rock spires, slot canyons barely wider than the pod, heat shimmer over the rim, and every so often a rival's engine wash crossing the view. Full repaint — it damages most of the panel every frame, because most of the panel is moving. | `PODRACER_CELL` (px, 4..32, default 8), `PODRACER_SPEED` (course m/s, 40..900, default 300), `PODRACER_FOV` (focal as a percent of panel width, 30..200, default 78), `PODRACER_WIDTH` (canyon half-width in metres, 6..90, default 30), `PODRACER_PINCH` (how far a slot closes, percent, 0..90, default 64), `PODRACER_SPREAD` (engine separation, percent of panel width, 10..90, default 46), `PODRACER_ENGINE` (engine radius, percent of panel width, 3..30, default 7, capped at 22% of the aspect-corrected height), `PODRACER_SHIMMER` (0..100, default 70), `PODRACER_FEATURES` (arches and spires alive at once, 0..24, default 7), `PODRACER_WASH_SECS` (mean seconds between a rival's wash, 0 = off..600, default 9), `PODRACER_SEED` (0 = roll one from the clock and pid; any other value reproduces the run exactly) |
| `speeder` | A first-person speeder-bike chase through the forest moon — enormous redwood trunks rush past at parallax while the bike weaves between them on two incommensurate sines, dappled canopy light streams over the mossy floor, and every so often a fallen trunk sweeps up out of frame to be ducked under or another bike flashes across the view. One spawn in twenty is aimed at where the camera WILL be, so the near misses are deliberate; a trunk moving too fast for the eye to hold an edge on is stippled rather than solid. Its grid is SQUARE, so all of the perspective is in cells and `SAVER_PIXEL_ASPECT` corrects it for free — the opposite choice to `warp`'s. | `SPEEDER_CELL` (px, 4..32, default 8, square), `SPEEDER_SPEED` (metres/sec, 10..300, default 58), `SPEEDER_TRUNKS` (8..400, default 60), `SPEEDER_FOV` (focal as a per-cent of COLUMNS, 20..200, default 62 — smaller is wider and faster-looking), `SPEEDER_HORIZON` (eye line as a per-cent of rows, 10..80, default 44), `SPEEDER_WEAVE` (swing off the path in DECIMETRES, 0..200, default 64; 0 flies straight), `SPEEDER_DAPPLE` (per-cent of the floor in a pool of light, 0..100, default 34), `SPEEDER_LOG_SECS` (mean seconds between fallen trunks, 0..600, default 16; 0 = off), `SPEEDER_RIDER_SECS` (mean seconds between other bikes, 0..600, default 12; 0 = off), `SPEEDER_SEED` (0 = roll one from the clock and pid; any other value reproduces the ride exactly) |

Common: `SAVER_FPS` (1..120, default 30; older spelling `FIRE_FPS`),
`SAVER_ROTATE_SECS` (0..86400, default 0 = off), `SAVER_PIXEL_ASPECT` (25..400,
default 100 = off — see below), `DRM_DEVICE` (default `/dev/dri/card0`),
`RETRY_SECONDS`.

All of these are plain deployment env changes — no image rebuild.

`SAVER` is only the startup choice: the mirror page has a button per saver, and
`POST /select?saver=<name>` does the same thing by hand. An unknown name is a
400 that changes nothing. The switch rebuilds the saver on the render thread and
bumps the mirror's epoch, so viewers reconnect onto the new geometry exactly as
they do for a modeset — and a restart goes back to whatever `SAVER` says.
`SAVER_ROTATE_SECS` works the same way: the page can move it live and a restart
goes back to the env value. See [Rotating on a timer](#rotating-on-a-timer).

### Squashed on pine: `SAVER_PIXEL_ASPECT`

Pine's monitor is a **1280x400** panel that advertises nothing the Pi can read —
its EDID is 0 bytes, the connector comes up as `Unknown-1`, there is no
`/dev/vcio`, and vc4/v3d are not in the Talos image at all. The VideoCore
firmware picks 1920x1080 at boot and that is what simpledrm hands us;
`framebuffer_width` / `framebuffer_height` in `config.txt` were tried on the real
node and measured dead (the evidence is in `cluster/schematic.yaml`).

So the panel rescales our output **non-uniformly**: 1920→1280 is 1.5x, 1080→400
is 2.7x. Everything reaches the glass squashed vertically by 2.7/1.5 = **1.8x** —
a circle is a wide ellipse, a square a wide rectangle.

`SAVER_PIXEL_ASPECT` is that number in per-cent: "one framebuffer pixel is this
much taller than it is wide once the panel has finished rescaling", so **180** on
pine and **100** (the default) everywhere else. It is a **process-wide** knob, not
a per-saver one, and 100 is a byte-for-byte no-op — proved by dumping all 25
savers before and after and diffing the PPMs.

It applies in `Grid::new`, which makes the CELL 1.8x taller. That is the whole
trick: every saver here draws in cells or in braille sub-cells of a cell, so a
cell that is visually square makes the saver's own coordinate space visually
square and 21 of the 25 are corrected without a line of their own. Four are not,
because they measure something in framebuffer PIXELS rather than in cells, and
each carries the stretch explicitly:

| saver            | what needed it                                                                                |
| ---------------- | --------------------------------------------------------------------------------------------- |
| `warp`           | the projection: `focal_y = focal * aspect`, so the tunnel is round                            |
| `podracer`       | the projection: `fy = f * aspect`, so the engines are round and the canyon is not a letterbox |
| `moire`          | the gratings are evaluated in a space `aspect` shorter than the framebuffer                   |
| `toasters{,2,3}` | the 2.5:1 flight diagonal, which is a pixel slope and not a cell slope                        |
| `confetti`       | the 48% pile incline, which is meant to be 48% on the GLASS                                   |

Everything else physical needs nothing and that is not luck: `hardrain`'s wind is
"cells sideways per 100 of fall" and `sakura`'s drift, `rain`, `worms` and the
rest are all in cells or sub-cells, so a taller cell re-leans them by exactly the
right amount. Vertical stays vertical at any aspect.

Two things change as a side effect, both wanted: there are fewer ROWS (1080/29
rather than 1080/16 at the usual 8x16 cell), and a saver sized off "the shorter
panel side" — `pov`, `hypercube` — now measures that side in visually square
units, so the figure comes out round rather than merely fitted.

The mirror sends the FRAMEBUFFER, which is pre-stretched, so `/meta` carries
`pixel_aspect` and the page divides its canvas height by it. The browser then
shows what the wall shows rather than what the renderer drew — the page has no
monitor to do the un-stretching for it. Only the displayed shape changes; the
pixels are the framebuffer's, untouched.

`meta_reports_the_aspect_the_renderer_actually_used` is what keeps the two from
drifting apart: a `/meta` aspect the renderer did not use is invisible in review
and surfaces only as "the web version does not match the screen".

### Actual size in the browser

`SAVER_PANEL_MM` is the panel's **visible glass width in millimetres**, 0 (the
default) meaning nobody has measured it. It is not discoverable — this monitor's
EDID is 0 bytes, so its physical size exists nowhere the Pi can read, and it has
to be typed in by someone with a ruler. Nothing in the render path reads it.

Set it and the mirror page grows three controls, labelled `actual`:

|               |                                                                                                                       |
| ------------- | --------------------------------------------------------------------------------------------------------------------- |
| **ratio**     | the panel's SHAPE, scaled to fit the window. Always exactly right — it needs nothing the browser cannot already know. |
| **size**      | the panel's shape AND physical size, so a saver on the page is the size it is on the wall.                            |
| **calibrate** | teaches the page how big your screen is, which is what makes `size` exact.                                            |

`ratio` and `size` are modes, and the choice is remembered per device. Without
`SAVER_PANEL_MM` the whole group is hidden: `ratio` is the CSS default anyway,
and offering `size` next to it would be offering a button that lies.

**`size` is the default.** CSS cannot supply the other half of the sum — how big
the viewer's own monitor is — because `width: 200mm` is 200mm only when the
browser's CSS inch is a real inch, which on a scaled or HiDPI display it is not.
Uncalibrated, the page uses the nominal 96 CSS px to the inch: within a few per
cent on an unscaled display, well out on a scaled one. The button reads `size ✓`
only once calibrated, because a viewer holding the page up against the real panel
deserves to know the number is a guess before concluding the maths is wrong. If
the result is larger than the window it says `size — too big for this window`
rather than shrinking, since a shrunk actual size is not one.

`ratio` is the escape hatch for exactly that case, and for a small window: the
shape is still exactly the panel's, it just fits.

**calibrate** makes it exact, and can be reopened at any time to re-adjust:
hold a credit card against the screen and drag the bar to match it. An ID-1 card
(ISO/IEC 7810) is 85.60mm to a tenth of a millimetre and is the one ruler
everybody already owns. The canvas resizes live under the slider — you are
matching the panel, not committing blind — and the readout shows the implied dpi.
**reset** drops back to the nominal inch by removing the stored value rather than
overwriting it with today's guess, so a future browser with a better answer is
not permanently overridden.

The px-per-mm lives in `localStorage`, per viewer and per device; it never
reaches the server.

At actual size the canvas deliberately ignores the fit-to-window limits — the
point is a fixed physical size, so a panel bigger than the browser window
overflows and the button says so rather than quietly shrinking it.

### Rotating on a timer

`SAVER_ROTATE_SECS` moves the panel to another saver every N seconds. **0 is
the default and means off**, so a deployment that does not set it behaves as it
always did; anything outside 0..86400 falls back to 0 rather than being clamped,
which is what every other numeric knob here does.

Like `SAVER`, the env var is only the STARTUP value: the mirror page has an
`auto-rotate` tick box and a minutes field, and `POST /rotate?mins=N` does the
same thing by hand (`mins=0` is off, the ceiling is 1440 — a day, same as the
env var). A value that is not a whole number of minutes in range is a 400 that
changes nothing, because a lenient parse of `5x` would turn rotation off, which
is the one outcome nobody asked for. The change takes effect on the next frame,
with no restart, and `/meta` reports the live interval as `rotate_secs` so a
second browser shows what the first one set rather than its own guess. Minutes
on the wire because that is what a person asks for, seconds in `/meta` because
that is the renderer's unit and the env var's; an interval that is not a whole
number of minutes (only reachable from the env var) shows rounded on the page.

Setting the interval **restarts the turn**, even when the number did not change:
asking for five minutes 4:59 into a five-minute turn must buy five minutes, not
one second. What the render loop reads per frame is one relaxed atomic load of a
control word — the interval in its low half, a change counter in its high half,
which is what makes re-asking for the same number count as a change. No lock, no
env lookup, no clock read: see CLAUDE.md on the frame loop.

`SAVER` still picks the STARTING saver — rotation moves on from there. The order
is a **shuffled bag**: every saver, in random order, none of them again until all
of them have been shown, then reshuffled. That is what "rotate through all the
savers" has to mean — rolling an independent choice each time takes about 95
turns to show you all 25 (coupon collector), eight hours at a five-minute
interval, where the bag takes exactly 25 and two hours.

A bag is not a walk down the table: a walk is predictable in the wrong way (the
same saver always follows the same saver, and the three toaster variants are
adjacent, so a walk shows them back to back to back), and a bag is reshuffled
every cycle. "It never shows the same saver twice in a row" stays a property of
the code rather than a probability — inside a bag the entries are distinct, and
at the boundary between two bags the refill swaps the top entry away if it is
the saver still on screen, rather than re-shuffling until it looks right. The bag
is a fixed-size array sized from the table, shuffled in place, so nothing
allocates. Every saver gets the
same length turn; there is no per-saver table of seconds, because the expensive
ones hold the target fps on this panel and so there is nothing to compensate for.

Clicking a saver on the mirror page **restarts the interval**, so a manual pick
always gets a whole turn rather than the two seconds that happened to be left.
It does not pause rotation: a pause needs a resume, which is a second knob plus
a page that has to show which mode it is in, to save someone setting this to 0.

A rotation is the same event as a click from `saver::switch` down, including the
epoch bump — so every connected viewer's stream ends, it re-reads `/meta` and
takes a keyframe. That is one keyframe per viewer per interval (32 KB for
`matrix`, 1 MB for `blocks`, which is the widest grid here), on the viewer's own
thread, and it is the cost a click has always had. The page used to sit out its
two-second reconnect backoff and show an error banner on a stream that ended
without a click; it now reconnects immediately and silently, because with this
knob on that is a routine event rather than a fault.

### About the pov saver

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

### About the doodles saver

The point is that it reads as hand-drawn rather than mathematical, which is the
whole difference from `lissajous`. Lissajous evaluates a closed-form curve —
position is a function of t, the figure is periodic, and it looks like
mathematics because it is. A doodle pen carries a heading and a turn rate, and
only the TURN RATE is driven, by a damped random walk. Position is the double
integral of noise: the line commits to an arc, curls out of it and wanders off,
and no frame of it can be reproduced from a formula.

Two knobs bracket the degenerate ends, and `DOODLES_INERTIA` is the one that
matters: too little and the heading is uncorrelated between steps, which is a
fuzzy blob rather than a line; too much and the turn rate never changes sign,
which is a circle retracing itself forever. The default 960 puts the turn rate's
correlation at about 25 steps and the typical radius at ~17 sub-cells, so
curvature persists for roughly one loop's arc.

`DOODLES_SPEED` is quoted for a 1080p panel and is scaled down with panel AREA
(to a floor of a third). A doodle lasts as long as the pen needs to fill the
sheet, so without that the same hand speed finishes a 1280x400 panel four times
sooner — measured 5s against 19s. With it, a doodle is ~16s at 1280x400 and
~20s at 1920x1080.

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

### About the life saver

Conway's Life is a bad screensaver by default: B3/S23 on a random soup burns
brightly for two hundred generations and then settles into blocks and blinkers
that never change again. Measured on this board at 240x135, churn falls from 49
changes per thousand cells per generation at generation 100 to 9 by generation
2000 and stays there for the next eighteen thousand — a torus keeps a few
gliders circulating, so the population never actually hits zero, which is why
"is anything alive" is not a test of anything.

The rules stay exact; the fix sits outside them. `step` already counts births
and deaths, and that one number catches all three ways a board gets boring —
still lifes churn zero, blinker fields churn a handful, and a nearly-black board
with one glider on it churns ten. A rolling board hash, the textbook stagnation
detector, sees the first two and is blind to the third. So when churn stays
under `LIFE_QUIET` per mille for `LIFE_PATIENCE` consecutive generations, one
disc of fresh soup lands at a random spot — the same `meteor` that seeds the
board at startup, so the panel never shows a transition it did not show at
second zero. At the defaults that settles into an equilibrium of 31 changes per
thousand cells per generation and 52 cells per thousand alive, flat from
generation 600 out to 20000 on both panel shapes, at about one meteor every ten
seconds. With the injector disabled the same measurement over eight seeded
boards reads 6 and 3 per mille; `it_never_dies_down` asserts a floor of 15.

Edges wrap. A dead border is a permanent absorber — every glider that reaches it
dies, and after an hour they all have.

Colour is age: white-hot at birth, cooling through amber and violet to a settled
blue after eight generations, with a dark-red ash that fades for `LIFE_FADE`
generations behind anything that dies. Both come free out of a pass that visits
every cell anyway, and they are what makes an age-saturated still life read as
debris rather than as part of the action.

`LIFE_GPS` paces generations independently of `SAVER_FPS` because Life at 30
generations a second is unreadable. A frame with no generation in it costs one
u32 compare per cell and reports no damage.

### About the marble saver

`image/src/marble.rs` is Atari's Marble Madness, not a marble run: an isometric
course seen from a fixed three-quarter view, a marble worked down it by a very
simple autopilot, and the void underneath everything.

**The projection is the whole look, and it is computed in GLASS units.** A tile
is `gx = (x - y) * tw`, `gy = (x + y) * tw/2 - z * zs` — a diamond exactly twice
as wide as it is tall. That 2:1 has to hold **on the panel**, not in the
framebuffer, and `SAVER_PIXEL_ASPECT=180` is precisely the difference between
the two. So there is one conversion from glass to sub-cells:

```text
ux = cell_w / 2                       px per sub-cell across
uy = cell_h / 4 * 100 / pixel_aspect  px per sub-cell down, un-stretched
```

`uy` divides the aspect back out — at 100 it is a no-op, at 180 it is the only
thing keeping the diamonds 2:1. `diamonds_are_two_to_one_on_the_glass_at_both_aspects`
measures the drawn tile, in glass units, at both aspects; it does not assert the
constants, because the constants are right in both worlds and the bug is not.

Tiles are painted back to front by increasing `x + y`, each a top diamond plus a
skirt down its two lower edges to whatever the neighbour's height is. The skirt
is what turns a heightfield into cliffs and catwalks instead of a flat mosaic.
The camera follows the marble, so most frames move every tile on the panel and
the damage model is a full repaint through `Grid::flush`.

**Courses are generated, then validated, then rejected.** A route of descending
straight segments is carved first — decks one to five tiles wide, some walled,
some slick and unwalled — and the geometry is built around it. Five criteria:
a flood fill from the start that may drop any distance but never climb more than
`CLIMB` must reach the goal (acid counts as solid, so the dry line past a pool
has to exist); every hazard's tile must border that reachable set; the route's
bounding box must span at least 11 tiles in both axes and descend at least 9
height units; the deck must be between 60 tiles and a third of the field; and
then the candidate is handed to a **physics probe** that runs the same step and
the same autopilot, hazards off, and must reach the goal. Measured over 400
candidates at 1080p: 359 pass the first four, 316 of those pass the probe —
about 1.3 candidates per accepted course, with the goal-unreachable check and
the probe doing essentially all the rejecting. Fourteen candidates in, the last
one ships anyway: a headless pod must never stall the frame loop over taste.

**Progress, not churn, is the stagnation measure.** `advance` already knows the
furthest waypoint reached. A fall or a hazard respawns the marble at the last
checkpoint; `MARBLE_PATIENCE` steps with no progress at all respawns it with a
shove; and a respawn that fails to beat the previous one four times running
blames the course and regenerates it. Everything that respawns routes through
one function, so there is one place to get that ladder right. Measured over
60 000 steps with the defaults: about 133 goals, 141 falls, 104 hazard deaths
and 19 no-progress respawns, of which roughly 15 escalate to a new course —
that is one fall per goal, which is what "someone playing reasonably well and
occasionally losing it" measures out as.

Two numbers in there were found the hard way. A hunter marble that chases
without a leash follows you the length of the course and shoves you off the same
catwalk forever: 40 falls per goal, measured. And a checkpoint beside an acid
pool is an infinite death loop — 1 992 deaths in 60 000 steps on one seed —
which is why a respawn buys 45 steps of grace.

Tunnelling is swept, not capped by inspection: a step is split into
`ceil(speed / 0.22)` substeps so nothing moves more than 0.22 of a tile at a
time, against a one-tile catwalk, and the per-step speed is clamped to the
substep budget independently of `MARBLE_SPEED` and of `SAVER_FPS`. Walls block
by tile KIND as well as by height, because `height` deliberately refuses to
blend into a wall — without that the bilinear smoothing builds a ramp up the
side of the thing that is there to stop you, and a fast marble simply drives
over it.

Cost: 342 us/frame at 1920x1080 with `SAVER_PIXEL_ASPECT=180`, against `moire`
at 277 in the same process — 1.23x the most expensive saver measured, which
puts it around 170m of the 500m limit. Most of that is the fill: the camera
moves, so every visible tile is redrawn every frame.

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

### About the tactiles saver

`image/src/tactiles.rs` reports the WHOLE panel as damaged every frame, and
that is not a bug on the list above. The three waves cross the entire tile
grid, so every 48px band of scanlines has some tile flipping in it and the runs
merge into one. What stays small is the blit: about 1% of cells a frame, since
`Grid::flush` skips every cell whose packed `Cell` did not change. Measured
interleaved against `moire` at 1920x1080, it costs 0.76x `moire` per frame.

The glyphs are baked once in the constructor — sixteen shape-by-rotation
bitmaps rasterised into braille cells — so the frame loop is four array reads
and a multiply per cell, with the trig per TILE rather than per cell.

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

This one draws through `Grid::flush`, not the sparse path every other
object-based saver uses, and that is measured rather than assumed. It ran on
`flush_sparse` with a hand-maintained dirty list and paid MORE than the
whole-grid diff it was avoiding: 4237 indices pushed per frame, sorted
(`k log k`, the largest single cost in the renderer), deduped to 2701, and most
of those blitted twice, because `flush_sparse` blits its whole list
unconditionally while the clear pass had just written a cell the stamp pass
immediately rewrote with the same value. Clear the grid, stamp the flock,
`flush`: 0.0799 → 0.0299 ms/frame, and 737 → 676 median damaged scanlines of
the 1072 the grid owns (against `toasters`' 672 of 1056, and 1056 for `ascii`
and `matrix`, which repaint everything every frame).

The second half of the trade is not speed. A hand-maintained dirty list can
under-report, and a cell written but left out of it keeps its old pixels on the
panel forever; damage derived by diffing cannot.

### About the strings saver

`image/src/strings.rs` is After Dark's "String Theory": a polygon whose corners
each bounce around the panel on their own heading, redrawn every frame while the
outlines behind it fade, so the stack reads as one ribbon sweeping and folding
through space. Three independent ribbons, one hue each.

- **The trail is a fading heat buffer, not a ring of polygons.** The classic
  keeps N outlines and erases the oldest as it draws the newest, which needs the
  old vertices kept, the old lines re-walked to erase, and — because ribbons
  overlap — an erase that cannot simply write black. A per-cell brightness that
  decays does all three: a cell's age IS its colour, an overlap is the newer
  stamp winning, and nothing is re-walked.
- **`STRINGS_SPEED` is in SUB-CELLS per second, not a fraction of the panel**,
  and that is what makes this look right at 1920x1080 and at 1280x400. What
  separates the outlines — and so what makes a ribbon read as strings rather
  than as a solid sheet — is the per-frame step in sub-cells and nothing else.
  Scaled to the panel, the 1280x400 one has 2.7x fewer sub-cells down its short
  edge, gets 2.7x less separation, and fills in. The first cut of this did scale
  it, and rendered coloured sheets.
- **`STRINGS_FADE_MS` is the cost knob, and it runs the wrong way round.** A
  cell steps down eight brightness levels over its life, and a level step is a
  re-blit; a SHORTER fade means more steps per frame, not fewer. Measured
  against `lissajous` in the same process: 3.3x at the default 1000 ms, 2.8x at
  2000, 2.4x at 4000. It is 1000 anyway, because a 4000 ms ribbon saturates the
  400-tall panel.
- **The dots carry a freshness guard.** A cell whose heat is below "what a cell
  stamped last frame would have after this frame's decay" has its braille dots
  cleared rather than OR-ed into. Without it, a nearly-faded crossing keeps the
  old pass's dots, which are then redrawn in the new pass's colour lying ACROSS
  the new line — phantom ticks at every crossing. `lissajous` shipped that bug;
  `a_faded_pass_leaves_no_dots_in_a_cell_a_later_pass_relights` pins the fix.

Like `lissajous` it draws through `Grid::flush` and repaints most of the panel
every frame (median 960 of 1080 damaged scanlines at 1920x1080, 400 of 400 at
1280x400). That is not an oversight: the trail fades, so every lit cell changes
colour on the frame it steps down a level, and "what changed" is most of the
trail. Interleaved against the other savers on one machine it measures the same
per frame as `matrix`: 122 vs 116 us over 4,000 frames, 3.8x and 3.6x
`lissajous`. Those are ratios on a laptop and do not convert to milli-cores on
the Pi — `matrix` is the saver to compare it to there, not a number derived from
`lissajous`'s 87m.

### About the hardrain saver

`image/src/hardrain.rs` is the storm; `image/src/rain.rs` is the drizzle. They
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

### About the zot saver

`image/src/zot.rs` is a bolt and then a gap, and the gap is most of it: the
saver's whole shape is the duty cycle. What it does with the lit part is fire
the SAME channel more than once.

- **A bolt is one channel and two or three flashes.** `strike` rasterises the
  branching tree once and then only runs a brightness envelope over it; when
  that envelope ends, `ZOT_RESTRIKE_PCT` (70) decides whether the channel
  lights again — dimmer (72% of the last peak) and quicker (80% of its length)
  — up to three strokes. Re-lighting the geometry is what makes it read as one
  bolt flashing, where a second `strike` would be two separate bolts in a row.
  Real flashes do exactly this, which is what "return stroke" means.
- **`ZOT_STROKE_GAP_MS` (110) is the beat between those flashes**, and it is
  the knob that decides whether they are one bolt or two. Three frames of dark
  channel at 30fps, with the afterglow wash still up across the panel — long
  enough to read as a distinct flash, too short to read as a new strike.
  `ZOT_GAP_MIN_MS` / `ZOT_GAP_MAX_MS` (700 / 2400) are the real gap, an order
  of magnitude longer; `a_bolt_flashes_again_down_the_same_channel` pins the
  two apart.
- **The channel decays as `1 - t²`, not `1 - t`.** A linear fall spends its
  last third in the bottom two steps of a six-step ramp, which on an 8x16 cell
  is a thread you cannot see: measured peak luma stepped 249 → 178 → 118 → 70 →
  35 → 11 → 3, and everything from 35 down is frames spent on nothing.
  Squaring holds the channel in the top half of the ramp for most of the stroke
  and then drops it off a cliff. Same frame budget, and over a 900-frame dump
  it moved frames at peak luma ≥ 64 from 6% to 16%.
- **Holding ONE bolt longer was the other candidate, and it looks wrong.**
  `ZOT_BOLT_MS=1000` with no re-strike renders the same static geometry for
  thirty frames while the strobe mask blinks it; it reads as a stuck image,
  not as lightning. Rendered side by side before choosing — the knob is still
  there if you disagree.

Measured over 900 frames at 1920x1080 with `ZOT_SEED=424242`, before the
re-strike and after: frames with anything on the panel 16% → 35%, frames at
peak luma ≥ 64 (the bolt genuinely visible rather than a trace of wash) 6% →
16%. Gaps went from 27-105 frames of black to 21-72, and the panel is still
black for 65% of the run — that is the point of the saver, and
`the_panel_goes_dark_between_bolts` holds the floor.

Nothing in the frame path changed to get any of it: the envelope is a handful
of integer ops per FRAME, the geometry is generated once per bolt, and a
re-strike reuses the buffers a strike already filled.

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

Damage is **rectangles**, not scanline bands: a run carries `x0..x1` as well as
`y0..y1`, taken from the exact cell `Surface::cell_rows` is handing out. A
224 px toaster used to report 1920 px-wide scanlines and the shadow copy moved
8.5x the pixels that changed; it now copies 0.14 Mpx a frame instead of 1.15.
Full-repaint savers are unaffected — their rects are the panel either way — so
**pixels, not damaged scanlines, is the number to read.** Two objects at
opposite ends of the panel touch every scanline between them and still copy
almost nothing, which is why the scanline tables further up understate how
cheap the sparse savers now are.

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

`POST /rotate?mins=N` sets the rotation interval; `GET /stat` is the live counters — `{"overruns":N,"viewers":N,"fps":N}`.
`overruns` is frames that ran past the frame budget, which is what a raised
`SAVER_FPS` against the pod's 500m CFS quota shows up as: the render loop is
stopped mid-period and runs a burst, and the burst is visible stutter on the
panel. It is a separate route on purpose — `/meta` is a string cached at
modeset, so a counter baked in there would report its value as of the last
modeset forever.

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
any pixel that changed without being inside a reported rect exits non-zero — per
pixel, not per scanline, because a rect one column too narrow freezes a vertical
band exactly as a missing scanline freezes a horizontal one. That is the one bug
a monitor cannot help with and a laptop can.

`<dir>/damage.txt` gets a line per frame with the reported rects and the pixel
count they imply — the headless read on whether a saver is quietly flushing the
whole panel.

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
| Image        | `busybox:1.36`, `ghcr.io/northisup/screensaver@sha256:032ef4a615fb61cf290340b145c1ddfa96c521cb3ba1d53b7314f43045fe3b51`, `nginxinc/nginx-unprivileged:1.27-alpine` |
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
