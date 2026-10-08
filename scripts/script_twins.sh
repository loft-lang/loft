#!/usr/bin/env bash
# @PLN179 strand 2 — every twin case the repository declares, run and summarised: the tests
# between an original script and its port, where to find them, and whether they still agree.
#
#   scripts/script_twins.sh                 run every case, interpreted and --native
#   scripts/script_twins.sh --list          print the cases (port, case, options, arguments)
#   scripts/script_twins.sh --only <port>   one port's cases (its basename)
#   scripts/script_twins.sh --interpret     the interpreted leg only (no rustc)
#   scripts/script_twins.sh --slow          the cases marked `slow` too (a corpus copy of a
#                                           minute; left out by default and by the CI test)
#
# A case is one line of tests/comparisons/scripts/<port>/cases.tsv — `case`, the twin
# options, the arguments, an optional `slow` — under two header lines naming the original and the port
# (`# orig:`, `# port:`); each is one `script_twin.sh` invocation.  The port runs as its
# `#!` line says, then again through `loft --native`, so a port that answers differently on
# one backend is a red case, never a note.  Exit 0 when every case on every leg agrees.
# why-not-loft: bash with the harness — it must run while no loft binary exists, and it is
# ported with the harness, last.
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

list=0; only=""; legs="interpret native"; slow=0
while [ $# -gt 0 ]; do
  case "$1" in
    --list) list=1; shift ;;
    --only) only="$2"; shift 2 ;;
    --interpret) legs="interpret"; shift ;;
    --slow) slow=1; shift ;;
    *) echo "script_twins: unknown option $1" >&2; exit 2 ;;
  esac
done

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
total=0; red=0
for cases in tests/comparisons/scripts/*/cases.tsv; do
  port_dir=$(basename "$(dirname "$cases")")
  [ -n "$only" ] && [ "$only" != "$port_dir" ] && continue
  orig=$(sed -n 's/^# orig: *//p' "$cases" | head -1)
  port=$(sed -n 's/^# port: *//p' "$cases" | head -1)
  [ -n "$orig" ] && [ -n "$port" ] || { echo "script_twins: $cases names no orig/port" >&2; exit 2; }
  # Warm the port once, unmeasured: its first run on a box installs and builds the registry
  # libraries it names, and that install talks on stderr — the box, not the script.  A
  # compile check resolves them without running anything, so no case's world is touched.
  [ "$list" = 1 ] || loft check "$port" > /dev/null 2>&1 || true
  while IFS= read -r line; do
    case "$line" in ''|'#'*) continue ;; esac
    # `cut`, not `read -r a b c`: an empty options column is two tabs in a row, which
    # `read` collapses (tab is whitespace to IFS), sliding the arguments into the options.
    name=$(printf '%s' "$line" | cut -f1); opts=$(printf '%s' "$line" | cut -f2); args=$(printf '%s' "$line" | cut -f3); mark=$(printf '%s' "$line" | cut -f4)
    if [ "$list" = 1 ]; then
      printf '%-22s %-12s %s -> %s  [%s] %s %s\n' "$port_dir" "$name" "$orig" "$port" "$opts" "$args" "${mark:+($mark)}"
      continue
    fi
    if [ "$mark" = slow ] && [ "$slow" = 0 ]; then
      printf 'skip  %s/%s (slow; --slow runs it)\n' "$port_dir" "$name"
      continue
    fi
    for leg in $legs; do
      total=$((total + 1))
      side="$port"
      if [ "$leg" = native ]; then
        side="$work/native_$port_dir"
        printf '#!/bin/sh\nexec loft --native "%s/%s" "$@"\n' "$PWD" "$port" > "$side"; chmod +x "$side"
      fi
      # shellcheck disable=SC2086 — the options and arguments are word lists by design.
      if out=$(scripts/script_twin.sh $opts "$orig" "$side" -- $args 2>&1); then
        printf 'ok    %s/%s [%s] %s\n' "$port_dir" "$name" "$leg" "$(printf '%s' "$out" | sed -n 's/^  time *//p')"
      else
        red=$((red + 1))
        printf 'RED   %s/%s [%s]\n%s\n' "$port_dir" "$name" "$leg" "$(printf '%s' "$out" | sed 's/^/      /')"
      fi
    done
  done < "$cases"
done
[ "$list" = 1 ] && exit 0
echo "script_twins: $total twin run(s), $red red"
[ "$red" -eq 0 ]
