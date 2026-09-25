#!/usr/bin/env bash
# @PLN85 — PER-FILE interpreter leak scan (specific logging the aggregate in-process
# runners can't give).  `loft_suite`/`library_suite` run the whole `.loft` corpus in
# ONE process, so LSan reports at process exit and cannot name the leaking file.  This
# runs the ASan `loft` binary over each file SEPARATELY under detect_leaks=1, so every
# leak is attributed to its file AND its demangled owner frame — emitted as a GitHub
# `::error file=…::` annotation (clickable in the run / PR) plus a plain summary line.
#
# Exits non-zero if ANY file leaks.  ir_read + the macOS dyld-TLS frame are suppressed
# via .github/lsan_suppressions.txt (pass it in LSAN_OPTIONS).
#
#   ABIN=<asan loft>  [LSAN_OPTIONS=suppressions=…]  asan_leak_scan.sh <file.loft>...
set -u
ABIN=${ABIN:?set ABIN to an ASan-instrumented loft binary}
[ -x "$ABIN" ] || { echo "::error::ASan loft binary not executable: '$ABIN'"; exit 2; }

demangle() { if command -v rustfilt >/dev/null; then rustfilt; \
  elif command -v c++filt >/dev/null; then c++filt; else cat; fi; }

# The files are independent processes, so they run several at a time and are REPORTED in
# order afterwards: one after another the scan took 32 of the nightly job's 60 minutes, which
# put the job one slow runner from its limit (cancelled on 2026-09-24).  Capped at 4 because
# an ASan process is memory-hungry and the macOS runner is small; the limit is not raised.
jobs=${JOBS:-$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 2)}
[ "$jobs" -gt 4 ] && jobs=4
tmp=$(mktemp -d "${TMPDIR:-/tmp}/asan-leak-scan.XXXXXX") || { echo "::error::cannot create a scratch directory"; exit 2; }
trap 'rm -rf "$tmp"' EXIT
i=0
for f in "$@"; do
  [ -f "$f" ] || continue
  printf '%s\n' "$f" > "$tmp/$i.name"
  i=$((i + 1))
done
total=$i
scan_one() {
  local f
  f=$(cat "$tmp/$1.name")
  ASAN_OPTIONS="detect_leaks=1:${ASAN_OPTIONS:-}" "$ABIN" --interpret "$f" >"$tmp/$1.out" 2>&1 || true
}
export -f scan_one
export ABIN tmp
[ "$total" -gt 0 ] && seq 0 $((total - 1)) | xargs -P "$jobs" -I{} bash -c 'scan_one {}'

fail=0
scanned=0
leakers=0
# `seq 0 -1` counts DOWN on BSD (macOS), so an empty list must not reach it.
for i in $( [ "$total" -gt 0 ] && seq 0 $((total - 1)) ); do
  f=$(cat "$tmp/$i.name")
  scanned=$((scanned + 1))
  out=$(cat "$tmp/$i.out" 2>/dev/null)
  roots=$(printf '%s\n' "$out" | grep -c '^Direct leak' || true)
  if [ "$roots" -gt 0 ]; then
    fail=1
    leakers=$((leakers + 1))
    owner=$(printf '%s\n' "$out" | demangle \
      | awk '/^Direct leak/{p=1} p; /^$/{p=0}' \
      | sed -E 's/^[[:space:]]*#[0-9]+ 0x[0-9a-f]+ in //; s/\+0x.*$//' \
      | grep -E 'loft::' | grep -vE 'ir_read|loft::main|4loft4main|__rust' \
      | head -1)
    echo "::error file=${f}::interpreter leak — ${roots} root(s); owner: ${owner:-<unknown>}"
    printf 'LEAK  %-52s roots=%s  owner=%s\n' "$f" "$roots" "${owner:-?}"
    # LOFT_LEAK_DUMP=1 — diagnostic: emit the RAW leak report so we can read what THIS runner's
    # symbolizer actually produced for the leaking frames (mangled? `<unknown>`? bare `+0x`?), plus
    # whether the ir_read suppression matched. A name-based suppression can only be validated by
    # reading the runner's real output — a local box symbolizes differently (macos-asan-leak-gate.md).
    if [ "${LOFT_LEAK_DUMP:-}" = 1 ]; then
      echo "::group::RAW ASan leak report (this runner's symbolizer) — ${f}"
      printf '%s\n' "$out" | awk '/(^Direct leak|LeakSanitizer|Suppressions used|SUMMARY:)/{on=1} on{print}' | head -200
      echo "--- does 'ir_read' appear in the raw stack? (0 ⇒ unsymbolized → suppression can't match) ---"
      printf '%s\n' "$out" | grep -c 'ir_read' | sed 's/^/  ir_read occurrences: /'
      echo "::endgroup::"
    fi
  fi
done
echo "=== leak scan: ${leakers} leaking file(s) of ${scanned} scanned ==="
exit $fail
