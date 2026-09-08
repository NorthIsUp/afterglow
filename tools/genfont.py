#!/usr/bin/env python3
"""Emit src/font.rs — the glyph atlas every saver blits.

Run: tools/genfont.py tools/unifont-subset.hex -o src/font.rs

Two sources, deliberately:

* The ten fire ramp glyphs are the 8x8 bitmaps that shipped in main.rs, ROW
  DOUBLED to 8x16. That makes the atlas one shape while leaving fire's output
  pixel-identical: the blit samples bits[py * 16 / cell_h], and flooring an
  already-floored index by two is the same as flooring the original by 2*cell,
  so bits16[py * 16 / cell] == bits8[py * 8 / cell] for every cell size.
* Everything else comes from GNU Unifont's .hex, which is already a bitmap —
  `CODEPOINT:HEXROWS`, 32 hex digits = 16 rows of one byte, MSB leftmost. That
  is byte-for-byte what the blit consumes, so there is no rasteriser here and no
  font library in the build stage.

Mirroring is baked in per glyph rather than flagged at runtime: the katakana are
the film's "back to front" forms and the digits are not, so it is a property of
the data, not a switch. Baking it means font.rs shows what the panel shows.

The output is committed. CI re-runs this and diffs, so the table cannot drift
from its source without failing the build.
"""

import argparse
import hashlib
import sys

# The film's Reloaded/Revolutions glyph order, recovered from production art
# (Rezmason/matrix `glyph order.txt`). 57 slots, 56 distinct glyphs: 33 katakana,
# nine distinct digits (there is no 6; 0 occupies two slots), one lowercase z,
# and 13 symbols including a space.
#
# The source list is fullwidth U+30xx, which Unifont draws 16 wide — that breaks
# the one-byte-per-row blit. The HALFWIDTH forms carry the same letterforms at
# 8x16, which is what a 16x32 screen cell wants anyway (exact 2x, no stretching).
MATRIX_KANA = {
    "モ": 0xFF93, "エ": 0xFF74, "ヤ": 0xFF94, "キ": 0xFF77, "オ": 0xFF75,
    "カ": 0xFF76, "ケ": 0xFF79, "サ": 0xFF7B, "ス": 0xFF7D, "ヨ": 0xFF96,
    "タ": 0xFF80, "ワ": 0xFF9C, "ネ": 0xFF88, "ヌ": 0xFF87, "ナ": 0xFF85,
    "ヒ": 0xFF8B, "ホ": 0xFF8E, "ア": 0xFF71, "ウ": 0xFF73, "セ": 0xFF7E,
    "ミ": 0xFF90, "ラ": 0xFF97, "リ": 0xFF98, "ツ": 0xFF82, "テ": 0xFF83,
    "ニ": 0xFF86, "ハ": 0xFF8A, "ソ": 0xFF7F, "コ": 0xFF7A, "シ": 0xFF7C,
    "マ": 0xFF8F, "ム": 0xFF91, "メ": 0xFF92,
}

MATRIX_ORDER = "モエヤキオカ7ケサスz152ヨタワ4ネヌナ98ヒ0ホア3ウ セ¦:\"꞊ミラリ╌ツテニハソ▪—<>0|+*コシマムメ"

# Unifont draws a handful of these outside 8x16 in some releases. Each carries a
# documented stand-in so a font bump degrades to a near-identical mark instead of
# silently shipping a garbled row.
FALLBACK = {
    0x00A6: 0x007C,  # broken bar -> pipe
    0xA78A: 0x003D,  # modifier short equals -> equals
    0x254C: 0x2500,  # light double dash -> light horizontal
    0x25AA: 0x002E,  # small black square -> period
    0x2014: 0x2500,  # em dash -> light horizontal
}

