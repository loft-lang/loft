#!/bin/sh
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
# The pilot of @PLN221: runs every cell gen.py wrote and compares the counts it prints with
# expect.txt.  LOFT names the binary; $1 is a backend flag such as --native-c.
while IFS="$(printf '\t')" read n e; do
  out=$(LOFT_TIMEOUT=120 ${LOFT:-loft} $1 $n.loft 2>&1 | tail -4)
  line=$(printf '%s\n' "$out" | grep "^$n " | head -1)
  if [ -z "$line" ]; then echo "ERR  $n :: $(printf '%s' "$out" | tr '\n' '|' | cut -c1-230)"; continue; fi
  ok=1; for kv in $e; do case "$line" in *" $kv "*) ;; *) ok=0;; esac; done
  [ $ok = 1 ] && echo "ok   $n" || echo "FAIL $n want[$e] got[$line]"
done < expect.txt
