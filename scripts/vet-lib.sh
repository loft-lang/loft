#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# vet-lib — run the library validation gate on an EXTERNAL (or own) library at a
# pinned source ref, and print a verdict for the admission decision.  The same gate
# own libs pass, so trust is uniform: a PASS is eligible for the next
# registry_maintain.sh fold-in; NEEDS-REVIEW carries native code (your one-time call);
# FAIL must not be admitted.
#
#   V2/V3  parity gate + compiles against THIS loft (interpret is the hard gate;
#          native is best-effort — an outside lib's system deps may be absent)
#   V5     metadata: name matches dir, license marker, version present
#   V6     risk tier: pure loft (auto-trustable) vs #native/#rust (human review),
#          plus the declared capabilities
#   V4     public API surface (informational — diff vs the prior version by eye)
#   V1     `loft package` sha256 + size (the entry the maintainer will sign)
#
# Usage:
#   scripts/vet-lib.sh <org/repo> <tag> [package] [--native]
#   e.g.  scripts/vet-lib.sh someone/cool-loft-lib widget-v0.1.0 widget
set -euo pipefail

usage() { echo "usage: scripts/vet-lib.sh <org/repo> <tag> [package] [--native]" >&2; exit 2; }
[ $# -ge 2 ] || usage
REPO="$1"; TAG="$2"; shift 2
PKG=""; WITH_NATIVE=0
while [ $# -gt 0 ]; do
  case "$1" in
    --native) WITH_NATIVE=1; shift ;;
    -*) usage ;;
    *) PKG="$1"; shift ;;
  esac
done

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# --manifest-path (not `cd $ROOT`) so the caller's CWD is preserved — the lib's
# `--tests tests` must resolve to the LIB's dir, not the loft repo's.
loft() { cargo run --quiet --manifest-path "$ROOT/Cargo.toml" --bin loft -- "$@"; }

tmp=$(mktemp -d -t vet-lib.XXXXXX); trap 'rm -rf "$tmp"' EXIT
echo "▶ cloning $REPO @ $TAG"
git clone --quiet --depth 1 --branch "$TAG" "https://github.com/$REPO.git" "$tmp/src" \
  || { echo "✗ cannot clone $REPO @ $TAG (bad repo/tag?)"; exit 1; }

