# Toasters: `toasters`, `toasters2` and `toasters3`

One flock, three brushes. `toasters` is the line-art original; `toasters2` and `toasters3` reuse its behaviour and palette.

## `toasters`

Flying toasters, after After Dark's. Four distinct models.

The art in `src/toasters.rs` is this repo's own ASCII, drawn from a
description — no Berkeley Systems bitmap is copied or transcribed. What is
copied is the behaviour, and the research behind each number is in the module
doc. The three that matter:

- **It travels down-and-left, not left.** 5 px across per 2 down, about 22°
  below horizontal. The 45° every web recreation uses comes from Bryan Braun's
  CSS version, not from the original; 2.5:1 is the only slope anyone has taken
  from shipped code (the After Dark 4.0 binary, where the flock drifts
  `(-60, +24)` per loop). The 1990 Mac module appears never to have been
  disassembled, so that is the best evidence there is.
- **Everything moves in lockstep**, one shared step vector in RUN/RISE units so
  the slope is exact and a per-object speed is not expressible. Objects differ
  only in where they entered and where they are in the wing beat.
- **The body is olive, not silver.** Quantising the 256x64 sheet by pixel count
  puts an olive chassis at a fifth of the toaster, behind a chrome front panel
  (`#909090`, the largest family) and white wings (`#F0F0F0`); the slot rims are
  a lighter chrome and the lever lighter still. Every stroke of the art carries
  an ink key naming its region, so a `/` is a white wing in one column and the
  body's receding edge in the next, and the four doneness slices ramp through
  all eight of the toast sprite's sampled golds and browns. The two olives are
  the depth cue — `#707030` lit top face, `#303010` sides turned away — and they
  pair with the near/far wing whites to keep the three-quarter view from
  flattening. Art and ink resolve to cells in a `const fn`, so a ragged row, an
  ink grid misaligned with its art, or an undefined ink key is `error[E0080]` at
  build time rather than a sprite that renders wrong. The sheet's near-black
  `#101010` is the one family with no entry: it outlined the sprite against the
  sheet's background, and here the gaps between glyphs already draw that.
- **Four wing positions, ping-ponged.** Up, mid, level, down and back, a full
  beat in 0.4 s. The original's sheet is four 64x64 frames — a half-stroke —
  and playing it 0,1,2,3 and snapping back draws only the downstroke.

Toast is the other quarter of the flock (the original spawner holds roughly
three toasters per slice) and comes in four doneness levels, each its own
sprite rather than one slice tinted, as the original's `toast0`..`toast3` were.
Entry is the original's "reverse L": lanes down the top edge and in from the
right, snapped to cell boundaries. Background is solid black and nothing paints
over it, which is why an idle region costs no blits at all.

Source: [`src/toasters.rs`](../../src/toasters.rs).

### Knobs

- `TOASTER_DENSITY` (per 1000 cells, 1..60, default 4)
- `TOASTER_SPEED` (px/sec, 8..2000, default 170)
- `TOASTER_TOAST_PCT` (0..100, default 25)
- `TOASTER_FLAP_FPS` (1..120, default 15)
- `TOASTER_CELL_W` / `TOASTER_CELL_H`

## `toasters2`

The same flock in BLOCK ELEMENTS at 8x16 cells, so the olive chassis is a solid fill rather than edge strokes.

Same scene, different brush: `src/toasters2.rs` flies the flock above
drawn in Block Elements — `█ ▀ ▄ ▒ ▛ ▜ ▙ ▟` — instead of in `/`, `|` and `=`.
Every behavioural number is `toasters`' and is not re-argued there: the 2.5:1
diagonal, the shared step vector, the six-step ping-pong, a quarter of the
flock as toast.

Three things are the difference:

- **The olive is a fill, not an outline.** Line art can only put the chassis
  colour on the strokes that outline it; a filled block puts it on the whole
  top face and the whole turned-away side, with the two slots punched out of it
  in the dark olive. Olive is **half the toaster's lit cells** — 63 of 125 in
  level flight, and within a cell of that in all four wing frames — where the
  line-art version could only outline it. That is the whole reason this saver
  exists, and `each_region_is_its_own_colour` asserts the figure so the claim
  cannot rot. (Across a whole panel it measures lower, 40%: the toast and the
  wings dilute it. The sprite is the population the claim is about.)
- **Half the cell, twice the resolution.** 8x16 against the line-art version's
  16x32, so a half block is a square 8x8 pixel and the toaster is 30x7 cells
  where the classic is 14x4 — the same 240 px on the panel, at twice the
  detail. Wing diagonals use the three-quarter blocks at their joints, without
  which a wing is a flight of steps 16 px to a tread.
- **One toaster, four slices.** The original flew one machine; `toasters` flies
  four models as an acknowledged departure. At this resolution the flap and the
  four doneness sprites already carry the variety, so this one is the
  original's — and the slices' scorch is drawn as a 50% shade creeping up from
  the bottom a row a level, not only tinted.

The flock's mix is COUNTED, not rolled per object. A 25% coin flip over sixteen
objects lands on a single slice about one seed in twenty, and nothing re-rolls
an object's kind, so that seed's sky holds one slice for the life of the pod.

