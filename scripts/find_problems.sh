#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# One-pass-find-all-problems workflow (see doc/claude/TESTING.md).
#
# Default mode: runs the CURATED set — everything except a short, named list of
# slow-and-few binaries (scripts/test_subjects.sh).  3733 of 3833 tests in ~70s
# instead of ~370s, so a run before every commit is affordable; the full suite is
# what you reach for, not what you have to remember to avoid.  Nothing the
# curated set skips can reach main: CI's `Test (ubuntu-latest)` runs the suite
# unsharded and is a required check.
#
# Tees the raw log to /tmp/loft_test.<id>.log (or $1); the summary writes to
# /tmp/loft_problems.txt (or $2) when the run finishes.  Avoids the
# fix-one-rerun-see-next loop that pays the compile + test-startup
# cost on every iteration.
#
# `<id>` is a per-checkout tag derived from the repo root, so two
# working trees (e.g. sibling agent checkouts) can run this script
# concurrently without sharing pid/log/summary files — a shared pid
# file let one tree's --wait consume the other's run.  The summary
# keeps the documented stable path PLUS the per-checkout copy; every
# mode prints the exact paths it used.
#
# Peek mode (no compile): `./scripts/find_problems.sh --peek` inspects
# the in-flight log and prints any failures
# discovered so far.  Shows last script run before a SIGSEGV so
# wrap-suite crashes point at the specific .loft file that blew up.
#
# Usage:
#   ./scripts/find_problems.sh                         # CURATED run+wait (~70s)
#   ./scripts/find_problems.sh --full                  # every test (~370s)
#   ./scripts/find_problems.sh --subject store         # one subject (seconds)
#   ./scripts/find_problems.sh --changed [ref]         # the subjects the diff touches
#                                                      #   (uncommitted edits; or vs ref)
#   ./scripts/find_problems.sh --list-subjects         # subjects + what is excluded
#   ./scripts/find_problems.sh --bg                    # run in background
#   ./scripts/find_problems.sh /tmp/log /tmp/problems  # custom paths
#   ./scripts/find_problems.sh --peek                  # in-flight peek
#   ./scripts/find_problems.sh --wait                  # wait for a --bg run
#   ./scripts/find_problems.sh --stop                  # stop THIS checkout's --bg run
#
# The selection flags combine with --bg: `--full --bg`, `--subject parser --bg`.
#
# All state is keyed by the checkout (dir name + path hash), so `--stop` only ever
# touches this working tree's run — never a sibling checkout's.  Never use a broad
# `pkill -f nextest`: it reaches into other checkouts and starts a run-killing battle.
#
# Reach for this any time a refactor is expected to surface multiple
# failures (e.g. after renaming a widely-used API, touching parser
# code paths, or replacing a native's signature).  For focused work
# on ONE test family, prefer a prefix filter instead:
#   cargo test --release --test issues q3_to_json
#
# Rule: never run `cargo test --release` (the full suite) in the
# foreground.  Always go through `--bg` so the blocking run does
# not occupy the terminal for 60-90 s.  `cargo clippy` and single-
# file tests stay foreground.
#
# ⚠ That rule is for a HUMAN at a terminal, where the cost is a blocked prompt.
# An AGENT wants the opposite: run the BLOCKING form and let its harness put the
# command in the background, because the harness notifies when the process it
# started exits.  `--bg` returns in seconds, so a harness watching the command
# sees a fast exit and has nothing left to report — and the agent falls back to
# polling.  Two backgrounding mechanisms; using both cancels the notification.
#
# ⚠ And never wait on this with a `pgrep` loop: `until ! pgrep -f "cargo nextest"`
# matches the waiting shell's OWN command line and never terminates.  Use `--peek`
# (reads state, not process tables) or `--wait` (watches the recorded pid).
set -euo pipefail

# Cache clean/release rebuilds with sccache when present (no-op otherwise).
source "$(dirname "${BASH_SOURCE[0]}")/sccache_env.sh"

