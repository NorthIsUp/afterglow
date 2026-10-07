# Third-party notices

afterglow is MIT-licensed (see `LICENSE`). Two parts of it derive from other
people's work.

## GNU Unifont glyphs

`src/font.rs` is generated from `tools/unifont-subset.hex`, a subset of GNU
Unifont's `.hex` bitmaps. Of Unifont's dual licence, the SIL Open Font License
1.1 arm is elected; the full text and the reserved-name note are in
`tools/LICENSE.unifont`. The derived table is not called Unifont.

## ascii.rest pieces

`src/ascii_rest/` ports pieces from [ascii.rest](https://github.com/bas3line/ascii),
and `tools/ascii-rest-golden.ts` renders goldens with its code. Used under:

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
