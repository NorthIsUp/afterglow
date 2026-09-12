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

| `SAVER`    | What                                                                                                                                | Knobs                                                                                                                                                                                                                        |
| ---------- | ----------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ascii`    | Doom fire as an ASCII ramp (`" .:-=+*#%@"`), one heat sample per character cell, coloured by the 37-step fire palette. The default. | `FIRE_CELL` (px, 8..64, default 16)                                                                                                                                                                                          |
| `blocks`   | The same fire drawn as chunky pixels — a solid glyph per cell.                                                                      | `FIRE_SCALE` (px, 1..16, default 4)                                                                                                                                                                                          |
| `matrix`   | Digital rain.                                                                                                                       | `MATRIX_CELL_W` (8..64, default 16), `MATRIX_CELL_H` (8..128, default 32)                                                                                                                                                    |
| `toasters` | Flying toasters, after After Dark's.                                                                                                | `TOASTER_DENSITY` (per 1000 cells, 1..60, default 4), `TOASTER_SPEED` (px/sec, 8..2000, default 170), `TOASTER_TOAST_PCT` (0..100, default 25), `TOASTER_FLAP_FPS` (1..120, default 15), `TOASTER_CELL_W` / `TOASTER_CELL_H` |

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
- **Four wing positions, ping-ponged.** Up, mid, level, down and back, a full
  beat in 0.4 s. The original's sheet is four 64x64 frames — a half-stroke —
  and playing it 0,1,2,3 and snapping back draws only the downstroke.

Toast is the other quarter of the flock (the original spawner holds roughly
three toasters per slice) and comes in four doneness levels, each its own
sprite rather than one slice tinted, as the original's `toast0`..`toast3` were.
Entry is the original's "reverse L": lanes down the top edge and in from the
right, snapped to cell boundaries. Background is solid black and nothing paints
over it, which is why an idle region costs no blits at all.

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
and `toasters.rs` (the savers), `saver.rs` (the trait and the name → saver
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
| Image        | `busybox:1.36`, `ghcr.io/northisup/screensaver@sha256:1b81aa21876b3af53def4068a45da938feeb74085d0fcbdbbc461d14465920c3`, `nginxinc/nginx-unprivileged:1.27-alpine` |
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
