<div align="center">

# afterglow

**Put something fun on your homelab's display.**

Fire, rain, flying toasters, a lighthouse, a turntable and 50-odd more ·
one ~230 KB container · no X, no browser

[![build](https://github.com/NorthIsUp/afterglow/actions/workflows/image.yml/badge.svg)](https://github.com/NorthIsUp/afterglow/actions/workflows/image.yml)
[![license: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![image](https://img.shields.io/badge/ghcr.io-northisup%2Fafterglow-2496ED?logo=docker&logoColor=white)](https://github.com/NorthIsUp/afterglow/pkgs/container/afterglow)

<img src="docs/media/tour.gif" width="100%" alt="Two seconds of every saver, with a channel change between each">

<sub>Two seconds of every saver, flipping channels.</sub>

</div>

Got a Pi with a little screen bolted to the rack, or an old monitor hanging
off a node, showing a login prompt nobody reads? Give it a screensaver.
afterglow runs as one small container on whichever machine holds the HDMI
display and paints straight into the kernel's framebuffer (DRM/KMS), so it needs
no desktop, sips CPU, and leaves the box to its real job.

- **50-odd savers**: Doom fire, digital rain, flying toasters, DVD bounce, warp,
  plasma, reaction-diffusion, a double pendulum, a turntable whose arm plays a
  side, a lighthouse sweeping its beam, and ports of the
  [ascii.rest](https://ascii.rest) scenes. Each has a clip on
  [its page](#savers).
- **A web page to drive it**: a live mirror of the screen, pick a saver, tweak
  its settings, and rotate through them on a timer.
- **Try it first in a terminal**: no Pi needed, [below](#run-it-in-your-terminal).

It grew up in the author's own homelab (a Talos Pi 5 cluster, which still
deploys it; `homelab-gitops#N` in older commit messages refers to that repo).
The clips are GIFs in Git LFS; `mise run media` regenerates them
([`tools/media.py`](tools/media.py)).

## Install

```sh
docker pull ghcr.io/northisup/afterglow:latest   # linux/arm64; also :sha-<commit>
```

`:latest-doom` (and `:sha-<commit>-doom`) is the same image plus the
[`doom`](docs/savers/doom.md) saver and Freedoom. It compiles in GPL-2.0 code,
so that image is GPL as a whole; the default image is MIT.

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

The dump also drives the **web mirror** on `SAVER_HTTP` (default
`127.0.0.1:8080`), so the same saver shows in a browser with no card at all —
the local preview, and the one way the mirror is testable off the hardware.
Pass a large `SAVER_DUMP_FRAMES` and it runs at `SAVER_FPS` indefinitely.
`SAVER_HTTP=off` binds nothing and dumps as fast as it can render.

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
| [`chess`](docs/savers/chess.md)                  | Two engines playing each other, a game per board — two side by side on 3.2:1 — with gliding pieces, an eval bar, captured pieces and a scrolling score sheet.                                                                           |
| [`tetris`](docs/savers/tetris.md)                | Falling blocks played by an AI, as many wells side by side as the panel's shape holds — four on pine — each its own game to the kill screen.                                                                                            |
| [`maze-chase`](docs/savers/maze-chase.md)        | A Pac-Man-style maze chase on autopilot: four ghosts with the classic personalities, in a maze generated to fill the panel, new every level.                                                                                            |
| [`doom`](docs/savers/doom.md)                    | Freedoom on autopilot, one widescreen game on a random map, its field of view sized to the panel. **Only in the `-doom` image, which is GPL** (see the page).                                                                           |

### ascii.rest halftone scenes

Ports of [ascii.rest](https://ascii.rest)'s scenes, each recomposed for the panel's own shape, from pine's 3.2:1 to portrait; `ASCII_REST_TOUR=1` adds a slow camera tour. The shared engine, the tour and its knobs: [the ascii.rest ports](docs/ascii-rest.md#scenes).

| `SAVER`                                           | What                                                                                                     |
| ------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| [`alpine-dawn`](docs/savers/alpine-dawn.md)       | Jagged snow peaks catch the first pink light on their east faces while their flanks stay in blue shadow. |
| [`aurora-fjord`](docs/savers/aurora-fjord.md)     | Curtains of aurora ripple over a fjord between snowy mountains.                                          |
| [`deep-reef`](docs/savers/deep-reef.md)           | Looking along a coral reef from a few metres down.                                                       |
| [`desert-night`](docs/savers/desert-night.md)     | A moonless desert under the milky way.                                                                   |
| [`earthrise`](docs/savers/earthrise.md)           | The Earth coming up over the lunar horizon.                                                              |
| [`kyoto-dusk`](docs/savers/kyoto-dusk.md)         | A five-storey pagoda at dusk over a temple pond, a cherry tree in bloom lit by a stone lantern.          |
| [`marine-drive`](docs/savers/marine-drive.md)     | The Queen's Necklace seen from Malabar Hill at night.                                                    |
| [`misty-forest`](docs/savers/misty-forest.md)     | Morning in a pine forest.                                                                                |
| [`night-coast`](docs/savers/night-coast.md)       | A lighthouse on a wooded headland under moonlit clouds.                                                  |
| [`ocean-sunset`](docs/savers/ocean-sunset.md)     | Golden hour at sea.                                                                                      |
| [`storm-plains`](docs/savers/storm-plains.md)     | An anvil thunderhead at dusk over open wheat country.                                                    |
| [`taj-dawn`](docs/savers/taj-dawn.md)             | The Taj Mahal at first light, seen down its long reflecting canal between rows of cypress.               |
| [`varanasi-ghats`](docs/savers/varanasi-ghats.md) | Dusk on the Ganga.                                                                                       |

### ascii.rest character pieces

Each draws in colour at the panel's own size and shape, recomposed for it rather than stretched, with no bars. The shared engine: [the ascii.rest ports](docs/ascii-rest.md#character-pieces).

| `SAVER`                                                   | What                                                                            |
| --------------------------------------------------------- | ------------------------------------------------------------------------------- |
| [`aurora`](docs/savers/aurora.md)                         | A curtain of light over a spruce treeline at night.                             |
| [`synthwave`](docs/savers/synthwave.md)                   | The eighties horizon.                                                           |
| [`tv-static`](docs/savers/tv-static.md)                   | Snow with a hum bar rolling through it, full screen or on an old set.           |
| [`vinyl`](docs/savers/vinyl.md)                           | A record turning on a turntable, seen from above.                               |
| [`lighthouse`](docs/savers/lighthouse.md)                 | A banded lighthouse on a heap of rocks at night.                                |
| [`fractal-tree`](docs/savers/fractal-tree.md)             | A trunk that forks seven times over into lobes of leaves, bending in the wind.  |
| [`reaction-diffusion`](docs/savers/reaction-diffusion.md) | A Gray-Scott reaction whose spots on the left give way to stripes on the right. |
| [`double-pendulum`](docs/savers/double-pendulum.md)       | Two equal rods hung end to end from one pivot, stepped with RK4.                |

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
