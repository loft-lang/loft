#!/bin/bash
# m.sh BIN N [DIR] — per-op instructions/cycles (5 repeats, n=N minus n=1), pinned to CPU 14.
B=$(readlink -f $1); N=$2; D=${3:-pkg}
cd $(dirname $0)/$D
run() { LC_ALL=C taskset -c 14 perf stat -x, -e cpu_atom/instructions/u,cpu_atom/cycles/u -r 5 $B --n $1 2>&1 >/tmp/claude-1000/m_$$.txt | awk -F, '{print $1}' | tr '\n' ' '; }
read i1 c1 <<< "$(run 1)"; read iN cN <<< "$(run $N)"
row=$(tail -1 /tmp/claude-1000/m_$$.txt)
echo "$(basename $B): instr/op=$(( (iN-i1)/(N-1) )) cyc/op=$(( (cN-c1)/(N-1) )) ns_op=$(echo "$row"|cut -f4) hash=$(echo "$row"|cut -f7)"
rm -f /tmp/claude-1000/m_$$.txt
