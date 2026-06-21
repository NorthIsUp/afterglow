# screensaver — HDMI fire screensaver on `fir`

A deliberately thin workload that paints a fire animation onto the HDMI display
attached to the Talos Pi5 worker **`fir`**. Renderer:
**`fbfire`**, a tiny C program that writes a Doom-fire straight to the
framebuffer.

## How it works

`fbfire` (`image/fbfire.c`) opens `/dev/fb0`, queries its geometry/format via
`ioctl` (`FBIOGET_VSCREENINFO`/`FSCREENINFO`), `mmap`s it, and renders the
classic Doom PSX fire — a low-res heat grid seeded white-hot along the bottom
and propagated upward each frame, palette-mapped — **directly into the mmap'd
framebuffer**. No terminal, no `fbterm`, no VT.

Two render styles (`FIRE_STYLE`):

- **`ascii`** (default) — the retro/lo-fi look: the panel is divided into
  `FIRE_CELL`-pixel character cells; each cell samples the fire heat, maps it to
  a glyph from an ASCII ramp (`" .:-=+*#%@"`), and blits a built-in 8×8 font
  glyph (nearest-scaled to the cell) coloured by the heat palette. Recreates the
  cacafire/aalib ASCII vibe at direct-framebuffer speed.
- **`blocks`** — chunky-pixel fire: the low-res grid is block-scaled straight to
  the panel. Chunkiness set by `FIRE_SCALE` (bigger = blockier).

This replaced an earlier `cacafire`+`fbterm`+`ncurses` path. That route rendered
ASCII fire into a terminal that `fbterm` repainted onto `/dev/fb0` — indirect
and slow: it burned **~0.5 core for ~10 fps** and required wrangling a foreground
kernel VT for `fbterm` (it rejects the pty a container gets). Writing the fb
directly is the fast path: **~30 fps at a fraction of the CPU.**

- Base image: multi-stage `debian:bookworm-slim` — build stage compiles
  `fbfire.c` with gcc; runtime stage ships just the binary + `fbset` (fb
  diagnostics) + the entrypoint. Supports 16bpp RGB565 (fir simplefb) and 32bpp
  XRGB8888 (the common KMS case).
- Entry point (`image/entrypoint.sh`) waits for `/dev/fb0`; if present it runs
  `fbfire` (loops forever); if **absent** it logs diagnostics and idles
  (re-checking every `RETRY_SECONDS`) so the pod stays `Running` instead of
  crash-looping.
- Tunables (env): `FIRE_FPS` (default 30), `FIRE_STYLE` (`ascii`|`blocks`,
  default `ascii`), `FIRE_CELL` (ascii cell px, 8..64, default 16), `FIRE_SCALE`
  (blocks grid = panel width / scale, default 4), `FB_DEVICE` (`/dev/fb0`).
- Scheduling: `nodeSelector: kubernetes.io/hostname: fir`,
  `securityContext.privileged: true`, host `/dev` mounted so the framebuffer is
  visible.

## Display state (verified 2026-06-20)

The monitor is plugged into **fir**, and fir exposes a working, writable
framebuffer — no vc4/KMS or image rebuild required:

- `fir`: U-Boot registers a **simplefb** at boot
  (`simple-framebuffer 3f800000.framebuffer: fb0: simplefb registered!`),
  giving `/dev/fb0` at **1920x1080, r5g6b5, 16bpp, stride 3840**. A throwaway
  privileged pod on fir successfully wrote pixels to `/dev/fb0` (screen
  flashed), so a pod can drive this display today.
- `spruce`: **no** framebuffer — kernel falls back to `Console: colour dummy
device`, `/proc/fb` is empty, no `/dev/fb0`, `/sys/class/drm` has only
  `version`. (spruce + fir both boot via U-Boot UEFI with the vc4 HDMI
  device-tree nodes `status = disabled`; fir happens to get a U-Boot simplefb
  handover, spruce does not.) This is why the screensaver is pinned to **fir**.

### Notes / limitations

- The framebuffer is **simplefb**, not DRM/KMS: there is **no** `/dev/dri/card0`.
  Tools that require KMS (kmscube, modern DRM clients) will not work; anything
  that writes `/dev/fb0` (fbfire's raw writes, fbi, etc.) does.
- 16bpp r5g6b5: `fbfire` palettes to RGB565; just no alpha/truecolor.
- If the simplefb ever stops appearing on fir (e.g. monitor unplugged at boot,
  or a kernel/boot change), the proper fix is enabling the `siderolabs/vc4`
  system extension + `dtoverlay=vc4-kms-v3d` in the **fir** boot image (built at
  image-generation time — see siderolabs sbc-raspberrypi docs). Do this on the
  worker `fir`, never on `spruce` (sole etcd/control-plane).

## Building / publishing the image

arm64-only cluster. Built locally on Apple Silicon (native arm64) and pushed to
the private ghcr package:

```sh
cd k8s/apps/screensaver/image
echo "$(gh auth token)" | docker login ghcr.io -u NorthIsUp --password-stdin
docker buildx build --platform linux/arm64 \
  -t ghcr.io/northisup/screensaver:0.5.0 --push .
docker buildx imagetools inspect ghcr.io/northisup/screensaver:0.5.0   # get @sha256
```

Then pin `image:` in `deployment.yaml` to the new tag + digest. Renovate is
wired (the `# renovate:` comment) to bump it like tinyframe. The image is
**private**; the `ghcr` pull secret is delivered to the `screensaver` namespace
via `k8s/secrets/ghcr-screensaver.sops.yaml`.

## Tuning the look + framerate

- `FIRE_STYLE` — `ascii` (retro glyph fire, default) or `blocks` (chunky pixels).
- `FIRE_CELL` — ascii character cell size in px (8..64, default 16). Bigger =
  chunkier/more retro, fewer cells. This is the main "more ASCII-ish" knob.
- `FIRE_SCALE` — blocks-mode chunkiness (panel width / scale, default 4). Bigger
  = blockier. Ignored in ascii mode.
- `FIRE_FPS` — frame-rate cap (default 30). Raise for a faster flame if you have
  CPU headroom; lower to save power.

Measured on fir @ 1920×1080 16bpp: `ascii` cell=16 ≈ 30 fps @ **216m CPU**;
`blocks` scale=4 ≈ 30 fps @ 379m CPU. Both well under the 500m limit. These are
plain deployment env changes — tweak `FIRE_*` in `deployment.yaml` with **no
image rebuild**.

## Future enhancement: more effects

`fbfire` is a single direct-to-fb effect. Follow-ups (all as direct fb writers,
to keep the fast path): plasma, starfield, Matrix rain, metaballs. Add a mode
switch in `fbfire.c` / the entrypoint.
