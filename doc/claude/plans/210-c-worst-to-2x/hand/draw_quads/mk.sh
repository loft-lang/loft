#!/bin/bash
# mk.sh N : apply step-1..N patches cumulatively to base.c -> sN.c, build gcc sN and clang sN_cl, check hash
N=$1; cp base.c s$N.c
for i in $(seq 1 $N); do python3 step-$i.py s$N.c || exit 1; done
gcc -std=c99 -O3 -o s$N s$N.c -lm -ldl -pthread 2>/dev/null || { echo gcc fail; exit 1; }
clang -std=c99 -O3 -o s${N}_cl s$N.c -lm -ldl -pthread 2>/dev/null || { echo clang fail; exit 1; }
for b in s$N s${N}_cl; do taskset -c 16 ./$b --n 5 | awk -F'\t' -v b=$b '$1=="draw_quads"{print b, "hash", $7}'; done
