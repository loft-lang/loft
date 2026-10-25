#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Package the installed layout of a lean build — `bin/loft` and `share/loft/` with the
# standard library, `libloft.rlib` and its `deps/` — as one tarball a machine can unpack
# into its prefix instead of compiling loft.  The release bundle (scripts/make-release.sh)
# stays byte-identical to a release and carries no runtime library; this is the other half,
# what `--native` links.  `scripts/install_main_ci.sh` is its reader.
#
#   scripts/runtime_bundle.sh <triple> <out-dir>
#
# Run after `scripts/lean_build.sh --lib --bin loft --target <triple>`.  An rlib links only
# against code from the SAME rustc, so `share/loft/RUSTC` records `rustc -vV` and the
# installer refuses a machine whose compiler differs.  `SHA256SUMS` covers every file.
set -euo pipefail
if [ "${1:-}" = "--help" ] || [ $# -ne 2 ]; then
  echo "usage: scripts/runtime_bundle.sh <triple> <out-dir>  — bin/loft + share/loft (rlib, deps, stdlib) as a tarball"
  [ "${1:-}" = "--help" ] && exit 0
  exit 2
fi
triple=$1
out=$2
cd "$(dirname "$0")/.."
build_host=$(rustc -vV | sed -n 's/^host: //p')
if [ "$build_host" != "$triple" ]; then
  echo "runtime_bundle: $triple cross-built on $build_host — its proc-macros are $build_host code, so no runtime ships" >&2
  exit 1
fi
rel="target/install-lib/$triple/release"
exe=loft
case "$triple" in *windows*) exe=loft.exe ;; esac
for f in "$rel/$exe" "$rel/libloft.rlib"; do
  [ -f "$f" ] || { echo "runtime_bundle: no $f — run scripts/lean_build.sh --lib --bin loft --target $triple" >&2; exit 1; }
done
name="loft-runtime-$triple"
stage="$out/$name"
rm -rf "$stage"
mkdir -p "$stage/bin" "$stage/share/loft/deps"
cp "$rel/$exe" "$stage/bin/$exe"
cp -r default "$stage/share/loft/"
cp "$rel/libloft.rlib" "$stage/share/loft/"
[ -f "$rel/libloft.rmeta" ] && cp "$rel/libloft.rmeta" "$stage/share/loft/"
cp "$rel"/deps/*.rlib "$stage/share/loft/deps/"
# Proc-macro crates are HOST dynamic libraries, and a `--target` build keeps host code apart:
# in `target/install-lib/release/deps/`, not the triple's directory.  A package's native build
# loads them, so they come too — which is why a cross-built leg cannot ship a runtime.
host="target/install-lib/release/deps"
for dir in "$rel/deps" "$host"; do
  for ext in so dylib dll; do
    if ls "$dir"/*."$ext" >/dev/null 2>&1; then cp "$dir"/*."$ext" "$stage/share/loft/deps/"; fi
  done
done
# Only the newest loft_ffi rlib: an older one beside it is a second candidate for one crate.
# Judged in the build directory: the copies all carry the time of the copy.
for old in $(ls -t "$rel"/deps/libloft_ffi-*.rlib 2>/dev/null | tail -n +2 || true); do
  rm -f "$stage/share/loft/deps/$(basename "$old")"
done
rustc -vV > "$stage/share/loft/RUSTC"
sum() { if command -v sha256sum >/dev/null; then sha256sum "$@"; else shasum -a 256 "$@"; fi; }
(cd "$stage" && find . -type f ! -name SHA256SUMS | LC_ALL=C sort | while read -r f; do sum "$f"; done > SHA256SUMS)
tar -C "$out" -czf "$out/$name.tar.gz" "$name"
rm -rf "$stage"
echo "runtime_bundle: $out/$name.tar.gz ($(du -h "$out/$name.tar.gz" | cut -f1))"
