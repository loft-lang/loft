#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# The release's valgrind gate, as one command (RELEASE.md § Memory safety, the checklist's
# `M-valgrind`): every script and document under memcheck, on the interpreter AND as the
# compiled native program, with one verdict.
#
#   scripts/valgrind-sweep.sh                 # everything: scripts, docs and the language suite
#   scripts/valgrind-sweep.sh tests/docs      # one tree, or any list of .loft files
#   VG_JOBS=8 scripts/valgrind-sweep.sh       # fewer parallel memchecks (each takes ~200 MB)
#   VG_OUT=target/vg-probe …                  # keep the logs apart from another sweep's
#
# What counts, and why:
#   * an INVALID ACCESS of any kind (read, write, uninitialised use, bad free, a syscall
#     handed uninitialised bytes) fails the sweep — that is the class the gate exists for,
#     the one Linux's allocator hides in slack and Windows' heap checker reports as
#     STATUS_HEAP_CORRUPTION (TEST_ENVIRONMENTS.md § Occasional valgrind pass);
#   * a DEFINITELY LOST block fails it — memory nothing can reach any more;
#   * a "possibly lost" record does NOT.  Rust's hashbrown tables and boxed strings keep
#     interior pointers, so every process-lifetime table — the parser's `Data`, the native
#     emitter registry — reads as possibly lost at exit; measured at 179 records on a clean
#     run.  `--errors-for-leak-kinds=definite` is that decision, spelled where valgrind
#     reads it.
#   * the one suppression (scripts/valgrind.supp) is the deliberate interning of a declared
#     text field default — bounded, one block per field, the same frame the ASan leak gate
#     names.  It suppresses nothing else on purpose: see the file for the text-buffer class
#     LSan hides and this sweep does not.
#   * loft's own store arena is INVISIBLE to memcheck (DEBUG_STORES.md § Debugging store-ownership
#     bugs): a leaked or over-freed STORE is a wrong answer, never a valgrind error.  That
#     half of the memory gate is `M-leaks` under `LOFT_STRICT_STORES=1`, not this one.
#
# The interpreter half runs `loft --interpret` on every file — `--tests` for tests/scripts,
# whose files have no `main` and run nothing without it (GUARDS.md § How a guard reads green while the defect stands).  The
# native half compiles each tests/docs document with `loft --native` (unchecked, so rustc is
# not traced) and then hands the cached binary in `<dir>/.loft/cache/` to memcheck directly:
# `--trace-children` would follow rustc, and the driver's own exec of the program is what
# `--trace-children-skip` cannot single out.  Per-file logs stay in target/vg/ for reading.

set -uo pipefail
cd "$(dirname "$0")/.." || exit 1
command -v valgrind >/dev/null || { echo "valgrind is not installed — the gate cannot run here"; exit 2; }
[ -x target/release/loft ] || { echo "target/release/loft is not built — cargo build --release first"; exit 2; }

