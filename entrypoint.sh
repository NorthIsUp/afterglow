#!/bin/sh
# entrypoint for the spruce HDMI ASCII screensaver.
#
# Renders cacafire (the libcaca colour fire demo) onto the Linux framebuffer
# attached to spruce's HDMI port. cacafire, like aafire, targets a *terminal*
# (libcaca has no framebuffer/KMS output driver of its own), so we run it inside
# fbterm, which paints a terminal onto /dev/fb0. The ncurses libcaca driver is
# the right one for a TTY/framebuffer terminal with no X server.
#
# Talos is headless/immutable: there is no console login here, just this pod.
# If the framebuffer device is absent (the Pi5 vc4 KMS / firmware framebuffer
# is not enabled yet) we DO NOT crash — we log a clear diagnostic and idle so
# the pod stays Running and the operator can inspect logs. The moment /dev/fb0
# (or /dev/dri/card0) appears, restarting the pod will start drawing.

set -u

FB="${FB_DEVICE:-/dev/fb0}"
LOOP_DELAY="${RETRY_SECONDS:-30}"
# Real kernel VT for fbterm's controlling tty (it rejects ptys). The host's
# /dev/tty0 (active VT) is visible via the host /dev mount. Override with VT_DEVICE.
VT="${VT_DEVICE:-/dev/tty0}"

log() { echo "[screensaver] $*"; }

log "starting; FB_DEVICE=${FB}"
log "uname: $(uname -a)"

diag() {
  log "--- framebuffer diagnostics ---"
  log "/dev entries (fb/dri/tty0):"
  # shellcheck disable=SC2012 # ls is fine here: human-readable device diagnostics
  ls -l /dev/fb* /dev/dri/* /dev/tty0 2>&1 | sed 's/^/[screensaver]   /' || true
  log "/sys/class/graphics:"
  # shellcheck disable=SC2012
  ls /sys/class/graphics 2>&1 | sed 's/^/[screensaver]   /' || true
  log "/sys/class/drm:"
  # shellcheck disable=SC2012
  ls /sys/class/drm 2>&1 | sed 's/^/[screensaver]   /' || true
  log "-------------------------------"
}

run_fire() {
  # cacafire loops forever on its own (colour fire animation), drawing into a
  # terminal that fbterm paints onto the framebuffer.
  log "framebuffer ${FB} present; launching fbterm -> cacafire (CACA_DRIVER=ncurses) on ${VT}"
  # fbterm REQUIRES a real kernel VT (/dev/tty*) on stdin — it explicitly
  # rejects ptys ("stdin isn't a interactive tty!"), which is all a container
  # normally gets. So we make a new session (setsid -c => controlling tty) with
  # the host VT ${VT} (default /dev/tty0, mounted via host /dev) as stdin/stdout.
  # Verified: with this, fbterm+cacafire stay alive and animate /dev/fb0.
  # TERM for the terminal libs; CACA_DRIVER pins libcaca to its ncurses driver
  # (libcaca has no framebuffer/KMS driver — fbterm is what paints the fb).
  export TERM=linux
  export CACA_DRIVER=ncurses
  # shellcheck disable=SC2094 # ${VT} is a tty device, not a regular file: reading
  # (keystrokes) and writing (screen) the same VT is correct and intended.
  exec setsid -c sh -c 'fbterm -- cacafire' <"${VT}" >"${VT}" 2>&1
}

# Main loop: wait for the framebuffer, then hand off to fbterm/aafire.
while true; do
  if [ -e "${FB}" ] || [ -e /dev/dri/card0 ]; then
    diag
    run_fire
    # If fbterm/cacafire ever exits, fall through and retry rather than die.
    log "fbterm/cacafire exited (rc=$?); retrying in ${LOOP_DELAY}s"
  else
    log "framebuffer ${FB} not present (no /dev/dri/card0 either)."
    log "On the Pi5 this means the vc4 KMS driver / firmware framebuffer is not"
    log "enabled in Talos. See app README for the kernel/overlay follow-up."
    diag
    log "idling ${LOOP_DELAY}s then re-checking (pod stays Running)."
  fi
  sleep "${LOOP_DELAY}"
done
