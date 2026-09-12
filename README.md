---
title: screensaver
kind: app
namespace: screensaver
icon: monitor
source: k8s/apps/screensaver/
---

# screensaver — HDMI screensavers on whichever Pi5 holds the monitor

A deliberately thin workload that paints an animation onto the HDMI display of
the Talos Pi5 carrying the `hardware.homelab/display: "true"` label. Renderer:
`image/` — a static musl Rust binary on a `FROM scratch` image, ~230 KB, which
writes pixels straight into a DRM/KMS dumb buffer.

## Savers

Pick one with `SAVER` (older spelling: `FIRE_STYLE`). Anything unrecognised
falls back to `ascii` — a headless pod must never crash-loop on a typo.

| `SAVER`  | What                                                                                                                                | Knobs                                                                     |
| -------- | ----------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| `ascii`  | Doom fire as an ASCII ramp (`" .:-=+*#%@"`), one heat sample per character cell, coloured by the 37-step fire palette. The default. | `FIRE_CELL` (px, 8..64, default 16)                                       |
| `blocks` | The same fire drawn as chunky pixels — a solid glyph per cell.                                                                      | `FIRE_SCALE` (px, 1..16, default 4)                                       |
| `matrix` | Digital rain.                                                                                                                       | `MATRIX_CELL_W` (8..64, default 16), `MATRIX_CELL_H` (8..128, default 32) |

Common: `SAVER_FPS` (1..120, default 30; older spelling `FIRE_FPS`),
`DRM_DEVICE` (default `/dev/dri/card0`), `RETRY_SECONDS`.

All of these are plain deployment env changes — no image rebuild.

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
and `font.rs` (character grid + the one glyph blitter), `fire.rs` / `matrix.rs`
(the savers), `saver.rs` (the trait and the name → saver dispatch), `host.rs`
(DRM), `dump.rs` (headless PPM rendering). Adding a saver is a module plus one
arm in `saver::make`.

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
pre-refactor build.

## Building / publishing the image

Built and pushed by CI (`.github/workflows/screensaver-image.yml`) on any change
under `image/`: arm64-native, running `cargo fmt --check`, `clippy -D warnings`,
the tests and a dump render, then publishing as `latest` and `sha-<commit>`. The
image is **private**; the `ghcr` pull secret is delivered to the `screensaver`
namespace via `k8s/secrets/ghcr-screensaver.sops.yaml`.

Then bump the `image:` digest in `deployment.yaml` — in its own commit, with no
env changes in it, so the new binary always runs against the old env block first.

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
