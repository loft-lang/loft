#!/usr/bin/env bash
# @PLN153 phase 0 — run the τ?? census over the corpus and sum it.
#
# One line per file: `<n> <file>` — the LAST census line of the run, which is the whole
# program; the earlier ones are the stdlib loads, each checked on its own.; a final `TOTAL nested=<n> files=<m> failed=<k>`.
# A file the compiler refuses (an @EXPECT_ERROR guard, a missing library) is counted as
# `failed` and contributes nothing — the census only reads programs that reached
# `scopes::check`, so a refused file is neither a 0 nor a finding.
set -u
root="$(cd "$(dirname "$0")/.." && pwd)"
bin="${LOFT_BIN:-$root/target/release/loft}"
[ -x "$bin" ] || { echo "no binary at $bin" >&2; exit 2; }
total=0; files=0; failed=0
while IFS= read -r f; do
  out=$(cd "$(dirname "$f")" && LOFT_NULL_CENSUS=1 LOFT_TIMEOUT=60 "$bin" introspect "$(basename "$f")" 2>&1 >/dev/null)
  line=$(printf '%s\n' "$out" | grep '^null-census: types-scanned=' | tail -1)
  if [ -z "$line" ]; then failed=$((failed+1)); echo "-  $f"; continue; fi
  n=${line##*nested-optional=}
  files=$((files+1)); total=$((total+n))
  echo "$n $f"
  [ "$n" != 0 ] && printf '%s\n' "$out" | grep '^null-census: where' | sed 's/^/     /'
done < <("$(dirname "$0")/corpus_files.sh" --root "$root" --examples | sed "s#^#$root/#")
echo "TOTAL nested=$total files=$files failed=$failed"
