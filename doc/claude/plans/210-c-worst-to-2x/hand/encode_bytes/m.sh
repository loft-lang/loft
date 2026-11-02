#!/bin/bash
# m.sh BIN N — min ns/op over 7 runs + instructions/cycles (perf stat -r 3), pinned to CPU 10
B=$1; N=${2:-40}
best=""; hash=""
for i in 1 2 3 4 5 6 7; do
  l=$(taskset -c 10 ./$B --n $N | sed -n 2p); ns=$(echo "$l" | cut -f4); hash=$(echo "$l" | cut -f7)
  if [ -z "$best" ] || [ "$ns" -lt "$best" ]; then best=$ns; fi
done
st=$(LC_ALL=C taskset -c 10 perf stat -x, -e cpu_core/instructions/u,cpu_core/cycles/u -r 3 ./$B --n $N 2>&1 >/dev/null | awk -F, '{printf "%s=%.3fG ", $3, $1/1e9}')
echo "$B ns_op=$best hash=$hash $st"
