# `gba`

A Game Boy Advance emulator playing your own ROMs on autopilot, the screen
filling the panel's height with a soft glow of its edges out to the sides.
Pokémon FireRed, LeafGreen and Emerald are recognised and played through
their intro to the overworld.

It is in the default image. The emulator is [mGBA](https://mgba.io) 0.10.5
(MPL-2.0), its GBA core vendored in `gba/mgba` and compiled in by the `gba`
cargo feature, which both images build with. MPL-2.0 is copyleft per file,
not per program: the MIT image can carry it as long as mGBA's source stays
available, and it is, in this repository. Licences:
[`THIRD_PARTY.md`](../../THIRD_PARTY.md).

Nothing is bundled: no ROM, and no Nintendo BIOS. Without a ROM the saver
shows a card saying where to put them.

## ROMs

```sh
GBA_ROM=/roms/gba                     # a folder of .gba files, in rotation (default)
GBA_ROM=/roms/gba/minish-cap.gba      # or one file
```

In the deployment that folder is `gba/` on the `screensaver-roms` volume,
beside `gameboy/`. ROMs are read where they are, one at a time: the
playing ROM is in memory once (mGBA reads it in place), and the next is
read only after the last is dropped, so a 32 MiB cartridge fits the pod's
128 MiB. The volume is 256 MiB, which holds about a dozen 16 MiB games.

The ROM must be one you have the right to use: dump a cartridge you own
with a cartridge reader (a GBxCart RW, an Epilogue GB Operator). Battery
saves are never read from or written to the ROM folder; each game's
progress is kept in a save state instead (below).

## BIOS

mGBA's built-in HLE BIOS answers the BIOS calls games make, so no BIOS is
needed and none is shipped (Nintendo's is copyrighted). The boot logo is
skipped. `GBA_BIOS` points at a real one dumped from your own GBA to boot
through it instead.

## The autopilot

The log prints every ROM's SHA-1. One pilot plays everything:

- **Boot:** A every quarter second, START every fourth tap. That gets
  through title screens, file menus and, in Pokémon, the professor's
  speech, the gender question and the naming screen (START jumps to OK).
- **Explore:** a direction held for a quarter second to a second and a
  half, A now and then to talk or pick, B more often to back out of
  whatever A opened. A screen that sits still for ten seconds (a pause, a
  game-over card) gets a START.
- A generic game switches from boot to explore after 90 seconds. Pokémon
  switches when its RAM says the overworld is running (`gMain.callback1`,
  null until then), and never presses START there: it is the menu.

The Pokémon revisions recognised are pret's USA/Europe releases: FireRed
and LeafGreen 1.0 and 1.1, Emerald. On the author's FireRed 1.1 and
LeafGreen 1.1 the player is in the bedroom after 75 s of play (named JON
from the preset list); on Emerald, in the moving truck after 78 s. From
there the walk is random: in FireRed it found the stairs down to Mom
after about eleven minutes, and wanders the house. A story bot like `gameboy`'s is a later
project.

`GBA_ROTATE_SECS` moves to the next ROM (default 15 minutes), which also
gets any game out of a spot the walk cannot leave.

## Save states

As [`gameboy`'s](gameboy.md#save-states), from the same shared runner: a
save state of the playing ROM when it rotates away, when another saver
takes the panel and every `GBA_STATE_SECS`, loaded on its next boot, which
then skips straight to the pilot's explore phase. A state is mGBA's own
(`mCoreSaveStateNamed`) with the game's battery save and clock in it:
about 516 KiB for FireRed (a 128 KiB flash save), taken in 0.1 ms and
written in under a millisecond on an M-series Mac. With `SAVER_STATE_DIR`
set it is `$SAVER_STATE_DIR/gba/<sha1>.state` (and `.prev`); unset, memory
for the life of the process. A bad one is dropped for the previous one or a
fresh boot; ↺ Restart deletes the playing ROM's.

## How it runs

- One `gba` thread runs the emulator at the GBA's 59.73 Hz and composes the
  view; it parks when another saver is showing and the game waits where it
  was. The render thread only takes the last finished view, and skips it if
  the thread is writing it.
- An mGBA abort, or a SIGSEGV or SIGBUS inside mGBA, lands on a setjmp
  boundary in `gba/afterglow_gba.c` (`abort` is defined to it; the signals
  have a handler that chains to the previous one outside the boundary), and
  a panic in the pilot or the view is caught at the frame boundary: that ROM
  is dropped for the next and not tried again. A ROM mGBA will not load is
  skipped the same way; with none left, the saver shows static. A core that
  hangs instead of faulting stalls only the `gba` thread.
- The view is scaled straight onto the panel's pixels, only the rows that
  changed, as `gameboy` and `doom` do: 284 GBA pixels wide at 16:9, 512 on
  pine's 3.2:1.

The emulator thread, per frame (core, pilot and view), over the first two
minutes of each game from power-on, and ten minutes for the Pokémon:

| Game            | M-series Mac | Pi 5 (pine) | of a Pi 5 core |
| --------------- | ------------ | ----------- | -------------- |
| FireRed, 10 min | 0.52 ms      | 1.24 ms     | 74m            |
| Emerald, 10 min | 0.32 ms      | 0.74 ms     | 44m            |
| Golden Sun      | 0.38 ms      | 0.88 ms     | 52m            |
| The Minish Cap  | 0.26 ms      | 0.65 ms     | 39m            |

The Pi numbers are the static aarch64 musl test binary run on pine itself
(Cortex-A76) with a one-core limit. Even FireRed, the heaviest, is a seventh
of the 500m the whole app gets. `GBA_FPS` runs the game slower, and cheaper
in proportion, if that is ever needed.

```sh
GBA_TEST_ROM=/path/to/rom.gba cargo test --release --features gba bench_core -- --ignored --nocapture
```

Source: [`src/gba/`](../../src/gba/mod.rs),
[`gba/afterglow_gba.c`](../../gba/afterglow_gba.c).

## Knobs

- `GBA_ROM`: a `.gba` file or a folder of them (default `/roms/gba`).
- `GBA_BIOS`: a GBA BIOS to boot through (default none: mGBA's HLE BIOS).
- `GBA_FPS`: emulated frames a second, 1..60 (default 60, full speed).
  Lower is slow motion for less CPU.
- `GBA_ROTATE_SECS`: seconds per ROM when there are several, 0..86400
  (default 900; 0 never moves on).
- `GBA_STATE_SECS`: seconds between save states of the playing ROM,
  0..86400 (default 300; 0 saves only when it rotates away or the saver
  switches).
- `GBA_SEED`: pins which ROM plays first and the pilot's rolls (default:
  from the clock).

## Tests

`cargo test --features gba` plays a ROM the tests assemble themselves (19
ARM instructions of crawling stripes), so CI needs no ROM. With
`GBA_TEST_ROM` set to a FireRed, LeafGreen or Emerald, it also checks the
pilot reaches the overworld.
