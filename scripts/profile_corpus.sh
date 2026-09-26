#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN140 arc D — run the profiling instruments over the benchmark corpus and check
# them against `bench/profile_oracle.tsv`.
#
#   scripts/profile_corpus.sh              # check the oracle + report drift
#   scripts/profile_corpus.sh --only 03    # one program
#   scripts/profile_corpus.sh --overhead   # also measure what profiling costs
#
# TWO JOBS, and only one of them can fail.
#
#   The ORACLE is a gate.  Every program here has a hot spot that is known in
#   advance — `fib`'s time is in `fib` — so an instrument that fails to name it is
#   WRONG, and says so immediately.  That is the only thing in this repo that can
#   prove a profiler wrong rather than merely exercise it, so a failing row exits
#   non-zero.  It is a regression in the PROFILER, not in the program.
#
#   The DRIFT is a report, never a gate — `make speed` is the precedent.  Shares
#   move with the machine, the load and the kernel, so a committed baseline would
#   be a permanent source of false diffs.  The previous local capture is diffed
#   instead (@PLN140 open question 5), and a human triages what moved.
set -uo pipefail
cd "$(dirname "$0")/.."

ONLY=""; OVERHEAD=0
while [ $# -gt 0 ]; do
  case "$1" in
    --only)     ONLY="$2"; shift 2;;
    --overhead) OVERHEAD=1; shift;;
    -h|--help)  sed -n '/^# @PLN140 arc D/,/^set -uo/p' "$0" | sed 's/^# \?//;$d'; exit 0;;
    *) echo "profile_corpus.sh: unknown option '$1'" >&2; exit 2;;
  esac
done

ORACLE=bench/profile_oracle.tsv
[ -f "$ORACLE" ] || { echo "profile_corpus.sh: $ORACLE is missing" >&2; exit 1; }

# An `engine` row samples loft ITSELF with perf (`scripts/profile.sh --engine`), which a box
# without perf, or with `perf_event_paranoid` above 2, cannot run.  Such a row is neither
# proved nor disproved there: it is announced as SKIPPED, never counted as held.
ENGINE_OK=1
command -v perf >/dev/null 2>&1 || ENGINE_OK=0
[ "$(cat /proc/sys/kernel/perf_event_paranoid 2>/dev/null || echo 4)" -le 2 ] 2>/dev/null || ENGINE_OK=0

echo "── building (release) ──" >&2
cargo build --release --bin loft >&2 || exit 1
BIN=target/release/loft

OUT="${LOFT_PROFILE_DIR:-${TMPDIR:-/tmp}}/loft-profile"
mkdir -p "$OUT"
PREV="$OUT/corpus-prev.tsv"
CUR="$OUT/corpus-cur.tsv"
: > "$CUR"

fails=0; checked=0
printf '%-16s %-4s %-24s %8s  %s\n' PROGRAM WHAT "TOP ROW" SHARE VERDICT

