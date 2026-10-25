#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Install `origin/main`'s loft from the runtime its daily build shipped
# (`.github/workflows/daily-build.yml`, `scripts/runtime_bundle.sh`) instead of compiling it:
# the binary, the standard library, and the `libloft.rlib` + `deps/` that `--native` links.
# `make install-main` tries this first and builds locally when it answers 3.
#
#   scripts/install_main_ci.sh            # into $PREFIX (default ~/.local)
#   LOFT_DAILY_RUN=<run id> scripts/install_main_ci.sh   # that run's runtime (a branch's)
#
# Exit 0 installed; 3 not available here (no run for main's commit yet, no artifact for this
# host, `gh` missing, or this machine's rustc is not the one that built the rlib — an rlib
# links only against code from the SAME compiler); anything else is a failure.
set -euo pipefail
if [ "${1:-}" = "--help" ]; then
  sed -n '5,15p' "$0" | sed 's/^# \{0,1\}//'
  exit 0
fi
PREFIX=${PREFIX:-$HOME/.local}
unavailable() { echo "install_main_ci: $1 — building locally instead" >&2; exit 3; }
command -v gh >/dev/null || unavailable "no gh"
command -v rustc >/dev/null || unavailable "no rustc"
cd "$(dirname "$0")/.."
triple=$(rustc -vV | sed -n 's/^host: //p')
if [ -n "${LOFT_DAILY_RUN:-}" ]; then
  # One named run (a branch's, to try a runtime before its PR merges); its commit is stamped.
  run=$LOFT_DAILY_RUN
  sha=$(gh run view "$run" --json headSha -q .headSha)
else
  sha=$(git ls-remote origin refs/heads/main | cut -f1)
  [ -n "$sha" ] || unavailable "origin/main not reachable"
  run=$(gh run list --workflow daily-build.yml --branch main --event push --limit 30 \
          --json databaseId,headSha,conclusion \
          -q ".[] | select(.headSha == \"$sha\" and .conclusion == \"success\") | .databaseId" | head -1)
  [ -n "$run" ] || unavailable "no finished daily build for main ${sha:0:9} yet"
fi
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
gh run download "$run" -n "loft-daily-runtime-$triple" -D "$tmp/dl" >/dev/null 2>&1 \
  || unavailable "run $run has no runtime for $triple"
mkdir -p "$tmp/x"
tar -C "$tmp/x" -xzf "$tmp"/dl/loft-runtime-"$triple".tar.gz
root="$tmp/x/loft-runtime-$triple"
# The compiler that built the rlib, line for line against this machine's (release, commit).
want=$(grep -E '^(release|commit-hash):' "$root/share/loft/RUSTC")
have=$(rustc -vV | grep -E '^(release|commit-hash):')
if [ "$want" != "$have" ]; then
  unavailable "the runtime was built by rustc $(sed -n 's/^release: //p' <<<"$want"), this machine has $(sed -n 's/^release: //p' <<<"$have")"
fi
sum() { if command -v sha256sum >/dev/null; then sha256sum "$@"; else shasum -a 256 "$@"; fi; }
(cd "$root" && while read -r h f; do
   [ "$(sum "$f" | cut -d' ' -f1)" = "$h" ] || { echo "install_main_ci: $f does not match SHA256SUMS" >&2; exit 1; }
 done < SHA256SUMS)
exe=loft
case "$triple" in *windows*) exe=loft.exe ;; esac
mkdir -p "$PREFIX/bin" "$PREFIX/share/loft"
rm -rf "$PREFIX/share/loft/default" "$PREFIX/share/loft/deps"
rm -f "$PREFIX/share/loft/libloft.rlib" "$PREFIX/share/loft/libloft.rmeta"
cp -r "$root/share/loft/." "$PREFIX/share/loft/"
install -m 755 "$root/bin/$exe" "$PREFIX/bin/$exe"
printf '%s clean\n' "$sha" > "$PREFIX/share/loft/SOURCE"
# Both halves, as `make install` checks the interpreter half: a runtime that cannot link a
# program is the one failure the download exists to avoid.
smoke="$tmp/smoke.loft"
printf 'fn main() {\n    println("install-main smoke ok")\n}\n' > "$smoke"
for mode in --interpret --native; do
  out=$(LOFT_TIMEOUT=300 "$PREFIX/bin/$exe" "$mode" "$smoke" 2>&1) || true
  [ "$out" = "install-main smoke ok" ] || { echo "install_main_ci: $mode smoke failed:" >&2; echo "$out" | tail -5 >&2; exit 1; }
done
echo "install_main_ci: $PREFIX/bin/$exe is $sha clean (daily build run $run, $triple)"
