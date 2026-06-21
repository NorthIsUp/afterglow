# screensaver — HDMI fire screensaver on `fir`

A deliberately thin workload that paints a fire animation onto the HDMI display
attached to the Talos Pi5 worker **`fir`**. Renderer:
**`fbfire`**, a tiny C program that writes a Doom-fire straight to the
framebuffer.

## How it works

`fbfire` (`image/fbfire.c`) opens `/dev/fb0`, queries its geometry/format via
`ioctl` (`FBIOGET_VSCREENINFO`/`FSCREENINFO`), `mmap`s it, and renders the
classic Doom PSX fire — a low-res heat grid seeded white-hot along the bottom
and propagated upward each frame, palette-mapped and block-scaled to the panel —
**directly into the mmap'd framebuffer**. No terminal, no `fbterm`, no VT.

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
- Tunables (env): `FIRE_FPS` (default 30), `FIRE_SCALE` (low-res grid =
  panel width / scale, default 4), `FB_DEVICE` (default `/dev/fb0`).
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
  -t ghcr.io/northisup/screensaver:0.4.0 --push .
docker buildx imagetools inspect ghcr.io/northisup/screensaver:0.4.0   # get @sha256
```

Then pin `image:` in `deployment.yaml` to the new tag + digest. Renovate is
wired (the `# renovate:` comment) to bump it like tinyframe. The image is
**private**; the `ghcr` pull secret is delivered to the `screensaver` namespace
via `k8s/secrets/ghcr-screensaver.sops.yaml`.

## Tuning the framerate

- `FIRE_FPS` caps the frame rate (default 30). Raise it if you want a faster
  flame and have CPU headroom; lower it to save power.
- `FIRE_SCALE` sets the low-res fire grid (panel width / scale). Default 4 gives
  a 480-wide grid block-scaled to 1920 — a good speed/look balance. Smaller =
  finer + more CPU; larger = chunkier + cheaper.

## Future enhancement: more effects

`fbfire` is a single direct-to-fb effect. Follow-ups (all as direct fb writers,
to keep the fast path): plasma, starfield, Matrix rain, metaballs. Add a mode
switch in `fbfire.c` / the entrypoint.
