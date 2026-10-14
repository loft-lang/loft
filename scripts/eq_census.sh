#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN175 / @C91 — the census of `==` / `!=` that compare IDENTITY today, which is exactly the
# set of sites whose answer the content `==` changes.  Every `.loft` file is compile-checked
# with `LOFT_TRACE_EQ_IDENTITY=1` (src/parser/mod.rs `trace_eq_identity`) over this tree, every
# library and a snapshot of each consumer — the corpus `trace_census.sh` walks.
#
# Output (stdout, or the file given): one line per distinct site, `tree  kind  file:line  types`,
# sorted, then a count per tree and kind, and the number of files that did not compile (a site
# in one of those is not seen — the count says how much is unmeasured).
#
# The binary must not be rebuilt while this runs (a check that meets a half-written binary
# counts as unmeasured): point `LOFT` at a copy, with the `default/` it was built for beside it.
#
# Usage:  scripts/eq_census.sh [out-file]
#         EQ_CENSUS_TREES="loft libraries" scripts/eq_census.sh   # a subset
set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd)
WORK=${EQ_CENSUS_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/tmp/loft-eq-census}
OUT=${1:-/dev/stdout}
export CENSUS_TREES=${EQ_CENSUS_TREES:-loft libraries consumers}
[ -n "${EQ_CENSUS_CONSUMERS:-}" ] && export CENSUS_CONSUMERS=$EQ_CENSUS_CONSUMERS
[ -n "${EQ_CENSUS_JOBS:-}" ] && export CENSUS_JOBS=$EQ_CENSUS_JOBS
"$ROOT/scripts/trace_census.sh" LOFT_TRACE_EQ_IDENTITY eq-identity "$WORK" >/dev/null || exit $?

sites=$WORK/sites.tsv
failed=$(wc -l <"$WORK/failed.tsv")
{
    echo "# @C91 identity census — \`==\` / \`!=\` that compare identity today ($(date -u +%F), loft $(git -C "$ROOT" rev-parse --short HEAD))"
    echo "# tree  kind  site  types"
    awk -F'\t' '{ split($2, a, "  "); printf "%s\t%s\t%s\t%s\n", $1, a[2], a[1], a[3] }' "$sites"
    echo "#"
    echo "# per tree and kind:"
    awk -F'\t' '{ split($2, a, "  "); n[$1 " " a[2]]++ } END { for (k in n) printf "#   %-28s %d\n", k, n[k] }' "$sites" | sort
    echo "# files checked: $(wc -l <"$WORK/files.tsv"), did not compile (unmeasured): $failed"
} >"$OUT"
