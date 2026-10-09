#!/bin/sh
# Prints the path to freedoom1.wad (Freedoom Phase 1 v0.13.0, BSD-3-Clause),
# downloading and checksumming it into $1 (default target/freedoom) once.
set -eu
dir=${1:-target/freedoom}
wad=$dir/freedoom1.wad
url=https://github.com/freedoom/freedoom/releases/download/v0.13.0/freedoom-0.13.0.zip
sum=3f9b264f3e3ce503b4fb7f6bdcb1f419d93c7b546f4df3e874dd878db9688f59
if [ ! -f "$wad" ]; then
    mkdir -p "$dir"
    zip=$dir/freedoom-0.13.0.zip
    if command -v curl >/dev/null; then curl -fsSL -o "$zip" "$url"; else wget -qO "$zip" "$url"; fi
    if command -v sha256sum >/dev/null; then
        echo "$sum  $zip" | sha256sum -c - >/dev/null
    else
        echo "$sum  $zip" | shasum -a 256 -c - >/dev/null
    fi
    unzip -p "$zip" freedoom-0.13.0/COPYING.txt >"$dir/COPYING.txt"
    unzip -p "$zip" freedoom-0.13.0/freedoom1.wad >"$wad.part"
    mv "$wad.part" "$wad"
    rm "$zip"
fi
echo "$wad"
