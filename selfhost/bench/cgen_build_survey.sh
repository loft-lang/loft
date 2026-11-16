#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN91 strand 12, U-0/U-1 — one round: snapshot the generator's sources, build the snapshot
# with `--ship`, then run every corpus program and every bench through its C
# (`cgen_survey.py --run --plain`), the corpus against the PINNED language suite.
#
#   selfhost/bench/cgen_build_survey.sh [N] [--check]
#
#   N        the round's number (default: one past the highest snapN there); everything lands in
#            $LOFT_CGEN_OUT as snapN/ (the sources), genN/cgen (the generator), buildN.log,
#            corpusN.txt and benchN.txt, whose first lines it prints
#   --check  hold both runs to selfhost/bench/cgen_survey.baseline (cgen_survey.py --check)
#
# The snapshot is what lets the port go on while the round runs (~6 minutes to build, about an
# hour for both surveys on 4 + 2 jobs): the build and the survey read the copy, not the tree.
set -u
. "$(dirname "$0")/cgen_env.sh"
n=""
check=""
for a in "$@"; do
  case "$a" in
    -h|--help) sed -n '5,17p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    --check) check="--check" ;;
    *) n="$a" ;;
  esac
done
if [ -z "$n" ]; then
  n=$(( $(ls -d "$cgen_out"/snap[0-9]* 2>/dev/null | sed 's/.*snap//' | sort -n | tail -1) + 1 ))
fi
snap="$cgen_out/snap$n"
gen="$cgen_out/gen$n"
rm -rf "$snap"
mkdir -p "$snap"
cp -r "$cgen_root/selfhost/src" "$cgen_root/selfhost/front" "$snap/"
cgen_suite || exit 2
export TMPDIR="$cgen_out"
cd "$cgen_root" || exit 2
if ! LOFT_NO_CACHE=1 LOFT_TIMEOUT=1500 "$cgen_loft" --native-c --ship --out "$gen" \
    --lib "$snap/src" --lib selfhost/crt/src "$snap/front/cgen.loft" > "$cgen_out/build$n.log" 2>&1 \
    || [ ! -x "$gen/cgen" ]; then
  echo "cgen_build_survey: the generator did not build ($cgen_out/build$n.log)"
  tail -20 "$cgen_out/build$n.log"
  exit 1
fi
rc=0
timeout 3600 python3 selfhost/bench/cgen_survey.py --run --plain --generator "$gen/cgen" --jobs 4 \
  --show refused --show fail --show nobuild $check > "$cgen_out/corpus$n.txt" 2>&1 || rc=1
head -1 "$cgen_out/corpus$n.txt"
timeout 3000 python3 selfhost/bench/cgen_survey.py --run --bench --plain --generator "$gen/cgen" --jobs 2 \
  --show refused --show fail $check > "$cgen_out/bench$n.txt" 2>&1 || rc=1
head -1 "$cgen_out/bench$n.txt"
[ -n "$check" ] && grep -h "^baseline" "$cgen_out/corpus$n.txt" "$cgen_out/bench$n.txt"
exit $rc
