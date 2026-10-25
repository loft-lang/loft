#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Write one golden image set (formal/structures.md § Conformance): every stored structure's
# image and sidecar, read by tests/scripts/golden-images-{whole,paged}.loft with every later build.
#
#   scripts/golden_images.sh <set>      # writes tests/golden/images/<set>/
#
# Name a set after the placement tokens that wrote it (src/placement.rs: h<HASH>-i<INDEX>-
# r<RADIX>-t<TRIE>).  A set that exists is refused: a committed image is never written again,
# because the guard's whole point is that its bytes come from an EARLIER build.  The hash image
# is written once per layout (Col-Layouts), each in a process of its own, since the layout a
# new table takes is the writing process's policy.  The seed is fixed so a set is reproducible.
set -euo pipefail
cd "$(dirname "$0")/.."
case "${1:-}" in
  -h|--help) sed -n '5,14p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
esac
set_name="${1:?usage: scripts/golden_images.sh <set>   (e.g. h3-i2-r1-t1)}"
dir="tests/golden/images/$set_name"
if [ -e "$dir" ]; then
  echo "golden_images: $dir exists — a committed set is never written again; name a new set" >&2
  exit 1
fi
loft="${LOFT_BIN:-target/release/loft}"
mkdir -p "$dir"
gen=tests/golden/images/generate.loft
export LOFT_GOLDEN_DIR="$PWD/$dir" LOFT_HASH_SEED=2172
LOFT_GOLDEN_HASH=inline "$loft" --interpret "$gen"
LOFT_GOLDEN_HASH=tagged LOFT_NO_HASH_INLINE=1 "$loft" --interpret "$gen"
LOFT_GOLDEN_HASH=untagged LOFT_NO_HASH_INLINE=1 LOFT_NO_HASH_TAG=1 "$loft" --interpret "$gen"
ls -l "$dir"
