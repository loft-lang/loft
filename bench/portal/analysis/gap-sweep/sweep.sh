#!/bin/bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
# Run scripts/gap_report.py over every routine in jobs.tsv, one at a time (this box: one heavy
# job), into OUT (default /tmp/gap-sweep).  Then: summary.py OUT (each row in twin units),
# mechanisms.py OUT (the inlined runtime by the function it came from).
here=$(cd "$(dirname "$0")" && pwd)
out=${1:-/tmp/gap-sweep}
mkdir -p "$out"
cd "$here/../../../.." || exit 1
: > "$out/status.tsv"
grep -v '^#' "$here/jobs.tsv" | while IFS=$'\t' read -r id arg fn; do
  f=$out/$(echo "$id" | tr / _)
  t0=$(date +%s)
  # shellcheck disable=SC2086  # `arg` is two words: the lane flag and its value
  timeout 1500 python3 scripts/gap_report.py $arg --fn "$fn" --out "$f.txt" > "$f.log" 2>&1 < /dev/null
  echo -e "$id\t$?\t$(( $(date +%s) - t0 ))s" >> "$out/status.tsv"
done
echo done >> "$out/status.tsv"
