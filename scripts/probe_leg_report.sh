#!/usr/bin/env bash
# One probe leg's outcome, for .github/workflows/macos-probe.yml.
#
# ⚠ THE OUTCOME IS READ FROM NEXTEST, NOT FROM A FAILURE COUNT.  The corpus prints its
# `native result: … N run failed` line on stdout, and nextest CAPTURES stdout for a test that
# PASSES — so a green leg has no such line, and a leg that never ran has no such line either.
# The first version of this counted names off that line and reported `count: 0` for both cases,
# which made "the corpus is clean" and "the corpus did not run" identical in the summary.  The
# nextest summary distinguishes them, so that is what decides; the corpus line is detail when
# it is there.
set -o pipefail
log=${1:?usage: probe_leg_report.sh <log> <name>}
name=${2:?usage: probe_leg_report.sh <log> <name>}

# ⚠ STRIP ANSI FIRST.  nextest colours its summary, so `^ *Summary` never matches the raw
# bytes — the first cut of this reported NO-VERDICT for a passing leg, a failing leg and an
# EMPTY file alike, which is the same three-way blindness it was written to remove.  The
# self-test below is what caught it; run it after any change here.
plain=$(sed $'s/\033\[[0-9;]*m//g' "$log")
# FLAKY is its own answer, and for this probe it is the important one.  nextest RETRIES, so a
# leg whose first try failed with hundreds of run failures still summarises as "1 passed
# (1 flaky)" — which is how the macOS corpus read for months: not red, not clean, just
# occasionally exhausting its retries.  Collapsing that into PASSED would have hidden the very
# thing the phase-3 bound fixed.
if printf '%s' "$plain" | grep -qE 'Summary \[.*[0-9]+ test.* run:' ; then
  if printf '%s' "$plain" | grep -qE 'Summary \[.*[0-9]+ failed'; then
    outcome=FAILED
  elif printf '%s' "$plain" | grep -qE 'Summary \[.*flaky'; then
    outcome=FLAKY
  else
    outcome=PASSED
  fi
else
  outcome=NO-VERDICT
fi
echo "$outcome" > "/tmp/$name.outcome"

printf '%s' "$plain" | grep -E 'native result:|run failures:' | tail -2 || true
printf '%s' "$plain" | grep -E 'run failures:' | tail -1 | sed 's/.*run failures: //' \
  | tr ',' '\n' | sed 's/^ *//' | sed '/^$/d' | sort > "/tmp/$name.failures" || true
count=$(wc -l < "/tmp/$name.failures" | tr -d ' ')
echo "$count" > "/tmp/$name.count"
echo "leg $name: outcome=$outcome, named failures=$count"
[ "$outcome" = PASSED ] && [ "$count" = 0 ] && echo "  (a PASSED leg prints no corpus line — nextest captures stdout on success)"
[ "$outcome" = FLAKY ] && echo "  (FLAKY: a try FAILED and a retry passed — the count above is that failing try's)"
exit 0

# ── Self-test: `probe_leg_report.sh --self-test <pass.log> <fail.log>` ──────
# Three inputs, three different answers, or the instrument is blind.
