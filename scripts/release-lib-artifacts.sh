#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# release-lib-artifacts — build the compiled artifacts of the published libraries with the
# loft a release ships, so a user without `rustc` runs them compiled (@PLN199 G2c).
#
# For every library the registry publishes, at its latest non-yanked version, install it
# into a scratch home and run a program
# that only `use`s it, the way a user's first run does: that run compiles the library and
# its dependencies into each package's `native-auto/`, under the names a user's run of this
# loft computes for them.  Every artifact found there is copied to OUT, and
# OUT/lib-artifacts-<triple>.json lists them — package, version, triple, stem, file, sha256,
# loft_exe_id — for `gen-toolchain-entry.py --artifacts OUT` to put in the release's row.
#
# LOFT must be the binary the release ships, still beside the `libloft.rlib` it was built
# with (`scripts/lean_build.sh --lib --bin loft`): a bundle carries no rlib, and an artifact
# runs only in the loft that built it.  A library that does not build is reported and left
# out — it runs interpreted — and the script exits 0; 2 when it cannot run at all.
#
# Usage:  scripts/release-lib-artifacts.sh --loft <bin> --out <dir> [--index <index.json>]
#                                          [--registry <url> --allow-unsigned] [package ...]
#   --index     the index whose packages to build (default: loft-lang/registry's main)
#   --registry  where `loft install` fetches from (default: the signed registry)
set -u
case "${1:-}" in
  -h|--help) sed -n '5,25p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
esac
root="$(cd "$(dirname "$0")/.." && pwd)"
loft="" out="" index="" registry="" unsigned=""
want=()
while [ $# -gt 0 ]; do
  case "$1" in
    --loft) loft="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    --index) index="$2"; shift 2 ;;
    --registry) registry="$2"; shift 2 ;;
    --allow-unsigned) unsigned="--allow-unsigned"; shift ;;
    -*) echo "release-lib-artifacts: unknown option $1" >&2; exit 2 ;;
    *) want+=("$1"); shift ;;
  esac
done
[ -n "$loft" ] && [ -x "$loft" ] || { echo "release-lib-artifacts: --loft <bin> is required and must run" >&2; exit 2; }
[ -n "$out" ] || { echo "release-lib-artifacts: --out <dir> is required" >&2; exit 2; }
loft="$(cd "$(dirname "$loft")" && pwd)/$(basename "$loft")"
py=""
for p in python3 python3.13 python3.12 python3.11 python; do
  command -v "$p" >/dev/null 2>&1 && "$p" -c 'import tomllib' 2>/dev/null && { py="$p"; break; }
done
[ -n "$py" ] || { echo "release-lib-artifacts: the matrix reader needs Python 3.11+ (tomllib)" >&2; exit 2; }
mkdir -p "$out"
out="$(cd "$out" && pwd)"
work="$(mktemp -d "${TMPDIR:-/tmp}/loft-lib-artifacts.XXXXXX")"
trap 'rm -rf "$work"' EXIT
home="$work/home"
mkdir -p "$home"
if [ -z "$index" ]; then
  index="$work/index.json"
  curl -fsSL --retry 3 -o "$index" https://raw.githubusercontent.com/loft-lang/registry/main/index.json \
    || { echo "release-lib-artifacts: cannot fetch the registry index" >&2; exit 2; }
fi
# Each library at its latest version: the matrix's own choice (`revalidate_matrix.py`'s
# `NOT_A_LIBRARY`, yanked versions withdrawn, its version order), without its GitHub filter —
# where a package's source lives does not decide whether it compiles.
"$py" - "$root/scripts" "$index" > "$work/matrix.tsv" <<'PY' \
  || { echo "release-lib-artifacts: cannot read the packages of $index" >&2; exit 2; }
import json, sys
sys.path.insert(0, sys.argv[1])
import revalidate_matrix as m
for name, pkg in sorted(json.load(open(sys.argv[2], encoding="utf-8"))["packages"].items()):
    if name in m.NOT_A_LIBRARY:
        continue
    yanked = set(pkg.get("yanked", []))
    versions = [v for v in pkg.get("versions", {}) if v not in yanked]
    if versions:
        print(f"{name}\t{max(versions, key=m._version_key)}")