# Per-checkout tag: the readable dir NAME plus a hash of the full path (distinct even when two
# checkouts share a basename).  So the log / pid / output files are `/tmp/loft_test.<dir>.<hash>.…`
# — named by their checkout, so two agents in sibling checkouts (e.g. `loft` and `loft2`) never
# share a file and `--stop` only ever touches THIS checkout's run.
REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
REPO_HASH=$(printf '%s' "$REPO_ROOT" | cksum | cut -d' ' -f1)
REPO_SLUG=$(printf '%s' "$(basename "$REPO_ROOT")" | tr -c 'A-Za-z0-9._-' '_')
REPO_TAG="$REPO_SLUG.$REPO_HASH"
# The per-checkout server-test port offset used to be computed here and exported.  It is now
# `common::checkout_port_offset`, which reproduces `(cksum(path) % 6 + 1) * 2000` in Rust — the
# SAME bands, so nothing moved, but every way of running the suite gets them: `make ci` and a
# bare `cargo test` as well as this script.  That gap was loft#1193 — two gates running at once
# collided on fixed ports and the red read as a browser bug.  Set `LOFT_TEST_PORT_OFFSET` by
# hand to pin a band.

# FFI toolchain guard (E0514 self-heal).  A nightly / sanitizer build run into the
# shared `target/` leaves `target/release/deps/libloft_ffi-*.rlib` compiled by a
# different rustc than the active one.  The native test harness links that rlib via
# direct rustc (out of cargo's fingerprint), and its mtime looks fresh — so neither
# cargo nor mtime rebuilds it, and every native test fails E0514 ("incompatible
# version of rustc").  Detect the mismatch from the rlib's embedded version and
# delete the stale rlib so the next build recompiles it with the active rustc.
# Prevent the pollution in the first place with scripts/asan.sh (isolated target dir).
ffi_toolchain_guard() {
  local active rlib built cleaned=0
  active=$(rustc -vV | sed -n 's/^release: //p')
  [[ -n "$active" ]] || return 0
  for rlib in "$REPO_ROOT"/target/release/deps/libloft_ffi-*.rlib; do
    [[ -e "$rlib" ]] || continue
    built=$(strings "$rlib" 2>/dev/null | grep -oiE 'rustc [0-9]+\.[0-9]+\.[0-9]+(-nightly)?' | head -1 | sed 's/^rustc //I')
    if [[ -n "$built" && "$built" != "$active" ]]; then
      echo "ffi-guard: libloft_ffi built by rustc $built ≠ active $active — removing stale rlib(s) so cargo rebuilds. Use scripts/asan.sh (isolated target dir) to avoid this." >&2
      rm -f "$REPO_ROOT"/target/release/deps/libloft_ffi-*.rlib
      cleaned=1
      break
    fi
  done
  return 0
}

# Keep native/wasm compiles OFF a small /tmp tmpfs.  The native test harness
# writes generated `.rs` + cached binaries via `scratch_dir()` (LOFT_TMPDIR,
# else temp_dir()), and rustc/rust-lld put their link intermediates under
# TMPDIR — both default to /tmp, which on this box is a ~7.5G tmpfs.  Parallel
# native compiles then exhaust it and the linker dies with SIGBUS (signal 7),
# manifesting as flaky, unrelated test failures.  Redirect both to a disk-backed
# dir, per-checkout (persistent, so the content-hash native cache survives
# run-to-run).  std::env::temp_dir() honours TMPDIR on Unix, so this one var also
# moves every `temp_dir()` user (cross_mode/exit_codes/html_wasm).  MUST live
# OUTSIDE the repo: a `target/`-relative TMPDIR breaks the package/registry tests
# (they build fixtures in temp_dir and package/extract them — anything under
# `target/` is excluded, so loft.toml goes missing).  /var/tmp is disk-backed.
#
# A sandbox, though, may not include /var/tmp in its write allow-list, so every
# test that writes via temp_dir() then fails "Read-only file system".  So VERIFY
# /var/tmp is actually writable (the dir can exist yet still be write-denied);
# if not, fall back to the INHERITED TMPDIR — a sandbox sets that to a writable,
# disk-backed scratch dir — or /tmp.  Non-sandbox machines are unaffected.
INHERITED_TMPDIR="${TMPDIR:-/tmp}"
LOFT_TEST_SCRATCH="/var/tmp/loft-test-scratch-$REPO_TAG"
if ! ( mkdir -p "$LOFT_TEST_SCRATCH" && touch "$LOFT_TEST_SCRATCH/.wtest" ) 2>/dev/null; then
	LOFT_TEST_SCRATCH="$INHERITED_TMPDIR/loft-test-scratch-$REPO_TAG"
	mkdir -p "$LOFT_TEST_SCRATCH"
