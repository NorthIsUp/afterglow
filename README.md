# afterglow — HDMI screensavers on whichever Pi5 holds the monitor

A deliberately thin workload that paints an animation onto the HDMI display of
the Talos Pi5 carrying the `hardware.homelab/display: "true"` label. Renderer:
a static musl Rust binary on a `FROM scratch` image, ~230 KB, which writes
pixels straight into a DRM/KMS dumb buffer. It grew up inside the author's
private homelab GitOps repo, which still deploys it; `homelab-gitops#N` in
older commit messages refers to that repo's PRs.

## Install

```sh
docker pull ghcr.io/northisup/afterglow:latest   # linux/arm64; also :sha-<commit>
```

It needs a node with a monitor on HDMI, `/dev/dri/card0`, and a privileged
container. [`examples/deployment.yaml`](examples/deployment.yaml) is a minimal
Kubernetes deployment: the privileged container, the `release-fbcon`
initContainer that takes the panel from the kernel console, and a display
`nodeSelector`. Pick a saver with the `SAVER` env var, below. The web mirror on
`SAVER_HTTP` has no auth of its own; put it behind yours.

## Run it in your terminal

No Pi, no monitor, no Docker — any saver animates in a truecolor terminal
(Terminal, iTerm2, Ghostty, kitty, most Linux ones):

```sh
SAVER=night-coast SAVER_TERM=1 cargo run --release
```

`q` or Ctrl-C quits. The saver is sized as if each character were an 8x16
glyph cell: glyph-shaped cells print one character each (matrix prints its
katakana), square ones stack two to a character with `▀`. Resize the window
and it rebuilds; shrink the font if a scene's edges are cropped. The web
mirror still runs on `SAVER_HTTP`, so `POST /select` and `SAVER_ROTATE_SECS`
work here too; stderr is muted while it draws.

## Looking at a saver without a monitor

`SAVER_DUMP` renders to PPM files and exits, on any machine, with no display:

