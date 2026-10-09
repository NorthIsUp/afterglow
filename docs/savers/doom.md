# `doom`

![`doom`](../media/doom.gif)

Freedoom, played by an autopilot in god mode. The panel shows as many 320x200
views side by side as its shape fits, and each view plays its own random map:
three on pine's 3.2:1 glass, two at 16:9, one at 4:3, on a square or in
portrait. A switch to `doom` starts every view on a new map, and so does each
`DOOM_MAP_SECS` after that.

**Only in the `-doom` image** (`ghcr.io/northisup/afterglow:latest-doom`,
`:sha-<commit>-doom`). That build compiles in
[doomgeneric](https://github.com/ozkl/doomgeneric), which is GPL-2.0, so the
`-doom` binary and image are GPL as a whole; the default image is MIT and has no
Doom in it. See [`THIRD_PARTY.md`](../../THIRD_PARTY.md). Build it with
`cargo build --release --features doom` (needs a C compiler) and point
`DOOM_WAD` at an IWAD; `tools/freedoom.sh` fetches Freedoom Phase 1.

The autopilot turns to and shoots the nearest monster it can see. Otherwise it
walks towards the most open, least visited heading, never one with a wall within
96 units, and presses use as it goes, which opens doors. If it covers no new
ground for 8 s (a pit, a locked door), it jumps to a random item on the map.
Intermissions and the end-of-episode text are skipped to a new random map.

## How it runs

- Each view is its own copy of the engine. Doom keeps its world in C globals,
  so `build.rs` compiles doomgeneric four times, renaming every global it
  defines to `dg<N>_*`. Four copies is the most views a panel gets.
- One `doom` thread runs the copies at 35 tics a second and parks when another
  saver is showing. The render thread only takes the last finished frame, and
  skips it if the engine thread is writing it, so a level load never delays a
  frame. Switching back reuses the running copies on new maps.
- An engine error (`I_Error`, or any other `exit`) unwinds to the boundary in
  `doom/afterglow_doom.c` instead of ending the process. That copy is retired,
  and its view moves to a spare one or, once none are left, shows static.
- Each Doom pixel is a solid cell. The palette holds all fourteen PLAYPAL
  palettes, so damage and pickup tints are ordinary colour indices and the web
  mirror works unchanged.

Source: [`src/doom/`](../../src/doom/mod.rs),
[`doom/afterglow_doom.c`](../../doom/afterglow_doom.c).

## Knobs

- `DOOM_WAD`: path to the IWAD (default `/freedoom1.wad`, where the `-doom`
  image puts it). Any Doom or Doom II IWAD works. If it can't be read, every
  view shows static.
- `DOOM_VIEWS`: views side by side, 0..4 (default 0, which picks from the
  panel's shape).
- `DOOM_MAP_SECS`: seconds before every view moves to a new random map, 0..86400
  (default 180). 0 changes maps only when you switch to `doom`.
- `DOOM_GAMMA`: palette brightness lift, 0..4 (default 2).
- `DOOM_LIGHT`: extra sector light, 0..2 (default 1). This is Doom's gun-flash
  boost held on, so dark rooms don't read as a black panel.
- `DOOM_SEED`: pins the random maps (default: from the clock).

`DOOM_WAD` and `DOOM_LIGHT` apply only when an engine starts or changes map.
An engine that is already running keeps the WAD it loaded.
