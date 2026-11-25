#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Package the installed layout of a lean build — `bin/loft` and `share/loft/default/`, the
# standard library — as one tarball a machine can unpack into its prefix instead of compiling
# loft.  `scripts/install_main_ci.sh` is its reader.  `--native` needs nothing more than this:
# it compiles through C with the machine's own C compiler.
#
#   scripts/runtime_bundle.sh <triple> <out-dir>
#
# Run after `scripts/lean_build.sh --bin loft --target <triple>`.  `SHA256SUMS` covers every
# file.
set -euo pipefail
if [ "${1:-}" = "--help" ] || [ $# -ne 2 ]; then
  echo "usage: scripts/runtime_bundle.sh <triple> <out-dir>  — bin/loft + share/loft (stdlib) as a tarball"
  [ "${1:-}" = "--help" ] && exit 0
  exit 2
fi
triple=$1
out=$2
cd "$(dirname "$0")/.."
rel="target/install-lib/$triple/release"
exe=loft
case "$triple" in *windows*) exe=loft.exe ;; esac
[ -f "$rel/$exe" ] || { echo "runtime_bundle: no $rel/$exe — run scripts/lean_build.sh --bin loft --target $triple" >&2; exit 1; }
name="loft-runtime-$triple"
stage="$out/$name"
rm -rf "$stage"
mkdir -p "$stage/bin" "$stage/share/loft"
cp "$rel/$exe" "$stage/bin/$exe"
cp -r default "$stage/share/loft/"
sum() { if command -v sha256sum >/dev/null; then sha256sum "$@"; else shasum -a 256 "$@"; fi; }
(cd "$stage" && find . -type f ! -name SHA256SUMS | LC_ALL=C sort | while read -r f; do sum "$f"; done > SHA256SUMS)
tar -C "$out" -czf "$out/$name.tar.gz" "$name"
rm -rf "$stage"
echo "runtime_bundle: $out/$name.tar.gz ($(du -h "$out/$name.tar.gz" | cut -f1))"
