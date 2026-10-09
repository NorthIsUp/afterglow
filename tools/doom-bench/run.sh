#!/bin/sh
# Build the headless autopilot bench and play maps with it.
# Usage: tools/doom-bench/run.sh [MINUTES] [SKILL] [GOD] [ExMy... | all] [-frames DIR]
# Defaults: 5 game-minutes per map, skill 4 (ultra-violence), god on, E1M1..E1M9.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
out="$root/target/doom-bench"
mkdir -p "$out"
cc -O2 ${AP_CFLAGS:-} -w -fno-common -fno-strict-aliasing -DCMAP256 -DDOOMGENERIC_RESX=320 -DDOOMGENERIC_RESY=200 \
    -DNORMALUNIX -DLINUX -D_DEFAULT_SOURCE -Dexit=dg_exit -I"$root/doom/doomgeneric" \
    "$root"/doom/doomgeneric/*.c "$root"/doom/*.c "$root/tools/doom-bench/bench.c" -lm -o "$out/bench"
wad=$(cd "$root" && tools/freedoom.sh)
case "$wad" in /*) ;; *) wad="$root/$wad" ;; esac
minutes=${1:-5} skill=${2:-4} god=${3:-1}
shift $(($# < 3 ? $# : 3))
exec "$out/bench" "$wad" "$minutes" "$skill" "$god" "$@"
