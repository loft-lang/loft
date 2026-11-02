#!/bin/bash
# ps.sh BIN N  -> "instr cycles ns_op hash"  (pinned CPU 0, run from pkg)
cd /home/jurjen/workspace/loft-c-perf/target-exp/wt-rebase/target-exp/hand/graphics/pkg
OUT=$(taskset -c ${CPU:-0} perf stat -x, -e instructions:u,cycles:u -r ${R:-3} ../$1 --n $2 2>/tmp/claude-1000/ps_$$.txt | sed -n 2p)
I=$(grep instructions /tmp/claude-1000/ps_$$.txt | cut -d, -f1); C=$(grep cycles /tmp/claude-1000/ps_$$.txt | cut -d, -f1)
NS=$(echo "$OUT" | cut -f4); H=$(echo "$OUT" | cut -f7)
printf "%-22s instr %6.3f G  cyc %6.3f G  ns/op %10s  hash %s\n" $1 $(echo "$I/1000000000" | bc -l) $(echo "$C/1000000000" | bc -l) "$NS" "$H"
rm -f /tmp/claude-1000/ps_$$.txt
