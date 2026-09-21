#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Compile an EMITTED Rust file the way `loft --native-release` compiles it — so a rewrite can
# be PRICED before it is built: emit a routine, edit its Rust to the form the rewrite would
# produce, compile both with this, run both, compare the row and its hash.
#
#   loft --native-release --native-emit /tmp/b.rs bench/14_stdlib_vector/bench.loft
#   cp /tmp/b.rs /tmp/b_v1.rs     # …edit ONE function of b_v1.rs by script, never by hand
#   bench/portal/hand_price.sh /tmp/b.rs /tmp/b  &&  bench/portal/hand_price.sh /tmp/b_v1.rs /tmp/b_v1
#   /tmp/b --n 3000 | grep push ;  /tmp/b_v1 --n 3000 | grep push      # ns/op is column 4, the hash column 7
#
# A price counts only when the hash is unchanged and two runs agree.  It is a CEILING for
# the form, not a proof of the rewrite: the conditions under which the form is sound are the
# rule's to state and the emitter's to validate (formal/rewrites.md).  The flags are the ones
# `src/main.rs` passes for a shipped binary; `target/release` must be current
# (`cargo build --release --lib --bin loft`, `make check-rlib`).
#
# Worked examples, with what each priced: bench/portal/analysis/vector-build.md § Method,
# bench/portal/analysis/records.md § Built (R2 — where skipping this cost a rebuild cycle).
set -euo pipefail
if [ $# -ne 2 ]; then
  echo "usage: $0 <emitted.rs> <out-binary>" >&2
  exit 2
fi
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
rel="$root/target/release"
ffi="$(ls "$rel"/deps/libloft_ffi-*.rlib 2>/dev/null | head -1 || true)"
ring="$(ls -d "$rel"/build/ring-*/out 2>/dev/null | head -1 || true)"
if [ ! -f "$rel/deps/libloft.rlib" ] || [ -z "$ffi" ]; then
  echo "$0: no release rlib under $rel — run: cargo build --release --lib --bin loft" >&2
  exit 2
fi
args=(--edition=2024 -o "$2" "$1" -C opt-level=3 -C codegen-units=1
      -Clink-arg=-Wl,--allow-multiple-definition
      --extern "loft=$rel/deps/libloft.rlib" -L "dependency=$rel/deps"
      --extern "loft_ffi=$ffi")
[ -n "$ring" ] && args+=(-L "native=$ring")
# rustc's warnings about generated code are noise here; an error is the answer.
rustc "${args[@]}" 2> >(grep -E "^error" -A 14 >&2 || true)
