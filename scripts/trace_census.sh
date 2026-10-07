#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# The corpus runner a parse-time census rides on: every `.loft` file is compile-checked with one
# trace switch set, and the lines the parser prints under it are collected per tree, over:
#
#   loft       every `.loft` file git carries in this tree;
#   libraries  every `loft-lang/loft-libs-*` repo, freshly cloned at `main` — never a local
#              clone, which can lag it (@PLN112);
#   consumers  a SNAPSHOT (`git archive HEAD`) of each consumer application beside this checkout
#              — read-only: a check writes caches, and their trees are someone else's work;
#   dir:<path> every `.loft` file under one directory, IN PLACE (its git files when it is a
#              repo), labelled `dir` — for a project measuring its own tree.
#
# Its callers own the switch and the report: `eq_census.sh` (@C91, `LOFT_TRACE_EQ_IDENTITY`,
# `[eq-identity]`) and `pub_census.sh` (@PLN187, `LOFT_TRACE_VISIBILITY`, `[visibility]`).
#
# Output: `$WORK/sites.tsv` — one line per distinct site, `tree<TAB>trace text` — and
# `$WORK/files.tsv` / `$WORK/failed.tsv`, the files checked and the ones that did not compile (a
# site in one of those is not seen, so the count says how much is unmeasured).  Prints `$WORK`.
#
# The binary must not be rebuilt while this runs (a check that meets a half-written binary
# counts as unmeasured): point `LOFT` at a copy, with the `default/` it was built for beside it.
#
# Usage:  scripts/trace_census.sh <TRACE_ENV_VAR> <tag> <work-dir>
#         CENSUS_TREES="loft libraries" …   # a subset
set -u
[ $# -eq 3 ] || { echo "usage: trace_census.sh <TRACE_ENV_VAR> <tag> <work-dir>" >&2; exit 2; }
TRACE=$1 TAG=$2 WORK=$3
ROOT=$(cd "$(dirname "$0")/.." && pwd)
LOFT=${LOFT:-$ROOT/target/release/loft}
TREES=${CENSUS_TREES:-loft libraries consumers}
CONSUMERS=${CENSUS_CONSUMERS:-crawler dryopea hexbody moros Moros-Economy-Development routing zero-trust-shared-files}
JOBS=${CENSUS_JOBS:-12}
# Extra `loft` flags for every check — a project's own `--lib` dirs, so its files compile the way
# its Makefile runs them (a file that does not compile hides its sites).
CENSUS_FLAGS=${CENSUS_FLAGS:-}

[ -x "$LOFT" ] || { echo "census: no loft binary at $LOFT (cargo build --release --bin loft)" >&2; exit 2; }
rm -rf "$WORK" && mkdir -p "$WORK/raw"

# check <tree> <file> — compile one file from its package root (the nearest loft.toml), so
# `use` resolves the way `loft` resolves it there; record the trace and whether it compiled.
check_one() {
    tree=$1 file=$2
    dir=$(dirname "$file")
    while [ "$dir" != / ] && [ ! -f "$dir/loft.toml" ]; do dir=$(dirname "$dir"); done
    [ -f "$dir/loft.toml" ] || dir=$(dirname "$file")
    key=$(printf '%s' "$file" | md5sum | cut -c1-16)
    (cd "$dir" && env LOFT_NO_CACHE=1 LOFT_STDLIB_CACHE=1 "$TRACE=1" \
        timeout 60 "$LOFT" $CENSUS_FLAGS --check "$file" >/dev/null 2>"$WORK/raw/$key.err")
    rc=$?
    grep "^\[$TAG\]" "$WORK/raw/$key.err" | sed "s|^\[$TAG\] |$tree\t|" >"$WORK/raw/$key.sites"
    [ $rc -eq 0 ] || printf '%s\t%s\n' "$tree" "$file" >"$WORK/raw/$key.failed"
    rm -f "$WORK/raw/$key.err"
}
export -f check_one
export LOFT WORK TRACE TAG CENSUS_FLAGS

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
                { echo "census: clone of $repo failed" >&2; exit 2; }
            git -C "$WORK/libraries/$repo" ls-files '*.loft' |
                sed "s|^|libraries\t$WORK/libraries/$repo/|" >>"$list"
        done ;;
    consumers)
        mkdir -p "$WORK/consumers"
        for app in $CONSUMERS; do
            src=$(dirname "$ROOT")/$app
            [ -d "$src/.git" ] || { echo "census: no checkout of $app beside loft — skipped" >&2; continue; }
            mkdir -p "$WORK/consumers/$app"
            git -C "$src" archive HEAD | tar -x -C "$WORK/consumers/$app"
            (cd "$WORK/consumers/$app" && find . -name '*.loft' | sed "s|^\./|consumers\t$WORK/consumers/$app/|") >>"$list"
        done ;;
    dir:*)
        d=$(cd "${tree#dir:}" 2>/dev/null && pwd) || { echo "census: no directory ${tree#dir:}" >&2; exit 2; }
        if git -C "$d" rev-parse --show-toplevel >/dev/null 2>&1; then
            git -C "$d" ls-files '*.loft' | sed "s|^|dir\t$d/|" >>"$list"
        else
            find "$d" -name '*.loft' -not -path '*/.loft/*' -not -path '*/target/*' | sed "s|^|dir\t|" >>"$list"
        fi ;;
    *) echo "census: unknown tree '$tree'" >&2; exit 2 ;;
    esac
done

tr '\t' '\n' <"$list" | xargs -P "$JOBS" -d '\n' -n 2 bash -c 'check_one "$0" "$1"'

# Paths are shortened to the tree they live in; a `dir` tree keeps them absolute, so a project
# inside this checkout still resolves against its own root.
cat "$WORK"/raw/*.sites 2>/dev/null | sed "s|$WORK/[a-z]*/||g; /^dir	/!s|$ROOT/||g" | sort -u >"$WORK/sites.tsv"
cat "$WORK"/raw/*.failed 2>/dev/null >"$WORK/failed.tsv"
echo "$WORK"
