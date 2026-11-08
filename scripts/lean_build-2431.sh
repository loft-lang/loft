#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Build loft the way a production install ships it: `make install`'s feature list, into
# target/install-lib.  The list is `default` without the development aids — today
# `strict-stores` (LOFT_STRICT_STORES, a use-after-free probe whose per-access test costs a
# store-heavy routine 4-11 %, bench/portal/analysis/store-checks.md) and `stack-verify`
# (LOFT_VERIFY_STACK, the interpreter stack's init shadow, tested on every store write).
# The ONE home of that list: `make install` builds the binary and runtime it installs through this,
# scripts/make-release.sh the bundle's binary (and scripts/repro-verify.sh rebuilds it the
# same way), and bench/stats.py the pair its lanes measure, because a perf number taken on
# the development build is not a measurement of what ships (NATIVE.md § Optimisation tiers).
#
#   scripts/lean_build.sh --lib --bin loft            # what `make install` installs, and
#                                                     # the pair a perf lane runs
#   scripts/lean_build.sh --bin loft --target <triple>  # a release bundle's binary
#
# The binary crate ALONE (`--bin loft` without `--lib`, the release bundle's build) is compiled
# as one codegen unit.  Split into rustc's default sixteen, the unit that holds the MSVC
# `type_info` vtable reference (`??_7type_info@@6B@`, the `rust_panic` exception descriptor's)
# is not the same from build to build — unit 04 or 06 — and the linker lays `.data` out by
# object order, so about one Windows rebuild in ten differed from the release (loft#2431,
# found by comparing rust-lld's LLD_REPRODUCE records of a matching and a differing link).  One
# unit leaves no placement to choose.  The library keeps its units: the interpreter's code is
# there, and the binary crate's single unit measured no slower over the bench suite.  The
# release and its verifier read this file from the same source archive, so they agree.
set -euo pipefail
if [ "${1:-}" = "--help" ]; then
  echo "usage: scripts/lean_build.sh <cargo build args>  — the production feature list, into target/install-lib"
  exit 0
fi
cd "$(dirname "$0")/.."
features=(--release --no-default-features
  --features mmap,random,threading,native-extensions,registry,remote-store
  --target-dir target/install-lib)
bin_loft=0 lib=0 prev=
for a in "$@"; do
  [ "$prev" = --bin ] && [ "$a" = loft ] && bin_loft=1
  [ "$a" = --lib ] && lib=1
  prev=$a
done
if [ "$bin_loft" = 1 ] && [ "$lib" = 0 ]; then
  exec cargo rustc "${features[@]}" "$@" -- -C codegen-units=1
fi
exec cargo build "${features[@]}" "$@"
