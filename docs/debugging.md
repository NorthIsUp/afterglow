# Debugging

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
