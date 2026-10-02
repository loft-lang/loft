#!/bin/bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# The seconds-long checks a `make ci` would otherwise report only after minutes of building.
#
#   scripts/gate_preflight.sh            # the default set, ~45 s on this box
#   CI_PREFLIGHT=full scripts/gate_preflight.sh   # + doc_hygiene and frontend_counts via nextest
#
# `scripts/ci-run.sh start` runs this first and refuses to queue a gate that would fail on one
# of these.  Why it exists: `make ci` rebuilds the native fixtures and both wasm rlibs BEFORE
# `cargo fmt --check`, so an unformatted file was reported after ~4 minutes, and a derived row
# (QUALITY.md's audit tables) after ~17 — each one a full restart of a ~20-minute gate.  Measured
# 2026-09-28: three consecutive gates on one change, ended by rustfmt, then clippy's
# `too_many_lines`, then a derived row and a fixture — none of them needing a gate to find.
#
# Checks, each timed, all run (not stop-at-first), so one pass names every cheap failure:
#   fmt          `cargo fmt -- --check`                                    ~8 s
#   audit rows   QUALITY.md's `unspan` and `optional` rows against
#                `ir_walker_audit.py`, compared exactly as doc_hygiene's
#                `quality_*_table_matches_the_audit` compare them         ~6 s
#   doc drift    `scripts/check_doc_drift.sh -q`                           ~30 s
#   stdlib       `scripts/compiled_stdlib_fresh.py` — the compiled stdlib
#                was compiled from default/ as it is now; when it was not,
#                `make compiled-stdlib` regenerates it here and the step
#                says so (commit the regenerated file with the change)      <1 s, or ~2 min
#   full only    `cargo nextest run --release` over doc_hygiene and
#                frontend_counts — seconds of tests, minutes of compiling
#                when the release test binaries are stale
#
# Clippy is not here: both variants the gate runs compile the whole crate, which is the cost
# this script exists to avoid.  Run them by hand when the change touched Rust.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

failed=()
step() { # name, command...
  local name=$1; shift
  local t0=$SECONDS out
  if out=$("$@" 2>&1); then
    printf '  ok    %-11s %3ss\n' "$name" $((SECONDS - t0))
  else
    printf '  FAIL  %-11s %3ss\n' "$name" $((SECONDS - t0))
    printf '%s\n' "$out" | tail -8 | sed 's/^/        /'
    failed+=("$name")
  fi
}

audit_rows() {
  python3 - <<'EOF'
import re, subprocess, sys
q = open("doc/claude/QUALITY.md", encoding="utf-8").read().split("\n")
def row(header):
    i = next(k for k, l in enumerate(q) if l.startswith(header))
    return [int(c.strip().strip("*")) for c in q[i + 2].split("|") if c.strip().strip("*").isdigit()]
def audit(mode, label):
    out = subprocess.run(["python3", "scripts/ir_walker_audit.py", mode],
                         capture_output=True, text=True).stdout
    m = re.search(re.escape(label) + r"\s*:\s*(\d+)", out)
    return int(m.group(1)) if m else None
bad = []
for header, mode, label in (
    ("| sites a `Span` hides the shape from", "unspan", "neither — a `Span` hides the shape from them"),
    ("| opaque to a wrapped shape", "optional", "opaque to a wrapped shape"),
):
    have, now = row(header), audit(mode, label)
    if have != [now]:
        bad.append(f"QUALITY.md's {mode} row says {have}, `ir_walker_audit.py {mode}` reports {now}"
                   + (" — re-pin: `python3 scripts/ir_walker_audit.py optional --write-ratchet`"
                      " and the row" if mode == "optional" else " — update the row"))
print("\n".join(bad))
sys.exit(1 if bad else 0)
EOF
}

# fd 3 reaches the terminal past `step`'s capture: a step that SUCCEEDS by repairing something
# (the stdlib regeneration) still says what it changed.
exec 3>&1
echo "gate pre-flight (CI_NO_PREFLIGHT=1 skips it):"
step fmt cargo fmt -- --check
step "audit rows" audit_rows
step "doc drift" scripts/check_doc_drift.sh -q
# A stale compiled stdlib is REGENERATED, not refused: it is a derived file whose only cure is
# this command, and a gate on a stale one measures an interpreted stdlib (5-23x slower on text).
stdlib_fresh() {
  python3 scripts/compiled_stdlib_fresh.py >/dev/null 2>&1 && return 0
  echo "    the compiled stdlib was stale: regenerating (make compiled-stdlib)" >&3
  make -s compiled-stdlib || return 1
  python3 scripts/compiled_stdlib_fresh.py || return 1
  echo "    regenerated src/compiled_stdlib_gen.rs — commit it with this change" >&3
}
step stdlib stdlib_fresh
if [ "${CI_PREFLIGHT:-}" = full ]; then
  step "hygiene+fe" cargo nextest run --release -E 'binary(doc_hygiene) + binary(frontend_counts)'
fi

if [ ${#failed[@]} -gt 0 ]; then
  echo "pre-flight FAILED (${failed[*]}) — fix these first; a gate would stop on them."
  exit 1
fi
echo "pre-flight ok"