# Fire's intensity ramp, cool/empty -> hot/dense, and the 8x8 bitmaps it blits.
# Verbatim from the pre-refactor main.rs; changing a byte here changes fire.
RAMP_CHARS = " .:-=+*#%@"
FIRE_8X8 = {
    " ": [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
    ".": [0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x18, 0x00],
    ":": [0x00, 0x18, 0x18, 0x00, 0x00, 0x18, 0x18, 0x00],
    "-": [0x00, 0x00, 0x00, 0x7E, 0x00, 0x00, 0x00, 0x00],
    "=": [0x00, 0x00, 0x7E, 0x00, 0x7E, 0x00, 0x00, 0x00],
    "+": [0x00, 0x18, 0x18, 0x7E, 0x18, 0x18, 0x00, 0x00],
    "*": [0x00, 0x66, 0x3C, 0xFF, 0x3C, 0x66, 0x00, 0x00],
    "#": [0x66, 0xFF, 0x66, 0x66, 0x66, 0xFF, 0x66, 0x00],
    "%": [0xC6, 0xCC, 0x18, 0x30, 0x66, 0xC6, 0x00, 0x00],
    "@": [0x3C, 0x42, 0x99, 0xA5, 0xA5, 0x9E, 0x40, 0x3C],
}


def load(path):
    table = {}
    for line in open(path, encoding="ascii"):
        line = line.strip()
        if not line:
            continue
        cp, _, data = line.partition(":")
        table[int(cp, 16)] = data
    return table


def unifont(table, cp, mirror):
    """One 8x16 glyph, resolving the fallback and failing loudly past it."""
    for candidate in (cp, FALLBACK.get(cp)):
        if candidate is None:
            break
        data = table.get(candidate)
        if data is None:
            continue
        if len(data) != 32:
            print(
                f"warning: U+{candidate:04X} is {len(data) // 4}x{len(data) // 2}, "
                f"not 8x16; trying the fallback",
                file=sys.stderr,
            )
            continue
        rows = [int(data[i * 2 : i * 2 + 2], 16) for i in range(16)]
        if mirror:
            rows = [int(f"{b:08b}"[::-1], 2) for b in rows]
        return rows, candidate
    raise SystemExit(f"U+{cp:04X}: no 8x16 glyph and no usable fallback")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("hexfile")
    ap.add_argument("-o", "--out", default="-")
    args = ap.parse_args()

    raw = open(args.hexfile, "rb").read()
    digest = hashlib.sha256(raw).hexdigest()
    table = load(args.hexfile)

    glyphs, notes, seen = [], [], {}

    def add(rows, note):
        """Intern one bitmap. Identical bitmaps share a slot, which is how the
        matrix set's two 0 slots and its space collapse into the atlas."""
        key = tuple(rows)
        if key in seen:
            return seen[key]
        idx = len(glyphs)
        seen[key] = idx
        glyphs.append(rows)
        notes.append(note)
        return idx

    blank = add([0x00] * 16, "BLANK")
    solid = add([0xFF] * 16, "SOLID — blocks mode is a glyph grid too")
    assert (blank, solid) == (0, 1), "BLANK and SOLID must be slots 0 and 1"

    ramp = [add([b for b in FIRE_8X8[c] for _ in (0, 1)], f"fire ramp {c!r}") for c in RAMP_CHARS]

    matrix = []
    for ch in MATRIX_ORDER:
        cp = MATRIX_KANA.get(ch, ord(ch))
        # The katakana and the z are the film's mirrored forms; digits and
        # symbols are not — a reversed digit reads as a broken font rather than
        # as alien script, and the symmetric symbols are mirror-invariant anyway.
        mirror = ch in MATRIX_KANA or ch == "z"
        rows, used = unifont(table, cp, mirror)
        note = f"matrix U+{used:04X} {ch!r}" + (" mirrored" if mirror else "")
        matrix.append(add(rows, note))

    w = sys.stdout if args.out == "-" else open(args.out, "w", encoding="utf-8")

    def p(s=""):
        print(s, file=w)

    p("// @generated by tools/genfont.py — do not edit by hand.")
    p("//")
    p(f"// Source: GNU Unifont .hex subset, sha256 {digest}")
    p("// Licensed under the SIL OFL 1.1 arm of Unifont's dual licence; see")
    p("// tools/LICENSE.unifont. Fire's ramp glyphs are this repo's own 8x8")
    p("// bitmaps, row-doubled.")
    p("//")
    p("//! One byte per row, MSB = leftmost pixel. Katakana are stored MIRRORED —")
    p("//! the film's glyphs are drawn back to front — so nothing at runtime has")
    p("//! to know which glyphs are reversed.")
    p()
    p("pub const GLYPH_W: usize = 8;")
    p("pub const GLYPH_H: usize = 16;")
    p()
    # rustfmt would rewrap these into unreadable ragged blocks, and would also
    # make `genfont.py | git diff --exit-code` fail against a formatted tree.
    p("#[rustfmt::skip]")
    p(f"pub const GLYPHS: [[u8; GLYPH_H]; {len(glyphs)}] = [")
    for i, (rows, note) in enumerate(zip(glyphs, notes)):
        body = ", ".join(f"0x{b:02X}" for b in rows)
        p(f"    [{body}], // {i}: {note}")
    p("];")
    p()
    p("/// Unlit cell. A saver that wants a cell dark still paints it.")
    p(f"pub const BLANK: u16 = {blank};")
    p("/// Every pixel lit — a whole cell of one colour.")
    p(f"pub const SOLID: u16 = {solid};")
    p()
    p('/// Fire\'s intensity ramp, " .:-=+*#%@", cool to hot.')
    p("#[rustfmt::skip]")
    p(f"pub const RAMP: [u16; {len(ramp)}] = {ramp!r};")
    p()
    p("/// The film's Reloaded/Revolutions glyph order. Uniform-random selection")
    p("/// over these 57 slots is what the rain draws; the duplicate 0 slot is")
    p("/// faithful, not a bug.")
    p("#[rustfmt::skip]")
    p(f"pub const MATRIX: [u16; {len(matrix)}] = {matrix!r};")

    if args.out != "-":
        w.close()
    print(
        f"{len(glyphs)} glyphs, {len(glyphs) * 16} bytes of table "
        f"({len(matrix)} matrix slots, {len(ramp)} ramp slots)",
        file=sys.stderr,
    )


if __name__ == "__main__":
    main()
