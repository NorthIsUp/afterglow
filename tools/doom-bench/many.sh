#!/bin/sh
# Play every map of an episode TRIALS times in parallel and count exits.
# Usage: tools/doom-bench/many.sh [EPISODE] [TRIALS] [MINUTES] [SKILL] [GOD]
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
ep=${1:-1} trials=${2:-4} minutes=${3:-5} skill=${4:-4} god=${5:-1}
"$root/tools/doom-bench/run.sh" 0 "$skill" "$god" E9M9 >/dev/null 2>&1 || true
wad=$(cd "$root" && tools/freedoom.sh)
case "$wad" in /*) ;; *) wad="$root/$wad" ;; esac
for m in 1 2 3 4 5 6 7 8 9; do for t in $(seq 1 "$trials"); do echo "E${ep}M$m $t"; done; done |
    xargs -P "$(sysctl -n hw.ncpu 2>/dev/null || nproc)" -n 2 sh -c \
        '"$0" "$1" "$2" "$3" "$4" "$5" -seed "$6" 2>/dev/null | grep -E "^E[0-9]"' \
        "$root/target/doom-bench/bench" "$wad" "$minutes" "$skill" "$god" |
    sort | awk '{n[$1]++; if ($2 == "EXIT") { e[$1]++; t[$1] += $3 } k[$1] += substr($5, 1, index($5, "/") - 1); s[$1] += substr($7, 1, index($7, "/") - 1); dd[$1] += $9; h[$1] += $11; ev[$1] += $13; ms[$1] += $15; if ($17 > mx[$1]) mx[$1] = $17}
        END { for (m in n) printf "%s  exits %d/%d  mean exit time %6.1f s  mean kills %5.1f  mean secrets %4.1f  deaths %4.1f  hops %4.1f  line-eval %.2f ms  worst plan %.2f ms\n", m, e[m], n[m], e[m] ? t[m] / e[m] : 0, k[m] / n[m], s[m] / n[m], dd[m] / n[m], h[m] / n[m], ev[m] ? ms[m] / ev[m] : 0, mx[m] }' | sort
