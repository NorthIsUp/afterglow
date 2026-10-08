# The web mirror

`https://screensaver.<tailnet>.ts.net` shows what the panel is drawing,
live. Identity is the Tailscale-injected `Tailscale-User-Login` header enforced
by the `tailscale-auth` sidecar — there is no login and there must never be one.

**What crosses the wire is cells, not pixels.** Every saver paints through
`Grid`, so the panel's whole state is `cols * rows` of a `Cell` — a glyph index
and a palette index packed into one `u32` — over a palette and a glyph table
that are both fixed for the modeset. The mirror sends the changed cells. It is
not compression; it is sending the thing the renderer already has.

Measured at 1920x1080, `SAVER_FPS=15`:

| `SAVER`  | grid    | cells  | changed/frame | damaged scanlines/frame |
| -------- | ------- | ------ | ------------- | ----------------------- |
| `matrix` | 120x33  | 3960   | 792 (20.0%)   | 1056 — the whole panel  |
| `ascii`  | 120x67  | 8040   | 2546 (31.7%)  | 1056                    |
| `blocks` | 480x270 | 129600 | 11022 (8.5%)  | ~620                    |

The right-hand column is why every pixel-shaped answer loses. Matrix dirties
every scanline every frame, so "ship the damaged rows" ships 8.1 MB per frame;
the same frame is 6.4 KB of cells. Re-encoding the panel as JPEG or PNG instead
costs the Pi tens of milliseconds per frame, against a renderer that measures
113m of one core in total.

**What it costs the renderer.** With nobody watching: one relaxed atomic load
per frame. With a viewer: one `memcpy` of the cell array (15.8 KB for matrix)
under a `try_lock` that is _skipped_ rather than waited on — the display can
never be made to wait for the web path, and a frame the mirror misses is just a
frame the mirror misses. The diff, the encode and the socket are all on the
viewer's own thread.

`GET /stream` is an HTTP/1.1 chunked binary stream — one-way server → client,
which `fetch` and a stream reader already do, so a WebSocket would buy nothing
for a hand-rolled SHA-1 and a frame codec. Records are self-describing
(`u32 count`, then `count * (u32 index, u32 cell)`), which is what makes them
survive nginx re-chunking them on the way through the gate. `GET /meta` is the
geometry, palette and glyph table; `GET /` is the page. `SAVER_HTTP=off`
removes all of it.

`POST /rotate?mins=N` sets the rotation interval; `GET /stat` is the live counters — `{"overruns":N,"viewers":N,"fps":N}`.
`overruns` is frames that ran past the frame budget, which is what a raised
`SAVER_FPS` against the pod's 500m CFS quota shows up as: the render loop is
stopped mid-period and runs a burst, and the burst is visible stutter on the
panel. It is a separate route on purpose — `/meta` is a string cached at
modeset, so a counter baked in there would report its value as of the last
modeset forever.

## Actual size in the browser

`SAVER_PANEL_MM` is the panel's **visible glass width in millimetres**, 0 (the
default) meaning nobody has measured it. It is not discoverable — this monitor's
EDID is 0 bytes, so its physical size exists nowhere the Pi can read, and it has
to be typed in by someone with a ruler. Nothing in the render path reads it.

Set it and the mirror page grows three controls, labelled `actual`:

|               |                                                                                                                       |
| ------------- | --------------------------------------------------------------------------------------------------------------------- |
| **ratio**     | the panel's SHAPE, scaled to fit the window. Always exactly right — it needs nothing the browser cannot already know. |
| **size**      | the panel's shape AND physical size, so a saver on the page is the size it is on the wall.                            |
| **calibrate** | teaches the page how big your screen is, which is what makes `size` exact.                                            |

`ratio` and `size` are modes, and the choice is remembered per device and kept
across saver switches. Without `SAVER_PANEL_MM` the whole group is hidden:
`ratio` is the fallback anyway, and offering `size` next to it would be offering a button that lies.

**`size` is the default.** CSS cannot supply the other half of the sum — how big
the viewer's own monitor is — because `width: 200mm` is 200mm only when the
browser's CSS inch is a real inch, which on a scaled or HiDPI display it is not.
Uncalibrated, the page uses the nominal 96 CSS px to the inch: within a few per
cent on an unscaled display, well out on a scaled one. The button reads `size ✓`
only once calibrated, because a viewer holding the page up against the real panel
deserves to know the number is a guess before concluding the maths is wrong. If
the result is larger than the window it says `size — too big for this window`
rather than shrinking, since a shrunk actual size is not one.

`ratio` is the escape hatch for exactly that case, and for a small window: the
shape is still exactly the panel's, it just fits.

**calibrate** makes it exact, and can be reopened at any time to re-adjust:
hold a credit card against the screen and drag the bar to match it. An ID-1 card
(ISO/IEC 7810) is 85.60mm to a tenth of a millimetre and is the one ruler
everybody already owns. The canvas resizes live under the slider — you are
matching the panel, not committing blind — and the readout shows the implied dpi.
**reset** drops back to the nominal inch by removing the stored value rather than
overwriting it with today's guess, so a future browser with a better answer is
not permanently overridden.

The px-per-mm lives in `localStorage`, per viewer and per device; it never
reaches the server.

At actual size the canvas deliberately ignores the fit-to-window limits — the
point is a fixed physical size, so a panel bigger than the browser window
overflows and the button says so rather than quietly shrinking it.
