# How it works

`src/main.rs` opens `/dev/dri/card0`, modesets the connector's preferred
mode, creates an XRGB8888 dumb buffer, and maps it. Each frame the active saver
draws into that mapping and the host tells the driver which scanlines changed.

**Why DRM and not `/dev/fb0`:** Talos v1.14.0 builds its kernel with
`# CONFIG_FB is not set`, so `/dev/fbN` exists on no node — verified on pine and
fir. `CONFIG_DRM_FBDEV_EMULATION` is on, but it only feeds the in-kernel console
(`fbcon`), which is exactly the bare terminal an unconfigured HDMI port shows. No
device tree overlay can bring fbdev back: it is compiled out, not unbound. There
is deliberately **no fbdev fallback** — no node has one, and an untestable
fallback path is worse than none.

**Why the dirty call is load-bearing:** simpledrm — the driver U-Boot hands over
on a Pi 5 — scans out of a _shadow_ buffer. A pixel written into the mapping
reaches the panel only if the driver is told its scanline changed. That is what
`src/surface.rs` is about, and its module doc is the contract every saver
is held to; read it before writing a new one. A region written but never reported
shows the previous frame forever, and that bug reproduces on hardware and
nowhere else.

Damage is **rectangles**, not scanline bands: a run carries `x0..x1` as well as
`y0..y1`, taken from the exact cell `Surface::cell_rows` is handing out. A
224 px toaster used to report 1920 px-wide scanlines and the shadow copy moved
8.5x the pixels that changed; it now copies 0.14 Mpx a frame instead of 1.15.
Full-repaint savers are unaffected — their rects are the panel either way — so
**pixels, not damaged scanlines, is the number to read.** Two objects at
opposite ends of the panel touch every scanline between them and still copy
almost nothing, which is why the scanline tables on the saver pages (e.g.
[`toasters3`](savers/toasters.md#toasters3)) understate how
cheap the sparse savers now are.

**Layout** (`src/`): `surface.rs` (the mapped frame + damage), `grid.rs`
and `font.rs` (character grid + the one glyph blitter), `fire.rs`, `matrix.rs`
`toasters.rs`, `toasters3.rs` and `city.rs` (the savers), `saver.rs` (the trait and the name → saver
dispatch), `host.rs` (DRM), `dump.rs` (headless PPM rendering). Adding a saver
is a module plus one row in `saver::SAVERS`. Its page goes in `docs/savers/` with a row in
the README's saver index; `docs_check` fails the build until both exist.
