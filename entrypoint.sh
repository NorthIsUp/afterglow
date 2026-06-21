#!/bin/sh
# entrypoint for the fir HDMI fire screensaver.
#
# Runs fbfire, which writes a Doom-fire animation DIRECTLY into the mmap'd
# framebuffer (/dev/fb0) — no terminal, no fbterm, no VT. This is the fast path
# (the old cacafire+fbterm+ncurses route burned ~0.5 core for ~10 fps).
#
# Talos is headless/immutable: there is no console login here, just this pod.
# If the framebuffer device is absent we DO NOT crash — we log a diagnostic and
# idle so the pod stays Running and the operator can inspect logs. The moment
# /dev/fb0 appears, restarting the pod (or the next retry) starts drawing.

set -u

FB="${FB_DEVICE:-/dev/fb0}"
LOOP_DELAY="${RETRY_SECONDS:-30}"

log() { echo "[screensaver] $*"; }

log "starting; FB_DEVICE=${FB} FIRE_FPS=${FIRE_FPS:-30} FIRE_SCALE=${FIRE_SCALE:-4}"
log "uname: $(uname -a)"

diag() {
  log "--- framebuffer diagnostics ---"
  log "/dev entries (fb/dri):"
  # shellcheck disable=SC2012 # ls is fine here: human-readable device diagnostics
  ls -l /dev/fb* /dev/dri/* 2>&1 | sed 's/^/[screensaver]   /' || true
  log "fbset -i:"
  fbset -i 2>&1 | sed 's/^/[screensaver]   /' || true
  log "-------------------------------"
}

# Main loop: wait for the framebuffer, then run fbfire (which loops forever).
while true; do
  if [ -e "${FB}" ]; then
    diag
    log "framebuffer ${FB} present; launching fbfire"
    # fbfire runs until killed; if it ever exits, fall through and retry.
    fbfire
    log "fbfire exited (rc=$?); retrying in ${LOOP_DELAY}s"
  else
    log "framebuffer ${FB} not present."
    log "On the Pi5 a usable /dev/fb0 needs the firmware/simplefb (fir) or vc4"
    log "KMS. See app README. Idling and re-checking; pod stays Running."
    diag
    log "idling ${LOOP_DELAY}s then re-checking."
  fi
  sleep "${LOOP_DELAY}"
done
