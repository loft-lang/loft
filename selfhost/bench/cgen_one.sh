#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN91 strand 12 — one function body, the Rust writer's against the loft writer's, as a
# unified diff, under the plain switches (cgen_w.loft's PLAIN_SWITCHES).
#
#   selfhost/bench/cgen_one.sh FILE FUNCTION [LINES]    e.g. tests/scripts/foo.loft n_main
#
# Uses the generator `cgen_cycle.sh` built ($LOFT_CGEN_OUT/gen/cgen; default
# $TMPDIR/loft-cgen) and leaves the Rust unit (o.c), its trace and the loft layer there.
set -u
case "${1:-}" in
  -h|--help|"") sed -n '4,12p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
esac
root="$(cd "$(dirname "$0")/../.." && pwd)"
out="${LOFT_CGEN_OUT:-${TMPDIR:-/tmp}/loft-cgen}"
loft="$root/target/release/loft"
file="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
func="${2:?a function name, as the bodies layer names it (n_main)}"
lines="${3:-70}"
switches=$(python3 - "$root" <<'PY'
import re, sys
src = open(sys.argv[1] + "/selfhost/src/cgen_w.loft").read()
row = re.search(r"^PLAIN_SWITCHES = \[(.*)\];$", src, re.M).group(1)
print(" ".join(x + "=1" for x in re.findall(r'"(LOFT_NO_[A-Z0-9_]+)"', row)))
PY
)
cd "$(dirname "$file")" || exit 2
env $switches LOFT_TRACE_BODIES=1 "$loft" --emit-c "$out/o.c" "$file" 2> "$out/o.trace"
env $switches "$loft" --native-c --emit-image "$out/o.img" "$file" 2>&1 | head -5
env $switches "$out/gen/cgen" "$out/o.img" "$out/g.layer" --layer bodies 2>&1 | head -5
python3 - "$out" "$func" "$lines" <<'PY'
import sys, difflib
out, fn, lines = sys.argv[1], sys.argv[2], int(sys.argv[3])
def body(p):
    for line in open(p, errors="replace").read().splitlines():
        c = line.split("\t")
        if c[0] == "[bodies]" and len(c) > 1 and c[1] == fn:
            return line.split("\\n")
    return ["<none in " + p + ">"]
a, b = body(out + "/o.trace"), body(out + "/g.layer")
for l in list(difflib.unified_diff(a, b, "oracle", "loft", n=2, lineterm=""))[:lines]:
    print(l[:210])
PY
