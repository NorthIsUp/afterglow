# `doom`

![`doom`](../media/doom.gif)

Freedoom, played by an autopilot (in god mode unless `DOOM_GOD=0`), as one game sized to the panel.
The screen is as wide as the panel's glass shape needs, and the view is Hor+
widescreen in the manner of Crispy Doom: the vertical field of view stays
classic Doom's, and the horizontal one grows with the width. That gives 90° at
4:3 and about 106° at 16:9. On pine's 3.2:1 glass it would reach 135°, so it is
capped at 120°. The view runs the full height of the screen, with the status
bar drawn over it in the middle. A switch to `doom` starts a new random map, and so does each
`DOOM_MAP_SECS` after that.

**Only in the `-doom` image** (`ghcr.io/northisup/afterglow:latest-doom`,
`:sha-<commit>-doom`). That build compiles in
[doomgeneric](https://github.com/ozkl/doomgeneric), which is GPL-2.0, so the
`-doom` binary and image are GPL as a whole. The default image is MIT and has no
Doom in it. See [`THIRD_PARTY.md`](../../THIRD_PARTY.md). Build it with
`cargo build --release --features doom`, which needs a C compiler, and point
`DOOM_WAD` at an IWAD. `tools/freedoom.sh` fetches Freedoom Phase 1.

The autopilot turns to the nearest monster it can see and shoots it. Otherwise
it walks towards the most open, least visited heading, never one with a wall
within 96 units, and presses use as it goes, which opens doors. If it stands
still for 2 s, or covers no new ground for 8 s, it jumps to a random item on the
map. Intermissions and the end-of-episode text are skipped to a new random map.

## How it runs

- doomgeneric's render width is a run-time value, at most 1280 columns, and the
  height stays 200. The 3D view projects with a focal length that keeps
  Doom's pixel aspect, so walls and sprites are never stretched. Menus, the
  status bar, HUD messages and the intermission keep their 320-wide
  coordinates and are drawn centred.
- One `doom` thread runs the engine at 35 tics a second and parks when another
  saver is showing. The render thread only takes the last finished frame, and
  skips it if the engine thread is writing it, so a level load never delays a
  frame. Switching back reuses the running engine on a new map. A knob change
  from the mirror page rebuilds the view and keeps the map going.
- An engine error (`I_Error`, or any other `exit`) unwinds to the boundary in
  `doom/afterglow_doom.c` instead of ending the process. The saver then shows
  static until the pod restarts, because Doom's globals cannot be re-initialised
  in place.
- The frame is scaled straight onto the panel's pixels, and only when the
  engine has finished a new one. Only source rows that changed are redrawn,
  each over the columns that changed. On pine that keeps a full-motion frame
  about as cheap as `plasma`, where a cell-by-cell diff of 207k cells cost
  five times as much.
- The web mirror and the terminal get the same frame as solid cells. The
  palette holds all fourteen PLAYPAL palettes, so damage and pickup tints are
  ordinary colour indices. A panel narrower than 4:3 shows the 320-wide game
  letterboxed.

Source: [`src/doom/`](../../src/doom/mod.rs),
[`doom/afterglow_doom.c`](../../doom/afterglow_doom.c).

## Knobs

- `DOOM_HUD`: 0 shows the view alone (default), 1 draws the status bar centred
  over it.
- `DOOM_SKILL`: 1..5, from I'm Too Young To Die to Nightmare (default 3, Hurt
  Me Plenty). The mirror page shows it as a slider. A new skill applies from
  the next map.
- `DOOM_GOD`: 1 makes the player invulnerable (default). With 0 the autopilot
  can die: the death view holds for 3 s, then a new random map starts, so the
  saver never waits on Doom's press-use screen.
- `DOOM_WIDTH_PCT`: the share of the screen's width the game view takes, 0..100
  (default 100; 0 also means the whole width, and below 10 counts as 10). The
  rest is the border flat with Doom's bevel, and the status bar stays centred.
- `DOOM_FOV`: the horizontal field of view inside that box, in degrees, 0..170
  (default 0: Hor+ for the box's shape, capped at 120°; 1..59 counts as 60). A
  fixed field of view leaves the vertical one to follow, Vert- or Vert+, so
  nothing is scaled.
- `DOOM_WAD`: path to the IWAD (default `/freedoom1.wad`, where the `-doom`
  image puts it). Any Doom or Doom II IWAD works. If it can't be read, the saver
  shows static.
- `DOOM_MAP_SECS`: seconds before a new random map, 0..86400 (default 180). 0
  changes maps only when you switch to `doom`.
- `DOOM_GAMMA`: palette brightness lift, 0..4 (default 2).
- `DOOM_LIGHT`: extra sector light, 0..2 (default 1). This is Doom's gun-flash
  boost held on, so dark rooms don't read as a black panel. It takes effect on
  the next map.
- `DOOM_SEED`: pins the random maps (default: from the clock).

`DOOM_VIEWS` is gone. The side-by-side views it chose between were replaced by
the single widescreen game. A running engine keeps the WAD it loaded.
