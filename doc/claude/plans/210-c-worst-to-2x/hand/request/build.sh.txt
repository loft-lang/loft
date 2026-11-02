#!/bin/bash
# build.sh X  -> compiles X.c to X with the unit's links
H=/home/jurjen/workspace/loft-c-perf/target-exp/wt-rebase/target-exp/hand/request
L=$(grep -o '/\* link: [^*]*\*/' $H/$1.c | sed 's#/\* link: ##; s# \*/##' | tr '\n' ' ')
gcc -std=c99 -O3 ${CF:-} -o $H/$1 $H/$1.c $L -lm -ldl -pthread 2>/dev/null