```sh
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

## Choosing a saver

Pick one with `SAVER` (older spelling: `FIRE_STYLE`). Anything unrecognised
falls back to `ascii` — a headless pod must never crash-loop on a typo.

Common: `SAVER_FPS` (1..120, default 30; older spelling `FIRE_FPS`),
`SAVER_ROTATE_SECS` (0..86400, default 0 = off), `SAVER_ROTATE_EXCLUDE`
(comma-separated savers rotation skips, default none), `SAVER_PIXEL_ASPECT` (25..400,
default 100 = off — see [`SAVER_PIXEL_ASPECT`](docs/pixel-aspect.md)), `DRM_DEVICE` (default `/dev/dri/card0`),
`RETRY_SECONDS`.

All of these are plain deployment env changes — no image rebuild.

`SAVER` is only the startup choice: the mirror page lists every saver, and
`POST /select?saver=<name>` does the same thing by hand. An unknown name is a
400 that changes nothing. The switch rebuilds the saver on the render thread and
bumps the mirror's epoch, so viewers reconnect onto the new geometry exactly as
they do for a modeset — and a restart goes back to whatever `SAVER` says.
`SAVER_ROTATE_SECS` works the same way: the page can move it live and a restart
goes back to the env value. See [Rotating on a timer](docs/rotation.md).

Each saver's own knobs are on its page, and the mirror page can change any of
them live — see [Live settings](docs/mirror.md#live-settings-config).

## Savers

| `SAVER`                                          | What                                                                                                                                                                                                                                    |
| ------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [`ascii`](docs/savers/fire.md#ascii)             | Doom fire as an ASCII ramp (`" .:-=+*#%@"`), one heat sample per character cell, coloured by the 37-step fire palette.                                                                                                                  |
| [`blocks`](docs/savers/fire.md#blocks)           | The same fire drawn as chunky pixels — a solid glyph per cell.                                                                                                                                                                          |
| [`matrix`](docs/savers/matrix.md)                | Digital rain.                                                                                                                                                                                                                           |
| [`toasters`](docs/savers/toasters.md#toasters)   | Flying toasters, after After Dark's.                                                                                                                                                                                                    |
| [`toasters2`](docs/savers/toasters.md#toasters2) | The same flock in BLOCK ELEMENTS at 8x16 cells, so the olive chassis is a solid fill rather than edge strokes.                                                                                                                          |
| [`toasters3`](docs/savers/toasters.md#toasters3) | The same flock drawn with a BRAILLE-style 2x4 dot matrix per cell, so one 16x6-cell toaster is a 32x24 bitmap — real slot openings, a dial, a lever, barbed wings.                                                                      |
| [`dvd`](docs/savers/dvd.md)                      | The bouncing DVD logo, and the corner hit it exists for.                                                                                                                                                                                |
| [`lissajous`](docs/savers/lissajous.md)          | A point tracing `x = sin(a·t + d)`, `y = sin(b·t)` with a fading trail, morphing through its family of figures.                                                                                                                         |
| [`satori`](docs/savers/satori.md)                | A slow colour-field composition: the panel is split into a handful of rectangles, each holding one muted tone, all of it moving far slower than a glance.                                                                               |
| [`warp`](docs/savers/warp.md)                    | Flying forward through a starfield.                                                                                                                                                                                                     |
| [`sakura`](docs/savers/sakura.md)                | A cherry tree at night, shedding blossom on a slow wind.                                                                                                                                                                                |
| [`fractal`](docs/savers/fractal.md)              | Escape-time fractals: nineteen families in rotation, each zooming continuously into a point known to sit on its boundary.                                                                                                               |
| [`hypercube`](docs/savers/hypercube.md)          | A rotating, inverting tesseract wireframe.                                                                                                                                                                                              |
| [`moire`](docs/savers/moire.md)                  | Moire interference from overlapping line families.                                                                                                                                                                                      |
| [`rain`](docs/savers/rain.md)                    | Falling streaks over black, in three depth tiers, with wind and a splash on the ground row.                                                                                                                                             |
| [`worms`](docs/savers/worms.md)                  | Segmented crawlers wandering a toroidal grid, head bright, body trailing behind it in alternating light and dark bands.                                                                                                                 |
| [`confetti`](docs/savers/confetti.md)            | Pieces flutter down, land, and pile up like sand.                                                                                                                                                                                       |
| [`city`](docs/savers/city.md)                    | The After Dark night skyline — lit windows on a black silhouette, scattered stars, a beacon on the tallest tower and the odd shooting star.                                                                                             |
| [`life`](docs/savers/life.md)                    | Conway's Game of Life on a toroidal board, cells coloured by age — white-hot at birth, cooling to blue — with a fading ash trail.                                                                                                       |
| [`doodles`](docs/savers/doodles.md)              | After Dark's scribbler: pens wander the panel leaving one continuous freehand line each, looping back over themselves until the sheet is full, then it fades and a new one starts.                                                      |
| [`strings`](docs/savers/strings.md)              | "String Theory", the After Dark module — a polygon whose corners each bounce independently, redrawn every frame over the fading outlines behind it, so a ribbon of lines sweeps and folds.                                              |
| [`tactiles`](docs/savers/tactiles.md)            | After Dark's TacTiles — a grid of square tiles carrying one geometric glyph each (bar, diagonal, corner, arc).                                                                                                                          |
| [`pov`](docs/savers/pov.md)                      | Points of View — a rotating platonic solid drawn as dots on its own surface, bursting into the next of the five every ten seconds.                                                                                                      |
| [`podracer`](docs/savers/podracer.md)            | First-person Boonta Eve: two podracer engines hang ahead of you on their cables, flaring and yawing independently as you turn, while an ochre canyon rips past on both sides.                                                           |
| [`speeder`](docs/savers/speeder.md)              | A first-person speeder-bike chase through the forest moon, weaving between enormous redwood trunks.                                                                                                                                     |
| [`marble`](docs/savers/marble.md)                | Marble Madness — an isometric course floating in black space, a chrome marble rolling down it on autopilot, and hazards trying to stop it.                                                                                              |
| [`xwing`](docs/savers/xwing.md)                  | The Death Star run from the cockpit, in three acts on a loop: the station swelling out of a starfield, a low pass over its greebled surface, then the trench — walls closing in and the targeting computer swinging down over the view. |
| [`hardrain`](docs/savers/hardrain.md)            | A downpour: steeply slanted streaks under a gusting wind, a mist the sky is veiled in, squalls sweeping across, and standing water at the bottom that ripples where the rain lands.                                                     |
| [`zot`](docs/savers/zot.md)                      | Lightning.                                                                                                                                                                                                                              |
| [`plasma`](docs/savers/plasma.md)                | The demo-scene plasma, full screen: four sine fields summed into soft blobs of density, drawn with ascii.rest's ramp `.,-~:;=+*#%@` at the panel's own resolution, any size or shape.                                                   |

### ascii.rest halftone scenes

