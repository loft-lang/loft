#!/usr/bin/env bash
# @PLN179 strand 2 — the twin: does a port leave the same world as the script it replaces?
#
#   scripts/script_twin.sh [--files DIR [--by-content]] [--runs N] ORIG PORT [-- ARG...]
#   scripts/script_twin.sh --self-test
#
# Runs ORIG and PORT from the repository root with the same ARGs and compares four
# channels byte for byte: stdout, stderr, exit status and — with --files DIR — every
# file under DIR after each run (DIR is emptied before each run and must hold nothing
# tracked).  --by-content compares the files' contents as a multiset and ignores their
# names, for a script whose numbering follows an unordered `find`.  --runs N times each
# side N times and reports the best wall clock, so the performance verdict is beside the
# behaviour one.  Exit 0 when every channel agrees, 1 on a divergence, 2 on usage.
#
# Kept in bash on purpose: it must run while no loft binary exists, and it is the last
# script this plan ports.  --self-test runs pairs that differ on exactly one channel each
# and fails unless every one is caught — a twin that cannot go red proves nothing.
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

files_dir=""; by_content=0; runs=1
while [ $# -gt 0 ]; do
  case "$1" in
    --files) files_dir="$2"; shift 2 ;;
    --by-content) by_content=1; shift ;;
    --runs) runs="$2"; shift 2 ;;
    --self-test) exec bash "$0" --run-self-test ;;
    --run-self-test) self_test=1; shift ;;
    --) shift; break ;;
    -*) echo "script_twin: unknown option $1" >&2; exit 2 ;;
    *) break ;;
  esac
done

if [ "${self_test:-0}" = 1 ]; then
  fail=0
  t=$(mktemp -d)
  printf '#!/bin/sh\necho same; echo warn >&2; exit 0\n' > "$t/a"; chmod +x "$t/a"
  printf '#!/bin/sh\necho other; echo warn >&2; exit 0\n' > "$t/b_out"; chmod +x "$t/b_out"
  printf '#!/bin/sh\necho same; echo other >&2; exit 0\n' > "$t/b_err"; chmod +x "$t/b_err"
  printf '#!/bin/sh\necho same; echo warn >&2; exit 3\n' > "$t/b_exit"; chmod +x "$t/b_exit"
  printf '#!/bin/sh\nmkdir -p "$1"; echo x > "$1/f"\n' > "$t/w_a"; chmod +x "$t/w_a"
  printf '#!/bin/sh\nmkdir -p "$1"; echo y > "$1/f"\n' > "$t/w_b"; chmod +x "$t/w_b"
  for pair in b_out b_err b_exit; do
    if bash "$0" "$t/a" "$t/$pair" > /dev/null 2>&1; then echo "self-test: $pair NOT caught"; fail=1; else echo "self-test: $pair caught"; fi
  done
  if bash "$0" "$t/a" "$t/a" > /dev/null 2>&1; then echo "self-test: identical pair agrees"; else echo "self-test: identical pair reported as divergent"; fail=1; fi
  if bash "$0" --files "$t/out" "$t/w_a" "$t/w_b" -- "$t/out" > /dev/null 2>&1; then echo "self-test: files NOT caught"; fail=1; else echo "self-test: files caught"; fi
  rm -rf "$t"
  exit $fail
fi

[ $# -ge 2 ] || { sed -n 4,6p "$0" | sed 's/^# //' >&2; exit 2; }
orig="$1"; port="$2"; shift 2
[ "${1:-}" = "--" ] && shift
[ -x "$orig" ] || { echo "script_twin: $orig is not executable" >&2; exit 2; }
[ -x "$port" ] || { echo "script_twin: $port is not executable" >&2; exit 2; }
if [ -n "$files_dir" ] && [ "$(git ls-files "$files_dir" 2>/dev/null | wc -l)" != 0 ]; then
  echo "script_twin: $files_dir holds tracked files; a twin only empties untracked output" >&2; exit 2
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# run SIDE (orig|port) SCRIPT: captures the four channels and the best wall clock.
run_side() {
  local side="$1" script="$2" best=""
  shift 2
  for ((i = 0; i < runs; i++)); do
    [ -n "$files_dir" ] && rm -rf "$files_dir"
    local s e ms
    s=$(date +%s%N)
    "$script" "$@" > "$work/$side.out" 2> "$work/$side.err"
    echo $? > "$work/$side.exit"
    e=$(date +%s%N)
    ms=$(( (e - s) / 1000000 ))
    [ -z "$best" ] || [ "$ms" -lt "$best" ] && best=$ms
  done
  echo "$best" > "$work/$side.ms"
  if [ -n "$files_dir" ]; then
    if [ -d "$files_dir" ]; then
      if [ "$by_content" = 1 ]; then
        find "$files_dir" -type f -print0 | xargs -0 sha256sum 2>/dev/null | cut -d' ' -f1 | sort > "$work/$side.files"
      else
        find "$files_dir" -type f -print0 | sort -z | xargs -0 sha256sum 2>/dev/null > "$work/$side.files"
      fi
    else
      : > "$work/$side.files"
    fi
  fi
}
run_side orig "$orig" "$@"
run_side port "$port" "$@"

rc=0
for ch in out err exit ${files_dir:+files}; do
  if cmp -s "$work/orig.$ch" "$work/port.$ch"; then
    printf '  %-6s same\n' "$ch"
  else
    printf '  %-6s DIFFERS\n' "$ch"; rc=1
    diff "$work/orig.$ch" "$work/port.$ch" | head -12 | sed 's/^/    /'
  fi
done
printf '  time   orig %s ms, port %s ms (best of %s)\n' "$(cat "$work/orig.ms")" "$(cat "$work/port.ms")" "$runs"
exit $rc
