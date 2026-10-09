# Third-party notices

afterglow is MIT-licensed (see `LICENSE`). Two parts of it derive from other
people's work, and the GPL image (`-gpl`, also tagged `-doom`) adds three
more, below.

## GNU Unifont glyphs

`src/font.rs` is generated from `tools/unifont-subset.hex`, a subset of GNU
Unifont's `.hex` bitmaps. Of Unifont's dual licence, the SIL Open Font License
1.1 arm is elected; the full text and the reserved-name note are in
`tools/LICENSE.unifont`. The derived table is not called Unifont.

## ascii.rest pieces

`src/ascii_rest/` ports pieces from [ascii.rest](https://github.com/bas3line/ascii),
`src/plasma.rs` started as its plasma, and `tools/ascii-rest-golden.ts` renders goldens with its code. Used under:

```
MIT License

Copyright (c) 2026 bas3line (https://github.com/bas3line)

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## The GPL image

Only a `--features doom` or `--features micropolis` build compiles any of
the code below. The image built with both, published as the `-gpl` tags and
under the older `-doom` tags too, is a combined work distributed under the GNU
General Public License v3: Micropolis is GPL-3.0-or-later, and doomgeneric's
GPL-2.0-or-later allows that. Its complete corresponding source is this
repository at the commit the tag names (`sha-<commit>-gpl`). The image carries
the licences under `/licenses/`. afterglow's own source stays MIT. The default
build and image contain none of it.

## doomgeneric (GPL image only)

`doom/doomgeneric/` is [doomgeneric](https://github.com/ozkl/doomgeneric) at
`dcb7a8d`, id Software's Doom by way of Chocolate Doom, under the GNU General
Public License v2 or later (`doom/doomgeneric/LICENSE`). `doom/afterglow_doom.c`
links against it and is under the same licence. The engine is modified, each
change marked `afterglow`: `g_game.c` calls the autopilot, `i_video.c` records
which palette is up and takes its width at run time, and the renderer, video
and UI files draw a Hor+ widescreen view with the 320-wide UI centred, in the
manner of [Crispy Doom](https://github.com/fabiangreffrath/crispy-doom) (also
GPL-2.0). `i_scale.c`, unused here, is removed.

## Freedoom (GPL image only)

The GPL image carries `freedoom1.wad` from
[Freedoom](https://freedoom.github.io/) Phase 1 v0.13.0, fetched and checksummed
by `tools/freedoom.sh`. Its notice, also at `/licenses/freedoom-COPYING.txt` in
the image:

```
Copyright © 2001-2024
Contributors to the Freedoom project.  All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

  * Redistributions of source code must retain the above copyright
    notice, this list of conditions and the following disclaimer.
  * Redistributions in binary form must reproduce the above copyright
    notice, this list of conditions and the following disclaimer in the
    documentation and/or other materials provided with the distribution.
  * Neither the name of the Freedoom project nor the names of its
    contributors may be used to endorse or promote products derived from
    this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS “AS
IS” AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED
TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A
PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER
OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF
LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

## Micropolis (GPL image only)

`micropolis/engine/` is the simulation engine of
[MicropolisCore](https://github.com/SimHacker/micropolis) at `c98f6b0`,
Electronic Arts' 2008 GPL release of SimCity Classic as reworked by Don
Hopkins, under the GNU General Public License v3 or later with EA's additional
terms under section 7 (in each source file's header; the licence text is
`micropolis/COPYING`). Among those terms: no right to the SimCity trademark is
granted, and modified versions must be marked as such. This one is modified,
each change marked `afterglow`: `afterglow.h` (new) routes every assert and
fatal path to the glue's setjmp boundary, `fileio.cpp` opens the bundled
cities from memory, `micropolis.h` lets the glue read private state, and
`simulate.cpp` clamps industrial demand where upstream clamped it into
residential's by mistake. `micropolis/afterglow_micropolis.cpp` links against
the engine and is under the same licence.

`micropolis/tiles.xpm` (the 16x16 tile set) and the sample cities in
`micropolis/cities/` come from the same release and licence.

Micropolis is a registered trademark of Micropolis Corporation (Micropolis
GmbH) and is licensed here as a courtesy of the owner
([micropolis.com](https://www.micropolis.com)), under the "Micropolis" Public
Name License in `micropolis/MicropolisPublicNameLicense.txt`. SimCity is a
trademark of Electronic Arts, which has no part in this.
