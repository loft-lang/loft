#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Does THIS source rebuild to the same bytes somewhere else?
#
# `repro-verify.sh` answers that for a PUBLISHED release: it downloads the bundle, its
# source archive and its rustc, and compares.  That makes it honest and late — it can only
# run after a release exists, on a weekly schedule, over the network.  A reproducibility
# break therefore surfaces long after the commit that caused it, attributed to nothing.
#
# This is the same question with the release taken out of it.  Build twice from one commit:
# once in the checkout (a git repo, the shape a release is cut from) and once from a `git
# archive` unpacked elsewhere (no `.git`, the shape a verifier rebuilds from).  If the two
# differ, the source is not reproducible, and it says so on the commit that did it.
#
# That difference is not hypothetical: it is exactly how `LOFT_BUILD_ID` broke every
# published target.  `build.rs` fell back to a TIMESTAMP when git was absent, so the
# verifier's copy could never match the release's commit stamp — and the weekly job blamed
# the source for a stamp the build had invented.  This probe reproduces that in one run,
# with no release and no network.
#
# Usage:
#   scripts/repro-roundtrip.sh            # round-trip the working tree's HEAD
#
# Exit 0 identical · 1 differs · 2 cannot run (missing tool / dirty tree).

set -euo pipefail
cd "$(dirname "$0")/.."

die() { echo "repro-roundtrip: $*" >&2; exit 2; }

command -v git >/dev/null || die "git is required"
git rev-parse --git-dir >/dev/null 2>&1 || die "not a git checkout — nothing to archive"

COMMIT=$(git rev-parse --short HEAD)
ROOT=$(mktemp -d "${TMPDIR:-/tmp}/loft-roundtrip.XXXXXX")
# A probe that reports a difference and then deletes both binaries has told you there is a
# bug and taken away the only way to find it.  Kept on failure; removed when identical.
cleanup() { [ "${KEEP:-0}" = 1 ] || rm -rf "$ROOT"; }
trap cleanup EXIT

SHA=sha256sum; command -v sha256sum >/dev/null || SHA="shasum -a 256"

echo "repro-roundtrip: commit $COMMIT"

# The version both builds answer.  A release is built with `LOFT_VERSION` set to its tag
# (release.yml), and its source archive has no git, so it answers `Cargo.toml` — the same
# string.  This probe builds an arbitrary commit, which a checkout would stamp as a dated
# daily (`build.rs` `effective_version`); pinning it to `Cargo.toml` on both sides keeps the
# comparison about the build rather than the date.
REL=$(grep -m1 '^version = ' Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')

# A — the checkout. A release is cut here, where `git rev-parse` answers, so `build.rs`
#     bakes the commit.  Its own target dir: the caller's must not be disturbed, and a
#     shared one would let the second build reuse the first's artifacts and pass trivially.
echo "   [A] building in the checkout"
( . scripts/repro-flags.sh \
  && LOFT_VERSION="$REL" CARGO_TARGET_DIR="$ROOT/ta" CARGO_INCREMENTAL=0 cargo build --release --bin loft ) \
  >"$ROOT/a.log" 2>&1 || { tail -20 "$ROOT/a.log"; die "build A failed"; }

# B — an exported tree with NO `.git`, which is what a verifier unpacks.  `LOFT_BUILD_ID`
#     carries the commit across, because git cannot answer there; that hand-off is the
#     whole contract this probe exists to hold.
echo "   [B] building from a git archive (no .git)"
mkdir -p "$ROOT/src"
git archive HEAD | tar -x -C "$ROOT/src"
[ -e "$ROOT/src/.git" ] && die "the export still carries .git — it would not test anything"
( cd "$ROOT/src" && . "$ROOT/src/scripts/repro-flags.sh" \
  && LOFT_BUILD_ID="$COMMIT" LOFT_VERSION="$REL" CARGO_TARGET_DIR="$ROOT/tb" CARGO_INCREMENTAL=0 \
     cargo build --release --bin loft ) \
  >"$ROOT/b.log" 2>&1 || { tail -20 "$ROOT/b.log"; die "build B failed"; }

a_bin="$ROOT/ta/release/loft"; b_bin="$ROOT/tb/release/loft"
[ -f "$a_bin" ] || die "no binary from build A"
[ -f "$b_bin" ] || die "no binary from build B"

a=$($SHA "$a_bin" | cut -d' ' -f1)
b=$($SHA "$b_bin" | cut -d' ' -f1)
echo "   [A] $a"
echo "   [B] $b"

if [ "$a" = "$b" ]; then
  echo "IDENTICAL — $COMMIT reproduces from an exported tree"
  exit 0
fi

sa=$(wc -c < "$a_bin"); sb=$(wc -c < "$b_bin")
echo "DIFFERS — checkout $sa bytes, exported $sb bytes"
if [ "$sa" = "$sb" ]; then
  echo "  (same size: look for embedded absolute paths or timestamps)"
else
  echo "  (different size: a genuinely different build — a stamp, a feature set, or a dep)"
fi
# The likeliest single cause, named because it has happened and costs an afternoon to
# rediscover: a value baked by build.rs that git can answer for and an export cannot.
echo "  first suspect: something build.rs derives from the repo (see LOFT_BUILD_ID)"
KEEP=1
echo "  both binaries kept for diffing: $ROOT"
echo "    A $a_bin"
echo "    B $b_bin"
exit 1
