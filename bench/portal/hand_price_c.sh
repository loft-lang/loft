#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Compile an EMITTED C unit the way bench/stats.py's `c` lane compiles it, so a rewrite or a
# runtime change can be PRICED before it is built: emit the bench, copy the unit, edit ONE
# function (or one runtime routine — the runtime is inside the unit) by script, compile both
# with this, and compare them with bench/portal/ab.py.
#
#   loft --emit-c /tmp/b.c --path $PWD/ bench/14_stdlib_vector/bench.loft
#   cp /tmp/b.c /tmp/b_v1.c                    # …edit by script, never by hand
#   bench/portal/hand_price_c.sh /tmp/b.c /tmp/b && bench/portal/hand_price_c.sh /tmp/b_v1.c /tmp/b_v1
#   python3 bench/portal/ab.py /tmp/b /tmp/b_v1 --routine push --subject c-push-window
#
# A price counts only when the hashes agree (ab.py checks).  HAND_PRICE_CC names the compiler
# (default `cc`), HAND_PRICE_CFLAGS replaces the lane's flags (`-std=c99 -O3`); add `-g` for a
# profile.  The ship lane's unit is emitted with LOFT_RELEASE_PASS_PROBE=1 (stats.py `cship`).
set -euo pipefail
if [ $# -ne 2 ]; then
  echo "usage: $0 <emitted.c> <out-binary>" >&2
  exit 2
fi
read -r -a flags <<< "${HAND_PRICE_CFLAGS:--std=c99 -O3}"
"${HAND_PRICE_CC:-cc}" "${flags[@]}" -o "$2" "$1" -lm