PY
identity="$("$loft" build-native --identity)" \
  || { echo "release-lib-artifacts: $loft cannot report its identity" >&2; exit 2; }
exe_id="$(printf '%s\n' "$identity" | sed -n 's/^loft_exe_id: //p')"
triple="$(printf '%s\n' "$identity" | sed -n 's/^triple: //p')"
env_run() {
  env LOFT_HOME="$home" HOME="$home" LOFT_NO_CACHE=1 LOFT_TIMEOUT=900 \
    ${registry:+"LOFT_REGISTRY_URL=$registry"} "$@"
}
built=0 failed=0
while IFS=$'\t' read -r name version; do
  if [ "${#want[@]}" -gt 0 ] && ! printf '%s\n' "${want[@]}" | grep -qxF "$name"; then
    continue
  fi
  proj="$work/probe-$name"
  mkdir -p "$proj/src"
  printf '[package]\nname = "artifact_probe"\nversion = "0.0.0"\n\n[dependencies]\n%s = "=%s"\n' \
    "$name" "$version" > "$proj/loft.toml"
  printf 'use %s;\nfn main() {}\n' "$name" > "$proj/src/main.loft"
  if ! (cd "$proj" && env_run "$loft" install $unsigned >"$work/$name.install" 2>&1); then
    echo "  --  $name $version: install failed — $(tail -1 "$work/$name.install")"
    failed=$((failed + 1))
    continue
  fi
  if (cd "$proj" && env_run "$loft" --interpret src/main.loft >"$work/$name.run" 2>&1); then
    built=$((built + 1))
  else
    echo "  --  $name $version: the run failed — $(tail -1 "$work/$name.run")"
    failed=$((failed + 1))
  fi
done < "$work/matrix.tsv"
# Every artifact the runs left, once: a dependency's is built by each library that uses it,
# under one name.  `<package>-<version>` is the install directory (registry_index::extract_dir).
"$py" - "$home/.loft/registry" "$out" "$exe_id" "$triple" <<'PY'
import hashlib, json, pathlib, re, shutil, sys
reg, out, exe_id, triple = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2]), sys.argv[3], sys.argv[4]
rows = []
for art in sorted(reg.glob("*/native-auto/*")):
    m = re.fullmatch(r"(?:lib)?(loft_auto_[A-Za-z0-9_]+)\.(so|dylib|dll)", art.name)
    if not m:
        continue
    pkg = re.fullmatch(r"(.+)-(\d[^-]*)", art.parent.parent.name)
    if not pkg:
        continue
    rows.append({"src": art,
        "package": pkg.group(1), "version": pkg.group(2), "triple": triple,
        "stem": m.group(1), "file": art.name,
        "sha256": hashlib.sha256(art.read_bytes()).hexdigest(), "loft_exe_id": exe_id,
    })
# One name per package version: two would be two programs' builds of it, and the index can
# name one.  Neither is kept — the library runs interpreted, and the line says why.
count = {}
for r in rows:
    count[(r["package"], r["version"])] = count.get((r["package"], r["version"]), 0) + 1
for (p, v), n in sorted(count.items()):
    if n > 1:
        print(f"  --  {p} {v}: {n} artifacts under different names; none is published")
rows = [r for r in rows if count[(r["package"], r["version"])] == 1]
for r in rows:
    shutil.copyfile(r.pop("src"), out / r["file"])
(out / f"lib-artifacts-{triple}.json").write_text(json.dumps(rows, indent=2) + "\n")
for r in rows:
    print(f"  ok  {r['package']} {r['version']}: {r['file']}")
print(f"{len(rows)} artifact(s) for {triple}")
PY
status=$?
[ "$failed" -eq 0 ] || echo "$failed package(s) not built — they run interpreted for this release"
exit $status
