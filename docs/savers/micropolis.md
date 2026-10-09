# `micropolis`

![`micropolis`](../media/micropolis.gif)

Micropolis, the open-source SimCity, with an AI mayor building the city on its
own. It starts on new land, lays roads, zones homes, shops and industry as the
demand valves ask, and powers, polices and protects them. Now and then a fire,
flood or earthquake strikes, and the mayor rebuilds what it knocked down. After
`MICROPOLIS_CITY_MINS` a new city starts. A quarter of the new cities are
instead sample cities from the release, which the mayor takes over and keeps
running. The map fills the panel at any shape, with a slow camera drifting over
what has been built.

**Only in the GPL image** (`ghcr.io/northisup/afterglow:latest-gpl`,
`:sha-<commit>-gpl`; the older `-doom` tags are the same image). That build
compiles in the Micropolis engine, which is GPL-3.0 with Electronic Arts'
additional terms, so the binary and image are GPL-3.0 as a whole. The default
image is MIT and has none of it. See [`THIRD_PARTY.md`](../../THIRD_PARTY.md).
Build it with `cargo build --release --features micropolis`, which needs a C++
compiler.

Micropolis is a registered trademark of Micropolis Corporation (Micropolis
GmbH) and is licensed here as a courtesy of the owner
([micropolis.com](https://www.micropolis.com)). SimCity is a trademark of
Electronic Arts, which has no part in this.

## The mayor

The city grows in blocks: a ring of road around two rows of three 3x3 lots, so
every zone touches a road and the zones in a block carry power to each other.
Homes and shops share "town" blocks, so a trip to the shops stays short.
Industry gets its own blocks on one side of the centre, and the power plants go
beyond it, so the smog drifts over nobody's house. A town block keeps one lot
as a park, which raises the land value around it.

Each step, the mayor builds the most urgent thing it has decided on and the map
does not show yet. That means plants first, then power lines, roads, stations,
zones and parks. A burnt-out zone or a road a quake broke is rebuilt the same
way as a new one. Once everything it wants stands, it makes one more decision:

1. a coal plant when what is built and planned would draw more than nine
   tenths of the power (nuclear once the city is rich and big);
2. a power line from the grid to any zone left without power, the cheapest run
   round land kept for buildings;
3. a stadium, seaport or airport when the engine caps homes, industry or shops
   for want of one, saving up for it first;
4. a police station for every forty zones (sooner if crime is high), and a fire
   station for any block more than three blocks from one;
5. a zone of whichever kind has the most demand, while there are fewer empty
   zones of that kind than it wants waiting. Empty zones nobody moves into for
   two city years stop counting, so a badly placed lot does not stall the city.

It keeps a plant's worth of money back once the grid is two thirds drawn or the
city passes 10,000 people. That way a quake that topples a plant is not the end
of the city. Taxes go up to 8 or 9 percent when funds run low and down to 6
when they pile up. With disasters off, most of its cities reach 40,000 to
70,000 people within 100 city years; on cramped or watery land some stall near
20,000.
`mise run bench-micropolis [seed] [years]` grows one headless at full speed
and prints a line a year. A city that falls to an eighth of its peak and stays
there for eight years is replaced by a new one.

## How it runs

- The tile size is square on the glass, and big enough that the view spans
  `MICROPOLIS_VIEW_PCT` of the 120x100-tile map along the panel's tighter
  axis. The map always covers the panel, so there are no bars at any shape. The
  camera drifts over the built-up area, and holds still on it while it fits.
- One `micropolis` thread runs the engine and the mayor at
  `MICROPOLIS_YEAR_SECS` per city year, animates traffic and smoke 8 times a
  second, and parks when another saver is showing. The city carries on when
  `micropolis` comes back. The render thread only takes the last finished map,
  and skips a frame's update if the engine is writing it.
- An engine error (an assert, or any other fatal path) unwinds to a setjmp
  boundary in `micropolis/afterglow_micropolis.cpp` instead of ending the
  process. The saver then shows the last map, frozen, with "stopped" in the
  overlay, until the pod restarts.
- Tiles are drawn straight onto the panel from a tile sheet pre-scaled to the
  tile size: one slice copy per tile row. A frame redraws only the tiles that
  changed, unless the camera moved a pixel. The overlay is the map under it at
  half brightness with the text over it, and it is redrawn only when its text
  or the map under it changes.
- The web mirror and the terminal get one solid cell per tile, in the tile's
  mean colour.
- Trains, planes, helicopters and the like are not drawn, so the mayor builds
  no rail and the disasters are the ones without a sprite: fire, flood and
  earthquake. A meltdown is possible once there is a nuclear plant.

Source: [`src/micropolis/`](../../src/micropolis/mod.rs),
[`micropolis/afterglow_micropolis.cpp`](../../micropolis/afterglow_micropolis.cpp),
the engine in [`micropolis/engine/`](../../micropolis/engine/micropolis.h).

## Knobs

- `MICROPOLIS_VIEW_PCT`: how much of the map is in view along the panel's
  tighter axis, 20..100 percent (default 70). 100 shows the whole map that way.
- `MICROPOLIS_PAN`: camera speed in panel pixels a second, 0..120 (default 6).
  0 holds the camera on the city's middle.
- `MICROPOLIS_HUD`: 1 shows the city's name, population, date and funds in the
  bottom-left corner (default), 0 shows the map alone.
- `MICROPOLIS_YEAR_SECS`: wall-clock seconds per city year, 1..3600 (default
  40).
- `MICROPOLIS_CITY_MINS`: minutes before a new city starts, 0..10080 (default
  120). 0 keeps one city forever.
- `MICROPOLIS_DISASTER_MINS`: mean minutes between disasters, 0..10080 (default
  25). 0 means none.
- `MICROPOLIS_BUNDLED_PCT`: the chance in a hundred that a new city is one of
  the release's sample cities instead of new land (default 25).
- `MICROPOLIS_SEED`: pins the terrain, cities and disasters (default: from the
  clock).
