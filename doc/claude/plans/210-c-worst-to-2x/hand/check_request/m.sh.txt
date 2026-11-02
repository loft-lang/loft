#!/bin/bash
# m.sh BIN [N] — instructions, cycles (perf stat -r 5) and ns/op (median of 5 rows), CPU 6
export LC_ALL=C
B=$(realpath $1); N=${2:-200}
cd $(dirname $0)/pkg
P=$(taskset -c 6 perf stat -x, -e instructions:u,cycles:u -r 5 $B --n $N 2>&1 >/dev/null | grep cpu_core)
I=$(echo "$P" | grep instructions | cut -d, -f1); C=$(echo "$P" | grep cycles | cut -d, -f1)
NS=$(for k in 1 2 3 4 5; do taskset -c 6 $B --n $N | awk -F'\t' '$1=="check_request"{print $4}'; done | sort -g | sed -n 3p)
H=$(taskset -c 6 $B --n 1 | awk -F'\t' '$1=="check_request"{print $7}')
awk -v b=$(basename $B) -v i=$I -v c=$C -v ns=$NS -v h=$H 'BEGIN{printf "%-10s instr %7.3f G  cycles %7.3f G  ns_op %9d  hash %s\n", b, i/1e9, c/1e9, ns, h}'
