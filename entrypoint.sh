#!/bin/sh
# entrypoint for the spruce HDMI ASCII screensaver.
#
# Renders aafire (the aalib fire demo) onto the Linux framebuffer attached to
# spruce's HDMI port. aafire targets a terminal, not the framebuffer directly,
# so we run it inside fbterm, which paints a terminal onto /dev/fb0.
#
# Talos is headless/immutable: there is no console login here, just this pod.
# If the framebuffer device is absent (the Pi5 vc4 KMS / firmware framebuffer
# is not enabled yet) we DO NOT crash — we log a clear diagnostic and idle so
# the pod stays Running and the operator can inspect logs. The moment /dev/fb0
# (or /dev/dri/card0) appears, restarting the pod will start drawing.

set -u

FB="${FB_DEVICE:-/dev/fb0}"
LOOP_DELAY="${RETRY_SECONDS:-30}"

log() { echo "[screensaver] $*"; }

log "starting; FB_DEVICE=${FB}"
log "uname: $(uname -a)"

diag() {
  log "--- framebuffer diagnostics ---"
  log "/dev entries (fb/dri/tty0):"
  ls -l /dev/fb* /dev/dri/* /dev/tty0 2>&1 | sed 's/^/[screensaver]   /' || true
  log "/sys/class/graphics:"
  ls /sys/class/graphics 2>&1 | sed 's/^/[screensaver]   /' || true
  log "/sys/class/drm:"
  ls /sys/class/drm 2>&1 | sed 's/^/[screensaver]   /' || true
  log "-------------------------------"
}

run_aafire() {
  # fbterm needs a tty; it opens the active VT. Run aafire inside it.
  # -s 12 sets a readable font size. aafire loops forever on its own.
  log "framebuffer ${FB} present; launching fbterm -> aafire"
  # fbterm refuses to run as a login shell without a controlling tty in some
  # setups; exec it directly with the command. TERM must be set for aalib.
  export TERM=linux
  exec fbterm -- aafire
}

# Main loop: wait for the framebuffer, then hand off to fbterm/aafire.
while true; do
  if [ -e "${FB}" ] || [ -e /dev/dri/card0 ]; then
    diag
    run_aafire
    # If fbterm/aafire ever exits, fall through and retry rather than die.
    log "fbterm/aafire exited (rc=$?); retrying in ${LOOP_DELAY}s"
  else
    log "framebuffer ${FB} not present (no /dev/dri/card0 either)."
    log "On the Pi5 this means the vc4 KMS driver / firmware framebuffer is not"
    log "enabled in Talos. See app README for the kernel/overlay follow-up."
    diag
    log "idling ${LOOP_DELAY}s then re-checking (pod stays Running)."
  fi
  sleep "${LOOP_DELAY}"
done