# Locate the package dir (a dir with loft.toml).
if [ -z "$PKG" ]; then
  mapfile -t dirs < <(cd "$tmp/src" && for d in */; do [ -f "${d}loft.toml" ] && basename "$d"; done)
  if [ ${#dirs[@]} -eq 1 ]; then PKG="${dirs[0]}"
  else echo "multiple/zero packages: ${dirs[*]:-none} — pass one explicitly"; exit 2; fi
fi
DIR="$tmp/src/$PKG"
[ -f "$DIR/loft.toml" ] || { echo "✗ no loft.toml under package '$PKG'"; exit 1; }

fail=0; review=0
echo "──────── vetting $PKG @ $TAG ($REPO) ────────"

# V5 — metadata
name=$(sed -n 's/^name *= *"\(.*\)"/\1/p'    "$DIR/loft.toml" | head -1)
ver=$( sed -n 's/^version *= *"\(.*\)"/\1/p' "$DIR/loft.toml" | head -1)
echo "V5 metadata      : name=${name:-?} version=${ver:-?}"
[ -n "$ver" ] || { echo "   ✗ no version"; fail=1; }
# A package in a subdirectory (`lib/lavition_ui`) is named after its LAST component.
[ "$name" = "$(basename "$PKG")" ] || { echo "   ✗ name '$name' != package dir '$PKG'"; fail=1; }
if ! ls "$DIR"/LICENSE* >/dev/null 2>&1 && ! grep -rqi "SPDX-License-Identifier" "$DIR/src" 2>/dev/null; then
  echo "   ⚠ no license marker (LICENSE file or SPDX header)"
fi

# V6 — risk tier  (greps must tolerate no-match under `set -e`)
native_files=$( { grep -rlE '#native|#rust' "$DIR/src" 2>/dev/null || true; } | wc -l | tr -d ' ')
caps=$( { grep -rhoE '\b(fs|net|db|env|proc|time|rand)#[a-z_]+' "$DIR/src" 2>/dev/null || true; } | sort -u | paste -sd, )
if [ "$native_files" -gt 0 ]; then
  echo "V6 risk tier     : contains #native/#rust in $native_files file(s) → HUMAN REVIEW required"
  review=1
else
  echo "V6 risk tier     : pure loft — auto-trustable"
fi
[ -n "$caps" ] && echo "   declared capabilities: $caps"

# V2/V3 — parity gate + compiles against THIS loft (interpret = hard gate)
printf "V2/V3 interpret  : "
if ( cd "$DIR" && loft --interpret --tests tests ) >"$tmp/i.log" 2>&1; then
  echo "✓ green (parses/types/tests against this loft)"
else
  echo "✗ FAILED — see below"; { grep -iE 'error|fail' "$tmp/i.log" || true; } | sed -n '1,5s/^/     /p'; fail=1
fi
if [ "$WITH_NATIVE" = 1 ]; then
  printf "V2 native        : "
  if ( cd "$DIR" && LOFT_TIMEOUT=240 loft --native --tests tests ) >/dev/null 2>&1; then
    echo "✓ green"
  else
    echo "⚠ failed (best-effort — an outside lib's system deps may be missing on this machine)"
  fi
fi

# V4 — public API surface (informational)
echo "V4 API surface   :"
if ( cd "$DIR" && loft api . ) >"$tmp/api.txt" 2>/dev/null && [ -s "$tmp/api.txt" ]; then
  # Not `| head -25`: under `pipefail` a surface past 25 lines ends the script
  # with SIGPIPE (141), which reads as a FAILED vet.
  sed -n '1,25s/^/     /p' "$tmp/api.txt"
  [ "$(wc -l <"$tmp/api.txt")" -gt 25 ] && echo "     … ($(wc -l <"$tmp/api.txt") lines total)"
else
  echo "     (api-surface unavailable for this package)"
fi

# V1 — package integrity (the entry a maintainer will sign)
echo "V1 package       :"
if ( cd "$DIR" && loft package --tarball-only ) >/dev/null 2>"$tmp/package.err"; then
  for t in "$DIR"/*.tar.gz; do
    [ -f "$t" ] && echo "     sha256 $(sha256sum "$t" | cut -d' ' -f1)  size $(stat -c%s "$t")  $(basename "$t")"
  done
else
  echo "     ✗ loft package failed"
  sed -n '1,14p' "$tmp/package.err" | sed 's/^/       /'
  fail=1
fi

# V5 — the three compatibility levels (registry-ADMISSION gate)
#
# Separate from V1 on purpose.  Packaging is something a library does to itself
# and must keep working exactly as before; declaring the levels is what admission
# to the registry requires, because that is where other people start depending on
# the answer.  This is the cheapest place to learn it — before a tag, a release,
# or a registry PR exists.
echo "V5 compat levels :"
if ( cd "$DIR" && loft compat levels ) >"$tmp/levels.out" 2>&1; then
  sed -n '1,2p' "$tmp/levels.out" | sed 's/^/     /'
else
  sed -n '1,16p' "$tmp/levels.out" | sed 's/^/     /'
  echo "     → measure what they should say:  (cd $DIR && loft compat floor --with-tests)"
  fail=1
fi

echo "──────── verdict ────────"
if [ "$fail" = 1 ]; then
  echo "✗ FAIL — a gate failed; do NOT admit."; exit 1
elif [ "$review" = 1 ]; then
  echo "⚠ NEEDS REVIEW — automated gates pass, but it carries #native/#rust; your one-time admission call."; exit 3
else
  echo "✓ PASS — pure loft, all gates green; eligible for the next registry_maintain.sh fold-in."; exit 0
fi