OUT=${VG_OUT:-target/vg}
# Absolute from here on: the stdlib cache below is `$OUT/xdg-cache`, and prefixing `$PWD` to an
# absolute VG_OUT wrote that cache INSIDE the repository.
case "$OUT" in /*) ;; *) OUT="$PWD/$OUT" ;; esac
rm -rf "$OUT"; mkdir -p "$OUT"
# Every core on a small machine — the CI runner has 4 and runs nothing else — and a sixth of
# them left free on a large shared one.
CORES=$(nproc); if [ "$CORES" -le 4 ]; then DEF_JOBS=$CORES; else DEF_JOBS=$(( CORES * 5 / 6 )); fi
JOBS=${VG_JOBS:-$DEF_JOBS}; [ "$JOBS" -lt 1 ] && JOBS=1
# `VG_CMD` replaces the memcheck command line (a falsification harness plants a report with it).
VG=${VG_CMD:-"valgrind --error-exitcode=77 --leak-check=full --errors-for-leak-kinds=definite --suppressions=scripts/valgrind.supp"}
# The standard library is parsed ONCE per sweep, not once per file.  Every run used to parse
# `default/` from source under memcheck — ~5.5 s of a trivial file's 6.9 s, the larger part of
# the whole sweep — and it is the same code on the same input every time.  The first run below
# parses it cold UNDER memcheck and writes the stdlib bundle (so both the parse and the writer
# stay covered); every other run loads that bundle (`LOFT_STDLIB_CACHE=1`, the warm start the
# language server takes, which puts the READER under memcheck on every file).  The cache
# directory is the sweep's own, so the writer run always starts with no bundle and a bundle
# from another build is never read.
export XDG_CACHE_HOME="$OUT/xdg-cache"
export VG OUT

# The population: every argument (a file or a directory), or the two shipped corpora.
if [ $# -gt 0 ]; then
  for a in "$@"; do [ -d "$a" ] && find "$a" -maxdepth 1 -name '*.loft' | sort || echo "$a"; done
else
  # tests/scripts, tests/docs and every program of the language suite beside the tree (@PLN193).
  scripts/corpus_files.sh
fi > "$OUT/list.txt"

# One line per run: kind, unit, exit, invalid-access count, definitely-lost bytes, seconds.
# A UNIT is a file, or `file::test_fn` for one test function of a heavy tests/scripts file.
check_one() {
  kind=$1; u=$2; f=${u%%::*}; stem=$(basename "$f" .loft)
  case "$u" in *::*) stem="$stem--${u#*::}";; esac
  log="$OUT/$kind-$stem.log"; t0=$(date +%s.%N)
  case "$kind" in
    interp) case "$f" in tests/docs/*) mode="--interpret";; *) mode="--interpret --tests";; esac
            LOFT_TIMEOUT=300 LOFT_TIMEOUT_CLEAN_EXIT=1 $VG --log-file="$log" target/release/loft $mode "$u" >/dev/null 2>&1; rc=$? ;;
    # `--out` builds without running and names the program after the source, so the binary
    # memcheck reads is the one this run built — not a guess at the driver's cache naming.
    native) nd="$OUT/native/$stem"
            LOFT_TIMEOUT=300 target/release/loft --native --out "$nd" "$f" >/dev/null 2>&1 || { echo "native	$f	build-failed	-	-"; return; }
            bin="$nd/$stem"
            [ -x "$bin" ] || { echo "native	$f	no-binary	-	-"; return; }
            $VG --log-file="$log" "$bin" >/dev/null 2>&1; rc=$? ;;
    # A BATCH is a directory of links to light files, memchecked in ONE process (`--tests DIR`
    # runs every file in-process): memcheck's start-up is most of a light file's cost, and a
    # batch pays it once.  A batch that reports anything, or ran fewer files than it holds, is
    # re-run file by file, so the verdict always names the file, as a single run would.
    batch)  n=$(wc -l < "$u/members.txt")
            LOFT_TIMEOUT=300 LOFT_TIMEOUT_CLEAN_EXIT=1 $VG --log-file="$log" target/release/loft --interpret --tests "$u" > "$log.out" 2>&1; rc=$?
            ran=$(sed -nE 's/^test result: .*; ([0-9]+) files?.*/\1/p' "$log.out" | tail -1) ;;
  esac
  bad=$(grep -cE "^==[0-9]+== (Invalid (read|write|free)|Conditional jump|Use of uninitialised|Syscall param|Mismatched free|Jump to the invalid|Source and destination overlap)" "$log")
  lost=$(grep -oE "definitely lost: [0-9,]+ bytes" "$log" | head -1 | tr -d ', ' | grep -oE "[0-9]+" || echo 0)
  if [ "$kind" = batch ]; then
    if [ "$bad" -gt 0 ] || [ "${lost:-0}" -gt 0 ] || [ "$rc" -eq 124 ] || [ "$rc" -ge 128 ] || [ "${ran:-0}" != "$n" ]; then
      while read -r m; do check_one interp "$m"; done < "$u/members.txt"
      return
    fi
    echo "batch	$u	$rc	0	0	$(echo "$(date +%s.%N) - $t0" | bc)	$n"
    return
  fi
  echo "$kind	$u	$rc	$bad	${lost:-0}	$(echo "$(date +%s.%N) - $t0" | bc)"
}
export -f check_one

