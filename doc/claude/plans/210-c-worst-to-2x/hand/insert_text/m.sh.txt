#!/bin/bash
# m.sh BIN... : hash + ns/op (3 runs) + instructions/cycles, pinned to CPU 2
for b in "$@"; do
  r=$(for i in 1 2 3; do taskset -c 2 ./$b --n 200 | grep insert_text | cut -f4; done | sort -n | head -1)
  h=$(taskset -c 2 ./$b --n 2 | grep insert_text | cut -f7)
  st=$(taskset -c 2 perf stat -x, -e instructions:u,cycles:u -r 3 ./$b --n 100 2>&1 >/dev/null | grep cpu_core | cut -d, -f1 | tr '\n' ' ')
  echo "$b hash=$h ns_op=$r instr/cycles=$st"
done
