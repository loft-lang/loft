#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN229 — is what loft PRODUCES reproducible?  The same program, built by one loft binary,
# must give the same C and the same executable wherever it is built and however often.
# (`repro-verify.sh` asks the same of the loft binary itself.)
#
#   scripts/repro-output.sh [--loft BIN] [PROGRAM_DIR [MAIN]]
#
# Copies PROGRAM_DIR (default: a small built-in program with a library) to two directories of
# different path lengths, emits the C (`--native-c --native-emit`) and builds the executable
# (`--native-c --out`) in each, twice in the first, and compares all three byte for byte.
# Silent and exit 0 when they agree; otherwise names each artefact that differs, with its
# first differing lines, and exits 1.
set -u
here=$(cd "$(dirname "$0")/.." && pwd)
loft="$here/target/release/loft"
case "${1:-}" in -h|--help) sed -n '5,/^$/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;; esac
if [ "${1:-}" = "--loft" ]; then loft=$2; shift 2; fi
src=${1:-}
main=${2:-main.loft}
work=$(mktemp -d "${TMPDIR:-/tmp}/loft-repro-XXXXXX")
[ -n "${REPRO_KEEP:-}" ] && echo "repro-output: kept $work" || trap 'rm -rf "$work"' EXIT

if [ -z "$src" ]; then
    src="$work/program"
    mkdir -p "$src/lib"
    cat > "$src/lib/shapes.loft" <<'EOF'
pub struct Shape { pub name: text, pub sides: integer }
pub fn describe(s: Shape) -> text { "{s.name}:{s.sides}" }
EOF
    cat > "$src/main.loft" <<'EOF'
use shapes;
struct Tally { key: text, n: integer }
fn main() {
  v = [shapes::Shape { name: "tri", sides: 3 }, shapes::Shape { name: "sq", sides: 4 }];
  h: hash<Tally[key]> = [];
  for s in v { h += [Tally { key: s.name, n: s.sides }]; }
  t = h["tri"];
  println("{shapes::describe(v[1])} {t.n}");
  assert(len(v) == 2, "two shapes");
}
EOF
fi

# Two copies whose paths differ in length, so a path written into an artefact moves its bytes.
a="$work/a"
b="$work/bb/second-copy"
mkdir -p "$a" "$b"
cp -r "$src/." "$a/"
cp -r "$src/." "$b/"

build() { # DIR TAG
    ( cd "$1" && LOFT_TIMEOUT=300 "$loft" --native-c --lib lib --native-emit "$work/$2.c" "$main" ) \
        > "$work/$2.emit.log" 2>&1 || { echo "repro-output: the C of $2 was not written:"; tail -5 "$work/$2.emit.log"; exit 2; }
    ( cd "$1" && LOFT_TIMEOUT=300 "$loft" --native-c --lib lib --out "$work/$2.out" "$main" ) \
        > "$work/$2.build.log" 2>&1 || { echo "repro-output: $2 did not build:"; tail -5 "$work/$2.build.log"; exit 2; }
}
build "$a" a1
build "$a" a2
build "$b" b

exe() { find "$work/$1.out" -maxdepth 1 -type f -perm -u+x | head -1; }
bad=0
compare() { # WHAT X Y
    if ! cmp -s "$2" "$3"; then
        bad=1
        echo "repro-output: $1 differ ($(basename "$2") / $(basename "$3"))"
        case "$1" in
            C*) diff "$2" "$3" | grep '^[<>]' | head -6 | cut -c1-200 ;;
        esac
    fi
}
compare "C, one directory twice:" "$work/a1.c" "$work/a2.c"
compare "C, two directories:" "$work/a1.c" "$work/b.c"
compare "executables, one directory twice:" "$(exe a1)" "$(exe a2)"
compare "executables, two directories:" "$(exe a1)" "$(exe b)"
exit $bad
