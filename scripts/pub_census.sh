#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN187 / C140 — the census of what needs `pub` once fields, struct literals and enum variants
# are private to the file that declares them: every use, OUTSIDE that file, of a field (read,
# write, pattern, literal field, key), of a struct literal and of an enum variant
# (`LOFT_TRACE_VISIBILITY=1`, src/parser/mod.rs `trace_visibility`), over the corpus
# `trace_census.sh` walks.
#
# Output (stdout, or the file given): one line per item — `tree  declared-in  kind  item  uses` —
# sorted by declaring file, then a count per tree and kind, and the number of files that did not
# compile (unmeasured).  A `field` / `literal-field` / `pattern-field` / `key` row is a field that
# needs `pub`; a `literal` or `variant` row is a type that needs `pub`.
#
# Usage:  scripts/pub_census.sh [out-file]
#         PUB_CENSUS_TREES="loft" scripts/pub_census.sh   # a subset
set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd)
WORK=${PUB_CENSUS_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/tmp/loft-pub-census}
OUT=${1:-/dev/stdout}
export CENSUS_TREES=${PUB_CENSUS_TREES:-loft libraries consumers}
"$ROOT/scripts/trace_census.sh" LOFT_TRACE_VISIBILITY visibility "$WORK" >/dev/null || exit $?

# A site line: `tree<TAB>file:line kind item — declared in decl-file`.
items=$WORK/items.tsv
awk -F'\t' '{
    split($2, w, " ");
    kind = w[2]; item = w[3];
    decl = $2; sub(/.* — declared in /, "", decl);
    n[$1 "\t" decl "\t" kind "\t" item]++
} END { for (k in n) printf "%s\t%d\n", k, n[k] }' "$WORK/sites.tsv" | sort -t$'\t' -k2,2 -k4,4 -k3,3 >"$items"
{
    echo "# @PLN187 pub census — uses outside the declaring file ($(date -u +%F), loft $(git -C "$ROOT" rev-parse --short HEAD))"
    echo "# tree  declared-in  kind  item  uses"
    cat "$items"
    echo "#"
    echo "# items per tree and kind:"
    awk -F'\t' '{ n[$1 " " $3]++ } END { for (k in n) printf "#   %-28s %d\n", k, n[k] }' "$items" | sort
    echo "# files checked: $(wc -l <"$WORK/files.tsv"), did not compile (unmeasured): $(wc -l <"$WORK/failed.tsv")"
} >"$OUT"
