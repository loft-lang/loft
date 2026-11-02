#!/bin/bash
# build.sh SRC.c OUTNAME  -> OUTNAME_gcc OUTNAME_clang
LINKS=$(grep -o '/\* link: [^*]*\*/' $1 | sed 's#/\* link: ##; s# \*/##' | tr '\n' ' ')
gcc -std=c99 -O3 ${XF:-} -o $2_gcc $1 $LINKS -lm -ldl -pthread 2>/dev/null &
clang -std=c99 -O3 ${XF:-} -o $2_clang $1 $LINKS -lm -ldl -pthread 2>/dev/null &
wait