fi
rm -f "$LOFT_TEST_SCRATCH/.wtest" 2>/dev/null
export TMPDIR="$LOFT_TEST_SCRATCH"
export LOFT_TMPDIR="$LOFT_TEST_SCRATCH"
LOG_DEFAULT=/tmp/loft_test.$REPO_TAG.log
OUT_DEFAULT=/tmp/loft_problems.$REPO_TAG.txt
OUT_STABLE=/tmp/loft_problems.txt
PID_FILE=/tmp/loft_test.$REPO_TAG.pid

# Refresh every derived artefact that the test suite depends on before
# running it.  There are three classes of stale artefact, each of which
# has caused a cascade of misleading test failures in the past:
#
#   1. Sibling cdylibs under lib/*/native/ — loaded by
#      `extensions::load_all`, linked by `--native`.  Source gains a
#      symbol → rustc: "cannot find function X in crate Y".
#   2. Test fixture cdylibs under tests/lib/*/native/ — native_loader
#      tests detect this and panic with a clear message, but one
#      detection panic per test is still one per test.
#   3. The wasm32-unknown-unknown rlib used by html_wasm tests —
#      the html_wasm suite checks staleness before running.
#   4. target/release/libloft.rlib — the one `--native` and the cdylib
#      tests link.  This step used to rebuild two of the three rlibs and
#      PRINT a timing row for each, which reads as "the rlibs are
#      handled"; the native one — the only rlib an ordinary `--bin loft`
#      edit loop makes stale — was the one left out, hidden behind the
#      two rows naming the ones it did build.  A whole green verdict was
#      read off a run whose native half linked a library four commits
#      old (2026-09-06) — `make check-rlib` says so in a second, but
#      only if you think to run it, and after the run it only tells
#      you the cycle was wasted.
#
# Cargo is incremental, so each step is ~free on a clean tree.  Logs
# go to /tmp/loft_cdylib.log so the test log stays focused on test
# output.  Failures here print a warning but do not stop the test run;
# the pre-existing in-test detection will surface the underlying
# problem with a specific rebuild command.
# Per-step wall-clock timings accumulate in /tmp/loft_timings.txt and
# stream live to stderr so a slow `--bg` start is visible.  Format:
#   `  cdylib lib/server/native       3.4s`  (always >= 0.1s precision)
# A `=== Wall-clock timing summary ===` block prints at run/wait end.
TIMINGS_FILE=/tmp/loft_timings.$REPO_TAG.txt

# Pick the test runner.  cargo-nextest parallelises at the test level
# (cargo-test only at the binary level), giving 2-3x faster wall-clock
# on the loft suite.  Falls back to plain `cargo test` if nextest
# isn't installed.  `--profile default` matches `.config/nextest.toml`'s
# fail-fast=off-by-default-flag-set / no-retries / immediate-failure
# settings.
test_runner_cmd() {
  if cargo nextest --version >/dev/null 2>&1; then
    # No `--release`: the test binaries are built in the dev/test profile, the SAME
    # profile `make ci` and CI's Test job use (@PLN159 phase B).  With `--release` here
    # a source edit cost THREE builds of the 267 test binaries — release for this loop,
    # dev for `cargo build --all-targets`, test for nextest — and none of the three was
    # reused by the next.  Now one build serves the loop and the gate.  The release
    # `loft` binary + `libloft.rlib` are still built by rebuild_native_cdylibs, because
    # the native harness links the release rlib and some tests spawn target/release/loft.
    local base="cargo nextest run --no-fail-fast --status-level fail"
    if [[ -n "${TEST_SELECT:-}" ]]; then
      echo "$base -E '$TEST_SELECT'"
    else
      echo "$base"
    fi
  else
    # No filterset support in plain `cargo test` — it runs everything, which is
    # the safe direction to fall back in.
    echo "cargo test --no-fail-fast"
  fi
}

