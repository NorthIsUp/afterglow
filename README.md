# screensaver — HDMI ASCII screensaver on `fir`

A deliberately thin workload that paints an ASCII screensaver onto the HDMI
display attached to the Talos Pi5 worker **`fir`**. Current
demo: **`cacafire`** (the libcaca colour fire animation).

## How it works

`cacafire` (like `aafire`) targets a _terminal_, not a framebuffer — libcaca
ships terminal drivers (`ncurses`/`slang`) but **no** framebuffer/KMS output
driver. So the container runs [`fbterm`] (a framebuffer terminal emulator)
which paints onto the Linux framebuffer `/dev/fb0`, and launches `cacafire`
inside it with the ncurses libcaca driver:

```
CACA_DRIVER=ncurses fbterm -- cacafire
```

- Base image: `debian:bookworm-slim` + `caca-utils` (provides `cacafire`) +
  `libaa-bin` (mono `aafire` fallback) + `fbterm` + `fbset`.
- Entry point (`image/entrypoint.sh`) checks for `/dev/fb0` (or
  `/dev/dri/card0`). If present it launches fbterm/cacafire; if **absent** it logs
  framebuffer diagnostics and idles (re-checking every `RETRY_SECONDS`) so the
  pod stays `Running` and you can read the logs instead of crash-looping.
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
  that writes `/dev/fb0` (fbterm→cacafire/aafire, fbi, raw writes) does.
- 16bpp r5g6b5: colour is fine for cacafire; just no alpha/truecolor.
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
  -t ghcr.io/northisup/screensaver:0.2.0 --push .
docker buildx imagetools inspect ghcr.io/northisup/screensaver:0.2.0   # get @sha256
```

Then pin `image:` in `deployment.yaml` to the new tag + digest. Renovate is
wired (the `# renovate:` comment) to bump it like tinyframe. The image is
**private**; the `ghcr` pull secret is delivered to the `screensaver` namespace
via `k8s/secrets/ghcr-screensaver.sops.yaml`.

## Future enhancement: rotate screensavers

`aafire` is just the first demo. A nice follow-up is rotating through other
ASCII/terminal eye-candy, e.g.:

- `bb` (the classic aalib demo) — `apt install bb`
- `cmatrix` — Matrix rain
- `aaflip` / `cacademo` / `cacafire` (libcaca, colour ASCII)
- `asciiquarium`, `pipes.sh`, `tty-clock`

Pick one per restart (or loop on a timer) inside the entrypoint.

[`fbterm`]: https://salsa.debian.org/debian/fbterm
