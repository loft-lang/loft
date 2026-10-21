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
# The build under test first: the program cache is on for every binary since @C133, so a
# from-source target/release/loft measures the path an installed one takes, and an installed
# loft may be months older than the tree it is asked about.
if [ -x target/release/loft ]; then loft="target/release/loft"; else loft="loft"; fi
[ "${1:-}" = "--loft" ] && loft="$2"
echo "recheck with: $(command -v "$loft") ($($loft --version 2>/dev/null | head -1))"
dir=doc/claude/plans/179-scripts-in-loft/findings
sha=$(git rev-parse --short HEAD)
tmp=$(mktemp); trap 'rm -f "$tmp" "$tmp.w"' EXIT
# Best-of-5 wall time of one command, in microseconds, timed around `posix_spawn` inside ONE
# process.  Two perl launches bracketing the run put a whole perl start-up inside every
# interval — about 9 ms on a laptop, so `/bin/echo` read 14–18 ms and a start-up bar of
# 12 ms measured perl — and perl's `system` forks all of perl first, another 2 ms.
best_us() {
  python3 -c '
import os, sys, time
out = os.open(os.devnull, os.O_WRONLY)
fa = [(os.POSIX_SPAWN_DUP2, out, 1), (os.POSIX_SPAWN_DUP2, out, 2)]
best = None
for _ in range(5):
    t = time.perf_counter()
    os.waitpid(os.posix_spawnp(sys.argv[1], sys.argv[1:], os.environ, file_actions=fa), 0)
    us = int((time.perf_counter() - t) * 1e6)
    best = us if best is None else min(best, us)
print(best)' "$@"
}
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
    startup-over:*|over:*)
      bar=${expect#*over:}
      best=$(best_us "$loft" --interpret "$dir/$probe")
      [ "$best" -gt $((bar * 1000)) ] && holds=yes
      label=$([ "${expect%%:*}" = startup-over ] && echo "start-up " || echo "")
      printf '  %s: %s%d.%d ms (bar %s)\n' "$(basename "$f")" "$label" \
        $((best / 1000)) $((best % 1000 / 100)) "$bar" ;;
    *) echo "script_recheck: $f: unknown expect '$expect'" >&2; continue ;;
  esac
  # Through a temp file: `sed -i` takes a mandatory suffix on macOS and none on GNU, so a
  # bare `-i` edits nothing on one of them and the register never moves.
  rewrite() { sed "$1" "$f" > "$tmp.w" && mv "$tmp.w" "$f"; }
  rewrite "s/^checked:.*/checked: $sha/; s/^holds:.*/holds: $holds/"
  grep -q '^checked:' "$f" || rewrite "s/^ref:\(.*\)/ref:\1\
checked: $sha/"
  grep -q '^holds:' "$f" || rewrite "s/^checked:\(.*\)/checked:\1\
holds: $holds/"
  [ "$holds" = no ] && { stale=$((stale + 1)); echo "  NO LONGER HOLDS: $(basename "$f")"; }
done
echo "rechecked $n findings at $sha, $stale no longer hold"
