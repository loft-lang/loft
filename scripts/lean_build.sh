#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Build loft the way a production install ships it: `make install`'s feature list, into
# target/install-lib.  The list is `default` without the development aids — today
# `strict-stores` (LOFT_STRICT_STORES, a use-after-free probe whose per-access test costs a
# store-heavy routine 4-11 %, bench/portal/analysis/store-checks.md).  The ONE home of that
# list: `make install` builds the binary and runtime it installs through this,
# scripts/make-release.sh the bundle's binary (and scripts/repro-verify.sh rebuilds it the
# same way), and bench/stats.py the pair its lanes measure, because a perf number taken on
# the development build is not a measurement of what ships (NATIVE.md § Optimisation tiers).
#
#   scripts/lean_build.sh --lib --bin loft            # what `make install` installs, and
#                                                     # the pair a perf lane runs
#   scripts/lean_build.sh --bin loft --target <triple>  # a release bundle's binary
set -euo pipefail
cd "$(dirname "$0")/.."
exec cargo build --release --no-default-features \
  --features mmap,random,threading,native-extensions,registry,remote-store \
  --target-dir target/install-lib "$@"