while IFS=$'\t' read -r prog what expect min_share want_line; do
  case "$prog" in ''|'#'*) continue;; esac
  if [ -n "$ONLY" ] && [ "${prog#"$ONLY"}" = "$prog" ]; then continue; fi
  if [ "$what" = engine ]; then
    # A compile's input is the front-end corpus, generated (never committed) at the size
    # the row names: `frontend_<size>` → `bench/frontend/frontend.py --emit <size>`.
    if [ "$ENGINE_OK" != 1 ]; then
      printf '%-16s %-4s %-24s %8s  %s\n' "$prog" "$what" "-" "-" "SKIPPED — perf unavailable or perf_event_paranoid > 2 (scripts/profile.sh says how)"
      continue
    fi
    src="$OUT/$prog.loft"
    python3 bench/frontend/frontend.py --emit "${prog#frontend_}" > "$src" ||
      { echo "  $prog: bench/frontend/frontend.py --emit ${prog#frontend_} failed — skipped" >&2; continue; }
  else
    src="bench/$prog/bench.loft"
    [ -f "$src" ] || { echo "  $prog: no $src — skipped" >&2; continue; }
  fi
  checked=$((checked + 1))

  if [ "$what" = engine ]; then
    # `--check` under `--interpret` is the front end alone; `--no-cache` so the parse runs
    # rather than the loader; 9999 Hz because a compile is short and a module share over a
    # few hundred samples is a coin toss.  The row is read off the BY-MODULE table — over a
    # compile the top symbol ties with the allocator's and moves run to run (@PLN166 A3).
    raw=$(LOFT_PROFILE_FREQ=9999 LOFT_TIMEOUT=300 scripts/profile.sh --engine --no-cache -- --interpret --check "$src" 2>&1)
    top=$(printf '%s\n' "$raw" | awk '/^════ by module/{f=1;next} f&&/^ +[0-9]/{print;exit}')
    topline="$top"
  elif [ "$what" = cpu ]; then
    raw=$(LOFT_PROFILE=1 LOFT_TIMEOUT=300 "$BIN" --interpret "$src" 2>&1)
    # The row under "by function" is the instrument's answer to "what is hot".
    top=$(printf '%s\n' "$raw" | awk '/^── by function/{f=1;next} f&&/^  /{print;exit}')
    topline=$(printf '%s\n' "$raw" | awk '/^── by line/{f=1;next} f&&/^  /{print;exit}')
  else
    raw=$(LOFT_ALLOC_SITES=1 LOFT_TIMEOUT=300 "$BIN" --interpret "$src" 2>&1)
    top=$(printf '%s\n' "$raw" | awk '/^════ allocation hot spots/{f=1;next} f&&/^ +[0-9]/{print;exit}')
    topline="$top"
  fi

  if [ -z "$top" ]; then
    printf '%-16s %-4s %-24s %8s  %s\n' "$prog" "$what" "(no rows)" "-" "FAIL — the instrument reported nothing"
    fails=$((fails + 1)); continue
  fi

  # Share: a percentage for cpu and engine; for mem, this site's bytes over the captured peak.
  if [ "$what" = cpu ]; then
    share=$(printf '%s\n' "$top" | sed -n 's/^[[:space:]]*\([0-9.]*\) %.*/\1/p')
  elif [ "$what" = engine ]; then
    share=$(printf '%s\n' "$top" | sed -n 's/^[[:space:]]*\([0-9.]*\)%.*/\1/p')
  else
    share=$(printf '%s\n' "$raw" | awk '
      /^════ allocation hot spots/ {
        # "captured at 273.4 MiB" — the denominator the rows add up to.
        for (i = 1; i <= NF; i++) if ($i == "at") { cap = $(i+1); unit = $(i+2) }
      }
      /^ +[0-9]/ && !done { row = $1; runit = $2; done = 1 }
      END {
        f["B"] = 1; f["KiB"] = 1024; f["MiB"] = 1048576; f["GiB"] = 1073741824
        sub("%", "", cap)
        if (f[unit] && f[runit] && cap > 0) printf "%.1f", row * f[runit] * 100 / (cap * f[unit])
      }')
  fi

  verdict=ok
  printf '%s' "$top" | grep -qE "$expect" || verdict="FAIL — expected /$expect/"
  if [ "$verdict" = ok ] && [ -n "${min_share:-}" ]; then
    below=$(awk -v s="${share:-0}" -v m="$min_share" 'BEGIN { print (s + 0 < m + 0) ? 1 : 0 }')
    [ "$below" = 1 ] && verdict="FAIL — ${share}% is under the ${min_share}% the oracle requires"
  fi
  if [ "$verdict" = ok ] && [ -n "${want_line:-}" ]; then
    printf '%s' "$topline" | grep -qE ":${want_line}([^0-9]|$)" ||
      verdict="FAIL — hottest line is not $want_line (got: $(printf '%s' "$topline" | tr -s ' '))"
  fi
  [ "$verdict" = ok ] || fails=$((fails + 1))

  if [ "$what" = engine ]; then
    label=$(printf '%s' "$top" | awk '{ print "module " $2 }')
  else
    label=$(printf '%s' "$top" | sed 's/^ *//' | cut -c1-24)
  fi
  printf '%-16s %-4s %-24s %7s%%  %s\n' "$prog" "$what" "$label" "${share:-?}" "$verdict"
  printf '%s\t%s\t%s\t%s\n' "$prog" "$what" "$label" "${share:-0}" >> "$CUR"

  if [ "$OVERHEAD" = 1 ]; then
    t_off=$(LOFT_TIMEOUT=300 "$BIN" --interpret "$src" 2>/dev/null | grep -o 'time: [0-9]*' | grep -o '[0-9]*')
    t_on=$(LOFT_PROFILE=1 LOFT_TIMEOUT=300 "$BIN" --interpret "$src" 2>/dev/null | grep -o 'time: [0-9]*' | grep -o '[0-9]*')
    if [ -n "$t_off" ] && [ -n "$t_on" ] && [ "$t_off" -gt 0 ]; then
      awk -v a="$t_off" -v b="$t_on" -v p="$prog" \
        'BEGIN { printf "%-16s      overhead %.0f ms → %.0f ms (×%.2f)\n", p, a, b, b / a }'
    fi
  fi
done < "$ORACLE"

echo
if [ -s "$PREV" ]; then
  echo "── drift since the previous capture (a report, never a gate) ──"
  moved=0
  while IFS=$'\t' read -r prog what label share; do
    old=$(awk -F'\t' -v p="$prog" -v w="$what" '$1 == p && $2 == w { print $4 }' "$PREV")
    [ -z "$old" ] && continue
    awk -v p="$prog" -v w="$what" -v o="$old" -v n="$share" 'BEGIN {
      d = n - o
      if (d < 0) d = -d
      if (d >= 5) printf "  %-16s %-4s %.1f%% → %.1f%%\n", p, w, o, n
    }'
    moved=1
  done < "$CUR"
  [ "$moved" = 0 ] && echo "  (nothing to compare)"
else
  echo "(no previous capture — this run becomes the baseline for the next)"
fi
cp -f "$CUR" "$PREV" 2>/dev/null

echo
if [ "$fails" -gt 0 ]; then
  echo "$fails of $checked oracle rows FAILED — the profiler is naming the wrong thing." >&2
  echo "Rationale for each row: doc/claude/plans/140-semi-automatic-profiling/ORACLE.md" >&2
  exit 1
fi
echo "$checked oracle rows hold: every instrument named the hot spot that was known in advance."
