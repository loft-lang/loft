#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# gate-census — which published library functions still run interpreted, and why (@PLN199 F1).
#
# For every package the registry publishes, at its latest non-yanked version (the matrix
# `scripts/revalidate_matrix.py` reads from ../loft-registry/index.json, the one the
# revalidate sweep uses), install that version into a scratch home and run a program that
# only `use`s it, with `LOFT_TRACE_NATIVE_GATE=only`: the gate's partition of the library,
# and no `rustc`.  Prints, per package, how many of its callable functions compile, then
# every decline reason counted over the whole registry, most frequent first — the order
# the standing shapes of the plan are lifted in.  A reason is grouped by the shape it names
# (a parameter's or a result's type); function names are blanked.  `KEEP_CENSUS=1` keeps the
# per-package traces and says where.
#
# A REPORT, never a gate: exit 0 whatever it finds; 2 when it cannot run at all.
#
# Usage:  scripts/gate_census.sh [package ...]
set -u
case "${1:-}" in
  -h|--help) sed -n '5,16p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
esac
root="$(cd "$(dirname "$0")/.." && pwd)"
registry="$(dirname "$root")/loft-registry/index.json"
loft="$root/target/release/loft"
work="${TMPDIR:-/tmp}/loft-gate-census-$$"
[ -x "$loft" ] || { echo "no release binary at $loft — run: cargo build --release --bin loft" >&2; exit 2; }
[ -f "$registry" ] || { echo "no registry index at $registry — clone loft-lang/loft-registry beside this checkout" >&2; exit 2; }
py=""
for p in python3 python3.13 python3.12 python3.11; do
  command -v "$p" >/dev/null 2>&1 && "$p" -c 'import tomllib' 2>/dev/null && { py="$p"; break; }
done
[ -n "$py" ] || { echo "the matrix reader needs Python 3.11+ (tomllib)" >&2; exit 2; }
mkdir -p "$work/home" "$work/traces"
trap '[ -n "${KEEP_CENSUS:-}" ] && echo "kept $work" >&2 || rm -rf "$work"' EXIT
"$py" "$root/scripts/revalidate_matrix.py" "$registry" 2>/dev/null > "$work/matrix.tsv" || {
  echo "cannot read the registry matrix from $registry" >&2; exit 2; }
want=("$@")
while IFS=$'\t' read -r name version _repo _tag _sub; do
  if [ "${#want[@]}" -gt 0 ] && ! printf '%s\n' "${want[@]}" | grep -qxF "$name"; then
    continue
  fi
  proj="$work/probe-$name"
  mkdir -p "$proj/src"
  printf '[package]\nname = "census_probe"\nversion = "0.0.0"\n\n[dependencies]\n%s = "=%s"\n' \
    "$name" "$version" > "$proj/loft.toml"
  printf 'use %s;\nfn main() {}\n' "$name" > "$proj/src/main.loft"
  (cd "$proj" && LOFT_HOME="$work/home" HOME="$work/home" LOFT_TIMEOUT=300 \
     "$loft" install >/dev/null 2>&1)
  (cd "$proj" && LOFT_HOME="$work/home" HOME="$work/home" LOFT_NO_CACHE=1 LOFT_TIMEOUT=300 \
     LOFT_TRACE_NATIVE_GATE=only "$loft" --interpret src/main.loft 2>&1) \
     | grep '^\[native-gate\]' \
     | awk -v me="$name" '/^\[native-gate\] [^ ]+: / { split($2, a, /[-:]/); own = (a[1] == me) }
                          own' > "$work/traces/$name.txt"
  line="$(grep -m1 "^\[native-gate\] $name[-_0-9.]*:" "$work/traces/$name.txt" \
    | sed 's/^\[native-gate\] [^:]*: //')"
  printf '%-16s %-9s %s\n' "$name" "$version" "${line:-no partition reported (nothing to compile, or the program did not load)}"
done < "$work/matrix.tsv"
echo
echo "decline reasons over the registry (each published function once, under its own package):"
cat "$work"/traces/*.txt 2>/dev/null \
  | grep ' interpreted — ' \
  | sed -E 's/.* interpreted — //; s/`[A-Za-z_][A-Za-z0-9_]*: /`/g; s/(calls|through) `[^`]*`/\1 `…`/g' \
  | sort | uniq -c | sort -rn
exit 0
