#!/bin/bash
# m.sh BIN [N] — instructions, cycles (P-core counters), ns/op, hash on CPU 8; per-op = (N - N1)
B=$1; N=${2:-100}
taskset -c 8 perf stat -x, -e cpu_core/instructions/u,cpu_core/cycles/u -r 5 ./$B --n $N 2>/tmp/m.$$ >/tmp/o.$$
I=$(grep instructions /tmp/m.$$ | cut -d, -f1); C=$(grep cycles /tmp/m.$$ | cut -d, -f1)
R=$(grep -v '^routine\|^time' /tmp/o.$$ | head -1 | cut -f4,7 | tr '\t' ' ')
printf "%-10s N=%-4s instr %13s  cycles %12s  ns_op,hash %s\n" $B $N $I $C "$R"
rm -f /tmp/m.$$ /tmp/o.$$