echo 'fn main() { println("cold"); }' > "$OUT/cold-stdlib.loft"
LOFT_STDLIB_CACHE=1 check_one interp "$OUT/cold-stdlib.loft" > "$OUT/results.tsv"
ls "$XDG_CACHE_HOME"/loft/stdlib-*.store >/dev/null 2>&1 \
  || { echo "the cold run wrote no stdlib bundle — every run would parse cold; not sweeping"; exit 2; }
export LOFT_STDLIB_CACHE=1
# No run asks the registry for a package it does not find: a cell that names an absent
# package on purpose (the F-Surface refusals) otherwise opens a TLS connection, and memcheck
# reports ring's and rustls's own buffers as this sweep's invalid accesses.
export LOFT_NO_AUTO_INSTALL=1

# The PLAN: every interpreter file runs once WITHOUT memcheck first — a median file takes
# 0.04 s — which gives two facts the memcheck runs are then arranged by:
#   * its COST.  Memcheck multiplies a run by ~100–200×, so the plain time predicts it, and the
#     work is started longest first: a heavy file queued last is a tail every other job waits on.
#   * its TEST FUNCTIONS, read off the runner's own `(N fns: a, b, …)` line.  A heavy file — a
#     store-ceiling guard loops 70 000 times per cell — is memchecked one test function at a time
#     (`file::name`), so its cells spread across the jobs and the per-run limit applies to a cell,
#     not to eleven of them in series.  A light file stays whole: splitting it would repeat the
#     start-up cost per function for nothing.
SPLIT=${VG_SPLIT:-0.5}
plan_one() {
  f=$1; t0=$(date +%s.%N)
  case "$f" in tests/docs/*) mode="--interpret";; *) mode="--interpret --tests";; esac
  out=$(LOFT_TIMEOUT=300 target/release/loft $mode "$f" 2>&1 | tr -d '\0')
  secs=$(echo "$(date +%s.%N) - $t0" | bc)
  fns=$(printf '%s\n' "$out" | sed -nE 's/.*\([0-9]+ fns?: (.*)\)$/\1/p' | head -1)
  # A file a `[sandbox]` policy governs (a `loft.toml` within four directories above it, the
  # walk `loft --tests` makes) runs alone: a batch directory sits outside that walk.
  gov=0; d=$(dirname "$f")
  for _ in 1 2 3 4; do [ -f "$d/loft.toml" ] && { gov=1; break; }; d=$(dirname "$d"); done
  printf '%s\t%s\t%s\t%s\n' "$f" "$secs" "$fns" "$gov"
}
export -f plan_one
xargs -r -P "$(nproc)" -I{} bash -c 'plan_one "$0"' {} < "$OUT/list.txt" > "$OUT/plan.tsv"
awk -F'\t' -v thr="$SPLIT" '
  { n = split($3, fn, ", ") }
  $2 + 0 >= thr + 0 && n >= 2 && $3 !~ /(^|, )main$/ {
    for (i = 1; i <= n; i++) printf "%.4f interp %s::%s\n", $2 / n, $1, fn[i]; next }
  { printf "%.4f interp %s %s\n", $2, $1, $4 }
  $1 ~ /^tests\/docs\// { printf "%.4f native %s 1\n", $2, $1 }
' "$OUT/plan.tsv" | sort -rn > "$OUT/costed.txt"
# The BATCHES: a light whole file (plain run under VG_BATCH_ONE seconds) that is not a document
# and no policy governs joins a batch, longest first, until the batch's plain cost passes
# VG_BATCH_SECS or it holds VG_BATCH_N files.  Two files of one name never share a batch: a
# link is named after its file.
mkdir -p "$OUT/batches"
awk -v one="${VG_BATCH_ONE:-0.2}" -v budget="${VG_BATCH_SECS:-1.6}" -v cap="${VG_BATCH_N:-16}" -v dir="$OUT/batches" '
  function close_batch() { if (cnt > 0) printf "%.4f batch %s/b%04d\n", sum, dir, nb; cnt = 0; sum = 0; delete names }
  $2 == "interp" && $3 !~ /::/ && $3 !~ /^tests\/docs\// && $4 == 0 && $1 + 0 < one + 0 {
    n = split($3, p, "/"); base = p[n]
    if (cnt > 0 && (sum + $1 > budget || cnt >= cap || (base in names))) close_batch()
    if (cnt == 0) nb++
    names[base] = 1; cnt++; sum += $1
    printf "%s\n", $3 > (dir "/b" sprintf("%04d", nb) ".list")
    next }
  { printf "%s %s %s\n", $1, $2, $3 }
  END { close_batch() }
' "$OUT/costed.txt" | sort -rn | cut -d' ' -f2- > "$OUT/units.txt"
for l in "$OUT"/batches/*.list; do
  [ -e "$l" ] || continue
  b=${l%.list}; mkdir -p "$b"; mv "$l" "$b/members.txt"
  while read -r m; do case "$m" in /*) ln -s "$m" "$b/";; *) ln -s "$PWD/$m" "$b/";; esac; done < "$b/members.txt"
done
# A sweep that checks nothing must not read GREEN.
[ -s "$OUT/units.txt" ] || { echo "the plan has no runs in it — nothing would be checked; not sweeping"; exit 2; }

xargs -r -P "$JOBS" -n 2 bash -c 'check_one "$0" "$1"' < "$OUT/units.txt" >> "$OUT/results.tsv"

runs=$(wc -l < "$OUT/results.tsv")
invalid=$(awk -F'\t' '$4 != "-" && $4 > 0' "$OUT/results.tsv" | wc -l)
lost=$(awk -F'\t' '$5 != "-" && $5 > 0' "$OUT/results.tsv" | wc -l)
unbuilt=$(awk -F'\t' '$3 == "build-failed" || $3 == "no-binary"' "$OUT/results.tsv" | wc -l)
# `LOFT_TIMEOUT` ends a run with 124 (the GNU `timeout` convention) only under
# `LOFT_TIMEOUT_CLEAN_EXIT`, which `check_one` sets; without it the hard-kill ABORTS (134).
# A run a signal ended (exit 128+) stopped where it stopped as surely as one the timeout
# ended.  The program's own stderr goes to /dev/null, so the exit code is the only place
# that says so.
ENDED='$3 == 124 || ($3 ~ /^[0-9]+$/ && $3 >= 128)'
timed=$(awk -F'\t' "$ENDED" "$OUT/results.tsv" | wc -l)
batches=$(grep -c '^batch' "$OUT/results.tsv")
batched=$(awk -F'\t' '$1 == "batch" { n += $7 } END { print n + 0 }' "$OUT/results.tsv")
echo "valgrind sweep: $runs runs over $(wc -l < "$OUT/list.txt") files, $(grep -c '::' "$OUT/units.txt") of them one test function of a split file ($(grep -c '^interp' "$OUT/results.tsv") interpreter runs, $batched files in $batches batches, $(grep -c '^native' "$OUT/results.tsv") native)"
echo "  invalid accesses: $invalid file(s) · definitely lost: $lost file(s) · native not built: $unbuilt · timed out or killed: $timed"
# A run the timeout ended was checked only up to where it stopped, so it is not a pass; a
# native document that was not built was not checked at all.
if [ "$invalid" -gt 0 ] || [ "$lost" -gt 0 ] || [ "$timed" -gt 0 ] || [ "$unbuilt" -gt 0 ]; then
  echo "  RED — the offending runs (kind, unit, exit, invalid-access count, bytes definitely lost, seconds):"
  awk -F'\t' "(\$4 != \"-\" && \$4 > 0) || (\$5 != \"-\" && \$5 > 0) || $ENDED || \$3 == \"build-failed\" || \$3 == \"no-binary\"" "$OUT/results.tsv" | head -40
  echo "  logs: $OUT/<kind>-<stem>.log"
  exit 1
fi
echo "  GREEN — no invalid access and nothing definitely lost, on either backend (logs in $OUT/)"
