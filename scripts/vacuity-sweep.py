#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Vacuity sweep — can each script test actually FAIL?

Flips the FIRST `assert` in every `tests/scripts/*.loft` to `false` and checks the file
then fails WITH an assertion failure.  A file that still passes never executed that
assert, so it is a guard that guards nothing.  A REPORT, never a gate.

Two things it taught, both worth keeping in mind before trusting a run:

  * Run each file the way the HARNESS runs it.  A script with no `main` uses `fn test_*`
    entry points, and `--interpret` executes nothing at all in it — the first sweep
    flagged 166 files, 164 of which simply had no `main`.  With `--tests` for those, the
    real number was 9.
  * `assert(` inside a COMMENT is not code.  Two more of the nine were the mutation
    editing prose.

Of the genuine survivors, the interesting ones were `par` tests: a runtime fault inside a
`parallel` arm is silently discarded (loft#1053), so an in-arm assertion can never fail a
run.  That is what this sweep is for — it finds guards that cannot fail, which no green
suite will ever report.

    python3 scripts/vacuity-sweep.py
"""
import re, subprocess, sys, os, glob, tempfile
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from corpus import corpus_files, rel
BITES, VACUOUS, MALFORMED, SKIP = [], [], [], []
# The programs here, then every program of the language suite beside the tree (@PLN193).
files = [rel(p) for p in corpus_files(docs=False)]
for f in files:
    src = open(f).read()
    m = re.search(r'\bassert\(', src)
    if not m:
        SKIP.append(f); continue
    # replace the first assert's condition with `false`, matching to the comma at depth 0
    i = m.end(); depth = 1; j = i; comma = None
    while j < len(src) and depth > 0:
        c = src[j]
        if c in '([{': depth += 1
        elif c in ')]}': depth -= 1
        elif c == ',' and depth == 1 and comma is None: comma = j
        j += 1
    if comma is None:
        SKIP.append(f); continue
    mutated = src[:i] + 'false' + src[comma:]
    with tempfile.NamedTemporaryFile('w', suffix='.loft', delete=False, dir='/tmp') as t:
        t.write(mutated); tmp = t.name
    env = dict(os.environ, LOFT_TIMEOUT='30')
    try:
        # a file with no `main` uses `fn test_*` entry points the harness discovers;
        # running it with --interpret executes NOTHING, which reads as vacuous and is not
        mode = '--interpret' if re.search(r'^fn main', src, re.M) else '--tests'
        p = subprocess.run(['./target/release/loft', mode, tmp],
                           capture_output=True, text=True, timeout=90, env=env)
        out = (p.stdout + p.stderr).lower()
    except subprocess.TimeoutExpired:
        SKIP.append(f); os.unlink(tmp); continue
    if 'assertion failed' in out:
        BITES.append(f)
    elif p.returncode != 0:
        MALFORMED.append(f)          # mutation broke something else; inconclusive
    else:
        VACUOUS.append(f)            # ran clean with a false assert -> never executed
    os.unlink(tmp)
print(f"scripts scanned      : {len(files)}")
print(f"  no assert / skipped: {len(SKIP)}")
print(f"  assert BITES       : {len(BITES)}")
print(f"  inconclusive       : {len(MALFORMED)}")
print(f"  VACUOUS            : {len(VACUOUS)}")
for f in VACUOUS: print("    VACUOUS", f)
