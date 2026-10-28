#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Is `src/ir_schema_gen.rs` still a faithful regeneration of `selfhost/src/ir.loft`?
#
# That file is `@generated … DO NOT EDIT — regenerate`, and the store LAYOUT is derived
# from it: record sizes, field byte offsets, the `Node` discriminants that
# `data_store.rs`'s baked `DISC_*` constants mirror.  Nothing gated the two staying in
# step, and they did not: `Key` gained a third field (`start`) in loft#812, the generated
# file was updated to match, `ir.loft` — the SOURCE — was not, and nobody regenerated for
# months.  The next regeneration silently DROPPED `Key.start` and took `KEY_STRIDE` from
# 24 to 16.  A wrong store layout is not a build error; it is a wrong byte offset.
#
# The check is the pipeline run for real, then a byte compare:
#
#   ir.loft --(loft --introspect --show-rust)--> generated.rs --(extract.py)--> compare
#
# `extract.py` emits a single trailing newline precisely so a fresh regeneration is
# byte-identical to the committed file, which is what makes the compare meaningful.
#
# Costs ~0.1 s.  Exits 0 when they agree, 1 when they drift, and 0 with a SKIP line when
# the inputs to run it are not present (see below).
#
# ⚠ It needs a BUILT loft to regenerate with, which is circular in the sense that the
# binary is compiled from the file being checked — but not viciously: whatever state
# `ir_schema_gen.rs` is in, the binary built from it still parses `ir.loft` the same way,
# so a hand-edit shows up as a diff and an un-regenerated `ir.loft` edit shows up as one
# too.  When no binary exists the check SKIPS rather than failing, so a fresh clone is not
# blocked; `make ci` builds one, so there it always runs.
#
# Usage:  scripts/ir_schema_check.sh [--fix]
#           --fix   write the regenerated file over src/ir_schema_gen.rs

set -u
cd "$(dirname "$0")/.."

GEN="src/ir_schema_gen.rs"
SRC="selfhost/src/ir.loft"
EXTRACT="tools/ir_schema/extract.py"

for f in "$GEN" "$SRC" "$EXTRACT"; do
  [ -f "$f" ] || { echo "SKIP ir-schema-check: $f is missing"; exit 0; }
done
command -v python3 >/dev/null 2>&1 || { echo "SKIP ir-schema-check: python3 unavailable"; exit 0; }

LOFT="${LOFT_BIN:-}"
if [ -z "$LOFT" ]; then
  for cand in target/release/loft target/debug/loft; do
    [ -x "$cand" ] && { LOFT="$cand"; break; }
  done
fi
[ -n "$LOFT" ] && [ -x "$LOFT" ] || {
  echo "SKIP ir-schema-check: no built loft (target/{release,debug}/loft, or set LOFT_BIN)"
  exit 0
}

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

if ! LOFT_TIMEOUT="${LOFT_TIMEOUT:-120}" "$LOFT" --introspect --show-rust \
        --rust-out "$TMP/generated.rs" "$SRC" >"$TMP/loft.out" 2>&1; then
  echo "ir-schema-check: FAILED to regenerate from $SRC using $LOFT"
  tail -20 "$TMP/loft.out"
  exit 1
fi

if ! python3 "$EXTRACT" "$TMP/generated.rs" > "$TMP/ir_schema_gen.rs" 2>"$TMP/extract.err"; then
  echo "ir-schema-check: extract.py failed"
  cat "$TMP/extract.err"
  exit 1
fi

if [ "${1:-}" = "--fix" ]; then
  cp "$TMP/ir_schema_gen.rs" "$GEN"
  echo "ir-schema-check: rewrote $GEN from $SRC"
  exit 0
fi

if diff -q "$GEN" "$TMP/ir_schema_gen.rs" >/dev/null 2>&1; then
  echo "ir-schema-check: ok — $GEN matches a fresh regeneration of $SRC"
  exit 0
fi

echo "ir-schema-check: DRIFT — $GEN is not what $SRC generates."
echo
echo "  Either the .loft source was edited without regenerating, or the generated file was"
echo "  hand-edited.  Both change the store LAYOUT silently.  The committed file is <, a"
echo "  fresh regeneration is >:"
echo
diff "$GEN" "$TMP/ir_schema_gen.rs" | head -40
echo
echo "  Fix: decide which side is right.  If ir.loft is, run"
echo "    scripts/ir_schema_check.sh --fix"
echo "  and re-run \`cargo test --lib baked_layout_mirrors_loft_schema\`, which is what"
echo "  proves data_store.rs's baked DISC_*/ND* constants still match the new layout."
exit 1
