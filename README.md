# screensaver — HDMI ASCII screensaver on `spruce`

A deliberately thin workload that paints an ASCII screensaver onto the HDMI
display attached to the Talos Pi5 worker **`spruce`**. First
demo: **`aafire`** (the aalib fire animation).

## How it works

`aafire` targets a *terminal*, not a framebuffer. So the container runs
[`fbterm`] (a framebuffer terminal emulator) which paints onto the Linux
framebuffer `/dev/fb0`, and launches `aafire` inside it:

```
fbterm -- aafire
```

- Base image: `debian:bookworm-slim` + `libaa-bin` (provides `aafire`) +
  `fbterm` + `fbset`.
- Entry point (`image/entrypoint.sh`) checks for `/dev/fb0` (or
  `/dev/dri/card0`). If present it launches fbterm/aafire; if **absent** it logs
  framebuffer diagnostics and idles (re-checking every `RETRY_SECONDS`) so the
  pod stays `Running` and you can read the logs instead of crash-looping.
- Scheduling: `nodeSelector: kubernetes.io/hostname: spruce`,
  `securityContext.privileged: true`, host `/dev` mounted so the framebuffer is
  visible the moment it appears.

## ⚠️ Current state: no framebuffer on spruce yet

As of this writing **spruce has no usable framebuffer**:

- `/dev/fb0` — does **not** exist.
- `/dev/dri` / `/dev/dri/card0` — does **not** exist.
- `/sys/class/drm` contains only `version` (no `card0`) → the **vc4 KMS driver
  is not loaded**.
- `dmesg` shows the legacy fb driver failing:
  `bcm2708_fb soc@107c000000:fb: Unable to determine number of FBs. Disabling
  driver. ... probe ... failed with error -2`.
- No `vc4` / `v3d` / `drm` / `simplefb` modules are loaded.

So the pod will run but only log the "framebuffer not present" diagnostic until
the framebuffer is enabled.

### Follow-up to light up the screen (Talos / Pi5 kernel + firmware)

The Raspberry Pi 5 needs the VideoCore KMS display stack enabled so the kernel
creates `/dev/dri/card0` (+ a `/dev/fb0` via simpledrm/fbcon). On Raspberry Pi
OS this is `dtoverlay=vc4-kms-v3d` in `config.txt`; on **Talos** it is governed
by the boot firmware / device-tree shipped in the talos-rpi5 image. Options to
investigate (spruce-only, does not affect cedar/fir):

1. **Firmware `config.txt` / overlay**: ensure the rpi5 boot media enables
   `vc4-kms-v3d` (and `max_framebuffers`, `hdmi_force_hotplug` if the screen is
   not always powered). The talos-rpi5 fork controls this in its image overlay.
2. **Talos machine config kernel args** for `spruce`: add the vc4/v3d modules /
   `video=` if the driver is built but not auto-probing. (Current cmdline has
   `console=tty0 ... talos.dashboard.disabled=1` — the dashboard is already
   disabled, so Talos is *not* fighting for the framebuffer; the device simply
   isn't created.)
3. Confirm the talos-rpi5 kernel actually has `CONFIG_DRM_VC4` / `CONFIG_DRM_V3D`
   built; if not, a custom image/extension is required.

Once `/dev/dri/card0` (or `/dev/fb0`) appears, delete the pod and it will start
drawing automatically — no manifest change needed.

> Note: enabling vc4 KMS only changes **spruce's** boot config, which is fine
> per the project's "spruce-only Talos config" guidance.

## Building / publishing the image

arm64-only cluster. Built locally on Apple Silicon (native arm64) and pushed to
the private ghcr package:

```sh
cd k8s/apps/screensaver/image
echo "$(gh auth token)" | docker login ghcr.io -u NorthIsUp --password-stdin
docker buildx build --platform linux/arm64 \
  -t ghcr.io/northisup/screensaver:0.1.0 --push .
docker buildx imagetools inspect ghcr.io/northisup/screensaver:0.1.0   # get @sha256
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
