# `SAVER_PIXEL_ASPECT`: squashed on pine

Pine's monitor is a **1280x400** panel that advertises nothing the Pi can read —
its EDID is 0 bytes, the connector comes up as `Unknown-1`, there is no
`/dev/vcio`, and vc4/v3d are not in the Talos image at all. The VideoCore
firmware picks 1920x1080 at boot and that is what simpledrm hands us;
`framebuffer_width` / `framebuffer_height` in `config.txt` were tried on the real
node and measured dead (the evidence is in `cluster/schematic.yaml`).

So the panel rescales our output **non-uniformly**: 1920→1280 is 1.5x, 1080→400
is 2.7x. Everything reaches the glass squashed vertically by 2.7/1.5 = **1.8x** —
a circle is a wide ellipse, a square a wide rectangle.

`SAVER_PIXEL_ASPECT` is that number in per-cent: **how much taller a cell must
be DRAWN so that it lands square on the glass** — so **180** on pine and **100**
(the default) everywhere else.

Read it that way round and nothing else. A framebuffer pixel lands _shorter_
than it is wide on this panel, never taller; the stretch is the compensation,
not the symptom. Stating it the other way is what made `xwing` multiply where it
should have divided, composing a 1:1 picture onto 3.2:1 glass. Converting a
`Grid` dimension back to square-glass units is therefore always a DIVISION by
`pixel_aspect()`, because `Grid::new` has already multiplied — `marble.rs` and
`xwing.rs` both do it and both say so. It is a **process-wide** knob, not
a per-saver one, and 100 is a byte-for-byte no-op — proved by dumping all 25
savers before and after and diffing the PPMs.

It applies in `Grid::new`, which makes the CELL 1.8x taller. That is the whole
trick: every saver here draws in cells or in braille sub-cells of a cell, so a
cell that is visually square makes the saver's own coordinate space visually
square and 21 of the 25 are corrected without a line of their own. Four are not,
because they measure something in framebuffer PIXELS rather than in cells, and
each carries the stretch explicitly:

| saver            | what needed it                                                                                |
| ---------------- | --------------------------------------------------------------------------------------------- |
| `warp`           | the projection: `focal_y = focal * aspect`, so the tunnel is round                            |
| `podracer`       | the projection: `fy = f * aspect`, so the engines are round and the canyon is not a letterbox |
| `moire`          | the gratings are evaluated in a space `aspect` shorter than the framebuffer                   |
| `toasters{,2,3}` | the 2.5:1 flight diagonal, which is a pixel slope and not a cell slope                        |
| `confetti`       | the 48% pile incline, which is meant to be 48% on the GLASS                                   |

Everything else physical needs nothing and that is not luck: `hardrain`'s wind is
"cells sideways per 100 of fall" and `sakura`'s drift, `rain`, `worms` and the
rest are all in cells or sub-cells, so a taller cell re-leans them by exactly the
right amount. Vertical stays vertical at any aspect.

Two things change as a side effect, both wanted: there are fewer ROWS (1080/29
rather than 1080/16 at the usual 8x16 cell), and a saver sized off "the shorter
panel side" — `pov`, `hypercube` — now measures that side in visually square
units, so the figure comes out round rather than merely fitted.

The mirror sends the FRAMEBUFFER, which is pre-stretched, so `/meta` carries
`pixel_aspect` and the page divides its canvas height by it. The browser then
shows what the wall shows rather than what the renderer drew — the page has no
monitor to do the un-stretching for it. Only the displayed shape changes; the
pixels are the framebuffer's, untouched.

`meta_reports_the_aspect_the_renderer_actually_used` is what keeps the two from
drifting apart: a `/meta` aspect the renderer did not use is invisible in review
and surfaces only as "the web version does not match the screen".
