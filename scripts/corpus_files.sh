#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# The corpus a sweep walks, one path per line, sorted — the ONE place that says what "every
# program" means now that the language suite lives beside this repository (@PLN193):
#
#   tests/scripts/*.loft         the programs this repository still names (TESTING.md)
#   tests/docs/*.loft            the user documentation's pages
#   <suite>/rules/*/*/*.loft     every cell of loft-lang/loft-suite
#   <suite>/attic/scripts/*.loft the suite's inherited corpus
#   examples/**/*.loft           only with --examples
#
# `<suite>` is `$LOFT_SUITE_DIR`, else `../loft-suite` beside the repository root; an absent
# checkout contributes nothing and is SAID on stderr, so a sweep that ran over 431 programs
# instead of 2300 never reads as the same sweep.  Groups are opt-out: `--no-docs`,
# `--no-scripts`, `--no-suite`.  Paths are relative to the repository root unless `--root DIR`
# names another, in which case they are under it.
#
#   scripts/corpus_files.sh                 # scripts + docs + suite
#   scripts/corpus_files.sh --examples      # + examples/
#   scripts/corpus_files.sh --no-docs       # what the corpus RUNNERS cover
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
scripts=1 docs=1 suite=1 examples=0
while [ $# -gt 0 ]; do
  case "$1" in
    --root) root="$2"; shift ;;
    --examples) examples=1 ;;
    --no-docs) docs=0 ;;
    --no-scripts) scripts=0 ;;
    --no-suite) suite=0 ;;
    *) echo "corpus_files.sh: unknown argument $1" >&2; exit 2 ;;
  esac
  shift
done
suite_dir="${LOFT_SUITE_DIR:-$root/../loft-suite}"
case "$suite_dir" in /*) ;; *) suite_dir="$root/$suite_dir" ;; esac
{
  [ "$scripts" = 1 ] && [ -d "$root/tests/scripts" ] && find "$root/tests/scripts" -maxdepth 1 -name '*.loft' -type f
  [ "$docs" = 1 ] && [ -d "$root/tests/docs" ] && find "$root/tests/docs" -maxdepth 1 -name '*.loft' -type f
  [ "$examples" = 1 ] && [ -d "$root/examples" ] && find "$root/examples" -name '*.loft' -type f
  if [ "$suite" = 1 ]; then
    if [ -d "$suite_dir/rules" ]; then
      find "$suite_dir/rules" -mindepth 3 -maxdepth 3 -name '*.loft' -type f
      find "$suite_dir/attic/scripts" -maxdepth 1 -name '*.loft' -type f 2>/dev/null
    else
      echo "corpus_files.sh: no language suite at $suite_dir — its programs are NOT in this sweep (LOFT_SUITE_DIR)" >&2
    fi
  fi
  true
} | sed "s#^$root/##" | LC_ALL=C sort
