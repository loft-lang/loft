#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN175 / @C91 — the census of `==` / `!=` that compare IDENTITY today, which is exactly the
# set of sites whose answer the content `==` changes.  Every `.loft` file is compile-checked
# with `LOFT_TRACE_EQ_IDENTITY=1` (src/parser/mod.rs `trace_eq_identity`), over:
#
#   loft       every `.loft` file git carries in this tree;
#   libraries  every `loft-lang/loft-libs-*` repo, freshly cloned at `main` — never a local
#              clone, which can lag it (@PLN112);
#   consumers  a SNAPSHOT (`git archive HEAD`) of each consumer application beside this checkout
#              — read-only: a check writes caches, and their trees are someone else's work.
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
LOFT=${LOFT:-$ROOT/target/release/loft}
WORK=${EQ_CENSUS_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/tmp/loft-eq-census}
TREES=${EQ_CENSUS_TREES:-loft libraries consumers}
CONSUMERS=${EQ_CENSUS_CONSUMERS:-crawler dryopea hexbody moros Moros-Economy-Development routing zero-trust-shared-files}
JOBS=${EQ_CENSUS_JOBS:-12}
OUT=${1:-/dev/stdout}

[ -x "$LOFT" ] || { echo "eq_census: no loft binary at $LOFT (cargo build --release --bin loft)" >&2; exit 2; }
rm -rf "$WORK" && mkdir -p "$WORK/raw"

# check <tree> <file> — compile one file from its package root (the nearest loft.toml), so
# `use` resolves the way `loft` resolves it there; record the trace and whether it compiled.
check_one() {
    tree=$1 file=$2
    dir=$(dirname "$file")
    while [ "$dir" != / ] && [ ! -f "$dir/loft.toml" ]; do dir=$(dirname "$dir"); done
    [ -f "$dir/loft.toml" ] || dir=$(dirname "$file")
    key=$(printf '%s' "$file" | md5sum | cut -c1-16)
    (cd "$dir" && LOFT_NO_CACHE=1 LOFT_STDLIB_CACHE=1 LOFT_TRACE_EQ_IDENTITY=1 \
        timeout 60 "$LOFT" --check "$file" >/dev/null 2>"$WORK/raw/$key.err")
    rc=$?
    grep '^\[eq-identity\]' "$WORK/raw/$key.err" | sed "s|^\[eq-identity\] |$tree\t|" >"$WORK/raw/$key.sites"
    [ $rc -eq 0 ] || printf '%s\t%s\n' "$tree" "$file" >"$WORK/raw/$key.failed"
    rm -f "$WORK/raw/$key.err"
}
export -f check_one
export LOFT WORK

list=$WORK/files.tsv
: >"$list"
for tree in $TREES; do
    case $tree in
    loft)
        git -C "$ROOT" ls-files '*.loft' | sed "s|^|loft\t$ROOT/|" >>"$list" ;;
    libraries)
        mkdir -p "$WORK/libraries"
        for repo in $(gh api 'orgs/loft-lang/repos?per_page=100' \
            --jq '.[] | select(.archived | not) | .name' | grep '^loft-libs-' | sort); do
            git clone -q --depth 1 "https://github.com/loft-lang/$repo" "$WORK/libraries/$repo" ||
                { echo "eq_census: clone of $repo failed" >&2; exit 2; }
            git -C "$WORK/libraries/$repo" ls-files '*.loft' |
                sed "s|^|libraries\t$WORK/libraries/$repo/|" >>"$list"
        done ;;
    consumers)
        mkdir -p "$WORK/consumers"
        for app in $CONSUMERS; do
            src=$(dirname "$ROOT")/$app
            [ -d "$src/.git" ] || { echo "eq_census: no checkout of $app beside loft — skipped" >&2; continue; }
            mkdir -p "$WORK/consumers/$app"
            git -C "$src" archive HEAD | tar -x -C "$WORK/consumers/$app"
            (cd "$WORK/consumers/$app" && find . -name '*.loft' | sed "s|^\./|consumers\t$WORK/consumers/$app/|") >>"$list"
        done ;;
    *) echo "eq_census: unknown tree '$tree'" >&2; exit 2 ;;
    esac
done

tr '\t' '\n' <"$list" | xargs -P "$JOBS" -d '\n' -n 2 bash -c 'check_one "$0" "$1"'

sites=$WORK/sites.tsv
cat "$WORK"/raw/*.sites 2>/dev/null | sed "s|$WORK/[a-z]*/||; s|$ROOT/||" | sort -u >"$sites"
failed=$(cat "$WORK"/raw/*.failed 2>/dev/null | wc -l)
{
    echo "# @C91 identity census — \`==\` / \`!=\` that compare identity today ($(date -u +%F), loft $(git -C "$ROOT" rev-parse --short HEAD))"
    echo "# tree  kind  site  types"
    awk -F'\t' '{ split($2, a, "  "); printf "%s\t%s\t%s\t%s\n", $1, a[2], a[1], a[3] }' "$sites"
    echo "#"
    echo "# per tree and kind:"
    awk -F'\t' '{ split($2, a, "  "); n[$1 " " a[2]]++ } END { for (k in n) printf "#   %-28s %d\n", k, n[k] }' "$sites" | sort
    echo "# files checked: $(wc -l <"$list"), did not compile (unmeasured): $failed"
} >"$OUT"
