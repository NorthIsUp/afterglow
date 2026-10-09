# `doom`

![`doom`](../media/doom.gif)

Freedoom, played by an autopilot (in god mode unless `DOOM_GOD=0`), as one game sized to the panel.
The screen is as wide as the panel's glass shape needs, and the view is Hor+
widescreen in the manner of Crispy Doom: the vertical field of view stays
classic Doom's, and the horizontal one grows with the width. That gives 90° at
4:3 and about 106° at 16:9. On pine's 3.2:1 glass it would reach 135°, so it is
capped at 120°. The view runs the full height of the screen, with the status
bar drawn over it in the middle. A switch to `doom` starts a new random map, and
so does finishing one, dying on one, or `DOOM_MAP_SECS` on the same map.

**Only in the GPL image** (`ghcr.io/northisup/afterglow:latest-gpl`,
`:sha-<commit>-gpl`; the older `-doom` tags are the same image). That build
compiles in [doomgeneric](https://github.com/ozkl/doomgeneric), which is
GPL-2.0-or-later, and [Micropolis](micropolis.md), which is GPL-3.0, so the
binary and image are GPL-3.0 as a whole. The default image is MIT and has no
Doom in it. See [`THIRD_PARTY.md`](../../THIRD_PARTY.md). Build it with
`cargo build --release --features doom`, which needs a C compiler, and point
`DOOM_WAD` at an IWAD. `tools/freedoom.sh` fetches Freedoom Phase 1.

The autopilot plays each map to its exit. It fights what it sees on the way,
opens doors, calls lifts, fetches keys for locked doors, presses switches,
detours for secrets and nearby pickups, and on a boss map hunts the bosses
down. In god mode it finishes most of Freedoom's Episode 1 inside
`DOOM_MAP_SECS`. A finished map shows its tally for 5 s, then a new random map
starts; so does the end-of-episode text.

### How the autopilot finds its way

At each map load `doom/ap_nav.c` lays a grid of 32-unit cells over the level.
An edge between neighbouring cells lists the two-sided lines its segment
crosses, found once with `P_PathTraverse`. Whether the player can cross is
judged when searching, from the sectors' current heights: a step of at most 24
units and 56 of headroom. A door it can open (with the key, if locked) counts
as open. A lift it can call counts as being at whichever end suits. A
teleporter line leads to its destination. Cells too close to a wall or a ledge
for the player's 16-unit radius are closed, so routes never grind a corner.

Once a second `doom/autopilot.c` floods that grid from the player (Dijkstra)
and picks one errand by path cost, in this order:

1. health, when low and god mode is off;
2. pickups in sight within about 480 units (further, with god mode off, for a
   first real gun, armour or ammo it is short of);
3. the exit, once a route reaches it, after any secret sector within reach in
   the first 90 s;
4. a switch, walk-over or gun line that opens the way to the exit, then keys,
   then a line that opens the way to a key, then secrets;
5. a line that opens up new ground, ground it has not seen, the monsters still
   alive, and last the lines that seem to do nothing.

What a line does is worked out in `doom/ap_effect.c`. For every switch,
walk-over and gun special it knows the trigger, whether it repeats, and the
action: a door opening or shutting, a floor raised or lowered to its target
(next floor up, lowest ceiling around, lowest floor around, +24, and so on), a
lift, a flight of stairs, a ceiling brought down. While the exit is out of
reach, a few lines a second have their effect laid over the tagged sectors'
heights as a hypothesis, and the grid is flooded again from where the player
would stand to set the line off. A line that brings the exit within reach is
the errand at once; one that reaches a key, or opens up new ground, comes
next; one that only shuts things is left alone. Scores hold until a line fires
for good, a key is taken, or 10 s pass. A walk-over line on the lip of a
ledge is crossed from the top, and a gun line is shot from a spot with a clear
line to it. Each score costs one flood, one to two milliseconds on a
desktop, and there are at most four a second.

It follows the path, aiming through the middle of each opening and at the
farthest cell in a clear straight walk. A door or lift on the way is pressed:
it walks up, faces the line and uses it, or stands on a lift and waits. A lift
called from elsewhere becomes an errand first. It strafes along the route while
turning to shoot the nearest monster it can see. Pressing a switch takes
priority over turning to fight. A cell it keeps failing to cross gets
expensive. An errand that fails three times, or outlasts its walk by 25 s, is dropped.
If it has not moved 96 units in 15 s, it jumps to a random item with room
around it.

Some ideas come from Ioan Chera's
[AutoDoom](https://github.com/ioan-chera/AutoDoom) bot for Eternity: aiming
through the middle of each opening, a lift that counts as being at either end,
and judging a switch by laying its effect over the sector heights and
searching again. No code is taken from it.

`tools/doom-bench/` plays maps headless at full speed and reports exits, time,
kills, secrets and stuck-jumps. `mise run bench-doom 1 8` plays every Episode 1
map eight times, in parallel, each run with its own random game.

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
[`doom/afterglow_doom.c`](../../doom/afterglow_doom.c),
[`doom/autopilot.c`](../../doom/autopilot.c),
[`doom/ap_nav.c`](../../doom/ap_nav.c).

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
- `DOOM_WAD`: path to the IWAD (default `/freedoom1.wad`, where the GPL
  image puts it). Any Doom or Doom II IWAD works. If it can't be read, the saver
  shows static.
- `DOOM_MAP_SECS`: the longest one map runs before a new random one, in
  seconds, 0..86400 (default 300). The clock starts again with each map, so
  the timer only cuts in when the autopilot is still short of the exit. 0
  changes maps only on an exit, a death, or a switch to `doom`.
- `DOOM_GAMMA`: palette brightness lift, 0..4 (default 2).
- `DOOM_LIGHT`: extra sector light, 0..2 (default 1). This is Doom's gun-flash
  boost held on, so dark rooms don't read as a black panel. It takes effect on
  the next map.
- `DOOM_SEED`: pins the random maps (default: from the clock).

`DOOM_VIEWS` is gone. The side-by-side views it chose between were replaced by
the single widescreen game. A running engine keeps the WAD it loaded.
