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

# fbterm needs a real kernel VT (it rejects ptys), AND it only paints the
# framebuffer when that VT is the FOREGROUND console. /dev/tty0 is an alias for
# "current VT" but fbterm does not render reliably through it; it must be given
# the concrete foreground VT (e.g. /dev/tty1). Resolve it from
# /sys/class/tty/tty0/active (e.g. "tty1"), falling back to tty1. Override with
# VT_DEVICE. Verified on fir: targeting the active VT gives sustained animation
# (~10 frame changes / 12s); targeting /dev/tty0 left the framebuffer static.
resolve_vt() {
  if [ -n "${VT_DEVICE:-}" ]; then echo "${VT_DEVICE}"; return; fi
  active="$(cat /sys/class/tty/tty0/active 2>/dev/null)"
  if [ -n "${active}" ] && [ -e "/dev/${active}" ]; then echo "/dev/${active}"; else echo /dev/tty1; fi
}
VT="$(resolve_vt)"

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
  # Re-resolve the active VT each time (it could change between retries).
  VT="$(resolve_vt)"
  # cacafire loops forever on its own (colour fire animation), drawing into a
  # terminal that fbterm paints onto the framebuffer.
  log "framebuffer ${FB} present; launching fbterm -> cacafire (CACA_DRIVER=ncurses) on ${VT}"
  # fbterm REQUIRES a real kernel VT (/dev/tty*) on stdin — it explicitly
  # rejects ptys ("stdin isn't a interactive tty!"), which is all a container
  # normally gets. So we make a new session (setsid -c => controlling tty) with
  # the foreground VT ${VT} (mounted via host /dev) as stdin/stdout.
  # Verified on fir: with this, fbterm+cacafire stay alive and animate /dev/fb0.
  # TERM for the terminal libs; CACA_DRIVER pins libcaca to its ncurses driver
  # (libcaca has no framebuffer/KMS driver — fbterm is what paints the fb).
  export TERM=linux
  export CACA_DRIVER=ncurses
  # Run in the background and wait, rather than exec: as PID 1 the entrypoint
  # must stay alive to hold the container open and keep the retry loop. Using
  # `exec` here detaches the new session and the container exits immediately
  # (verified crash-loop). setsid -c gives fbterm its own session + controlling
  # tty on the VT.
  # shellcheck disable=SC2094 # ${VT} is a tty device, not a regular file: reading
  # (keystrokes) and writing (screen) the same VT is correct and intended.
  setsid -c sh -c 'fbterm -- cacafire' <"${VT}" >"${VT}" 2>&1 &
  wait $!
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
