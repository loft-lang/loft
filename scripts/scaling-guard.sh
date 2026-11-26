#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN229 A3 — does loft's own work grow linearly with a program's size?  Writes a program of N
# functions and its twin of 2N, in two shapes — a call chain (each function calls the one
# before it) and the same functions called flat from `main` — and times loft writing each one's
# C (`--native-c --native-emit`, no C compiler), the best of three runs.  A cost that walks a
# function's callees per function, or every definition per definition, doubles the time more
# than twice over; the bound is 2.5x.
#
#   scripts/scaling-guard.sh [--loft BIN] [--n N]
#
# Silent and exit 0 when every shape stays within the bound; otherwise names the shape, both
# times and the ratio, and exits 1.  A perf lane (the nightly), not the PR gate: it times.
#
# @falsified-at: 92736ae85 — the chain grew 5.4x (8385 ms -> 45642 ms): each function's
#   ownership span, fn-ref guard and call-cycle questions re-walked every callee below it, and
#   the parser's `&`-parameter check walked the chain once per function.
set -u
here=$(cd "$(dirname "$0")/.." && pwd)
loft="$here/target/release/loft"
n=2000
case "${1:-}" in -h|--help) sed -n '5,/^$/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;; esac
while [ $# -gt 0 ]; do
    case "$1" in
        --loft) loft=$2; shift 2 ;;
        --n) n=$2; shift 2 ;;
        *) echo "scaling-guard: unknown argument $1"; exit 2 ;;
    esac
done
work=$(mktemp -d "${TMPDIR:-/tmp}/loft-scaling-XXXXXX")
trap 'rm -rf "$work"' EXIT

gen() { # SHAPE N > program
    python3 - "$1" "$2" <<'EOF'
import sys
shape, n = sys.argv[1], int(sys.argv[2])
out = ["struct R { a: integer, s: text }"]
for i in range(n):
    tail = f"step{i - 1}(x + 1)" if shape == "chain" and i > 0 else "x + 1"
    out.append(f"fn step{i}(x: integer) -> integer {{\n  r = R {{ a: x + {i}, s: \"v{i}\" }};\n"
               f"  if x > {i % 7} {{ r.a * 2 + len(r.s) }} else {{ {tail} }}\n}}")
if shape == "chain":
    out.append(f"fn main() {{\n  println(\"{{step{n - 1}(3)}}\");\n}}")
else:
    body = "\n".join(f"  t += step{i}(t % 9);" for i in range(n))
    out.append(f"fn main() {{\n  t = 0;\n{body}\n  println(\"{{t}}\");\n}}")
print("\n".join(out))
EOF
}

best() { # PROGRAM — the best of three emission times, in milliseconds
    local b=""
    for _ in 1 2 3; do
        local s e
        s=$(date +%s%N)
        LOFT_NO_CACHE=1 LOFT_TIMEOUT=600 "$loft" --native-c --native-emit "$work/out.c" "$1" > "$work/log" 2>&1 \
            || { echo "scaling-guard: loft did not write $1:"; tail -5 "$work/log"; exit 2; }
        e=$(( ($(date +%s%N) - s) / 1000000 ))
        if [ -z "$b" ] || [ "$e" -lt "$b" ]; then b=$e; fi
    done
    echo "$b"
}

bad=0
for shape in chain flat; do
    gen "$shape" "$n" > "$work/$shape-1.loft"
    gen "$shape" $((2 * n)) > "$work/$shape-2.loft"
    t1=$(best "$work/$shape-1.loft")
    t2=$(best "$work/$shape-2.loft")
    # ratio x100, integer: 250 is the bound
    r=$(( t2 * 100 / (t1 > 0 ? t1 : 1) ))
    if [ "$r" -gt 250 ]; then
        bad=1
        echo "scaling-guard: $shape grows $((r / 100)).$((r % 100 / 10))x from $n to $((2 * n)) functions (${t1} ms -> ${t2} ms; bound 2.5x)"
    fi
done
exit $bad