# Sweep the disk-backed fixture scratch before a full run: tests write
# fixtures (and their .loft/cache native binaries) under target/test-tmp —
# per-run artifacts that otherwise accumulate run over run.  Owned by THIS
# repo's tests only, so the sweep can be unconditional.
sweep_test_tmp() {
  rm -rf "$(dirname "$0")/../target/test-tmp" 2>/dev/null || true
}

# Run all rebuilds in parallel.  Each cargo invocation has fixed
# startup overhead (~0.05–0.7 s on a no-op rebuild); doing them in
# parallel collapses the serial 1.5–2 s wall-clock to whatever the
# slowest single rebuild costs.  When something genuinely needs
# rebuilding the wins are larger (10s of seconds).
#
# Per-step timings are written to per-PID files and concatenated
# back into TIMINGS_FILE in submission order so the summary stays
# stable.
rebuild_one() {
  local label="$1" dir="$2" cmd="$3" log="$4" timing_file="$5"
  local start_ns end_ns elapsed_ms
  start_ns=$(date +%s%N)
  if ! bash -c "$cmd" >> "$log" 2>&1; then
    echo "warning: rebuild of $dir failed — see $log" >&2
  fi
  end_ns=$(date +%s%N)
  elapsed_ms=$(( (end_ns - start_ns) / 1000000 ))
  printf '  %-44s %6d.%03ds\n' "$label" \
    "$(( elapsed_ms / 1000 ))" "$(( elapsed_ms % 1000 ))" \
    > "$timing_file"
}

