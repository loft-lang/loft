#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN91 strand 12 — the LIVE generator: selfhost/front/cgen.loft and the sources beside it,
# run on the interpreter (~20 s a program) instead of a `--ship` build (~6 minutes), so a
# change to the port is tried on one program without building it.
#
#   selfhost/bench/cgen_live.sh IMAGE OUT.c SRC_C [--layer L]   a generator, as LOFT_CGEN or
#                                                                 cgen_survey.py --generator take it
#   selfhost/bench/cgen_live.sh --diff FILE.loft                 FILE's unit, the Rust writer's
#                                                                 against the live one's, under the
#                                                                 plain switches: IDENTICAL, or the diff
#                                                                 beside the _MSC_VER split copy
#
# --diff leaves the image and both units in $LOFT_CGEN_OUT/live (cgen_env.sh).
set -u
. "$(dirname "$0")/cgen_env.sh"
case "${1:-}" in
  -h|--help|"") sed -n '5,15p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
esac
live() {
  env LOFT_NO_WARNINGS=1 LOFT_TIMEOUT="${LOFT_TIMEOUT:-600}" "$cgen_loft" --interpret \
    --lib "$cgen_root/selfhost/src" --lib "$cgen_root/selfhost/crt/src" \
    "$cgen_root/selfhost/front/cgen.loft" "$@"
}
if [ "$1" != "--diff" ]; then
  live "$@"
  exit $?
fi
file="$(realpath "${2:?--diff FILE.loft}")"
cgen_plain
export LOFT_NO_WARNINGS=1 TMPDIR="$cgen_out"
s="$cgen_out/live"
mkdir -p "$s"
rm -f "$s/got.c" "$s/want.c"
cd "$(cgen_cwd "$file")" || exit 2
"$cgen_loft" --emit-image "$s/p.store" "$file" || exit 1
"$cgen_loft" --native-c --native-emit "$s/want.c" "$file" || exit 1
time live "$s/p.store" "$s/got.c" "$cgen_root/src/c"
echo "generator rc=$?"
[ -f "$s/got.c" ] || exit 1
if cmp -s "$s/want.c" "$s/got.c"; then
  echo IDENTICAL
  exit 0
fi
# The `_MSC_VER` split copy is not ported yet: name what differs beside it.
python3 "$cgen_root/selfhost/bench/cgen_msvc_diff.py" "$s/want.c" "$s/got.c" 20 \
  && echo "IDENTICAL but for the _MSC_VER split copy"
exit 1