Ports of [ascii.rest](https://ascii.rest)'s scenes; the camera slowly tours each one. Every scene also has a `-wide` variant recomposed at 3.2:1. The shared engine, the tour and its knobs: [the ascii.rest ports](docs/ascii-rest.md).

| `SAVER`                                           | `-wide`                                                                    | What                                                                                                     |
| ------------------------------------------------- | -------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| [`alpine-dawn`](docs/savers/alpine-dawn.md)       | [`alpine-dawn-wide`](docs/savers/alpine-dawn.md#alpine-dawn-wide)          | Jagged snow peaks catch the first pink light on their east faces while their flanks stay in blue shadow. |
| [`aurora-fjord`](docs/savers/aurora-fjord.md)     | [`aurora-fjord-wide`](docs/savers/aurora-fjord.md#aurora-fjord-wide)       | Curtains of aurora ripple over a fjord between snowy mountains.                                          |
| [`deep-reef`](docs/savers/deep-reef.md)           | [`deep-reef-wide`](docs/savers/deep-reef.md#deep-reef-wide)                | Looking along a coral reef from a few metres down.                                                       |
| [`desert-night`](docs/savers/desert-night.md)     | [`desert-night-wide`](docs/savers/desert-night.md#desert-night-wide)       | A moonless desert under the milky way.                                                                   |
| [`earthrise`](docs/savers/earthrise.md)           | [`earthrise-wide`](docs/savers/earthrise.md#earthrise-wide)                | The Earth coming up over the lunar horizon.                                                              |
| [`kyoto-dusk`](docs/savers/kyoto-dusk.md)         | [`kyoto-dusk-wide`](docs/savers/kyoto-dusk.md#kyoto-dusk-wide)             | A five-storey pagoda at dusk over a temple pond, a cherry tree in bloom lit by a stone lantern.          |
| [`marine-drive`](docs/savers/marine-drive.md)     | [`marine-drive-wide`](docs/savers/marine-drive.md#marine-drive-wide)       | The Queen's Necklace seen from Malabar Hill at night.                                                    |
| [`misty-forest`](docs/savers/misty-forest.md)     | [`misty-forest-wide`](docs/savers/misty-forest.md#misty-forest-wide)       | Morning in a pine forest.                                                                                |
| [`night-coast`](docs/savers/night-coast.md)       | [`night-coast-wide`](docs/savers/night-coast.md#night-coast-wide)          | A lighthouse on a wooded headland under moonlit clouds.                                                  |
| [`ocean-sunset`](docs/savers/ocean-sunset.md)     | [`ocean-sunset-wide`](docs/savers/ocean-sunset.md#ocean-sunset-wide)       | Golden hour at sea.                                                                                      |
| [`storm-plains`](docs/savers/storm-plains.md)     | [`storm-plains-wide`](docs/savers/storm-plains.md#storm-plains-wide)       | An anvil thunderhead at dusk over open wheat country.                                                    |
| [`taj-dawn`](docs/savers/taj-dawn.md)             | [`taj-dawn-wide`](docs/savers/taj-dawn.md#taj-dawn-wide)                   | The Taj Mahal at first light, seen down its long reflecting canal between rows of cypress.               |
| [`varanasi-ghats`](docs/savers/varanasi-ghats.md) | [`varanasi-ghats-wide`](docs/savers/varanasi-ghats.md#varanasi-ghats-wide) | Dusk on the Ganga.                                                                                       |

### ascii.rest character pieces

One ink each. Every piece also has a `-wide` twin drawn at the panel's own size, any shape, with no bars. The shared engine: [the ascii.rest ports](docs/ascii-rest.md#character-pieces).

| `SAVER`                                                   | `-wide`                                                     | What                                                                            |
| --------------------------------------------------------- | ----------------------------------------------------------- | ------------------------------------------------------------------------------- |
| [`aurora`](docs/savers/aurora.md)                         | [`aurora-wide`](docs/savers/aurora.md#aurora-wide)          | A curtain of light over a spruce treeline at night.                             |
| [`synthwave`](docs/savers/synthwave.md)                   | [`synthwave-wide`](docs/savers/synthwave.md#synthwave-wide) | The eighties horizon.                                                           |
| [`tv-static`](docs/savers/tv-static.md)                   |                                                             | An old set showing snow, with a hum bar rolling through it.                     |
| [`vinyl`](docs/savers/vinyl.md)                           |                                                             | A record turning on a turntable, seen from above.                               |
| [`lighthouse`](docs/savers/lighthouse.md)                 |                                                             | A banded lighthouse on a heap of rocks at night.                                |
| [`fractal-tree`](docs/savers/fractal-tree.md)             |                                                             | A trunk that forks seven times over into lobes of leaves, bending in the wind.  |
| [`reaction-diffusion`](docs/savers/reaction-diffusion.md) |                                                             | A Gray-Scott reaction whose spots on the left give way to stripes on the right. |
| [`double-pendulum`](docs/savers/double-pendulum.md)       |                                                             | Two equal rods hung end to end from one pivot, stepped with RK4.                |

## Docs

- [How it works](docs/how-it-works.md) — DRM, damage rectangles, the source layout.
- [The web mirror](docs/mirror.md) — the page, what crosses the wire, `/stream`, `/meta`, `/stat`, live settings (`/config`), actual size in the browser.
- [`SAVER_PIXEL_ASPECT`](docs/pixel-aspect.md) — correcting pine's non-uniformly scaled panel.
- [Rotating on a timer](docs/rotation.md) — `SAVER_ROTATE_SECS` and the shuffled bag.
- [The ascii.rest ports](docs/ascii-rest.md) — the shared engine, fit, the scene tour, golden tests.
- [The glyph table](docs/glyph-table.md) — the generated `src/font.rs`.
- [Building and publishing the image](docs/building.md)
- [Debugging](docs/debugging.md)
- [Gotchas](docs/gotchas.md)
- [Third-party notices](THIRD_PARTY.md)
