# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN91 strand 12 — what the cgen_*.sh helpers share; SOURCED, not run.
#
#   cgen_root    this repository
#   cgen_out     $LOFT_CGEN_OUT (default $TMPDIR/loft-cgen): built generators, units, logs
#   cgen_loft    the loft binary the helpers drive (target/release/loft)
#   cgen_plain   export every switch of cgen_w.loft's PLAIN_SWITCHES (=1)
#   cgen_cwd F   the directory a program runs from, as the test suite runs it: a
#                tests/scripts program from the repository root (`19-files.loft` reads
#                tests/example), every other one from its own directory
#   cgen_suite   point LOFT_SUITE_DIR at the language suite at the commit tests/loft-suite.pin
#                names (a detached worktree of ../loft-suite under $cgen_out/suite), unless
#                LOFT_SUITE_DIR is already set — the checkout beside this tree can be far
#                from the pin, and a stale cell there fails on every loft

cgen_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cgen_out="${LOFT_CGEN_OUT:-${TMPDIR:-/tmp}/loft-cgen}"
cgen_loft="$cgen_root/target/release/loft"
mkdir -p "$cgen_out"

cgen_plain() {
  local n
  for n in $(grep -E '^PLAIN_SWITCHES = \[' "$cgen_root/selfhost/src/cgen_w.loft" | grep -oE 'LOFT_NO_[A-Z0-9_]+'); do
    export "$n=1"
  done
}

cgen_cwd() {
  case "$1" in
    */tests/scripts/*) echo "${1%%/tests/scripts/*}" ;;
    *) dirname "$1" ;;
  esac
}

cgen_suite() {
  [ -n "${LOFT_SUITE_DIR:-}" ] && return 0
  local pin src wt
  pin="$(tr -d '[:space:]' < "$cgen_root/tests/loft-suite.pin")"
  src="$(dirname "$cgen_root")/loft-suite"
  wt="$cgen_out/suite"
  if [ ! -d "$wt" ]; then
    git -C "$src" cat-file -e "$pin^{commit}" 2>/dev/null || git -C "$src" fetch -q origin
    git -C "$src" worktree add -q --detach "$wt" "$pin" || return 1
  elif [ "$(git -C "$wt" rev-parse HEAD)" != "$pin" ]; then
    # Our own throwaway worktree: move it only when it holds nothing of anybody's.
    if [ -n "$(git -C "$wt" status --porcelain)" ]; then
      echo "cgen_suite: $wt has local changes; not moving it to the pin $pin" >&2
      return 1
    fi
    git -C "$wt" cat-file -e "$pin^{commit}" 2>/dev/null || git -C "$src" fetch -q origin
    git -C "$wt" checkout -q --detach "$pin" || return 1
  fi
  export LOFT_SUITE_DIR="$wt"
}
