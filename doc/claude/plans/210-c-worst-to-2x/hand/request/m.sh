#!/bin/bash
# m.sh BIN [N] -> instructions, cycles (perf -r 5) and the row
H=/home/jurjen/workspace/loft-c-perf/target-exp/wt-rebase/target-exp/hand/request
N=${2:-200}; cd $H/pkg
taskset -c 4 $H/$1 --n $N | grep -v '^routine\|^time' | cut -f1,4,7
taskset -c 4 perf stat -x, -e instructions:u,cycles:u -r 5 $H/$1 --n $N 2>&1 >/dev/null | awk -F, '{printf "%s %.3fG  ", $3, $1/1e9} END {print ""}'
