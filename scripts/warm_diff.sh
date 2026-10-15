#!/usr/bin/env bash
# warm_diff — does a program run the SAME from its cache as from its sources?
#
# The startup cache (@PLN11) replaces the parse on a warm run: the bundle holds the parsed
# program, and with `LOFT_CODEGEN_STORE=1` it is loaded as a SKELETON whose bodies codegen
# reads straight from the mapped store.  The corpus runner never exercises either path (the
# program cache is off under cargo), so a warm run that differs from a cold one is seen by a
# user first.  This script runs every corpus file through five lanes, in a private cache,
# and compares each lane's exit code, stdout and stderr with the cold reference:
#
#   ref     LOFT_NO_CACHE=1                          parses, caches nothing (the reference)
#   prime   cache on, cold                           parses and SAVES the bundle
#   warm    cache on, warm                           loads the bundle (`open_bundle_into`)
#   sprime  cache on, cold, LOFT_CODEGEN_STORE=1     parses, codegen from a materialised store
#   skel    cache on, warm, LOFT_CODEGEN_STORE=1     skeleton load, codegen from the mapped store
#
# `sprime` is its own priming run: a counting `LOFT_*` switch keys its own bundle
# (`cache::env_counts`), so `skel` after `prime` alone would parse again and prove nothing.
# A file whose `warm` agrees with `ref` and whose `skel` does not is the skeleton load's
# defect; a file where every lane differs — a clock, a random, a pid in the output — is
# NONDETERMINISTIC and not the cache's, and is reported apart.  Each file's five lanes run
# in ONE private cache directory seeded with the stdlib caches (hard links), so a parallel
# run never shares a program bundle and the prune after a cold save cannot evict another
# file's.  Each lane runs from the file's directory with `--path <root>`, as a user would.
#
# Usage:
#   scripts/warm_diff.sh [--loft BIN] [--jobs N] [--root DIR] [--only SUBSTRING] [--keep DIR]
# Prints one line per differing file — `DIFF <lanes> <file>` or `NONDET <file>` — then
# `SAME <n>/<m>` or `DIFFERENT <k> of <m> (prime a, warm b, sprime c, skel d; nondet e)`;
# exit 0 when every deterministic file agrees, 1 when one does not, 2 on a usage error.
# Read the verdict from the SCRIPT's exit, not a pipeline's.  The differing files' outputs
# stay under `--keep DIR` (default: a directory under the temp dir, named at the end) as
# `<key>.<lane>.{out,err,exit}` beside `<key>.file` naming the corpus file.
set -u
root="$(cd "$(dirname "$0")/.." && pwd)"
loft=""; jobs=4; only=""; keep=""
while [ $# -gt 0 ]; do
  case "$1" in
    --loft) loft="$2"; shift 2 ;;
    --jobs) jobs="$2"; shift 2 ;;
    --root) root="$2"; shift 2 ;;
    --only) only="$2"; shift 2 ;;
    --keep) keep="$2"; shift 2 ;;
    *) echo "warm_diff: unknown option '$1'" >&2; exit 2 ;;
  esac
done
[ -n "$loft" ] || loft="$root/target/release/loft"
[ -x "$loft" ] || { echo "warm_diff: no loft binary at '$loft' (cargo build --release --bin loft, or --loft)" >&2; exit 2; }
# Each lane cd's into the file's directory, so a relative binary would resolve there.
loft="$(readlink -f "$loft")"
[ -d "$root/default" ] || { echo "warm_diff: no default/ under '$root' — use --root" >&2; exit 2; }
tmp="${LOFT_TMPDIR:-${TMPDIR:-/tmp}}"
[ -n "$keep" ] || keep="$tmp/loft_warm_diff.$$"
mkdir -p "$keep" || exit 2
work="$(mktemp -d "$tmp/loft_warm_diff_work.XXXXXX")"; trap 'rm -rf "$work"' EXIT

# The stdlib caches, primed ONCE under each environment the lanes use, so no two files race
# to write them and no file pays the stdlib parse twice.  A `hello` with no body of its own.
seed="$work/seed"; mkdir -p "$seed"
printf 'fn main() {}\n' > "$work/seed.loft"
( cd "$work" && env -u LOFT_NO_CACHE -u LOFT_CODEGEN_STORE XDG_CACHE_HOME="$seed" LOFT_TIMEOUT=120 "$loft" --path "$root" --interpret seed.loft >/dev/null 2>&1 )
( cd "$work" && env -u LOFT_NO_CACHE XDG_CACHE_HOME="$seed" LOFT_CODEGEN_STORE=1 LOFT_TIMEOUT=120 "$loft" --path "$root" --interpret seed.loft >/dev/null 2>&1 )
rm -f "$seed"/loft/program-*
[ -n "$(ls "$seed/loft" 2>/dev/null)" ] || { echo "warm_diff: the seed run wrote no stdlib cache under '$seed' — is '$loft' a loft?" >&2; exit 2; }

