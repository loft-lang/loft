#!/usr/bin/env bash
# @PLN179 — re-measure every finding that carries a probe, and write the answer back into
# its file, so the register can say which findings no longer hold.
#
#   scripts/script_recheck.sh [--loft BIN]      (default: target/release/loft, else `loft`)
#
# A finding's file may carry `probe: <file under findings/>` and `expect: <kind>` — what the
# probe does WHILE THE FINDING STILL HOLDS:
#   refused           the probe does not compile (the capability is still missing)
#   refused-line:N    the probe is refused, but the diagnostic does NOT point at line N
#   crash             the probe dies with a signal (the defect is still there)
#   startup-over:MS   `loft --interpret` of a hello takes more than MS ms, best of 5
#   over:MS           the probe itself takes more than MS ms interpreted, best of 5
# After each probe the driver rewrites the finding's `checked:` (the loft commit) and
# `holds:` (yes | no) lines.  A finding whose probe says `no` is the one to look at: the
# reason it recorded no longer measures, so it is fixed, or the probe needs a new name.
# Bash today because it runs programs; strand 4's `run` ports it.
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
# The INSTALLED loft first (its program cache is on; a from-source target/release/loft runs
# with it off — PERFORMANCE.md rule 4 — and a start-up probe would measure the wrong thing).
if command -v loft > /dev/null; then loft="loft"; else loft="target/release/loft"; fi
[ "${1:-}" = "--loft" ] && loft="$2"
echo "recheck with: $(command -v "$loft") ($($loft --version 2>/dev/null | head -1))"
dir=doc/claude/plans/179-scripts-in-loft/findings
sha=$(git rev-parse --short HEAD)
tmp=$(mktemp); trap 'rm -f "$tmp"' EXIT
n=0; stale=0
for f in "$dir"/[0-9][0-9][0-9]-*.md; do
  probe=$(sed -n 's/^probe: *//p' "$f" | head -1); expect=$(sed -n 's/^expect: *//p' "$f" | head -1)
  [ -n "$probe" ] && [ -n "$expect" ] || continue
  n=$((n + 1)); holds=no
  "$loft" --interpret "$dir/001.probe.loft" 2>&1 | grep -q 'does not match this loft' && {
    echo "script_recheck: $loft refuses its own default/ — reinstall it before rechecking" >&2; exit 2; }
  case "$expect" in
    refused)
      LOFT_TIMEOUT=60 "$loft" --interpret "$dir/$probe" > /dev/null 2> "$tmp"; rc=$?
      [ $rc -ne 0 ] && grep -q '^error' "$tmp" && holds=yes ;;
    refused-line:*)
      line=${expect#refused-line:}
      LOFT_TIMEOUT=60 "$loft" --interpret "$dir/$probe" > /dev/null 2> "$tmp"; rc=$?
      if [ $rc -ne 0 ] && grep -q '^error' "$tmp"; then
        grep -qE -- "--> .*:$line:" "$tmp" || holds=yes
      fi ;;
    crash)
      LOFT_TIMEOUT=120 "$loft" --interpret "$dir/$probe" > /dev/null 2> "$tmp"; rc=$?
      { [ $rc -ge 128 ] || grep -q SIGSEGV "$tmp"; } && holds=yes ;;
    startup-over:*)
      bar=${expect#startup-over:}; best=""
      for i in 1 2 3 4 5; do
        s=$(date +%s%N); "$loft" --interpret "$dir/$probe" > /dev/null 2>&1; e=$(date +%s%N); ms=$(( (e - s) / 1000000 ))
        [ -z "$best" ] || [ "$ms" -lt "$best" ] && best=$ms
      done
      [ "$best" -gt "$bar" ] && holds=yes
      printf '  %s: start-up %s ms (bar %s)\n' "$(basename "$f")" "$best" "$bar" ;;
    over:*)
      bar=${expect#over:}; best=""
      for i in 1 2 3 4 5; do
        s=$(date +%s%N); "$loft" --interpret "$dir/$probe" > /dev/null 2>&1; e=$(date +%s%N); ms=$(( (e - s) / 1000000 ))
        [ -z "$best" ] || [ "$ms" -lt "$best" ] && best=$ms
      done
      [ "$best" -gt "$bar" ] && holds=yes
      printf '  %s: %s ms (bar %s)\n' "$(basename "$f")" "$best" "$bar" ;;
    *) echo "script_recheck: $f: unknown expect '$expect'" >&2; continue ;;
  esac
  sed -i "s/^checked:.*/checked: $sha/; s/^holds:.*/holds: $holds/" "$f"
  grep -q '^checked:' "$f" || sed -i "s/^ref:\(.*\)/ref:\1\nchecked: $sha/" "$f"
  grep -q '^holds:' "$f" || sed -i "s/^checked:\(.*\)/checked:\1\nholds: $holds/" "$f"
  [ "$holds" = no ] && { stale=$((stale + 1)); echo "  NO LONGER HOLDS: $(basename "$f")"; }
done
echo "rechecked $n findings at $sha, $stale no longer hold"
