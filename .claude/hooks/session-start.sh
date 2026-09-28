#!/bin/bash
# A Claude Code on the web session runs on a fresh container with a FIXED writable
# allowance, and the disk — never the CPU — is what ends a gate there: measured
# 2026-09-28, three runs died mid-link (`ld terminated with signal 7 [Bus error]`,
# `No space left on device`) after `target/debug` reached 16 GB of test binaries plus
# a 5 GB incremental cache, and each death left a log with no verdict.  This hook sets
# the session up so that does not happen: no incremental cache (every target is built
# once per session anyway), the leftovers a previous session cannot have needed swept,
# and the headroom printed where the agent reads it.  Local sessions are untouched.
# CI_BUDGET.md § A cloud session has the measurements.
set -euo pipefail

if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

cd "$CLAUDE_PROJECT_DIR"

# Cargo's incremental cache is a per-edit accelerator; in a session that builds each
# target once it is 5 GB of pure cost.
echo 'export CARGO_INCREMENTAL=0' >> "$CLAUDE_ENV_FILE"

# Leftovers of a previous session on this image: the incremental cache, `make falsify`'s
# control builds (a full release target per ref), loft's own native scratch.
rm -rf target/debug/incremental target/release/incremental
rm -rf /root/.cache/loft-falsify/*-target
make sweep-scratch >/dev/null 2>&1 || true

# The headroom, once, where it is read before the first heavy run.
df -h / | awk 'NR==2 {printf "disk: %s free of the session allowance (%s used)\n", $4, $3}'
