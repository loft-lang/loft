#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN117 — run the five headless browser-threading gates and print one verdict.
#
#   scripts/par_gates.sh          # local (also `make par-gates`)
#   scripts/par_gates.sh --ci     # CI: a SKIPPED gate is a FAILURE
#
# Why --ci exists.  Every gate exits 0 when a prerequisite is missing (no
# threaded bundle, no chromium, no nightly), so a dev without them still gets a
# useful run.  On the runner all of them are installed, so "skipped" can only
# mean the provisioning broke — and a gate that skips itself green is exactly
# the silent rot this suite is here to prevent.  --ci therefore preflights the
# prerequisites, fails on any SKIP, and writes a per-gate table to the job
# summary so the scaling curve stays on the record.
#
# The gates: dispatch + fallback + nested par (par-thread-proof), the
# shared-memory model (par-memory-proof), scaling (par-scaling-bench), UI
# responsiveness (par-ui-responsive), and a `loft --html` page that uses `par`
# (html-thread-proof).  par-scaling-bench and par-ui-responsive read thresholds
# from the environment because runner hardware is smaller than this dev box; the
# workflow .github/workflows/browser-threads.yml sets them.
#
# One gate here is not about threading at all: html-unavailable-builtin proves a
# builtin the browser cannot serve still builds and answers at runtime
# (loft#709).  It rides along because this is where a headless chromium exists
# in CI, and a second workflow for one gate would cost more than it explains.
set -u
cd "$(dirname "$0")/.."

ci=0
case "${1:-}" in
  --ci) ci=1 ;;
  "") ;;
  *) echo "usage: $0 [--ci]" >&2; exit 2 ;;
esac

GATES="par-thread-proof.sh par-memory-proof.sh par-scaling-bench.sh par-ui-responsive.sh html-thread-proof.sh html-unavailable-builtin.sh"

# Preflight: in CI a missing prerequisite must be named up front, not inferred
# from five identical SKIP lines twenty minutes later.
if [ $ci -eq 1 ]; then
  pre=0
  [ -f tests/wasm/pkg-mt/loft.js ] || { echo "::error::no threaded bundle at tests/wasm/pkg-mt/ — 'make wasm-mt' did not run"; pre=1; }
  [ -x target/release/loft ] || { echo "::error::no target/release/loft — 'cargo build --release' did not run"; pre=1; }
  command -v python3 >/dev/null || { echo "::error::no python3 — the COOP/COEP report server cannot start"; pre=1; }
  command -v chromium >/dev/null || command -v chromium-browser >/dev/null || command -v google-chrome >/dev/null \
    || { echo "::error::no headless chromium/chrome on PATH"; pre=1; }
  rustup run nightly rustc --version >/dev/null 2>&1 \
    || { echo "::error::no nightly toolchain — the threaded bundle rebuilds std with atomics"; pre=1; }
  [ $pre -eq 0 ] || { echo "PREFLIGHT FAILED — the gates would have skipped themselves green."; exit 1; }
fi

fail=0
rows=""
add_row() { rows="$rows$1
"; }
for g in $GATES; do
  echo "══════════════ $g"
  out="$(tests/wasm/$g 2>&1)"; rc=$?
  printf '%s\n' "$out"
  # The gate's own last word, so the summary carries its numbers (the scaling
  # curve, the worker count) rather than a bare pass/fail.
  note="$(printf '%s\n' "$out" | grep -m1 -E '^ *(PASS|SKIP|FAIL)' | sed 's/^ *//')"
  if printf '%s\n' "$out" | grep -q '^SKIP:'; then
    if [ $ci -eq 1 ]; then
      echo "::error::$g SKIPPED — a prerequisite is missing; the gate never ran"
      add_row "❌ \`$g\` — SKIPPED (fatal in CI): ${note#SKIP: }"
      fail=1
    else
      add_row "⤵️ \`$g\` — ${note}"
    fi
  elif [ $rc -eq 0 ]; then
    add_row "✅ \`$g\` — ${note#PASS: }"
  else
    [ $ci -eq 1 ] && echo "::error::$g FAILED"
    add_row "❌ \`$g\` — ${note:-exit $rc}"
    fail=1
  fi
done

emit() {
  printf '%s\n' "$1"
  [ -n "${GITHUB_STEP_SUMMARY:-}" ] && printf '%s\n' "$1" >> "$GITHUB_STEP_SUMMARY"
  return 0
}
echo
emit "### headless browser gates (@PLN117 threading + loft#709 host surface)"
emit ""
# Note the trailing newline: `read` drops a final unterminated line, which
# silently cost the LAST gate its row in the table.
printf '%s\n' "$rows" | tr '|' '\n' | while IFS= read -r row; do
  [ -n "$row" ] && emit "- $row"
done
emit ""
if [ $fail -eq 0 ]; then
  emit "**PASS** — \`par\` threads in the browser on every measured shape."
else
  emit "**FAIL** — see the failing gate above."
fi
exit $fail
