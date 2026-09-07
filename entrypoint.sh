#!/bin/sh
# entrypoint for the HDMI fire screensaver.
#
# Runs fbfire, which draws a Doom-fire animation straight into a mapped display
# buffer — no terminal, no fbterm, no VT. (The old cacafire+fbterm+ncurses route
# burned ~0.5 core for ~10 fps.)
#
# Two output paths, tried in that order by fbfire itself:
#   /dev/fb0        fbdev — what fir had before the Talos v1.14.0 upgrade
#   /dev/dri/card0  DRM/KMS dumb buffer — the only path on current Talos
#
# Talos v1.14.0 builds its kernel with `# CONFIG_FB is not set`, so /dev/fbN
# does not exist on ANY node — confirmed on pine and fir. CONFIG_DRM_FBDEV_EMULATION
# is enabled but only drives the in-kernel console (fbcon), which is why an
# otherwise idle HDMI port shows a Linux terminal. No device tree overlay can
# bring fbdev back: it is compiled out, not merely unbound.
#
# Talos is headless/immutable: there is no console login here, just this pod. If
# neither device is usable we DO NOT crash — log diagnostics and idle so the pod
# stays Running and the logs stay readable.

set -u

FB="${FB_DEVICE:-/dev/fb0}"
DRM="${DRM_DEVICE:-/dev/dri/card0}"
LOOP_DELAY="${RETRY_SECONDS:-30}"

log() { echo "[screensaver] $*"; }

log "starting; FB_DEVICE=${FB} DRM_DEVICE=${DRM} FIRE_FPS=${FIRE_FPS:-30} FIRE_SCALE=${FIRE_SCALE:-4}"
log "uname: $(uname -a)"

diag() {
  log "--- display diagnostics ---"
  log "/dev entries (fb/dri):"
  # shellcheck disable=SC2012 # ls is fine here: human-readable device diagnostics
  ls -l /dev/fb* /dev/dri/* 2>&1 | sed 's/^/[screensaver]   /' || true
  log "fbset -i:"
  fbset -i 2>&1 | sed 's/^/[screensaver]   /' || true
  log "---------------------------"
}

# Main loop: wait for either output device, then run fbfire (which loops forever).
while true; do
  if [ -e "${FB}" ] || [ -e "${DRM}" ]; then
    diag
    if [ -e "${FB}" ]; then
      log "fbdev ${FB} present; launching fbfire"
    else
      log "no ${FB}; using DRM/KMS at ${DRM} (expected on Talos >= v1.14.0)"
    fi
    # fbfire runs until killed; if it ever exits, fall through and retry.
    fbfire
    log "fbfire exited (rc=$?); retrying in ${LOOP_DELAY}s"
  else
    log "no display: neither ${FB} nor ${DRM} exists."
    log "Expected on a node with no monitor attached — the framebuffer only"
    log "appears when U-Boot hands one over at boot. Check the monitor is on"
    log "this node and was connected BEFORE it booted. See app README."
    diag
    log "idling ${LOOP_DELAY}s then re-checking."
  fi
  sleep "${LOOP_DELAY}"
done