# One file, five lanes, one verdict line: `SAME`, `DIFF <lanes>` or `NONDET`, then the file.
one() {
  local f="$1" key dir base cache lane out
  key=$(printf '%s' "$f" | md5sum | cut -c1-12)
  dir="$(dirname "$f")"; base="$(basename "$f")"
  cache="$work/c/$key"; mkdir -p "$cache"
  cp -al "$seed/loft" "$cache/loft"
  for lane in ref prime warm sprime skel; do
    out="$work/o/$key.$lane"
    case "$lane" in
      ref)    ( cd "$dir" && env -u LOFT_CODEGEN_STORE LOFT_NO_CACHE=1 LOFT_TIMEOUT=120 "$loft" --path "$root" --interpret "$base" >"$out.out" 2>"$out.err"; echo $? >"$out.exit" ) ;;
      prime|warm)
              ( cd "$dir" && env -u LOFT_NO_CACHE -u LOFT_CODEGEN_STORE XDG_CACHE_HOME="$cache" LOFT_TIMEOUT=120 "$loft" --path "$root" --interpret "$base" >"$out.out" 2>"$out.err"; echo $? >"$out.exit" ) ;;
      sprime|skel)
              ( cd "$dir" && env -u LOFT_NO_CACHE XDG_CACHE_HOME="$cache" LOFT_CODEGEN_STORE=1 LOFT_TIMEOUT=120 "$loft" --path "$root" --interpret "$base" >"$out.out" 2>"$out.err"; echo $? >"$out.exit" ) ;;
    esac
  done
  rm -rf "$cache"
  local ref="$work/o/$key.ref" bad=""
  for lane in prime warm sprime skel; do
    out="$work/o/$key.$lane"
    cmp -s "$ref.out" "$out.out" && cmp -s "$ref.err" "$out.err" && cmp -s "$ref.exit" "$out.exit" || bad="$bad$lane,"
  done
  if [ -z "$bad" ]; then
    rm -f "$work/o/$key".*
    echo "SAME $f"
  else
    mv "$work/o/$key".* "$keep/"; printf '%s\n' "$f" > "$keep/$key.file"
    if [ "$bad" = "prime,warm,sprime,skel," ]; then echo "NONDET $f"; else echo "DIFF ${bad%,} $f"; fi
  fi
}
export -f one; export work seed keep loft root
mkdir -p "$work/o" "$work/c"
find "$root/tests/scripts" "$root/tests/docs" "$root/examples" -type f -name '*.loft' | LC_ALL=C sort > "$work/files"
if [ -n "$only" ]; then grep -F -- "$only" "$work/files" > "$work/files.only"; mv "$work/files.only" "$work/files"; fi
m=$(wc -l < "$work/files")
[ "$m" -gt 0 ] || { echo "warm_diff: no corpus file under '$root' matches — use --root / --only" >&2; exit 2; }
xargs -P "$jobs" -I{} bash -c 'one "$1"' _ {} < "$work/files" > "$work/verdicts"
n=$(grep -c '^SAME ' "$work/verdicts"); e=$(grep -c '^NONDET ' "$work/verdicts"); k=$(grep -c '^DIFF ' "$work/verdicts")
grep -v '^SAME ' "$work/verdicts" | LC_ALL=C sort -k2
if [ $((n + e + k)) -ne "$m" ]; then echo "warm_diff: $((m - n - e - k)) of $m files gave no verdict — a lane was killed?" >&2; exit 2; fi
if [ "$k" = 0 ]; then
  if [ "$e" = 0 ]; then rmdir "$keep" 2>/dev/null; echo "SAME $n/$m"; else echo "SAME $n/$m (nondet $e, outputs under $keep)"; fi
  exit 0
fi
a=$(grep -cE '^DIFF ([^ ]*,)?prime(,| )' "$work/verdicts"); b=$(grep -cE '^DIFF ([^ ]*,)?warm(,| )' "$work/verdicts")
c=$(grep -cE '^DIFF ([^ ]*,)?sprime(,| )' "$work/verdicts"); d=$(grep -cE '^DIFF ([^ ]*,)?skel(,| )' "$work/verdicts")
echo "DIFFERENT $k of $m (prime $a, warm $b, sprime $c, skel $d; nondet $e) — outputs under $keep"
exit 1
