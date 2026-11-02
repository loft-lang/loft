#!/bin/bash
# pi.sh BIN — instructions and cycles per op: (n=80) - (n=40), / 40
B=$1
f() { LC_ALL=C taskset -c 10 perf stat -x, -e cpu_core/instructions/u,cpu_core/cycles/u -r 3 ./$B --n $1 2>&1 >/dev/null | awk -F, '{print $1}' | tr '\n' ' '; }
read i1 c1 <<< "$(f 40)"; read i2 c2 <<< "$(f 80)"
echo "$B instr/op=$(( (i2-i1)/40 )) cycles/op=$(( (c2-c1)/40 ))"
