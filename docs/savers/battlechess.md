# `battlechess`

Battle Chess (Interplay, 1988) on an emulated Macintosh Plus, the Mac playing
both sides, one game after another, capture fights and all. The Mac's 512x342
screen fills the panel's height with its pixels square, a dark bezel and the
Finder's grey desktop pattern beside it.

**Only in the GPL image** (`ghcr.io/northisup/afterglow:latest-gpl`,
`:sha-<commit>-gpl`), and only with your own files: no ROM, System or game is
bundled, downloaded or cached, and none ever will be. Without them the saver
shows a card naming what is missing.

## The files

| Variable     | Default                     | What                                                                               |
| ------------ | --------------------------- | ---------------------------------------------------------------------------------- |
| `MAC_ROM`    | `/roms/mac/Mac-Plus.ROM`    | A Mac Plus ROM (any of the three revisions). Only the first 128 KiB are read.      |
| `MAC_SYSTEM` | `/roms/mac/System7_5_3.img` | A bootable raw HFS disk image with a System that runs on a 4 MB Plus (7.5.3 does). |
| `MAC_DISK`   | `/roms/mac/BattleChess.img` | A raw HFS disk image holding the `Battle Chess` application at its top level.      |

Disk images are the raw volumes Mini vMac uses (not DiskCopy). They are read
in place and never written: the Mac's own writes stay in memory and are
dropped on every reboot. On pine they live on the `screensaver-roms` volume,
mounted read-only at `/roms`:

```
/roms/mac/Mac-Plus.ROM
/roms/mac/System7_5_3.img
/roms/mac/BattleChess.img
```

`BATTLECHESS_TINT` colours the 1-bit screen: `paper` (default), `white`,
`amber`, `green` or `blue`.

## How it plays

The emulator is [Mini vMac](https://www.gryphel.com/c/minivmac/) 36.04,
configured as a 4 MB Mac Plus. It runs in `mac-engine`, a program of its own
beside the screensaver (see [`THIRD_PARTY.md`](../../THIRD_PARTY.md) for why),
which the screensaver starts and talks to over a pipe, one exchange per
sixtieth of a second, and starts again if it dies.

The screensaver works the mouse and keyboard from the screen alone. It boots
the Mac at 8x, types the game disk's name in the Finder and opens it, opens
Battle Chess, then drops to the Mac's own speed and chooses **Settings → Mac
White** (Mac Black is the game's default). When a game ends on its "Check and
mate" alert it presses Return, **File → New Game** (Command-N) and confirms.
Anything that does not move the screen when it should is retried; a game that
keeps stalling, or five minutes of a still screen, reboots the Mac.

A switch away pauses the Mac where it is; a switch back carries on.

## Cost

The Mac at 1x measured about 1% of one core of an Apple M-series laptop;
expect a few per cent on a Pi 5. The screensaver redraws only the rows that
changed. The boot at 8x takes most of a core for its ten or so seconds.
