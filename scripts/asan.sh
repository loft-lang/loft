#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Run the AddressSanitizer suite locally in an ISOLATED target dir.
#
# WHY: ASan needs a nightly toolchain (`-Zsanitizer=address`).  A nightly build
# run into the shared `target/` recompiles everything with nightly rustc and an
# instrumented RUSTFLAGS, and the next stable build recompiles it all again.
# Keeping nightly artifacts in `target/asan/` leaves the stable `target/` alone, so
# `find_problems.sh` and the gate keep reusing their stable artifacts.  (CI's
# `asan` job in .github/workflows/miri.yml is safe already — it runs on a fresh,
# separately-cached checkout; this wrapper gives local runs the same isolation.)
#
# Mirrors that CI job's flags.  Extra args pass through to nextest, e.g.
#   ./scripts/asan.sh -E 'test(store_)'
#
# Two of those flags are load-bearing, and running without them fails in ways that
# look like a code problem but are not:
#
#   --target x86_64-unknown-linux-gnu
#       Scopes RUSTFLAGS to the TARGET, so host proc-macros and build scripts stay
#       uninstrumented.  Without it, `-Zsanitizer=address` is applied to the
#       proc-macro crates too and the build dies in a dependency with
#       `E0463: can't find crate for zerofrom_derive`.
#
#   +$TOOLCHAIN (pinned)
#       A bare `+nightly` follows whatever nightly is installed; a recent one fails
#       to compile `curve25519-dalek` (avx512 backend).  CI pins this exact nightly
#       (ci.yml, the `asan` job) — track it here or local runs diverge from CI.
#       Override with ASAN_TOOLCHAIN=... when CI's pin moves.
#
# WHAT THIS GATE ACTUALLY COVERS.  loft's Store is ONE arena allocation, and ASan
# checks allocation bounds — so a read past a RECORD but still inside the arena is
# invisible here (measured, not assumed).  ASan catches WILD offsets that leave the
# arena.  For intra-arena bounds use `-C debug-assertions=on`, which turns on
# `Store::valid()` (CI's `debug-asserts` job); loft's ~200 `debug_assert!`s are
# compiled out of normal builds by `[profile.dev.package.loft]`.
set -euo pipefail

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
TOOLCHAIN=${ASAN_TOOLCHAIN:-nightly-2026-05-28}

if ! rustc "+$TOOLCHAIN" --version >/dev/null 2>&1; then
  echo "asan.sh: needs $TOOLCHAIN (rustup toolchain install $TOOLCHAIN)" >&2
  exit 1
fi

exec env \
  CARGO_TARGET_DIR="$REPO_ROOT/target/asan" \
  RUSTFLAGS='-Zsanitizer=address' \
  ASAN_OPTIONS='detect_leaks=0' \
  cargo "+$TOOLCHAIN" nextest run --profile ci --release \
  --target x86_64-unknown-linux-gnu "$@"
