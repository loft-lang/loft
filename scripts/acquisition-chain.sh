#!/bin/sh
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# acquisition-chain.sh — the end-to-end path a USER takes to get this release, run as
# one gate: real transport, signed-index anchor, and a program actually executing.
#
# @PLN156 phase 2.  This chain was verified by hand exactly once (2026-08-31, in a
# throwaway clone) after being assumed for every prior release — and for every prior
# release it was false: the registry carried no toolchain entry, so `verify-self` had
# never anchored any installation anywhere.  This script is that verification as a
# per-release gate instead of a rescue.
#
# The chain, each step failing loudly:
#   1. scripts/install.sh --version <v> into a throwaway prefix — the documented
#      curl|sh path over the REAL transport (GitHub CDN + the sidecar it serves).
#   2. bin/loft --version names <v> — the artifact is the release it claims.
#   3. bin/loft self-update --dry-run --refresh RESOLVES — the signed index reaches
#      this binary.  "no releases published to compare against" is a FAILURE here,
#      never a quiet line: a cache predating the splice prints the same words as an
#      empty index, which is why --refresh is not optional (RELEASE.md step 4).
#   4. bin/loft verify-self reports the ANCHOR: the literal
#      "origin: matches the signed registry index".  Exit 0 alone is NOT the pass —
#      an unreachable registry SKIPS the origin check and still exits 0, and a check
#      that was skipped must never read as a check that passed (@PLN156's one rule).
#   5. The installed loft runs a program and its output is asserted.
#
# Exit codes (scripts/release-checklist.py reads them apart):
#   0  the whole chain held
#   3  the release is not acquirable YET (download failed) — "could not run", not red
#   1  the chain is broken: a step that could run answered wrong
#
# Usage:  scripts/acquisition-chain.sh --version 2026.9.0 [--keep]
set -u

VERSION=""
KEEP=0
while [ $# -gt 0 ]; do
  case "$1" in
    --version) VERSION="${2:-}"; shift 2 ;;
    --keep)    KEEP=1; shift ;;
    *) echo "acquisition-chain.sh: unknown option: $1" >&2; exit 1 ;;
  esac
done
[ -n "$VERSION" ] || { echo "acquisition-chain.sh: --version is required" >&2; exit 1; }

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
PREFIX=$(mktemp -d "${TMPDIR:-/tmp}/loft-acquisition-XXXXXX")
[ "$KEEP" = 1 ] || trap 'rm -rf "$PREFIX"' EXIT INT TERM

say()  { echo "acquisition-chain: $*"; }
fail() { echo "acquisition-chain: FAIL — $*" >&2; exit 1; }

# 1. The documented install path, real transport.  A download failure is exit 3, not a
#    red: before Publish is clicked the assets are simply not there, and "could not run"
#    and "ran and failed" are the two answers this plan exists to keep apart.
say "install.sh --version $VERSION -> $PREFIX"
if ! sh "$ROOT/scripts/install.sh" --version "$VERSION" --prefix "$PREFIX"; then
  say "download/install failed — the release may not be published yet (exit 3)"
  exit 3
fi
LOFT="$PREFIX/bin/loft"
[ -x "$LOFT" ] || fail "install.sh succeeded but $LOFT is not executable"

# 2. The binary is the release it claims.
got=$("$LOFT" --version 2>&1) || fail "the installed loft cannot report --version"
case "$got" in
  *"$VERSION"*) say "--version: $got" ;;
  *) fail "--version says '$got', expected it to name $VERSION" ;;
esac

# 3. The signed index resolves for this binary.  --refresh, because a cached index
#    predating the splice prints the empty-index message.
out=$("$LOFT" self-update --dry-run --refresh 2>&1)
code=$?
echo "$out" | sed 's/^/    /'
[ "$code" = 0 ] || fail "self-update --dry-run --refresh exited $code"
case "$out" in
  *"no releases published to compare against"*)
    fail "the signed index resolves NOTHING — the registry splice has not landed (or the index is unreachable); this is the step-4 omission this gate exists for" ;;
  *"is the newest release"*|*"is available"*) say "self-update resolves against the signed index" ;;
  *"not built for"*) fail "the index has a release but no build for this host" ;;
  *) fail "self-update output matched no known verdict — the gate cannot read it (update this script beside the CLI wording)" ;;
esac

# 4. The anchor, by its literal line.  Exit 0 with the origin check SKIPPED is the
#    trap: intact-but-untraced must not pass a gate about provenance.
vout=$("$LOFT" verify-self 2>&1)
vcode=$?
echo "$vout" | sed 's/^/    /'
[ "$vcode" = 0 ] || fail "verify-self exited $vcode"
case "$vout" in
  *"origin: matches the signed registry index"*) say "verify-self anchors to the signed index" ;;
  *) fail "verify-self did NOT anchor to the signed index (a skipped origin check exits 0 too — that is not a pass)" ;;
esac

# 5. The installed loft runs a program.  A fresh file with a distinctive output, so a
#    cached artefact or an empty run cannot fake the cell (a no-output cell is vacuous).
probe="$PREFIX/chain-probe.loft"
printf 'fn main() {\n    print("chain-ok:%s\\n")\n}\n' "$VERSION" > "$probe"
run=$("$LOFT" --interpret "$probe" 2>&1) || fail "the installed loft cannot run a program: $run"
case "$run" in
  *"chain-ok:$VERSION"*) say "program ran: $run" ;;
  *) fail "program output was '$run', expected chain-ok:$VERSION" ;;
esac

triple=$(uname -m)-$(uname -s)
say "PASS — $VERSION acquired, anchored and executed ($triple, prefix $PREFIX)"
exit 0
