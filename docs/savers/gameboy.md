# `gameboy`

![`gameboy`](../media/gameboy.gif)

A Game Boy and Game Boy Color emulator playing free homebrew on autopilot,
with the screen filling the panel's height and the game's world carried on
out to its sides. Give it a ROM of your own and it plays that instead; give
it Pokémon Red, Blue or Yellow and a bot plays it for good.

It is in the default image. The emulator is
[mizu-core](https://github.com/Amjad50/mizu) (MIT), vendored in
`vendor/mizu-core`, and the bundled cartridges are all free to redistribute:

| Cartridge                                                               | What the autopilot does                                                                                       |
| ----------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| [Tobu Tobu Girl Deluxe](https://github.com/SimonLarsen/tobutobugirl-dx) | Steers her under the next cloud or bird to bounce on, clear of spikes and fireballs, from the game's own RAM. |
| [Rebound](https://github.com/DevEd2/ReboundGB)                          | Rolls right and jumps, through a level that scrolls past, which the wide view keeps on screen behind it.      |
| [LIFE_gb](https://github.com/Brehana/LIFE_gb)                           | Seeds the board and lets Conway's Life run.                                                                   |

One plays at a time, a random one first, the next after `GAMEBOY_ROTATE_SECS`.
Licences: [`THIRD_PARTY.md`](../../THIRD_PARTY.md). No commercial ROM is
bundled, and none ever will be.

## Full screen

The Game Boy's screen is 160x144. It is scaled to the panel's full height and
centred, its pixels kept square (pine's `SAVER_PIXEL_ASPECT` included), and
the view is as wide as the panel's glass: 256 Game Boy pixels at 16:9, 461 on
pine's 3.2:1. What fills the rest depends on `GAMEBOY_WIDE`:

- **Off: the glow.** Each side is the average colour of the screen's edge
  beside it, blurred down the rows and dimmed, so the picture spills softly
  onto the panel.
- **On (default): more of the world.** For any game this is the trick
  [WideGB](https://github.com/kemenaran/SameBoy/tree/wide_gb) and WideNES
  use: remember the background as the screen scrolls past it and leave what
  scrolled off where it was. Sprites are not remembered (a sprite frozen at
  the edge looks wrong), and nor are status bars or the window layer. A
  screen that disagrees with most of what is remembered under it counts as
  a new room and starts the memory over. Where nothing is remembered yet,
  the glow. No code is taken from either; the idea is theirs.
- **For Pokémon, the real map.** The overworld is drawn from the cartridge's
  own map data, in [pret's pokered layout](https://github.com/pret/pokered):
  the map you are on and the maps its connections join, block by block,
  with the live tiles from VRAM so water and flowers animate, and the people
  standing off the screen in their places. It shows ground you have never
  walked, which the remembering trick cannot. Its colours are learned every
  frame from the screen itself, which follows fades and Yellow's GBC colours
  and doubles as the check that it lines up. In a battle, a menu or a map
  change it steps aside for the glow.

## Playing your own cartridge

```sh
GAMEBOY_ROM=/roms/pokemon_red.gb      # a file
GAMEBOY_ROM=/roms                     # or a folder of .gb/.gbc, in rotation
```

The ROM must be one you have the right to use. The usual way is to dump a
cartridge you own with a cartridge reader (a GBxCart RW, an Epilogue GB
Operator) and keep the file to yourself. afterglow never downloads, bundles
or caches a ROM: it reads the path you give it, in place, and writes nothing
anywhere.

In Kubernetes, put the ROMs somewhere the node can read and mount them
read-only into the container:

```yaml
spec:
  containers:
    - name: screensaver
      image: ghcr.io/northisup/afterglow:latest
      env:
        - { name: SAVER, value: gameboy }
        - { name: GAMEBOY_ROM, value: /roms/pokemon_red.gb }
      volumeMounts:
        - { name: roms, mountPath: /roms, readOnly: true }
  volumes:
    - name: roms
      hostPath: { path: /var/mnt/roms, type: Directory } # or a PVC, an NFS share
```

`GAMEBOY_SAV` points at a battery save to start from (a `.sav` from the same
cartridge reader, say): the bot then carries on your own adventure. It is read
once and never written; a restart starts from it again.

### Pokémon

A ROM is recognised as Pokémon by its SHA-1, against the revisions pret's
disassemblies build:

| Game                       | SHA-1                                      |
| -------------------------- | ------------------------------------------ |
| Pokémon Red (USA, Europe)  | `ea9bcae617fdf159b045185467ae58b2e4a48b9a` |
| Pokémon Blue (USA, Europe) | `d7037c83e1ae5b39bde3c30787637ba1d4c48ce2` |
| Pokémon Yellow (USA, Eur.) | `cc7d03262ebfaf2f06772c1a480c7d9d5f4a38e1` |

The log prints the checksum of every ROM it loads. Any other revision plays
like any other game: Start and A, and the remembered-background wide view.

The bot plays from the game's own RAM. It gets through the title screen, the
intro and both names, walks out of the house, takes a starter from Oak's
table and fights the rival. From there it roams: it plans routes over the
map's walkable squares, read from the cartridge, to whichever door,
staircase or map edge it has used least; it reads every text box and fights
every battle with its first move. When the party faints the game sends it
back to the last Pokémon Center, healed, and it sets off again. If some
scripted moment holds it in one place for three minutes it rewinds to a save
state from a few minutes before and tries again with different luck.

At the title it reads the menu off the screen rather than counting presses:
with a battery save the menu gains CONTINUE, and the bot takes it, so it
carries on the save (yours, from `GAMEBOY_SAV`) instead of starting over. An
options screen it wanders into is backed out of with B, and an intro still
going after eight minutes gets the game's own soft reset.

The mirror page's **↺ Restart** (`POST /restart?saver=gameboy`) is a
power-on reset: the cartridge boots again with no battery save, so the bot
takes NEW GAME.

It does not beat the game. That needs either a full route through the story
(every key item, gym and cutscene) or a tool-assisted movie replayed input
for input. TASVideos' Pokémon movies are BizHawk recordings on Gambatte or
GBHawk, from power-on with Nintendo's GBC boot ROM, which afterglow cannot
ship. Tried here with the 75-second Red "save glitch" movie (4329M): the
menus sync (the game is saving at the frame the movie cuts the power), but
the full run does not, across 630 boot-offset and power-cut timings. A
movie would need your own boot ROM dump to start from the same state, which
the emulator could load but this saver does not take yet. In the author's
tests, an hour and a half of play takes the bot from the bedroom through
Pallet Town and Route 1 to Viridian City.

## How it runs

- One `gameboy` thread runs the emulator at the Game Boy's 59.73 Hz and
  composes the view; it parks when another saver is showing and the game
  waits where it was. The render thread only takes the last finished view,
  and skips it if the thread is writing it.
- A panic anywhere in a frame (the core, a pilot, a view) is caught at the
  frame boundary, and that cartridge is dropped for the next. That is why
  the release build unwinds instead of aborting. A ROM that will not load is
  skipped the same way; with none left, the saver shows static.
- The view is scaled straight onto the panel's pixels, only the rows that
  changed, each over the columns that changed, as `doom` does. The web
  mirror and the terminal get it as solid cells in RGB444.
- On an M-series Mac the emulator thread takes about 0.4 to 0.55 ms a frame
  (3% of a core) and the worst-case redraw about a quarter of a millisecond,
  as much as `plasma`. Pi 5 cores are several times slower, which puts it at
  roughly 0.15 to 0.2 of a core, inside pine's 500m.

Source: [`src/gameboy/`](../../src/gameboy/mod.rs),
[`vendor/mizu-core/`](../../vendor/mizu-core/src/lib.rs).

## Knobs

- `GAMEBOY_ROM`: a `.gb` or `.gbc` file, or a folder of them (default empty:
  the bundled homebrew).
- `GAMEBOY_SAV`: a battery save to start a single ROM from (default none).
- `GAMEBOY_WIDE`: 1 fills the sides with more of the game's world (default),
  0 with the glow.
- `GAMEBOY_PALETTE`: the four shades a monochrome game is drawn in: `auto`
  (default: Red and Blue in their cover colours, anything else green),
  `green`, `pocket`, `grey`, `red`, `blue`. Colour games keep their own.
- `GAMEBOY_ROTATE_SECS`: seconds per cartridge when there are several,
  0..86400 (default 600; 0 never moves on).
- `GAMEBOY_SEED`: pins which cartridge plays first and the pilots' choices
  (default: from the clock).
