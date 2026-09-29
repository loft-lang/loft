#!/usr/bin/env bash
# introspect_diff — is the emission of two compilers BYTE-IDENTICAL over the corpus?
#
# The B7r/B7s method, made a script: run `loft introspect` (IR + bytecode + generated Rust)
# with a BEFORE binary and an AFTER binary over every corpus file and diff the outputs.  A
# refactor that claims to change nothing is verified by this and by nothing weaker — the
# suite passes on a compiler that emits differently, because most cells assert values, and a
# changed emission that happens to compute the same values is still a change nobody asked for.
#
# Usage:
#   scripts/introspect_diff.sh <before-loft> <after-loft> [--env VAR=VALUE]... [--jobs N] [--root DIR]
# Prints one line per DIFFERING file and `IDENTICAL <n>/<m>` or `DIFFERENT <k> of <m>`;
# exit 0 when identical, 1 when not, 2 on a usage error — read it from the SCRIPT, not
# from a pipeline: `introspect_diff.sh a b | tail` reports tail's 0 (`PIPESTATUS` is
# bash-only), which is how a DIFFERENT verdict once read as "identical".  Validated
# 2026-09-05: a binary against itself reads IDENTICAL 1268/1268; a pre-fix build against
# a post-fix one reads DIFFERENT 2 of 1268 and names both files (`.loft` cache DIRECTORIES
# matched `-name '*.loft'` until `-type f`; they were the 9 extra "files" of an earlier 1277).
# A file one compiler REFUSES is compared on its stderr too, so a changed diagnostic is a
# difference as well.
# Both compilers PARSE (`LOFT_NO_CACHE=1`): the question is what the parser emits, and a warm
# startup-cache hit replaces the parse — measured 2026-09-05, a cached bundle rendered every
# variable's number as 65535 in the dump where the fresh parse rendered the real one, so a
# binary living outside `target/` (cached) read as different from one inside it (never cached).
# The corpus is the repo this script lives in (`--root` overrides); an EMPTY corpus is a
# usage error, never a verdict — a copy run from outside the tree once read `IDENTICAL 0/0`
# with exit 0, the vacuous pass GUARDS.md § How a guard reads green while the defect stands warns about.
set -u
root="$(cd "$(dirname "$0")/.." && pwd)"
before="${1:?before binary}"; after="${2:?after binary}"; shift 2
jobs=6; envs=()
while [ $# -gt 0 ]; do
  case "$1" in
    --env) envs+=("$2"); shift 2 ;;
    --jobs) jobs="$2"; shift 2 ;;
    --root) root="$2"; shift 2 ;;
    *) echo "introspect_diff: unknown option '$1'" >&2; exit 2 ;;
  esac
done
[ -x "$before" ] && [ -x "$after" ] || { echo "introspect_diff: both binaries must exist" >&2; exit 2; }
# Each run cd's into the file's directory, so a RELATIVE binary (`target/debug/loft`) would
# resolve there and fail on every file — which reads as DIFFERENT 693 of 693, not as an error.
before="$(readlink -f "$before")"; after="$(readlink -f "$after")"
work="$(mktemp -d)"; trap 'rm -rf "$work"' EXIT
one() {
  local f="$1" bin="$2" out="$3"
  ( cd "$(dirname "$f")" && env LOFT_NO_CACHE=1 "${envs[@]}" LOFT_TIMEOUT=120 "$bin" --path "$root" introspect "$(basename "$f")" >"$out" 2>"$out.err" )
}
export -f one; export root; export envs
find "$root/tests/scripts" "$root/tests/docs" "$root/examples" -type f -name '*.loft' | LC_ALL=C sort > "$work/files"
m=$(wc -l < "$work/files"); n=0; k=0
[ "$m" -gt 0 ] || { echo "introspect_diff: no corpus under '$root' (tests/scripts, tests/docs, examples) — use --root" >&2; exit 2; }
while IFS= read -r f; do
  key=$(printf '%s' "$f" | md5sum | cut -c1-12)
  one "$f" "$before" "$work/$key.a" &
  one "$f" "$after" "$work/$key.b"
  wait
  if cmp -s "$work/$key.a" "$work/$key.b" && cmp -s "$work/$key.a.err" "$work/$key.b.err"; then n=$((n+1)); else k=$((k+1)); echo "DIFF $f"; fi
done < "$work/files"
if [ "$k" = 0 ]; then echo "IDENTICAL $n/$m"; exit 0; else echo "DIFFERENT $k of $m"; exit 1; fi
