# Third-party notices

afterglow is MIT-licensed (see `LICENSE`). Two parts of it derive from other
people's work, and the `-doom` image adds two more, below.

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

## doomgeneric (`-doom` image only)

`doom/doomgeneric/` is [doomgeneric](https://github.com/ozkl/doomgeneric) at
`dcb7a8d`, id Software's Doom by way of Chocolate Doom, under the GNU General
Public License v2 or later (`doom/doomgeneric/LICENSE`). `doom/afterglow_doom.c`
links against it and is under the same licence. The engine is modified, each
change marked `afterglow`: `g_game.c` calls the autopilot, `i_video.c` records
which palette is up and takes its width at run time, and the renderer, video
and UI files draw a Hor+ widescreen view with the 320-wide UI centred, in the
manner of [Crispy Doom](https://github.com/fabiangreffrath/crispy-doom) (also
GPL-2.0). `i_scale.c`, unused here, is removed.

Only a `--features doom` build compiles any of it. That build, published as the
`-doom` image tags, is a combined work distributed under the GPL v2: its
complete corresponding source is this repository at the commit the tag names
(`sha-<commit>-doom`). afterglow's own source stays MIT. The default build and
image contain none of it.

## Freedoom (`-doom` image only)

The `-doom` image carries `freedoom1.wad` from
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