rebuild_native_cdylibs() {
  local repo_root
  repo_root=$(cd "$(dirname "$0")/.." && pwd)
  local log=/tmp/loft_cdylib.$REPO_TAG.log
  : > "$log"
  : > "$TIMINGS_FILE"
  local any_src_cdylib=0
  local timing_dir
  timing_dir=$(mktemp -d /tmp/loft_timings.XXXXXX)
  local rebuild_start_ns
  rebuild_start_ns=$(date +%s%N)

  echo "=== rebuild_native_cdylibs (parallel; per-step timings) ===" >&2

  local jobs=()
  local timing_files=()
  local idx=0

  schedule() {
    local label="$1" dir="$2" cmd="$3"
    local tf="$timing_dir/$idx"
    timing_files+=("$tf")
    idx=$(( idx + 1 ))
    rebuild_one "$label" "$dir" "$cmd" "$log" "$tf" &
    jobs+=($!)
  }

  # 1. Sibling cdylibs under lib/*/native/
  for manifest in "$repo_root"/lib/*/native/Cargo.toml; do
    [[ -f "$manifest" ]] || continue
    any_src_cdylib=1
    local dir
    dir=$(dirname "$manifest")
    local rel="${dir#"$repo_root"/}"
    echo "== rebuild $dir ==" >> "$log"
    schedule "cdylib $rel" "$dir" "cd '$dir' && cargo build --release -q"
  done

  # 2. Test fixture cdylibs under tests/lib/*/native/
  while IFS= read -r manifest; do
    [[ -f "$manifest" ]] || continue
    # Skip git-ignored manifests: orphaned generated fixtures left over from old
    # runs (e.g. tests/test_native_pkg/ from a prior plugin-ABI) are not part of
    # the tracked build and may no longer compile against the current tree.
    git -C "$repo_root" check-ignore -q "$manifest" 2>/dev/null && continue
    any_src_cdylib=1
    local dir
    dir=$(dirname "$manifest")
    local rel="${dir#"$repo_root"/}"
    echo "== rebuild $dir ==" >> "$log"
    schedule "cdylib $rel" "$dir" "cd '$dir' && cargo build --release -q"
  done < <(find "$repo_root/tests" -name Cargo.toml -not -path '*/target/*' 2>/dev/null)

  # 3. The wasm32-unknown-unknown rlib used by the html_wasm suite.
  #    Only rebuild if the target directory already exists — the very
  #    first run lets the --html driver build it so we don't impose a
  #    wasm-target install on developers who never touch the HTML gate.
  if [[ -d "$repo_root/target/wasm32-unknown-unknown" ]]; then
    echo "== rebuild wasm32-unknown-unknown rlib ==" >> "$log"
    schedule "wasm32 rlib" "$repo_root" \
      "cd '$repo_root' && cargo build --release --target wasm32-unknown-unknown --lib --no-default-features --features random -q"
  fi

  # 3b. The wasm32-wasip2 rlib that `--native-wasm` links (html_wasm's wasip2 cells).
  #     A DIFFERENT triple from 3, and it was missing here: any change to the loft lib
  #     left this rlib stale, so the first suite run after one failed with a compile
  #     error inside the generated wasm program ("no method named <the new fn>") that
  #     looks like a real regression and disappears on a re-run.  Same guard as above:
  #     only rebuild when the target dir already exists, so a developer who never
  #     touches the wasm gate is not made to install the target.
  if [[ -d "$repo_root/target/wasm32-wasip2" ]]; then
    echo "== rebuild wasm32-wasip2 rlib ==" >> "$log"
    schedule "wasip2 rlib" "$repo_root" \
      "cd '$repo_root' && cargo build --release --target wasm32-wasip2 --lib --no-default-features --features random -q"
  fi

  # 4. target/release/libloft.rlib — linked by `--native` and the cdylib
  #    tests.  Unconditional: it is the host target, so unlike 3/3b there
  #    is no toolchain to impose on anyone.  Incremental, so it is ~free
  #    when current and is exactly the cycle it saves when it is not.
  echo "== rebuild native libloft.rlib ==" >> "$log"
  schedule "native rlib" "$repo_root" \
    "cd '$repo_root' && cargo build --release --lib -q"

  # Wait for all parallel rebuilds; `wait` exits after the slowest.
  for pid in "${jobs[@]}"; do wait "$pid"; done

  # Collect timings — one file per scheduled job.  Concatenate in
  # scheduled order so the summary table reads top-to-bottom by
  # submission, even though jobs finished in arbitrary order.
  for tf in "${timing_files[@]}"; do
    if [[ -f "$tf" ]]; then
      cat "$tf" >> "$TIMINGS_FILE"
      cat "$tf" >&2
    fi
  done
  rm -rf "$timing_dir"

  local rebuild_end_ns
  rebuild_end_ns=$(date +%s%N)
  local rebuild_ms=$(( (rebuild_end_ns - rebuild_start_ns) / 1000000 ))
  printf '  %-44s %6d.%03ds\n' \
    "(rebuild_native_cdylibs total wall-clock)" \
    "$(( rebuild_ms / 1000 ))" "$(( rebuild_ms % 1000 ))" \
    | tee -a "$TIMINGS_FILE" >&2

  if [[ "$any_src_cdylib" -eq 0 ]]; then
    echo "no sibling cdylibs found — skipping freshness step" >&2
  fi
}

# Extract a compact failure summary from the raw log.
# $1: log path, $2: output path
summarise() {
  local log="$1" out="$2"
  do_summarise "$log" "$out"
  # Keep the documented stable path readable too (last writer wins when
  # several checkouts run concurrently; the per-checkout file is the
  # authoritative one and every mode prints its exact path).
  if [[ "$out" != "$OUT_STABLE" ]]; then
    cp -f "$out" "$OUT_STABLE" 2>/dev/null || true
  fi
}

