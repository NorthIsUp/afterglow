# TODO

- **Shared palette ramp.** Five savers hand-roll the same loop — interpolate
  colour stops across N brightness levels and pack them into a palette:
  `plasma.rs` `palette()`, `doodles.rs` `ramp()`, `strings.rs` `ramp()`,
  `lissajous.rs` `ramp()` and `zot.rs` `ramps()`. Pull it into one
  `grid::ramp(stops, levels, f)`, `f` being the per-level brightness curve
  (linear, quadratic, plasma's lift-to-white), so a new saver picks a curve
  instead of copying a loop. Const where the curve allows it; every saver's
  palette must come out byte-identical.
