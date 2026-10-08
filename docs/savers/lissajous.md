# `lissajous`

![`lissajous`](../media/lissajous.webp)

A point tracing `x = sin(a·t + d)`, `y = sin(b·t)`, leaving a trail that fades
behind it, with `b` and `d` drifting so the figure morphs through its family
instead of settling on one shape. Up to three pens draw at once, each in its own
hue.

Each axis keeps a phase accumulator advanced by `b·dt` rather than evaluating
`sin(b·t)` with an unbounded `t`, so a change to `b` bends the curve from where
the pen is instead of whipping the whole figure around.

The curve is stamped as braille, 2x4 dots per cell, so it is drawn at four times
the vertical and twice the horizontal cell resolution. The trail fades, so most
lit cells change every frame and the saver does a full repaint.

Source: [`src/lissajous.rs`](../../src/lissajous.rs) — the module doc has the
full design notes.

## Knobs

- `LISSAJOUS_CELL_W` / `LISSAJOUS_CELL_H` (px, 4..64 / 8..128, default 8 / 16)
- `LISSAJOUS_CURVES` (pens drawing at once, 1..3, default 3)
- `LISSAJOUS_FADE_MS` (how long a trail takes to fade out, ms, 200..20000,
  default 9000)
- `LISSAJOUS_SPEED` (pen speed, milli-radians per second, 10..5000, default 900)
- `LISSAJOUS_MORPH_S` (seconds for one full sweep of the ratio `b` from
  `RATIO_LO` to `RATIO_HI` and back, 5..3600, default 95)
- `LISSAJOUS_PHASE_S` (seconds for one full turn of the phase offset `d`,
  2..3600, default 26)
- `LISSAJOUS_SAMPLES` (curve samples per second, 100..40000, default 3200)
- `LISSAJOUS_RATIO_LO` / `LISSAJOUS_RATIO_HI` (range the frequency ratio `b`
  morphs across, hundredths, 50..1200, default 100 / 500; swapped if LO is
  above HI)