do_summarise() {
  local log="$1" out="$2"
  {
    echo "=== Test binaries that reported FAILED ==="
    # Both runner formats: cargo test ("test ... FAILED") and nextest
    # ("        FAIL [   0.040s] (2256/2332) loft::wrap stack_trace_script"
    # + the closing "Summary [...] N failed") — the summary previously
    # missed every nextest failure and reported a failing run as clean.
    grep -a -E "^test .* FAILED$|^[[:space:]]*FAIL \[|failed;|[0-9]+ failed" "$log" \
      | grep -av " 0 failed" || echo "(none)"
    echo
    echo "=== Test stdout blocks for FAILED tests ==="
    grep -a -B1 -A10 "^---- " "$log" || echo "(none)"
    echo
    echo "=== SIGSEGV / signal crashes (with last context) ==="
    # For each SIGSEGV line, include the last 15 lines of context
    # before it — typically captures the last `run "tests/scripts/..."`
    # line so crashes point at a specific .loft file.
    if grep -aq "signal:" "$log"; then
      awk '
        /signal:/ {
          for (i = NR - 15; i < NR; i++) if (i > 0 && buf[i]) print buf[i]
          print "    *** " $0
          print "    ---"
        }
        { buf[NR] = $0 }
      ' "$log"
    else
      echo "(none)"
    fi
    echo
    echo "=== cargo-level target failures (compile or link) ==="
    grep -a -B1 -A3 "error: test failed\|error: .* target\(s\) failed\|error: could not compile" "$log" || echo "(none)"
    echo
    echo "=== panic! / thread panics (inline) ==="
    grep -a -B1 -A3 "thread .* panicked at" "$log" | head -80 || echo "(none)"
    # If a wrap-suite test SIGSEGV'd, cargo captured its stdout
    # into the void — re-run wrap with --nocapture to recover
    # the last `run "tests/scripts/..."` print before the crash.
    if grep -aq "wrap.* signal:" "$log" || grep -aq "test failed.*--test wrap" "$log"; then
      echo
      echo "=== wrap-suite SIGSEGV rerun with --nocapture ==="
      echo "(to recover the crashing script name)"
      cargo test --test wrap loft_suite -- --nocapture --test-threads=1 2>&1 \
        | grep -E '^(run |thread |test |error:|Caused|  process|Warning: [0-9]+ stores)' \
        | tail -50 || echo "(rerun failed)"
    fi
  } > "$out"
}

# `--peek`: look at the in-flight log without starting a run.
# ── What this run selects ───────────────────────────────────────────────────
# The DEFAULT is the curated set, and the full suite is what you reach for.  That
# way round because the expensive thing should cost a keystroke to ask for, not
# a keystroke to avoid: at 367s a full run is something you skip, and a check you
# skip is not a check.  The curated set is 3733 of 3833 tests in 67s, and it is
# built by EXCLUSION — see scripts/test_subjects.sh for what it drops and why.
source "$(dirname "${BASH_SOURCE[0]}")/test_subjects.sh"