This one draws through `Grid::flush`, not the sparse path every other
object-based saver uses, and that is measured rather than assumed. It ran on
`flush_sparse` with a hand-maintained dirty list and paid MORE than the
whole-grid diff it was avoiding: 4237 indices pushed per frame, sorted
(`k log k`, the largest single cost in the renderer), deduped to 2701, and most
of those blitted twice, because `flush_sparse` blits its whole list
unconditionally while the clear pass had just written a cell the stamp pass
immediately rewrote with the same value. Clear the grid, stamp the flock,
`flush`: 0.0799 → 0.0299 ms/frame, and 737 → 676 median damaged scanlines of
the 1072 the grid owns (against `toasters`' 672 of 1056, and 1056 for `ascii`
and `matrix`, which repaint everything every frame).

The second half of the trade is not speed. A hand-maintained dirty list can
under-report, and a cell written but left out of it keeps its old pixels on the
panel forever; damage derived by diffing cannot.

Source: [`src/toasters2.rs`](../../src/toasters2.rs).

### Knobs

- `TOASTER2_DENSITY`
- `TOASTER2_SPEED`
- `TOASTER2_TOAST_PCT`
- `TOASTER2_FLAP_FPS`
- `TOASTER2_CELL_W` / `TOASTER2_CELL_H`

## `toasters3`

The same flock drawn with a BRAILLE-style 2x4 dot matrix per cell, so one 16x6-cell toaster is a 32x24 bitmap — real slot openings, a dial, a lever, barbed wings. One model, not four.

`toasters3` is `toasters`' behaviour at eight times the shape resolution. The
slope, the lockstep, the ping-pong beat, the 3:1 toast ratio and the sampled
palette are the same values, shared where it matters: `toasters3/art.rs` imports
`toasters::art`'s `PAL_RGB` and `ink`, so the two savers cannot drift to
different colours.

**Braille here is this repo's own bitmap, not Unifont's.** Unifont draws
U+2800..28FF as a _reading_ font: a set dot is a 2x2 pip and an UNSET dot is a
1x1 pip, so U+2800 is not blank and a fully-set U+28FF lights 16 of a cell's 128
pixels. As sub-cell graphics that is a 12.5%-coverage ghost with a permanent pip
grid under it, and the non-blank U+2800 would break the transparency the sprite
stamp depends on. `tools/genfont.py` therefore generates the 256 cells as FILLED
4x4 quadrants — the same 2x4 addressing, drawn solid. Pattern `0x00` interns
onto `BLANK` and `0xFF` onto `SOLID`, so a dotless cell is transparent for free.
Cost: the atlas goes from 144 glyphs (2304 B) to 398 (6368 B), `font.rs` from
185 lines to 446. The glyph index is already `u16`, so nothing widened.

**Detail is 8x; colour is 1x.** A `Cell` is one glyph and one palette index, so
every dot inside a cell is the same colour. The art is drawn around that: colour
regions are at least a cell (two dots) wide, and everything finer than a cell is
drawn as NEGATIVE space — the slots, the lever track, the dial recess, the two
chrome ribs, the crumb-tray seam and the gap between the feet are all unlit dots
reading black against chrome. An interior detail drawn as a second _shade_ of
chrome disappears at cell resolution; the same detail drawn as holes does not.
The first draft of this sprite had a solid interior and rendered as a grey blob.

**The art is a dot bitmap, not a row of braille characters.** Sprites in
`toasters3/art.rs` are written as `#` and spaces, two characters across and four
rows down per cell; `bake_braille` packs each block into its pattern byte at
compile time. Braille characters would be the same data in a form nobody can
edit, and would break the byte-indexed const parser — U+28xx is three bytes of
UTF-8 where a column is one byte.

**One model, not four.** `toasters` flies four silhouettes because a sky of one
14x4 line-art shape is repetitive. A 32x24 bitmap already carries more
information than all four of those, so this flies one toaster drawn properly
rather than four drawn four times as expensively.

Measured over 599 frames at 1920x1080, damaged scanlines per frame:

| saver       | median | mean | max  |
| ----------- | ------ | ---- | ---- |
| `city`      | 16     | 24   | 144  |
| `toasters3` | 288    | 304  | 672  |
| `toasters`  | 672    | 626  | 960  |
| `matrix`    | 1056   | 1056 | 1056 |

`city`'s max is its one moving thing: a shooting star fires about once a minute
and costs a median 112 scanlines for the ~25 frames it crosses in. Its quiet
frames — 3,553 of 3,599 in a two-minute run — top out at 64.

`toasters3` is _cheaper_ than `toasters` despite the bigger sprite: the default
density is 2 per 1000 cells rather than 4, because the original sized its flock
by area under sprite (~22% of the screen) and this sprite is 96 cells where the
classic is 56. Nine bigger objects at nine heights merge into fewer damage runs
than fifteen smaller ones.

Source: [`src/toasters3.rs`](../../src/toasters3.rs).

### Knobs

- `TOASTER3_DENSITY` (per 1000 cells, 1..60, default 2)
- `TOASTER3_SPEED`
- `TOASTER3_TOAST_PCT`
- `TOASTER3_FLAP_FPS`
- `TOASTER3_CELL_W` / `TOASTER3_CELL_H`
