#!/usr/bin/env bash
# probe-local — run the ci-probe phases LOCALLY with the exact CI commands +
# env, so cache/timing optimisations can be verified in minutes instead of a
# 20-minute CI round.  Faithfully reproduces loft's INTERNAL cache behaviour
# (the build fingerprint, the Timing object); the
# only thing it can't reproduce is the GitHub actions/cache save/restore layer
# (that's the run_id-key persistence, GitHub-specific).
#
# Usage:
#   scripts/probe-local.sh cold     # wipe loft's native caches first
#   scripts/probe-local.sh warm     # keep them (measure the warm path)
#
# "cold" wipes ONLY loft's own caches (~/.loft/build-cache + the native-fixture
# cache under target/), never the cargo build cache — matching what a fresh CI
# cache key would and would not carry.

set -u
mode="${1:-warm}"
case "$mode" in cold|warm) ;; *) echo "usage: $0 cold|warm" >&2; exit 2 ;; esac

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root" || exit 1

# Same env the CI test/probe job sets.
export LOFT_TMPDIR="$root/target/loft-native-cache"
export LOFT_TIMING_LEDGER="$root/target/_probe_local_timing"
rm -rf "$LOFT_TIMING_LEDGER"          # fresh ledger each run (as CI does)

if [ "$mode" = cold ]; then
  echo "== COLD: wiping loft's native caches (~/.loft/build-cache + fixture cache) =="
  rm -rf ~/.loft/build-cache "$LOFT_TMPDIR"
fi

ledger="$root/target/_probe_local_phases.tsv"
: > "$ledger"

phase() {   # phase <name> <cmd...>
  local name="$1"; shift
  local s e
  s=$(date +%s)
  "$@" > "/tmp/probe_${name}.log" 2>&1
  local rc=$?
  e=$(date +%s)
  printf '%s\t%d\t%d\n' "$name" "$(( e - s ))" "$rc" >> "$ledger"
  printf '  %-10s %3ds  (rc=%d)\n' "$name" "$(( e - s ))" "$rc"
}

echo "== $mode run — phases =="
phase build    cargo build --release --bin loft
phase cdylibs  cargo nextest run --profile ci --test native -E 'test(native_library_suite)'
phase fixtures cargo nextest run --profile ci --test native -E 'test(native_scripts)'
phase interp   cargo nextest run --profile ci --test wrap loft_suite

echo
echo "== per-phase ($mode) =="
total=0
while IFS=$'\t' read -r name secs rc; do
  total=$(( total + secs ))
done < "$ledger"
column -t -s$'\t' "$ledger" | sed 's/^/  /'
printf '  total      %ds\n' "$total"

echo
echo "== loft internal native-compile timing ($mode) =="
if [ -d "$LOFT_TIMING_LEDGER" ] && [ -n "$(ls -A "$LOFT_TIMING_LEDGER" 2>/dev/null)" ]; then
  all=$(cat "$LOFT_TIMING_LEDGER"/*.tsv)
  printf '  %-26s %6s %6s %8s\n' cdylib miss hit "secs"
  for pkg in $(echo "$all" | awk -F'\t' '$1=="cdylib"{print $2}' | sort -u); do
    miss=$(echo "$all" | awk -F'\t' -v p="$pkg" '$1=="cdylib"&&$2==p&&$3=="miss"' | wc -l)
    hit=$(echo "$all" | awk -F'\t' -v p="$pkg" '$1=="cdylib"&&$2==p&&$3=="hit"' | wc -l)
    secs=$(echo "$all" | awk -F'\t' -v p="$pkg" '$1=="cdylib"&&$2==p{s+=$4} END{printf "%.1f", s}')
    printf '  %-26s %6s %6s %8s\n' "$pkg" "$miss" "$hit" "$secs"
  done
  fixn=$(echo "$all" | awk -F'\t' '$1=="fixture"' | wc -l)
  fixs=$(echo "$all" | awk -F'\t' '$1=="fixture"{s+=$4} END{printf "%.1f", s}')
  printf '  fixtures: %s compiled, %ss total rustc\n' "$fixn" "$fixs"
else
  echo "  (no timing ledger — LOFT_TIMING_LEDGER unset or no native compiles)"
fi
