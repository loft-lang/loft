#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Compile an EMITTED Rust file the way `loft --ship` compiles it — so a rewrite can
# be PRICED before it is built: emit a routine, edit its Rust to the form the rewrite would
# produce, compile both with this, run both, compare the row and its hash.
#
#   loft --ship --native-emit /tmp/b.rs bench/14_stdlib_vector/bench.loft
#   cp /tmp/b.rs /tmp/b_v1.rs     # …edit ONE function of b_v1.rs by script, never by hand
#   bench/portal/hand_price.sh /tmp/b.rs /tmp/b  &&  bench/portal/hand_price.sh /tmp/b_v1.rs /tmp/b_v1
#   /tmp/b --n 3000 | grep push ;  /tmp/b_v1 --n 3000 | grep push      # ns/op is column 4, the hash column 7
#
# A price counts only when the hash is unchanged and two runs agree.  It is a CEILING for
# the form, not a proof of the rewrite: the conditions under which the form is sound are the
# rule's to state and the emitter's to validate (formal/rewrites.md).  The flags are the ones
# loft passes for a shipped binary (`src/native_release_args.txt`); `target/release` must be
# current (it links the DEVELOPMENT runtime, with `strict-stores`: a price is a difference, the
# absolute numbers are `bench/stats.py`'s, which measures the production pair) — `cargo build
# --release --lib --bin loft`, `make check-rlib`.
#
# A RUNTIME change (an edit to the rlib, not to the emitter) is A/B'd the same way without
# editing any Rust: emit once, save `target/release/deps/libloft.rlib` from each build, copy
# each into place before compiling the same emitted file, and interleave the two binaries'
# runs — formal/rewrites-history.md § (R-Cold)'s fold clause is the worked example.
#
# Worked examples, with what each priced: bench/portal/analysis/vector-build.md § Method,
# bench/portal/analysis/records.md § Built (R2 — where skipping this cost a rebuild cycle).
#
# A PACKAGE's bench links its libraries' native halves, and which ones, from where, only the
# compile itself knows.  So that compile writes its arguments down and this replays them:
#
#   cd <package> && LOFT_NATIVE_NO_CACHE=1 LOFT_NATIVE_RUSTC_ARGS=/tmp/b.args \
#       loft --ship bench/bench.loft --n 2
#   loft --ship --native-emit /tmp/b.rs bench/bench.loft
#   bench/portal/hand_price.sh /tmp/b.rs /tmp/b /tmp/b.args
#
# Only the source and the output are replaced, and the symbols are kept (a profile needs them).
# HAND_PRICE_RUSTFLAGS adds flags to either compile (`-C debuginfo=line-tables-only`, which
# scripts/gap_report.py uses to charge instructions to loft lines).  HAND_PRICE_REL names the
# build directory to link instead of `target/release`: `target/install-lib/release` is the
# production runtime (`scripts/lean_build.sh`), the one `bench/stats.py` measures.
set -euo pipefail
read -r -a extra <<< "${HAND_PRICE_RUSTFLAGS:-}"
if [ $# -lt 2 ] || [ $# -gt 3 ]; then
  echo "usage: $0 <emitted.rs> <out-binary> [<rustc-args-file>]" >&2
  exit 2
fi
if [ $# -eq 3 ]; then
  args=()
  prev=""
  while IFS= read -r arg; do
    if [ "$prev" = "-o" ]; then
      args+=("$2")
    elif [ "$arg" = "-Cstrip=symbols" ]; then
      :
    elif [[ "$arg" == *.rs && "$prev" != "--extern" && "$prev" != "-L" ]]; then
      args+=("$1")
    else
      args+=("$arg")
    fi
    prev="$arg"
  done < "$3"
  rustc "${args[@]}" ${extra[@]+"${extra[@]}"} 2> >(grep -E "^error" -A 14 >&2 || true)
  exit
fi
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
rel="${HAND_PRICE_REL:-$root/target/release}"
ffi="$(ls -t "$rel"/deps/libloft_ffi-*.rlib 2>/dev/null | head -1 || true)"
ring="$(ls -d "$rel"/build/ring-*/out 2>/dev/null | head -1 || true)"
if [ ! -f "$rel/deps/libloft.rlib" ] || [ -z "$ffi" ]; then
  echo "$0: no release rlib under $rel — run: cargo build --release --lib --bin loft (scripts/lean_build.sh --lib --bin loft for target/install-lib)" >&2
  exit 2
fi
mapfile -t release < <(grep -vE '^[[:space:]]*(#|$)' "$root/src/native_release_args.txt")
args=(--edition=2024 -o "$2" "$1" "${release[@]}"
      --extern "loft=$rel/deps/libloft.rlib" -L "dependency=$rel/deps"
      --extern "loft_ffi=$ffi")
[ -n "$ring" ] && args+=(-L "native=$ring")
# macOS ld64 rejects the GNU option; `src/main.rs` leaves it out there too.
[ "$(uname -s)" != Darwin ] && args+=(-Clink-arg=-Wl,--allow-multiple-definition)
# rustc's warnings about generated code are noise here; an error is the answer.
rustc "${args[@]}" ${extra[@]+"${extra[@]}"} 2> >(grep -E "^error" -A 14 >&2 || true)
