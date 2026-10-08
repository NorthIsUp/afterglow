# `zot`

![`zot`](../media/zot.webp)

Lightning. A leader crosses the panel and forks, each fork forking again; the channel strobes over a white core in a blue halo, goes out, and usually fires again down the same channel a beat later — a double or triple flash — before the panel goes dark for a second or two. Endpoints are drawn on the perimeter by arc length, so the bolt crosses the panel at any aspect — 1920x1080 or the native 1280x400.

`src/zot.rs` is a bolt and then a gap, and the gap is most of it: the
saver's whole shape is the duty cycle. What it does with the lit part is fire
the SAME channel more than once.

- **A bolt is one channel and two or three flashes.** `strike` rasterises the
  branching tree once and then only runs a brightness envelope over it; when
  that envelope ends, `ZOT_RESTRIKE_PCT` (70) decides whether the channel
  lights again — dimmer (72% of the last peak) and quicker (80% of its length)
  — up to three strokes. Re-lighting the geometry is what makes it read as one
  bolt flashing, where a second `strike` would be two separate bolts in a row.
  Real flashes do exactly this, which is what "return stroke" means.
- **`ZOT_STROKE_GAP_MS` (110) is the beat between those flashes**, and it is
  the knob that decides whether they are one bolt or two. Three frames of dark
  channel at 30fps, with the afterglow wash still up across the panel — long
  enough to read as a distinct flash, too short to read as a new strike.
  `ZOT_GAP_MIN_MS` / `ZOT_GAP_MAX_MS` (700 / 2400) are the real gap, an order
  of magnitude longer; `a_bolt_flashes_again_down_the_same_channel` pins the
  two apart.
- **The channel decays as `1 - t²`, not `1 - t`.** A linear fall spends its
  last third in the bottom two steps of a six-step ramp, which on an 8x16 cell
  is a thread you cannot see: measured peak luma stepped 249 → 178 → 118 → 70 →
  35 → 11 → 3, and everything from 35 down is frames spent on nothing.
  Squaring holds the channel in the top half of the ramp for most of the stroke
  and then drops it off a cliff. Same frame budget, and over a 900-frame dump
  it moved frames at peak luma ≥ 64 from 6% to 16%.
- **Holding ONE bolt longer was the other candidate, and it looks wrong.**
  `ZOT_BOLT_MS=1000` with no re-strike renders the same static geometry for
  thirty frames while the strobe mask blinks it; it reads as a stuck image,
  not as lightning. Rendered side by side before choosing — the knob is still
  there if you disagree.

Measured over 900 frames at 1920x1080 with `ZOT_SEED=424242`, before the
re-strike and after: frames with anything on the panel 16% → 35%, frames at
peak luma ≥ 64 (the bolt genuinely visible rather than a trace of wash) 6% →
16%. Gaps went from 27-105 frames of black to 21-72, and the panel is still
black for 65% of the run — that is the point of the saver, and
`the_panel_goes_dark_between_bolts` holds the floor.

Nothing in the frame path changed to get any of it: the envelope is a handful
of integer ops per FRAME, the geometry is generated once per bolt, and a
re-strike reuses the buffers a strike already filled.

Source: [`src/zot.rs`](../../src/zot.rs).

## Knobs

- `ZOT_BOLT_MS` (one stroke, 60..3000, default 380, ±25% per stroke)
- `ZOT_RESTRIKE_PCT` (chance of another stroke down the same channel, 0..100, default 70, up to three strokes)
- `ZOT_STROKE_GAP_MS` (dark between those strokes, 10..1000, default 110)
- `ZOT_GAP_MIN_MS` / `ZOT_GAP_MAX_MS` (100..60000, default 700 / 2400)
- `ZOT_FORK_PCT` (0..100, default 9)
- `ZOT_AIR_PCT` (bolts ending in mid-air, 0..100, default 30)
- `ZOT_JITTER` (milli-radians of wander per step, 10..3000, default 900)
- `ZOT_GLOW_PCT` (afterglow wash peak, 0..100, default 45)
- `ZOT_HALO_PCT` (0..100, default 70)
- `ZOT_CELL_W` / `ZOT_CELL_H` (8, 16)
- `ZOT_SEED` (0 = roll one from the clock and pid; any other value reproduces the storm exactly)
