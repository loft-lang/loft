#!/bin/bash
# ps.sh BIN N  -> instructions cycles (per op, setup subtracted via N=1 run) and printed ns_op
B=$1; N=${2:-400}
one=$(taskset -c 16 perf stat -x, -e cpu_atom/instructions/u,cpu_atom/cycles/u -r 3 ./$B --n 1 2>&1 >/dev/null | awk -F, '{print $1}' | tr '\n' ' ')
many=$(taskset -c 16 perf stat -x, -e cpu_atom/instructions/u,cpu_atom/cycles/u -r 3 ./$B --n $N 2>&1 >/dev/null | awk -F, '{print $1}' | tr '\n' ' ')
ns=$(taskset -c 16 ./$B --n $N | awk -F'\t' '$1=="draw_quads"{print $4, $7}')
python3 -c "
o=list(map(float,'$one'.split())); m=list(map(float,'$many'.split())); n=$N-1
print('%-10s instr/op %10.0f  cycles/op %10.0f  ns_op/hash %s' % ('$B', (m[0]-o[0])/n, (m[1]-o[1])/n, '$ns'))"
