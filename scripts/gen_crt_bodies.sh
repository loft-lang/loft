#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN91 — write selfhost/crt/shim/loft_bodies.c, the compiled standard-library bodies the loft
# runner's C runtime shim includes (`loft --emit-c-bodies`, src/c/bodies.rs).  Run after a
# change to default/*.loft or to the C writer; tests/crt_bodies.rs fails while it is stale.
#
#   scripts/gen_crt_bodies.sh [loft binary]      (default: target/release/loft)
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
loft=${1:-$root/target/release/loft}
stub=$(mktemp -d)/stub.loft
printf 'fn main() {\n}\n' > "$stub"
"$loft" --emit-c-bodies "$root/selfhost/crt/shim/loft_bodies.c" "$stub"
rm -rf "$(dirname "$stub")"