SELECT_LABEL="curated"
TEST_SELECT="$(curated_filter)"
_args=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --full)
      SELECT_LABEL="full"; TEST_SELECT=""; shift ;;
    --subject)
      shift
      [[ $# -gt 0 ]] || { echo "--subject needs a name; try --list-subjects" >&2; exit 2; }
      if ! TEST_SELECT="$(subject_filter "$1")"; then
        echo "unknown subject '$1'.  Known: $(subject_names | tr '\n' ' ')" >&2
        exit 2
      fi
      SELECT_LABEL="subject:$1"; shift ;;
    --changed)
      # @PLN159 phase D — the subjects the diff touches (see changed_filter in
      # test_subjects.sh).  An optional ref widens the diff to a whole branch:
      # `--changed origin/main`.  Falls back to the curated set, saying why, when the
      # diff touches something every binary depends on.
      shift
      _ref="HEAD"
      if [[ $# -gt 0 && "$1" != --* && "$1" != /* ]]; then _ref="$1"; shift; fi
      if TEST_SELECT="$(changed_filter "$_ref")"; then
        SELECT_LABEL="changed:$_ref"
      else
        SELECT_LABEL="curated"; TEST_SELECT="$(curated_filter)"
      fi ;;
    --list-subjects)
      echo "subjects (use: --subject <name>):"
      for s in $(subject_names); do
        f="$(subject_filter "$s" 2>/dev/null)" || continue
        printf '  %-10s %2d binaries\n' "$s" "$(grep -o 'binary(' <<<"$f" | wc -l)"
      done
      echo
      echo "default (no flag) runs everything EXCEPT these, which are slow-and-few"
      echo "and covered by CI's unsharded Test job:"
      printf '  %s\n' "${HEAVY_BINARIES[@]}"
      un="$(unmatched_binaries | tr '\n' ' ')"
      [[ -n "${un// /}" ]] && { echo; echo "matched by no subject (still in the default run): $un"; }
      exit 0 ;;
    *) _args+=("$1"); shift ;;
  esac
done
set -- "${_args[@]+"${_args[@]}"}"

# Announced only by the modes that actually RUN something.  `--peek`/`--wait`/
# `--stop` inspect an existing run, and telling them which selection they would
# have used is noise about a decision they are not making.
announce_selection() { echo "selection: $SELECT_LABEL"; }

if [[ "${1:-}" == "--peek" ]]; then
  LOG="${2:-$LOG_DEFAULT}"
  if [[ ! -f "$LOG" ]]; then
    echo "no log at $LOG yet — run without --peek to start a fresh pass" >&2
    exit 1
  fi
  running="no"
  if [[ -f "$PID_FILE" ]] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
    running="yes (pid $(cat "$PID_FILE"))"
  fi
  echo "=== in-flight peek (log: $LOG, $(wc -l < "$LOG") lines, running=$running) ==="
  failures=$(grep -a -E "^test .* FAILED$|^[[:space:]]*FAIL \[" "$LOG" || true)
  segfaults=$(grep -a "signal:" "$LOG" || true)
  if [[ -z "$failures" && -z "$segfaults" ]]; then
    echo "no failures yet"
    echo "current tail:"
    tail -5 "$LOG"
    exit 0
  fi
  if [[ -n "$failures" ]]; then
    echo "$failures"
    echo
    grep -a -B1 -A10 "^---- " "$LOG" || true
  fi
  if [[ -n "$segfaults" ]]; then
    echo
    echo "SIGSEGV detected — last context before crash:"
    awk '
      /signal:/ {
        for (i = NR - 15; i < NR; i++) if (i > 0 && buf[i]) print buf[i]
        print "    *** " $0
      }
      { buf[NR] = $0 }
    ' "$LOG"
  fi
  exit 0
fi

# `--stop`: stop THIS checkout's background run (its wrapper + the nextest/rustc tree it
# spawned), scoped by PID file — never a broad `pkill -f nextest`, which would kill a sibling
# checkout's run and start a battle.
if [[ "${1:-}" == "--stop" ]]; then
  if [[ ! -f "$PID_FILE" ]] || ! kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
    echo "no background run for $REPO_ROOT (nothing to stop)"
    rm -f "$PID_FILE"
    exit 0
  fi
  pid=$(cat "$PID_FILE")
  # Recursively signal the whole descendant tree rooted at THIS wrapper pid (wrapper →
  # cargo-nextest → nextest runner → test procs), leaves first.  Scoped by pid, so a sibling
  # checkout's run is never touched — no broad `pkill -f nextest`.
  kill_tree() {
    local p=$1 sig=$2 c
    for c in $(pgrep -P "$p" 2>/dev/null); do kill_tree "$c" "$sig"; done
    kill "-$sig" "$p" 2>/dev/null || true
  }
  kill_tree "$pid" TERM
  for _ in 1 2 3; do kill -0 "$pid" 2>/dev/null || break; sleep 1; done
  kill_tree "$pid" KILL
  rm -f "$PID_FILE"
  echo "stopped background run (pid $pid) for $REPO_ROOT"
  exit 0
fi

# `--wait`: wait for a background run to finish, then summarise.
if [[ "${1:-}" == "--wait" ]]; then
  LOG="${2:-$LOG_DEFAULT}"
  OUT="${3:-$OUT_DEFAULT}"
  # A run that has ALREADY finished is a success, not an error.  The pid file is
  # removed by the background subshell the moment it is done, so a `--wait` that
  # starts a second late used to exit 1 with "no background run found" and lose a
  # completed result — the failure mode is worst exactly when the run was fastest.
  # Summarise whatever the log holds instead, and say which case this was.
  if [[ ! -f "$PID_FILE" ]]; then
    if [[ -f "$LOG" ]]; then
      echo "no run in flight — summarising the completed log at $LOG"
    else
      echo "no background run found (expected $PID_FILE) and no log at $LOG" >&2
      exit 1
    fi
  else
    pid=$(cat "$PID_FILE")
    echo "waiting for cargo test pid $pid..."
    while kill -0 "$pid" 2>/dev/null; do sleep 2; done
    rm -f "$PID_FILE"
  fi
  summarise "$LOG" "$OUT"
  echo
  echo "=== Wall-clock timing summary ==="
  if [[ -f "$TIMINGS_FILE" ]]; then
    cat "$TIMINGS_FILE"
  else
    echo "(no timings recorded — older log)"
  fi
  echo "wrote problems summary to $OUT"
  wc -l "$OUT"
  exit 0
fi

# Self-heal a cross-toolchain-polluted loft_ffi rlib before any build (run + --bg
# modes; --peek/--wait already exited above without building).
ffi_toolchain_guard

# `--bg`: start the run in the background and return immediately.
if [[ "${1:-}" == "--bg" ]]; then
  LOG="${2:-$LOG_DEFAULT}"
  OUT="${3:-$OUT_DEFAULT}"
  if [[ -f "$PID_FILE" ]] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
    echo "a background run is already in flight (pid $(cat "$PID_FILE"))" >&2
    echo "use --peek to inspect or --wait to block until it finishes" >&2
    exit 1
  fi
  # Remove stale bytecode caches so tests always compile fresh.
  find tests/ -name '*.loftc' -delete 2>/dev/null || true
  find /tmp -maxdepth 1 -name '*.loftc' -delete 2>/dev/null || true
  # Refresh sibling cdylibs before forking; see rebuild_native_cdylibs
  # for the rationale.  Runs in the foreground so the caller sees build
  # errors immediately, not 90 s later inside the test log.
  rebuild_native_cdylibs
  announce_selection
  # Tee via a subshell so the script returns after backgrounding.
  # `|| true` after the runner so summarise still fires when tests
  # fail (cargo's non-zero exit would otherwise short-circuit `set -e`
  # in the subshell, leaving /tmp/loft_problems.txt unwritten).
  RUNNER="$(sweep_test_tmp; test_runner_cmd)"
  echo "test runner: $RUNNER"
  (
    start_ns=$(date +%s%N)
    eval "$RUNNER" > "$LOG" 2>&1 || true
    end_ns=$(date +%s%N)
    elapsed_ms=$(( (end_ns - start_ns) / 1000000 ))
    printf '  %-44s %6d.%03ds\n' "$RUNNER" \
      "$(( elapsed_ms / 1000 ))" "$(( elapsed_ms % 1000 ))" \
      >> "$TIMINGS_FILE"
    summarise "$LOG" "$OUT"
    rm -f "$PID_FILE"
  ) > /dev/null 2>&1 &
  # ^ stdio detached: the subshell writes only to files, but an inherited
  #   stdout keeps a caller's pipe (`--bg | tail`) open until the whole
  #   suite ends — silently serialising the "background" run.
  echo "$!" > "$PID_FILE"
  echo "background run started (pid $!), log: $LOG, summary on finish: $OUT"
  echo "use --peek to inspect in flight, --wait to block until done"
  exit 0
fi

# Default: foreground run — stream output AND write summary.
LOG="${1:-$LOG_DEFAULT}"
OUT="${2:-$OUT_DEFAULT}"
find tests/ -name '*.loftc' -delete 2>/dev/null || true
find /tmp -maxdepth 1 -name '*.loftc' -delete 2>/dev/null || true
rebuild_native_cdylibs
announce_selection
RUNNER="$(sweep_test_tmp; test_runner_cmd)"
echo "test runner: $RUNNER"
fg_start_ns=$(date +%s%N)
# `|| true` so test failures don't short-circuit `set -e` and skip
# the post-run summary block.
eval "$RUNNER" 2>&1 | tee "$LOG" || true
fg_end_ns=$(date +%s%N)
fg_elapsed_ms=$(( (fg_end_ns - fg_start_ns) / 1000000 ))
printf '  %-44s %6d.%03ds\n' "$RUNNER" \
  "$(( fg_elapsed_ms / 1000 ))" "$(( fg_elapsed_ms % 1000 ))" \
  >> "$TIMINGS_FILE"
summarise "$LOG" "$OUT"
echo
echo "=== Wall-clock timing summary ==="
cat "$TIMINGS_FILE"
echo "wrote problems summary to $OUT"
wc -l "$OUT"
