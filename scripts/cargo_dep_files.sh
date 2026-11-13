#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# The dependency files a cargo build profile holds, one per line, under either cargo layout:
# the classic `<profile>/deps/<pattern>`, and the per-unit layout (cargo 1.100 stable) that
# has no `deps/` and puts each crate in `<profile>/build/<crate>/<hash>/out/<pattern>`.
# The Rust side's twin is `loft::native_lib::dependency_dirs`.
#
#   scripts/cargo_dep_files.sh target/install-lib/release '*.rlib'
#   scripts/cargo_dep_files.sh target/release 'libloft_ffi-*.rlib'
#
# An unhashed `libloft.*` in a per-unit dir is the runtime itself (the profile's uplift), never
# a dependency, and is left out.
set -euo pipefail
if [ "${1:-}" = "--help" ] || [ $# -ne 2 ]; then
  echo "usage: scripts/cargo_dep_files.sh <profile-dir> <file-pattern>   e.g. target/release '*.rlib'"
  [ "${1:-}" = "--help" ] && exit 0 || exit 2
fi
profile=$1
pattern=$2
shopt -s nullglob
for f in "$profile"/deps/$pattern "$profile"/build/*/*/out/$pattern; do
  case "$(basename "$f")" in libloft.rlib|libloft.rmeta|libloft.so|libloft.dylib|loft.dll) continue ;; esac
  printf '%s\n' "$f"
done
